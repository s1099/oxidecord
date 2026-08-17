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

/// Formats the creation time encoded in a snowflake as `Jan 5, 2021` (UTC),
/// the form Discord uses for "Member Since".
pub(super) fn format_snowflake_date(id: u64) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];

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
