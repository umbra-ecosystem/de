//! What the author said about suggestions (`state.db`): dismissed, snoozed or done.
//!
//! Append-only: every answer is a new row and the latest row per suggestion id is the
//! current one, so the history stays available for tuning the rules. Only the engine's
//! output filter (`next::apply_responses`) interprets these rows.

use std::collections::BTreeMap;

use eyre::{Context, bail};
use rusqlite::params;

use super::Store;
use crate::domain::TicketKey;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseKind {
    /// Not wanted. Stays hidden until the suggestion's facts change materially.
    Dismissed,
    /// Hidden until `snooze_until`.
    Snoozed,
    /// Carried out. Hidden while the facts that triggered it are unchanged (so a sync that
    /// has not caught up yet does not bring it back).
    Done,
}

impl ResponseKind {
    pub const ALL: &'static [ResponseKind] = &[
        ResponseKind::Dismissed,
        ResponseKind::Snoozed,
        ResponseKind::Done,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            ResponseKind::Dismissed => "dismissed",
            ResponseKind::Snoozed => "snoozed",
            ResponseKind::Done => "done",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|k| k.as_str() == s)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuggestionResponse {
    pub suggestion_id: String,
    pub ticket: Option<TicketKey>,
    pub rule: String,
    pub response: ResponseKind,
    pub reason: Option<String>,
    /// Set exactly for `Snoozed`.
    pub snooze_until: Option<i64>,
    /// Hash of the suggestion's facts when it was answered (see `next::facts_hash`).
    pub facts_hash: String,
    pub at: i64,
}

/// Appends a response. A snooze needs `snooze_until`; the others must not have one.
pub fn record(store: &Store, response: &SuggestionResponse) -> eyre::Result<()> {
    if (response.response == ResponseKind::Snoozed) != response.snooze_until.is_some() {
        bail!("only a snooze has (and needs) a snooze_until time");
    }
    store
        .conn()
        .execute(
            "INSERT INTO suggestion_responses
                (suggestion_id, ticket_key, rule, response, reason, snooze_until, facts_hash, at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                response.suggestion_id,
                response.ticket,
                response.rule,
                response.response.as_str(),
                response.reason,
                response.snooze_until,
                response.facts_hash,
                response.at
            ],
        )
        .wrap_err("Failed to record the response to the suggestion")?;
    Ok(())
}

fn from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<SuggestionResponse> {
    let response: String = r.get(3)?;
    Ok(SuggestionResponse {
        suggestion_id: r.get(0)?,
        ticket: r.get(1)?,
        rule: r.get(2)?,
        response: ResponseKind::parse(&response).ok_or_else(|| {
            rusqlite::Error::FromSqlConversionFailure(
                3,
                rusqlite::types::Type::Text,
                format!("unknown response {response:?}").into(),
            )
        })?,
        reason: r.get(4)?,
        snooze_until: r.get(5)?,
        facts_hash: r.get(6)?,
        at: r.get(7)?,
    })
}

const COLUMNS: &str =
    "suggestion_id, ticket_key, rule, response, reason, snooze_until, facts_hash, at";

/// The current (latest) response of every answered suggestion, in id order.
pub fn latest_per_suggestion(store: &Store) -> eyre::Result<Vec<SuggestionResponse>> {
    let mut stmt = store.conn().prepare(&format!(
        "SELECT {COLUMNS} FROM suggestion_responses r
         WHERE r.id = (SELECT MAX(id) FROM suggestion_responses WHERE suggestion_id = r.suggestion_id)
         ORDER BY suggestion_id"
    ))?;
    stmt.query_map([], from_row)?
        .collect::<Result<Vec<_>, _>>()
        .wrap_err("Failed to read the responses to suggestions")
}

/// Every response ever given to `suggestion_id`, oldest first.
pub fn history(store: &Store, suggestion_id: &str) -> eyre::Result<Vec<SuggestionResponse>> {
    let mut stmt = store.conn().prepare(&format!(
        "SELECT {COLUMNS} FROM suggestion_responses WHERE suggestion_id = ?1 ORDER BY id"
    ))?;
    Ok(stmt
        .query_map(params![suggestion_id], from_row)?
        .collect::<Result<Vec<_>, _>>()?)
}

/// The current responses keyed by suggestion id.
pub fn latest_map(store: &Store) -> eyre::Result<BTreeMap<String, SuggestionResponse>> {
    Ok(latest_per_suggestion(store)?
        .into_iter()
        .map(|r| (r.suggestion_id.clone(), r))
        .collect())
}
