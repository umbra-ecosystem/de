//! The output filter: what the author dismissed, snoozed or finished stays out of the list.
//!
//! Pure over the suggestions and the stored responses (`suggestion_id` to its latest
//! response), plus `now`.
//!
//! - **Dismissed** stays hidden while the suggestion's facts hash equals the one stored with
//!   the dismissal; once the facts change materially (a new commit, a new comment, another
//!   state) it comes back.
//! - **Snoozed** is hidden until `snooze_until` (exclusive: at that second it is back),
//!   whatever the facts do meanwhile.
//! - **Done** behaves like dismissed: it is hidden while the facts that triggered it are
//!   unchanged (so a cache that has not caught up with what you just did does not bring it
//!   back), and resurfaces when they change.

use std::collections::BTreeMap;

use serde::Serialize;

use super::model::Suggestion;
use crate::store::suggestion_responses::{ResponseKind, SuggestionResponse};

/// Where a suggestion stands with respect to what the author said about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseState {
    /// Never answered.
    Open,
    /// Answered before, but the answer no longer applies (facts changed or snooze over).
    Resurfaced,
    Dismissed,
    Snoozed,
    Done,
}

impl ResponseState {
    pub fn as_str(self) -> &'static str {
        match self {
            ResponseState::Open => "open",
            ResponseState::Resurfaced => "resurfaced",
            ResponseState::Dismissed => "dismissed",
            ResponseState::Snoozed => "snoozed",
            ResponseState::Done => "done",
        }
    }

    /// Whether the suggestion belongs in the list.
    pub fn is_visible(self) -> bool {
        matches!(self, ResponseState::Open | ResponseState::Resurfaced)
    }
}

/// How `response` (the latest for this suggestion, if any) applies to `suggestion` now.
pub fn classify(
    suggestion: &Suggestion,
    response: Option<&SuggestionResponse>,
    now: i64,
) -> ResponseState {
    let Some(r) = response else {
        return ResponseState::Open;
    };
    match r.response {
        ResponseKind::Snoozed => match r.snooze_until {
            Some(until) if now < until => ResponseState::Snoozed,
            _ => ResponseState::Resurfaced,
        },
        ResponseKind::Dismissed | ResponseKind::Done => {
            if r.facts_hash == suggestion.facts_hash() {
                if r.response == ResponseKind::Done {
                    ResponseState::Done
                } else {
                    ResponseState::Dismissed
                }
            } else {
                ResponseState::Resurfaced
            }
        }
    }
}

/// Every suggestion with its [`ResponseState`], in the given order.
pub fn annotate(
    suggestions: Vec<Suggestion>,
    responses: &BTreeMap<String, SuggestionResponse>,
    now: i64,
) -> Vec<(Suggestion, ResponseState)> {
    suggestions
        .into_iter()
        .map(|s| {
            let state = classify(&s, responses.get(&s.id), now);
            (s, state)
        })
        .collect()
}

/// Only the suggestions that are visible, in the given order.
pub fn apply_responses(
    suggestions: Vec<Suggestion>,
    responses: &BTreeMap<String, SuggestionResponse>,
    now: i64,
) -> Vec<Suggestion> {
    annotate(suggestions, responses, now)
        .into_iter()
        .filter(|(_, state)| state.is_visible())
        .map(|(s, _)| s)
        .collect()
}

/// The response record to store for `suggestion`, tied to its current facts.
pub fn make_response(
    suggestion: &Suggestion,
    kind: ResponseKind,
    reason: Option<String>,
    snooze_until: Option<i64>,
    now: i64,
) -> SuggestionResponse {
    SuggestionResponse {
        suggestion_id: suggestion.id.clone(),
        ticket: suggestion.ticket.clone(),
        rule: suggestion.rule.as_str().into(),
        response: kind,
        reason,
        snooze_until,
        facts_hash: suggestion.facts_hash(),
        at: now,
    }
}
