//! When the app syncs by itself.
//!
//! Every `interval_minutes` from the last attempt, and never while one is running. After failures it waits
//! longer (double each time, at most an hour) so an unreachable or logged-out Jira is not asked every cycle.
//! A manual sync counts as an attempt. The clock is passed in; nothing here reads the time.

/// The longest wait between attempts after failures.
const MAX_BACKOFF_SECS: i64 = 60 * 60;

/// When the next automatic sync is due (unix seconds), or `None` when they are off (`interval_minutes` is 0).
pub fn next_due(last_attempt: i64, interval_minutes: u32, failures: u32) -> Option<i64> {
    if interval_minutes == 0 {
        return None;
    }
    let base = i64::from(interval_minutes) * 60;
    // 2^failures, without overflowing for a long run of failures.
    let factor = 1_i64 << failures.min(10);
    Some(last_attempt + (base * factor).min(MAX_BACKOFF_SECS.max(base)))
}

/// Whether an automatic sync should start now.
pub fn is_due(now: i64, last_attempt: i64, interval_minutes: u32, running: bool, failures: u32) -> bool {
    !running && next_due(last_attempt, interval_minutes, failures).is_some_and(|due| now >= due)
}

/// After how many minutes without a sync the window calls it overdue (the "Sync now" suggestion): two
/// intervals, but never under six minutes. Automatic syncs off keeps the old six.
pub fn overdue_after_minutes(interval_minutes: u32) -> i64 {
    if interval_minutes == 0 {
        6
    } else {
        (i64::from(interval_minutes) * 2).max(6)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_waits_one_interval_from_the_last_attempt() {
        assert_eq!(next_due(1_000, 10, 0), Some(1_000 + 600));
        assert!(!is_due(1_599, 1_000, 10, false, 0));
        assert!(is_due(1_600, 1_000, 10, false, 0));
    }

    #[test]
    fn zero_turns_it_off() {
        assert_eq!(next_due(0, 0, 0), None);
        assert!(!is_due(i64::MAX / 2, 0, 0, false, 0));
    }

    #[test]
    fn it_never_starts_one_while_one_is_running() {
        assert!(!is_due(10_000, 0, 10, true, 0));
    }

    #[test]
    fn failures_double_the_wait_up_to_an_hour_and_a_success_resets_it() {
        let waits: Vec<i64> = (0..6).map(|f| next_due(0, 10, f).unwrap()).collect();
        assert_eq!(waits, [600, 1200, 2400, 3600, 3600, 3600]);
        assert_eq!(next_due(0, 10, 0), Some(600), "no failures: the plain interval");
        // A long run of failures does not overflow.
        assert_eq!(next_due(0, 10, u32::MAX), Some(3600));
    }

    #[test]
    fn an_interval_longer_than_an_hour_is_not_cut_short_by_the_backoff_cap() {
        assert_eq!(next_due(0, 120, 0), Some(7200));
        assert_eq!(next_due(0, 120, 3), Some(7200));
    }

    #[test]
    fn the_overdue_threshold_follows_the_interval() {
        assert_eq!(overdue_after_minutes(0), 6);
        assert_eq!(overdue_after_minutes(2), 6);
        assert_eq!(overdue_after_minutes(10), 20);
    }
}
