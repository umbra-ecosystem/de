//! Per-source sync bookkeeping (`cache.db`): when each source last succeeded, was last
//! tried, and why the last try failed.
//!
//! Source keys are opaque strings: [`JIRA`] and [`code_host_source`]`(repo)`.

use eyre::Context;
use rusqlite::{OptionalExtension, params};

use super::Store;

/// The source key of the Jira sync.
pub const JIRA: &str = "jira";

/// The source key of the sync of one hosted repo (`workspace/slug`).
pub fn code_host_source(repo: &str) -> String {
    format!("bitbucket:{repo}")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncState {
    pub source: String,
    /// The last time the source synced completely; `None` if it never did.
    pub last_ok_at: Option<i64>,
    pub last_attempt_at: i64,
    /// Why the last attempt did not fully succeed; cleared by the next success.
    pub last_error: Option<String>,
}

impl SyncState {
    /// Whether the source synced completely within `min_interval` seconds of `now`.
    pub fn is_fresh(&self, now: i64, min_interval: i64) -> bool {
        self.last_ok_at
            .is_some_and(|ok| ok <= now && now - ok < min_interval)
    }
}

pub fn get(cache: &Store, source: &str) -> eyre::Result<Option<SyncState>> {
    cache
        .conn()
        .query_row(
            "SELECT source, last_ok_at, last_attempt_at, last_error FROM sync_state WHERE source = ?1",
            params![source],
            |r| {
                Ok(SyncState {
                    source: r.get(0)?,
                    last_ok_at: r.get(1)?,
                    last_attempt_at: r.get(2)?,
                    last_error: r.get(3)?,
                })
            },
        )
        .optional()
        .wrap_err_with(|| format!("Failed to read the sync state of {source}"))
}

/// Every source with a recorded state, by name.
pub fn list(cache: &Store) -> eyre::Result<Vec<SyncState>> {
    let mut stmt = cache.conn().prepare(
        "SELECT source, last_ok_at, last_attempt_at, last_error FROM sync_state ORDER BY source",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(SyncState {
                source: r.get(0)?,
                last_ok_at: r.get(1)?,
                last_attempt_at: r.get(2)?,
                last_error: r.get(3)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()
        .wrap_err("Failed to list sync states")?;
    Ok(rows)
}

/// Whether `source` synced completely within `min_interval` seconds of `now`.
pub fn is_fresh(cache: &Store, source: &str, now: i64, min_interval: i64) -> eyre::Result<bool> {
    Ok(get(cache, source)?.is_some_and(|s| s.is_fresh(now, min_interval)))
}

/// Records a complete, successful sync at `now`; clears the last error.
pub fn record_ok(cache: &Store, source: &str, now: i64) -> eyre::Result<()> {
    cache
        .conn()
        .execute(
            "INSERT INTO sync_state (source, last_ok_at, last_attempt_at, last_error)
             VALUES (?1, ?2, ?2, NULL)
             ON CONFLICT (source) DO UPDATE SET
                last_ok_at = excluded.last_ok_at,
                last_attempt_at = excluded.last_attempt_at,
                last_error = NULL",
            params![source, now],
        )
        .wrap_err_with(|| format!("Failed to record the sync of {source}"))?;
    Ok(())
}

/// Records an attempt that did not fully succeed. `last_ok_at` is left as it was.
pub fn record_failure(cache: &Store, source: &str, now: i64, error: &str) -> eyre::Result<()> {
    cache
        .conn()
        .execute(
            "INSERT INTO sync_state (source, last_ok_at, last_attempt_at, last_error)
             VALUES (?1, NULL, ?2, ?3)
             ON CONFLICT (source) DO UPDATE SET
                last_attempt_at = excluded.last_attempt_at,
                last_error = excluded.last_error",
            params![source, now, error],
        )
        .wrap_err_with(|| format!("Failed to record the failed sync of {source}"))?;
    Ok(())
}
