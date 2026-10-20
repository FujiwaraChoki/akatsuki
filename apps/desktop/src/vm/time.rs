//! Dates as the user sees them, in their time zone (§1.3: user-facing date logic resolves on the
//! client; only ms instants go to the server).

use jiff::civil::{Time, Weekday};
use jiff::tz::TimeZone;
use jiff::{Timestamp, ToSpan, Zoned};

/// The system time zone (UTC when it cannot be read).
pub fn local_tz() -> TimeZone {
    TimeZone::try_system().unwrap_or(TimeZone::UTC)
}

/// The IANA name of a zone (for the search grammar), `UTC` when unnamed.
pub fn tz_name(tz: &TimeZone) -> String {
    tz.iana_name().unwrap_or("UTC").to_owned()
}

fn zoned(ms: i64, tz: &TimeZone) -> Option<Zoned> {
    Timestamp::from_millisecond(ms).ok().map(|t| t.to_zoned(tz.clone()))
}

/// List-row time: `14:05` today, `Oct 3` this year, `Oct 3, 2025` before.
pub fn list_time(ms: i64, now_ms: i64, tz: &TimeZone) -> String {
    let (Some(t), Some(now)) = (zoned(ms, tz), zoned(now_ms, tz)) else { return String::new() };
    if t.date() == now.date() {
        t.strftime("%H:%M").to_string()
    } else if t.year() == now.year() {
        t.strftime("%b %-d").to_string()
    } else {
        t.strftime("%b %-d, %Y").to_string()
    }
}

/// Full date for message headers: `Tue, Oct 3, 2026 at 14:05`.
pub fn full_time(ms: i64, tz: &TimeZone) -> String {
    zoned(ms, tz).map(|t| t.strftime("%a, %b %-d, %Y at %H:%M").to_string()).unwrap_or_default()
}

/// A typed time in pickers: `Fri 23 Oct 15:00`.
pub fn short_time(ms: i64, tz: &TimeZone) -> String {
    zoned(ms, tz).map(|t| t.strftime("%a %-d %b %H:%M").to_string()).unwrap_or_default()
}

/// A snooze or reminder choice.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preset {
    /// Label.
    pub label: &'static str,
    /// When.
    pub at: i64,
    /// Shown next to the label (`Tue 08:00`).
    pub hint: String,
}

fn at_eight(z: &Zoned) -> Option<Zoned> {
    z.with().time(Time::constant(8, 0, 0, 0)).build().ok()
}

/// Superhuman-style presets: later today (in 3 hours, before 18:00 only), tomorrow 08:00, this
/// weekend (Saturday 08:00, not on weekends), next week (Monday 08:00).
pub fn presets(now_ms: i64, tz: &TimeZone) -> Vec<Preset> {
    let Some(now) = zoned(now_ms, tz) else { return Vec::new() };
    let mut out = Vec::new();
    let push = |out: &mut Vec<Preset>, label, z: Option<Zoned>| {
        if let Some(z) = z {
            out.push(Preset { label, at: z.timestamp().as_millisecond(), hint: z.strftime("%a %H:%M").to_string() });
        }
    };
    if now.hour() < 18 {
        let later = now
            .checked_add(3.hours())
            .ok()
            .and_then(|z| z.with().minute(0).second(0).subsec_nanosecond(0).build().ok());
        push(&mut out, "Later today", later);
    }
    let tomorrow = now.checked_add(1.day()).ok().and_then(|z| at_eight(&z));
    push(&mut out, "Tomorrow", tomorrow);
    let wd = now.weekday();
    if !matches!(wd, Weekday::Saturday | Weekday::Sunday) {
        let days = i64::from(Weekday::Saturday.to_monday_one_offset() - wd.to_monday_one_offset());
        push(&mut out, "This weekend", now.checked_add(days.days()).ok().and_then(|z| at_eight(&z)));
    }
    let to_monday = 8 - i64::from(wd.to_monday_one_offset());
    push(&mut out, "Next week", now.checked_add(to_monday.days()).ok().and_then(|z| at_eight(&z)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(s: &str) -> i64 {
        s.parse::<Timestamp>().unwrap().as_millisecond()
    }

    #[test]
    fn list_times_depend_on_distance() {
        let tz = TimeZone::get("Europe/Zurich").unwrap();
        let now = ms("2026-10-20T12:00:00Z");
        assert_eq!(list_time(ms("2026-10-20T07:05:00Z"), now, &tz), "09:05");
        assert_eq!(list_time(ms("2026-10-03T07:05:00Z"), now, &tz), "Oct 3");
        assert_eq!(list_time(ms("2025-10-03T07:05:00Z"), now, &tz), "Oct 3, 2025");
        assert_eq!(full_time(ms("2026-10-20T07:05:00Z"), &tz), "Tue, Oct 20, 2026 at 09:05");
        assert_eq!(short_time(ms("2026-10-23T13:00:00Z"), &tz), "Fri 23 Oct 15:00");
        assert_eq!(tz_name(&tz), "Europe/Zurich");
    }

    #[test]
    fn presets_on_a_tuesday_morning() {
        let tz = TimeZone::get("Europe/Zurich").unwrap();
        // Tuesday 2026-10-20 10:20 in Zurich.
        let p = presets(ms("2026-10-20T08:20:00Z"), &tz);
        let labels: Vec<_> = p.iter().map(|x| x.label).collect();
        assert_eq!(labels, vec!["Later today", "Tomorrow", "This weekend", "Next week"]);
        assert_eq!(p[0].at, ms("2026-10-20T11:00:00Z"), "13:00 local");
        assert_eq!(p[1].at, ms("2026-10-21T06:00:00Z"), "Wednesday 08:00 local");
        assert_eq!(p[2].at, ms("2026-10-24T06:00:00Z"), "Saturday 08:00 local");
        assert_eq!(p[3].at, ms("2026-10-26T07:00:00Z"), "Monday 08:00 local, after the DST change");
        assert_eq!(p[3].hint, "Mon 08:00");
    }

    #[test]
    fn presets_on_a_saturday_evening() {
        let tz = TimeZone::UTC;
        let p = presets(ms("2026-10-24T19:00:00Z"), &tz);
        let labels: Vec<_> = p.iter().map(|x| x.label).collect();
        assert_eq!(labels, vec!["Tomorrow", "Next week"]);
        assert_eq!(p[1].at, ms("2026-10-26T08:00:00Z"));
    }
}
