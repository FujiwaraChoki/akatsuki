//! Typed times for snooze, reminders and send later, resolved in the user's zone (§1.3; the Rust
//! twin of `parseSnoozeInput` in `apps/web/src/mail/snoozeTimes.ts`, D-297): "3h", "2d", "in 2
//! months", "tonight", "eod", "tomorrow 9am", "fri 3pm", "next fri", "nov 3 at 2pm",
//! "2026-11-02 9:30", "17:30". Only the resulting ms instant goes to the server.

use jiff::civil::{Date, DateTime, Weekday};
use jiff::tz::TimeZone;
use jiff::{Timestamp, ToSpan, Zoned};

/// The morning hour presets use.
pub const MORNING_HOUR: i8 = 8;

const WEEKDAYS: [&str; 7] = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];
const MONTHS: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];

/// Sunday = 0, like the web.
fn weekday_index(w: Weekday) -> i64 {
    i64::from(w.to_sunday_zero_offset())
}

fn at(date: Date, h: i8, min: i8, tz: &TimeZone) -> Option<i64> {
    let dt = DateTime::new(date.year(), date.month(), date.day(), h, min, 0, 0).ok()?;
    dt.to_zoned(tz.clone()).ok().map(|z| z.timestamp().as_millisecond())
}

fn add_days(date: Date, days: i64) -> Option<Date> {
    date.checked_add(days.days()).ok()
}

/// `9`, `9am`, `9:30`, `21:05`, `12pm`.
fn parse_clock(s: &str) -> Option<(i8, i8)> {
    let s = s.trim();
    let (digits, meridiem) = if let Some(d) = s.strip_suffix("am") {
        (d.trim(), Some(false))
    } else if let Some(d) = s.strip_suffix("pm") {
        (d.trim(), Some(true))
    } else {
        (s, None)
    };
    let (h, min) = match digits.split_once(':') {
        Some((h, m)) => (h, m),
        None => (digits, "0"),
    };
    if h.is_empty() || h.len() > 2 || !h.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    if digits.contains(':') && (min.len() != 2 || !min.chars().all(|c| c.is_ascii_digit())) {
        return None;
    }
    let mut h: i8 = h.parse().ok()?;
    let min: i8 = min.parse().ok()?;
    if min > 59 {
        return None;
    }
    match meridiem {
        Some(pm) => {
            if !(1..=12).contains(&h) {
                return None;
            }
            if pm && h != 12 {
                h += 12;
            }
            if !pm && h == 12 {
                h = 0;
            }
        }
        None if h > 23 => return None,
        None => {}
    }
    Some((h, min))
}

fn month_index(word: &str) -> Option<i8> {
    let w = word.trim_end_matches('.');
    if w.len() < 3 || w.len() > 9 || !w.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    MONTHS.iter().position(|m| w.starts_with(m)).and_then(|i| i8::try_from(i + 1).ok())
}

fn day_number(word: &str) -> Option<i8> {
    let w = word.trim_end_matches(',');
    let w = ["st", "nd", "rd", "th"].iter().find_map(|s| w.strip_suffix(s)).unwrap_or(w);
    if w.is_empty() || w.len() > 2 || !w.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    w.parse().ok().filter(|d| (1..=31).contains(d))
}

/// "nov 3", "3 nov", "3rd of december at 2pm", "nov 3, 2027": the date and the rest (a time).
fn parse_month_date(words: &[&str], today: Date) -> Option<(Date, Option<String>)> {
    let first_month = words.first().and_then(|w| month_index(w));
    let second_day = words.get(1).and_then(|w| day_number(w));
    let (month, day, mut rest) = if let (Some(m), Some(d)) = (first_month, second_day) {
        (m, d, &words[2..])
    } else {
        let d = words.first().and_then(|w| day_number(w))?;
        let skip = usize::from(words.get(1) == Some(&"of"));
        let m = words.get(1 + skip).and_then(|w| month_index(w))?;
        (m, d, &words[2 + skip..])
    };
    let mut year = None;
    if let Some(y) = rest.first().filter(|y| y.len() == 4 && y.chars().all(|c| c.is_ascii_digit())) {
        year = y.parse::<i16>().ok();
        rest = &rest[1..];
    }
    let rest: Vec<&str> = rest.iter().copied().skip_while(|w| *w == "at").collect();
    let time = (!rest.is_empty()).then(|| rest.join(" "));
    let y = match year {
        Some(y) => y,
        None if (month, day) < (today.month(), today.day()) => today.year() + 1,
        None => today.year(),
    };
    Date::new(y, month, day).ok().map(|d| (d, time))
}

/// Parses a typed time. `None` when it is not understood or not in the future.
pub fn parse(input: &str, now_ms: i64, tz: &TimeZone) -> Option<i64> {
    let mut s = input.trim().to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ");
    for prefix in ["on ", "at ", "until ", "till ", "by "] {
        if let Some(rest) = s.strip_prefix(prefix) {
            s = rest.to_owned();
            break;
        }
    }
    for unit in ["minute", "hour", "day", "week", "month"] {
        for article in ["a", "an", "one"] {
            s = s.replace(&format!("{article} {unit}"), &format!("1 {unit}"));
        }
    }
    if s.is_empty() {
        return None;
    }
    let now: Zoned = Timestamp::from_millisecond(now_ms).ok()?.to_zoned(tz.clone());
    let today = now.date();
    let future = |t: Option<i64>| t.filter(|t| *t > now_ms);
    let next_at = |h: i8, min: i8| {
        let t = at(today, h, min, tz)?;
        if t > now_ms { Some(t) } else { at(add_days(today, 1)?, h, min, tz) }
    };

    // Relative: "3h", "in 2 days", "45 min", "2 months".
    let rel = s.strip_prefix("in ").unwrap_or(&s);
    let digits: String = rel.chars().take_while(char::is_ascii_digit).collect();
    if !digits.is_empty() && digits.len() <= 3 {
        let unit = rel[digits.len()..].trim();
        let n: i64 = digits.parse().ok()?;
        let ms = match unit {
            "m" | "min" | "mins" | "minute" | "minutes" => Some(60_000),
            "h" | "hr" | "hrs" | "hour" | "hours" => Some(3_600_000),
            "d" | "day" | "days" => Some(86_400_000),
            "w" | "week" | "weeks" => Some(7 * 86_400_000),
            "month" | "months" => None,
            _ => Some(-1),
        };
        match ms {
            Some(-1) => {}
            Some(ms) => return (n > 0).then(|| now_ms + n * ms),
            None => {
                let target = now.date().first_of_month().checked_add(n.months()).ok()?;
                let day = now.day().min(28);
                let date = Date::new(target.year(), target.month(), day).ok()?;
                return if n > 0 { at(date, now.hour(), now.minute(), tz) } else { None };
            }
        }
    }

    let wd = weekday_index(now.weekday());
    let named = match s.as_str() {
        "tonight" => Some(future(at(today, 19, 0, tz))),
        "this evening" | "evening" => Some(next_at(19, 0)),
        "end of day" | "eod" => Some(next_at(17, 0)),
        "this afternoon" | "afternoon" => Some(next_at(13, 0)),
        "noon" => Some(next_at(12, 0)),
        "morning" => Some(next_at(MORNING_HOUR, 0)),
        "tomorrow morning" => Some(at(add_days(today, 1)?, MORNING_HOUR, 0, tz)),
        "tomorrow afternoon" => Some(at(add_days(today, 1)?, 13, 0, tz)),
        "tomorrow evening" => Some(at(add_days(today, 1)?, 19, 0, tz)),
        "this weekend" => Some(future(at(add_days(today, (6 - wd + 7) % 7)?, MORNING_HOUR, 0, tz))),
        "weekend" => {
            let d = (6 - wd + 7) % 7;
            Some(future(at(add_days(today, if d == 0 { 7 } else { d })?, MORNING_HOUR, 0, tz)))
        }
        "next week" => {
            let d = (8 - wd) % 7;
            Some(at(add_days(today, if d == 0 { 7 } else { d })?, MORNING_HOUR, 0, tz))
        }
        "next month" => Some(at(today.first_of_month().checked_add(1.month()).ok()?, MORNING_HOUR, 0, tz)),
        _ => None,
    };
    if let Some(t) = named {
        return t;
    }

    let words: Vec<&str> = s.split(' ').collect();
    let clock_of = |rest: &[&str]| -> Option<(i8, i8)> {
        let rest: Vec<&str> = rest.iter().copied().skip_while(|w| *w == "at").collect();
        if rest.is_empty() { Some((MORNING_HOUR, 0)) } else { parse_clock(&rest.join(" ")) }
    };
    let weekday_of = |w: &str| -> Option<i64> {
        WEEKDAYS.iter().position(|d| w.starts_with(d) && w.chars().all(|c| c.is_ascii_alphabetic())).map(|i| i as i64)
    };

    if words[0] == "next"
        && let Some(target) = words.get(1).and_then(|w| weekday_of(w))
    {
        let (h, min) = clock_of(&words[2..])?;
        let base = (target - wd + 7) % 7;
        let days = (if base == 0 { 7 } else { base }) + if target > wd { 7 } else { 0 };
        return at(add_days(today, days)?, h, min, tz);
    }
    if let Some((date, time)) = parse_month_date(&words, today) {
        let (h, min) = match time {
            Some(t) => parse_clock(&t)?,
            None => (MORNING_HOUR, 0),
        };
        return future(at(date, h, min, tz));
    }
    let first = words[0];
    let date = if first == "today" {
        Some(today)
    } else if first == "tomorrow" || first == "tmrw" {
        add_days(today, 1)
    } else if first.len() == 10 && first.as_bytes().get(4) == Some(&b'-') {
        let mut parts = first.split('-').map(str::parse::<i16>);
        match (parts.next(), parts.next(), parts.next()) {
            (Some(Ok(y)), Some(Ok(m)), Some(Ok(d))) => {
                Some(Date::new(y, i8::try_from(m).ok()?, i8::try_from(d).ok()?).ok()?)
            }
            _ => return None,
        }
    } else if let Some(target) = weekday_of(first) {
        let d = (target - wd + 7) % 7;
        add_days(today, if d == 0 { 7 } else { d })
    } else {
        None
    };
    match date {
        Some(date) => {
            let (h, min) = clock_of(&words[1..])?;
            future(at(date, h, min, tz))
        }
        None => {
            let (h, min) = parse_clock(&s)?;
            next_at(h, min)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(s: &str) -> i64 {
        s.parse::<Timestamp>().unwrap().as_millisecond()
    }

    #[test]
    fn typed_times_resolve_in_the_zone() {
        let tz = TimeZone::get("Europe/Zurich").unwrap();
        // Tuesday 2026-10-20 10:20 in Zurich (UTC+2 until the 25th).
        let now = ms("2026-10-20T08:20:00Z");
        let p = |s: &str| parse(s, now, &tz);
        assert_eq!(p("3h"), Some(now + 3 * 3_600_000));
        assert_eq!(p("in 45 min"), Some(now + 45 * 60_000));
        assert_eq!(p("an hour"), Some(now + 3_600_000));
        assert_eq!(p("2d"), Some(now + 2 * 86_400_000));
        assert_eq!(p("0h"), None);
        assert_eq!(p("tonight"), Some(ms("2026-10-20T17:00:00Z")));
        assert_eq!(p("eod"), Some(ms("2026-10-20T15:00:00Z")));
        assert_eq!(p("tomorrow"), Some(ms("2026-10-21T06:00:00Z")), "08:00 by default");
        assert_eq!(p("Tomorrow at 9:30am"), Some(ms("2026-10-21T07:30:00Z")));
        assert_eq!(p("fri 3pm"), Some(ms("2026-10-23T13:00:00Z")));
        assert_eq!(p("tue"), Some(ms("2026-10-27T07:00:00Z")), "the same weekday means next week; after the DST end");
        assert_eq!(p("next fri"), Some(ms("2026-10-30T07:00:00Z")), "next Friday is the one after this one");
        assert_eq!(p("next mon"), Some(ms("2026-10-26T07:00:00Z")));
        assert_eq!(p("next week"), Some(ms("2026-10-26T07:00:00Z")));
        assert_eq!(p("this weekend"), Some(ms("2026-10-24T06:00:00Z")));
        assert_eq!(p("nov 3 at 2pm"), Some(ms("2026-11-03T13:00:00Z")));
        assert_eq!(p("3rd of december"), Some(ms("2026-12-03T07:00:00Z")));
        assert_eq!(p("oct 1"), Some(ms("2027-10-01T06:00:00Z")), "a past date means next year");
        assert_eq!(p("2026-11-02 9:30"), Some(ms("2026-11-02T08:30:00Z")));
        assert_eq!(p("2025-11-02"), None, "the past is refused");
        assert_eq!(p("17:30"), Some(ms("2026-10-20T15:30:00Z")));
        assert_eq!(p("9"), Some(ms("2026-10-21T07:00:00Z")), "a passed hour means tomorrow");
        assert_eq!(p("in 2 months"), Some(ms("2026-12-20T09:20:00Z")));
        assert_eq!(p("until noon"), Some(ms("2026-10-20T10:00:00Z")));
        assert_eq!(p("someday"), None);
        assert_eq!(p("25:00"), None);
        assert_eq!(p("13pm"), None);
        assert_eq!(p(""), None);
    }
}
