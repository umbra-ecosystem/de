//! The audit log as the window shows it, kept in `state.db` (`audit_log`, append-only) so it survives a restart.
//!
//! The app records what it does itself (a sync, a change to the Jira settings) here; the write gateway records
//! its remote writes in the same table, so they appear too.

use de_core::domain::{AuditOutcome as CoreOutcome, TicketKey};
use de_core::store::Store;
use de_core::store::audit::{self, NewAuditEntry};
use de_widgets::sim::model::{AuditEntry, AuditOutcome};

use crate::localtime;

/// Entries read back (newest first).
const LIMIT: u32 = 500;

/// Appends one entry.
pub fn record(
    state: &Store,
    now: i64,
    action: &str,
    ticket: Option<&TicketKey>,
    ok: bool,
    text: &str,
) -> eyre::Result<()> {
    audit::append(
        state,
        &NewAuditEntry {
            at: now,
            action: action.to_string(),
            ticket: ticket.cloned(),
            repo: None,
            details: serde_json::json!({ "text": text }),
            outcome: if ok {
                CoreOutcome::Success
            } else {
                CoreOutcome::Failure
            },
        },
    )?;
    Ok(())
}

/// The stored entries as the window's audit rows, newest first. `now` decides how a time is written: today's
/// as a clock time, an earlier day's with its date.
pub fn entries(state: &Store, now: i64) -> eyre::Result<Vec<AuditEntry>> {
    Ok(audit::list(state, None, LIMIT)?
        .into_iter()
        .map(|e| AuditEntry {
            id: e.id.max(0) as u64,
            at: when(e.at, now),
            action: e.action,
            ticket: e.ticket.map(|k| k.as_str().into()),
            repo: e.repo.map(Into::into),
            outcome: match e.outcome {
                CoreOutcome::Success => AuditOutcome::Success,
                CoreOutcome::Failure => AuditOutcome::Failure,
                CoreOutcome::Skipped => AuditOutcome::Skipped,
            },
            details: details_text(&e.details),
        })
        .collect())
}

/// `03:07` for today, `09-30 21:24` for another day (local time).
pub fn when(at: i64, now: i64) -> String {
    when_at(at, now, localtime::offset_at(at), localtime::offset_at(now))
}

fn when_at(at: i64, now: i64, at_offset: i64, now_offset: i64) -> String {
    let local = |secs: i64, off: i64| localtime::format_at(secs, off); // `YYYY-MM-DD HH:MM`
    let (then, today) = (local(at, at_offset), local(now, now_offset));
    if then[..10] == today[..10] {
        then[11..].to_string()
    } else {
        format!("{} {}", &then[5..10], &then[11..])
    }
}

/// The entry's own words: what the app recorded under `text`, or else the details as they are.
fn details_text(details: &serde_json::Value) -> String {
    match details.get("text").and_then(|t| t.as_str()) {
        Some(text) => text.to_string(),
        None if details.is_null() => String::new(),
        None => details.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use de_core::store::Kind;

    #[test]
    fn what_is_recorded_is_read_back_newest_first_with_its_outcome_and_words() {
        let state = Store::open_in_memory(Kind::State).unwrap();
        let key: TicketKey = "PROJ-1".parse().unwrap();
        record(&state, 1_700_000_000, "sync", None, true, "63 tickets").unwrap();
        record(&state, 1_700_000_100, "sync", Some(&key), false, "offline, cache kept").unwrap();

        let rows = entries(&state, 1_700_000_200).unwrap();

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].details, "offline, cache kept");
        assert_eq!(rows[0].outcome, AuditOutcome::Failure);
        assert_eq!(rows[0].ticket.as_deref(), Some("PROJ-1"));
        assert_eq!(rows[1].details, "63 tickets");
        assert_eq!(rows[1].outcome, AuditOutcome::Success);
        assert!(rows[0].id > rows[1].id);
    }

    #[test]
    fn an_entry_is_still_there_after_the_database_is_reopened() {
        let dir = tempfile::tempdir().unwrap();
        {
            let state = Store::open_in(dir.path(), Kind::State).unwrap();
            record(&state, 1_700_000_000, "config.jira", None, true, "Saved 1 setting").unwrap();
        }
        let state = Store::open_in(dir.path(), Kind::State).unwrap();
        let rows = entries(&state, 1_700_000_100).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].action, "config.jira");
    }

    #[test]
    fn today_reads_as_a_clock_time_and_another_day_with_its_date() {
        // The entry is at 2023-11-14 22:13 UTC.
        let at = 1_700_000_000;
        assert_eq!(when_at(at, at + 3600, 0, 0), "22:13", "same day: the time alone");
        assert_eq!(when_at(at, at + 86_400, 0, 0), "11-14 22:13", "another day: with its date");
        // Three hours later it is already the next day in UTC, but still the same day at UTC+5.
        assert_eq!(when_at(at, at + 3 * 3600, 0, 0), "11-14 22:13");
        assert_eq!(when_at(at, at + 3 * 3600, 5 * 3600, 5 * 3600), "03:13");
    }

    #[test]
    fn details_without_our_text_show_as_they_are() {
        assert_eq!(details_text(&serde_json::json!({"text": "hi"})), "hi");
        assert_eq!(details_text(&serde_json::json!({"sha": "abc"})), "{\"sha\":\"abc\"}");
        assert_eq!(details_text(&serde_json::Value::Null), "");
    }
}
