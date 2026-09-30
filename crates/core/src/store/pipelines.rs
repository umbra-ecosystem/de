//! Mirror of Bitbucket pipeline runs and their steps (`cache.db`).

use eyre::Context;
use rusqlite::{Row, params};

use super::Store;
use crate::providers::{PipelineRun, PipelineState, PipelineStep, commit_matches};

const COLUMNS: &str = "repo, id, number, state, branch, commit_sha, created_at, completed_at, url";

fn from_row(r: &Row<'_>) -> rusqlite::Result<PipelineRun> {
    let state: String = r.get(3)?;
    Ok(PipelineRun {
        repo: r.get(0)?,
        id: r.get(1)?,
        number: r.get::<_, Option<i64>>(2)?.map(|n| n as u64),
        state: PipelineState::from_db(&state),
        branch: r.get(4)?,
        commit: r.get(5)?,
        created_at: r.get(6)?,
        completed_at: r.get(7)?,
        url: r.get(8)?,
        steps: Vec::new(),
    })
}

fn with_steps(cache: &Store, mut runs: Vec<PipelineRun>) -> eyre::Result<Vec<PipelineRun>> {
    let mut stmt = cache.conn().prepare(
        "SELECT name, state, deployment_environment FROM pipeline_steps
         WHERE repo = ?1 AND run_id = ?2 ORDER BY position",
    )?;
    for run in &mut runs {
        run.steps = stmt
            .query_map(params![run.repo, run.id], |r| {
                let state: String = r.get(1)?;
                Ok(PipelineStep {
                    name: r.get(0)?,
                    state: PipelineState::from_db(&state),
                    deployment_environment: r.get(2)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
    }
    Ok(runs)
}

fn query(
    cache: &Store,
    sql: &str,
    params: impl rusqlite::Params,
) -> eyre::Result<Vec<PipelineRun>> {
    let mut stmt = cache.conn().prepare(sql)?;
    let runs = stmt
        .query_map(params, from_row)?
        .collect::<Result<Vec<_>, _>>()
        .wrap_err("Failed to read cached pipeline runs")?;
    with_steps(cache, runs)
}

/// Inserts or replaces a run and its steps. `now` is when it was fetched.
///
/// A run without steps (as list endpoints often return) never erases the steps already
/// cached for it: they came from a detail fetch and are only replaced by another one.
pub fn upsert(cache: &Store, run: &PipelineRun, now: i64) -> eyre::Result<()> {
    let tx = cache.conn().unchecked_transaction()?;
    tx.execute(
        "INSERT INTO pipeline_runs (repo, id, number, state, branch, commit_sha, created_at, completed_at, url, fetched_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT (repo, id) DO UPDATE SET
            number = excluded.number,
            state = excluded.state,
            branch = excluded.branch,
            commit_sha = excluded.commit_sha,
            created_at = excluded.created_at,
            completed_at = excluded.completed_at,
            url = excluded.url,
            fetched_at = excluded.fetched_at",
        params![
            run.repo,
            run.id,
            run.number.map(|n| n as i64),
            run.state.to_db(),
            run.branch,
            run.commit,
            run.created_at,
            run.completed_at,
            run.url,
            now
        ],
    )?;
    if !run.steps.is_empty() {
        tx.execute(
            "DELETE FROM pipeline_steps WHERE repo = ?1 AND run_id = ?2",
            params![run.repo, run.id],
        )?;
        for (position, step) in run.steps.iter().enumerate() {
            tx.execute(
                "INSERT INTO pipeline_steps (repo, run_id, position, name, state, deployment_environment)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    run.repo,
                    run.id,
                    position as i64,
                    step.name,
                    step.state.to_db(),
                    step.deployment_environment
                ],
            )?;
        }
    }
    tx.commit()
        .wrap_err_with(|| format!("Failed to cache pipeline {} {}", run.repo, run.id))
}

pub fn get(cache: &Store, repo: &str, id: &str) -> eyre::Result<Option<PipelineRun>> {
    Ok(query(
        cache,
        &format!("SELECT {COLUMNS} FROM pipeline_runs WHERE repo = ?1 AND id = ?2"),
        params![repo, id],
    )?
    .pop())
}

/// Every cached run of `repo`, newest first.
pub fn list_for_repo(cache: &Store, repo: &str) -> eyre::Result<Vec<PipelineRun>> {
    query(
        cache,
        &format!(
            "SELECT {COLUMNS} FROM pipeline_runs WHERE repo = ?1 ORDER BY created_at DESC, id"
        ),
        params![repo],
    )
}

/// The cached runs of `repo` on `branch`, newest first.
pub fn for_branch(cache: &Store, repo: &str, branch: &str) -> eyre::Result<Vec<PipelineRun>> {
    query(
        cache,
        &format!(
            "SELECT {COLUMNS} FROM pipeline_runs WHERE repo = ?1 AND branch = ?2
             ORDER BY created_at DESC, id"
        ),
        params![repo, branch],
    )
}

/// The cached runs of `repo` that built `commit` (a full or abbreviated SHA of at least 7
/// characters, matched either way round), newest first.
///
/// This is the run *for that commit*. "Any later run that contains it" needs git ancestry
/// and is the caller's job.
pub fn for_commit(cache: &Store, repo: &str, commit: &str) -> eyre::Result<Vec<PipelineRun>> {
    Ok(list_for_repo(cache, repo)?
        .into_iter()
        .filter(|r| commit_matches(&r.commit, commit))
        .collect())
}
