//! Local time tracking per ticket (`state.db`). Nothing here reads a clock.

use eyre::{Context, bail};
use rusqlite::{OptionalExtension, Row, params};

use super::Store;
use crate::domain::TicketKey;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimeEntry {
    pub id: i64,
    pub ticket: TicketKey,
    pub started_at: i64,
    /// `None` while the timer is running.
    pub ended_at: Option<i64>,
}

fn from_row(r: &Row<'_>) -> rusqlite::Result<TimeEntry> {
    Ok(TimeEntry {
        id: r.get(0)?,
        ticket: r.get(1)?,
        started_at: r.get(2)?,
        ended_at: r.get(3)?,
    })
}

/// The running entry, if any. At most one exists across all tickets.
pub fn open_entry(store: &Store) -> eyre::Result<Option<TimeEntry>> {
    store
        .conn()
        .query_row(
            "SELECT id, ticket_key, started_at, ended_at FROM time_entries WHERE ended_at IS NULL",
            [],
            from_row,
        )
        .optional()
        .wrap_err("Failed to read the running timer")
}

/// Starts a timer on `ticket`. Errors if any timer is already running (stop it first).
pub fn start(store: &Store, ticket: &TicketKey, now: i64) -> eyre::Result<TimeEntry> {
    if let Some(open) = open_entry(store)? {
        bail!("A timer is already running on {}", open.ticket);
    }
    let conn = store.conn();
    conn.execute(
        "INSERT INTO time_entries (ticket_key, started_at) VALUES (?1, ?2)",
        params![ticket, now],
    )
    .wrap_err_with(|| format!("Failed to start the timer on {ticket}"))?;
    Ok(TimeEntry {
        id: conn.last_insert_rowid(),
        ticket: ticket.clone(),
        started_at: now,
        ended_at: None,
    })
}

/// Stops the running timer and returns the closed entry, or `None` if nothing was running.
/// A `now` earlier than the start (clock moved back) closes the entry with zero length.
pub fn stop(store: &Store, now: i64) -> eyre::Result<Option<TimeEntry>> {
    store
        .conn()
        .query_row(
            "UPDATE time_entries SET ended_at = MAX(?1, started_at) WHERE ended_at IS NULL
             RETURNING id, ticket_key, started_at, ended_at",
            params![now],
            from_row,
        )
        .optional()
        .wrap_err("Failed to stop the timer")
}

/// Entries of `ticket`, oldest first.
pub fn list(store: &Store, ticket: &TicketKey) -> eyre::Result<Vec<TimeEntry>> {
    let mut stmt = store.conn().prepare(
        "SELECT id, ticket_key, started_at, ended_at FROM time_entries
         WHERE ticket_key = ?1 ORDER BY started_at, id",
    )?;
    let entries = stmt
        .query_map(params![ticket], from_row)?
        .collect::<Result<Vec<_>, _>>()
        .wrap_err_with(|| format!("Failed to list time entries of {ticket}"))?;
    Ok(entries)
}

/// Total tracked seconds for `ticket`; a running entry counts up to `now`.
pub fn total_seconds(store: &Store, ticket: &TicketKey, now: i64) -> eyre::Result<i64> {
    store
        .conn()
        .query_row(
            "SELECT COALESCE(SUM(COALESCE(ended_at, MAX(?2, started_at)) - started_at), 0)
             FROM time_entries WHERE ticket_key = ?1",
            params![ticket, now],
            |r| r.get(0),
        )
        .wrap_err_with(|| format!("Failed to total the time of {ticket}"))
}
