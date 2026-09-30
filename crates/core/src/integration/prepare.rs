//! Preparing, pushing (via the gateway), finalizing and cancelling an integration.

use std::path::{Path, PathBuf};

use eyre::{Context, bail, eyre};
use serde_json::json;

use super::actions;
use crate::{
    activation::{DeactivationReport, WorkspaceRepo, deactivate_for_integration},
    domain::{AuditOutcome, LocalStatus, TicketKey},
    gateway::{
        Action, CommitLine, Confirmed, Executed, Gateway, Outcome, PushReport, PushUat, PushUatRepo,
    },
    git::{GitRepo, GitRunner, MergeOutcome},
    overlay::{
        CommandRunner, ExternalCommand, check_range_for_overlay, run_checked,
        working_tree_has_overlay,
    },
    project::{Project, config::BranchesConfig},
    store::{
        Store,
        audit::{self, NewAuditEntry},
        overlays,
        restore::{self, RepoRole},
        tickets,
        uat_details::{self, MergeDetails, MergeKind},
        uat_merges::UatMerge,
    },
};

/// A merge that is ready to be pushed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadyMerge {
    pub merge_commit: String,
    pub uat_before: String,
    /// Ticket commits being merged (newest first).
    pub commits: Vec<CommitLine>,
    /// Files the merge changes relative to `uat`.
    pub files: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepoOutcome {
    Ready(ReadyMerge),
    /// The ticket branch is already contained in `origin/<uat>`; nothing to push.
    UpToDate {
        uat: String,
    },
    /// A recorded merge already contains the ticket branch tip (an earlier, partial run).
    AlreadyPushed {
        merge_commit: String,
    },
    /// The merge conflicted and was aborted; the worktree is clean.
    Conflict {
        files: Vec<String>,
    },
    Blocked {
        reason: String,
    },
}

impl RepoOutcome {
    /// Nothing stands in the way of pushing.
    pub fn is_pushable(&self) -> bool {
        !matches!(
            self,
            RepoOutcome::Conflict { .. } | RepoOutcome::Blocked { .. }
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoIntegration {
    /// Workspace project name.
    pub repo: String,
    pub repo_dir: PathBuf,
    pub remote: String,
    pub uat_branch: String,
    pub ticket_branch: String,
    /// SHA of the ticket branch tip (when it could be read).
    pub ticket_tip: Option<String>,
    /// The temporary worktree, when one was made.
    pub worktree: Option<PathBuf>,
    pub overlay_packages: Vec<String>,
    pub outcome: RepoOutcome,
    /// Things worth knowing that do not block (uncommitted changes, unpushed commits).
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegrationPrep {
    pub ticket: TicketKey,
    pub repos: Vec<RepoIntegration>,
}

impl IntegrationPrep {
    /// Whether every touched repo is pushable and at least one has something to push.
    pub fn is_ready(&self) -> bool {
        self.repos.iter().all(|r| r.outcome.is_pushable())
            && self
                .repos
                .iter()
                .any(|r| matches!(r.outcome, RepoOutcome::Ready(_)))
    }

    /// Whether finalize could run once everything ready is pushed.
    pub fn is_pushable(&self) -> bool {
        self.repos.iter().all(|r| r.outcome.is_pushable())
    }

    /// The `PushUat` action for the ready repos. Refuses when any touched repo is blocked or
    /// conflicting: pushing some repos and not others is never offered.
    pub fn push_action(&self) -> eyre::Result<Action> {
        let mut repos = Vec::new();
        for r in &self.repos {
            match &r.outcome {
                RepoOutcome::Conflict { files } => {
                    bail!(
                        "{} conflicts with uat ({}); nothing can be pushed",
                        r.repo,
                        files.join(", ")
                    )
                }
                RepoOutcome::Blocked { reason } => {
                    bail!("{} is blocked: {reason}", r.repo)
                }
                RepoOutcome::Ready(m) => repos.push(PushUatRepo {
                    repo: r.repo.clone(),
                    repo_dir: r.repo_dir.clone(),
                    remote: r.remote.clone(),
                    uat_branch: r.uat_branch.clone(),
                    ticket_branch: r.ticket_branch.clone(),
                    ticket_tip: r.ticket_tip.clone().unwrap_or_default(),
                    uat_before: m.uat_before.clone(),
                    merge_commit: m.merge_commit.clone(),
                    commits: m.commits.clone(),
                    files: m.files.clone(),
                    overlay_packages: r.overlay_packages.clone(),
                }),
                RepoOutcome::UpToDate { .. } | RepoOutcome::AlreadyPushed { .. } => {}
            }
        }
        if repos.is_empty() {
            bail!("Nothing to push for {}", self.ticket);
        }
        Ok(Action::PushUat(PushUat {
            ticket: self.ticket.clone(),
            repos,
        }))
    }
}

/// `<data_dir>/integration/<KEY>`, under which each repo gets a worktree named after it.
pub fn integration_dir(data_dir: &Path, ticket: &TicketKey) -> PathBuf {
    data_dir.join("integration").join(ticket.as_str())
}

fn temp_branch(ticket: &TicketKey) -> String {
    format!("de/integrate/{ticket}")
}

fn audit_local(
    store: &Store,
    now: i64,
    action: &str,
    ticket: &TicketKey,
    details: serde_json::Value,
    outcome: AuditOutcome,
) -> eyre::Result<()> {
    audit::append(
        store,
        &NewAuditEntry {
            at: now,
            action: action.into(),
            ticket: Some(ticket.clone()),
            repo: None,
            details,
            outcome,
        },
    )?;
    Ok(())
}

/// Removes a temporary worktree and its branch (both ours), tolerating that either is gone.
fn remove_worktree(repo_dir: &Path, path: &Path, branch: &str) {
    let runner = GitRunner::new(repo_dir);
    if let Some(p) = path.to_str() {
        let _ = runner.run_raw(&["worktree", "remove", "--force", p]);
    }
    if path.exists() {
        let _ = std::fs::remove_dir_all(path);
    }
    let _ = runner.run_raw(&["worktree", "prune"]);
    let _ = runner.run_raw(&["branch", "-D", branch]);
}

struct Ctx<'a> {
    store: &'a Store,
    runner: &'a dyn CommandRunner,
    data_dir: &'a Path,
    ticket: &'a TicketKey,
}

/// Builds the integration of `ticket` without pushing anything. See the module docs.
///
/// Stale worktrees from an earlier prepare are removed first.
pub fn prepare_integration(
    store: &Store,
    runner: &dyn CommandRunner,
    data_dir: &Path,
    ticket: &TicketKey,
    repos: &[WorkspaceRepo],
    now: i64,
) -> eyre::Result<IntegrationPrep> {
    let tracking = tickets::get(store, ticket)?.ok_or_else(|| eyre!("{ticket} is not tracked"))?;
    if tracking.status != LocalStatus::Active {
        bail!(
            "{ticket} is {}, not active: only a ticket you activated (and tested) is integrated",
            tracking.status
        );
    }

    let touched: Vec<_> = restore::list(store, ticket)?
        .into_iter()
        .filter(|r| r.role == RepoRole::Ticket)
        .collect();
    if touched.is_empty() {
        bail!("{ticket} has no repo on its ticket branch; nothing to integrate");
    }

    let ctx = Ctx {
        store,
        runner,
        data_dir,
        ticket,
    };
    let mut out = Vec::new();
    for rec in &touched {
        let repo = repos.iter().find(|r| r.name == rec.repo);
        out.push(prepare_repo(
            &ctx,
            rec.repo.as_str(),
            &rec.repo_dir,
            &rec.branch,
            repo,
        ));
    }

    audit_local(
        store,
        now,
        actions::PREPARED,
        ticket,
        json!({ "repos": out.iter().map(|r| json!({ "repo": r.repo, "outcome": format!("{:?}", r.outcome) })).collect::<Vec<_>>() }),
        AuditOutcome::Success,
    )?;
    Ok(IntegrationPrep {
        ticket: ticket.clone(),
        repos: out,
    })
}

fn prepare_repo(
    ctx: &Ctx<'_>,
    name: &str,
    repo_dir: &Path,
    ticket_branch: &str,
    repo: Option<&WorkspaceRepo>,
) -> RepoIntegration {
    let branches = repo
        .map(|r| r.manifest.branches.clone())
        .unwrap_or_default();
    let mut result = RepoIntegration {
        repo: name.into(),
        repo_dir: repo_dir.into(),
        remote: repo.map_or("origin", |r| r.remote()).into(),
        uat_branch: branches
            .uat
            .clone()
            .unwrap_or_else(|| BranchesConfig::DEFAULT_UAT.into()),
        ticket_branch: ticket_branch.into(),
        ticket_tip: None,
        worktree: None,
        overlay_packages: repo
            .and_then(|r| r.manifest.overlay.composer.as_ref())
            .map(|c| c.packages.keys().cloned().collect())
            .unwrap_or_default(),
        outcome: RepoOutcome::Blocked {
            reason: "not prepared".into(),
        },
        warnings: Vec::new(),
    };
    let Some(repo) = repo else {
        result.outcome = RepoOutcome::Blocked {
            reason: format!("{name} is not a project of the workspace"),
        };
        return result;
    };
    result.outcome = match prepare_inner(ctx, repo, &mut result) {
        Ok(outcome) => outcome,
        Err(e) => RepoOutcome::Blocked {
            reason: format!("{e:#}"),
        },
    };
    result
}

fn prepare_inner(
    ctx: &Ctx<'_>,
    repo: &WorkspaceRepo,
    r: &mut RepoIntegration,
) -> eyre::Result<RepoOutcome> {
    let git = GitRepo::open(&r.repo_dir)?;
    let branch_ref = format!("refs/heads/{}", r.ticket_branch);
    let tip = git
        .rev_parse(&branch_ref)
        .wrap_err_with(|| format!("ticket branch '{}' does not exist locally", r.ticket_branch))?;
    r.ticket_tip = Some(tip.clone());

    // Stale worktree of an earlier prepare.
    let worktree = integration_dir(ctx.data_dir, ctx.ticket).join(&r.repo);
    let tmp = temp_branch(ctx.ticket);
    remove_worktree(&r.repo_dir, &worktree, &tmp);

    // Something an earlier (partial) run already pushed.
    for rec in uat_details::list_for_ticket(ctx.store, ctx.ticket)? {
        if rec.merge.repo == r.repo && git.is_ancestor(&tip, &rec.merge.commit).unwrap_or(false) {
            return Ok(RepoOutcome::AlreadyPushed {
                merge_commit: rec.merge.commit,
            });
        }
    }

    let status = git.status()?;
    if !status.is_clean() {
        r.warnings.push(format!(
            "{} has uncommitted changes; they are not part of the merge",
            r.repo
        ));
    }
    if status.branch.as_deref() == Some(r.ticket_branch.as_str()) {
        if status.unpushed > 0 {
            r.warnings.push(format!(
                "{} commit(s) of {} are not pushed to its remote branch; the merge includes them",
                status.unpushed, r.ticket_branch
            ));
        }
        if status.behind > 0 {
            r.warnings.push(format!(
                "{} is {} commit(s) behind its remote branch; those are not in the merge",
                r.ticket_branch, status.behind
            ));
        }
    }

    git.fetch(&r.remote)
        .wrap_err("cannot fetch (uat must be current before merging)")?;
    let uat_ref = format!("refs/remotes/{}/{}", r.remote, r.uat_branch);
    let uat_before = git
        .rev_parse(&uat_ref)
        .wrap_err_with(|| format!("{}/{} does not exist on the remote", r.remote, r.uat_branch))?;

    // A local uat with commits the remote lacks: someone committed there; do not guess.
    if let Some(local) = git
        .branches()?
        .into_iter()
        .find(|b| b.refname == r.uat_branch && b.kind == crate::git::BranchKind::Local)
    {
        let ahead = git.commits_not_in(&uat_before, &local.tip)?;
        if !ahead.is_empty() {
            return Ok(RepoOutcome::Blocked {
                reason: format!(
                    "local {} has {} commit(s) that {}/{} does not; push or reset it first",
                    r.uat_branch,
                    ahead.len(),
                    r.remote,
                    r.uat_branch
                ),
            });
        }
    }

    if git.is_ancestor(&tip, &uat_before)? {
        return Ok(RepoOutcome::UpToDate { uat: uat_before });
    }

    // The worktree, on a temporary branch at the remote uat.
    if let Some(parent) = worktree.parent() {
        std::fs::create_dir_all(parent)
            .wrap_err_with(|| format!("Failed to create {}", parent.display()))?;
    }
    let wt = worktree
        .to_str()
        .ok_or_else(|| eyre!("worktree path is not valid UTF-8"))?;
    GitRunner::new(&r.repo_dir)
        .run(&["worktree", "add", "--no-track", "-b", &tmp, wt, &uat_before])
        .wrap_err("Failed to create the integration worktree")?;
    r.worktree = Some(worktree.clone());

    let merged = git
        .merge_in_worktree(&worktree, &branch_ref)
        .wrap_err("merge failed")?;
    let merge_commit = match merged {
        MergeOutcome::Conflict { files } => return Ok(RepoOutcome::Conflict { files }),
        MergeOutcome::UpToDate => return Ok(RepoOutcome::UpToDate { uat: uat_before }),
        MergeOutcome::Merged { commit } => commit,
    };

    // The overlay guard, on SHAs.
    let packages: Vec<&str> = r.overlay_packages.iter().map(String::as_str).collect();
    if let Err(e) = check_range_for_overlay(&git, &uat_before, &merge_commit, &packages) {
        return Ok(RepoOutcome::Blocked {
            reason: e.to_string(),
        });
    }
    if working_tree_has_overlay(&worktree, &packages)? {
        return Ok(RepoOutcome::Blocked {
            reason: "the integration worktree carries the test overlay".into(),
        });
    }
    if let Some(b) = overlays::find_by_dir(ctx.store, &worktree)? {
        return Ok(RepoOutcome::Blocked {
            reason: format!(
                "an overlay of {} is recorded for the integration worktree",
                b.ticket
            ),
        });
    }

    // Configured local checks, run in the worktree.
    if !repo.manifest.integrate.checks.is_empty() {
        let project = Project::from_parts(worktree.clone(), repo.manifest.clone());
        for check in &repo.manifest.integrate.checks {
            let Some(task) = project.resolve_task(check)? else {
                return Ok(RepoOutcome::Blocked {
                    reason: format!("check task '{check}' is not defined"),
                });
            };
            let command = ExternalCommand::from_command(
                &task.to_command(&[]),
                &worktree,
                format!("check {check}"),
            );
            if let Err(e) = run_checked(ctx.runner, &command) {
                return Ok(RepoOutcome::Blocked {
                    reason: format!("check '{check}' failed: {e:#}"),
                });
            }
        }
    }

    let commits = git
        .commits_not_in(&uat_before, &tip)?
        .into_iter()
        .map(|c| CommitLine {
            sha: c.sha,
            summary: c.summary,
        })
        .collect();
    let files = git
        .diff(&uat_before, &merge_commit)?
        .files
        .into_iter()
        .map(|f| f.path)
        .collect();
    Ok(RepoOutcome::Ready(ReadyMerge {
        merge_commit,
        uat_before,
        commits,
        files,
    }))
}

/// Removes the temporary worktrees (and temporary branches) of `ticket`. Safe to call any time.
pub fn cancel_integration(
    store: &Store,
    data_dir: &Path,
    ticket: &TicketKey,
    repos: &[WorkspaceRepo],
    now: i64,
) -> eyre::Result<Vec<String>> {
    let base = integration_dir(data_dir, ticket);
    let mut removed = Vec::new();
    for repo in repos {
        let path = base.join(&repo.name);
        if path.exists() {
            remove_worktree(&repo.dir, &path, &temp_branch(ticket));
            removed.push(repo.name.clone());
        } else {
            // The branch can outlive a hand-deleted directory.
            let _ = GitRunner::new(&repo.dir).run_raw(&["worktree", "prune"]);
            let _ = GitRunner::new(&repo.dir).run_raw(&["branch", "-D", &temp_branch(ticket)]);
        }
    }
    if base.exists() {
        let _ = std::fs::remove_dir_all(&base);
    }
    if !removed.is_empty() {
        audit_local(
            store,
            now,
            actions::CANCELLED,
            ticket,
            json!({ "removed": removed }),
            AuditOutcome::Success,
        )?;
    }
    Ok(removed)
}

/// Pushes what `confirmed` (from [`IntegrationPrep::push_action`]) describes, through the
/// gateway. Each merge is recorded right after its push.
pub fn execute_push(
    gateway: &Gateway<'_>,
    confirmed: Confirmed,
    now: i64,
) -> Result<(PushReport, Executed), crate::gateway::GatewayError> {
    if !matches!(confirmed.action(), Action::PushUat(_)) {
        return Err(crate::gateway::GatewayError::Precondition(
            "not a push action".into(),
        ));
    }
    let mut executed = gateway.execute(confirmed, now)?;
    let report = match &executed.outcome {
        Outcome::Pushed(r) => r.clone(),
        _ => {
            return Err(crate::gateway::GatewayError::Precondition(
                "a push action yielded no push report".into(),
            ));
        }
    };
    executed.audit_warnings.dedup();
    Ok((report, executed))
}

#[derive(Debug)]
pub struct FinalizeReport {
    pub deactivation: DeactivationReport,
    /// Repos recorded as already in `uat` (nothing was pushed for them).
    pub already_in_uat: Vec<String>,
}

/// The last step: once every touched repo of `prep` is pushed (or was already in `uat`),
/// deactivates the ticket to `Integrated`, which reverts overlays and restores branches. The
/// only way a ticket becomes `Integrated`. Refuses (changing nothing) otherwise.
pub fn finalize_integration(
    store: &Store,
    runner: &dyn CommandRunner,
    data_dir: &Path,
    prep: &IntegrationPrep,
    repos: &[WorkspaceRepo],
    now: i64,
) -> eyre::Result<FinalizeReport> {
    let ticket = &prep.ticket;
    let recorded = uat_details::list_for_ticket(store, ticket)?;

    // Verify everything before recording or changing anything.
    let mut to_record = Vec::new();
    for r in &prep.repos {
        match &r.outcome {
            RepoOutcome::Ready(m) => {
                if !recorded
                    .iter()
                    .any(|d| d.merge.repo == r.repo && d.merge.commit == m.merge_commit)
                {
                    bail!(
                        "{} has not been pushed (no recorded merge of {}); push first",
                        r.repo,
                        m.merge_commit
                    );
                }
            }
            RepoOutcome::AlreadyPushed { .. } => {}
            RepoOutcome::UpToDate { uat } => to_record.push((r, uat.clone())),
            RepoOutcome::Conflict { .. } | RepoOutcome::Blocked { .. } => {
                bail!(
                    "{} is not integrated ({:?}); cannot finalize",
                    r.repo,
                    r.outcome
                )
            }
        }
    }

    let mut already = Vec::new();
    for (r, uat) in to_record {
        uat_details::record(
            store,
            &UatMerge {
                ticket: ticket.clone(),
                repo: r.repo.clone(),
                branch: r.uat_branch.clone(),
                commit: uat.clone(),
                recorded_at: now,
            },
            &MergeDetails {
                kind: MergeKind::AlreadyInUat,
                ticket_branch: r.ticket_branch.clone(),
                ticket_tip: r.ticket_tip.clone().unwrap_or_default(),
                uat_before: uat,
            },
        )?;
        already.push(r.repo.clone());
    }

    let deactivation = deactivate_for_integration(store, runner, ticket, now)?;
    audit_local(
        store,
        now,
        actions::FINALIZED,
        ticket,
        json!({
            "complete": deactivation.is_complete(),
            "already_in_uat": already,
        }),
        if deactivation.is_complete() {
            AuditOutcome::Success
        } else {
            AuditOutcome::Failure
        },
    )?;
    let _ = cancel_integration(store, data_dir, ticket, repos, now);
    Ok(FinalizeReport {
        deactivation,
        already_in_uat: already,
    })
}
