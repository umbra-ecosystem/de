//! Code-host sync: open PRs, their details and comments, and pipeline runs.

use std::collections::BTreeSet;

use super::SyncContext;
use super::hosted::HostedRepo;
use super::report::{Collector, SourceReport, Step, SyncSource, local, run_source};
use crate::domain::TicketKey;
use crate::providers::{
    CodeHost, PipelineFilter, PipelineRun, Pr, PrFilter, PrState, ProviderError,
};
use crate::store::{jira_cache, links, pipelines, prs, sync_state, tickets, uat_merges};

/// A `(hosting repo, commit)` pair whose pipeline runs sync should look up. The repo is the
/// code-host path (`workspace/slug`), the commit a full or abbreviated SHA.
pub type RepoCommit = (String, String);

/// How many runs to ask for per branch or commit.
const RUN_LIMIT: usize = 10;

/// The `(hosting repo, commit)` pairs of every `uat` merge recorded in `state.db`, for
/// repos that are hosted. Feed the result to [`sync_code_host`](super::sync_code_host).
pub fn uat_commits(
    state: &crate::store::Store,
    hosted: &[HostedRepo],
) -> eyre::Result<Vec<RepoCommit>> {
    let mut out: Vec<RepoCommit> = Vec::new();
    for merge in uat_merges::list(state)? {
        if let Some(h) = hosted.iter().find(|h| h.project == merge.repo) {
            let pair = (h.repo.clone(), merge.commit);
            if !out.contains(&pair) {
                out.push(pair);
            }
        }
    }
    Ok(out)
}

/// Refreshes the mirror of each hosted repo, one independent source per repo
/// (`bitbucket:workspace/slug`), and returns one report each. Reads only.
///
/// For each repo:
///
/// 1. **Open PRs**, kept only when their source branch or title contains the key of a
///    ticket of interest (tracked tickets plus every cached Jira ticket, which includes the
///    Review pool). Each kept PR is refreshed with `pr` (full reviewers) and `pr_comments`.
/// 2. **Reconciliation**, only when the open list was fetched: a cached open PR that is no
///    longer in it is looked up by id. Merged/declined/superseded is recorded; not found
///    removes the row; a still-open PR nobody is interested in any more is dropped; any
///    other error leaves the row alone.
/// 3. **Pipelines** for the branches of tracked tickets (their repo links and their PRs'
///    source branches) and for each `(repo, commit)` in `uat_commits`. A run without steps
///    gets them from `pipeline`.
///
/// An environmental error aborts that repo only; other repos still run.
pub fn sync_code_host(
    ctx: &SyncContext<'_>,
    host: &dyn CodeHost,
    repos: &[HostedRepo],
    uat_commits: &[RepoCommit],
) -> Vec<SourceReport> {
    repos
        .iter()
        .map(|repo| {
            run_source(
                ctx,
                SyncSource::Bitbucket,
                sync_state::code_host_source(&repo.repo),
                |c| repo_body(ctx, host, repo, uat_commits, c),
            )
        })
        .collect()
}

fn interest_keys(ctx: &SyncContext<'_>) -> eyre::Result<Vec<TicketKey>> {
    let mut keys: BTreeSet<TicketKey> = tickets::list(ctx.state)?
        .into_iter()
        .map(|t| t.key)
        .collect();
    keys.extend(jira_cache::list(ctx.cache)?.into_iter().map(|t| t.key));
    Ok(keys.into_iter().collect())
}

fn is_interesting(pr: &Pr, interest: &[TicketKey]) -> bool {
    interest.iter().any(|k| prs::pr_mentions(pr, k))
}

fn repo_body(
    ctx: &SyncContext<'_>,
    host: &dyn CodeHost,
    hosted: &HostedRepo,
    uat_commits: &[RepoCommit],
    c: &mut Collector,
) -> Step {
    let repo = hosted.repo.as_str();
    let interest = interest_keys(ctx).map_err(local)?;

    sync_prs(ctx, host, repo, &interest, c)?;
    sync_pipelines(ctx, host, hosted, uat_commits, c)
}

fn sync_prs(
    ctx: &SyncContext<'_>,
    host: &dyn CodeHost,
    repo: &str,
    interest: &[TicketKey],
    c: &mut Collector,
) -> Step {
    let listed = match host.list_prs(repo, &PrFilter::open()) {
        Ok(listed) => listed,
        Err(e) => return c.handle("open PRs", &e),
    };

    let wanted: Vec<Pr> = listed
        .into_iter()
        .filter(|p| p.state == PrState::Open && is_interesting(p, interest))
        .collect();
    let wanted_ids: BTreeSet<u64> = wanted.iter().map(|p| p.id).collect();

    for listed_pr in &wanted {
        prs::upsert(ctx.cache, listed_pr, ctx.now).map_err(local)?;
        c.counts.prs += 1;
        let subject = format!("{repo} #{}", listed_pr.id);

        match host.pr(repo, listed_pr.id) {
            Ok(full) => prs::upsert(ctx.cache, &full, ctx.now).map_err(local)?,
            Err(e) => c.handle(&subject, &e)?,
        }
        match host.pr_comments(repo, listed_pr.id) {
            Ok(list) => {
                prs::replace_comments(ctx.cache, repo, listed_pr.id, &list).map_err(local)?;
                c.counts.pr_comments += list.len();
            }
            Err(e) => c.handle(format!("comments of {subject}"), &e)?,
        }
    }

    // Reconcile cached open PRs that dropped out of the list.
    let cached_open = prs::list_by_state(ctx.cache, repo, PrState::Open).map_err(local)?;
    for cached in cached_open.iter().filter(|p| !wanted_ids.contains(&p.id)) {
        let subject = format!("{repo} #{}", cached.id);
        match host.pr(repo, cached.id) {
            Ok(now_pr) if now_pr.state == PrState::Open => {
                // Still open, but nobody cares about it any more.
                if !is_interesting(&now_pr, interest) {
                    prs::delete(ctx.cache, repo, cached.id).map_err(local)?;
                    c.counts.removed += 1;
                } else {
                    prs::upsert(ctx.cache, &now_pr, ctx.now).map_err(local)?;
                }
            }
            Ok(closed) => {
                prs::upsert(ctx.cache, &closed, ctx.now).map_err(local)?;
                c.counts.prs += 1;
            }
            Err(ProviderError::NotFound(_)) => {
                prs::delete(ctx.cache, repo, cached.id).map_err(local)?;
                c.counts.removed += 1;
            }
            Err(e) => c.handle(subject, &e)?,
        }
    }
    Ok(())
}

/// Whether the steps of `run` must be fetched: it has none, and the cache does not already
/// hold the same finished state with steps.
fn needs_detail(ctx: &SyncContext<'_>, run: &PipelineRun) -> eyre::Result<bool> {
    if !run.steps.is_empty() {
        return Ok(false);
    }
    Ok(match pipelines::get(ctx.cache, &run.repo, &run.id)? {
        Some(cached) => {
            !(cached.state == run.state && cached.state.is_finished() && !cached.steps.is_empty())
        }
        None => true,
    })
}

fn cache_runs(
    ctx: &SyncContext<'_>,
    host: &dyn CodeHost,
    repo: &str,
    runs: Vec<PipelineRun>,
    c: &mut Collector,
) -> Step {
    for run in runs {
        let detail = needs_detail(ctx, &run).map_err(local)?;
        pipelines::upsert(ctx.cache, &run, ctx.now).map_err(local)?;
        c.counts.pipelines += 1;
        if detail {
            match host.pipeline(repo, &run.id) {
                Ok(full) => pipelines::upsert(ctx.cache, &full, ctx.now).map_err(local)?,
                Err(e) => c.handle(format!("pipeline {repo} {}", run.id), &e)?,
            }
        }
    }
    Ok(())
}

fn sync_pipelines(
    ctx: &SyncContext<'_>,
    host: &dyn CodeHost,
    hosted: &HostedRepo,
    uat_commits: &[RepoCommit],
    c: &mut Collector,
) -> Step {
    let repo = hosted.repo.as_str();

    // (a) Branches of tracked tickets: their links here, and their PRs' source branches.
    let mut branches: BTreeSet<String> = BTreeSet::new();
    for tracking in tickets::list(ctx.state).map_err(local)? {
        for link in links::list_included(ctx.state, &tracking.key).map_err(local)? {
            if link.repo == hosted.project
                && let Some(branch) = link.branch
            {
                branches.insert(branch);
            }
        }
        for pr in prs::for_ticket(ctx.cache, &tracking.key).map_err(local)? {
            if pr.repo == repo {
                branches.insert(pr.source_branch);
            }
        }
    }
    for branch in branches {
        let filter = PipelineFilter {
            branch: Some(branch.clone()),
            commit: None,
            limit: Some(RUN_LIMIT),
        };
        match host.pipelines(repo, &filter) {
            Ok(runs) => cache_runs(ctx, host, repo, runs, c)?,
            Err(e) => c.handle(format!("pipelines of {repo} {branch}"), &e)?,
        }
    }

    // (b) Commits recorded as pushed to uat.
    let commits: BTreeSet<&str> = uat_commits
        .iter()
        .filter(|(r, _)| r.eq_ignore_ascii_case(repo))
        .map(|(_, commit)| commit.as_str())
        .collect();
    for commit in commits {
        let filter = PipelineFilter {
            branch: None,
            commit: Some(commit.to_string()),
            limit: Some(RUN_LIMIT),
        };
        let short = &commit[..commit.len().min(8)];
        match host.pipelines(repo, &filter) {
            Ok(runs) => cache_runs(ctx, host, repo, runs, c)?,
            Err(e) => c.handle(format!("pipelines of {repo} for {short}"), &e)?,
        }
    }
    Ok(())
}
