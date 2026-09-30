//! The next-action engine (M6, with the M9 after-alpha rules): given everything known about
//! the author's tickets, what should they do now?
//!
//! - [`suggest`] is a **pure function** `(&Snapshot, now) -> Vec<Suggestion>`: no I/O, no
//!   clock, no writer. Each rule lives in [`rules`] with the workflow step it comes from.
//! - [`Snapshot::load`] is the impure half: it gathers SQLite and git state into plain data.
//! - [`apply_responses`] removes what was dismissed, snoozed or finished, unless the facts
//!   changed materially since (the responses live in `state.db`).
//! - [`exec`] carries suggestions out through the existing flows. External actions are
//!   always `Gateway::draft_because(action, facts)`, a human confirmation, `confirm`,
//!   `execute`: the engine never confirms or executes a write itself.
//!
//! Priority scheme: see [`Priority`] and [`prio`].

mod engine;
pub mod exec;
mod load;
mod model;
mod responses;
pub mod rules;
mod snapshot;

#[cfg(test)]
mod rule_tests;
#[cfg(test)]
mod scenario_tests;

pub use engine::{next_actions, sort, suggest};
pub use load::{LoadContext, ticket_heads};
pub use model::{
    ExecutionLevel, Priority, RuleId, SuggestedAction, Suggestion, facts_hash, prio,
};
pub use responses::{ResponseState, annotate, apply_responses, classify, make_response};
pub use snapshot::*;

/// Parses a duration such as `90s`, `30m`, `2h`, `1d` or `2w` into seconds.
pub fn parse_duration(text: &str) -> Option<i64> {
    let text = text.trim();
    let split = text.find(|c: char| !c.is_ascii_digit())?;
    let (number, unit) = text.split_at(split);
    let n: i64 = number.parse().ok()?;
    let unit = match unit {
        "s" => 1,
        "m" => 60,
        "h" => 3600,
        "d" => 86_400,
        "w" => 7 * 86_400,
        _ => return None,
    };
    (n > 0).then(|| n.checked_mul(unit)).flatten()
}
