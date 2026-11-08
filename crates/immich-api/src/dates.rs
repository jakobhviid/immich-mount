//! Lenient parsing of the date formats Immich sends, and the one format we send.
//!
//! Dates are `chrono` types throughout the crate: `DateTime<Utc>` for instants, `NaiveDateTime` for wall-clock
//! times. Immich v3.3.5 mixes three spellings:
//! - RFC 3339 with `Z` or an offset: `2025-04-24T16:21:34.000Z`, `2026-11-01T21:02:04.237574+00:00`
//! - UTC **without** a zone, in the columnar timeline bucket: `2018-11-17T12:04:36`
//! - a wall-clock time with a **fake** `Z`: `localDateTime` is the local time the photo was taken, not UTC
//!
//! So instants accept all three (no zone means UTC), and wall-clock times drop whatever zone is attached.

use chrono::{DateTime, NaiveDateTime, SecondsFormat, Utc};

const NAIVE_FORMATS: [&str; 2] = ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%d %H:%M:%S%.f"];

/// An instant. A time without a zone is taken as UTC.
pub(crate) fn parse_utc(text: &str) -> Option<DateTime<Utc>> {
    let text = text.trim();
    if let Ok(with_offset) = DateTime::parse_from_rfc3339(text) {
        return Some(with_offset.with_timezone(&Utc));
    }
    parse_naive(text).map(|naive| naive.and_utc())
}

/// A wall-clock time. Any zone attached (Immich's fake `Z` on `localDateTime`) is dropped, not applied.
pub(crate) fn parse_wall_clock(text: &str) -> Option<NaiveDateTime> {
    let text = text.trim();
    if let Ok(with_offset) = DateTime::parse_from_rfc3339(text) {
        return Some(with_offset.naive_local());
    }
    parse_naive(text)
}

fn parse_naive(text: &str) -> Option<NaiveDateTime> {
    NAIVE_FORMATS
        .iter()
        .find_map(|format| NaiveDateTime::parse_from_str(text, format).ok())
}

/// The format we send: RFC 3339 in UTC with milliseconds and `Z`, as Immich's own clients do.
pub(crate) fn format_utc(instant: &DateTime<Utc>) -> String {
    instant.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// Parse an optional instant from a wire field.
pub(crate) fn optional_utc(text: Option<&str>) -> Option<DateTime<Utc>> {
    text.and_then(parse_utc)
}

/// Serde helpers for model fields, so the lenient parsers above apply wherever a model holds a date.
///
/// Models decode with a struct-level `#[serde(default)]`, so a missing or unreadable date falls back to the
/// field's default (the Unix epoch for an instant, `None` for an optional one) instead of failing the answer.
pub(crate) mod serde_dates {
    use chrono::{DateTime, NaiveDateTime, Utc};
    use serde::{Deserialize, Deserializer};

    /// An instant; an unreadable one decodes as the Unix epoch.
    pub(crate) fn utc<'de, D: Deserializer<'de>>(deserializer: D) -> Result<DateTime<Utc>, D::Error> {
        let text = Option::<String>::deserialize(deserializer)?;
        Ok(text.as_deref().and_then(super::parse_utc).unwrap_or_default())
    }

    /// An optional instant; an unreadable one decodes as `None`.
    pub(crate) fn optional_utc<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<DateTime<Utc>>, D::Error> {
        let text = Option::<String>::deserialize(deserializer)?;
        Ok(text.as_deref().and_then(super::parse_utc))
    }

    /// An optional wall-clock time (Immich's `localDateTime`, with its fake `Z` dropped).
    pub(crate) fn optional_wall_clock<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<NaiveDateTime>, D::Error> {
        let text = Option::<String>::deserialize(deserializer)?;
        Ok(text.as_deref().and_then(super::parse_wall_clock))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn wall(year: i32, month: u32, day: u32, hour: u32, minute: u32, second: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(year, month, day)
            .unwrap()
            .and_hms_opt(hour, minute, second)
            .unwrap()
    }

    #[test]
    fn instants_with_z_offset_or_no_zone() {
        let expected = wall(2018, 11, 17, 12, 4, 36).and_utc();
        assert_eq!(parse_utc("2018-11-17T12:04:36.000Z"), Some(expected));
        assert_eq!(parse_utc("2018-11-17T13:04:36+01:00"), Some(expected));
        // The columnar bucket sends UTC without a zone.
        assert_eq!(parse_utc("2018-11-17T12:04:36"), Some(expected));
        assert_eq!(parse_utc("2018-11-17 12:04:36"), Some(expected));
    }

    #[test]
    fn fractional_seconds_survive() {
        let parsed = parse_utc("2026-11-01T21:02:04.237574+00:00").unwrap();
        assert_eq!(parsed.timestamp_subsec_micros(), 237_574);
    }

    #[test]
    fn wall_clock_drops_the_fake_z() {
        // A photo taken 2020-03-01 00:30 at +02:00: localDateTime arrives as 00:30 with a Z.
        assert_eq!(
            parse_wall_clock("2020-03-01T00:30:00.000Z"),
            Some(wall(2020, 3, 1, 0, 30, 0))
        );
        assert_eq!(
            parse_wall_clock("2020-03-01T00:30:00+02:00"),
            Some(wall(2020, 3, 1, 0, 30, 0))
        );
    }

    #[test]
    fn garbage_is_none() {
        assert_eq!(parse_utc("yesterday"), None);
        assert_eq!(parse_wall_clock(""), None);
    }

    #[test]
    fn we_send_millis_and_z() {
        let instant = wall(2020, 2, 29, 22, 30, 0).and_utc();
        assert_eq!(format_utc(&instant), "2020-02-29T22:30:00.000Z");
    }
}
