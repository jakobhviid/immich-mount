//! The Asset: one photo or video in Immich, as search and `GET /assets/{id}` describe it.
//!
//! Decoding is lenient by construction. The wire struct has every field optional and ignores unknown ones; the
//! conversion to [`Asset`] then insists only on what the mount cannot work without (id, file name, a capture
//! date) and fills the rest with documented fallbacks. So a field missing on an older server, or a new one,
//! never breaks a listing.
//!
//! Quirks encoded here, seen on Immich v3.3.5:
//! - `localDateTime` is a wall-clock time with a fake `Z`; it is kept as a [`NaiveDateTime`]
//! - the file size exists only in `exifInfo.fileSizeInByte`, and `exifInfo` exists only with `withExif: true`
//! - `checksum` is a base64 SHA-1
//! - `duration` is whole milliseconds on v3.3.5 (null for images); older servers sent `"H:MM:SS.ffffff"`
//! - an unknown or deleted id answers 400 `Not found or no asset.read access`, which maps to `NotFound`

use chrono::{DateTime, NaiveDateTime, Utc};
use serde::Deserialize;

use crate::client::Client;
use crate::dates::{optional_utc, parse_utc, parse_wall_clock};
use crate::error::ApiError;

/// What kind of media an Asset is.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AssetKind {
    Image,
    Video,
    /// Anything else the server sends (`AUDIO`, `OTHER`, or a future kind), verbatim.
    Other(String),
}

impl AssetKind {
    /// The wire value, as sent in filters.
    pub fn as_str(&self) -> &str {
        match self {
            AssetKind::Image => "IMAGE",
            AssetKind::Video => "VIDEO",
            AssetKind::Other(value) => value,
        }
    }
}

impl From<String> for AssetKind {
    fn from(wire: String) -> Self {
        match wire.to_ascii_uppercase().as_str() {
            "IMAGE" => AssetKind::Image,
            "VIDEO" => AssetKind::Video,
            _ => AssetKind::Other(wire),
        }
    }
}

impl Default for AssetKind {
    /// A kind the server did not say.
    fn default() -> Self {
        AssetKind::Other(String::new())
    }
}

impl<'de> Deserialize<'de> for AssetKind {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(AssetKind::from)
    }
}

/// Where an Asset shows up in Immich.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub enum Visibility {
    /// On the main timeline.
    #[default]
    Timeline,
    /// Archived: off the timeline, still in albums and searches that do not filter on visibility.
    Archive,
    /// Hidden, such as the video half of a Live Photo.
    Hidden,
    /// In the locked folder, which needs elevated permission.
    Locked,
    /// A value this client does not know, verbatim.
    Other(String),
}

impl Visibility {
    /// The wire value, as sent in filters.
    pub fn as_str(&self) -> &str {
        match self {
            Visibility::Timeline => "timeline",
            Visibility::Archive => "archive",
            Visibility::Hidden => "hidden",
            Visibility::Locked => "locked",
            Visibility::Other(value) => value,
        }
    }
}

impl From<String> for Visibility {
    fn from(wire: String) -> Self {
        match wire.as_str() {
            "timeline" => Visibility::Timeline,
            "archive" => Visibility::Archive,
            "hidden" => Visibility::Hidden,
            "locked" => Visibility::Locked,
            _ => Visibility::Other(wire),
        }
    }
}

impl<'de> Deserialize<'de> for Visibility {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(Visibility::from)
    }
}

impl serde::Serialize for Visibility {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

/// A model field holding a duration, in either spelling ([`DurationWire`]), as milliseconds.
pub(crate) fn duration_ms<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<u64>, D::Error> {
    let wire = Option::<DurationWire>::deserialize(deserializer)?;
    Ok(wire.as_ref().and_then(DurationWire::to_milliseconds))
}

/// One photo or video.
#[derive(Debug, Clone, PartialEq)]
pub struct Asset {
    pub id: String,
    /// The name the file had when it was uploaded.
    pub original_file_name: String,
    /// When it was taken, in UTC (from EXIF, converted with the GPS time zone).
    pub file_created_at: DateTime<Utc>,
    /// When it was taken, on the wall clock where it was taken. Timeline months group by this.
    pub local_date_time: NaiveDateTime,
    /// The file's modification time as the uploading client sent it; also the original's `Last-Modified`.
    pub file_modified_at: DateTime<Utc>,
    /// When it was uploaded to Immich.
    pub created_at: DateTime<Utc>,
    /// When Immich last changed anything about it (not bumped by album membership).
    pub updated_at: DateTime<Utc>,
    pub kind: AssetKind,
    pub visibility: Visibility,
    pub is_trashed: bool,
    pub is_favorite: bool,
    /// Base64 SHA-1 of the original.
    pub checksum: String,
    pub original_mime_type: Option<String>,
    /// The original's size in bytes; `None` when the server sent no `exifInfo`.
    pub file_size: Option<u64>,
    pub city: Option<String>,
    pub country: Option<String>,
    /// Video length in milliseconds; `None` for images.
    pub duration_ms: Option<u64>,
    pub stack_id: Option<String>,
    /// For the still half of a Live Photo: the id of its (hidden) video half.
    pub live_photo_video_id: Option<String>,
}

/// An Asset as it arrives. Everything optional, unknown fields ignored; see the module header.
#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct AssetWire {
    id: Option<String>,
    original_file_name: Option<String>,
    file_created_at: Option<String>,
    local_date_time: Option<String>,
    file_modified_at: Option<String>,
    created_at: Option<String>,
    updated_at: Option<String>,
    #[serde(rename = "type")]
    kind: Option<AssetKind>,
    visibility: Option<Visibility>,
    is_archived: Option<bool>,
    is_trashed: Option<bool>,
    is_favorite: Option<bool>,
    checksum: Option<String>,
    original_mime_type: Option<String>,
    duration: Option<DurationWire>,
    exif_info: Option<ExifWire>,
    stack: Option<StackWire>,
    live_photo_video_id: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct ExifWire {
    file_size_in_byte: Option<u64>,
    city: Option<String>,
    country: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct StackWire {
    id: Option<String>,
}

/// A duration as whole milliseconds (v3.3.5) or as `"H:MM:SS.ffffff"` (older servers).
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum DurationWire {
    Milliseconds(f64),
    Text(String),
}

impl DurationWire {
    pub(crate) fn to_milliseconds(&self) -> Option<u64> {
        match self {
            DurationWire::Milliseconds(milliseconds) if *milliseconds >= 0.0 => Some(milliseconds.round() as u64),
            DurationWire::Milliseconds(_) => None,
            DurationWire::Text(text) => parse_clock_duration(text),
        }
    }
}

/// `"0:01:02.500000"` as 62 500 ms.
fn parse_clock_duration(text: &str) -> Option<u64> {
    let mut parts = text.trim().split(':');
    let hours: f64 = parts.next()?.parse().ok()?;
    let minutes: f64 = parts.next()?.parse().ok()?;
    let seconds: f64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || hours < 0.0 || minutes < 0.0 || seconds < 0.0 {
        return None;
    }
    Some(((hours * 3600.0 + minutes * 60.0 + seconds) * 1000.0).round() as u64)
}

/// Text that is present and not blank.
pub(crate) fn non_blank(text: Option<String>) -> Option<String> {
    text.filter(|value| !value.trim().is_empty())
}

impl AssetWire {
    /// The Asset, or a protocol error naming what essential field was missing.
    pub(crate) fn into_asset(self) -> Result<Asset, ApiError> {
        let id = non_blank(self.id).ok_or_else(|| ApiError::Protocol("an Asset without an id".to_owned()))?;
        let missing = |field: &str| ApiError::Protocol(format!("Asset {id} has no usable {field}"));

        let original_file_name = non_blank(self.original_file_name).ok_or_else(|| missing("originalFileName"))?;
        let file_created_at = self
            .file_created_at
            .as_deref()
            .and_then(parse_utc)
            .ok_or_else(|| missing("fileCreatedAt"))?;
        // Fallbacks for dates every supported server sends; they only matter if a future server drops one.
        let local_date_time = self
            .local_date_time
            .as_deref()
            .and_then(parse_wall_clock)
            .unwrap_or_else(|| file_created_at.naive_utc());
        let file_modified_at = optional_utc(self.file_modified_at.as_deref()).unwrap_or(file_created_at);
        let created_at = optional_utc(self.created_at.as_deref()).unwrap_or(file_created_at);
        let updated_at = optional_utc(self.updated_at.as_deref()).unwrap_or(created_at);

        let visibility = match (self.visibility, self.is_archived) {
            (Some(visibility), _) => visibility,
            (None, Some(true)) => Visibility::Archive,
            (None, _) => Visibility::Timeline,
        };
        let exif = self.exif_info.unwrap_or_default();

        Ok(Asset {
            original_file_name,
            file_created_at,
            local_date_time,
            file_modified_at,
            created_at,
            updated_at,
            kind: self.kind.unwrap_or(AssetKind::Image),
            visibility,
            is_trashed: self.is_trashed.unwrap_or(false),
            is_favorite: self.is_favorite.unwrap_or(false),
            checksum: self.checksum.unwrap_or_default(),
            original_mime_type: non_blank(self.original_mime_type),
            file_size: exif.file_size_in_byte,
            city: non_blank(exif.city),
            country: non_blank(exif.country),
            duration_ms: self.duration.as_ref().and_then(DurationWire::to_milliseconds),
            stack_id: self.stack.and_then(|stack| non_blank(stack.id)),
            live_photo_video_id: non_blank(self.live_photo_video_id),
            id,
        })
    }
}

pub(crate) const ENDPOINTS: &[crate::wire::Endpoint] = crate::wire::endpoints![
    GET "/assets/{id}" "asset.read",
];

impl Client {
    /// One Asset by id (`GET /assets/{id}`).
    ///
    /// An unknown, deleted or invisible id is [`ApiError::NotFound`] (Immich answers 400 there). Note that this
    /// endpoint does not take `withExif`, but its answer includes `exifInfo`, so `file_size` is filled in.
    /// Needs `asset.read`.
    pub async fn asset(&self, id: &str) -> Result<Asset, ApiError> {
        let wire: AssetWire = self.get_json(&["assets", id], &[]).await?;
        wire.into_asset()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::SEARCH_ITEM;
    use crate::test_server::{CannedResponse, FakeServer};
    use chrono::NaiveDate;

    fn decode(json: &str) -> Result<Asset, ApiError> {
        serde_json::from_str::<AssetWire>(json).unwrap().into_asset()
    }

    #[test]
    fn decodes_a_search_item() {
        let asset = decode(SEARCH_ITEM).unwrap();
        assert_eq!(asset.id, "12ecb329-c96f-41b1-8406-e5b8fe134454");
        assert_eq!(asset.original_file_name, "Tjarnarbíó 2025.jpg");
        assert_eq!(asset.kind, AssetKind::Image);
        assert_eq!(asset.visibility, Visibility::Timeline);
        assert_eq!(asset.file_size, Some(1_012_520));
        assert_eq!(asset.city.as_deref(), Some("Reykjavík"));
        assert_eq!(asset.country.as_deref(), Some("Iceland"));
        assert_eq!(asset.checksum, "JpqXUgvoX45cHzen+eI2fGVg89U=");
        assert_eq!(asset.original_mime_type.as_deref(), Some("image/jpeg"));
        assert_eq!(asset.duration_ms, None);
        assert!(!asset.is_trashed);
    }

    #[test]
    fn local_date_time_keeps_the_wall_clock_despite_the_fake_z() {
        // The month-boundary photo: taken 2020-03-01 00:30 at +02:00.
        let json = r#"{"id":"a","originalFileName":"x.jpg","type":"IMAGE",
            "fileCreatedAt":"2020-02-29T22:30:00.000Z","localDateTime":"2020-03-01T00:30:00.000Z"}"#;
        let asset = decode(json).unwrap();
        assert_eq!(
            asset.file_created_at,
            NaiveDate::from_ymd_opt(2020, 2, 29).unwrap().and_hms_opt(22, 30, 0).unwrap().and_utc()
        );
        assert_eq!(
            asset.local_date_time,
            NaiveDate::from_ymd_opt(2020, 3, 1).unwrap().and_hms_opt(0, 30, 0).unwrap()
        );
    }

    #[test]
    fn missing_exif_info_means_no_size_and_no_place() {
        let json = r#"{"id":"a","originalFileName":"x.jpg","type":"VIDEO","duration":61500,
            "fileCreatedAt":"2022-06-15T08:00:00.000Z","localDateTime":"2022-06-15T10:00:00.000Z",
            "fileModifiedAt":"2022-06-15T08:00:00.000Z","createdAt":"2026-11-01T21:01:55.220Z",
            "updatedAt":"2026-11-01T21:01:55.220Z","visibility":"archive","isTrashed":true}"#;
        let asset = decode(json).unwrap();
        assert_eq!(asset.file_size, None);
        assert_eq!(asset.city, None);
        assert_eq!(asset.kind, AssetKind::Video);
        assert_eq!(asset.duration_ms, Some(61_500));
        assert_eq!(asset.visibility, Visibility::Archive);
        assert!(asset.is_trashed);
    }

    #[test]
    fn older_servers_duration_text_and_archive_flag() {
        let json = r#"{"id":"a","originalFileName":"x.mp4","type":"VIDEO","duration":"0:01:02.500000",
            "fileCreatedAt":"2022-06-15T08:00:00.000Z","isArchived":true}"#;
        let asset = decode(json).unwrap();
        assert_eq!(asset.duration_ms, Some(62_500));
        assert_eq!(asset.visibility, Visibility::Archive);
        // Missing dates fall back to the capture date.
        assert_eq!(asset.created_at, asset.file_created_at);
        assert_eq!(asset.updated_at, asset.file_created_at);
    }

    #[test]
    fn unknown_kinds_and_visibilities_are_kept() {
        let json = r#"{"id":"a","originalFileName":"x.mp3","type":"AUDIO","visibility":"vault",
            "fileCreatedAt":"2022-06-15T08:00:00.000Z","stack":{"id":"s1","primaryAssetId":"a","assetCount":2}}"#;
        let asset = decode(json).unwrap();
        assert_eq!(asset.kind, AssetKind::Other("AUDIO".to_owned()));
        assert_eq!(asset.visibility, Visibility::Other("vault".to_owned()));
        assert_eq!(asset.stack_id.as_deref(), Some("s1"));
    }

    #[tokio::test]
    async fn bad_request_not_found_is_not_found() {
        let body = r#"{"message":"Not found or no asset.read access","error":"Bad Request","statusCode":400}"#;
        let server = FakeServer::start(vec![CannedResponse::json(400, body)]).await;
        let client = Client::new(&server.url, None, "test").unwrap();
        assert_eq!(client.asset("gone").await.unwrap_err(), ApiError::NotFound);
        assert_eq!(server.requests()[0].target, "/api/assets/gone");
    }

    #[test]
    fn essential_fields_are_required() {
        assert!(matches!(decode(r#"{"originalFileName":"x.jpg"}"#), Err(ApiError::Protocol(_))));
        assert!(matches!(
            decode(r#"{"id":"a","fileCreatedAt":"2022-06-15T08:00:00.000Z"}"#),
            Err(ApiError::Protocol(_))
        ));
        assert!(matches!(
            decode(r#"{"id":"a","originalFileName":"x.jpg","fileCreatedAt":"never"}"#),
            Err(ApiError::Protocol(_))
        ));
    }
}
