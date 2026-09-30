//! When a ticket's review was finished (`state.db`), and which branch tips it covered.
//!
//! The next-action engine compares these tips (or, without them, the PR's `updated_at`) with
//! the current ones to notice new commits since the review.

use std::collections::BTreeMap;

use eyre::{Context, eyre};
use rusqlite::{OptionalExtension, params};

use super::Store;
use crate::domain::TicketKey;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewMark {
    pub ticket: TicketKey,
    pub reviewed_at: i64,
    /// Workspace project name to the commit of the ticket branch that was reviewed.
    pub heads: BTreeMap<String, String>,
}

/// Records that `ticket` was reviewed at `now`, covering `heads` (replaces an older mark).
/// The ticket must be tracked.
pub fn mark(
    store: &Store,
    ticket: &TicketKey,
    heads: &BTreeMap<String, String>,
    now: i64,
) -> eyre::Result<()> {
    let json = serde_json::to_string(heads).wrap_err("Failed to encode the reviewed heads")?;
    store
        .conn()
        .execute(
            "INSERT INTO ticket_reviews (ticket_key, reviewed_at, heads) VALUES (?1, ?2, ?3)
             ON CONFLICT (ticket_key) DO UPDATE SET reviewed_at = excluded.reviewed_at,
                                                    heads = excluded.heads",
            params![ticket, now, json],
        )
        .wrap_err_with(|| format!("Failed to mark {ticket} as reviewed (is it tracked?)"))?;
    Ok(())
}

/// Forgets the mark: the review starts over.
pub fn clear(store: &Store, ticket: &TicketKey) -> eyre::Result<bool> {
    Ok(store.conn().execute(
        "DELETE FROM ticket_reviews WHERE ticket_key = ?1",
        params![ticket],
    )? > 0)
}

pub fn get(store: &Store, ticket: &TicketKey) -> eyre::Result<Option<ReviewMark>> {
    let row: Option<(i64, String)> = store
        .conn()
        .query_row(
            "SELECT reviewed_at, heads FROM ticket_reviews WHERE ticket_key = ?1",
            params![ticket],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .wrap_err("Failed to read the review mark")?;
    row.map(|(reviewed_at, heads)| {
        Ok(ReviewMark {
            ticket: ticket.clone(),
            reviewed_at,
            heads: serde_json::from_str(&heads)
                .map_err(|e| eyre!("The stored heads of {ticket} are unreadable: {e}"))?,
        })
    })
    .transpose()
}
