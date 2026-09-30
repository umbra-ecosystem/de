//! Records of the merge commits pushed to `uat` (`state.db`).
//!
//! Written by the integration flow (M5) after a push; read by sync, which looks up the
//! pipeline of each recorded commit. `repo` is the workspace project name, as in
//! [`super::links`], not the hosting path.

use eyre::Context;
use rusqlite::params;

use super::Store;
use crate::domain::TicketKey;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UatMerge {
    pub ticket: TicketKey,
    pub repo: String,
    /// The integration branch pushed (usually `uat`).
    pub branch: String,
    /// The full SHA of the merge commit that was pushed.
    pub commit: String,
    pub recorded_at: i64,
}

/// Records a pushed merge. Recording the same `(ticket, repo, commit)` again is a no-op.
pub fn record(store: &Store, merge: &UatMerge) -> eyre::Result<()> {
    store
        .conn()
        .execute(
            "INSERT OR IGNORE INTO uat_merges (ticket_key, repo, branch, commit_sha, recorded_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                merge.ticket,
                merge.repo,
                merge.branch,
                merge.commit,
                merge.recorded_at
            ],
        )
        .wrap_err_with(|| format!("Failed to record the uat merge of {}", merge.ticket))?;
    Ok(())
}

fn query(store: &Store, sql: &str, params: impl rusqlite::Params) -> eyre::Result<Vec<UatMerge>> {
    let mut stmt = store.conn().prepare(sql)?;
    let rows = stmt
        .query_map(params, |r| {
            Ok(UatMerge {
                ticket: r.get(0)?,
                repo: r.get(1)?,
                branch: r.get(2)?,
                commit: r.get(3)?,
                recorded_at: r.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()
        .wrap_err("Failed to read uat merges")?;
    Ok(rows)
}

const SELECT: &str = "SELECT ticket_key, repo, branch, commit_sha, recorded_at FROM uat_merges";

/// The merges recorded for a ticket, oldest first.
pub fn list_for_ticket(store: &Store, ticket: &TicketKey) -> eyre::Result<Vec<UatMerge>> {
    query(
        store,
        &format!("{SELECT} WHERE ticket_key = ?1 ORDER BY id"),
        params![ticket],
    )
}

/// Every recorded merge, oldest first.
pub fn list(store: &Store) -> eyre::Result<Vec<UatMerge>> {
    query(store, &format!("{SELECT} ORDER BY id"), [])
}
