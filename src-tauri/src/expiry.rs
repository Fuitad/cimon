//! Pure UTC expiry math for token-health warnings.
//!
//! No external date crate: both providers report token expiry in UTC, so a civil-days conversion
//! (Howard Hinnant's `days_from_civil` algorithm) plus integer arithmetic is enough and stays
//! trivially unit-testable. The poller (Task 3) uses these to decide when to warn before expiry.

use std::time::{SystemTime, UNIX_EPOCH};

/// Warning thresholds in hours, DESCENDING. The poller warns once as the token enters each bracket.
pub const THRESHOLDS_HOURS: [i64; 2] = [72, 24];

/// Days from the civil date `y-m-d` to 1970-01-01 (Howard Hinnant's `days_from_civil`). Valid for
/// any proleptic Gregorian date; negative before the epoch. Source: howardhinnant.github.io/date_algorithms.html
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400; // [0, 399]
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146097 + doe - 719468
}

/// Number of days in civil month `m` (1..=12) of year `y`, honoring Gregorian leap years. Used to
/// reject impossible dates (e.g. Feb 31) that `days_from_civil` would otherwise silently roll forward
/// into the next month. Returns 0 for an out-of-range month.
fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        2 => 28,
        _ => 0,
    }
}

/// Parse a provider expiry string into UTC epoch seconds. Accepts GitLab `YYYY-MM-DD` (midnight
/// UTC) and GitHub `YYYY-MM-DD HH:MM:SS UTC` (also tolerates a `T` separator and trailing `Z`).
/// Any malformed/out-of-range input returns `None` rather than panicking.
pub fn parse_expiry(s: &str) -> Option<i64> {
    let s = s.trim();
    // Split the date from an optional time, tolerating both a space and a `T` separator.
    let (date_part, time_part) = match s.split_once([' ', 'T']) {
        Some((d, t)) => (d, Some(t)),
        None => (s, None),
    };
    let mut dit = date_part.split('-');
    let y: i64 = dit.next()?.parse().ok()?;
    let mo: i64 = dit.next()?.parse().ok()?;
    let d: i64 = dit.next()?.parse().ok()?;
    if dit.next().is_some() || !(1..=12).contains(&mo) {
        return None;
    }
    // Validate the day per-month (so Feb 31 / Apr 31 are rejected, not rolled forward by
    // `days_from_civil`). `mo` is now known to be in 1..=12.
    if !(1..=days_in_month(y, mo)).contains(&d) {
        return None;
    }
    let (mut hh, mut mm, mut ss) = (0i64, 0i64, 0i64);
    if let Some(t) = time_part {
        // Strip a trailing `Z` or ` UTC` (both providers report UTC) before parsing HH:MM:SS.
        let t = t
            .trim()
            .trim_end_matches('Z')
            .trim_end_matches("UTC")
            .trim();
        let mut tit = t.split(':');
        hh = tit.next()?.parse().ok()?;
        mm = tit.next()?.parse().ok()?;
        ss = tit.next().unwrap_or("0").parse().ok()?;
        if tit.next().is_some()
            || !(0..=23).contains(&hh)
            || !(0..=59).contains(&mm)
            || !(0..=60).contains(&ss)
        {
            return None;
        }
        // A leap second (`:60`) is a valid wall-clock value, but the civil-days timeline we map onto
        // has no slot for it. Clamp it to `:59` of the SAME minute so it never rolls forward into the
        // next minute (and, at 23:59:60, into the next day).
        if ss == 60 {
            ss = 59;
        }
    }
    Some(days_from_civil(y, mo, d) * 86_400 + hh * 3_600 + mm * 60 + ss)
}

/// Current UNIX time in whole seconds (UTC). `0` if the clock is before the epoch (never in practice).
pub fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Whole hours from `now_secs` until `expiry_secs` (negative once expired). Truncates toward zero.
pub fn hours_until(expiry_secs: i64, now_secs: i64) -> i64 {
    (expiry_secs - now_secs) / 3_600
}

/// Whole UTC calendar days from `now_secs` until the parsed `expires_at` (negative once past, `0`
/// on the expiry day), or `None` when `expires_at` cannot be parsed. Floors each instant to its UTC
/// civil day before differencing, so the result is stable across the viewer's timezone and never
/// over-reports the way ceiling a fractional 24h chunk would. This is the single source of truth for
/// "days until expiry": the frontend renders this value instead of re-parsing the provider string.
pub fn days_until(expires_at: &str, now_secs: i64) -> Option<i64> {
    let exp = parse_expiry(expires_at)?;
    // `div_euclid` floors toward negative infinity (unlike `/`, which truncates toward zero), so a
    // pre-epoch or pre-`now` instant lands on the correct earlier civil day.
    Some(exp.div_euclid(86_400) - now_secs.div_euclid(86_400))
}

/// Which warning bracket the token is CURRENTLY in, by ascending ceiling. A token at 25h is still
/// in the 72h bracket (returns `Some(72)`), NOT the 24h one. Expired (`<= 0`) returns `None` because
/// an expired token is an auth-failure, handled separately by the poller.
pub fn current_bracket(hours_remaining: i64) -> Option<i64> {
    if hours_remaining <= 0 {
        return None;
    }
    // Smallest threshold the token is at or under (THRESHOLDS_HOURS is descending, so iterate
    // reversed, i.e. ascending): 25h -> 72, 24h -> 24, 10h -> 24.
    THRESHOLDS_HOURS
        .iter()
        .rev()
        .copied()
        .find(|&t| hours_remaining <= t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_from_civil_matches_known_epochs() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(1970, 1, 2), 1);
        assert_eq!(days_from_civil(1969, 12, 31), -1);
        // 1970-01-01 .. 2000-01-01 = 30*365 + 7 leap days (1972,76,80,84,88,92,96) = 10957.
        assert_eq!(days_from_civil(2000, 1, 1), 10_957);
    }

    #[test]
    fn parse_expiry_handles_both_formats() {
        assert_eq!(parse_expiry("1970-01-01 00:00:00 UTC"), Some(0));
        assert_eq!(parse_expiry("1970-01-02"), Some(86_400));
        assert_eq!(parse_expiry("1970-01-02 00:00:01 UTC"), Some(86_401));
        // T-separator + Z also parse (defensive; same instant as the space/UTC form).
        assert_eq!(
            parse_expiry("1970-01-02T00:00:01Z"),
            parse_expiry("1970-01-02 00:00:01 UTC")
        );
        // A real future date computed via the same civil-days path (second-path cross-check).
        assert_eq!(
            parse_expiry("2026-08-15"),
            Some(days_from_civil(2026, 8, 15) * 86_400)
        );
    }

    #[test]
    fn parse_expiry_rejects_malformed() {
        assert_eq!(parse_expiry(""), None);
        assert_eq!(parse_expiry("not-a-date"), None);
        assert_eq!(parse_expiry("2026-13-01"), None); // month out of range
        assert_eq!(parse_expiry("2026-08-40"), None); // day out of range
        assert_eq!(parse_expiry("2026-08"), None); // missing day
        assert_eq!(parse_expiry("2026-08-15 99:00:00 UTC"), None); // hour out of range
    }

    #[test]
    fn parse_expiry_rejects_impossible_calendar_days() {
        // In-range (1..=31) but impossible for the month: must be rejected, not silently rolled
        // forward into the next month by days_from_civil.
        assert_eq!(parse_expiry("2026-02-31"), None);
        assert_eq!(parse_expiry("2026-02-30"), None);
        assert_eq!(parse_expiry("2026-02-29"), None); // 2026 is not a leap year
        assert_eq!(parse_expiry("2026-04-31"), None); // April has 30 days
                                                      // Valid month-end edges still parse.
        assert_eq!(
            parse_expiry("2024-02-29"), // 2024 is a leap year
            Some(days_from_civil(2024, 2, 29) * 86_400)
        );
        assert_eq!(
            parse_expiry("2026-01-31"),
            Some(days_from_civil(2026, 1, 31) * 86_400)
        );
        assert_eq!(
            parse_expiry("2026-04-30"),
            Some(days_from_civil(2026, 4, 30) * 86_400)
        );
    }

    #[test]
    fn hours_until_positive_and_negative() {
        let now = 1_000_000;
        assert_eq!(hours_until(now + 7_200, now), 2);
        assert_eq!(hours_until(now - 3_600, now), -1);
        assert_eq!(hours_until(now, now), 0);
    }

    #[test]
    fn days_until_counts_whole_utc_days() {
        let exp_day = days_from_civil(2026, 8, 15); // civil-day index of the expiry date
        let exp = "2026-08-15";
        // Midday the day before -> 1 day remaining (floors to the next-day boundary).
        assert_eq!(
            days_until(exp, (exp_day - 1) * 86_400 + 12 * 3_600),
            Some(1)
        );
        // Midday on the expiry day -> 0 ("expires today").
        assert_eq!(days_until(exp, exp_day * 86_400 + 12 * 3_600), Some(0));
        // Two days past -> -2 (already expired): the case that used to render "Expires today".
        assert_eq!(days_until(exp, (exp_day + 2) * 86_400 + 3_600), Some(-2));
        // Unparseable -> None: the guard the frontend lacked (it rendered "Expires in NaN days").
        assert_eq!(days_until("not-a-date", exp_day * 86_400), None);
    }

    #[test]
    fn parse_expiry_clamps_leap_second_to_same_minute() {
        // A leap second (:60) clamps to :59 of the SAME minute, never rolling into the next minute.
        assert_eq!(
            parse_expiry("2026-08-15 23:59:60 UTC"),
            parse_expiry("2026-08-15 23:59:59 UTC")
        );
        // It must NOT equal the next day's midnight, which a raw +60s roll would produce.
        assert_ne!(
            parse_expiry("2026-08-15 23:59:60 UTC"),
            Some(days_from_civil(2026, 8, 16) * 86_400)
        );
    }

    #[test]
    fn current_bracket_ladder() {
        assert_eq!(current_bracket(80), None);
        assert_eq!(current_bracket(73), None);
        assert_eq!(current_bracket(72), Some(72));
        assert_eq!(current_bracket(70), Some(72));
        assert_eq!(current_bracket(25), Some(72));
        assert_eq!(current_bracket(24), Some(24));
        assert_eq!(current_bracket(10), Some(24));
        assert_eq!(current_bracket(1), Some(24));
        assert_eq!(current_bracket(0), None);
        assert_eq!(current_bracket(-5), None);
    }
}
