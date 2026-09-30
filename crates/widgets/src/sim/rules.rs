//! The next-action rules: pure over the world, ranked, each with a stated reason.
//! A simplified copy of the real engine in `de-core::next` (same rule names and bands, fewer inputs).

use super::Sim;
use super::model::*;
use crate::vm::{Command, Intent, Level, Route, SugState, TicketTab};

#[derive(Clone, Debug)]
pub struct Sug {
    pub id: String,
    pub ticket: Option<String>,
    pub rule: &'static str,
    pub title: String,
    pub reason: String,
    pub level: Level,
    pub info: bool,
    pub hotfix: bool,
    /// The tool a write needs: `acli` (Jira) or `gh` (GitHub).
    pub needs: Option<&'static str>,
    pub act: Intent,
    pub alt: Option<(String, Intent)>,
    pub hash: String,
    pub priority: i32,
    pub rank: usize,
    pub state: SugState,
}

fn band(rule: &str) -> i32 {
    match rule {
        "repo_stale" => 400,
        "stale_lock" => 880,
        "threads_return" => 855,
        "threads_waiting" => 545,
        "pr_missing_return" => 860,
        "pr_waiting" => 550,
        "uat_conflict" => 900,
        "uat_moved" => 700,
        "prepush_checks" => 678,
        "deploy_failed" => 1000,
        "integration_blocked" => 990,
        "returned_to_review" => 850,
        "returned_mention" => 840,
        "re_review" => 830,
        "start_review" => 820,
        "finish_review" => 810,
        "approve_prs" => 690,
        "integrate_ready" => 680,
        "push_ready" => 675,
        "compose_deploy_comment" => 670,
        "post_deploy_comment" => 665,
        "transition_alpha" => 660,
        "remerge_needed" => 645,
        "activate_reviewed" => 640,
        "park_active" => 620,
        "deploy_waiting" => 600,
        "claim_new" => 480,
        "adapter_unavailable" => 299,
        "sync_stale" => 1200,
        _ => 100,
    }
}

/// Rules that mean something is broken, someone is waiting, or a hotfix is in play.
const ATTENTION: [&str; 11] = [
    "stale_lock",
    "threads_return",
    "pr_missing_return",
    "uat_conflict",
    "deploy_failed",
    "integration_blocked",
    "returned_mention",
    "returned_to_review",
    "re_review",
    "approve_prs",
    "adapter_unavailable",
];

fn cmd(c: Command) -> Intent {
    Intent::Do(c)
}

fn open(key: &str, tab: TicketTab) -> Intent {
    Intent::Go(Route::ticket(key, tab))
}

fn plural(n: usize) -> &'static str {
    if n > 1 { "s" } else { "" }
}

impl Sim {
    fn base_sug(
        rule: &'static str,
        id: String,
        title: String,
        reason: String,
        act: Intent,
        hash: String,
    ) -> Sug {
        Sug {
            id,
            ticket: None,
            rule,
            title,
            reason,
            level: Level::Local,
            info: false,
            hotfix: false,
            needs: None,
            act,
            alt: None,
            hash,
            priority: 0,
            rank: 0,
            state: SugState::Open,
        }
    }

    /// All suggestions, ranked, with their response state. Hidden ones are included.
    pub(crate) fn suggest(&self) -> Vec<Sug> {
        let mut out: Vec<Sug> = Vec::new();
        let act = self.active_key();
        let mut push = |mut s: Sug, rank_hint: Option<usize>| {
            s.priority = band(s.rule) - rank_hint.map_or(0, |r| r as i32 * 10);
            out.push(s);
        };

        // sync
        let age = self.mins_ago(self.sync.last_ok);
        let since_attempt = self.mins_ago(self.sync.last_attempt);
        if !self.sync.running
            && ((self.sync.last_error.is_some() && since_attempt >= 15)
                || (self.sync.last_error.is_none() && age >= 6 && since_attempt >= 1))
        {
            let mut s = Self::base_sug(
                "sync_stale",
                "-:sync_stale:all".to_string(),
                "Sync now".to_string(),
                format!("Data was last refreshed {age} min ago."),
                cmd(Command::Sync),
                "sync".to_string(),
            );
            s.level = Level::Automatic;
            push(s, None);
        }

        for (repo, l) in &self.locks {
            if matches!(l.kind, LockKind::Stale) {
                push(
                    Self::base_sug(
                        "stale_lock",
                        format!("-:stale_lock:{repo}"),
                        format!("Stale git lock in {repo}"),
                        format!(
                            "{repo} has a leftover .git/index.lock and no git process is running, so nothing can change in that repo. Remove the lock."
                        ),
                        cmd(Command::BreakLock(repo.clone())),
                        format!("sl{repo}"),
                    ),
                    None,
                );
            }
        }

        for t in &self.tickets {
            let key = t.key.as_str();
            let st = t.local;
            let hot = self.is_hotfix(t);
            let mk = |rule: &'static str,
                      id: String,
                      title: String,
                      reason: String,
                      act: Intent,
                      hash: String| {
                let mut s = Self::base_sug(rule, id, title, reason, act, hash);
                s.ticket = Some(key.to_string());
                s.hotfix = hot;
                s
            };

            // mentions
            let ms = Self::unseen_mentions(t);
            if !ms.is_empty()
                && matches!(
                    st,
                    None | Some(
                        Local::Integrated
                            | Local::Done
                            | Local::Claimed
                            | Local::Reviewing
                            | Local::Parked
                            | Local::Active
                    )
                )
            {
                let last = ms[ms.len() - 1];
                let mut s = mk(
                    "returned_mention",
                    format!("{key}:returned_mention"),
                    if t.jira == JIRA_RETURNED {
                        "Returned: you were mentioned".to_string()
                    } else {
                        "You were mentioned".to_string()
                    },
                    format!(
                        "{} mentioned you on {key} ({} unseen comment{}).",
                        last.who,
                        ms.len(),
                        plural(ms.len())
                    ),
                    open(key, TicketTab::Overview),
                    format!("m{}", ms.len()),
                );
                s.info = true;
                push(s, None);
            }
            if t.jira == JIRA_REVIEW
                && matches!(st, Some(Local::Integrated | Local::Done))
                && !t.new_commits
                && Self::draft(t).is_some_and(|d| d.posted)
                && !t.merges.is_empty()
            {
                push(
                    mk(
                        "returned_to_review",
                        format!("{key}:returned_to_review"),
                        "Back in the Review column".to_string(),
                        format!(
                            "{key} returned to Review after alpha. Treat it as new: claim it again."
                        ),
                        cmd(Command::Reclaim(key.to_string())),
                        "rr".to_string(),
                    ),
                    None,
                );
            }

            if self.conflict_sent(t).is_none() {
                for (repo, with, files) in self.uat_conflicts(t) {
                    let mut s = mk(
                        "uat_conflict",
                        format!("{key}:uat_conflict:{repo}"),
                        format!("Conflict with uat in {repo}"),
                        format!(
                            "{key} will not merge into uat: it conflicts with {with} in {}. Send the developer a prepared comment, and return it if you like.",
                            files.join(", ")
                        ),
                        cmd(Command::ConflictComment {
                            key: key.to_string(),
                            also_return: false,
                        }),
                        format!("uc{with}"),
                    );
                    s.level = Level::External;
                    s.needs = Some("acli");
                    push(s, None);
                }
            }

            if st.is_some() && self.pr_gap(t).is_some() {
                let left = self.wait_left(t.pr_wait);
                if let (Some(w), Some(l)) = (t.pr_wait, left)
                    && l <= 0
                {
                    let mut s = mk(
                        "pr_missing_return",
                        format!("{key}:pr_return"),
                        format!("Return {key}: no pull request after {} min", w.mins),
                        format!(
                            "{} Review cannot start. Return it with a comment that explains.",
                            self.gap_text(t)
                        ),
                        cmd(Command::ReturnMissingPr(key.to_string())),
                        format!("pr{}:{}", w.since, w.mins),
                    );
                    s.level = Level::External;
                    s.needs = Some("acli");
                    push(s, None);
                } else {
                    let mins = left.map_or(0, |l| {
                        (l + super::model::MS_PER_MIN - 1) / super::model::MS_PER_MIN
                    });
                    let mut s = mk(
                        "pr_waiting",
                        format!("{key}:pr_waiting"),
                        "Waiting for the pull request".to_string(),
                        format!(
                            "{} {mins} min left. It clears itself when the PR appears.",
                            self.gap_text(t)
                        ),
                        open(key, TicketTab::Overview),
                        "pw".to_string(),
                    );
                    s.info = true;
                    s.level = Level::Automatic;
                    push(s, None);
                }
                continue;
            }

            let bt = Self::blocking_threads(t);
            if !bt.is_empty() && t.reviewed && st.is_some() {
                let left = self.wait_left(t.th_wait);
                let n = bt.len();
                if let (Some(w), Some(l)) = (t.th_wait, left)
                    && l <= 0
                {
                    let mut s = mk(
                        "threads_return",
                        format!("{key}:threads_return"),
                        format!("Return {key}: {n} unresolved comment{} after {} min", plural(n), w.mins),
                        "The review comments are still open, so activation is blocked. Return it with a comment that lists them, or proceed anyway.".to_string(),
                        cmd(Command::ReturnThreads(key.to_string())),
                        format!("tr{}:{}:{n}", w.since, w.mins),
                    );
                    s.level = Level::External;
                    s.needs = Some("acli");
                    s.alt = Some((
                        "Proceed anyway…".to_string(),
                        cmd(Command::AcceptThreads(key.to_string())),
                    ));
                    push(s, None);
                } else {
                    let wait = left.map_or(String::new(), |l| {
                        format!("{} min left. ", (l + MS_PER_MIN - 1) / MS_PER_MIN)
                    });
                    let mut s = mk(
                        "threads_waiting",
                        format!("{key}:threads_waiting"),
                        format!("Waiting for {n} comment{} to be resolved", plural(n)),
                        format!(
                            "Activation is blocked until they are. {wait}It clears itself when they are resolved."
                        ),
                        open(key, TicketTab::Overview),
                        format!("tw{n}"),
                    );
                    s.info = true;
                    s.level = Level::Automatic;
                    s.alt = Some((
                        "Proceed anyway…".to_string(),
                        cmd(Command::AcceptThreads(key.to_string())),
                    ));
                    push(s, None);
                }
                continue;
            }

            if st.is_none() && t.jira == JIRA_REVIEW {
                // unclaimed: handled by the claim queue below
            } else {
                if st == Some(Local::Claimed) {
                    push(
                        mk(
                            "start_review",
                            format!("{key}:start_review"),
                            "Start the review".to_string(),
                            format!(
                                "{key} is claimed. Read its {} PR{} against {}.",
                                t.prs.len(),
                                plural(t.prs.len()),
                                {
                                    let mut d: Vec<&str> =
                                        t.prs.iter().map(|p| p.dst.as_str()).collect();
                                    d.dedup();
                                    d.join(", ")
                                }
                            ),
                            cmd(Command::StartReview(key.to_string())),
                            "s".to_string(),
                        ),
                        None,
                    );
                }
                if st == Some(Local::Reviewing) && !t.reviewed {
                    push(
                        mk(
                            "finish_review",
                            format!("{key}:finish_review"),
                            "Finish the review".to_string(),
                            "Review in progress. Mark it reviewed when the diffs and comments are done.".to_string(),
                            open(key, TicketTab::Review),
                            "f".to_string(),
                        ),
                        None,
                    );
                }
                if Self::stale_review(t)
                    && matches!(st, Some(Local::Reviewing | Local::Parked | Local::Claimed))
                {
                    let seq = t.prs.iter().map(|p| p.updated_seq).max().unwrap_or(0);
                    push(
                        mk(
                            "re_review",
                            format!("{key}:re_review"),
                            "Re-review: new commits".to_string(),
                            format!("The branches moved since you reviewed {key}."),
                            cmd(Command::StartReview(key.to_string())),
                            format!("rv{seq}"),
                        ),
                        None,
                    );
                }

                if act.is_none()
                    && t.reviewed
                    && matches!(st, Some(Local::Reviewing | Local::Parked))
                    && t.jira != JIRA_RETURNED
                {
                    push(
                        mk(
                            "activate_reviewed",
                            format!("{key}:activate"),
                            if hot {
                                "Activate hotfix (choose baseline)".to_string()
                            } else {
                                "Activate for local test".to_string()
                            },
                            format!(
                                "Reviewed{}. Nothing is active. This switches the touched repos to the ticket branches and stashes local changes.",
                                if st == Some(Local::Parked) {
                                    " and parked"
                                } else {
                                    ""
                                }
                            ),
                            cmd(Command::Activate {
                                key: key.to_string(),
                                baseline: None,
                            }),
                            "a".to_string(),
                        ),
                        None,
                    );
                }
                if let Some(a) = &act
                    && a != key
                    && hot
                    && t.reviewed
                    && matches!(st, Some(Local::Reviewing | Local::Parked))
                {
                    let mut s = Self::base_sug(
                        "park_active",
                        format!("{a}:park_active:{key}"),
                        format!("Park {a} for the hotfix {key}"),
                        format!(
                            "{key} is a hotfix waiting to be tested. Parking restores the repos and reverts the overlay."
                        ),
                        cmd(Command::Park(a.clone())),
                        "pa".to_string(),
                    );
                    s.ticket = Some(a.clone());
                    s.hotfix = true;
                    push(s, None);
                }

                if st == Some(Local::Active) {
                    let done = !t.checklist.is_empty() && t.checklist.iter().all(|c| c.1);
                    let pushed = self.pushed_all(t);
                    let touched = self.touched(t).len();
                    let blocked = t.prep.as_ref().is_some_and(|p| {
                        p.rows.iter().any(|r| {
                            matches!(
                                r.outcome,
                                PrepOutcome::Conflict { .. } | PrepOutcome::Blocked { .. }
                            )
                        })
                    });
                    if done && t.prep.is_none() && !pushed {
                        push(
                            mk(
                                "integrate_ready",
                                format!("{key}:integrate_ready"),
                                "Prepare integration to uat".to_string(),
                                format!(
                                    "Checklist complete ({} of {}). {touched} repo{} touched. Preparing pushes nothing.",
                                    t.checklist.len(),
                                    t.checklist.len(),
                                    plural(touched)
                                ),
                                cmd(Command::Prepare(key.to_string())),
                                "i".to_string(),
                            ),
                            None,
                        );
                    }
                    let missing = self.prepush_missing(t);
                    if Self::prep_ready(t) && !missing.is_empty() {
                        push(
                            mk(
                                "prepush_checks",
                                format!("{key}:prepush"),
                                "Tick the pre-push checks".to_string(),
                                format!(
                                    "{} check{} left before the push unlocks: {}.",
                                    missing.len(),
                                    plural(missing.len()),
                                    missing
                                        .iter()
                                        .map(|(r, x)| format!("{r}: {x}"))
                                        .collect::<Vec<_>>()
                                        .join(", ")
                                ),
                                open(key, TicketTab::Ship),
                                format!("pp{}", missing.len()),
                            ),
                            None,
                        );
                    }
                    if let Some(prep) = &t.prep
                        && self.prep_pushable(t)
                    {
                        let ready: Vec<&str> = prep
                            .rows
                            .iter()
                            .filter(|r| matches!(r.outcome, PrepOutcome::Ready { .. }))
                            .map(|r| r.repo.as_str())
                            .collect();
                        let mut s = mk(
                            "push_ready",
                            format!("{key}:push_ready"),
                            "Review and push to uat".to_string(),
                            format!(
                                "Merge ready in {}. Pushing deploys to alpha.",
                                ready.join(", ")
                            ),
                            cmd(Command::PushUat(key.to_string())),
                            format!("p{}", prep.at),
                        );
                        s.level = Level::External;
                        push(s, None);
                    }
                    if blocked && let Some(prep) = &t.prep {
                        for r in &prep.rows {
                            let (title, reason, tag) = match &r.outcome {
                                PrepOutcome::Conflict { files, with } => (
                                    format!("Resolve the uat conflict in {}", r.repo),
                                    format!(
                                        "Merging into uat conflicts with {with} in {}. Nothing was pushed.",
                                        files.join(", ")
                                    ),
                                    "conflict",
                                ),
                                PrepOutcome::Blocked { reason } => (
                                    format!("Integration blocked in {}", r.repo),
                                    reason.clone(),
                                    "blocked",
                                ),
                                _ => continue,
                            };
                            let mut s = mk(
                                "integration_blocked",
                                format!("{key}:blocked:{}", r.repo),
                                title,
                                reason,
                                open(key, TicketTab::Ship),
                                format!("{tag}{}", r.repo),
                            );
                            s.info = true;
                            push(s, None);
                        }
                    }
                }

                let sent_back = t.jira == JIRA_RETURNED;
                let mut skip_rest = false;
                if !t.merges.is_empty() {
                    for repo in Self::failed_repos(t) {
                        let d = &t.deploy[&repo];
                        let mut s = mk(
                            "deploy_failed",
                            format!("{key}:deploy_failed:{repo}"),
                            format!("Deploy failed in {repo}"),
                            format!(
                                "The deploy step of run #{} failed after your push to uat.",
                                d.run
                            ),
                            cmd(Command::Rerun {
                                key: key.to_string(),
                                repo: repo.clone(),
                            }),
                            format!("df{}", d.run),
                        );
                        s.level = Level::External;
                        s.needs = Some("gh");
                        push(s, None);
                    }
                    if sent_back {
                        skip_rest = true;
                    } else {
                        let moved = t
                            .merges
                            .iter()
                            .find_map(|m| t.deploy.get(&m.repo).and_then(|d| d.uat_moved.clone()));
                        if let Some(u) = moved
                            && !Self::is_signed_off(t)
                        {
                            let mut s = mk(
                                "uat_moved",
                                format!("{key}:uat_moved"),
                                "uat moved after your push".to_string(),
                                format!(
                                    "{} ({}) was pushed to uat at {}. Alpha now has both, so check the combined behaviour.",
                                    u.ticket, u.by, u.at
                                ),
                                open(key, TicketTab::Ship),
                                format!("um{}", u.at),
                            );
                            s.info = true;
                            push(s, None);
                        }
                        if Self::any_running(t) && Self::failed_repos(t).is_empty() {
                            let running: Vec<String> = t
                                .merges
                                .iter()
                                .filter_map(|m| {
                                    let d = t.deploy.get(&m.repo)?;
                                    matches!(d.state, DeployState::Pending | DeployState::Running)
                                        .then(|| format!("{} #{} ({})", m.repo, d.run, d.step))
                                })
                                .collect();
                            let mut s = mk(
                                "deploy_waiting",
                                format!("{key}:deploy_waiting"),
                                "Waiting for Actions runs".to_string(),
                                format!("{}.", running.join(", ")),
                                open(key, TicketTab::Ship),
                                "w".to_string(),
                            );
                            s.info = true;
                            s.level = Level::Automatic;
                            push(s, None);
                        }
                        let d = Self::draft(t);
                        if self.all_deployed(t) && d.is_none() {
                            push(
                                mk(
                                    "compose_deploy_comment",
                                    format!("{key}:compose"),
                                    "Draft the deploy comment".to_string(),
                                    "Every touched repo is deployed to alpha. The draft lists each repo with its PR and Actions run.".to_string(),
                                    cmd(Command::ComposeDraft(key.to_string())),
                                    "c".to_string(),
                                ),
                                None,
                            );
                        }
                        if let Some(d) = d {
                            if !d.posted {
                                let mut s = mk(
                                    "post_deploy_comment",
                                    format!("{key}:post"),
                                    format!("Post the deploy comment and move to {JIRA_ALPHA}"),
                                    "A draft is ready with the deploy, your checklist and the comments from testing. Edit it, then post; the ticket moves in the same step.".to_string(),
                                    cmd(Command::PostAndMove { key: key.to_string(), draft: d.id.clone() }),
                                    format!("pc{}", d.id),
                                );
                                s.level = Level::External;
                                s.needs = Some("acli");
                                push(s, None);
                            } else if t.jira == JIRA_REVIEW {
                                let mut s = mk(
                                    "transition_alpha",
                                    format!("{key}:transition"),
                                    format!("Move to {JIRA_ALPHA}"),
                                    format!(
                                        "The comment is posted but the ticket did not move. Transition {JIRA_REVIEW} → {JIRA_ALPHA}."
                                    ),
                                    cmd(Command::Transition(key.to_string())),
                                    "t".to_string(),
                                );
                                s.level = Level::External;
                                s.needs = Some("acli");
                                push(s, None);
                            }
                        }
                    }
                }
                if skip_rest {
                    // a returned ticket is only your business when mentioned or failing
                } else {
                    if st == Some(Local::Integrated) && t.new_commits && act.is_none() {
                        push(
                            mk(
                                "remerge_needed",
                                format!("{key}:remerge"),
                                if hot {
                                    "Re-merge needed (choose baseline)".to_string()
                                } else {
                                    "Re-merge needed".to_string()
                                },
                                "The ticket branches have commits that uat lacks after your recorded merge. Activate it again, test, and integrate again.".to_string(),
                                cmd(Command::Activate { key: key.to_string(), baseline: None }),
                                "rm".to_string(),
                            ),
                            None,
                        );
                    }
                    if st == Some(Local::Integrated) && Self::is_signed_off(t) {
                        let pend: Vec<&Pr> = t
                            .prs
                            .iter()
                            .filter(|p| !p.reviewers.iter().any(|r| r.name == ME && r.approved))
                            .collect();
                        if pend.len() > 1 {
                            let mut s = mk(
                                "approve_prs",
                                format!("{key}:approve:all"),
                                format!("Approve {} pull requests", pend.len()),
                                format!(
                                    "{key} passed testing (Jira: {}). {}. Approval is the last gate.",
                                    t.jira,
                                    pend.iter()
                                        .map(|p| format!("{} #{}", p.repo, p.id))
                                        .collect::<Vec<_>>()
                                        .join(", ")
                                ),
                                cmd(Command::ApproveAll(key.to_string())),
                                format!("apa{}", pend.len()),
                            );
                            s.level = Level::External;
                            s.needs = Some("gh");
                            push(s, None);
                        } else {
                            for p in pend {
                                let mut s = mk(
                                    "approve_prs",
                                    format!("{key}:approve:{}#{}", p.repo, p.id),
                                    format!("Approve {} #{}", self.repo(&p.repo).host, p.id),
                                    format!(
                                        "{key} passed testing (Jira: {}). Approval is the last gate.",
                                        t.jira
                                    ),
                                    cmd(Command::Approve {
                                        key: key.to_string(),
                                        repo: p.repo.clone(),
                                        pr: p.id,
                                    }),
                                    format!("ap{}", p.id),
                                );
                                s.level = Level::External;
                                s.needs = Some("gh");
                                push(s, None);
                            }
                        }
                    }
                }
            }
        }

        // claim queue
        let mut pool: Vec<&Ticket> = self
            .tickets
            .iter()
            .filter(|t| t.local.is_none() && t.jira == JIRA_REVIEW)
            .collect();
        pool.sort_by(|a, b| {
            a.priority
                .rank()
                .cmp(&b.priority.rank())
                .then_with(|| natural_key(&a.key).cmp(&natural_key(&b.key)))
        });
        let eligible: Vec<&&Ticket> = pool
            .iter()
            .filter(|t| self.in_hand(&t.key).is_none() || self.is_hotfix(t))
            .collect();
        for (i, t) in eligible.iter().enumerate() {
            let hot = self.is_hotfix(t);
            let mut s = Self::base_sug(
                "claim_new",
                format!("{}:claim_new", t.key),
                format!("Claim {}", t.key),
                format!(
                    "In the Review column, priority {}{}{}, #{} in your queue.",
                    t.priority.label(),
                    if hot {
                        ", targets production (hotfix)"
                    } else {
                        ""
                    },
                    if self.pr_gap(t).is_some() {
                        ", but it has no pull request yet"
                    } else {
                        ""
                    },
                    i + 1
                ),
                cmd(Command::Claim(t.key.clone())),
                "q".to_string(),
            );
            s.ticket = Some(t.key.clone());
            s.hotfix = hot;
            push(s, Some(i));
        }

        // hotfix boost and adapter availability
        let mut res: Vec<Sug> = Vec::new();
        for s in out {
            let mut p = s.priority;
            if s.hotfix && s.priority >= 400 && s.rule != "park_active" {
                p += 200;
            }
            if let Some(n) = s.needs {
                let ready = if n == "acli" {
                    self.jira_ready
                } else {
                    self.gh_ready
                };
                if !ready {
                    res.push(Sug {
                        id: format!("-:adapter:{n}:{}", s.rule),
                        ticket: s.ticket.clone(),
                        rule: "adapter_unavailable",
                        title: format!("{n} is signed out"),
                        reason: format!(
                            "Cannot {} until {n} is logged in. Run {n} auth login.",
                            s.title.to_lowercase()
                        ),
                        level: Level::Automatic,
                        info: true,
                        hotfix: s.hotfix,
                        needs: None,
                        act: Intent::Go(Route::Settings),
                        alt: None,
                        hash: "ad".to_string(),
                        priority: 299,
                        rank: 0,
                        state: SugState::Open,
                    });
                    continue;
                }
            }
            res.push(Sug { priority: p, ..s });
        }
        let mut seen = std::collections::BTreeSet::new();
        res.retain(|s| seen.insert(s.id.clone()));
        res.sort_by(|a, b| b.priority.cmp(&a.priority).then_with(|| a.id.cmp(&b.id)));
        for (i, s) in res.iter_mut().enumerate() {
            s.rank = i + 1;
            s.state = self.sug_state(s);
        }
        res
    }

    fn sug_state(&self, s: &Sug) -> SugState {
        match self.responses.get(&s.id) {
            None => SugState::Open,
            Some(Response::Dismissed { hash }) => {
                if *hash == s.hash {
                    SugState::Dismissed
                } else {
                    SugState::Resurfaced
                }
            }
            Some(Response::Snoozed { until }) => {
                if self.ms < *until {
                    SugState::Snoozed
                } else {
                    SugState::Open
                }
            }
        }
    }

    pub(crate) fn visible_suggestions(&self) -> Vec<Sug> {
        self.suggest()
            .into_iter()
            .filter(|s| matches!(s.state, SugState::Open | SugState::Resurfaced))
            .collect()
    }

    /// What needs you now.
    pub(crate) fn attention(&self) -> Vec<Sug> {
        let mut v: Vec<Sug> = self
            .visible_suggestions()
            .into_iter()
            .filter(|s| !s.info && (ATTENTION.contains(&s.rule) || s.hotfix))
            .collect();
        v.sort_by_key(|s| std::cmp::Reverse(s.priority));
        v
    }
}

/// Orders `PROJ-9` before `PROJ-10`.
pub fn natural_key(key: &str) -> (String, u32) {
    match key.rsplit_once('-') {
        Some((p, n)) => (p.to_string(), n.parse().unwrap_or(0)),
        None => (key.to_string(), 0),
    }
}
