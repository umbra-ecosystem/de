//! Mirror of Bitbucket pull requests, their reviewers and comments (`cache.db`).
//!
//! Upserts are idempotent: writing the same PR twice leaves one row and one reviewer set.
//! Nothing here deletes on its own; sync decides when a PR is gone (see [`delete`]) and only
//! after a successful fetch.

use eyre::Context;
use rusqlite::{Row, params};

use super::Store;
use crate::domain::TicketKey;
use crate::providers::{DiffSide, InlineAnchor, Pr, PrComment, PrState, Reviewer};

const PR_COLUMNS: &str =
    "repo, id, title, state, source_branch, destination_branch, author, url, updated_at";

fn state_column(text: &str) -> rusqlite::Result<PrState> {
    PrState::parse(text).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            format!("unknown PR state {text:?}").into(),
        )
    })
}

fn pr_from_row(r: &Row<'_>) -> rusqlite::Result<Pr> {
    let state: String = r.get(3)?;
    Ok(Pr {
        repo: r.get(0)?,
        id: r.get::<_, i64>(1)? as u64,
        title: r.get(2)?,
        state: state_column(&state)?,
        source_branch: r.get(4)?,
        destination_branch: r.get(5)?,
        author: r.get(6)?,
        reviewers: Vec::new(),
        url: r.get(7)?,
        updated_at: r.get(8)?,
    })
}

/// Fills in the reviewers of each PR.
fn with_reviewers(cache: &Store, mut prs: Vec<Pr>) -> eyre::Result<Vec<Pr>> {
    let mut stmt = cache.conn().prepare(
        "SELECT account, approved, changes_requested FROM pr_reviewers
         WHERE repo = ?1 AND pr_id = ?2 ORDER BY account",
    )?;
    for pr in &mut prs {
        pr.reviewers = stmt
            .query_map(params![pr.repo, pr.id as i64], |r| {
                Ok(Reviewer {
                    account: r.get(0)?,
                    approved: r.get::<_, i64>(1)? != 0,
                    changes_requested: r.get::<_, i64>(2)? != 0,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
    }
    Ok(prs)
}

fn query_prs(cache: &Store, sql: &str, params: impl rusqlite::Params) -> eyre::Result<Vec<Pr>> {
    let mut stmt = cache.conn().prepare(sql)?;
    let prs = stmt
        .query_map(params, pr_from_row)?
        .collect::<Result<Vec<_>, _>>()
        .wrap_err("Failed to read cached pull requests")?;
    with_reviewers(cache, prs)
}

/// Inserts or replaces a PR and its reviewers. `now` is when it was fetched.
pub fn upsert(cache: &Store, pr: &Pr, now: i64) -> eyre::Result<()> {
    let tx = cache.conn().unchecked_transaction()?;
    tx.execute(
        "INSERT INTO prs (repo, id, title, state, source_branch, destination_branch, author, url, updated_at, fetched_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT (repo, id) DO UPDATE SET
            title = excluded.title,
            state = excluded.state,
            source_branch = excluded.source_branch,
            destination_branch = excluded.destination_branch,
            author = excluded.author,
            url = excluded.url,
            updated_at = excluded.updated_at,
            fetched_at = excluded.fetched_at",
        params![
            pr.repo,
            pr.id as i64,
            pr.title,
            pr.state.as_str(),
            pr.source_branch,
            pr.destination_branch,
            pr.author,
            pr.url,
            pr.updated_at,
            now
        ],
    )?;
    tx.execute(
        "DELETE FROM pr_reviewers WHERE repo = ?1 AND pr_id = ?2",
        params![pr.repo, pr.id as i64],
    )?;
    for reviewer in &pr.reviewers {
        tx.execute(
            "INSERT OR REPLACE INTO pr_reviewers (repo, pr_id, account, approved, changes_requested)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                pr.repo,
                pr.id as i64,
                reviewer.account,
                reviewer.approved,
                reviewer.changes_requested
            ],
        )?;
    }
    tx.commit()
        .wrap_err_with(|| format!("Failed to cache PR {} #{}", pr.repo, pr.id))
}

pub fn get(cache: &Store, repo: &str, id: u64) -> eyre::Result<Option<Pr>> {
    Ok(query_prs(
        cache,
        &format!("SELECT {PR_COLUMNS} FROM prs WHERE repo = ?1 AND id = ?2"),
        params![repo, id as i64],
    )?
    .pop())
}

/// Every cached PR of `repo`, by id.
pub fn list_for_repo(cache: &Store, repo: &str) -> eyre::Result<Vec<Pr>> {
    query_prs(
        cache,
        &format!("SELECT {PR_COLUMNS} FROM prs WHERE repo = ?1 ORDER BY id"),
        params![repo],
    )
}

/// The cached PRs of `repo` in `state`, by id.
pub fn list_by_state(cache: &Store, repo: &str, state: PrState) -> eyre::Result<Vec<Pr>> {
    query_prs(
        cache,
        &format!("SELECT {PR_COLUMNS} FROM prs WHERE repo = ?1 AND state = ?2 ORDER BY id"),
        params![repo, state.as_str()],
    )
}

/// Every cached PR in any repo, by repo and id.
pub fn list_all(cache: &Store) -> eyre::Result<Vec<Pr>> {
    query_prs(
        cache,
        &format!("SELECT {PR_COLUMNS} FROM prs ORDER BY repo, id"),
        [],
    )
}

/// Whether `pr` belongs to `key`: the key appears (as a whole token, see
/// [`TicketKey::find_in`]) in its source branch or its title. `PROJ-1` never matches
/// `PROJ-12`.
pub fn pr_mentions(pr: &Pr, key: &TicketKey) -> bool {
    TicketKey::find_in(&pr.source_branch).contains(key)
        || TicketKey::find_in(&pr.title).contains(key)
}

/// The cached PRs of a ticket, in any state, by repo and id.
pub fn for_ticket(cache: &Store, key: &TicketKey) -> eyre::Result<Vec<Pr>> {
    // SQL narrows to candidates that contain the key text at all; the token-boundary rule
    // is then applied in Rust so it is exactly `TicketKey::find_in`.
    let candidates = query_prs(
        cache,
        &format!(
            "SELECT {PR_COLUMNS} FROM prs
             WHERE instr(upper(source_branch), ?1) > 0 OR instr(upper(title), ?1) > 0
             ORDER BY repo, id"
        ),
        params![key.as_str()],
    )?;
    Ok(candidates
        .into_iter()
        .filter(|pr| pr_mentions(pr, key))
        .collect())
}

/// The open PRs of a ticket; what the hotfix rule and review use.
pub fn open_for_ticket(cache: &Store, key: &TicketKey) -> eyre::Result<Vec<Pr>> {
    Ok(for_ticket(cache, key)?
        .into_iter()
        .filter(|pr| pr.state == PrState::Open)
        .collect())
}

/// Forgets a PR and, through the foreign keys, its reviewers and comments. Returns whether
/// a row existed.
pub fn delete(cache: &Store, repo: &str, id: u64) -> eyre::Result<bool> {
    let n = cache.conn().execute(
        "DELETE FROM prs WHERE repo = ?1 AND id = ?2",
        params![repo, id as i64],
    )?;
    Ok(n > 0)
}

fn comment_from_row(r: &Row<'_>) -> rusqlite::Result<PrComment> {
    let path: Option<String> = r.get(4)?;
    let line: Option<i64> = r.get(5)?;
    let side: Option<String> = r.get(6)?;
    let inline = match (path, line, side.as_deref().and_then(DiffSide::parse)) {
        (Some(path), Some(line), Some(side)) => Some(InlineAnchor {
            path,
            line: line as u32,
            side,
        }),
        _ => None,
    };
    Ok(PrComment {
        id: r.get::<_, i64>(0)? as u64,
        pr: r.get::<_, i64>(1)? as u64,
        author: r.get(2)?,
        body: r.get(3)?,
        inline,
        created_at: r.get(7)?,
    })
}

/// Makes the cached comments of a PR exactly `comments` (one transaction). The PR row must
/// exist. Comments missing from `comments` are removed: call this only with a complete,
/// successfully fetched list.
pub fn replace_comments(
    cache: &Store,
    repo: &str,
    pr_id: u64,
    comments: &[PrComment],
) -> eyre::Result<()> {
    let tx = cache.conn().unchecked_transaction()?;
    tx.execute(
        "DELETE FROM pr_comments WHERE repo = ?1 AND pr_id = ?2",
        params![repo, pr_id as i64],
    )?;
    for c in comments {
        tx.execute(
            "INSERT OR REPLACE INTO pr_comments
                (repo, pr_id, id, author, body, inline_path, inline_line, inline_side, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                repo,
                pr_id as i64,
                c.id as i64,
                c.author,
                c.body,
                c.inline.as_ref().map(|i| i.path.as_str()),
                c.inline.as_ref().map(|i| i.line),
                c.inline.as_ref().map(|i| i.side.as_str()),
                c.created_at
            ],
        )?;
    }
    tx.commit()
        .wrap_err_with(|| format!("Failed to cache the comments of {repo} #{pr_id}"))
}

/// The cached comments of a PR, oldest first.
pub fn comments(cache: &Store, repo: &str, pr_id: u64) -> eyre::Result<Vec<PrComment>> {
    let mut stmt = cache.conn().prepare(
        "SELECT id, pr_id, author, body, inline_path, inline_line, inline_side, created_at
         FROM pr_comments WHERE repo = ?1 AND pr_id = ?2 ORDER BY created_at, id",
    )?;
    let rows = stmt
        .query_map(params![repo, pr_id as i64], comment_from_row)?
        .collect::<Result<Vec<_>, _>>()
        .wrap_err("Failed to read cached PR comments")?;
    Ok(rows)
}
