//! The state changes: claim, review, activate, integrate, deploy, announce, approve, sync.

use super::Sim;
use super::data;
use super::model::*;
use crate::store::Outcome;
use crate::vm::{Baseline, Block, ReportRow, ReportVm, SimEvent, ToastKind, Undo, WaitKind};

impl Sim {
    /* ---------------------------- local lifecycle ---------------------------- */

    pub(crate) fn claim(&mut self, key: &str) -> Outcome {
        let Some(i) = self.idx(key) else {
            return Outcome::fail("Unknown ticket.");
        };
        if self.tickets[i].local.is_some() {
            return Outcome::fail("Already claimed.");
        }
        if let Some(b) = self.claim_block(key) {
            return Outcome::blocked(b);
        }
        self.tickets[i].local = Some(Local::Claimed);
        self.start_pr_wait(i);
        self.ok("ticket.claim", Some(key), None, "");
        Outcome::ok().with_toast(
            format!("Claimed {key}"),
            ToastKind::Ok,
            Some(Undo::Claim(key.to_string())),
        )
    }

    pub(crate) fn unclaim(&mut self, key: &str) {
        if let Some(i) = self.idx(key)
            && self.tickets[i].local == Some(Local::Claimed)
        {
            self.tickets[i].local = None;
            self.ok("undo.claim", Some(key), None, "");
        }
    }

    pub(crate) fn start_review(&mut self, key: &str) -> Outcome {
        let Some(i) = self.idx(key) else {
            return Outcome::fail("Unknown ticket.");
        };
        if self.tickets[i].local.is_none()
            && let Some(b) = self.claim_block(key)
        {
            return Outcome::blocked(b);
        }
        if let Some(e) = self.gap_error(&self.tickets[i]) {
            return Outcome::fail(e);
        }
        let t = &mut self.tickets[i];
        if t.local.is_none() {
            t.local = Some(Local::Claimed);
        }
        if matches!(t.local, Some(Local::Claimed | Local::Reviewing)) {
            t.local = Some(Local::Reviewing);
        }
        t.reviewed = false;
        t.reviewed_seq = None;
        self.ok("review.start", Some(key), None, "");
        Outcome::ok()
    }

    pub(crate) fn mark_reviewed(&mut self, key: &str) -> Outcome {
        let Some(i) = self.idx(key) else {
            return Outcome::fail("Unknown ticket.");
        };
        if let Some(e) = self.gap_error(&self.tickets[i]) {
            return Outcome::fail(e);
        }
        let prev = Undo::Reviewed {
            key: key.to_string(),
            prev_seq: self.tickets[i].reviewed_seq,
            prev_status: self.tickets[i]
                .local
                .map_or("claimed", Local::word)
                .to_string(),
        };
        let t = &mut self.tickets[i];
        t.reviewed = true;
        t.reviewed_seq = Some(t.prs.iter().map(|p| p.updated_seq).max().unwrap_or(0));
        for p in &mut t.prs {
            p.since = None;
        }
        if t.local == Some(Local::Claimed) {
            t.local = Some(Local::Reviewing);
        }
        self.start_th_wait(i, false);
        self.ok("review.mark_reviewed", Some(key), None, "");
        Outcome::ok().with_toast(format!("Marked {key} reviewed"), ToastKind::Ok, Some(prev))
    }

    pub(crate) fn unmark_reviewed(&mut self, key: &str, prev_seq: Option<u32>, prev_status: &str) {
        if let Some(i) = self.idx(key) {
            let t = &mut self.tickets[i];
            t.reviewed = false;
            t.reviewed_seq = prev_seq;
            t.local = Some(Local::parse(prev_status));
            self.ok("undo.mark_reviewed", Some(key), None, "");
        }
    }

    pub(crate) fn reclaim(&mut self, key: &str) -> Outcome {
        let Some(i) = self.idx(key) else {
            return Outcome::fail("Unknown ticket.");
        };
        if let Some(b) = self.claim_block(key) {
            return Outcome::blocked(b);
        }
        let t = &mut self.tickets[i];
        t.local = Some(Local::Claimed);
        t.reviewed = false;
        t.reviewed_seq = None;
        t.merges.clear();
        t.deploy.clear();
        t.drafts.clear();
        t.new_commits = false;
        t.prep = None;
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

    pub(crate) fn toggle_checklist(&mut self, key: &str, index: usize) {
        if let Some(i) = self.idx(key)
            && let Some(c) = self.tickets[i].checklist.get_mut(index)
        {
            c.1 = !c.1;
        }
    }

    pub(crate) fn add_checklist(&mut self, key: &str, text: &str) {
        if let Some(i) = self.idx(key)
            && !text.trim().is_empty()
        {
            self.tickets[i]
                .checklist
                .push((text.trim().to_string(), false));
        }
    }

    pub(crate) fn choose(&mut self, key: &str, repo: &str, branch: &str) {
        if let Some(i) = self.idx(key) {
            self.tickets[i]
                .link
                .insert(repo.to_string(), branch.to_string());
            self.ok("links.choose", Some(key), Some(repo), branch);
        }
    }

    pub(crate) fn mark_seen(&mut self, key: &str) {
        if let Some(i) = self.idx(key) {
            self.tickets[i].seen_n = Self::max_n(&self.tickets[i]);
        }
    }

    /* ---------------------------- waits ---------------------------- */

    fn start_pr_wait(&mut self, i: usize) {
        if self.pr_gap(&self.tickets[i]).is_some() && self.tickets[i].pr_wait.is_none() {
            self.tickets[i].pr_wait = Some(Wait {
                since: self.ms,
                mins: self.wait_min,
            });
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

    pub(crate) fn extend_wait(&mut self, key: &str, kind: WaitKind) -> Outcome {
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

    pub(crate) fn activate(&mut self, key: &str, choice: Option<Baseline>) -> Outcome {
        let Some(i) = self.idx(key) else {
            return Outcome::fail("Unknown ticket.");
        };
        let t = self.tickets[i].clone();
        if t.local.is_none() {
            return Outcome::fail("Claim the ticket first.");
        }
        if let Some(other) = self.active_key() {
            return Outcome::fail(format!(
                "{other} is already active. Park or finish it first."
            ));
        }
        if !matches!(
            t.local,
            Some(Local::Claimed | Local::Reviewing | Local::Parked | Local::Integrated)
        ) {
            return Outcome::fail(format!(
                "A {} ticket cannot be activated.",
                t.local.map_or("", Local::word)
            ));
        }
        if self.is_hotfix(&t) && choice.is_none() {
            return Outcome {
                need_baseline: true,
                ..Outcome::default()
            };
        }
        let plan = self.activation_plan(&t, choice);
        if !plan.errors.is_empty() {
            return Outcome {
                ok: false,
                errors: plan.errors,
                ..Outcome::default()
            };
        }
        let repos: Vec<String> = plan.rows.iter().map(|r| r.repo.clone()).collect();
        if let Some(b) = self.lock_busy(&repos, "activate", key) {
            return Outcome::busy(b);
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
            self.ok(
                "activation.repo_switched",
                Some(key),
                Some(&r.repo),
                &format!(
                    "{} {}",
                    if r.ticket_role {
                        "ticket branch"
                    } else {
                        "baseline"
                    },
                    r.branch
                ),
            );
            report.push(ReportRow {
                repo: r.repo.clone(),
                detail: format!(
                    "switched to {} ({}){}",
                    r.branch,
                    if r.ticket_role {
                        "ticket branch"
                    } else {
                        "baseline"
                    },
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
        let ms = self.ms;
        let t = &mut self.tickets[i];
        t.act = Some(Activation {
            started_ms: ms,
            records,
            overlay: plan.overlay,
        });
        t.local = Some(Local::Active);
        t.prep = None;
        t.test_from_n = Some(Self::max_n(t));
        self.ok("activation.complete", Some(key), None, "");
        Outcome {
            ok: true,
            report: Some(ReportVm {
                title: format!("Activated {key}"),
                rows: report,
                notes,
            }),
            ..Outcome::default()
        }
        .with_toast(format!("{key} is active"), ToastKind::Ok, None)
    }

    /// Restore every repo. `finalize` is the integration path, which skips the lock check.
    pub(crate) fn deactivate(
        &mut self,
        key: &str,
        target: Local,
        finalize: bool,
    ) -> Result<ReportVm, Outcome> {
        let Some(i) = self.idx(key) else {
            return Err(Outcome::fail("Unknown ticket."));
        };
        let Some(act) = self.tickets[i].act.clone() else {
            return Err(Outcome::fail(format!("Nothing to restore for {key}.")));
        };
        if !finalize {
            let repos: Vec<String> = act.records.iter().map(|r| r.repo.clone()).collect();
            if let Some(b) = self.lock_busy(&repos, "park", key) {
                return Err(Outcome::busy(b));
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
        let ms = self.ms;
        let t = &mut self.tickets[i];
        t.time_ms += ms - act.started_ms;
        t.act = None;
        t.local = Some(target);
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

    pub(crate) fn park(&mut self, key: &str) -> Outcome {
        match self.deactivate(key, Local::Parked, false) {
            Ok(report) => Outcome {
                ok: true,
                report: Some(report),
                ..Outcome::default()
            }
            .with_toast(format!("Parked {key}"), ToastKind::Ok, None),
            Err(o) => o,
        }
    }

    /// Park whatever is in hand so another ticket can be taken.
    pub(crate) fn park_in_hand(&mut self, key: &str) -> Result<(), Outcome> {
        let Some(i) = self.idx(key) else {
            return Err(Outcome::fail("Unknown ticket."));
        };
        if self.tickets[i].act.is_some() {
            return self.deactivate(key, Local::Parked, false).map(|_| ());
        }
        self.tickets[i].local = Some(Local::Parked);
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
        from: &str,
        key: &str,
        then: crate::vm::Then,
    ) -> Outcome {
        if let Err(o) = self.park_in_hand(from) {
            return o;
        }
        let out = match then {
            crate::vm::Then::Reclaim => self.reclaim(key),
            crate::vm::Then::Claim => self.claim(key),
            crate::vm::Then::StartReview => {
                let c = self.claim(key);
                if c.ok { self.start_review(key) } else { c }
            }
        };
        if !out.ok {
            return out;
        }
        Outcome::ok().with_toast(format!("{from} parked, {key} claimed"), ToastKind::Ok, None)
    }

    pub(crate) fn claim_block(&self, key: &str) -> Option<crate::vm::ClaimBlock> {
        let b = self.in_hand(key)?;
        Some(crate::vm::ClaimBlock {
            blocker: b.key.clone(),
            title: b.title.clone(),
            what: match b.local {
                Some(Local::Active) => "active (your repos are on its branches)",
                Some(Local::Reviewing) => "in review",
                _ => "claimed",
            }
            .to_string(),
            active: b.local == Some(Local::Active),
        })
    }

    /* ---------------------------- integration ---------------------------- */

    pub(crate) fn prepare(&mut self, key: &str) -> Outcome {
        let Some(i) = self.idx(key) else {
            return Outcome::fail("Unknown ticket.");
        };
        let t = self.tickets[i].clone();
        if t.local != Some(Local::Active) {
            return Outcome::fail(
                "The ticket must be active (tested locally) before it can be integrated.",
            );
        }
        let touched = self.touched(&t);
        let names: Vec<String> = touched.iter().map(|p| p.repo.clone()).collect();
        if let Some(b) = self.lock_busy(&names, "prepare", key) {
            return Outcome::busy(b);
        }
        let mut rows = Vec::new();
        for p in &touched {
            let repo = p.repo.clone();
            let outcome = if let Some(done) = t.merges.iter().find(|m| m.repo == repo) {
                PrepOutcome::AlreadyPushed {
                    commit: done.commit.clone(),
                }
            } else if self.sims.conflict.as_deref() == Some(repo.as_str()) {
                PrepOutcome::Conflict {
                    files: vec!["src/session.rs".to_string()],
                    with: "PROJ-127".to_string(),
                }
            } else if self.sims.leak.as_deref() == Some(repo.as_str()) {
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
            if all_ok { "success" } else { "failure" },
            &summary,
        );
        self.tickets[i].prep = Some(Prep { at: self.ms, rows });
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

    pub(crate) fn push(&mut self, key: &str) -> Outcome {
        let Some(i) = self.idx(key) else {
            return Outcome::fail("Unknown ticket.");
        };
        let t = self.tickets[i].clone();
        let Some(prep) = t.prep.clone().filter(|_| self.prep_pushable(&t)) else {
            return Outcome::fail("Nothing is ready to push. Prepare the integration again.");
        };
        let names: Vec<String> = prep.rows.iter().map(|r| r.repo.clone()).collect();
        if let Some(b) = self.lock_busy(&names, "push", key) {
            return Outcome::busy(b);
        }
        let spec = prep
            .rows
            .iter()
            .map(|r| match &r.outcome {
                PrepOutcome::Ready { merge, .. } => format!("{} {merge}:refs/heads/uat", r.repo),
                PrepOutcome::AlreadyPushed { commit } => {
                    format!("{} {commit}:refs/heads/uat", r.repo)
                }
                _ => r.repo.clone(),
            })
            .collect::<Vec<_>>()
            .join("; ");
        self.audit("git.push_uat.attempted", Some(key), None, "skipped", &spec);
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
                    "failure",
                    "rejected: uat moved",
                );
                self.sims.uat_moved = false;
                self.tickets[i].prep = None;
                rejected = true;
                break;
            }
            let at = self.clock(None);
            self.run += 1;
            let run = self.run;
            let ms = self.ms;
            let t = &mut self.tickets[i];
            t.merges.push(Merge {
                repo: r.repo.clone(),
                commit: merge.clone(),
                at,
            });
            t.deploy.insert(
                r.repo.clone(),
                Deploy {
                    state: DeployState::Pending,
                    run,
                    since: ms,
                    step: "Queued".to_string(),
                    log: None,
                    uat_moved: None,
                },
            );
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
                "failure",
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
            let _ = self.deactivate(key, Local::Integrated, true);
            self.tickets[i].prep = None;
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
                "failure",
                "not every repo was pushed",
            );
            Outcome::fail("Not every repo was pushed.")
        }
    }

    pub(crate) fn rerun(&mut self, key: &str, repo: &str) -> Outcome {
        let Some(i) = self.idx(key) else {
            return Outcome::fail("Unknown ticket.");
        };
        if !self.tickets[i].deploy.contains_key(repo) {
            return Outcome::fail("No run to re-run.");
        }
        self.run += 1;
        let run = self.run;
        let ms = self.ms;
        if let Some(d) = self.tickets[i].deploy.get_mut(repo) {
            d.run = run;
            d.state = DeployState::Pending;
            d.since = ms;
            d.step = "Queued".to_string();
            d.log = None;
        }
        if self.sims.fail_deploy.as_deref() == Some(repo) {
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
            for (repo, d) in &mut t.deploy {
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
                    let failed = fail.as_deref() == Some(repo.as_str());
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
                            "{} {repo} run #{} {}",
                            t.key,
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
        }
    }

    /* ---------------------------- announce ---------------------------- */

    pub(crate) fn compose_draft(&mut self, key: &str) -> Outcome {
        let Some(i) = self.idx(key) else {
            return Outcome::fail("Unknown ticket.");
        };
        let t = self.tickets[i].clone();
        if !self.all_deployed(&t) {
            return Outcome::fail("Every touched repo must be deployed first.");
        }
        let lines: Vec<String> = t
            .merges
            .iter()
            .map(|m| {
                let pr = t.prs.iter().find(|p| p.repo == m.repo);
                let d = &t.deploy[&m.repo];
                format!(
                    "• {}  PR #{} · run #{} · https://github.com/{}/actions/runs/{}",
                    m.repo,
                    pr.map_or("-".to_string(), |p| p.id.to_string()),
                    d.run,
                    self.repo(&m.repo).host,
                    d.run
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
        let id = format!("d{}", self.next_seq());
        self.tickets[i].drafts.push(Draft {
            id,
            body: parts.join("\n\n"),
            posted: false,
        });
        self.ok("draft.compose", Some(key), None, "");
        Outcome::ok()
    }

    pub(crate) fn edit_draft(&mut self, key: &str, id: &str, text: &str) {
        if let Some(i) = self.idx(key)
            && let Some(d) = self.tickets[i].drafts.iter_mut().find(|d| d.id == id)
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

    pub(crate) fn post_comment(&mut self, key: &str, draft: &str) -> Outcome {
        let Some(i) = self.idx(key) else {
            return Outcome::fail("Unknown ticket.");
        };
        let Some(d) = self.tickets[i]
            .drafts
            .iter()
            .find(|d| d.id == draft)
            .cloned()
        else {
            return Outcome::fail("That draft was already posted.");
        };
        if d.posted {
            return Outcome::fail("That draft was already posted.");
        }
        if let Some(x) = self.tickets[i].drafts.iter_mut().find(|x| x.id == draft) {
            x.posted = true;
        }
        self.push_comment(i, &d.body);
        self.ok("jira.comment", Some(key), None, "deploy comment");
        Outcome::ok()
    }

    pub(crate) fn transition(&mut self, key: &str) -> Outcome {
        let Some(i) = self.idx(key) else {
            return Outcome::fail("Unknown ticket.");
        };
        if !Self::draft(&self.tickets[i]).is_some_and(|d| d.posted) {
            return Outcome::fail("Post the deploy comment first.");
        }
        self.tickets[i].jira = JIRA_ALPHA.to_string();
        self.ok(
            "jira.transition",
            Some(key),
            None,
            &format!("{JIRA_REVIEW} -> {JIRA_ALPHA}"),
        );
        Outcome::ok().with_toast(format!("{key} moved to {JIRA_ALPHA}"), ToastKind::Ok, None)
    }

    pub(crate) fn post_and_move(&mut self, key: &str, draft: &str) -> Outcome {
        let r = self.post_comment(key, draft);
        if !r.ok {
            return r;
        }
        if self.tk(key).is_some_and(|t| t.jira == JIRA_REVIEW) {
            return self.transition(key);
        }
        Outcome::ok().with_toast("Comment posted to Jira", ToastKind::Ok, None)
    }

    pub(crate) fn post_free_comment(&mut self, key: &str, text: &str) -> Outcome {
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

    pub(crate) fn approve(&mut self, key: &str, repo: &str, pr: u32) -> Outcome {
        let Some(i) = self.idx(key) else {
            return Outcome::fail("Unknown ticket.");
        };
        if !self.gh_ready {
            return Outcome::fail("gh is signed out.");
        }
        if let Some(p) = self.tickets[i]
            .prs
            .iter_mut()
            .find(|p| p.repo == repo && p.id == pr)
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
        if self.tickets[i].prs.iter().all(Self::my_approved) {
            self.tickets[i].local = Some(Local::Done);
            self.ok("ticket.done", Some(key), None, "");
        }
        Outcome::ok().with_toast(format!("Approved {repo} #{pr}"), ToastKind::Ok, None)
    }

    pub(crate) fn approve_all(&mut self, key: &str) -> Outcome {
        let Some(t) = self.tk(key) else {
            return Outcome::fail("Unknown ticket.");
        };
        let pend: Vec<(String, u32)> = t
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

    pub(crate) fn request_changes(&mut self, key: &str, pr: u32, text: &str) -> Outcome {
        let Some(i) = self.idx(key) else {
            return Outcome::fail("Unknown ticket.");
        };
        let had = !Self::blocking_threads(&self.tickets[i]).is_empty();
        let id = format!("th{}", self.next_seq());
        let mut repo = String::new();
        if let Some(p) = self.tickets[i].prs.iter_mut().find(|p| p.id == pr) {
            repo = p.repo.clone();
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
            Some(&repo),
            &format!("#{pr}"),
        );
        Outcome::ok().with_toast("Requested changes", ToastKind::Ok, None)
    }

    pub(crate) fn add_thread(
        &mut self,
        key: &str,
        pr: u32,
        file: &str,
        line: &str,
        text: &str,
    ) -> Outcome {
        let Some(i) = self.idx(key) else {
            return Outcome::fail("Unknown ticket.");
        };
        let had = !Self::blocking_threads(&self.tickets[i]).is_empty();
        let id = format!("th{}", self.next_seq());
        let mut repo = String::new();
        if let Some(p) = self.tickets[i].prs.iter_mut().find(|p| p.id == pr) {
            repo = p.repo.clone();
            p.threads.push(Thread {
                id,
                file: file.to_string(),
                line: Some(line.to_string()),
                author: ME.to_string(),
                text: text.to_string(),
                resolved: false,
            });
        }
        self.start_th_wait(i, had);
        self.ok(
            "github.pr_comment",
            Some(key),
            Some(&repo),
            &format!("{file}:{}", line.get(1..).unwrap_or("")),
        );
        Outcome::ok().with_toast("Comment posted on the pull request", ToastKind::Ok, None)
    }

    /* ---------------------------- returns and conflicts ---------------------------- */

    fn send_return(&mut self, key: &str, body: &str, note: &str) -> Outcome {
        let Some(i) = self.idx(key) else {
            return Outcome::fail("Unknown ticket.");
        };
        if self.tickets[i].act.is_some()
            && let Err(o) = self.deactivate(key, Local::Parked, false)
        {
            return o;
        }
        self.push_comment(i, body);
        let t = &mut self.tickets[i];
        t.jira = JIRA_RETURNED.to_string();
        t.local = None;
        t.pr_wait = None;
        t.th_wait = None;
        t.reviewed = false;
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
            &format!("{JIRA_REVIEW} -> {JIRA_RETURNED}"),
        );
        Outcome::ok().with_toast(
            format!("Returned {key} with a comment"),
            ToastKind::Ok,
            None,
        )
    }

    pub(crate) fn return_missing_pr(&mut self, key: &str) -> Outcome {
        let Some(t) = self.tk(key) else {
            return Outcome::fail("Unknown ticket.");
        };
        if self.pr_gap(t).is_none() {
            return Outcome::fail("A pull request exists now. Nothing was sent.");
        }
        let body = self.return_body(t);
        self.send_return(key, &body, "no pull request")
    }

    pub(crate) fn return_threads(&mut self, key: &str) -> Outcome {
        let Some(t) = self.tk(key) else {
            return Outcome::fail("Unknown ticket.");
        };
        if Self::blocking_threads(t).is_empty() {
            return Outcome::fail("Nothing is unresolved now. Nothing was sent.");
        }
        let body = self.return_threads_body(t);
        self.send_return(key, &body, "unresolved review comments")
    }

    pub(crate) fn accept_threads(&mut self, key: &str) -> Outcome {
        let Some(i) = self.idx(key) else {
            return Outcome::fail("Unknown ticket.");
        };
        let ids: Vec<String> = Self::blocking_threads(&self.tickets[i])
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

    pub(crate) fn send_conflict(&mut self, key: &str, also_return: bool) -> Outcome {
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

    pub(crate) fn break_lock(&mut self, repo: &str) -> Outcome {
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
            self.audit("sync", None, None, "failure", "offline");
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
            if failed { "failure" } else { "success" },
            &text,
        );
    }

    /// The scripted "things that happen in the outside world" on successive syncs.
    fn deliver_arrival(&mut self) -> Option<String> {
        let step = self.sync.script;
        let text = match step {
            0 => {
                self.tickets.push(data::arriving_ticket());
                "PROJ-155 appeared in the Review column"
            }
            1 => {
                let repos = self.repos.clone();
                let serial = self.seq as u32;
                if let Some(i) = self.idx("PROJ-163") {
                    data::open_fake_prs(&mut self.tickets[i], &repos, serial);
                }
                "PROJ-163: a pull request was opened in web"
            }
            2 => {
                if let Some(i) = self.idx("PROJ-150") {
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

    pub(crate) fn dismiss(&mut self, id: &str) -> Outcome {
        let sug = self.suggest().into_iter().find(|s| s.id == id);
        let hash = sug.as_ref().map(|s| s.hash.clone()).unwrap_or_default();
        self.responses
            .insert(id.to_string(), Response::Dismissed { hash });
        self.ok(
            "suggestion.dismiss",
            sug.and_then(|s| s.ticket).as_deref(),
            None,
            id,
        );
        Outcome::ok().with_toast(
            "Dismissed",
            ToastKind::Info,
            Some(Undo::Response(id.to_string())),
        )
    }

    pub(crate) fn snooze(&mut self, id: &str, minutes: u32) -> Outcome {
        self.responses.insert(
            id.to_string(),
            Response::Snoozed {
                until: self.ms + i64::from(minutes) * MS_PER_MIN,
            },
        );
        self.ok("suggestion.snooze", None, None, &format!("{id} {minutes}m"));
        Outcome::ok().with_toast(
            format!("Snoozed for {minutes} min"),
            ToastKind::Info,
            Some(Undo::Response(id.to_string())),
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
                    self.tickets[i].jira = "Done".to_string();
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
                    t.jira = JIRA_RETURNED.to_string();
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
                    self.tickets[i].jira = JIRA_REVIEW.to_string();
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
                    ticket: "PROJ-150".to_string(),
                    at: self.clock(None),
                };
                if let Some(i) = self.idx(key) {
                    let repos: Vec<String> = self.tickets[i]
                        .merges
                        .iter()
                        .map(|m| m.repo.clone())
                        .collect();
                    for r in repos {
                        if let Some(d) = self.tickets[i].deploy.get_mut(&r) {
                            d.uat_moved = Some(who.clone());
                        }
                    }
                    self.toast(
                        format!("Someone pushed to uat after {key}"),
                        ToastKind::Warn,
                    );
                }
            }
            SimEvent::SkipMinutes(m) => self.advance(i64::from(*m) * MS_PER_MIN),
        }
        Outcome::ok()
    }
}
