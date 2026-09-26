//! Date formatting for the two timestamps the UI shows, without pulling in a
//! date library.

use twilight_model::util::Timestamp;

/// Milliseconds between the Unix epoch and Discord's (2015-01-01), the offset
/// the timestamp inside a snowflake is measured from.
const DISCORD_EPOCH_MS: u64 = 1_420_070_400_000;

/// Formats a Discord timestamp as `YYYY-MM-DD HH:MM` (UTC). Its ISO 8601 form
/// is `2021-08-10T11:16:37.020000+00:00`.
pub(super) fn format_timestamp(timestamp: Timestamp) -> String {
    let iso = timestamp.iso_8601().to_string();
    match (iso.get(..10), iso.get(11..16)) {
        (Some(date), Some(time)) => format!("{date} {time}"),
        _ => iso,
    }
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// Formats an embed's footer timestamp as `Jan 5, 2021 4:56 PM` (UTC), the
/// long form Discord falls back to for anything older than yesterday.
pub(super) fn format_embed_timestamp(timestamp: Timestamp) -> String {
    // `2021-08-02T16:56:43.772000+00:00`, so the fields sit at fixed offsets.
    let iso = timestamp.iso_8601().to_string();
    let parsed = (|| {
        let year: i64 = iso.get(..4)?.parse().ok()?;
        let month: usize = iso.get(5..7)?.parse().ok()?;
        let day: u32 = iso.get(8..10)?.parse().ok()?;
        let hour: u32 = iso.get(11..13)?.parse().ok()?;
        let minute: u32 = iso.get(14..16)?.parse().ok()?;
        let name = MONTHS.get(month.checked_sub(1)?)?;
        let (hour12, meridiem) = match hour {
            0 => (12, "AM"),
            1..=11 => (hour, "AM"),
            12 => (12, "PM"),
            _ => (hour - 12, "PM"),
        };
        Some(format!(
            "{name} {day}, {year} {hour12}:{minute:02} {meridiem}"
        ))
    })();
    parsed.unwrap_or(iso)
}

/// Formats the creation time encoded in a snowflake as `Jan 5, 2021` (UTC),
/// the form Discord uses for "Member Since".
pub(super) fn format_snowflake_date(id: u64) -> String {
    // The upper 42 bits are milliseconds since the Discord epoch.
    let unix_ms = (id >> 22) + DISCORD_EPOCH_MS;
    let (year, month, day) = civil_from_days((unix_ms / 86_400_000) as i64);
    format!("{} {day}, {year}", MONTHS[(month - 1) as usize])
}

/// Converts days since the Unix epoch into a `(year, month, day)` civil date,
/// via Howard Hinnant's `civil_from_days` algorithm.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    // Shift the era to start on 0000-03-01, so the leap day lands at the end of
    // the year and every era is exactly 146097 days.
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    // March-based month index (0 = March … 11 = February).
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * shifted_month + 2) / 5 + 1) as u32;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    } as u32;
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

const MONTH_NAMES: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

const WEEKDAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

/// Splits Unix seconds into a civil date and the seconds into that day (UTC).
fn split_unix(unix: i64) -> ((i64, u32, u32), i64) {
    let days = unix.div_euclid(86_400);
    (civil_from_days(days), unix.rem_euclid(86_400))
}

/// `4:20 PM`, or `4:20:30 PM` with seconds (UTC).
pub(super) fn format_unix_time(unix: i64, seconds: bool) -> String {
    let (_, secs) = split_unix(unix);
    let (hour, minute, second) = (secs / 3600, secs / 60 % 60, secs % 60);
    let (hour12, meridiem) = match hour {
        0 => (12, "AM"),
        1..=11 => (hour, "AM"),
        12 => (12, "PM"),
        _ => (hour - 12, "PM"),
    };
    if seconds {
        format!("{hour12}:{minute:02}:{second:02} {meridiem}")
    } else {
        format!("{hour12}:{minute:02} {meridiem}")
    }
}

/// `09/26/2026` (UTC).
pub(super) fn format_unix_short_date(unix: i64) -> String {
    let ((year, month, day), _) = split_unix(unix);
    format!("{month:02}/{day:02}/{year}")
}

/// `September 26, 2026`, or `Saturday, September 26, 2026` with the weekday
/// (UTC).
pub(super) fn format_unix_date(unix: i64, weekday: bool) -> String {
    let ((year, month, day), _) = split_unix(unix);
    let date = format!("{} {day}, {year}", MONTH_NAMES[(month - 1) as usize]);
    if weekday {
        // 1970-01-01 was a Thursday.
        let index = (unix.div_euclid(86_400) + 4).rem_euclid(7) as usize;
        format!("{}, {date}", WEEKDAYS[index])
    } else {
        date
    }
}

/// `in 2 hours`, `3 days ago`: the largest whole unit between `unix` and
/// `now`, both Unix seconds.
pub(super) fn format_relative(unix: i64, now: i64) -> String {
    let delta = unix - now;
    let seconds = delta.unsigned_abs();
    let (amount, unit) = match seconds {
        0..60 => (seconds, "second"),
        60..3_600 => (seconds / 60, "minute"),
        3_600..86_400 => (seconds / 3_600, "hour"),
        86_400..2_592_000 => (seconds / 86_400, "day"),
        2_592_000..31_536_000 => (seconds / 2_592_000, "month"),
        _ => (seconds / 31_536_000, "year"),
    };
    let plural = if amount == 1 { "" } else { "s" };
    if delta >= 0 {
        format!("in {amount} {unit}{plural}")
    } else {
        format!("{amount} {unit}{plural} ago")
    }
}
