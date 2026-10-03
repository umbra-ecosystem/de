//! The state changes: claim, review, activate, integrate, deploy, announce, approve, sync.
//!
//! A change of a ticket's stage always goes through a `Stage` transition; an illegal move becomes a refused
//! outcome instead of a silent assignment.

use super::Sim;
use super::data;
use super::model::*;
use crate::store::Outcome;
use crate::vm::{
    Baseline, Block, Branch, ClaimBlock, DraftId, Phase, PrNumber, RepoName, ReportRow, ReportVm,
    SimEvent, SuggestionId, Then, TicketKey, ToastKind, Undo, WaitKind,
};

fn phase_of(t: &Ticket) -> Option<Phase> {
    match &t.stage {
        Stage::InHand { phase, .. } => Some(*phase),
        _ => None,
    }
}

fn refuse(e: Illegal) -> Outcome {
    Outcome::fail(e.to_string())
}

impl Sim {
    /* ---------------------------- local lifecycle ---------------------------- */

    pub(crate) fn claim(&mut self, key: &TicketKey) -> Outcome {
        let Some(i) = self.idx(key) else {
            return Outcome::fail("Unknown ticket.");
        };
        if self.tickets[i].local().is_some() {
            return Outcome::fail("Already claimed.");
        }
        if let Some(b) = self.claim_block(key) {
            return Outcome::Blocked(b);
        }
        if let Err(e) = self.tickets[i].stage.claim() {
            return refuse(e);
        }
        self.ok("ticket.claim", Some(key), None, "");
        Outcome::ok().with_toast(
            format!("Claimed {key}"),
            ToastKind::Ok,
            Some(Undo::Claim(key.clone())),
        )
    }

    pub(crate) fn unclaim(&mut self, key: &TicketKey) {
        if let Some(i) = self.idx(key)
            && self.tickets[i].stage.unclaim().is_ok()
        {
            self.ok("undo.claim", Some(key), None, "");
        }
    }

    pub(crate) fn start_review(&mut self, key: &TicketKey) -> Outcome {
        let Some(i) = self.idx(key) else {
            return Outcome::fail("Unknown ticket.");
        };
        if self.tickets[i].local().is_none()
            && let Some(b) = self.claim_block(key)
        {
            return Outcome::Blocked(b);
        }
        if let Some(e) = self.gap_error(&self.tickets[i]) {
            return Outcome::fail(e);
        }
        if let Err(e) = self.tickets[i].stage.start_review() {
            return refuse(e);
        }
        self.ok("review.start", Some(key), None, "");
        Outcome::ok()
    }

    pub(crate) fn mark_reviewed(&mut self, key: &TicketKey) -> Outcome {
        let Some(i) = self.idx(key) else {
            return Outcome::fail("Unknown ticket.");
        };
        if let Some(e) = self.gap_error(&self.tickets[i]) {
            return Outcome::fail(e);
        }
        let Some(prev_phase) = phase_of(&self.tickets[i]) else {
            return Outcome::fail("Only a ticket in your hands can be marked reviewed.");
        };
        let prev = Undo::Reviewed {
            key: key.clone(),
            prev_mark: self.tickets[i].review_mark().map(|m| m.0),
            prev_phase,
        };
        let mark = ReviewMark(Self::latest_seq(&self.tickets[i]));
        if let Err(e) = self.tickets[i].stage.mark_reviewed(mark) {
            return refuse(e);
        }
        for p in &mut self.tickets[i].prs {
            p.since = None;
        }
        self.ok("review.mark_reviewed", Some(key), None, "");
        Outcome::ok().with_toast(format!("Marked {key} reviewed"), ToastKind::Ok, Some(prev))
    }

    pub(crate) fn unmark_reviewed(
        &mut self,
        key: &TicketKey,
        prev_mark: Option<u32>,
        prev_phase: Phase,
    ) {
        if let Some(i) = self.idx(key)
            && self.tickets[i]
                .stage
                .restore_review(prev_phase, prev_mark.map(ReviewMark))
                .is_ok()
        {
            self.ok("undo.mark_reviewed", Some(key), None, "");
        }
    }

    pub(crate) fn reclaim(&mut self, key: &TicketKey) -> Outcome {
        let Some(i) = self.idx(key) else {
            return Outcome::fail("Unknown ticket.");
        };
        if let Some(b) = self.claim_block(key) {
            return Outcome::Blocked(b);
        }
        if let Err(e) = self.tickets[i].stage.reclaim() {
            return refuse(e);
        }
        let t = &mut self.tickets[i];
        t.new_commits = false;
        t.pre.clear();
        t.pr_wait = None;
        t.th_wait = None;
        t.accepted.clear();
        t.conflict_sent = None;
        self.ok(
            "ticket.reclaim",
            Some(key),
            None,
            "back in Review, treated as new",
        );
        Outcome::ok().with_toast(format!("Claimed {key} again"), ToastKind::Ok, None)
    }

    pub(crate) fn toggle_checklist(&mut self, key: &TicketKey, index: usize) {
        if let Some(i) = self.idx(key)
            && let Some(c) = self.tickets[i].checklist.get_mut(index)
        {
            c.1 = !c.1;
        }
    }

    pub(crate) fn add_checklist(&mut self, key: &TicketKey, text: &str) {
        if let Some(i) = self.idx(key)
            && !text.trim().is_empty()
        {
            self.tickets[i]
                .checklist
                .push((text.trim().to_string(), false));
        }
    }

    pub(crate) fn choose(&mut self, key: &TicketKey, repo: &RepoName, branch: &Branch) {
        if let Some(i) = self.idx(key) {
            self.tickets[i].link.insert(repo.clone(), branch.clone());
            self.ok("links.choose", Some(key), Some(repo), branch);
        }
    }

    pub(crate) fn mark_seen(&mut self, key: &TicketKey) {
        if let Some(i) = self.idx(key) {
            self.tickets[i].seen_n = Self::max_n(&self.tickets[i]);
        }
    }

    /* ---------------------------- waits ---------------------------- */

    /// Note when a missing pull request or an unresolved review comment was **first seen**: that is the only thing
    /// stored. How long is left is worked out from it when it is read, so nothing runs on a timer and nothing can
    /// drift. It is not from when you claim or review: a ticket that has been in Review for days without a pull
    /// request has had its wait already. What is no longer missing or open is forgotten, so a later gap is new.
    ///
    /// Called when the data changes (a refresh, a command), never on a tick.
    pub(crate) fn observe(&mut self) {
        for i in 0..self.tickets.len() {
            let gap = self.pr_gap(&self.tickets[i]).is_some();
            let blocking = !Self::blocking_threads(&self.tickets[i]).is_empty();
            let (now, mins) = (self.ms, self.wait_min);
            // A missing pull request has been missing since the ticket was first seen in this status, which the
            // store remembers across restarts; without that, since it is first noticed now.
            let seen = self.ms_of(self.tickets[i].status_since).unwrap_or(now).min(now);
            let t = &mut self.tickets[i];
            t.pr_wait = match (gap, t.pr_wait) {
                (true, None) => Some(Wait { since: seen, mins }),
                (true, kept) => kept,
                (false, _) => None,
            };
            t.th_wait = match (blocking, t.th_wait) {
                (true, None) => Some(Wait { since: now, mins }),
                (true, kept) => kept,
                (false, _) => None,
            };
        }
    }

    pub(crate) fn start_th_wait(&mut self, i: usize, had_blocking: bool) {
        if Self::blocking_threads(&self.tickets[i]).is_empty() {
            return;
        }
        if !had_blocking || self.tickets[i].th_wait.is_none() {
            self.tickets[i].th_wait = Some(Wait {
                since: self.ms,
                mins: self.wait_min,
            });
        }
    }

    pub(crate) fn extend_wait(&mut self, key: &TicketKey, kind: WaitKind) -> Outcome {
        let Some(i) = self.idx(key) else {
            return Outcome::fail("Unknown ticket.");
        };
        let mins = self.wait_min;
        let ms = self.ms;
        let left = self.wait_left(match kind {
            WaitKind::PullRequest => self.tickets[i].pr_wait,
            WaitKind::Threads => self.tickets[i].th_wait,
        });
        let slot = match kind {
            WaitKind::PullRequest => &mut self.tickets[i].pr_wait,
            WaitKind::Threads => &mut self.tickets[i].th_wait,
        };
        match slot {
            Some(w) if left.is_some_and(|l| l > 0) => w.mins += mins,
            _ => *slot = Some(Wait { since: ms, mins }),
        }
        let action = match kind {
            WaitKind::PullRequest => "pr_wait.extend",
            WaitKind::Threads => "thread_wait.extend",
        };
        self.ok(action, Some(key), None, &format!("{mins} min"));
        Outcome::ok().with_toast(
            format!("Waiting {mins} more minutes"),
            ToastKind::Info,
            None,
        )
    }

    /* ---------------------------- activation ---------------------------- */

    pub(crate) fn activate(&mut self, key: &TicketKey, choice: Option<Baseline>) -> Outcome {
        let Some(i) = self.idx(key) else {
            return Outcome::fail("Unknown ticket.");
        };
        let t = self.tickets[i].clone();
        let Some(local) = t.local() else {
            return Outcome::fail("Claim the ticket first.");
        };
        if let Some(other) = self.active_key() {
            return Outcome::fail(format!(
                "{other} is already active. Park or finish it first."
            ));
        }
        if !matches!(
            local,
            Local::Claimed | Local::Reviewing | Local::Parked | Local::Integrated
        ) {
            return Outcome::fail(format!("A {} ticket cannot be activated.", local.word()));
        }
        if self.is_hotfix(&t) && choice.is_none() {
            return Outcome::NeedsBaseline;
        }
        let plan = self.activation_plan(&t, choice);
        if !plan.errors.is_empty() {
            return Outcome::Refused(plan.errors);
        }
        let repos: Vec<RepoName> = plan.rows.iter().map(|r| r.repo.clone()).collect();
        if let Some(b) = self.lock_busy(&repos, "activate", key) {
            return Outcome::Busy(b);
        }
        self.ok("activation.start", Some(key), None, "");
        let mut records = Vec::new();
        let mut report = Vec::new();
        for r in &plan.rows {
            let prev = self.ws.branches.get(&r.repo).cloned().unwrap_or_default();
            let stash = self.ws.dirty.contains(&r.repo);
            self.ws.branches.insert(r.repo.clone(), r.branch.clone());
            if stash {
                self.ws.dirty.remove(&r.repo);
            }
            let label = stash.then(|| format!("de:{key}:{}", r.repo));
            records.push(Record {
                repo: r.repo.clone(),
                ticket_role: r.ticket_role,
                branch: r.branch.clone(),
                prev: prev.clone(),
                stash,
                label: label.clone(),
            });
            let role = if r.ticket_role {
                "ticket branch"
            } else {
                "baseline"
            };
            self.ok(
                "activation.repo_switched",
                Some(key),
                Some(&r.repo),
                &format!("{role} {}", r.branch),
            );
            report.push(ReportRow {
                repo: r.repo.clone(),
                detail: format!(
                    "switched to {} ({role}){}",
                    r.branch,
                    label.map_or(String::new(), |l| format!(", stashed as \"{l}\""))
                ),
                ok: true,
            });
        }
        let mut notes = Vec::new();
        if let Some(o) = &plan.overlay {
            self.ok(
                "overlay.apply",
                Some(key),
                Some(&o.repo),
                &format!(
                    "composer path repository -> ../{}, constraint \"*\"",
                    o.provider
                ),
            );
            report.push(ReportRow {
                repo: o.repo.clone(),
                detail: format!(
                    "test overlay applied: composer path → ../{}, constraint \"*\"",
                    o.provider
                ),
                ok: true,
            });
            notes.push(
                "The overlay is reverted when you park or finish, and blocked by a guard if it reaches a commit."
                    .to_string(),
            );
        }
        let act = Activation {
            started_ms: self.ms,
            records,
            overlay: plan.overlay,
        };
        if let Err(e) = self.tickets[i].stage.activate(act) {
            return refuse(e);
        }
        self.tickets[i].test_from_n = Some(Self::max_n(&self.tickets[i]));
        self.ok("activation.complete", Some(key), None, "");
        Outcome::ok()
            .with_report(ReportVm {
                title: format!("Activated {key}"),
                rows: report,
                notes,
            })
            .with_toast(format!("{key} is active"), ToastKind::Ok, None)
    }

    /// Restore every repo and move the ticket on. `finalize` is the integration path, which skips the lock check.
    pub(crate) fn deactivate(
        &mut self,
        key: &TicketKey,
        after: After,
        finalize: bool,
    ) -> Result<ReportVm, Outcome> {
        let Some(i) = self.idx(key) else {
            return Err(Outcome::fail("Unknown ticket."));
        };
        let Some(act) = self.tickets[i].act().cloned() else {
            return Err(Outcome::fail(format!("Nothing to restore for {key}.")));
        };
        if !finalize {
            let repos: Vec<RepoName> = act.records.iter().map(|r| r.repo.clone()).collect();
            if let Some(b) = self.lock_busy(&repos, "park", key) {
                return Err(Outcome::Busy(b));
            }
        }
        let mut rows = Vec::new();
        if let Some(o) = &act.overlay {
            self.ok(
                "overlay.revert",
                Some(key),
                Some(&o.repo),
                "composer.json and composer.lock restored, composer install run",
            );
            rows.push(ReportRow {
                repo: o.repo.clone(),
                detail: "test overlay reverted, composer files restored byte for byte".to_string(),
                ok: true,
            });
        }
        for r in act.records.iter().rev() {
            self.ws.branches.insert(r.repo.clone(), r.prev.clone());
            if r.stash {
                self.ws.dirty.insert(r.repo.clone());
            }
            let detail = format!(
                "back on {}{}",
                r.prev,
                if r.stash { ", stash popped" } else { "" }
            );
            self.ok(
                "deactivation.repo_restored",
                Some(key),
                Some(&r.repo),
                &detail,
            );
            rows.push(ReportRow {
                repo: r.repo.clone(),
                detail,
                ok: true,
            });
        }
        let act = self.tickets[i].stage.finish_active(after).map_err(refuse)?;
        self.tickets[i].time_ms += self.ms - act.started_ms;
        self.ok(
            if finalize {
                "integration.finalize"
            } else {
                "deactivation.complete"
            },
            Some(key),
            None,
            "",
        );
        Ok(ReportVm {
            title: format!("Restored after {key}"),
            rows,
            notes: Vec::new(),
        })
    }

    pub(crate) fn park(&mut self, key: &TicketKey) -> Outcome {
        match self.deactivate(key, After::Park, false) {
            Ok(report) => Outcome::ok().with_report(report).with_toast(
                format!("Parked {key}"),
                ToastKind::Ok,
                None,
            ),
            Err(o) => o,
        }
    }

    /// Park whatever is in hand so another ticket can be taken.
    pub(crate) fn park_in_hand(&mut self, key: &TicketKey) -> Result<(), Outcome> {
        let Some(i) = self.idx(key) else {
            return Err(Outcome::fail("Unknown ticket."));
        };
        if self.tickets[i].act().is_some() {
            return self.deactivate(key, After::Park, false).map(|_| ());
        }
        self.tickets[i].stage.park_in_hand().map_err(refuse)?;
        self.ok(
            "ticket.park",
            Some(key),
            None,
            "parked to take another ticket",
        );
        Ok(())
    }

    pub(crate) fn park_and_continue(
        &mut self,
        from: &TicketKey,
        key: &TicketKey,
        then: Then,
    ) -> Outcome {
        if let Err(o) = self.park_in_hand(from) {
            return o;
        }
        let out = match then {
            Then::Reclaim => self.reclaim(key),
            Then::Claim => self.claim(key),
            Then::StartReview => {
                let c = self.claim(key);
                if c.is_done() {
                    self.start_review(key)
                } else {
                    c
                }
            }
        };
        if !out.is_done() {
            return out;
        }
        Outcome::ok().with_toast(format!("{from} parked, {key} claimed"), ToastKind::Ok, None)
    }

    pub(crate) fn claim_block(&self, key: &TicketKey) -> Option<ClaimBlock> {
        let b = self.in_hand(key)?;
        Some(ClaimBlock {
            blocker: b.key.clone(),
            title: b.title.clone(),
            what: match b.local() {
                Some(Local::Active) => "active (your repos are on its branches)",
                Some(Local::Reviewing) => "in review",
                _ => "claimed",
            }
            .to_string(),
            active: b.local() == Some(Local::Active),
        })
    }

    /* ---------------------------- integration ---------------------------- */

    fn set_prep(&mut self, i: usize, value: Option<Prep>) {
        if let Stage::Active { prep, .. } = &mut self.tickets[i].stage {
            *prep = value;
        }
    }

    pub(crate) fn prepare(&mut self, key: &TicketKey) -> Outcome {
        let Some(i) = self.idx(key) else {
            return Outcome::fail("Unknown ticket.");
        };
        let t = self.tickets[i].clone();
        if t.local() != Some(Local::Active) {
            return Outcome::fail(
                "The ticket must be active (tested locally) before it can be integrated.",
            );
        }
        let touched = self.touched(&t);
        let names: Vec<RepoName> = touched.iter().map(|p| p.repo.clone()).collect();
        if let Some(b) = self.lock_busy(&names, "prepare", key) {
            return Outcome::Busy(b);
        }
        let mut rows = Vec::new();
        for p in &touched {
            let repo = p.repo.clone();
            let outcome = if let Some(done) = t.landing(&repo) {
                PrepOutcome::AlreadyPushed {
                    commit: done.commit.clone(),
                }
            } else if self.sims.conflict.as_ref() == Some(&repo) {
                PrepOutcome::Conflict {
                    files: vec!["src/session.rs".to_string()],
                    with: "PROJ-127".into(),
                }
            } else if self.sims.leak.as_ref() == Some(&repo) {
                PrepOutcome::Blocked {
                    reason: format!(
                        "overlay leak: composer.json adds a path repository (commit {})",
                        Self::hex(&format!("leak{repo}"))
                    ),
                }
            } else {
                let files = t
                    .prs
                    .iter()
                    .filter(|x| x.repo == repo)
                    .map(|x| x.files.len() as u32)
                    .sum::<u32>();
                PrepOutcome::Ready {
                    branch: p.chosen.clone().unwrap_or_default(),
                    commits: 2 + files,
                    files,
                    uat_before: Self::hex(&format!("u{repo}{}", self.seq)),
                    merge: Self::hex(&format!("m{repo}{key}{}", self.seq)),
                    overlaps: self.overlaps_for(&t, &repo),
                }
            };
            rows.push(PrepItem { repo, outcome });
        }
        let all_ok = rows.iter().all(|r| {
            matches!(
                r.outcome,
                PrepOutcome::Ready { .. } | PrepOutcome::AlreadyPushed { .. }
            )
        });
        let summary = rows
            .iter()
            .map(|r| {
                let w = match r.outcome {
                    PrepOutcome::Ready { .. } => "ready",
                    PrepOutcome::AlreadyPushed { .. } => "alreadyPushed",
                    PrepOutcome::Conflict { .. } => "conflict",
                    PrepOutcome::Blocked { .. } => "blocked",
                };
                format!("{}:{w}", r.repo)
            })
            .collect::<Vec<_>>()
            .join(" ");
        self.audit(
            "integration.prepare",
            Some(key),
            None,
            if all_ok {
                AuditOutcome::Success
            } else {
                AuditOutcome::Failure
            },
            &summary,
        );
        let at = self.ms;
        self.set_prep(i, Some(Prep { at, rows }));
        if all_ok {
            Outcome::ok().with_toast("Merges prepared. Nothing was pushed.", ToastKind::Ok, None)
        } else {
            Outcome::ok().with_toast(
                "Preparation found a problem. Nothing was pushed.",
                ToastKind::Warn,
                None,
            )
        }
    }

    pub(crate) fn push(&mut self, key: &TicketKey) -> Outcome {
        let Some(i) = self.idx(key) else {
            return Outcome::fail("Unknown ticket.");
        };
        let t = self.tickets[i].clone();
        let Some(prep) = t.prep().cloned().filter(|_| self.prep_pushable(&t)) else {
            return Outcome::fail("Nothing is ready to push. Prepare the integration again.");
        };
        let names: Vec<RepoName> = prep.rows.iter().map(|r| r.repo.clone()).collect();
        if let Some(b) = self.lock_busy(&names, "push", key) {
            return Outcome::Busy(b);
        }
        let spec = prep
            .rows
            .iter()
            .map(|r| match &r.outcome {
                PrepOutcome::Ready { merge, .. } => format!("{} {merge}:refs/heads/uat", r.repo),
                PrepOutcome::AlreadyPushed { commit } => {
                    format!("{} {commit}:refs/heads/uat", r.repo)
                }
                _ => r.repo.to_string(),
            })
            .collect::<Vec<_>>()
            .join("; ");
        self.audit(
            "git.push_uat.attempted",
            Some(key),
            None,
            AuditOutcome::Skipped,
            &spec,
        );
        let mut rejected = false;
        for r in &prep.rows {
            let PrepOutcome::Ready {
                merge, uat_before, ..
            } = &r.outcome
            else {
                continue;
            };
            if self.sims.uat_moved {
                self.audit(
                    "git.push_uat.repo",
                    Some(key),
                    Some(&r.repo),
                    AuditOutcome::Failure,
                    "rejected: uat moved",
                );
                self.sims.uat_moved = false;
                self.set_prep(i, None);
                rejected = true;
                break;
            }
            let at = self.clock(None);
            self.run += 1;
            let landing = Landing {
                repo: r.repo.clone(),
                commit: merge.clone(),
                at,
                deploy: Deploy {
                    state: DeployState::Pending,
                    run: self.run,
                    since: self.ms,
                    step: "Queued".to_string(),
                    log: None,
                    uat_moved: None,
                    url: None,
                },
            };
            if let Some(h) = self.tickets[i].stage.held_mut() {
                h.shipped.landings.push(landing);
            }
            self.ok(
                "git.push_uat.repo",
                Some(key),
                Some(&r.repo),
                &format!("{uat_before} -> {merge}"),
            );
        }
        if rejected {
            self.audit(
                "git.push_uat",
                Some(key),
                None,
                AuditOutcome::Failure,
                "not every repo was pushed",
            );
            return Outcome::fail(
                "uat moved on the remote since you prepared. Nothing further was pushed. Prepare again.",
            );
        }
        let snapshot = self.tickets[i].clone();
        if self.pushed_all(&snapshot) {
            self.ok(
                "git.push_uat",
                Some(key),
                None,
                &format!("{} repos", prep.rows.len()),
            );
            let _ = self.deactivate(key, After::Integrate, true);
            Outcome::ok().with_toast(
                format!("Pushed {key} to uat. Actions are running."),
                ToastKind::Ok,
                None,
            )
        } else {
            self.audit(
                "git.push_uat",
                Some(key),
                None,
                AuditOutcome::Failure,
                "not every repo was pushed",
            );
            Outcome::fail("Not every repo was pushed.")
        }
    }

    pub(crate) fn rerun(&mut self, key: &TicketKey, repo: &RepoName) -> Outcome {
        let Some(i) = self.idx(key) else {
            return Outcome::fail("Unknown ticket.");
        };
        if !self.tickets[i].has_landed(repo) {
            return Outcome::fail("No run to re-run.");
        }
        self.run += 1;
        let run = self.run;
        let ms = self.ms;
        if let Some(d) = self.tickets[i].landing_mut(repo).map(|l| &mut l.deploy) {
            d.run = run;
            d.state = DeployState::Pending;
            d.since = ms;
            d.step = "Queued".to_string();
            d.log = None;
        }
        if self.sims.fail_deploy.as_ref() == Some(repo) {
            self.sims.fail_deploy = None;
        }
        self.ok(
            "github.actions_rerun",
            Some(key),
            Some(repo),
            &format!("new run #{run}"),
        );
        Outcome::ok().with_toast(
            format!("Re-running {repo} as #{run}"),
            ToastKind::Info,
            None,
        )
    }

    fn step_deploy(&mut self) {
        let ms = self.ms;
        let fail = self.sims.fail_deploy.clone();
        let mut raised: Vec<(String, ToastKind)> = Vec::new();
        for t in &mut self.tickets {
            let key = t.key.clone();
            let Some(held) = t.stage.held_mut() else {
                continue;
            };
            for l in &mut held.shipped.landings {
                let repo = l.repo.clone();
                let d = &mut l.deploy;
                if matches!(d.state, DeployState::Deployed | DeployState::Failed) {
                    continue;
                }
                let el = ms - d.since;
                if el < 3000 {
                    d.state = DeployState::Pending;
                    d.step = "Queued".to_string();
                } else if el < 8000 {
                    d.state = DeployState::Running;
                    d.step = if el < 5000 {
                        "Build"
                    } else if el < 6500 {
                        "Test"
                    } else {
                        "Deploy alpha"
                    }
                    .to_string();
                } else {
                    let failed = fail.as_ref() == Some(&repo);
                    d.state = if failed {
                        DeployState::Failed
                    } else {
                        DeployState::Deployed
                    };
                    d.step = "Deploy alpha".to_string();
                    if failed {
                        d.log = Some(vec![
                            format!("$ deploy --env alpha --repo {repo}"),
                            "build ok (41s)".to_string(),
                            "tests ok (312 passed)".to_string(),
                            format!(
                                "pushing image acme/{repo}:{}",
                                Self::hex(&format!("img{}", d.run))
                            ),
                            "rolling update: 1/3 ready".to_string(),
                            "ERROR health check failed: GET /healthz returned 503 for 60s"
                                .to_string(),
                            "step \"Deploy alpha\" exited with code 1".to_string(),
                        ]);
                    }
                    raised.push((
                        format!(
                            "{key} {repo} run #{} {}",
                            d.run,
                            if failed {
                                "failed"
                            } else {
                                "deployed to alpha"
                            }
                        ),
                        if failed {
                            ToastKind::Bad
                        } else {
                            ToastKind::Ok
                        },
                    ));
                }
            }
        }
        self.toasts.extend(raised);
    }

    pub(crate) fn advance(&mut self, ms: i64) {
        self.ms += ms;
        self.step_deploy();
        if self.sync.finish_at.is_some_and(|at| self.ms >= at) {
            self.sync_finish();
            // The refresh may have brought a pull request or a review thread.
            self.observe();
        }
    }

    /* ---------------------------- announce ---------------------------- */

    pub(crate) fn compose_draft(&mut self, key: &TicketKey) -> Outcome {
        let Some(i) = self.idx(key) else {
            return Outcome::fail("Unknown ticket.");
        };
        let t = self.tickets[i].clone();
        if !self.all_deployed(&t) {
            return Outcome::fail("Every touched repo must be deployed first.");
        }
        let lines: Vec<String> = t
            .landings()
            .iter()
            .map(|l| {
                let pr = t.prs.iter().find(|p| p.repo == l.repo);
                format!(
                    "• {}  PR #{} · run #{} · https://github.com/{}/actions/runs/{}",
                    l.repo,
                    pr.map_or("-".to_string(), |p| p.id.to_string()),
                    l.deploy.run,
                    self.repo(&l.repo).host,
                    l.deploy.run
                )
            })
            .collect();
        let mut parts = vec![format!("Deployed to alpha:\n{}", lines.join("\n"))];
        if !t.checklist.is_empty() {
            parts.push(format!(
                "Tested locally:\n{}",
                t.checklist
                    .iter()
                    .map(|(x, d)| format!("{} {x}", if *d { "☑" } else { "☐" }))
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
        }
        let acc = Self::accepted_threads(&t);
        if !acc.is_empty() {
            parts.push(format!(
                "Proceeded with unresolved review comments:\n{}",
                acc.iter()
                    .map(|x| format!("• {}", Self::thread_line(x)))
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
        }
        if let Some(from) = t.test_from_n {
            let during: Vec<String> = t
                .comments
                .iter()
                .filter(|c| c.n > from)
                .map(|c| {
                    format!(
                        "> {} ({}): {}",
                        c.who,
                        c.at,
                        c.body
                            .iter()
                            .map(super::queries::block_text)
                            .collect::<Vec<_>>()
                            .join(" ")
                            .replace('\n', " ")
                    )
                })
                .collect();
            if !during.is_empty() {
                parts.push(format!("Comments while testing:\n{}", during.join("\n")));
            }
        }
        let id = DraftId::new(format!("d{}", self.next_seq()));
        if let Some(h) = self.tickets[i].stage.held_mut() {
            h.shipped.drafts.push(Draft {
                id,
                body: parts.join("\n\n"),
                posted: false,
            });
        }
        self.ok("draft.compose", Some(key), None, "");
        Outcome::ok()
    }

    pub(crate) fn edit_draft(&mut self, key: &TicketKey, id: &DraftId, text: &str) {
        if let Some(i) = self.idx(key)
            && let Some(d) = self.tickets[i].draft_mut(id)
            && !d.posted
        {
            d.body = text.to_string();
        }
    }

    fn push_comment(&mut self, i: usize, body: &str) {
        let n = Self::max_n(&self.tickets[i]) + 1;
        let at = format!("today {}", self.clock(None));
        let t = &mut self.tickets[i];
        t.comments.push(Comment {
            n,
            who: ME.to_string(),
            at,
            body: body
                .split('\n')
                .map(|x| Block::Para(x.to_string()))
                .collect(),
        });
        t.seen_n = n;
    }

    pub(crate) fn post_comment(&mut self, key: &TicketKey, draft: &DraftId) -> Outcome {
        let Some(i) = self.idx(key) else {
            return Outcome::fail("Unknown ticket.");
        };
        let Some(d) = self.tickets[i].draft_mut(draft) else {
            return Outcome::fail("That draft was already posted.");
        };
        if d.posted {
            return Outcome::fail("That draft was already posted.");
        }
        d.posted = true;
        let body = d.body.clone();
        self.push_comment(i, &body);
        self.ok("jira.comment", Some(key), None, "deploy comment");
        Outcome::ok()
    }

    pub(crate) fn transition(&mut self, key: &TicketKey) -> Outcome {
        let Some(i) = self.idx(key) else {
            return Outcome::fail("Unknown ticket.");
        };
        if !Self::draft(&self.tickets[i]).is_some_and(|d| d.posted) {
            return Outcome::fail("Post the deploy comment first.");
        }
        self.tickets[i].jira = JiraStatus::AlphaTesting;
        self.ok(
            "jira.transition",
            Some(key),
            None,
            &format!(
                "{} -> {}",
                JiraStatus::InReview.label(),
                JiraStatus::AlphaTesting.label()
            ),
        );
        Outcome::ok().with_toast(
            format!("{key} moved to {}", JiraStatus::AlphaTesting.label()),
            ToastKind::Ok,
            None,
        )
    }

    pub(crate) fn post_and_move(&mut self, key: &TicketKey, draft: &DraftId) -> Outcome {
        let r = self.post_comment(key, draft);
        if !r.is_done() {
            return r;
        }
        if self.tk(key).is_some_and(|t| t.jira == JiraStatus::InReview) {
            return self.transition(key);
        }
        Outcome::ok().with_toast("Comment posted to Jira", ToastKind::Ok, None)
    }

    pub(crate) fn post_free_comment(&mut self, key: &TicketKey, text: &str) -> Outcome {
        let Some(i) = self.idx(key) else {
            return Outcome::fail("Unknown ticket.");
        };
        self.push_comment(i, text);
        self.ok("jira.comment", Some(key), None, "free comment");
        Outcome::ok().with_toast("Comment posted to Jira", ToastKind::Ok, None)
    }

    /* ---------------------------- review writes ---------------------------- */

    fn my_approved(p: &Pr) -> bool {
        p.reviewers.iter().any(|r| r.name == ME && r.approved)
    }

    pub(crate) fn approve(&mut self, key: &TicketKey, repo: &RepoName, pr: PrNumber) -> Outcome {
        let Some(i) = self.idx(key) else {
            return Outcome::fail("Unknown ticket.");
        };
        if !self.gh_ready {
            return Outcome::fail("gh is signed out.");
        }
        if let Some(p) = self.tickets[i]
            .prs
            .iter_mut()
            .find(|p| p.repo == *repo && p.id == pr)
            && let Some(me) = p.reviewers.iter_mut().find(|r| r.name == ME)
        {
            me.approved = true;
        }
        self.ok(
            "github.pr_approve",
            Some(key),
            Some(repo),
            &format!("#{pr}"),
        );
        if self.tickets[i].prs.iter().all(Self::my_approved)
            && self.tickets[i].stage.approved().is_ok()
        {
            self.ok("ticket.done", Some(key), None, "");
        }
        Outcome::ok().with_toast(format!("Approved {repo} #{pr}"), ToastKind::Ok, None)
    }

    pub(crate) fn approve_all(&mut self, key: &TicketKey) -> Outcome {
        let Some(t) = self.tk(key) else {
            return Outcome::fail("Unknown ticket.");
        };
        let pend: Vec<(RepoName, PrNumber)> = t
            .prs
            .iter()
            .filter(|p| !Self::my_approved(p))
            .map(|p| (p.repo.clone(), p.id))
            .collect();
        for (repo, id) in &pend {
            self.approve(key, repo, *id);
        }
        Outcome::ok().with_toast(
            format!("Approved {} pull requests", pend.len()),
            ToastKind::Ok,
            None,
        )
    }

    fn new_thread_id(&mut self) -> ThreadId {
        ThreadId(self.next_seq())
    }

    pub(crate) fn request_changes(&mut self, key: &TicketKey, pr: PrNumber, text: &str) -> Outcome {
        let Some(i) = self.idx(key) else {
            return Outcome::fail("Unknown ticket.");
        };
        let had = !Self::blocking_threads(&self.tickets[i]).is_empty();
        let id = self.new_thread_id();
        let mut repo = None;
        if let Some(p) = self.tickets[i].prs.iter_mut().find(|p| p.id == pr) {
            repo = Some(p.repo.clone());
            let file = p.files.first().map(|f| f.path.clone()).unwrap_or_default();
            p.threads.push(Thread {
                id,
                file,
                line: None,
                author: ME.to_string(),
                text: format!("Changes requested: {text}"),
                resolved: false,
            });
        }
        self.start_th_wait(i, had);
        self.ok(
            "github.pr_request_changes",
            Some(key),
            repo.as_ref(),
            &format!("#{pr}"),
        );
        Outcome::ok().with_toast("Requested changes", ToastKind::Ok, None)
    }

    pub(crate) fn add_thread(
        &mut self,
        key: &TicketKey,
        pr: PrNumber,
        file: &str,
        line: crate::vm::LineAnchor,
        text: &str,
    ) -> Outcome {
        let Some(i) = self.idx(key) else {
            return Outcome::fail("Unknown ticket.");
        };
        let had = !Self::blocking_threads(&self.tickets[i]).is_empty();
        let id = self.new_thread_id();
        let mut repo = None;
        if let Some(p) = self.tickets[i].prs.iter_mut().find(|p| p.id == pr) {
            repo = Some(p.repo.clone());
            p.threads.push(Thread {
                id,
                file: file.to_string(),
                line: Some(line),
                author: ME.to_string(),
                text: text.to_string(),
                resolved: false,
            });
        }
        self.start_th_wait(i, had);
        self.ok(
            "github.pr_comment",
            Some(key),
            repo.as_ref(),
            &format!("{file}:{}", line.line()),
        );
        Outcome::ok().with_toast("Comment posted on the pull request", ToastKind::Ok, None)
    }

    /* ---------------------------- returns and conflicts ---------------------------- */

    fn send_return(&mut self, key: &TicketKey, body: &str, note: &str) -> Outcome {
        let Some(i) = self.idx(key) else {
            return Outcome::fail("Unknown ticket.");
        };
        if self.tickets[i].act().is_some()
            && let Err(o) = self.deactivate(key, After::Park, false)
        {
            return o;
        }
        if let Err(e) = self.tickets[i].stage.returned() {
            return refuse(e);
        }
        self.push_comment(i, body);
        let t = &mut self.tickets[i];
        t.jira = JiraStatus::Returned;
        t.pr_wait = None;
        t.th_wait = None;
        self.ok(
            "jira.comment",
            Some(key),
            None,
            &format!("returned: {note}"),
        );
        self.ok(
            "jira.transition",
            Some(key),
            None,
            &format!(
                "{} -> {}",
                JiraStatus::InReview.label(),
                JiraStatus::Returned.label()
            ),
        );
        Outcome::ok().with_toast(
            format!("Returned {key} with a comment"),
            ToastKind::Ok,
            None,
        )
    }

    pub(crate) fn return_missing_pr(&mut self, key: &TicketKey) -> Outcome {
        let Some(t) = self.tk(key) else {
            return Outcome::fail("Unknown ticket.");
        };
        if self.pr_gap(t).is_none() {
            return Outcome::fail("A pull request exists now. Nothing was sent.");
        }
        let body = self.return_body(t);
        self.send_return(key, &body, "no pull request")
    }

    pub(crate) fn return_threads(&mut self, key: &TicketKey) -> Outcome {
        let Some(t) = self.tk(key) else {
            return Outcome::fail("Unknown ticket.");
        };
        if Self::blocking_threads(t).is_empty() {
            return Outcome::fail("Nothing is unresolved now. Nothing was sent.");
        }
        let body = self.return_threads_body(t);
        self.send_return(key, &body, "unresolved review comments")
    }

    pub(crate) fn accept_threads(&mut self, key: &TicketKey) -> Outcome {
        let Some(i) = self.idx(key) else {
            return Outcome::fail("Unknown ticket.");
        };
        let ids: Vec<ThreadId> = Self::blocking_threads(&self.tickets[i])
            .into_iter()
            .map(|x| x.id)
            .collect();
        let n = ids.len();
        self.tickets[i].accepted.extend(ids);
        self.ok(
            "threads.accept",
            Some(key),
            None,
            &format!(
                "{n} unresolved comment{} accepted",
                if n > 1 { "s" } else { "" }
            ),
        );
        Outcome::ok().with_toast("Proceeding with the open comments", ToastKind::Warn, None)
    }

    pub(crate) fn send_conflict(&mut self, key: &TicketKey, also_return: bool) -> Outcome {
        let Some(i) = self.idx(key) else {
            return Outcome::fail("Unknown ticket.");
        };
        if self.uat_conflicts(&self.tickets[i]).is_empty() {
            return Outcome::fail("No conflict with uat now. Nothing was sent.");
        }
        let body = self.conflict_body(&self.tickets[i]);
        let sig = self.conflict_sig(&self.tickets[i]);
        if also_return {
            return self.send_return(key, &body, "merge conflict with uat");
        }
        self.push_comment(i, &body);
        let at = self.clock(None);
        self.tickets[i].conflict_sent = Some((sig, at));
        self.ok("jira.comment", Some(key), None, "merge conflict with uat");
        Outcome::ok().with_toast("Comment sent to the developer", ToastKind::Ok, None)
    }

    pub(crate) fn break_lock(&mut self, repo: &RepoName) -> Outcome {
        match self.locks.get(repo) {
            Some(l) if matches!(l.kind, LockKind::Stale) => {
                self.locks.remove(repo);
                self.ok(
                    "lock.remove_stale",
                    None,
                    Some(repo),
                    ".git/index.lock removed after checking no git process runs",
                );
                Outcome::ok().with_toast(
                    format!("Removed the stale lock in {repo}"),
                    ToastKind::Ok,
                    None,
                )
            }
            _ => Outcome::fail("That lock is held by a running process, so it is not stale."),
        }
    }

    /* ---------------------------- sync ---------------------------- */

    pub(crate) fn sync_start(&mut self) -> Outcome {
        if self.sync.running {
            return Outcome::fail("A sync is already running.");
        }
        self.sync.running = true;
        self.sync.last_attempt = self.ms;
        self.sync.finish_at = Some(self.ms + 1500);
        Outcome::ok()
    }

    pub(crate) fn sync_finish(&mut self) {
        self.sync.running = false;
        self.sync.finish_at = None;
        if self.sims.offline {
            self.sync.last_error = Some("network unreachable".to_string());
            self.sync.report = vec![
                (
                    false,
                    "Jira (acli)".to_string(),
                    "network unreachable, cache kept".to_string(),
                ),
                (
                    false,
                    "GitHub (gh)".to_string(),
                    "network unreachable, cache kept".to_string(),
                ),
            ];
            self.audit("sync", None, None, AuditOutcome::Failure, "offline");
            self.toast(
                "Sync failed: network unreachable. The cache was kept.",
                ToastKind::Warn,
            );
            return;
        }
        let mut report = vec![(
            true,
            "Jira (acli)".to_string(),
            "pool refreshed".to_string(),
        )];
        if self.gh_ready {
            report.push((
                true,
                "GitHub (gh)".to_string(),
                "PRs and Actions runs refreshed".to_string(),
            ));
        } else {
            report.push((
                false,
                "GitHub (gh)".to_string(),
                "signed out, cache kept. Run gh auth login".to_string(),
            ));
        }
        if let Some(text) = self.deliver_arrival() {
            report.push((true, "Jira (acli)".to_string(), text.clone()));
            self.toast(text, ToastKind::Info);
        }
        self.sync.last_ok = self.ms;
        let failed = report.iter().any(|r| !r.0);
        self.sync.last_error = failed.then(|| "partial".to_string());
        let text = report
            .iter()
            .map(|r| r.2.clone())
            .collect::<Vec<_>>()
            .join("; ");
        self.sync.report = report;
        self.audit(
            "sync",
            None,
            None,
            if failed {
                AuditOutcome::Failure
            } else {
                AuditOutcome::Success
            },
            &text,
        );
    }

    /// The scripted "things that happen in the outside world" on successive syncs.
    fn deliver_arrival(&mut self) -> Option<String> {
        let text = match self.sync.script {
            0 => {
                self.tickets.push(data::arriving_ticket());
                "PROJ-155 appeared in the Review column"
            }
            1 => {
                let repos = self.repos.clone();
                let serial = self.seq as u32;
                if let Some(i) = self.idx(&"PROJ-163".into()) {
                    data::open_fake_prs(&mut self.tickets[i], &repos, serial);
                }
                "PROJ-163: a pull request was opened in web"
            }
            2 => {
                if let Some(i) = self.idx(&"PROJ-150".into()) {
                    self.tickets[i].comments.push(Comment {
                        n: 3,
                        who: "Marta Lind".to_string(),
                        at: "just now".to_string(),
                        body: vec![Block::Para(
                            "@[you] can you check the header on iPad landscape too?".to_string(),
                        )],
                    });
                }
                "PROJ-150 got a new comment that mentions you"
            }
            _ => return None,
        };
        self.sync.script += 1;
        Some(text.to_string())
    }

    /* ---------------------------- responses ---------------------------- */

    pub(crate) fn dismiss(&mut self, id: &SuggestionId) -> Outcome {
        let sug = self.suggest().into_iter().find(|s| s.id == *id);
        let hash = sug.as_ref().map(|s| s.hash.clone()).unwrap_or_default();
        self.responses
            .insert(id.clone(), Response::Dismissed { hash });
        self.ok(
            "suggestion.dismiss",
            sug.and_then(|s| s.ticket).as_ref(),
            None,
            id,
        );
        Outcome::ok().with_toast(
            "Dismissed",
            ToastKind::Info,
            Some(Undo::Response(id.clone())),
        )
    }

    pub(crate) fn snooze(&mut self, id: &SuggestionId, minutes: u32) -> Outcome {
        self.responses.insert(
            id.clone(),
            Response::Snoozed {
                until: self.ms + i64::from(minutes) * MS_PER_MIN,
            },
        );
        self.ok("suggestion.snooze", None, None, &format!("{id} {minutes}m"));
        Outcome::ok().with_toast(
            format!("Snoozed for {minutes} min"),
            ToastKind::Info,
            Some(Undo::Response(id.clone())),
        )
    }

    /* ---------------------------- simulate ---------------------------- */

    pub(crate) fn simulate(&mut self, ev: &SimEvent) -> Outcome {
        let repos = self.repos.clone();
        match ev {
            SimEvent::Offline(on) => self.sims.offline = *on,
            SimEvent::Conflict(r) => self.sims.conflict = r.clone(),
            SimEvent::OverlayLeak(r) => self.sims.leak = r.clone(),
            SimEvent::UatMoves(on) => self.sims.uat_moved = *on,
            SimEvent::FailDeploy(r) => self.sims.fail_deploy = r.clone(),
            SimEvent::Mention(key) => {
                if let Some(i) = self.idx(key) {
                    let n = Self::max_n(&self.tickets[i]) + 1;
                    let at = format!("today {}", self.clock(None));
                    self.tickets[i].comments.push(Comment {
                        n,
                        who: "Jane Doe".to_string(),
                        at,
                        body: vec![Block::Para(
                            "@[you] can you look at this again? It still misbehaves on staging."
                                .to_string(),
                        )],
                    });
                    self.toast(
                        format!("New comment mentioning you on {key}"),
                        ToastKind::Info,
                    );
                }
            }
            SimEvent::SignOff(key) => {
                if let Some(i) = self.idx(key) {
                    self.tickets[i].jira = JiraStatus::Done;
                    self.toast(
                        format!("{key} moved to Done in Jira (signed off)"),
                        ToastKind::Ok,
                    );
                }
            }
            SimEvent::Returned(key) => {
                if let Some(i) = self.idx(key) {
                    let n = Self::max_n(&self.tickets[i]) + 1;
                    let at = format!("today {}", self.clock(None));
                    let t = &mut self.tickets[i];
                    t.jira = JiraStatus::Returned;
                    t.comments.push(Comment {
                        n,
                        who: "Jane Doe".to_string(),
                        at,
                        body: vec![Block::Para(
                            "Sent back. @[you] please re-check.".to_string(),
                        )],
                    });
                    self.toast(format!("{key} was returned in Jira"), ToastKind::Warn);
                }
            }
            SimEvent::BackToReview(key) => {
                if let Some(i) = self.idx(key) {
                    self.tickets[i].jira = JiraStatus::InReview;
                    self.toast(
                        format!("{key} is back in the Review column"),
                        ToastKind::Info,
                    );
                }
            }
            SimEvent::ExternalLock(repo) => {
                if matches!(self.locks.get(repo), Some(l) if matches!(l.kind, LockKind::External)) {
                    self.locks.remove(repo);
                } else {
                    self.locks.insert(
                        repo.clone(),
                        Lock {
                            by: "de CLI".to_string(),
                            kind: LockKind::External,
                            op: "de stop".to_string(),
                        },
                    );
                }
            }
            SimEvent::StaleLock(repo) => {
                if matches!(self.locks.get(repo), Some(l) if matches!(l.kind, LockKind::Stale)) {
                    self.locks.remove(repo);
                } else {
                    self.locks.insert(
                        repo.clone(),
                        Lock {
                            by: "git".to_string(),
                            kind: LockKind::Stale,
                            op: String::new(),
                        },
                    );
                }
            }
            SimEvent::PrArrives(key) => {
                let serial = self.seq as u32;
                if let Some(i) = self.idx(key) {
                    data::open_fake_prs(&mut self.tickets[i], &repos, serial);
                    self.toast(
                        format!("A pull request was opened for {key}"),
                        ToastKind::Ok,
                    );
                }
            }
            SimEvent::ResolveThreads(key) => {
                if let Some(i) = self.idx(key) {
                    for p in &mut self.tickets[i].prs {
                        for th in &mut p.threads {
                            th.resolved = true;
                        }
                    }
                    self.tickets[i].th_wait = None;
                    self.toast(
                        format!("The review comments on {key} were resolved"),
                        ToastKind::Ok,
                    );
                }
            }
            SimEvent::NewCommits(key) => {
                if let Some(i) = self.idx(key) {
                    for p in &mut self.tickets[i].prs {
                        p.updated_seq += 1;
                        let Some(f) = p.files.first() else { continue };
                        let line = |k, o, n, x: &str| Line {
                            k,
                            o,
                            n,
                            x: x.to_string(),
                        };
                        p.since = Some(SinceReview {
                            commit: Self::hex(&format!("c{}{}", p.id, p.updated_seq)),
                            msg: "Address review comments".to_string(),
                            files: vec![FileDiff {
                                path: f.path.clone(),
                                adds: 2,
                                dels: 1,
                                lines: vec![
                                    line(Kind::Ctx, Some(30), Some(30), "// existing code"),
                                    line(Kind::Del, Some(31), None, "// handle the empty case"),
                                    line(
                                        Kind::Add,
                                        None,
                                        Some(31),
                                        "// address review: log and handle the empty case",
                                    ),
                                    line(Kind::Add, None, Some(32), "// covered by the new test"),
                                    line(Kind::Ctx, Some(32), Some(33), "// existing code"),
                                ],
                            }],
                        });
                    }
                    self.tickets[i].new_commits = true;
                    self.toast(
                        format!("New commits pushed to the branches of {key}"),
                        ToastKind::Info,
                    );
                }
            }
            SimEvent::UatAfterPush(key) => {
                let who = UatMoved {
                    by: "Priya Nair".to_string(),
                    ticket: "PROJ-150".into(),
                    at: self.clock(None),
                };
                if let Some(i) = self.idx(key) {
                    if let Some(h) = self.tickets[i].stage.held_mut() {
                        for l in &mut h.shipped.landings {
                            l.deploy.uat_moved = Some(who.clone());
                        }
                    }
                    self.toast(
                        format!("Someone pushed to uat after {key}"),
                        ToastKind::Warn,
                    );
                }
            }
            SimEvent::SkipMinutes(m) => self.advance(i64::from(*m) * MS_PER_MIN),
            SimEvent::DockerDown(on) => self.sims.docker_down = *on,
        }
        Outcome::ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A re-merge (an integrated ticket activated again) that is returned must not lose its landings.
    #[test]
    fn returning_an_active_remerge_keeps_its_landings_and_restores_the_repos() {
        let mut sim = Sim::new();
        let key: TicketKey = "PROJ-127".into();
        let before = sim.ws.branches.clone();
        assert!(sim.activate(&key, None).is_done());
        assert_eq!(sim.tk(&key).unwrap().local(), Some(Local::Active));
        assert_ne!(
            sim.ws.branches, before,
            "repos switched to the ticket branches"
        );

        let out = sim.send_return(&key, "Sent back.", "test");
        assert!(out.is_done(), "{out:?}");
        let t = sim.tk(&key).unwrap();
        assert_eq!(t.jira, JiraStatus::Returned);
        assert_eq!(
            t.local(),
            Some(Local::Integrated),
            "back where it was, not forgotten"
        );
        assert_eq!(t.landings().len(), 2, "what it shipped is kept");
        assert!(t.drafts().iter().any(|d| d.posted));
        assert!(t.act().is_none());
        assert_eq!(sim.ws.branches, before, "the repos were restored first");
    }

    #[test]
    fn returning_a_ticket_that_never_shipped_makes_it_unclaimed() {
        let mut sim = Sim::new();
        let key: TicketKey = "PROJ-142".into();
        assert!(sim.mark_reviewed(&key).is_done());
        let out = sim.send_return(&key, "Sent back.", "test");
        assert!(out.is_done(), "{out:?}");
        assert_eq!(sim.tk(&key).unwrap().local(), None);
    }
}
