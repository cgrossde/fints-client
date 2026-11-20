//! Shifting the dates of a trace.
//!
//! A trace moves every date it shows by the same number of days
//! (`Identities::days`, drawn per recorder), so the days between two dates
//! stay and the calendar days do not: two traces of one person cannot be
//! laid side by side by date. This is not strong protection: weekends
//! without bookings still show where the weeks are.
//!
//! Times of day go too ([`noon`]): several traces recorded in one sitting
//! would otherwise show it by their clock times, and with it the day they
//! were all made. A time is written as 12:00:00, a UTC offset as zero.
//!
//! Dates are shifted in place, in the notation they have: `YYYYMMDD`
//! (FinTS, MT535), `YYMMDD` (MT940; 20YY), `MMDD` (MT940 `:61:` entry date,
//! with the year of the value date beside it) and `YYYY-MM-DD` (camt). A
//! value that is not a date of its notation stays as it is.

/// The notation of a date in a message.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Form {
    /// `YYYYMMDD`.
    Ymd8,
    /// `YYMMDD`, read as 20YY.
    Ymd6,
    /// `YYYY-MM-DD`.
    Iso,
}

impl Form {
    pub(crate) fn len(self) -> usize {
        match self {
            Form::Ymd8 => 8,
            Form::Ymd6 => 6,
            Form::Iso => 10,
        }
    }
}

/// Days since 1970-01-01 of a civil date (proleptic Gregorian calendar).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The civil date of a day count.
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

fn number(b: &[u8]) -> Option<i64> {
    if b.is_empty() || !b.iter().all(u8::is_ascii_digit) {
        return None;
    }
    Some(b.iter().fold(0, |n, &c| n * 10 + i64::from(c - b'0')))
}

fn valid(y: i64, m: i64, d: i64) -> bool {
    (1900..=2099).contains(&y)
        && (1..=12).contains(&m)
        && d >= 1
        && civil_from_days(days_from_civil(y, m, d)) == (y, m, d)
}

/// Year, month and day of `b` read as `form`, if it is such a date.
fn read(b: &[u8], form: Form) -> Option<(i64, i64, i64)> {
    if b.len() != form.len() {
        return None;
    }
    let (y, m, d) = match form {
        Form::Ymd8 => (number(&b[..4])?, number(&b[4..6])?, number(&b[6..])?),
        Form::Ymd6 => (2000 + number(&b[..2])?, number(&b[2..4])?, number(&b[4..])?),
        Form::Iso if b[4] == b'-' && b[7] == b'-' => {
            (number(&b[..4])?, number(&b[5..7])?, number(&b[8..])?)
        }
        Form::Iso => return None,
    };
    valid(y, m, d).then_some((y, m, d))
}

/// Whether `b` is a date of `form`.
pub(crate) fn is_date(b: &[u8], form: Form) -> bool {
    read(b, form).is_some()
}

/// Moves the date in `b` (of notation `form`) by `days`, in place. A value
/// that is not such a date stays.
pub(crate) fn shift(b: &mut [u8], form: Form, days: i32) {
    let Some((y, m, d)) = read(b, form) else {
        return;
    };
    let (y, m, d) = civil_from_days(days_from_civil(y, m, d) + i64::from(days));
    let text = match form {
        Form::Ymd8 => format!("{y:04}{m:02}{d:02}"),
        Form::Ymd6 => format!("{:02}{m:02}{d:02}", y.rem_euclid(100)),
        Form::Iso => format!("{y:04}-{m:02}-{d:02}"),
    };
    if text.len() == b.len() {
        b.copy_from_slice(text.as_bytes());
    }
}

/// Moves an MT940 entry date `MMDD` by `days`, in place. It has no year:
/// it is the day nearest to the value date `value` (`YYMMDD`, as sent) in
/// that year, the one before or the one after.
pub(crate) fn shift_entry(b: &mut [u8], value: &[u8], days: i32) {
    let (Some((vy, vm, vd)), Some(m), Some(d)) = (
        read(value, Form::Ymd6),
        b.get(..2).and_then(number),
        b.get(2..4).and_then(number),
    ) else {
        return;
    };
    if b.len() != 4 {
        return;
    }
    let at = days_from_civil(vy, vm, vd);
    let Some(day) = [vy - 1, vy, vy + 1]
        .into_iter()
        .filter(|&y| valid(y, m, d))
        .map(|y| days_from_civil(y, m, d))
        .min_by_key(|&day| (day - at).abs())
    else {
        return;
    };
    let (_, m, d) = civil_from_days(day + i64::from(days));
    b.copy_from_slice(format!("{m:02}{d:02}").as_bytes());
}

/// Writes a time of day, and what follows it, as noon: the first two
/// digits become `12`, every other digit `0` (`101530` → `120000`,
/// `T10:15:30.123+01:00` → `T12:00:00.000+00:00`). Everything else stays.
pub(crate) fn noon(b: &mut [u8]) {
    for (n, c) in b.iter_mut().filter(|c| c.is_ascii_digit()).enumerate() {
        *c = match n {
            0 => b'1',
            1 => b'2',
            _ => b'0',
        };
    }
}

/// A FinTS time element (`hhmmss`) as noon; any other text as it is.
pub(crate) fn noon_text(text: &str) -> String {
    if text.len() != 6 || !text.bytes().all(|c| c.is_ascii_digit()) {
        return text.to_string();
    }
    "120000".to_string()
}

/// A FinTS date element (`YYYYMMDD`) moved by `days`; any other text as it
/// is.
pub(crate) fn shift_text(text: &str, days: i32) -> String {
    let mut b = text.as_bytes().to_vec();
    shift(&mut b, Form::Ymd8, days);
    String::from_utf8(b).unwrap_or_else(|_| text.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn moved(text: &str, form: Form, days: i32) -> String {
        let mut b = text.as_bytes().to_vec();
        shift(&mut b, form, days);
        String::from_utf8(b).unwrap()
    }

    #[test]
    fn dates_move_in_their_notation() {
        assert_eq!(moved("20261120", Form::Ymd8, -20), "20261031");
        assert_eq!(moved("20260301", Form::Ymd8, -1), "20260228");
        assert_eq!(moved("20240301", Form::Ymd8, -1), "20240229");
        assert_eq!(moved("260105", Form::Ymd6, -10), "251226");
        assert_eq!(moved("2026-01-05", Form::Iso, -10), "2025-12-26");
        assert_eq!(moved("20261120", Form::Ymd8, 0), "20261120");
    }

    #[test]
    fn what_is_not_a_date_stays() {
        assert_eq!(moved("20041155", Form::Ymd8, -5), "20041155");
        assert_eq!(moved("20260230", Form::Ymd8, -5), "20260230");
        assert_eq!(moved("########", Form::Ymd8, -5), "########");
        assert_eq!(moved("2026/01/05", Form::Iso, -5), "2026/01/05");
        assert_eq!(shift_text("N", -5), "N");
        assert_eq!(shift_text("20261120", -5), "20261115");
    }

    #[test]
    fn a_time_of_day_becomes_noon() {
        let at_noon = |text: &str| {
            let mut b = text.as_bytes().to_vec();
            noon(&mut b);
            String::from_utf8(b).unwrap()
        };
        assert_eq!(at_noon("101530"), "120000");
        assert_eq!(at_noon("T10:15:30.123+01:00"), "T12:00:00.000+00:00");
        assert_eq!(at_noon("1230+0100"), "1200+0000");
        assert_eq!(at_noon(""), "");
        assert_eq!(noon_text("235959"), "120000");
        assert_eq!(noon_text("12345"), "12345");
        assert_eq!(noon_text("N"), "N");
    }

    #[test]
    fn an_entry_date_takes_its_year_from_the_value_date() {
        let entry = |mmdd: &str, value: &str, days: i32| {
            let mut b = mmdd.as_bytes().to_vec();
            shift_entry(&mut b, value.as_bytes(), days);
            String::from_utf8(b).unwrap()
        };
        assert_eq!(entry("1120", "261120", -20), "1031");
        // Value date 2 January, entered 30 December of the year before.
        assert_eq!(entry("1230", "260102", -3), "1227");
        assert_eq!(entry("0102", "251230", -3), "1230");
        assert_eq!(entry("0229", "240301", -1), "0228");
        assert_eq!(entry("9999", "261120", -1), "9999");
    }
}
