//! Timestamp parsing for the formats Jira and `acli` print, without a date-time dependency.

/// Parses a Jira timestamp into unix seconds (UTC).
///
/// Accepted: `2024-05-01T10:15:30.123+0000` (Jira's REST format, offset without a colon),
/// `2024-05-01T10:15:30.123+00:00` and `...Z` (RFC 3339), a space instead of `T`, no
/// seconds, no zone (taken as UTC), a bare date, and all-digit epoch values (seconds, or
/// milliseconds when 12 or more digits). Fractions are truncated. Returns `None` for
/// anything else; never panics.
pub fn parse_timestamp(text: &str) -> Option<i64> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    if text.bytes().all(|b| b.is_ascii_digit()) {
        let n: i64 = text.parse().ok()?;
        return Some(if text.len() >= 12 { n / 1000 } else { n });
    }
    if !text.is_ascii() {
        return None;
    }
    let b = text.as_bytes();
    let num = |from: usize, len: usize| -> Option<i64> {
        let s = text.get(from..from + len)?;
        if s.bytes().all(|c| c.is_ascii_digit()) {
            s.parse().ok()
        } else {
            None
        }
    };
    if b.len() < 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let (year, month, day) = (num(0, 4)?, num(5, 2)?, num(8, 2)?);
    if !(1..=12).contains(&month) || day < 1 || day > days_in_month(year, month) {
        return None;
    }
    let mut secs_of_day = 0;
    let mut offset_secs = 0;
    let mut i = 10;
    if i < b.len() {
        if b[i] != b'T' && b[i] != b't' && b[i] != b' ' {
            return None;
        }
        i += 1;
        let hour = num(i, 2)?;
        if b.get(i + 2) != Some(&b':') {
            return None;
        }
        let minute = num(i + 3, 2)?;
        i += 5;
        let mut second = 0;
        if b.get(i) == Some(&b':') {
            second = num(i + 1, 2)?;
            i += 3;
            if b.get(i) == Some(&b'.') || b.get(i) == Some(&b',') {
                i += 1;
                let start = i;
                while i < b.len() && b[i].is_ascii_digit() {
                    i += 1;
                }
                if i == start {
                    return None;
                }
            }
        }
        if hour > 23 || minute > 59 || second > 60 {
            return None;
        }
        secs_of_day = hour * 3600 + minute * 60 + second;
        match b.get(i) {
            None => {}
            Some(b'Z') | Some(b'z') => {
                if i + 1 != b.len() {
                    return None;
                }
            }
            Some(&sign @ (b'+' | b'-')) => {
                let oh = num(i + 1, 2)?;
                let rest = &text[i + 3..];
                let om = match rest.len() {
                    0 => 0,
                    2 => num(i + 3, 2)?,
                    3 if rest.starts_with(':') => num(i + 4, 2)?,
                    _ => return None,
                };
                if oh > 23 || om > 59 {
                    return None;
                }
                let total = oh * 3600 + om * 60;
                offset_secs = if sign == b'+' { total } else { -total };
            }
            Some(_) => return None,
        }
    }
    Some(days_from_civil(year, month, day) * 86_400 + secs_of_day - offset_secs)
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if is_leap(y) => 29,
        _ => 28,
    }
}

/// Days since 1970-01-01 of a proleptic Gregorian date (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jira_and_rfc3339_formats() {
        // 2024-05-01T10:15:30Z
        let base = 1_714_558_530;
        assert_eq!(parse_timestamp("2024-05-01T10:15:30.123+0000"), Some(base));
        assert_eq!(parse_timestamp("2024-05-01T10:15:30.123+00:00"), Some(base));
        assert_eq!(parse_timestamp("2024-05-01T10:15:30Z"), Some(base));
        assert_eq!(parse_timestamp("2024-05-01 10:15:30"), Some(base));
        assert_eq!(parse_timestamp("2024-05-01T10:15Z"), Some(base - 30));
        assert_eq!(parse_timestamp("2024-05-01T12:15:30+0200"), Some(base));
        assert_eq!(parse_timestamp("2024-05-01T05:45:30-04:30"), Some(base));
        assert_eq!(parse_timestamp("2024-05-01T12:15:30+02"), Some(base));
    }

    #[test]
    fn epochs_dates_and_leap_years() {
        assert_eq!(parse_timestamp("0"), Some(0));
        assert_eq!(parse_timestamp("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_timestamp("1969-12-31T23:59:59Z"), Some(-1));
        assert_eq!(parse_timestamp("1714558530"), Some(1_714_558_530));
        assert_eq!(parse_timestamp("1714558530123"), Some(1_714_558_530));
        assert_eq!(parse_timestamp("2024-02-29"), Some(1_709_164_800));
        assert_eq!(parse_timestamp("2000-03-01T00:00:00Z"), Some(951_868_800));
    }

    #[test]
    fn rejects_garbage_without_panicking() {
        for bad in [
            "",
            "yesterday",
            "2023-02-29",
            "2024-13-01",
            "2024-05-01T25:00:00Z",
            "2024-05-01T10:15:30+25:00",
            "2024-05-01T10:15:30+0",
            "2024-05-01T10:15:30 UTC",
            "2024-05-01T10:15:30.Z",
            "2024-05-01Tхх:15",
            "2024-05",
        ] {
            assert_eq!(parse_timestamp(bad), None, "{bad}");
        }
    }
}
