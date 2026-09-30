//! Backups of the files the test overlay rewrites (`state.db`).
//!
//! The overlay is deliberately temporary, and the only copy of the original
//! `composer.json` and `composer.lock` lives here while it is applied, so this table is what
//! makes a revert possible after a crash.

use std::path::PathBuf;

use eyre::{Context, bail, eyre};
use rusqlite::{OptionalExtension, Row, params};

use super::Store;
use crate::domain::TicketKey;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlayBackup {
    pub ticket: TicketKey,
    pub repo: String,
    pub repo_dir: PathBuf,
    /// JSON description of what the overlay points where (informational).
    pub packages: String,
    /// Original `composer.json`, byte for byte.
    pub composer_json: Vec<u8>,
    /// Original `composer.lock`; `None` when there was none.
    pub composer_lock: Option<Vec<u8>>,
    /// `composer.json` as the overlay left it.
    pub json_after: Option<Vec<u8>>,
    /// `composer.lock` as the overlay left it (`None`: not written yet or absent).
    pub lock_after: Option<Vec<u8>>,
    /// The original files are back on disk; only `composer install` may remain to do.
    pub files_restored: bool,
    pub created_at: i64,
}

const COLUMNS: &str = "ticket_key, repo, repo_dir, packages, composer_json, composer_lock, \
                       json_after, lock_after, files_restored, created_at";

fn from_row(r: &Row<'_>) -> rusqlite::Result<OverlayBackup> {
    let dir: String = r.get(2)?;
    Ok(OverlayBackup {
        ticket: r.get(0)?,
        repo: r.get(1)?,
        repo_dir: PathBuf::from(dir),
        packages: r.get(3)?,
        composer_json: r.get(4)?,
        composer_lock: r.get(5)?,
        json_after: r.get(6)?,
        lock_after: r.get(7)?,
        files_restored: r.get(8)?,
        created_at: r.get(9)?,
    })
}

/// Stores a backup. Refuses (without touching the existing row) when this ticket and repo,
/// or any other ticket on the same checkout, already has one: overwriting the first backup
/// would make the overlay permanent.
pub fn insert(store: &Store, backup: &OverlayBackup) -> eyre::Result<()> {
    let dir = backup
        .repo_dir
        .to_str()
        .ok_or_else(|| eyre!("The path {} is not valid UTF-8", backup.repo_dir.display()))?;

    if let Some(existing) = find_by_dir(store, &backup.repo_dir)? {
        bail!(
            "The overlay of {} is already applied in {} (repo {}); revert it first",
            existing.ticket,
            backup.repo_dir.display(),
            existing.repo
        );
    }

    store
        .conn()
        .execute(
            &format!(
                "INSERT INTO overlay_backups ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)"
            ),
            params![
                backup.ticket,
                backup.repo,
                dir,
                backup.packages,
                backup.composer_json,
                backup.composer_lock,
                backup.json_after,
                backup.lock_after,
                backup.files_restored,
                backup.created_at,
            ],
        )
        .wrap_err_with(|| {
            format!(
                "Failed to back up the composer files of {} for {}",
                backup.repo, backup.ticket
            )
        })?;
    Ok(())
}

pub fn get(store: &Store, ticket: &TicketKey, repo: &str) -> eyre::Result<Option<OverlayBackup>> {
    store
        .conn()
        .query_row(
            &format!("SELECT {COLUMNS} FROM overlay_backups WHERE ticket_key = ?1 AND repo = ?2"),
            params![ticket, repo],
            from_row,
        )
        .optional()
        .wrap_err_with(|| format!("Failed to read the overlay backup of {repo} for {ticket}"))
}

/// The backup for a checkout, whichever ticket made it.
pub fn find_by_dir(store: &Store, dir: &std::path::Path) -> eyre::Result<Option<OverlayBackup>> {
    let dir = dir
        .to_str()
        .ok_or_else(|| eyre!("The path {} is not valid UTF-8", dir.display()))?;
    store
        .conn()
        .query_row(
            &format!("SELECT {COLUMNS} FROM overlay_backups WHERE repo_dir = ?1"),
            params![dir],
            from_row,
        )
        .optional()
        .wrap_err("Failed to read the overlay backup of a checkout")
}

pub fn list(store: &Store, ticket: &TicketKey) -> eyre::Result<Vec<OverlayBackup>> {
    query_list(
        store,
        &format!(
            "SELECT {COLUMNS} FROM overlay_backups WHERE ticket_key = ?1 ORDER BY created_at, repo"
        ),
        params![ticket],
    )
}

/// Every applied overlay of every ticket.
pub fn list_all(store: &Store) -> eyre::Result<Vec<OverlayBackup>> {
    query_list(
        store,
        &format!("SELECT {COLUMNS} FROM overlay_backups ORDER BY ticket_key, created_at, repo"),
        params![],
    )
}

fn query_list(
    store: &Store,
    sql: &str,
    params: impl rusqlite::Params,
) -> eyre::Result<Vec<OverlayBackup>> {
    let mut stmt = store.conn().prepare(sql)?;
    let backups = stmt
        .query_map(params, from_row)?
        .collect::<Result<Vec<_>, _>>()
        .wrap_err("Failed to list overlay backups")?;
    Ok(backups)
}

/// Records what the overlay left in the composer files.
pub fn set_after(
    store: &Store,
    ticket: &TicketKey,
    repo: &str,
    json_after: &[u8],
    lock_after: Option<&[u8]>,
) -> eyre::Result<()> {
    store
        .conn()
        .execute(
            "UPDATE overlay_backups SET json_after = ?3, lock_after = ?4
             WHERE ticket_key = ?1 AND repo = ?2",
            params![ticket, repo, json_after, lock_after],
        )
        .wrap_err("Failed to record the state the overlay left behind")?;
    Ok(())
}

/// Notes that the original files are back on disk.
pub fn mark_files_restored(store: &Store, ticket: &TicketKey, repo: &str) -> eyre::Result<()> {
    store
        .conn()
        .execute(
            "UPDATE overlay_backups SET files_restored = 1 WHERE ticket_key = ?1 AND repo = ?2",
            params![ticket, repo],
        )
        .wrap_err("Failed to mark the composer files as restored")?;
    Ok(())
}

/// Forgets a backup once the overlay is fully reverted. Returns whether one existed.
pub fn delete(store: &Store, ticket: &TicketKey, repo: &str) -> eyre::Result<bool> {
    let removed = store
        .conn()
        .execute(
            "DELETE FROM overlay_backups WHERE ticket_key = ?1 AND repo = ?2",
            params![ticket, repo],
        )
        .wrap_err("Failed to delete the overlay backup")?;
    Ok(removed > 0)
}
