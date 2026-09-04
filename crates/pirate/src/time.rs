//! Reading a timestamp, and saying how long is left of one.
//!
//! A plugin has a clock — the host hands it the milliseconds since the epoch —
//! and nothing else. There is no calendar in a sandbox, so the two things a
//! countdown needs are here: turning the RFC 3339 stamp Anthropic answers with
//! into that same number, and turning the difference into the four words a
//! panel has room for.
//!
//! Written out rather than taken from a date library because a date library is
//! a hundred kilobytes of a plugin somebody downloads, and this needs exactly
//! two operations, neither of which involves a time zone: the stamp is UTC and
//! so is the clock.

/// Milliseconds since the epoch for an RFC 3339 stamp, or `None` for anything
/// this does not recognise.
///
/// Deliberately narrow: `YYYY-MM-DDTHH:MM:SS`, then an optional fraction that
/// is read to milliseconds and truncated past them, then `Z` or `±HH:MM`. That
/// is what the endpoint answers with, and a parser that accepted more would be
/// a parser with more to be wrong about.
pub fn parse_rfc3339(stamp: &str) -> Option<i64> {
    let bytes = stamp.as_bytes();
    if bytes.len() < 19 {
        return None;
    }

    let year: i64 = stamp.get(0..4)?.parse().ok()?;
    let month: i64 = stamp.get(5..7)?.parse().ok()?;
    let day: i64 = stamp.get(8..10)?.parse().ok()?;
    let hour: i64 = stamp.get(11..13)?.parse().ok()?;
    let minute: i64 = stamp.get(14..16)?.parse().ok()?;
    let second: i64 = stamp.get(17..19)?.parse().ok()?;

    if bytes[4] != b'-' || bytes[7] != b'-' || bytes[13] != b':' || bytes[16] != b':' {
        return None;
    }
    // The date and the time are separated by a `T`, and by nothing else:
    // a space is what a log line uses and not what RFC 3339 means.
    if !matches!(bytes[10], b'T' | b't') {
        return None;
    }
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    if hour > 23 || minute > 59 || second > 60 {
        return None;
    }

    let mut rest = stamp.get(19..)?;

    // A fraction, read to three places and truncated. Anthropic sends six.
    let mut millis = 0;
    if let Some(fraction) = rest.strip_prefix('.') {
        let digits: String = fraction.chars().take_while(char::is_ascii_digit).collect();
        if digits.is_empty() {
            return None;
        }
        let mut places = digits.chars().take(3);
        for scale in [100, 10, 1] {
            millis += places
                .next()
                .and_then(|digit| digit.to_digit(10))
                .unwrap_or(0) as i64
                * scale;
        }
        rest = rest.get(1 + digits.len()..)?;
    }

    // The offset, which is what makes the stamp mean an instant.
    let offset = match rest.as_bytes().first() {
        Some(b'Z' | b'z') if rest.len() == 1 => 0,
        Some(sign @ (b'+' | b'-')) if rest.len() == 6 => {
            let hours: i64 = rest.get(1..3)?.parse().ok()?;
            let minutes: i64 = rest.get(4..6)?.parse().ok()?;
            if rest.as_bytes()[3] != b':' || hours > 23 || minutes > 59 {
                return None;
            }
            let magnitude = hours * 3_600 + minutes * 60;
            if *sign == b'-' { -magnitude } else { magnitude }
        }
        _ => return None,
    };

    let seconds = days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second;
    Some((seconds - offset) * 1_000 + millis)
}

/// Days from 1970-01-01 to a civil date, for any year the calendar covers.
///
/// Howard Hinnant's algorithm, which is the one everything else uses: it shifts
/// the year to start in March so that the leap day is the last of it and the
/// month lengths fall into a pattern with no table.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// An instant as the endpoint and the transcripts both write one.
///
/// Only ever used to say "at or after this", which is a string comparison the
/// host makes against the stamps it finds — so this has to produce exactly the
/// shape those are in, and nothing else about it matters.
pub fn format_rfc3339(millis: i64) -> String {
    let days = millis.div_euclid(86_400_000);
    let rest = millis.rem_euclid(86_400_000) / 1_000;
    let (year, month, day) = civil_from_days(days);

    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3_600,
        (rest % 3_600) / 60,
        rest % 60
    )
}

/// The civil date a day number falls on.
///
/// Howard Hinnant's algorithm again, run the other way: the same shifted year
/// starting in March, so that the leap day is the last of it.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let days = days + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted + 2) / 5 + 1;
    let month = if shifted < 10 {
        shifted + 3
    } else {
        shifted - 9
    };

    (year + i64::from(month <= 2), month, day)
}

/// How long is left, in the shortest phrase that is still true.
///
/// Truncates rather than rounds — 119 seconds reads as "1m" — and the day
/// branch drops minutes entirely, because a reset four days away is not a
/// thing anybody is counting minutes to. The same rule the chip followed when
/// it was in the box.
pub fn format_countdown(remaining_millis: i64) -> String {
    let seconds = remaining_millis / 1_000;
    if seconds <= 0 {
        return String::from("now");
    }

    let days = seconds / 86_400;
    let hours = (seconds % 86_400) / 3_600;
    let minutes = (seconds % 3_600) / 60;
    if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h {minutes}m")
    } else {
        format!("{minutes}m")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_epoch_is_where_everything_is_measured_from() {
        assert_eq!(parse_rfc3339("1970-01-01T00:00:00Z"), Some(0));
    }

    #[test]
    fn a_stamp_the_endpoint_actually_sends_is_read() {
        // Six places of fraction, truncated to three, and a Z.
        assert_eq!(
            parse_rfc3339("2026-09-04T18:30:00.123456Z"),
            Some(1_788_546_600_123)
        );
    }

    #[test]
    fn an_offset_moves_the_instant_the_other_way() {
        // 12:00 two hours east of UTC is 10:00 UTC.
        assert_eq!(
            parse_rfc3339("2026-09-04T12:00:00+02:00"),
            parse_rfc3339("2026-09-04T10:00:00Z")
        );
        assert_eq!(
            parse_rfc3339("2026-09-04T12:00:00-05:30"),
            parse_rfc3339("2026-09-04T17:30:00Z")
        );
    }

    #[test]
    fn a_leap_day_is_a_day() {
        let leap = parse_rfc3339("2024-02-29T00:00:00Z").expect("a leap day is a date");
        let after = parse_rfc3339("2024-03-01T00:00:00Z").expect("and so is the day after it");

        assert_eq!(after - leap, 86_400_000);
    }

    #[test]
    fn a_century_that_is_not_a_leap_year_is_not_one() {
        // 1900 was not a leap year and 2000 was, which is the whole reason the
        // algorithm above is not "divisible by four".
        let before = parse_rfc3339("1900-02-28T00:00:00Z").expect("a date");
        let after = parse_rfc3339("1900-03-01T00:00:00Z").expect("a date");

        assert_eq!(after - before, 86_400_000);
    }

    #[test]
    fn anything_that_is_not_a_stamp_is_refused_rather_than_guessed_at() {
        for nonsense in [
            "",
            "2026-09-04",
            "2026-09-04 18:30:00Z",
            "2026-13-04T18:30:00Z",
            "2026-09-04T18:30:00",
            "2026-09-04T18:30:00+0200",
            "2026-09-04T18:30:00.Z",
            "not a date at all",
        ] {
            assert_eq!(
                parse_rfc3339(nonsense),
                None,
                "{nonsense:?} was read as a date"
            );
        }
    }

    #[test]
    fn a_stamp_written_here_is_read_back_by_the_same_reader() {
        // The one thing that matters about writing one: the host compares it
        // as text against the stamps in the transcripts, so it has to be in
        // exactly their shape.
        for millis in [
            0,
            1_788_546_600_000,
            // A leap day, and the day either side of one.
            1_709_164_800_000,
            1_709_251_200_000,
            // A century that was not a leap year.
            -2_203_891_200_000,
        ] {
            let written = format_rfc3339(millis);
            assert_eq!(parse_rfc3339(&written), Some(millis), "{written}");
        }

        assert_eq!(format_rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_rfc3339(1_788_546_600_000), "2026-09-04T18:30:00Z");
    }

    #[test]
    fn a_countdown_says_the_largest_two_units_it_has() {
        assert_eq!(format_countdown(0), "now");
        assert_eq!(format_countdown(-5_000), "now");
        assert_eq!(format_countdown(119_000), "1m");
        assert_eq!(format_countdown(3_600_000 * 3 + 60_000 * 12), "3h 12m");
        assert_eq!(format_countdown(86_400_000 * 4 + 3_600_000 * 2), "4d 2h");
    }
}
