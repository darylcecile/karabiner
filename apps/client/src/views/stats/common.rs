//! Date helpers shared by the Home, document and review views. Visual primitives live in
//! `crate::components`.

use std::time::{SystemTime, UNIX_EPOCH};

/// Seconds since the Unix epoch for an ISO-8601 UTC timestamp (`2026-11-24T16:35:55.123Z`).
pub fn parse_timestamp(value: &str) -> Option<i64> {
    let (date, time) = value.split_once('T')?;
    let mut date_parts = date.splitn(3, '-').map(|part| part.parse::<i64>().ok());
    let (year, month, day) = (date_parts.next()??, date_parts.next()??, date_parts.next()??);
    let clock = time.trim_end_matches('Z');
    let clock = clock.split(['.', '+']).next()?;
    let mut clock_parts = clock.splitn(3, ':').map(|part| part.parse::<i64>().ok());
    let (hour, minute, second) = (clock_parts.next()??, clock_parts.next()??, clock_parts.next()??);
    Some(days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second)
}

/// Days since 1970-01-01 (Howard Hinnant's algorithm).
pub fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// (year, month, day) for days since 1970-01-01.
pub fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era = (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let mp = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

pub fn now_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0)
}

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/// Monday = 0.
pub fn weekday(days: i64) -> usize {
    (days + 3).rem_euclid(7) as usize
}

/// "Thu, Oct 29" (UTC).
pub fn day_label(days: i64) -> String {
    let (_, month, day) = civil_from_days(days);
    format!("{}, {} {day}", WEEKDAYS[weekday(days)], MONTHS[(month - 1) as usize])
}

pub fn month_label(days: i64) -> &'static str {
    MONTHS[(civil_from_days(days).1 - 1) as usize]
}

/// "just now", "5m ago", "3h ago", "2d ago", or a date for anything older.
pub fn relative_time(timestamp: &str) -> String {
    let Some(seconds) = parse_timestamp(timestamp) else {
        return timestamp.to_owned();
    };
    let elapsed = (now_seconds() - seconds).max(0);
    match elapsed {
        0..60 => "just now".into(),
        60..3_600 => format!("{}m ago", elapsed / 60),
        3_600..86_400 => format!("{}h ago", elapsed / 3_600),
        86_400..604_800 => format!("{}d ago", elapsed / 86_400),
        _ => day_label(seconds.div_euclid(86_400)),
    }
}

#[cfg(test)]
mod tests {
    use super::{civil_from_days, day_label, parse_timestamp};

    #[test]
    fn timestamps_round_trip_through_civil_days() {
        let seconds = parse_timestamp("2026-11-24T16:35:55.123Z").unwrap();
        let days = seconds.div_euclid(86_400);
        assert_eq!(civil_from_days(days), (2026, 11, 24));
        assert_eq!(day_label(days), "Tue, Nov 24");
    }
}
