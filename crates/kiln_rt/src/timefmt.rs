//! Times and dates as the database stores them: ISO 8601 text, in UTC.

/// Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm, as
/// `k_civil` in `kiln.c`).
fn civil(days: i64) -> (i64, i64, i64) {
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let doe = days - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// A civil date to days since 1970-01-01.
fn days(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

const NS: i64 = 1_000_000_000;

/// `2026-10-24T21:30:00Z`, with the fraction of a second when there is
/// one, as `Instant.iso()` writes it.
pub fn iso(unix_nanos: i64) -> String {
    let secs = unix_nanos.div_euclid(NS);
    let nanos = unix_nanos.rem_euclid(NS);
    let (y, m, d) = civil(secs.div_euclid(86_400));
    let t = secs.rem_euclid(86_400);
    let mut s = format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}",
        t / 3600,
        t / 60 % 60,
        t % 60
    );
    if nanos != 0 {
        let frac = format!("{nanos:09}");
        s.push('.');
        s.push_str(frac.trim_end_matches('0'));
    }
    s.push('Z');
    s
}

/// `YYYY-MM-DD`.
pub fn date(s: &str) -> Option<(i64, i64, i64)> {
    let s = s.get(..10)?;
    let b = s.as_bytes();
    if b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let y = s[..4].parse().ok()?;
    let m = s[5..7].parse().ok()?;
    let d = s[8..10].parse().ok()?;
    ((1..=12).contains(&m) && (1..=31).contains(&d)).then_some((y, m, d))
}

/// Reads `2026-10-24T21:30:00Z`, `2026-10-24 21:30:00` (taken as UTC, as
/// SQLite's `CURRENT_TIMESTAMP` writes it), offsets such as `+02:00`,
/// fractions of a second, or a date alone, as nanoseconds since 1970.
pub fn parse(s: &str) -> Option<i64> {
    let s = s.trim();
    let (y, m, d) = date(s)?;
    let mut secs = days(y, m, d) * 86_400;
    let rest = &s[10..];
    if rest.is_empty() {
        return Some(secs * NS);
    }
    let rest = rest.strip_prefix(['T', 't', ' '])?;
    let b = rest.as_bytes();
    if b.len() < 8 || b[2] != b':' || b[5] != b':' {
        return None;
    }
    let h: i64 = rest[..2].parse().ok()?;
    let mi: i64 = rest[3..5].parse().ok()?;
    let sec: i64 = rest[6..8].parse().ok()?;
    secs += h * 3600 + mi * 60 + sec;
    let mut rest = &rest[8..];
    let mut nanos = 0i64;
    if let Some(frac) = rest.strip_prefix('.') {
        let digits = frac.bytes().take_while(u8::is_ascii_digit).count();
        let mut scaled = 0i64;
        for (i, c) in frac[..digits].bytes().enumerate() {
            if i < 9 {
                scaled = scaled * 10 + (c - b'0') as i64;
            }
        }
        for _ in digits.min(9)..9 {
            scaled *= 10;
        }
        nanos = scaled;
        rest = &frac[digits..];
    }
    let offset = match rest {
        "" | "Z" | "z" | "+00" | "+00:00" => 0,
        _ => {
            let sign = match rest.as_bytes()[0] {
                b'+' => 1,
                b'-' => -1,
                _ => return None,
            };
            let digits: String = rest[1..].chars().filter(char::is_ascii_digit).collect();
            if digits.len() != 4 && digits.len() != 2 {
                return None;
            }
            let oh: i64 = digits[..2].parse().ok()?;
            let om: i64 = digits.get(2..).map_or(Some(0), |m| m.parse().ok())?;
            sign * (oh * 3600 + om * 60)
        }
    };
    Some((secs - offset) * NS + nanos)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_round_trip() {
        let t = parse("2026-10-24T21:30:00Z").unwrap();
        assert_eq!(iso(t), "2026-10-24T21:30:00Z");
        assert_eq!(parse("2026-10-24 21:30:00"), Some(t));
        assert_eq!(parse("2026-10-24T23:30:00+02:00"), Some(t));
        assert_eq!(
            iso(parse("1969-12-31T23:59:59.5Z").unwrap()),
            "1969-12-31T23:59:59.5Z"
        );
        assert_eq!(parse("2026-10-24"), Some(days(2026, 10, 24) * 86_400 * NS));
        assert_eq!(parse("not a time"), None);
    }
}
