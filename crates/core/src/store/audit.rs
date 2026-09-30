//! Append-only audit log (`state.db`).
//!
//! Every external write goes through the write gateway (later milestone), which records
//! it here with the facts that motivated it. The database rejects UPDATE and DELETE.

use eyre::Context;
use rusqlite::params;

use super::Store;
use crate::domain::{AuditOutcome, TicketKey};

/// An entry to append.
#[derive(Debug, Clone, PartialEq)]
pub struct NewAuditEntry {
    pub at: i64,
    /// Free-form action kind, e.g. `git.push_uat` or `jira.comment`.
    pub action: String,
    pub ticket: Option<TicketKey>,
    pub repo: Option<String>,
    /// Facts behind the action (JSON).
    pub details: serde_json::Value,
    pub outcome: AuditOutcome,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AuditEntry {
    pub id: i64,
    pub at: i64,
    pub action: String,
    pub ticket: Option<TicketKey>,
    pub repo: Option<String>,
    pub details: serde_json::Value,
    pub outcome: AuditOutcome,
}

/// Appends an entry and returns its id.
pub fn append(store: &Store, entry: &NewAuditEntry) -> eyre::Result<i64> {
    let conn = store.conn();
    conn.execute(
        "INSERT INTO audit_log (at, action, ticket_key, repo, details, outcome)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            entry.at,
            entry.action,
            entry.ticket,
            entry.repo,
            entry.details.to_string(),
            entry.outcome
        ],
    )
    .wrap_err_with(|| format!("Failed to append audit entry {}", entry.action))?;
    Ok(conn.last_insert_rowid())
}

/// Newest entries first, optionally only those about `ticket`, at most `limit`.
pub fn list(
    store: &Store,
    ticket: Option<&TicketKey>,
    limit: u32,
) -> eyre::Result<Vec<AuditEntry>> {
    let mut stmt = store.conn().prepare(
        "SELECT id, at, action, ticket_key, repo, details, outcome FROM audit_log
         WHERE (?1 IS NULL OR ticket_key = ?1) ORDER BY id DESC LIMIT ?2",
    )?;
    let rows = stmt
        .query_map(params![ticket, limit], |r| {
            let details: String = r.get(5)?;
            Ok(AuditEntry {
                id: r.get(0)?,
                at: r.get(1)?,
                action: r.get(2)?,
                ticket: r.get(3)?,
                repo: r.get(4)?,
                details: serde_json::from_str(&details).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        5,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })?,
                outcome: r.get(6)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()
        .wrap_err("Failed to list audit entries")?;
    Ok(rows)
}
