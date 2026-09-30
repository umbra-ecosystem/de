//! Deploy tracking: which of a ticket's `uat` merges have been deployed, from the recorded
//! merges (`state.db`) and the cached pipeline runs (`cache.db`).
//!
//! A pipeline run belongs to a merge when it built the merge commit, or any later commit that
//! contains it (another ticket's push to `uat` creates newer runs). The exact-commit run is
//! preferred. Ancestry is asked of git by SHA, so a run of an older commit never counts.

use std::collections::BTreeSet;

use crate::{
    activation::WorkspaceRepo,
    domain::TicketKey,
    git::GitRepo,
    providers::{PipelineRun, PipelineState},
    store::{Store, links, pipelines, uat_details},
    sync::HostedRepo,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeployState {
    /// No merge of this ticket has been pushed (or recorded) for the repo.
    NotPushed,
    /// Pushed; no pipeline for it is known yet (sync may not have seen it).
    Pending,
    Running,
    Deployed,
    Failed,
    /// Pushed, but the repo has no `[hosting]`, so its pipelines cannot be followed. Never
    /// counts as deployed.
    Untracked,
}

impl DeployState {
    pub fn as_str(self) -> &'static str {
        match self {
            DeployState::NotPushed => "not pushed",
            DeployState::Pending => "pending",
            DeployState::Running => "running",
            DeployState::Deployed => "deployed",
            DeployState::Failed => "failed",
            DeployState::Untracked => "untracked",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoDeploy {
    /// Workspace project name.
    pub repo: String,
    /// `workspace/slug` when the repo is hosted.
    pub hosting_repo: Option<String>,
    /// The environment that counts as deployed (`None`: any deployment step).
    pub environment: Option<String>,
    /// The recorded `uat` commit being followed.
    pub merge_commit: Option<String>,
    pub state: DeployState,
    /// The run the state is read from.
    pub run: Option<PipelineRun>,
}

fn run_state(run: &PipelineRun, environment: Option<&str>) -> DeployState {
    if run.deployed(environment) {
        return DeployState::Deployed;
    }
    let deploy_step_failed = run.steps.iter().any(|s| {
        s.deployment_environment
            .as_deref()
            .is_some_and(|e| environment.is_none_or(|want| e.eq_ignore_ascii_case(want)))
            && matches!(s.state, PipelineState::Failed | PipelineState::Stopped)
    });
    if deploy_step_failed || matches!(run.state, PipelineState::Failed | PipelineState::Stopped) {
        DeployState::Failed
    } else if run.state == PipelineState::Running {
        DeployState::Running
    } else {
        DeployState::Pending
    }
}

/// The runs that belong to `commit`: built it, or built a commit containing it.
fn runs_for_merge(
    cache: &Store,
    git: Option<&GitRepo>,
    hosting_repo: &str,
    commit: &str,
) -> eyre::Result<Vec<PipelineRun>> {
    let mut out = Vec::new();
    for run in pipelines::list_for_repo(cache, hosting_repo)? {
        let exact = run.is_for_commit(commit);
        let descendant = !exact
            && git.is_some_and(|g| {
                // A run of a commit this checkout does not know cannot be shown to contain it.
                g.is_ancestor(commit, &run.commit).unwrap_or(false)
            });
        if exact || descendant {
            out.push(run);
        }
    }
    Ok(out)
}

/// Picks the run and state that describe a merge, from its candidate runs.
fn best(
    runs: Vec<PipelineRun>,
    commit: &str,
    environment: Option<&str>,
) -> (DeployState, Option<PipelineRun>) {
    let mut scored: Vec<(DeployState, bool, PipelineRun)> = runs
        .into_iter()
        .map(|r| (run_state(&r, environment), r.is_for_commit(commit), r))
        .collect();
    // Oldest first among descendants: the closest to the merge.
    scored.sort_by_key(|(_, _, r)| r.created_at);

    let pick = |wanted: &[DeployState], scored: &[(DeployState, bool, PipelineRun)]| {
        scored
            .iter()
            .filter(|(s, _, _)| wanted.contains(s))
            .min_by_key(|(_, exact, _)| !*exact)
            .cloned()
    };
    // The exact-commit run wins among equals, else the oldest (closest to the merge).
    for wanted in [
        &[DeployState::Deployed][..],
        &[DeployState::Running],
        &[DeployState::Pending],
        &[DeployState::Failed],
    ] {
        if let Some((state, _, run)) = pick(wanted, &scored) {
            return (state, Some(run));
        }
    }
    (DeployState::Pending, None)
}

/// The deploy state of every repo the ticket touches (linked repos plus any with a recorded
/// merge), by workspace project name. `hosted` says where each project's pipelines live.
pub fn deploy_status(
    state: &Store,
    cache: &Store,
    ticket: &TicketKey,
    hosted: &[HostedRepo],
) -> eyre::Result<Vec<RepoDeploy>> {
    let merges = uat_details::list_for_ticket(state, ticket)?;

    let mut names: BTreeSet<String> = links::list_included(state, ticket)?
        .into_iter()
        .map(|l| l.repo)
        .collect();
    names.extend(merges.iter().map(|m| m.merge.repo.clone()));

    let mut out = Vec::new();
    for name in names {
        let host = hosted.iter().find(|h| h.project == name);
        let latest = merges.iter().rfind(|m| m.merge.repo == name);
        let mut deploy = RepoDeploy {
            repo: name.clone(),
            hosting_repo: host.map(|h| h.repo.clone()),
            environment: host.and_then(|h| h.deploy_environment.clone()),
            merge_commit: latest.map(|m| m.merge.commit.clone()),
            state: DeployState::NotPushed,
            run: None,
        };
        if let Some(latest) = latest {
            match host {
                None => deploy.state = DeployState::Untracked,
                Some(h) => {
                    let git = GitRepo::open(&h.dir).ok();
                    let runs = runs_for_merge(cache, git.as_ref(), &h.repo, &latest.merge.commit)?;
                    let (s, run) = best(runs, &latest.merge.commit, deploy.environment.as_deref());
                    deploy.state = s;
                    deploy.run = run;
                }
            }
        }
        out.push(deploy);
    }
    Ok(out)
}

/// Every repo the ticket touches is deployed. False when there are none.
pub fn all_deployed(
    state: &Store,
    cache: &Store,
    ticket: &TicketKey,
    hosted: &[HostedRepo],
) -> eyre::Result<bool> {
    let status = deploy_status(state, cache, ticket, hosted)?;
    Ok(!status.is_empty() && status.iter().all(|r| r.state == DeployState::Deployed))
}

/// Whether the ticket branch has commits that `uat` lacks after the recorded merge (a fix
/// after alpha): a reason to merge again. Compares by SHA against `origin/<uat>` when that
/// already contains the recorded merge, else against the merge itself. Both the local and
/// the remote ticket branch count. Call after a fetch for current data.
pub fn needs_remerge(
    state: &Store,
    ticket: &TicketKey,
    repo: &WorkspaceRepo,
) -> eyre::Result<bool> {
    let Some(latest) = uat_details::latest(state, ticket, &repo.name)? else {
        return Ok(false);
    };
    let Some(details) = latest.details else {
        return Ok(false);
    };
    let git = GitRepo::open(&repo.dir)?;

    let uat_ref = format!("refs/remotes/{}/{}", repo.remote(), latest.merge.branch);
    let base = match git.rev_parse(&uat_ref) {
        Ok(tip) if git.is_ancestor(&latest.merge.commit, &tip).unwrap_or(false) => tip,
        _ => latest.merge.commit.clone(),
    };

    for tip_ref in [
        format!("refs/heads/{}", details.ticket_branch),
        format!("refs/remotes/{}/{}", repo.remote(), details.ticket_branch),
    ] {
        let Ok(tip) = git.rev_parse(&tip_ref) else {
            continue;
        };
        if !git.commits_not_in(&base, &tip)?.is_empty() {
            return Ok(true);
        }
    }
    Ok(false)
}
