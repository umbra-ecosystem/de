//! The rules. Each is a small pure function `(&Snapshot, now, &mut Vec<Suggestion>)`, named
//! after what it notices, whose doc comment says which step of the workflow (GOAL.md, "The
//! real workflow") it comes from. Rules never do I/O, never read a clock (`now` is passed
//! in) and never look at responses; [`super::engine::suggest`] runs them all and post-
//! processes the result (hotfix boost, unavailable adapters, de-duplication, ordering).
//!
//! A rule states the facts in its `reason` and in `facts`. `facts` must only hold values
//! that change when the situation changes (ids, states, commits, timestamps of events), not
//! ones that change with the clock, because a dismissal is tied to their hash.

use serde_json::{Value, json};

use super::{
    model::{RuleId, SuggestedAction, Suggestion, prio},
    snapshot::{DeployInfo, PrepState, Snapshot, TicketSnapshot, jira_priority_rank, status_is},
};
use crate::{domain::LocalStatus, gateway::Action, git::short_sha, integration::DeployState};

/// Data older than this (five minutes) is stale and a sync is suggested.
pub const SYNC_STALE_SECS: i64 = 300;
/// After a sync attempt (successful or not) another is not suggested for this long.
pub const SYNC_RETRY_SECS: i64 = 60;
/// An `Active` ticket untouched for this long (five days) is stale.
pub const STALE_ACTIVE_SECS: i64 = 5 * 86_400;
/// A `Parked` ticket untouched for this long (two weeks) is stale.
pub const STALE_PARKED_SECS: i64 = 14 * 86_400;
/// How much of a comment goes into the facts.
const MENTION_TEXT_MAX: usize = 400;

/// Runs every rule, in a fixed order, and returns their raw output.
pub fn all_rules(s: &Snapshot, now: i64) -> Vec<Suggestion> {
    let mut out = Vec::new();
    sync_stale(s, now, &mut out);
    claim_new(s, now, &mut out);
    returned_to_review(s, now, &mut out);
    start_review(s, now, &mut out);
    finish_review(s, now, &mut out);
    re_review(s, now, &mut out);
    activate_reviewed(s, now, &mut out);
    park_active(s, now, &mut out);
    stale_ticket(s, now, &mut out);
    integrate_ready(s, now, &mut out);
    integration_blocked(s, now, &mut out);
    remerge_needed(s, now, &mut out);
    revert_overlay(s, now, &mut out);
    deploy_problems(s, now, &mut out);
    compose_deploy_comment(s, now, &mut out);
    post_deploy_comment(s, now, &mut out);
    transition_alpha(s, now, &mut out);
    returned_mention(s, now, &mut out);
    approve_prs(s, now, &mut out);
    out
}

// ---------------------------------------------------------------- helpers

fn tracked(s: &Snapshot) -> impl Iterator<Item = &TicketSnapshot> {
    s.tickets.iter().filter(|t| t.is_tracked())
}

/// Jira shows the ticket as Returned or finished. GOAL.md: a Returned ticket is ignored
/// unless you are mentioned ([`returned_mention`]) or it comes back to Review
/// ([`returned_to_review`]), so the rules that push it along the local flow stay quiet.
fn sent_back(s: &Snapshot, t: &TicketSnapshot) -> bool {
    let status = t.jira_status.as_deref();
    status_is(status, &s.statuses.returned) || s.statuses.done.iter().any(|d| status_is(status, d))
}

fn pr_lines(t: &TicketSnapshot) -> Vec<Value> {
    t.open_prs()
        .map(|p| {
            json!({
                "repo": p.pr.repo,
                "pr": p.pr.id,
                "destination": p.pr.destination_branch,
                "url": p.pr.url,
            })
        })
        .collect()
}

fn describe_prs(t: &TicketSnapshot) -> String {
    let prs: Vec<String> = t
        .open_prs()
        .map(|p| {
            format!(
                "{} #{} into {}",
                p.pr.repo, p.pr.id, p.pr.destination_branch
            )
        })
        .collect();
    if prs.is_empty() {
        "no open PR is synced yet".into()
    } else {
        prs.join(", ")
    }
}

/// The newest recorded merge time of the ticket.
fn latest_merge_at(t: &TicketSnapshot) -> Option<i64> {
    t.merges.iter().map(|m| m.recorded_at).max()
}

/// The deploy comment was already posted after the newest merge.
fn announced(t: &TicketSnapshot) -> bool {
    match (t.comment_posted_at, latest_merge_at(t)) {
        (Some(posted), Some(merged)) => posted >= merged,
        (Some(_), None) => true,
        _ => false,
    }
}

fn ticket_base(t: &TicketSnapshot) -> serde_json::Map<String, Value> {
    let mut m = serde_json::Map::new();
    m.insert("ticket".into(), json!(t.key));
    m
}

fn facts(t: &TicketSnapshot, extra: Value) -> Value {
    let mut base = ticket_base(t);
    if let Value::Object(extra) = extra {
        base.extend(extra);
    }
    Value::Object(base)
}

// ------------------------------------------------------------------ rules

/// Sync data when a source is stale or was never synced. Not from a workflow step: every
/// other rule is only as good as the cache. Level `Automatic`.
pub fn sync_stale(s: &Snapshot, now: i64, out: &mut Vec<Suggestion>) {
    let mut stale = Vec::new();
    for source in &s.sync.expected {
        let state = s.sync.states.iter().find(|st| &st.source == source);
        let due = match state {
            None => true,
            Some(st) => {
                let fresh = st.last_ok_at.is_some_and(|ok| now - ok < SYNC_STALE_SECS);
                let tried_lately = now - st.last_attempt_at < SYNC_RETRY_SECS;
                !fresh && !tried_lately
            }
        };
        if due {
            stale.push(json!({
                "source": source,
                "last_ok_at": state.and_then(|st| st.last_ok_at),
                "last_error": state.and_then(|st| st.last_error.clone()),
            }));
        }
    }
    if stale.is_empty() {
        return;
    }
    let names: Vec<&str> = stale
        .iter()
        .filter_map(|v| v.get("source").and_then(Value::as_str))
        .collect();
    out.push(Suggestion::new(
        None,
        RuleId::SyncStale,
        "-",
        SuggestedAction::Sync,
        format!(
            "{} not synced in the last {} minutes: {}",
            if names.len() == 1 {
                "A source is"
            } else {
                "Sources are"
            },
            SYNC_STALE_SECS / 60,
            names.join(", ")
        ),
        json!({ "stale": stale }),
        prio::AUTOMATIC,
    ));
}

/// Step 1, "Appears": a ticket in the Review column that you have not claimed. Ordered by
/// Jira priority (the priority number steps down with each rank), then key.
pub fn claim_new(s: &Snapshot, _now: i64, out: &mut Vec<Suggestion>) {
    let mut candidates: Vec<&TicketSnapshot> = s
        .tickets
        .iter()
        .filter(|t| !t.is_tracked() && status_is(t.jira_status.as_deref(), &s.statuses.review))
        .collect();
    candidates.sort_by(|a, b| {
        jira_priority_rank(a.jira_priority.as_deref())
            .cmp(&jira_priority_rank(b.jira_priority.as_deref()))
            .then_with(|| a.key.cmp(&b.key))
    });
    for (position, t) in candidates.iter().enumerate() {
        let rank = jira_priority_rank(t.jira_priority.as_deref());
        out.push(Suggestion::new(
            Some(&t.key),
            RuleId::ClaimNew,
            "-",
            SuggestedAction::Claim {
                ticket: t.key.clone(),
            },
            format!(
                "{} is in {} and not claimed (Jira priority {}, #{} in the queue)",
                t.label(),
                s.statuses.review,
                t.jira_priority.as_deref().unwrap_or("unset"),
                position + 1
            ),
            facts(
                t,
                json!({
                    "jira_status": t.jira_status,
                    "jira_priority": t.jira_priority,
                    // No queue position: it changes whenever another ticket enters or
                    // leaves the queue and would bring a dismissed claim back each time.
                }),
            ),
            prio::CLAIM_TOP - prio::CLAIM_STEP * rank,
        ));
    }
}

/// Step 7, "Afterwards": a ticket you integrated came back from testing. It went to alpha
/// (its transition was posted), and Jira, synced after that, shows Review again, so it is
/// treated as new: claim it again.
pub fn returned_to_review(s: &Snapshot, _now: i64, out: &mut Vec<Suggestion>) {
    for t in tracked(s) {
        let status = t.status();
        if !matches!(status, Some(LocalStatus::Integrated | LocalStatus::Done)) {
            continue;
        }
        let (Some(moved), Some(fetched)) = (t.transition_posted_at, t.jira_fetched_at) else {
            continue;
        };
        if fetched <= moved || !status_is(t.jira_status.as_deref(), &s.statuses.review) {
            continue;
        }
        out.push(Suggestion::new(
            Some(&t.key),
            RuleId::ReturnedToReview,
            "-",
            SuggestedAction::Claim {
                ticket: t.key.clone(),
            },
            format!(
                "{} went to {} and is back in {}: handle it as a new ticket",
                t.label(),
                s.statuses.alpha_testing,
                s.statuses.review
            ),
            facts(
                t,
                json!({
                    "local_status": status.map(LocalStatus::as_str),
                    "jira_status": t.jira_status,
                    "transition_posted_at": moved,
                    "jira_fetched_at": fetched,
                }),
            ),
            prio::RETURNED_TO_REVIEW,
        ));
    }
}

/// Step 2, "Review": a claimed ticket whose review has not begun. The diff to read is each
/// PR's own source against its destination.
pub fn start_review(s: &Snapshot, _now: i64, out: &mut Vec<Suggestion>) {
    for t in tracked(s).filter(|t| t.status() == Some(LocalStatus::Claimed) && !sent_back(s, t)) {
        out.push(Suggestion::new(
            Some(&t.key),
            RuleId::StartReview,
            "-",
            SuggestedAction::StartReview {
                ticket: t.key.clone(),
            },
            format!(
                "{} is claimed and not reviewed: {}",
                t.label(),
                describe_prs(t)
            ),
            facts(t, json!({ "prs": pr_lines(t) })),
            prio::START_REVIEW,
        ));
    }
}

/// Step 2, "Review": under review but not marked reviewed yet. Marking it records the
/// branch tips covered, which is what [`re_review`] compares against later.
pub fn finish_review(s: &Snapshot, _now: i64, out: &mut Vec<Suggestion>) {
    for t in tracked(s).filter(|t| {
        t.status() == Some(LocalStatus::Reviewing)
            && t.review.reviewed_at.is_none()
            && !sent_back(s, t)
    }) {
        out.push(Suggestion::new(
            Some(&t.key),
            RuleId::FinishReview,
            "-",
            SuggestedAction::MarkReviewed {
                ticket: t.key.clone(),
            },
            format!(
                "{} is under review; mark it reviewed when you are done ({})",
                t.label(),
                describe_prs(t)
            ),
            facts(t, json!({ "prs": pr_lines(t) })),
            prio::FINISH_REVIEW,
        ));
    }
}

/// One way a reviewed ticket changed since the review.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewChange {
    /// The ticket branch of `repo` moved.
    HeadMoved {
        repo: String,
        from: String,
        to: String,
    },
    /// A PR was updated after the review, in a repo whose branch tips are not known.
    PrUpdated {
        repo: String,
        pr: u64,
        updated_at: i64,
    },
}

/// What changed since the ticket was reviewed. Branch tips decide where both the reviewed
/// and the current tip are known (so a comment on the PR does not count); the PR's
/// `updated_at` decides for the rest.
pub fn review_changes(t: &TicketSnapshot) -> Vec<ReviewChange> {
    let Some(reviewed_at) = t.review.reviewed_at else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (repo, old) in &t.review.reviewed_heads {
        if let Some(now) = t.review.current_heads.get(repo)
            && now != old
        {
            out.push(ReviewChange::HeadMoved {
                repo: repo.clone(),
                from: old.clone(),
                to: now.clone(),
            });
        }
    }
    for p in t.open_prs() {
        let covered = p.project.as_ref().is_some_and(|proj| {
            t.review.reviewed_heads.contains_key(proj) && t.review.current_heads.contains_key(proj)
        });
        if !covered && p.pr.updated_at > reviewed_at {
            out.push(ReviewChange::PrUpdated {
                repo: p.pr.repo.clone(),
                pr: p.pr.id,
                updated_at: p.pr.updated_at,
            });
        }
    }
    out
}

/// Step 2, "Review", again: new commits (or a PR update) since you reviewed. Restarting the
/// review clears the marker.
pub fn re_review(s: &Snapshot, _now: i64, out: &mut Vec<Suggestion>) {
    for t in tracked(s).filter(|t| {
        matches!(
            t.status(),
            Some(LocalStatus::Reviewing | LocalStatus::Parked | LocalStatus::Active)
        ) && !sent_back(s, t)
    }) {
        let changes = review_changes(t);
        if changes.is_empty() {
            continue;
        }
        let described: Vec<String> = changes
            .iter()
            .map(|c| match c {
                ReviewChange::HeadMoved { repo, from, to } => {
                    format!("{repo} moved {}..{}", short_sha(from), short_sha(to))
                }
                ReviewChange::PrUpdated { repo, pr, .. } => format!("{repo} PR #{pr} was updated"),
            })
            .collect();
        let as_json: Vec<Value> = changes
            .iter()
            .map(|c| match c {
                ReviewChange::HeadMoved { repo, from, to } => {
                    json!({ "repo": repo, "from": from, "to": to })
                }
                ReviewChange::PrUpdated {
                    repo,
                    pr,
                    updated_at,
                } => {
                    json!({ "repo": repo, "pr": pr, "updated_at": updated_at })
                }
            })
            .collect();
        out.push(Suggestion::new(
            Some(&t.key),
            RuleId::ReReview,
            "-",
            SuggestedAction::StartReview {
                ticket: t.key.clone(),
            },
            format!(
                "{} changed since you reviewed it: {}",
                t.label(),
                described.join("; ")
            ),
            facts(
                t,
                json!({ "reviewed_at": t.review.reviewed_at, "changes": as_json }),
            ),
            prio::RE_REVIEW,
        ));
    }
}

/// Step 3, "Local test": a reviewed (or parked) ticket, and nothing is active. A hotfix asks
/// for its baseline first. While another ticket is active, [`park_active`] speaks instead.
pub fn activate_reviewed(s: &Snapshot, _now: i64, out: &mut Vec<Suggestion>) {
    if s.active().is_some() {
        return;
    }
    for t in tracked(s).filter(|t| !sent_back(s, t)) {
        let (priority, why) = match t.status() {
            Some(LocalStatus::Reviewing) if t.review.reviewed_at.is_some() => {
                (prio::ACTIVATE_REVIEWED, "is reviewed and not tested yet")
            }
            Some(LocalStatus::Parked) if t.remerge_repos.is_empty() => {
                (prio::ACTIVATE_PARKED, "is parked; resume testing it")
            }
            _ => continue,
        };
        let action = if t.is_hotfix() {
            SuggestedAction::ChooseHotfixBaseline {
                ticket: t.key.clone(),
            }
        } else {
            SuggestedAction::Activate {
                ticket: t.key.clone(),
                baseline: None,
            }
        };
        out.push(Suggestion::new(
            Some(&t.key),
            RuleId::ActivateReviewed,
            "-",
            action,
            format!(
                "{} {why} and nothing is active{}",
                t.label(),
                if t.is_hotfix() {
                    " (hotfix: you choose the baseline for the other repos)"
                } else {
                    ""
                }
            ),
            facts(
                t,
                json!({ "local_status": t.status().map(LocalStatus::as_str), "hotfix": t.is_hotfix() }),
            ),
            priority,
        ));
    }
}

/// Step 3, "Local test": a reviewed ticket is waiting while another is active, so finish
/// or park the active one first. Uncommitted work on it is stashed by parking, which is
/// what makes the dirty-tree case safe; the reason says so. A waiting hotfix raises this.
pub fn park_active(s: &Snapshot, _now: i64, out: &mut Vec<Suggestion>) {
    let Some(active) = s.active() else { return };
    let waiting: Vec<&TicketSnapshot> = tracked(s)
        .filter(|t| t.key != active.key && !sent_back(s, t))
        .filter(|t| {
            (t.status() == Some(LocalStatus::Reviewing) && t.review.reviewed_at.is_some())
                || t.status() == Some(LocalStatus::Parked)
        })
        .collect();
    if waiting.is_empty() {
        return;
    }
    let hotfix_waiting = waiting.iter().any(|t| t.is_hotfix());
    let dirty = !active.dirty_repos.is_empty();
    let base = if dirty {
        prio::PARK_ACTIVE_DIRTY
    } else {
        prio::PARK_ACTIVE
    };
    let names: Vec<String> = waiting.iter().map(|t| t.key.to_string()).collect();
    out.push(Suggestion::new(
        Some(&active.key),
        RuleId::ParkActive,
        "-",
        SuggestedAction::Park {
            ticket: active.key.clone(),
        },
        format!(
            "{} is waiting to be tested but {} is active: finish or park {} first{}",
            names.join(", "),
            active.key,
            active.key,
            if dirty {
                format!(
                    " (uncommitted changes in {} will be stashed)",
                    active.dirty_repos.join(", ")
                )
            } else {
                String::new()
            }
        ),
        facts(
            active,
            json!({
                "waiting": names,
                "waiting_hotfix": hotfix_waiting,
                "dirty_repos": active.dirty_repos,
            }),
        ),
        if hotfix_waiting {
            base + prio::HOTFIX_BOOST
        } else {
            base
        },
    ));
}

/// Stale active or parked ticket: an active one nobody touched for days is parked, a parked
/// one nobody wants any more is marked done.
pub fn stale_ticket(s: &Snapshot, now: i64, out: &mut Vec<Suggestion>) {
    for t in tracked(s) {
        let Some(tracking) = &t.tracking else {
            continue;
        };
        let age = now - tracking.updated_at;
        let (action, what) = match tracking.status {
            LocalStatus::Active if age >= STALE_ACTIVE_SECS => (
                SuggestedAction::Park {
                    ticket: t.key.clone(),
                },
                "active",
            ),
            LocalStatus::Parked if age >= STALE_PARKED_SECS => (
                SuggestedAction::MarkDone {
                    ticket: t.key.clone(),
                },
                "parked",
            ),
            _ => continue,
        };
        out.push(Suggestion::new(
            Some(&t.key),
            RuleId::StaleTicket,
            "-",
            action,
            format!(
                "{} has been {what} for {} days without a change",
                t.label(),
                age / 86_400
            ),
            facts(
                t,
                json!({ "status": tracking.status.as_str(), "since": tracking.updated_at }),
            ),
            prio::STALE_TICKET,
        ));
    }
}

/// Step 4, "Integrate": the active ticket's checklist is complete. Preparing builds the
/// merge into `uat` without pushing; the push (`PushUat`) comes out of that result, since it
/// needs the prepared merge, and is always previewed and confirmed. Also what to do again
/// after a fix following alpha ([`remerge_needed`] re-activates the ticket first).
pub fn integrate_ready(s: &Snapshot, _now: i64, out: &mut Vec<Suggestion>) {
    for t in tracked(s).filter(|t| t.status() == Some(LocalStatus::Active)) {
        if !t.checklist.is_complete() || t.touched_repos.is_empty() || sent_back(s, t) {
            continue;
        }
        let blocked = t.prep.as_ref().is_some_and(|p| {
            p.repos
                .iter()
                .any(|r| matches!(r.state, PrepState::Conflict | PrepState::Blocked))
        });
        let again = !t.remerge_repos.is_empty();
        out.push(Suggestion::new(
            Some(&t.key),
            RuleId::IntegrateReady,
            "-",
            SuggestedAction::RunIntegrationPrepare {
                ticket: t.key.clone(),
            },
            format!(
                "{} is tested ({}/{} checklist items done): integrate {} into uat{}{}",
                t.label(),
                t.checklist.done,
                t.checklist.total,
                t.touched_repos.join(", "),
                if again {
                    format!(
                        " again ({} have new commits since the last merge)",
                        t.remerge_repos.join(", ")
                    )
                } else {
                    String::new()
                },
                if blocked {
                    "; the last attempt was blocked, retry once it is fixed"
                } else {
                    ""
                }
            ),
            facts(
                t,
                json!({
                    "touched_repos": t.touched_repos,
                    "checklist": { "done": t.checklist.done, "total": t.checklist.total },
                    "remerge_repos": t.remerge_repos,
                    "blocked": blocked,
                }),
            ),
            prio::INTEGRATE_READY,
        ));
    }
}

/// Step 4, "Integrate": the last prepare found `uat` conflicts or a blocked repo. Nothing
/// can be pushed until it is fixed; the files are shown so you know whose changes clash.
pub fn integration_blocked(s: &Snapshot, _now: i64, out: &mut Vec<Suggestion>) {
    for t in tracked(s).filter(|t| t.status() == Some(LocalStatus::Active)) {
        let Some(prep) = &t.prep else { continue };
        for r in &prep.repos {
            let (reason, files) = match r.state {
                PrepState::Conflict => (
                    format!(
                        "{} conflicts with uat ({}); resolve it before integrating",
                        r.repo,
                        r.files.join(", ")
                    ),
                    r.files.clone(),
                ),
                PrepState::Blocked => (
                    format!(
                        "{} is blocked from integrating: {}",
                        r.repo,
                        r.reason.as_deref().unwrap_or("unknown reason")
                    ),
                    Vec::new(),
                ),
                _ => continue,
            };
            out.push(Suggestion::new(
                Some(&t.key),
                RuleId::IntegrationBlocked,
                &r.repo,
                SuggestedAction::ResolveConflict {
                    ticket: t.key.clone(),
                    repo: r.repo.clone(),
                    files: files.clone(),
                },
                reason,
                facts(
                    t,
                    json!({
                        "repo": r.repo,
                        "state": r.state.as_str(),
                        "files": files,
                        "reason": r.reason,
                        "prepared_at": prep.at,
                    }),
                ),
                prio::INTEGRATION_BLOCKED,
            ));
        }
    }
}

/// Step 7, "Afterwards" (M9): the ticket branch has commits `uat` lacks after the recorded
/// merge, a fix after alpha. Integrating needs the ticket active and tested, so the step to
/// suggest is activating it again; once it is active [`integrate_ready`] takes over.
pub fn remerge_needed(s: &Snapshot, _now: i64, out: &mut Vec<Suggestion>) {
    for t in tracked(s) {
        if t.remerge_repos.is_empty()
            || !matches!(
                t.status(),
                Some(LocalStatus::Integrated | LocalStatus::Done | LocalStatus::Parked)
            )
            || s.active().is_some()
        {
            continue;
        }
        let action = if t.is_hotfix() {
            SuggestedAction::ChooseHotfixBaseline {
                ticket: t.key.clone(),
            }
        } else {
            SuggestedAction::Activate {
                ticket: t.key.clone(),
                baseline: None,
            }
        };
        out.push(Suggestion::new(
            Some(&t.key),
            RuleId::RemergeNeeded,
            "-",
            action,
            format!(
                "{} has commits in {} that uat does not have since it was merged: activate, test and integrate it again",
                t.label(),
                t.remerge_repos.join(", ")
            ),
            facts(
                t,
                json!({
                    "remerge_repos": t.remerge_repos,
                    "merges": t.merges.iter().map(|m| json!({ "repo": m.repo, "commit": m.commit })).collect::<Vec<_>>(),
                }),
            ),
            prio::REMERGE_NEEDED,
        ));
    }
}

/// Step 3/4, "Safety": the test overlay is still applied where it must not be: on a ticket
/// that is no longer active, or when the last prepare says an overlay is in the way.
pub fn revert_overlay(s: &Snapshot, _now: i64, out: &mut Vec<Suggestion>) {
    for t in tracked(s) {
        if t.overlays.is_empty() {
            continue;
        }
        let left = t.status() != Some(LocalStatus::Active);
        let in_the_way = t.prep.as_ref().is_some_and(|p| {
            p.repos.iter().any(|r| {
                r.state == PrepState::Blocked
                    && r.reason
                        .as_deref()
                        .is_some_and(|why| why.to_ascii_lowercase().contains("overlay"))
            })
        });
        if !left && !in_the_way {
            continue;
        }
        out.push(Suggestion::new(
            Some(&t.key),
            RuleId::RevertOverlay,
            "-",
            SuggestedAction::RevertOverlay {
                ticket: t.key.clone(),
                repos: t.overlays.clone(),
            },
            if left {
                format!(
                    "The test overlay of {} is still applied in {} although it is no longer active",
                    t.key,
                    t.overlays.join(", ")
                )
            } else {
                format!(
                    "The test overlay is in the way of integrating {} in {}: revert it",
                    t.key,
                    t.overlays.join(", ")
                )
            },
            facts(
                t,
                json!({
                    "repos": t.overlays,
                    "local_status": t.status().map(LocalStatus::as_str),
                }),
            ),
            prio::REVERT_OVERLAY,
        ));
    }
}

fn run_label(d: &DeployInfo) -> String {
    match (&d.run_number, &d.run_id) {
        (Some(n), _) => format!("pipeline #{n}"),
        (None, Some(id)) => format!("pipeline {id}"),
        _ => "pipeline".into(),
    }
}

/// Step 5, "Watch pipelines": pushed and a pipeline failed (view the log, or re-run it
/// through the gateway), or pushed and still running (an informational "waiting").
pub fn deploy_problems(s: &Snapshot, _now: i64, out: &mut Vec<Suggestion>) {
    for t in tracked(s) {
        if t.merges.is_empty() {
            continue;
        }
        let failed: Vec<&DeployInfo> = t
            .deploy
            .iter()
            .filter(|d| d.state == DeployState::Failed)
            .collect();
        for d in &failed {
            if let Some(url) = &d.run_url {
                out.push(Suggestion::new(
                    Some(&t.key),
                    RuleId::DeployFailed,
                    &format!("open:{}", d.repo),
                    SuggestedAction::OpenPipeline {
                        repo: d.hosting_repo.clone().unwrap_or_else(|| d.repo.clone()),
                        run_id: d.run_id.clone().unwrap_or_default(),
                        url: url.clone(),
                    },
                    format!(
                        "The deploy of {} in {} failed ({}): look at the step log",
                        t.key,
                        d.repo,
                        run_label(d)
                    ),
                    facts(
                        t,
                        json!({ "repo": d.repo, "run_id": d.run_id, "url": url, "state": "failed" }),
                    ),
                    prio::DEPLOY_FAILED,
                ));
            }
            if let (Some(host), Some(run_id)) = (&d.hosting_repo, &d.run_id) {
                out.push(Suggestion::new(
                    Some(&t.key),
                    RuleId::DeployFailed,
                    &format!("rerun:{}", d.repo),
                    SuggestedAction::Gateway(Action::RerunPipeline {
                        repo: host.clone(),
                        run_id: run_id.clone(),
                    }),
                    format!(
                        "Re-run the failed {} of {} in {}",
                        run_label(d),
                        t.key,
                        d.repo
                    ),
                    facts(
                        t,
                        json!({ "repo": d.repo, "hosting_repo": host, "run_id": run_id, "state": "failed" }),
                    ),
                    prio::DEPLOY_RERUN,
                ));
            }
        }
        if !failed.is_empty() {
            continue;
        }
        let waiting: Vec<&DeployInfo> = t
            .deploy
            .iter()
            .filter(|d| matches!(d.state, DeployState::Pending | DeployState::Running))
            .collect();
        if waiting.is_empty() {
            continue;
        }
        let described: Vec<String> = waiting
            .iter()
            .map(|d| format!("{} ({})", d.repo, d.state.as_str()))
            .collect();
        out.push(Suggestion::new(
            Some(&t.key),
            RuleId::DeployWaiting,
            "-",
            SuggestedAction::Waiting {
                ticket: t.key.clone(),
                what: format!("the deploy of {}", described.join(", ")),
            },
            format!(
                "{} is pushed to uat; waiting for the deploy of {}",
                t.key,
                described.join(", ")
            ),
            facts(
                t,
                json!({
                    "repos": waiting.iter().map(|d| json!({ "repo": d.repo, "state": d.state.as_str() })).collect::<Vec<_>>(),
                }),
            ),
            prio::DEPLOY_WAITING,
        ));
    }
}

/// Step 6, "Announce": every repo is deployed and no deploy comment is drafted or posted
/// for the newest merge yet. When some repos can never be followed (no hosting section) but
/// the rest deployed, a partial comment is suggested.
pub fn compose_deploy_comment(s: &Snapshot, _now: i64, out: &mut Vec<Suggestion>) {
    for t in tracked(s) {
        if t.merges.is_empty() || t.deploy.is_empty() || t.comment_draft.is_some() || announced(t) {
            continue;
        }
        let deployed = t
            .deploy
            .iter()
            .filter(|d| d.state == DeployState::Deployed)
            .count();
        let untracked = t
            .deploy
            .iter()
            .filter(|d| d.state == DeployState::Untracked)
            .count();
        let partial = if deployed == t.deploy.len() {
            false
        } else if deployed > 0 && deployed + untracked == t.deploy.len() {
            true
        } else {
            continue;
        };
        out.push(Suggestion::new(
            Some(&t.key),
            RuleId::ComposeDeployComment,
            "-",
            SuggestedAction::ComposeDeployComment { ticket: t.key.clone(), partial },
            if partial {
                format!(
                    "{} is deployed everywhere it can be followed ({untracked} repo(s) have no pipeline tracking): draft the deploy comment, marked partial",
                    t.key
                )
            } else {
                format!(
                    "Every repo of {} is deployed: draft the deploy comment",
                    t.key
                )
            },
            facts(
                t,
                json!({
                    "partial": partial,
                    "repos": t.deploy.iter().map(|d| json!({ "repo": d.repo, "state": d.state.as_str(), "run_id": d.run_id })).collect::<Vec<_>>(),
                    "latest_merge_at": latest_merge_at(t),
                }),
            ),
            prio::COMPOSE_DEPLOY_COMMENT,
        ));
    }
}

/// Step 6, "Announce": a drafted, unposted deploy comment. Posting it goes through the
/// gateway (preview, confirm, audit). A comment that was posted is never suggested again:
/// only an unposted draft counts.
pub fn post_deploy_comment(s: &Snapshot, _now: i64, out: &mut Vec<Suggestion>) {
    for t in tracked(s) {
        let Some(draft) = &t.comment_draft else {
            continue;
        };
        out.push(Suggestion::new(
            Some(&t.key),
            RuleId::PostDeployComment,
            "-",
            SuggestedAction::Gateway(Action::PostJiraComment {
                ticket: t.key.clone(),
                body: draft.body.clone(),
            }),
            format!(
                "The deploy comment for {} is drafted (draft {}): post it to Jira",
                t.key, draft.id
            ),
            // The same facts the announce flow records, so the audit trail matches.
            json!({ "draft_id": draft.id, "kind": "deploy_comment", "ticket": t.key }),
            prio::POST_DEPLOY_COMMENT,
        ));
    }
}

/// Step 6, "Announce": the comment is posted and the ticket is not moved to alpha yet.
/// Skipped when Jira already shows alpha, UAT or a done status.
pub fn transition_alpha(s: &Snapshot, _now: i64, out: &mut Vec<Suggestion>) {
    let past_alpha = |status: Option<&str>| {
        status_is(status, &s.statuses.alpha_testing)
            || status_is(status, &s.statuses.uat)
            || s.statuses.done.iter().any(|d| status_is(status, d))
    };
    for t in tracked(s) {
        let Some(comment_at) = t.comment_posted_at else {
            continue;
        };
        if t.transition_posted_at
            .is_some_and(|moved| moved >= comment_at)
            || past_alpha(t.jira_status.as_deref())
        {
            continue;
        }
        out.push(Suggestion::new(
            Some(&t.key),
            RuleId::TransitionAlpha,
            "-",
            SuggestedAction::Gateway(Action::TransitionJira {
                ticket: t.key.clone(),
                to_status: s.statuses.alpha_testing.clone(),
            }),
            format!(
                "The deploy comment of {} is posted: move it to {}",
                t.key, s.statuses.alpha_testing
            ),
            facts(
                t,
                json!({
                    "to_status": s.statuses.alpha_testing,
                    "comment_posted_at": comment_at,
                    "jira_status": t.jira_status,
                }),
            ),
            prio::TRANSITION_ALPHA,
        ));
    }
}

/// Step 1, "Appears": a Returned ticket is only relevant when you are @mentioned in a
/// comment newer than the last time the ticket was handled (your own comments do not count).
/// Surfaces the ticket with the comment as a fact; tracked or not.
pub fn returned_mention(s: &Snapshot, _now: i64, out: &mut Vec<Suggestion>) {
    for t in &s.tickets {
        if !status_is(t.jira_status.as_deref(), &s.statuses.returned) {
            continue;
        }
        let since = t.last_handled_at.unwrap_or(i64::MIN);
        let newest = t
            .mentions
            .iter()
            .filter(|m| m.created_at > since)
            .filter(|m| s.me.as_deref() != Some(m.author_account.as_str()))
            .max_by(|a, b| {
                a.created_at
                    .cmp(&b.created_at)
                    .then_with(|| a.comment_id.cmp(&b.comment_id))
            });
        let Some(m) = newest else { continue };
        let text: String = m.text.chars().take(MENTION_TEXT_MAX).collect();
        out.push(Suggestion::new(
            Some(&t.key),
            RuleId::ReturnedMention,
            "-",
            SuggestedAction::OpenTicket {
                ticket: t.key.clone(),
                url: t.url.clone(),
            },
            format!(
                "{} is {} and {} mentioned you: \"{}\"",
                t.label(),
                s.statuses.returned,
                m.author,
                text
            ),
            facts(
                t,
                json!({
                    "comment_id": m.comment_id,
                    "author": m.author,
                    "text": text,
                    "created_at": m.created_at,
                }),
            ),
            prio::RETURNED_MENTION,
        ));
    }
}

/// Step 7, "Afterwards" (M9): UAT is signed off (Jira shows the `uat` or a done status
/// after alpha), so each open PR you have not approved is offered for approval. This is the
/// only place approval is ever suggested. Never for your own PRs.
pub fn approve_prs(s: &Snapshot, _now: i64, out: &mut Vec<Suggestion>) {
    for t in tracked(s).filter(|t| t.testing_complete) {
        for p in t.open_prs() {
            let mine = s.me.as_deref().is_some_and(|me| p.pr.author == me);
            let approved =
                s.me.as_deref()
                    .is_some_and(|me| p.pr.reviewers.iter().any(|r| r.account == me && r.approved));
            if mine || approved {
                continue;
            }
            out.push(Suggestion::new(
                Some(&t.key),
                RuleId::ApprovePrs,
                &format!("{}#{}", p.pr.repo, p.pr.id),
                SuggestedAction::Gateway(Action::ApprovePr {
                    repo: p.pr.repo.clone(),
                    pr: p.pr.id,
                }),
                format!(
                    "{} passed testing (Jira: {}): approve {} #{}",
                    t.key,
                    t.jira_status.as_deref().unwrap_or("unknown"),
                    p.pr.repo,
                    p.pr.id
                ),
                facts(
                    t,
                    json!({
                        "repo": p.pr.repo,
                        "pr": p.pr.id,
                        "url": p.pr.url,
                        "jira_status": t.jira_status,
                    }),
                ),
                prio::APPROVE_PRS,
            ));
        }
    }
}
