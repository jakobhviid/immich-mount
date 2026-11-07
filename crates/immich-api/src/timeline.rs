//! Timeline buckets: how many Assets each month holds, and which ids one month holds.
//!
//! Buckets group by the Asset's **local wall-clock** time (`localDateTime`), not by UTC, so a photo taken
//! 2020-03-01 00:30 at +02:00 is in the March bucket although its UTC time is in February (seen on
//! v3.3.5). A UTC date-range search therefore does not reproduce a bucket; listing a month goes
//! through [`Client::time_bucket`] (ids) and then search or [`Client::asset`] for details.
//!
//! Visibility: the bucket endpoints include archived Assets by default. The library and person timelines send
//! `visibility=timeline`; an album sends nothing, because an album shows its archived members too; the archive sends
//! `visibility=archive`, as Immich's own Archive page does; favorites send `isFavorite=true` and no visibility, as
//! Immich's Favorites page does, so archived favorites are among them.
//!
//! `GET /timeline/bucket` answers in **columns** (`{"id":[...],"fileCreatedAt":[...],...}`), with
//! `fileCreatedAt` in UTC without a `Z`. A column the server leaves out reads as its default; a column of the
//! wrong length is a protocol error.

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveDateTime, Utc};
use serde::Deserialize;

use crate::asset::Visibility;
use crate::client::{Client, Fetched};
use crate::dates::parse_utc;
use crate::error::ApiError;

/// Which timeline to read.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TimelineScope {
    /// The whole library: timeline visibility only (no archived, no trashed).
    Library,
    /// One album, by id, archived members included.
    Album(String),
    /// One person, by id: timeline visibility only.
    Person(String),
    /// The archive: archived visibility only.
    Archive,
    /// The favorites, archived ones included.
    Favorites,
}

impl TimelineScope {
    fn query(&self) -> Vec<(&'static str, &str)> {
        match self {
            TimelineScope::Library => vec![("visibility", "timeline")],
            TimelineScope::Album(album_id) => vec![("albumId", album_id.as_str())],
            TimelineScope::Person(person_id) => vec![("personId", person_id.as_str()), ("visibility", "timeline")],
            TimelineScope::Archive => vec![("visibility", "archive")],
            TimelineScope::Favorites => vec![("isFavorite", "true")],
        }
    }
}

/// A calendar month, the unit of a timeline bucket.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct YearMonth {
    pub year: i32,
    /// 1 to 12.
    pub month: u32,
}

impl YearMonth {
    /// A month, if `month` is 1 to 12 and the year is one chrono can represent.
    pub fn new(year: i32, month: u32) -> Option<YearMonth> {
        NaiveDate::from_ymd_opt(year, month, 1).map(|_| YearMonth { year, month })
    }

    /// Parse `"2025-04-01"` (a bucket key), `"2025-04-01T00:00:00.000Z"` or `"2025-04"`.
    pub fn parse(text: &str) -> Option<YearMonth> {
        let text = text.trim();
        let mut parts = text.splitn(3, '-');
        let year = parts.next()?.parse().ok()?;
        let month_text = parts.next()?;
        if month_text.len() != 2 {
            return None;
        }
        let month = month_text.parse().ok()?;
        if let Some(rest) = parts.next() {
            // The rest must at least start with a valid day, so "2025-04-xx" is refused.
            let day: u32 = rest.get(..2)?.parse().ok()?;
            NaiveDate::from_ymd_opt(year, month, day)?;
        }
        YearMonth::new(year, month)
    }

    /// The key the bucket endpoints use: the first day, `"2025-04-01"`.
    pub fn bucket_key(&self) -> String {
        format!("{:04}-{:02}-01", self.year, self.month)
    }

    /// The first day of the month.
    pub fn first_day(&self) -> NaiveDate {
        // `new` and `parse` only build valid months.
        NaiveDate::from_ymd_opt(self.year, self.month, 1).unwrap_or_default()
    }

    /// The month after this one.
    pub fn next(&self) -> YearMonth {
        if self.month == 12 {
            YearMonth { year: self.year + 1, month: 1 }
        } else {
            YearMonth { year: self.year, month: self.month + 1 }
        }
    }

    /// The month before this one.
    pub fn previous(&self) -> YearMonth {
        if self.month == 1 {
            YearMonth { year: self.year - 1, month: 12 }
        } else {
            YearMonth { year: self.year, month: self.month - 1 }
        }
    }

    /// The month a wall-clock time falls in.
    pub fn of(wall_clock: &NaiveDateTime) -> YearMonth {
        YearMonth {
            year: wall_clock.year(),
            month: wall_clock.month(),
        }
    }

    /// The UTC instants that can hold this month's wall-clock times: the month read as UTC, widened by
    /// `margin` on each side. With a 14 hour margin it covers every time zone; filter the results by
    /// `local_date_time` afterwards (ARCHITECTURE.md, "Names and sizes without a session").
    pub fn utc_range_widened(&self, margin: Duration) -> std::ops::Range<DateTime<Utc>> {
        let start = self.first_day().and_time(chrono::NaiveTime::MIN).and_utc();
        let end = self.next().first_day().and_time(chrono::NaiveTime::MIN).and_utc();
        (start - margin)..(end + margin)
    }
}

impl fmt::Display for YearMonth {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:04}-{:02}", self.year, self.month)
    }
}

impl FromStr for YearMonth {
    type Err = ApiError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        YearMonth::parse(text).ok_or_else(|| ApiError::Protocol(format!("not a month: `{text}`")))
    }
}

/// How many Assets one month holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeBucket {
    pub month: YearMonth,
    pub count: u32,
}

/// One Asset in a bucket: ids and dates only; no names and no sizes.
#[derive(Debug, Clone, PartialEq)]
pub struct BucketAsset {
    pub id: String,
    /// When it was taken, in UTC.
    pub file_created_at: DateTime<Utc>,
    /// The offset of the wall clock where it was taken, in hours (can be fractional, such as 5.5).
    pub local_offset_hours: f64,
    pub is_trashed: bool,
    pub visibility: Visibility,
    pub is_image: bool,
}

impl BucketAsset {
    /// The wall-clock time it was taken: the UTC time plus the local offset. Buckets group by this.
    pub fn local_date_time(&self) -> NaiveDateTime {
        let offset_seconds = (self.local_offset_hours * 3600.0).round() as i64;
        self.file_created_at.naive_utc() + Duration::seconds(offset_seconds)
    }
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct TimeBucketWire {
    time_bucket: Option<String>,
    count: u32,
}

fn into_time_buckets(wire: Vec<TimeBucketWire>) -> Result<Vec<TimeBucket>, ApiError> {
    wire.into_iter()
        .map(|bucket| {
            let key = bucket.time_bucket.unwrap_or_default();
            let month = YearMonth::parse(&key)
                .ok_or_else(|| ApiError::Protocol(format!("a timeline bucket with an unreadable month `{key}`")))?;
            Ok(TimeBucket {
                month,
                count: bucket.count,
            })
        })
        .collect()
}

/// The columnar answer of `GET /timeline/bucket`. Only the columns we use; the rest are ignored.
#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct BucketColumns {
    id: Vec<String>,
    file_created_at: Vec<Option<String>>,
    local_offset_hours: Vec<Option<f64>>,
    is_trashed: Vec<Option<bool>>,
    visibility: Vec<Option<Visibility>>,
    is_image: Vec<Option<bool>>,
}

/// The value at `index` of a column, or `fallback` when the column was left out.
fn cell<T: Clone>(column: &[Option<T>], index: usize, fallback: T) -> T {
    column.get(index).cloned().flatten().unwrap_or(fallback)
}

fn check_column_length<T>(name: &str, column: &[T], rows: usize) -> Result<(), ApiError> {
    if column.is_empty() || column.len() == rows {
        Ok(())
    } else {
        Err(ApiError::Protocol(format!(
            "timeline bucket column `{name}` has {} entries for {rows} ids",
            column.len()
        )))
    }
}

fn into_bucket_assets(columns: BucketColumns) -> Result<Vec<BucketAsset>, ApiError> {
    let rows = columns.id.len();
    check_column_length("fileCreatedAt", &columns.file_created_at, rows)?;
    check_column_length("localOffsetHours", &columns.local_offset_hours, rows)?;
    check_column_length("isTrashed", &columns.is_trashed, rows)?;
    check_column_length("visibility", &columns.visibility, rows)?;
    check_column_length("isImage", &columns.is_image, rows)?;

    columns
        .id
        .iter()
        .enumerate()
        .map(|(index, id)| {
            let file_created_at = columns
                .file_created_at
                .get(index)
                .and_then(|text| text.as_deref())
                .and_then(parse_utc)
                .ok_or_else(|| ApiError::Protocol(format!("bucket Asset {id} has no usable fileCreatedAt")))?;
            Ok(BucketAsset {
                id: id.clone(),
                file_created_at,
                local_offset_hours: cell(&columns.local_offset_hours, index, 0.0),
                is_trashed: cell(&columns.is_trashed, index, false),
                visibility: cell(&columns.visibility, index, Visibility::Timeline),
                is_image: cell(&columns.is_image, index, true),
            })
        })
        .collect()
}

impl Client {
    /// The months of a timeline and how many Assets each holds, newest first (`GET /timeline/buckets`).
    pub async fn time_buckets(&self, scope: &TimelineScope) -> Result<Vec<TimeBucket>, ApiError> {
        let wire: Vec<TimeBucketWire> = self.get_json(&["timeline", "buckets"], &scope.query()).await?;
        into_time_buckets(wire)
    }

    /// [`Client::time_buckets`], unless nothing changed since `etag` (Immich answers 304 then).
    pub async fn time_buckets_if_changed(
        &self,
        scope: &TimelineScope,
        etag: Option<&str>,
    ) -> Result<Fetched<Vec<TimeBucket>>, ApiError> {
        let fetched: Fetched<Vec<TimeBucketWire>> = self
            .get_json_if_changed(&["timeline", "buckets"], &scope.query(), etag)
            .await?;
        match fetched {
            Fetched::NotModified => Ok(Fetched::NotModified),
            Fetched::Changed { value, etag } => Ok(Fetched::Changed {
                value: into_time_buckets(value)?,
                etag,
            }),
        }
    }

    /// The Assets of one month of a timeline (`GET /timeline/bucket`): ids and dates only.
    pub async fn time_bucket(&self, scope: &TimelineScope, month: YearMonth) -> Result<Vec<BucketAsset>, ApiError> {
        let key = month.bucket_key();
        let mut query = scope.query();
        query.push(("timeBucket", key.as_str()));
        let columns: BucketColumns = self.get_json(&["timeline", "bucket"], &query).await?;
        into_bucket_assets(columns)
    }

    /// [`Client::time_bucket`], unless nothing changed since `etag`.
    pub async fn time_bucket_if_changed(
        &self,
        scope: &TimelineScope,
        month: YearMonth,
        etag: Option<&str>,
    ) -> Result<Fetched<Vec<BucketAsset>>, ApiError> {
        let key = month.bucket_key();
        let mut query = scope.query();
        query.push(("timeBucket", key.as_str()));
        let fetched: Fetched<BucketColumns> = self.get_json_if_changed(&["timeline", "bucket"], &query, etag).await?;
        match fetched {
            Fetched::NotModified => Ok(Fetched::NotModified),
            Fetched::Changed { value, etag } => Ok(Fetched::Changed {
                value: into_bucket_assets(value)?,
                etag,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures;
    use crate::test_server::{CannedResponse, FakeServer};

    #[test]
    fn year_month_parses_bucket_keys() {
        assert_eq!(YearMonth::parse("2025-04-01"), Some(YearMonth { year: 2025, month: 4 }));
        assert_eq!(
            YearMonth::parse("2024-01-01T00:00:00.000Z"),
            Some(YearMonth { year: 2024, month: 1 })
        );
        assert_eq!(YearMonth::parse("1970-01"), Some(YearMonth { year: 1970, month: 1 }));
        for bad in ["2025-13-01", "2025-4-01", "2025", "", "2025-04-xx", "April"] {
            assert_eq!(YearMonth::parse(bad), None, "accepted {bad}");
        }
    }

    #[test]
    fn year_month_formats_and_steps() {
        let december = YearMonth::new(2019, 12).unwrap();
        assert_eq!(december.to_string(), "2019-12");
        assert_eq!(december.bucket_key(), "2019-12-01");
        assert_eq!(december.next(), YearMonth { year: 2020, month: 1 });
        assert_eq!(december.next().previous(), december);
        assert!(YearMonth::new(2019, 0).is_none());
        assert!("2019-12-01".parse::<YearMonth>().is_ok());
    }

    #[test]
    fn widened_utc_range_covers_every_zone() {
        let march = YearMonth::new(2020, 3).unwrap();
        let range = march.utc_range_widened(Duration::hours(14));
        assert_eq!(range.start, parse_utc("2020-02-29T10:00:00Z").unwrap());
        assert_eq!(range.end, parse_utc("2020-04-01T14:00:00Z").unwrap());
    }

    #[test]
    fn buckets_decode() {
        let wire: Vec<TimeBucketWire> = serde_json::from_str(fixtures::TIME_BUCKETS).unwrap();
        let buckets = into_time_buckets(wire).unwrap();
        assert_eq!(buckets.len(), 3);
        assert_eq!(
            buckets[0],
            TimeBucket {
                month: YearMonth { year: 2025, month: 4 },
                count: 1
            }
        );
        assert_eq!(buckets[2].month, YearMonth { year: 1970, month: 1 });
    }

    #[test]
    fn columnar_bucket_decodes_with_utc_without_z() {
        let columns: BucketColumns = serde_json::from_str(fixtures::TIME_BUCKET_COLUMNS).unwrap();
        let assets = into_bucket_assets(columns).unwrap();
        assert_eq!(assets.len(), 3);
        let first = &assets[0];
        assert_eq!(first.id, "2d6e3655-50dd-40f5-b8cf-f7d3f56aa95e");
        assert_eq!(first.file_created_at, parse_utc("2018-11-17T12:04:36Z").unwrap());
        assert_eq!(first.local_offset_hours, 1.0);
        assert_eq!(first.visibility, Visibility::Timeline);
        assert!(first.is_image);
        assert!(!first.is_trashed);
        assert_eq!(
            first.local_date_time(),
            parse_utc("2018-11-17T13:04:36Z").unwrap().naive_utc()
        );
    }

    #[test]
    fn missing_columns_default_and_short_columns_fail() {
        let only_ids = r#"{"id":["a","b"],"fileCreatedAt":["2018-11-17T12:04:36","2018-11-17T11:31:54"]}"#;
        let assets = into_bucket_assets(serde_json::from_str(only_ids).unwrap()).unwrap();
        assert_eq!(assets[1].local_offset_hours, 0.0);
        assert_eq!(assets[1].visibility, Visibility::Timeline);

        let short = r#"{"id":["a","b"],"fileCreatedAt":["2018-11-17T12:04:36"]}"#;
        assert!(matches!(
            into_bucket_assets(serde_json::from_str(short).unwrap()),
            Err(ApiError::Protocol(_))
        ));
    }

    #[test]
    fn fractional_offsets() {
        let columns = r#"{"id":["a"],"fileCreatedAt":["2020-01-01T00:00:00"],"localOffsetHours":[5.5]}"#;
        let assets = into_bucket_assets(serde_json::from_str(columns).unwrap()).unwrap();
        assert_eq!(
            assets[0].local_date_time(),
            parse_utc("2020-01-01T05:30:00Z").unwrap().naive_utc()
        );
    }

    #[tokio::test]
    async fn scopes_send_the_right_filters() {
        let server = FakeServer::start(vec![
            CannedResponse::json(200, "[]"),
            CannedResponse::json(200, "[]"),
            CannedResponse::json(200, "{}"),
            CannedResponse::json(200, "[]"),
            CannedResponse::json(200, "[]"),
        ])
        .await;
        let client = Client::new(&server.url, None, "test").unwrap();
        client.time_buckets(&TimelineScope::Library).await.unwrap();
        client.time_buckets(&TimelineScope::Album("album-1".into())).await.unwrap();
        let month = YearMonth::new(2018, 11).unwrap();
        client
            .time_bucket(&TimelineScope::Person("person-1".into()), month)
            .await
            .unwrap();
        client.time_buckets(&TimelineScope::Archive).await.unwrap();
        client.time_buckets(&TimelineScope::Favorites).await.unwrap();
        let targets: Vec<String> = server.requests().into_iter().map(|request| request.target).collect();
        assert_eq!(
            targets,
            [
                "/api/timeline/buckets?visibility=timeline",
                "/api/timeline/buckets?albumId=album-1",
                "/api/timeline/bucket?personId=person-1&visibility=timeline&timeBucket=2018-11-01",
                "/api/timeline/buckets?visibility=archive",
                "/api/timeline/buckets?isFavorite=true",
            ]
        );
    }
}
