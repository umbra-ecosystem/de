//! Executing `PushUat`: guard, push without force, record each merge right after its push.

use serde_json::{Value, json};

use super::{Gateway, GatewayError, PushUat, PushUatRepo, actions};
use crate::{
    domain::{AuditOutcome, LocalStatus},
    git::{GitRepo, GitRunner},
    overlay::check_range_for_overlay,
    store::{
        audit::{self, NewAuditEntry},
        tickets,
        uat_details::{self, MergeDetails, MergeKind},
        uat_merges::UatMerge,
    },
};

/// What happened to one repo's push.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushResult {
    /// Pushed and recorded in `uat_merges`.
    Pushed { merge_commit: String },
    /// Skipped: a recorded merge already contains the ticket branch tip.
    AlreadyPushed,
    /// The remote refused a non-fast-forward: `uat` moved since the merge was prepared.
    /// Nothing was forced; prepare again.
    Rejected { reason: String },
    /// Refused by the guard before any push (overlay leak, merge commit not as prepared).
    Blocked { reason: String },
    /// git failed (network, credentials, a server hook).
    Failed { error: String },
    /// Pushed, but the merge could not be recorded locally. `origin/uat` already has the
    /// commit; preparing again treats the repo as already in `uat`.
    PushedUnrecorded { merge_commit: String, error: String },
    /// Not tried because an earlier repo did not succeed.
    NotAttempted,
}

impl PushResult {
    pub fn is_done(&self) -> bool {
        matches!(self, PushResult::Pushed { .. } | PushResult::AlreadyPushed)
    }

    pub(super) fn describe(&self) -> Value {
        match self {
            PushResult::Pushed { merge_commit } => json!({ "pushed": merge_commit }),
            PushResult::AlreadyPushed => json!("already_pushed"),
            PushResult::Rejected { reason } => json!({ "rejected": reason }),
            PushResult::Blocked { reason } => json!({ "blocked": reason }),
            PushResult::Failed { error } => json!({ "failed": error }),
            PushResult::PushedUnrecorded {
                merge_commit,
                error,
            } => json!({ "pushed_unrecorded": merge_commit, "error": error }),
            PushResult::NotAttempted => json!("not_attempted"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoPush {
    pub repo: String,
    pub result: PushResult,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushReport {
    pub repos: Vec<RepoPush>,
}

impl PushReport {
    /// Every repo is pushed (now or before).
    pub fn all_done(&self) -> bool {
        self.repos.iter().all(|r| r.result.is_done())
    }
}

fn classify_push_failure(stderr: &str) -> PushResult {
    let stderr = stderr.trim();
    let non_ff = stderr.contains("[rejected]")
        && (stderr.contains("non-fast-forward")
            || stderr.contains("fetch first")
            || stderr.contains("stale info"));
    if non_ff {
        PushResult::Rejected {
            reason: format!("uat moved on the remote since the merge was prepared: {stderr}"),
        }
    } else {
        PushResult::Failed {
            error: stderr.into(),
        }
    }
}

impl Gateway<'_> {
    /// The guard re-run at push time, on the exact commits about to be pushed.
    fn preflight(&self, repo: &PushUatRepo) -> Result<(), String> {
        let git = GitRepo::open(&repo.repo_dir).map_err(|e| format!("{e:#}"))?;
        let merge = git
            .commit(&repo.merge_commit)
            .map_err(|e| format!("the merge commit is gone: {e:#}"))?;
        let parents: Vec<String> = merge.parent_ids().map(|p| p.to_string()).collect();
        if parents != [repo.uat_before.clone(), repo.ticket_tip.clone()] {
            return Err(format!(
                "{} is not the merge of {} into {} that was prepared",
                repo.merge_commit, repo.ticket_tip, repo.uat_before
            ));
        }
        let packages: Vec<&str> = repo.overlay_packages.iter().map(String::as_str).collect();
        check_range_for_overlay(&git, &repo.uat_before, &repo.merge_commit, &packages)
            .map_err(|e| e.to_string())
    }

    fn already_pushed(&self, push: &PushUat, repo: &PushUatRepo) -> eyre::Result<bool> {
        let git = GitRepo::open(&repo.repo_dir)?;
        for rec in uat_details::list_for_ticket(self.state, &push.ticket)? {
            if rec.merge.repo != repo.repo {
                continue;
            }
            // Only a merge the remote's `uat` (as prepared) still contains counts; after a
            // reset of `uat` an old record must not stop the push.
            if git
                .is_ancestor(&repo.ticket_tip, &rec.merge.commit)
                .unwrap_or(false)
                && git
                    .is_ancestor(&rec.merge.commit, &repo.uat_before)
                    .unwrap_or(false)
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub(super) fn push_uat(
        &self,
        push: &PushUat,
        now: i64,
        warnings: &mut Vec<String>,
    ) -> Result<PushReport, GatewayError> {
        match tickets::get(self.state, &push.ticket)? {
            Some(t) if t.status == LocalStatus::Active => {}
            Some(t) => {
                return Err(GatewayError::Precondition(format!(
                    "{} is {}, not active; only a tested (active) ticket is integrated",
                    push.ticket, t.status
                )));
            }
            None => {
                return Err(GatewayError::Precondition(format!(
                    "{} is not tracked",
                    push.ticket
                )));
            }
        }

        let mut skip: Vec<bool> = Vec::new();
        let mut blocked: Option<(usize, String)> = None;

        // Pre-flight every repo before pushing any, so a leak in the last repo cannot leave
        // the first ones pushed.
        for (index, repo) in push.repos.iter().enumerate() {
            if self.already_pushed(push, repo)? {
                skip.push(true);
                continue;
            }
            skip.push(false);
            if let Err(reason) = self.preflight(repo) {
                blocked = Some((index, reason));
                break;
            }
        }
        if let Some((at, reason)) = blocked {
            let repos = push
                .repos
                .iter()
                .enumerate()
                .map(|(i, r)| RepoPush {
                    repo: r.repo.clone(),
                    result: if i == at {
                        PushResult::Blocked {
                            reason: reason.clone(),
                        }
                    } else if skip.get(i).copied().unwrap_or(false) {
                        PushResult::AlreadyPushed
                    } else {
                        PushResult::NotAttempted
                    },
                })
                .collect();
            return Ok(PushReport { repos });
        }

        let mut report: Vec<RepoPush> = Vec::new();
        let mut stopped = false;
        for (repo, was_skipped) in push.repos.iter().zip(&skip) {
            let result = if stopped {
                PushResult::NotAttempted
            } else if *was_skipped {
                PushResult::AlreadyPushed
            } else {
                let result = self.push_one(push, repo, now, warnings);
                stopped = !result.is_done();
                result
            };
            report.push(RepoPush {
                repo: repo.repo.clone(),
                result,
            });
        }
        Ok(PushReport { repos: report })
    }

    fn push_one(
        &self,
        push: &PushUat,
        repo: &PushUatRepo,
        now: i64,
        warnings: &mut Vec<String>,
    ) -> PushResult {
        let refspec = format!("{}:refs/heads/{}", repo.merge_commit, repo.uat_branch);
        // No --force, ever.
        let outcome = GitRunner::new(&repo.repo_dir).run_raw(&["push", &repo.remote, &refspec]);
        let mut result = match outcome {
            Err(e) => PushResult::Failed {
                error: format!("{e:#}"),
            },
            Ok(out) if !out.success => classify_push_failure(&out.stderr),
            Ok(_) => PushResult::Pushed {
                merge_commit: repo.merge_commit.clone(),
            },
        };

        if let PushResult::Pushed { merge_commit } = &result {
            // Record immediately, before the next repo is touched.
            let recorded = uat_details::record(
                self.state,
                &UatMerge {
                    ticket: push.ticket.clone(),
                    repo: repo.repo.clone(),
                    branch: repo.uat_branch.clone(),
                    commit: repo.merge_commit.clone(),
                    recorded_at: now,
                },
                &MergeDetails {
                    kind: MergeKind::Merge,
                    ticket_branch: repo.ticket_branch.clone(),
                    ticket_tip: repo.ticket_tip.clone(),
                    uat_before: repo.uat_before.clone(),
                },
            );
            if let Err(e) = recorded {
                result = PushResult::PushedUnrecorded {
                    merge_commit: merge_commit.clone(),
                    error: format!("{e:#}"),
                };
            }
        }

        let entry = audit::append(
            self.state,
            &NewAuditEntry {
                at: now,
                action: actions::PUSH_UAT_REPO.into(),
                ticket: Some(push.ticket.clone()),
                repo: Some(repo.repo.clone()),
                details: json!({
                    "remote": repo.remote,
                    "branch": repo.uat_branch,
                    "merge_commit": repo.merge_commit,
                    "uat_before": repo.uat_before,
                    "ticket_tip": repo.ticket_tip,
                    "result": result.describe(),
                }),
                outcome: if result.is_done() {
                    AuditOutcome::Success
                } else {
                    AuditOutcome::Failure
                },
            },
        );
        if let Err(e) = entry {
            warnings.push(format!(
                "the push of {} could not be audited: {e:#}",
                repo.repo
            ));
        }
        result
    }
}
