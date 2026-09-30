//! Mirror of Jira tickets (`cache.db`). Disposable: a sync rebuilds it.

use eyre::Context;
use rusqlite::{OptionalExtension, Row, params};

use super::{Store, tickets};
use crate::domain::TicketKey;
use crate::providers::RemoteTicket;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JiraTicket {
    pub key: TicketKey,
    pub title: String,
    pub jira_status: String,
    pub priority: Option<String>,
    pub assignee: Option<String>,
    pub url: Option<String>,
    /// The payload as fetched, so new fields can be read without another migration.
    pub raw_json: String,
    pub fetched_at: i64,
}

const COLUMNS: &str = "key, title, jira_status, priority, assignee, url, raw_json, fetched_at";

fn from_row(r: &Row<'_>) -> rusqlite::Result<JiraTicket> {
    Ok(JiraTicket {
        key: r.get(0)?,
        title: r.get(1)?,
        jira_status: r.get(2)?,
        priority: r.get(3)?,
        assignee: r.get(4)?,
        url: r.get(5)?,
        raw_json: r.get(6)?,
        fetched_at: r.get(7)?,
    })
}

/// Inserts or fully replaces the cached copy of a ticket.
pub fn upsert(cache: &Store, t: &JiraTicket) -> eyre::Result<()> {
    cache
        .conn()
        .execute(
            "INSERT INTO jira_tickets (key, title, jira_status, priority, assignee, url, raw_json, fetched_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT (key) DO UPDATE SET
                title = excluded.title,
                jira_status = excluded.jira_status,
                priority = excluded.priority,
                assignee = excluded.assignee,
                url = excluded.url,
                raw_json = excluded.raw_json,
                fetched_at = excluded.fetched_at",
            params![
                t.key,
                t.title,
                t.jira_status,
                t.priority,
                t.assignee,
                t.url,
                t.raw_json,
                t.fetched_at
            ],
        )
        .wrap_err_with(|| format!("Failed to cache {}", t.key))?;
    Ok(())
}

/// Caches a ticket as the provider reported it (the payload is stored as JSON).
pub fn upsert_remote(cache: &Store, t: &RemoteTicket, now: i64) -> eyre::Result<()> {
    upsert(
        cache,
        &JiraTicket {
            key: t.key.clone(),
            title: t.title.clone(),
            jira_status: t.status.clone(),
            priority: t.priority.clone(),
            assignee: t.assignee.clone(),
            url: t.url.clone(),
            raw_json: serde_json::to_string(t).wrap_err("Failed to serialise the ticket")?,
            fetched_at: now,
        },
    )
}

/// Removes every cached ticket whose key is not in `keep`; returns how many went.
pub fn delete_except(cache: &Store, keep: &[TicketKey]) -> eyre::Result<usize> {
    let mut removed = 0;
    for t in list(cache)?.into_iter().filter(|t| !keep.contains(&t.key)) {
        removed += cache
            .conn()
            .execute("DELETE FROM jira_tickets WHERE key = ?1", params![t.key])
            .wrap_err_with(|| format!("Failed to remove cached {}", t.key))?;
    }
    Ok(removed)
}

pub fn get(cache: &Store, key: &TicketKey) -> eyre::Result<Option<JiraTicket>> {
    cache
        .conn()
        .query_row(
            &format!("SELECT {COLUMNS} FROM jira_tickets WHERE key = ?1"),
            params![key],
            from_row,
        )
        .optional()
        .wrap_err_with(|| format!("Failed to read cached {key}"))
}

/// All cached tickets ordered by key.
pub fn list(cache: &Store) -> eyre::Result<Vec<JiraTicket>> {
    let mut stmt = cache
        .conn()
        .prepare(&format!("SELECT {COLUMNS} FROM jira_tickets ORDER BY key"))?;
    let rows = stmt
        .query_map([], from_row)?
        .collect::<Result<Vec<_>, _>>()
        .wrap_err("Failed to list cached tickets")?;
    Ok(rows)
}

/// A ticket as the UI sees it: the Jira mirror joined with local tracking. Either side may
/// be missing (tracked by hand before any sync, or in the pool but not claimed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TicketView {
    pub key: TicketKey,
    pub jira: Option<JiraTicket>,
    pub tracking: Option<tickets::TicketTracking>,
}

/// The view of one ticket, or `None` if neither database knows it.
pub fn view(state: &Store, cache: &Store, key: &TicketKey) -> eyre::Result<Option<TicketView>> {
    let jira = get(cache, key)?;
    let tracking = tickets::get(state, key)?;
    if jira.is_none() && tracking.is_none() {
        return Ok(None);
    }
    Ok(Some(TicketView {
        key: key.clone(),
        jira,
        tracking,
    }))
}

/// Every known ticket: tracked ones first in manual order, then untracked cached ones by key.
pub fn list_views(state: &Store, cache: &Store) -> eyre::Result<Vec<TicketView>> {
    let mut cached = list(cache)?;
    let mut views = Vec::new();

    for tracking in tickets::list(state)? {
        let jira = cached
            .iter()
            .position(|j| j.key == tracking.key)
            .map(|i| cached.remove(i));
        views.push(TicketView {
            key: tracking.key.clone(),
            jira,
            tracking: Some(tracking),
        });
    }
    views.extend(cached.into_iter().map(|j| TicketView {
        key: j.key.clone(),
        jira: Some(j),
        tracking: None,
    }));
    Ok(views)
}
