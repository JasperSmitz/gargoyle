//! RFC 3339 timestamps (`generated_at`). Hand-written because nothing else in Gargoyle needs
//! calendar code (ADR 0007).

/// Parses `YYYY-MM-DDTHH:MM:SS[.fraction](Z|±HH:MM)` into unix seconds. The fraction is
/// dropped (rounded towards earlier); `t` and `z` may be lowercase, as RFC 3339 allows. A leap
/// second (`:60`) counts as the first second of the next minute. Returns `None` for anything
/// else, including impossible dates such as February 30.
pub fn parse_rfc3339(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 20 {
        return None;
    }
    let year = digits(b, 0, 4)?;
    let month = digits(b, 5, 2)?;
    let day = digits(b, 8, 2)?;
    let hour = digits(b, 11, 2)?;
    let minute = digits(b, 14, 2)?;
    let second = digits(b, 17, 2)?;
    if b[4] != b'-'
        || b[7] != b'-'
        || !matches!(b[10], b'T' | b't')
        || b[13] != b':'
        || b[16] != b':'
    {
        return None;
    }
    if !(1..=12).contains(&month)
        || day == 0
        || day > days_in_month(year, month)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }

    let mut rest = &b[19..];
    if let Some(after_dot) = rest.strip_prefix(b".") {
        let n = after_dot.iter().take_while(|c| c.is_ascii_digit()).count();
        if n == 0 {
            return None;
        }
        rest = &after_dot[n..];
    }
    let offset = match rest {
        [b'Z' | b'z'] => 0,
        [sign @ (b'+' | b'-'), _, _, b':', _, _] => {
            let h = digits(rest, 1, 2)?;
            let m = digits(rest, 4, 2)?;
            if h > 23 || m > 59 {
                return None;
            }
            let offset = h * 3600 + m * 60;
            if *sign == b'-' { -offset } else { offset }
        }
        _ => return None,
    };

    let days = days_from_civil(year, month, day);
    Some(days * 86_400 + hour * 3600 + minute * 60 + second - offset)
}

/// `len` ASCII digits at `at`.
fn digits(b: &[u8], at: usize, len: usize) -> Option<i64> {
    let part = b.get(at..at + len)?;
    part.iter().try_fold(0i64, |n, &c| {
        c.is_ascii_digit().then(|| n * 10 + i64::from(c - b'0'))
    })
}

fn is_leap(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        2 if is_leap(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Days since 1970-01-01 of a proleptic Gregorian date (Howard Hinnant's `days_from_civil`).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_timestamps() {
        let rows = [
            ("1970-01-01T00:00:00Z", 0),
            ("2026-11-08T10:00:00Z", 1_794_132_000),
            ("2026-11-08t10:00:00z", 1_794_132_000),
            ("2026-11-08T10:00:00.999999Z", 1_794_132_000),
            ("2026-11-08T10:00:00.5+00:00", 1_794_132_000),
            ("2026-11-08T11:30:00+01:30", 1_794_132_000),
            ("2026-11-08T08:00:00-02:00", 1_794_132_000),
            ("2000-02-29T00:00:00Z", 951_782_400),
            ("1969-12-31T23:59:59Z", -1),
            ("2016-12-31T23:59:60Z", 1_483_228_800),
            ("2038-01-19T03:14:08Z", 2_147_483_648),
            ("0000-01-01T00:00:00Z", -62_167_219_200),
            ("9999-12-31T23:59:59Z", 253_402_300_799),
        ];
        for (s, expected) in rows {
            assert_eq!(parse_rfc3339(s), Some(expected), "{s}");
        }
    }

    #[test]
    fn invalid_timestamps() {
        for s in [
            "",
            "2026-11-08",
            "2026-11-08T10:00:00",
            "2026-11-08 10:00:00Z",
            "2026-11-08T10:00Z",
            "2026/11/08T10:00:00Z",
            "2026-13-08T10:00:00Z",
            "2026-00-08T10:00:00Z",
            "2026-11-00T10:00:00Z",
            "2026-11-31T10:00:00Z",
            "2026-02-29T10:00:00Z",
            "1900-02-29T10:00:00Z",
            "2026-11-08T24:00:00Z",
            "2026-11-08T10:60:00Z",
            "2026-11-08T10:00:61Z",
            "2026-11-08T10:00:00.Z",
            "2026-11-08T10:00:00+0100",
            "2026-11-08T10:00:00+24:00",
            "2026-11-08T10:00:00+01:60",
            "2026-11-08T10:00:00ZZ",
            "2026-11-08T10:00:00Z ",
            "+2026-11-08T10:00:00Z",
            "２026-11-08T10:00:00Z",
            "2026-1a-08T10:00:00Z",
        ] {
            assert_eq!(parse_rfc3339(s), None, "{s:?}");
        }
    }

    #[test]
    fn every_day_of_a_leap_cycle_is_one_day_apart() {
        let mut prev = days_from_civil(1999, 12, 31);
        for year in 2000..2401 {
            for month in 1..=12 {
                for day in 1..=days_in_month(year, month) {
                    let d = days_from_civil(year, month, day);
                    assert_eq!(d, prev + 1, "{year}-{month}-{day}");
                    prev = d;
                }
            }
        }
    }
}
