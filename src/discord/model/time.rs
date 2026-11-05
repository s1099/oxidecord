//! Date formatting for the timestamps the UI shows. Message times and `<t:…>`
//! markup are local, as Discord shows them, which takes chrono for the time
//! zone; the rest is UTC and done by hand.

use chrono::{DateTime, Local, TimeZone};
use twilight_model::util::Timestamp;

use super::markdown::TimestampStyle;

/// Milliseconds from the Unix epoch to Discord's (2015-01-01).
const DISCORD_EPOCH_MS: u64 = 1_420_070_400_000;

/// A message's send time the way Discord's header shows it, in local time:
/// `11:02 AM` for one sent today, `9/26/26, 11:02 AM` for anything older.
pub(super) fn format_message_time(unix: i64) -> String {
    let Some(sent) = local(unix) else {
        return String::new();
    };
    if sent.date_naive() == Local::now().date_naive() {
        sent.format("%-I:%M %p").to_string()
    } else {
        sent.format("%-m/%-d/%y, %-I:%M %p").to_string()
    }
}

/// `11:02 AM` whatever the date, as shown beside a continuation message.
pub(super) fn format_message_clock(unix: i64) -> String {
    local(unix).map_or_else(String::new, |time| time.format("%-I:%M %p").to_string())
}

/// The local calendar day, to tell where a new day starts in a conversation.
pub(super) fn local_day(unix: i64) -> Option<chrono::NaiveDate> {
    local(unix).map(|time| time.date_naive())
}

/// `October 27, 2026`, in local time, as the divider between days reads.
pub(super) fn format_local_day(unix: i64) -> String {
    local(unix).map_or_else(String::new, |time| time.format("%B %-d, %Y").to_string())
}

/// `Saturday, September 26, 2026 4:20 PM`, in local time.
pub(super) fn format_local_full(unix: i64) -> String {
    local(unix).map_or_else(String::new, |time| {
        time.format("%A, %B %-d, %Y %-I:%M %p").to_string()
    })
}

fn local(unix: i64) -> Option<DateTime<Local>> {
    DateTime::from_timestamp(unix, 0).map(|utc| utc.with_timezone(&Local))
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// `Jan 5, 2021 4:56 PM` (UTC).
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

/// A snowflake's creation date as `Jan 5, 2021` (UTC), for "Member Since".
pub(super) fn format_snowflake_date(id: u64) -> String {
    let unix_ms = (id >> 22) + DISCORD_EPOCH_MS;
    let (year, month, day) = civil_from_days((unix_ms / 86_400_000) as i64);
    format!("{} {day}, {year}", MONTHS[(month - 1) as usize])
}

/// Howard Hinnant's `civil_from_days`: days since the Unix epoch to a civil date.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    // Eras start on 0000-03-01 so the leap day ends the year.
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

/// `<t:…>` markup in the reader's time zone, in Discord's en-US wording.
pub(super) fn format_markup_timestamp(unix: i64, style: TimestampStyle) -> String {
    local(unix).map_or_else(|| unix.to_string(), |time| format_stamp(&time, style))
}

fn format_stamp<Tz: TimeZone>(time: &DateTime<Tz>, style: TimestampStyle) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let pattern = match style {
        TimestampStyle::ShortTime => "%-I:%M %p",
        TimestampStyle::LongTime => "%-I:%M:%S %p",
        TimestampStyle::ShortDate => "%-m/%-d/%Y",
        TimestampStyle::LongDate => "%B %-d, %Y",
        TimestampStyle::ShortDateTime => "%B %-d, %Y at %-I:%M %p",
        TimestampStyle::LongDateTime | TimestampStyle::Relative => "%A, %B %-d, %Y at %-I:%M %p",
        TimestampStyle::ShortDateShortTime => "%-m/%-d/%Y, %-I:%M %p",
        TimestampStyle::ShortDateLongTime => "%-m/%-d/%Y, %-I:%M:%S %p",
    };
    time.format(pattern).to_string()
}

/// `in 2 hours`, `3 days ago`; both arguments are Unix seconds.
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

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;

    #[test]
    fn markup_styles() {
        // Tuesday, April 20, 2021 16:20:30 UTC.
        let time = Utc.timestamp_opt(1_618_935_630, 0).unwrap();
        let cases = [
            (TimestampStyle::ShortTime, "4:20 PM"),
            (TimestampStyle::LongTime, "4:20:30 PM"),
            (TimestampStyle::ShortDate, "4/20/2021"),
            (TimestampStyle::LongDate, "April 20, 2021"),
            (TimestampStyle::ShortDateTime, "April 20, 2021 at 4:20 PM"),
            (
                TimestampStyle::LongDateTime,
                "Tuesday, April 20, 2021 at 4:20 PM",
            ),
            (TimestampStyle::ShortDateShortTime, "4/20/2021, 4:20 PM"),
            (TimestampStyle::ShortDateLongTime, "4/20/2021, 4:20:30 PM"),
        ];
        for (style, expected) in cases {
            assert_eq!(format_stamp(&time, style), expected);
        }
    }
}
