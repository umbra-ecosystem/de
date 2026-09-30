//! Checklist items and free-form notes per ticket (`state.db`).

use eyre::{Context, bail};
use rusqlite::{OptionalExtension, params};

use super::Store;
use crate::domain::TicketKey;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChecklistItem {
    pub id: i64,
    pub ticket: TicketKey,
    /// 0-based position within the ticket's checklist.
    pub position: i64,
    pub text: String,
    pub done: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    pub id: i64,
    pub ticket: TicketKey,
    pub body: String,
    pub created_at: i64,
}

/// Appends an unchecked item at the end of the ticket's checklist.
pub fn add_checklist_item(
    store: &Store,
    ticket: &TicketKey,
    text: &str,
) -> eyre::Result<ChecklistItem> {
    let conn = store.conn();
    conn.execute(
        "INSERT INTO checklist_items (ticket_key, position, text)
         VALUES (?1, (SELECT COALESCE(MAX(position) + 1, 0) FROM checklist_items WHERE ticket_key = ?1), ?2)",
        params![ticket, text],
    )
    .wrap_err_with(|| format!("Failed to add a checklist item to {ticket}"))?;
    let id = conn.last_insert_rowid();
    Ok(ChecklistItem {
        id,
        ticket: ticket.clone(),
        position: conn.query_row(
            "SELECT position FROM checklist_items WHERE id = ?1",
            params![id],
            |r| r.get(0),
        )?,
        text: text.into(),
        done: false,
    })
}

/// Sets an item's done flag.
pub fn set_checklist_done(store: &Store, id: i64, done: bool) -> eyre::Result<()> {
    let changed = store
        .conn()
        .execute(
            "UPDATE checklist_items SET done = ?2 WHERE id = ?1",
            params![id, done],
        )
        .wrap_err("Failed to update the checklist item")?;
    if changed == 0 {
        bail!("Checklist item {id} does not exist");
    }
    Ok(())
}

/// Flips an item's done flag and returns the new state.
pub fn toggle_checklist_item(store: &Store, id: i64) -> eyre::Result<bool> {
    let done: Option<bool> = store
        .conn()
        .query_row(
            "UPDATE checklist_items SET done = 1 - done WHERE id = ?1 RETURNING done",
            params![id],
            |r| r.get(0),
        )
        .optional()
        .wrap_err("Failed to toggle the checklist item")?;
    match done {
        Some(done) => Ok(done),
        None => bail!("Checklist item {id} does not exist"),
    }
}

/// The ticket's checklist in order.
pub fn list_checklist(store: &Store, ticket: &TicketKey) -> eyre::Result<Vec<ChecklistItem>> {
    let mut stmt = store.conn().prepare(
        "SELECT id, ticket_key, position, text, done FROM checklist_items
         WHERE ticket_key = ?1 ORDER BY position",
    )?;
    let items = stmt
        .query_map(params![ticket], |r| {
            Ok(ChecklistItem {
                id: r.get(0)?,
                ticket: r.get(1)?,
                position: r.get(2)?,
                text: r.get(3)?,
                done: r.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()
        .wrap_err_with(|| format!("Failed to list the checklist of {ticket}"))?;
    Ok(items)
}

pub fn add_note(store: &Store, ticket: &TicketKey, body: &str, now: i64) -> eyre::Result<Note> {
    let conn = store.conn();
    conn.execute(
        "INSERT INTO notes (ticket_key, body, created_at) VALUES (?1, ?2, ?3)",
        params![ticket, body, now],
    )
    .wrap_err_with(|| format!("Failed to add a note to {ticket}"))?;
    Ok(Note {
        id: conn.last_insert_rowid(),
        ticket: ticket.clone(),
        body: body.into(),
        created_at: now,
    })
}

/// Notes oldest first.
pub fn list_notes(store: &Store, ticket: &TicketKey) -> eyre::Result<Vec<Note>> {
    let mut stmt = store.conn().prepare(
        "SELECT id, ticket_key, body, created_at FROM notes
         WHERE ticket_key = ?1 ORDER BY created_at, id",
    )?;
    let notes = stmt
        .query_map(params![ticket], |r| {
            Ok(Note {
                id: r.get(0)?,
                ticket: r.get(1)?,
                body: r.get(2)?,
                created_at: r.get(3)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()
        .wrap_err_with(|| format!("Failed to list notes of {ticket}"))?;
    Ok(notes)
}
