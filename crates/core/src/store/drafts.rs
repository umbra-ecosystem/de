//! Local drafts of external writes (`state.db`): the deploy comment and the transition.
//!
//! A `Posted` draft can never be posted again; that is what makes retries safe.

use eyre::{Context, bail, eyre};
use rusqlite::{OptionalExtension, Row, params};

use super::Store;
use crate::domain::TicketKey;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DraftKind {
    DeployComment,
    Transition,
}

impl DraftKind {
    pub const ALL: &'static [DraftKind] = &[DraftKind::DeployComment, DraftKind::Transition];

    pub fn as_str(self) -> &'static str {
        match self {
            DraftKind::DeployComment => "deploy_comment",
            DraftKind::Transition => "transition",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|k| k.as_str() == s)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DraftStatus {
    Draft,
    Posted,
    Discarded,
}

impl DraftStatus {
    pub const ALL: &'static [DraftStatus] = &[
        DraftStatus::Draft,
        DraftStatus::Posted,
        DraftStatus::Discarded,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            DraftStatus::Draft => "draft",
            DraftStatus::Posted => "posted",
            DraftStatus::Discarded => "discarded",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|k| k.as_str() == s)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredDraft {
    pub id: i64,
    pub ticket: TicketKey,
    pub kind: DraftKind,
    pub body: String,
    pub status: DraftStatus,
    pub created_at: i64,
    pub posted_at: Option<i64>,
    pub remote_id: Option<String>,
}

const COLUMNS: &str = "id, ticket_key, kind, body, status, created_at, posted_at, remote_id";

fn bad(col: usize, what: &str) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        col,
        rusqlite::types::Type::Text,
        format!("unknown {what}").into(),
    )
}

fn from_row(r: &Row<'_>) -> rusqlite::Result<StoredDraft> {
    let kind: String = r.get(2)?;
    let status: String = r.get(4)?;
    Ok(StoredDraft {
        id: r.get(0)?,
        ticket: r.get(1)?,
        kind: DraftKind::parse(&kind).ok_or_else(|| bad(2, "draft kind"))?,
        body: r.get(3)?,
        status: DraftStatus::parse(&status).ok_or_else(|| bad(4, "draft status"))?,
        created_at: r.get(5)?,
        posted_at: r.get(6)?,
        remote_id: r.get(7)?,
    })
}

/// Creates a `Draft`. Earlier unposted drafts of the same kind for the ticket are discarded.
pub fn create(
    store: &Store,
    ticket: &TicketKey,
    kind: DraftKind,
    body: &str,
    now: i64,
) -> eyre::Result<StoredDraft> {
    let tx = store.conn().unchecked_transaction()?;
    tx.execute(
        "UPDATE drafts SET status = 'discarded'
         WHERE ticket_key = ?1 AND kind = ?2 AND status = 'draft'",
        params![ticket, kind.as_str()],
    )?;
    tx.execute(
        "INSERT INTO drafts (ticket_key, kind, body, status, created_at)
         VALUES (?1, ?2, ?3, 'draft', ?4)",
        params![ticket, kind.as_str(), body, now],
    )
    .wrap_err("Failed to save the draft")?;
    let id = tx.last_insert_rowid();
    tx.commit()?;
    get(store, id)?.ok_or_else(|| eyre!("draft vanished"))
}

pub fn get(store: &Store, id: i64) -> eyre::Result<Option<StoredDraft>> {
    store
        .conn()
        .query_row(
            &format!("SELECT {COLUMNS} FROM drafts WHERE id = ?1"),
            params![id],
            from_row,
        )
        .optional()
        .wrap_err("Failed to read the draft")
}

/// Drafts of a ticket, oldest first.
pub fn list_for_ticket(store: &Store, ticket: &TicketKey) -> eyre::Result<Vec<StoredDraft>> {
    let mut stmt = store.conn().prepare(&format!(
        "SELECT {COLUMNS} FROM drafts WHERE ticket_key = ?1 ORDER BY id"
    ))?;
    Ok(stmt
        .query_map(params![ticket], from_row)?
        .collect::<Result<Vec<_>, _>>()?)
}

/// The newest draft of `kind` with `status`.
pub fn latest(
    store: &Store,
    ticket: &TicketKey,
    kind: DraftKind,
    status: DraftStatus,
) -> eyre::Result<Option<StoredDraft>> {
    Ok(list_for_ticket(store, ticket)?
        .into_iter()
        .rfind(|d| d.kind == kind && d.status == status))
}

/// Replaces the body of a `Draft`; a posted or discarded draft cannot be edited.
pub fn update_body(store: &Store, id: i64, body: &str) -> eyre::Result<()> {
    if body.trim().is_empty() {
        bail!("A draft cannot be empty");
    }
    let changed = store.conn().execute(
        "UPDATE drafts SET body = ?2 WHERE id = ?1 AND status = 'draft'",
        params![id, body],
    )?;
    if changed == 0 {
        bail!("Draft {id} does not exist or is no longer editable (already posted or discarded)");
    }
    Ok(())
}

pub fn discard(store: &Store, id: i64) -> eyre::Result<()> {
    let changed = store.conn().execute(
        "UPDATE drafts SET status = 'discarded' WHERE id = ?1 AND status = 'draft'",
        params![id],
    )?;
    if changed == 0 {
        bail!("Draft {id} does not exist or is not an open draft");
    }
    Ok(())
}

/// Marks a `Draft` as posted. Fails (changing nothing) for any other status.
pub fn mark_posted(store: &Store, id: i64, now: i64, remote_id: Option<&str>) -> eyre::Result<()> {
    let changed = store.conn().execute(
        "UPDATE drafts SET status = 'posted', posted_at = ?2, remote_id = ?3
         WHERE id = ?1 AND status = 'draft'",
        params![id, now, remote_id],
    )?;
    if changed == 0 {
        bail!("Draft {id} is not an open draft (already posted or discarded)");
    }
    Ok(())
}
