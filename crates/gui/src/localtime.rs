//! Local time, for what the window shows. The engine and its logs keep UTC; a person reads their own clock.
//!
//! The offset comes from the C library's `localtime_r` for the instant in question (so daylight saving is right
//! for past dates too). The arithmetic on top is pure and tested with fixed offsets.

/// The offset of local time from UTC at `secs` (unix seconds), in seconds. UTC when it cannot be found.
pub fn offset_at(secs: i64) -> i64 {
    // SAFETY: `localtime_r` only writes into the `tm` we hand it, which is zero-initialised and lives here.
    unsafe {
        let t: libc::time_t = secs as libc::time_t;
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&t, &mut tm).is_null() {
            return 0;
        }
        // `tm_gmtoff` is an `i64` on macOS and a narrower type elsewhere.
        #[allow(clippy::useless_conversion)]
        i64::from(tm.tm_gmtoff)
    }
}

/// The minute of the local day (0..1440) at `secs`.
pub fn minute_of_day(secs: i64) -> i64 {
    minute_of_day_at(secs, offset_at(secs))
}

pub fn minute_of_day_at(secs: i64, offset: i64) -> i64 {
    (secs + offset).rem_euclid(86_400) / 60
}

/// `2026-10-01 09:40` in local time.
pub fn format(secs: i64) -> String {
    format_at(secs, offset_at(secs))
}

/// `2026-10-01 09:40:12` in local time.
pub fn format_seconds(secs: i64) -> String {
    let off = offset_at(secs);
    format!("{}:{:02}", format_at(secs, off), (secs + off).rem_euclid(60))
}

/// `2026-10-01 09:40` for `secs` shifted by `offset` seconds.
pub fn format_at(secs: i64, offset: i64) -> String {
    let local = secs + offset;
    let days = local.div_euclid(86_400);
    let rem = local.rem_euclid(86_400);
    // Civil-from-days (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}", rem / 3600, rem % 3600 / 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fixed_offset_shifts_the_date_and_the_clock() {
        // 2023-11-14 22:13:20 UTC.
        assert_eq!(format_at(1_700_000_000, 0), "2023-11-14 22:13");
        // UTC+5 crosses midnight into the next day.
        assert_eq!(format_at(1_700_000_000, 5 * 3600), "2023-11-15 03:13");
        // UTC-8 stays on the same day.
        assert_eq!(format_at(1_700_000_000, -8 * 3600), "2023-11-14 14:13");
        assert_eq!(format_at(0, 0), "1970-01-01 00:00");
    }

    #[test]
    fn the_minute_of_day_follows_the_offset() {
        assert_eq!(minute_of_day_at(1_700_000_000, 0), 22 * 60 + 13);
        assert_eq!(minute_of_day_at(1_700_000_000, 5 * 3600), 3 * 60 + 13);
        assert_eq!(minute_of_day_at(1_700_000_000, -23 * 3600), 23 * 60 + 13);
    }

    #[test]
    fn the_real_offset_is_a_plausible_one() {
        let off = offset_at(1_700_000_000);
        assert!((-14 * 3600..=14 * 3600).contains(&off), "{off}");
        assert_eq!(format(1_700_000_000), format_at(1_700_000_000, off));
    }
}
