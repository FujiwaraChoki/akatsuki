//! Date resolution for the query grammar (ARCHITECTURE §11.1). Dates resolve in the user's IANA
//! time zone (`profiles.timezone`); the time-zone database is embedded so this also works in the
//! distroless worker image.

use jiff::Timestamp;
use jiff::civil::Date;
use jiff::tz::TimeZone;

fn zone(tz: &str) -> Result<TimeZone, String> {
    TimeZone::get(tz).map_err(|_| format!("unknown time zone: {tz}"))
}

/// Parses `YYYY-MM-DD` or `YYYY/MM/DD`.
pub fn parse_day(value: &str) -> Option<Date> {
    let sep = if value.contains('-') { '-' } else { '/' };
    let mut parts = value.split(sep);
    let (y, m, d) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() || y.len() != 4 || m.is_empty() || m.len() > 2 || d.is_empty() || d.len() > 2 {
        return None;
    }
    if !(y.bytes().chain(m.bytes()).chain(d.bytes()).all(|b| b.is_ascii_digit())) {
        return None;
    }
    Date::new(y.parse().ok()?, m.parse().ok()?, d.parse().ok()?).ok()
}

/// The start of the day `value` in `tz`, as Unix milliseconds.
pub fn start_of_day_ms(value: &str, tz: &str) -> Result<i64, String> {
    let day = parse_day(value).ok_or_else(|| format!("invalid date: {value}"))?;
    let zone = zone(tz)?;
    let zoned = day.to_zoned(zone).map_err(|e| format!("invalid date: {value} ({e})"))?;
    Ok(zoned.timestamp().as_millisecond())
}

/// Renders an instant as `YYYY-MM-DD` in `tz` (UTC if the zone is unknown).
pub fn format_day(at_ms: i64, tz: &str) -> String {
    let zone = zone(tz).unwrap_or(TimeZone::UTC);
    match Timestamp::from_millisecond(at_ms) {
        Ok(ts) => ts.to_zoned(zone).date().strftime("%Y-%m-%d").to_string(),
        Err(_) => at_ms.to_string(),
    }
}

/// `N(d|w|m|y)` → days (`m` = 30, `y` = 365). Case-insensitive unit.
pub fn relative_days(value: &str) -> Option<u32> {
    let value = value.trim();
    let unit = value.chars().last()?;
    // Slice at the char boundary: the unit may be any (multi-byte) character.
    let n: u32 = value[..value.len() - unit.len_utf8()].parse().ok()?;
    let unit = unit.to_ascii_lowercase();
    let mult = match unit {
        'd' => 1,
        'w' => 7,
        'm' => 30,
        'y' => 365,
        _ => return None,
    };
    n.checked_mul(mult)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days() {
        assert_eq!(start_of_day_ms("2026-10-01", "UTC"), Ok(1_790_812_800_000));
        assert_eq!(start_of_day_ms("2026/10/01", "America/New_York"), Ok(1_790_827_200_000));
        assert!(start_of_day_ms("2026-02-30", "UTC").is_err());
        assert!(start_of_day_ms("26-02-01", "UTC").is_err());
        assert!(start_of_day_ms("2026-02-01", "Mars/Olympus").is_err());
        assert_eq!(format_day(1_790_805_600_000, "Europe/Zurich"), "2026-10-01");
        assert_eq!(format_day(1_790_805_600_000, "UTC"), "2026-09-30");
    }

    #[test]
    fn relative() {
        assert_eq!(relative_days("3d"), Some(3));
        assert_eq!(relative_days("2W"), Some(14));
        assert_eq!(relative_days("1m"), Some(30));
        assert_eq!(relative_days("x"), None);
        assert_eq!(relative_days("d"), None);
        assert_eq!(relative_days("-1d"), None);
        assert_eq!(relative_days("3é"), None, "a multi-byte unit must not panic");
        assert_eq!(relative_days("é"), None);
    }
}
