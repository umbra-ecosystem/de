//! What activating a ticket did to each repo (`state.db`), so deactivating can undo it.
//!
//! A record is written right after a repo is switched and removed only once the repo has
//! been restored. The whole [`StashRef`] is stored (label and commit): stash indexes shift.

use std::path::PathBuf;

use eyre::{Context, eyre};
use rusqlite::{Row, params};

use super::Store;
use crate::{domain::TicketKey, git::StashRef};

/// Why a repo was switched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepoRole {
    /// The repo has the ticket's branch.
    Ticket,
    /// The repo does not touch the ticket and went to a baseline branch.
    Baseline,
}

impl RepoRole {
    pub fn as_str(self) -> &'static str {
        match self {
            RepoRole::Ticket => "ticket",
            RepoRole::Baseline => "baseline",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        match text {
            "ticket" => Some(RepoRole::Ticket),
            "baseline" => Some(RepoRole::Baseline),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreRecord {
    pub ticket: TicketKey,
    pub repo: String,
    /// Order in which the repos were switched, 0-based.
    pub position: i64,
    pub repo_dir: PathBuf,
    pub role: RepoRole,
    /// The branch activation put the repo on.
    pub branch: String,
    /// `None` when HEAD was detached (see `previous_commit`).
    pub previous_branch: Option<String>,
    pub previous_commit: Option<String>,
    /// The stash made for a dirty tree, to pop when restoring.
    pub stash: Option<StashRef>,
    pub created_at: i64,
}

const COLUMNS: &str = "ticket_key, repo, position, repo_dir, role, branch, previous_branch, \
                       previous_commit, stash_label, stash_commit, created_at";

fn from_row(r: &Row<'_>) -> rusqlite::Result<RestoreRecord> {
    let role: String = r.get(4)?;
    let stash_label: Option<String> = r.get(8)?;
    let stash_commit: Option<String> = r.get(9)?;
    let repo_dir: String = r.get(3)?;
    Ok(RestoreRecord {
        ticket: r.get(0)?,
        repo: r.get(1)?,
        position: r.get(2)?,
        repo_dir: PathBuf::from(repo_dir),
        role: RepoRole::parse(&role).ok_or_else(|| {
            rusqlite::Error::FromSqlConversionFailure(
                4,
                rusqlite::types::Type::Text,
                format!("unknown repo role {role:?}").into(),
            )
        })?,
        branch: r.get(5)?,
        previous_branch: r.get(6)?,
        previous_commit: r.get(7)?,
        stash: stash_label
            .zip(stash_commit)
            .map(|(label, commit)| StashRef { label, commit }),
        created_at: r.get(10)?,
    })
}

/// Records a switched repo. Errors if the ticket already has a record for `repo`.
pub fn insert(store: &Store, record: &RestoreRecord) -> eyre::Result<()> {
    let dir = record
        .repo_dir
        .to_str()
        .ok_or_else(|| eyre!("The path {} is not valid UTF-8", record.repo_dir.display()))?;
    store
        .conn()
        .execute(
            &format!(
                "INSERT INTO activation_repos ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)"
            ),
            params![
                record.ticket,
                record.repo,
                record.position,
                dir,
                record.role.as_str(),
                record.branch,
                record.previous_branch,
                record.previous_commit,
                record.stash.as_ref().map(|s| s.label.as_str()),
                record.stash.as_ref().map(|s| s.commit.as_str()),
                record.created_at,
            ],
        )
        .wrap_err_with(|| {
            format!(
                "Failed to record the restore point of {} for {}",
                record.repo, record.ticket
            )
        })?;
    Ok(())
}

/// The records of `ticket` in the order the repos were switched.
pub fn list(store: &Store, ticket: &TicketKey) -> eyre::Result<Vec<RestoreRecord>> {
    let mut stmt = store.conn().prepare(&format!(
        "SELECT {COLUMNS} FROM activation_repos WHERE ticket_key = ?1 ORDER BY position"
    ))?;
    let records = stmt
        .query_map(params![ticket], from_row)?
        .collect::<Result<Vec<_>, _>>()
        .wrap_err_with(|| format!("Failed to list the restore points of {ticket}"))?;
    Ok(records)
}

/// Every record of every ticket.
pub fn list_all(store: &Store) -> eyre::Result<Vec<RestoreRecord>> {
    let mut stmt = store.conn().prepare(&format!(
        "SELECT {COLUMNS} FROM activation_repos ORDER BY ticket_key, position"
    ))?;
    let records = stmt
        .query_map([], from_row)?
        .collect::<Result<Vec<_>, _>>()
        .wrap_err("Failed to list the restore points")?;
    Ok(records)
}

/// Forgets the record of a repo that has been restored. Returns whether one existed.
pub fn delete(store: &Store, ticket: &TicketKey, repo: &str) -> eyre::Result<bool> {
    let removed = store
        .conn()
        .execute(
            "DELETE FROM activation_repos WHERE ticket_key = ?1 AND repo = ?2",
            params![ticket, repo],
        )
        .wrap_err_with(|| format!("Failed to clear the restore point of {repo} for {ticket}"))?;
    Ok(removed > 0)
}
