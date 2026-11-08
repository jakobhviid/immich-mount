//! Everything Immich knows about one Asset, for a viewer's info panel: the full Asset with its camera details,
//! people, tags and stack, plus its OCR text, its key-value metadata, and counts over the library.
//!
//! [`crate::Asset`] is the slim model the mount lists by; [`AssetDetails`] is the same `GET /assets/{id}` answer
//! read whole.

use chrono::{DateTime, NaiveDateTime, Utc};
use reqwest::Method;
use serde::Deserialize;

use crate::asset::{AssetKind, Visibility};
use crate::client::Client;
use crate::error::ApiError;
use crate::person::Person;
use crate::tag::Tag;
use crate::user::User;
use crate::wire::{endpoints, Endpoint};

pub(crate) const ENDPOINTS: &[Endpoint] = endpoints![
    GET "/assets/statistics" "asset.statistics",
    GET "/assets/{id}/ocr" "asset.read",
    GET "/assets/{id}/metadata" "asset.read",
    GET "/assets/{id}/metadata/{key}" "asset.read",
];

/// One Asset, read whole (`AssetResponseDto`).
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AssetDetails {
    pub id: String,
    pub owner_id: String,
    /// Sent when the server includes it (not for every endpoint).
    pub owner: Option<User>,
    #[serde(rename = "type")]
    pub kind: AssetKind,
    pub visibility: Visibility,
    pub original_file_name: String,
    pub original_mime_type: Option<String>,
    /// The original's path on the server.
    pub original_path: String,
    /// When it was taken, in UTC.
    #[serde(deserialize_with = "crate::dates::serde_dates::utc")]
    pub file_created_at: DateTime<Utc>,
    #[serde(deserialize_with = "crate::dates::serde_dates::utc")]
    pub file_modified_at: DateTime<Utc>,
    /// When it was taken, on the wall clock where it was taken.
    #[serde(deserialize_with = "crate::dates::serde_dates::optional_wall_clock")]
    pub local_date_time: Option<NaiveDateTime>,
    /// When it was uploaded.
    #[serde(deserialize_with = "crate::dates::serde_dates::utc")]
    pub created_at: DateTime<Utc>,
    #[serde(deserialize_with = "crate::dates::serde_dates::utc")]
    pub updated_at: DateTime<Utc>,
    pub is_favorite: bool,
    pub is_archived: bool,
    pub is_trashed: bool,
    /// The file of an external library is missing on disk.
    pub is_offline: bool,
    /// Rotated or cropped in Immich ([`Client::asset_edits`]).
    pub is_edited: bool,
    pub has_metadata: bool,
    /// Base64 SHA-1 of the original.
    pub checksum: String,
    /// A tiny placeholder picture (base64 ThumbHash), shown while the thumbnail loads.
    pub thumbhash: Option<String>,
    /// Video length in milliseconds; `None` for images.
    #[serde(rename = "duration", deserialize_with = "crate::asset::duration_ms")]
    pub duration_ms: Option<u64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    #[serde(rename = "exifInfo")]
    pub exif: Option<Exif>,
    /// The people recognised in it; sent only by some endpoints (`GET /assets/{id}` among them).
    pub people: Vec<Person>,
    pub tags: Vec<Tag>,
    pub stack: Option<StackSummary>,
    /// For the still half of a Live Photo: its video half.
    pub live_photo_video_id: Option<String>,
    /// The duplicate group it is in, if Immich found duplicates.
    pub duplicate_id: Option<String>,
    /// The external library it comes from; `None` for uploads.
    pub library_id: Option<String>,
}

/// Camera and place details (`ExifResponseDto`). Every field may be missing.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Exif {
    pub make: Option<String>,
    pub model: Option<String>,
    pub lens_model: Option<String>,
    #[serde(rename = "fNumber")]
    pub f_number: Option<f64>,
    /// In millimetres.
    pub focal_length: Option<f64>,
    pub iso: Option<u32>,
    /// As the camera wrote it, such as `1/250`.
    pub exposure_time: Option<String>,
    #[serde(rename = "exifImageWidth")]
    pub image_width: Option<u32>,
    #[serde(rename = "exifImageHeight")]
    pub image_height: Option<u32>,
    pub orientation: Option<String>,
    #[serde(rename = "fileSizeInByte")]
    pub file_size: Option<u64>,
    #[serde(deserialize_with = "crate::dates::serde_dates::optional_utc")]
    pub date_time_original: Option<DateTime<Utc>>,
    #[serde(deserialize_with = "crate::dates::serde_dates::optional_utc")]
    pub modify_date: Option<DateTime<Utc>>,
    /// An IANA zone such as `Europe/Copenhagen`, or an offset such as `UTC+2`.
    pub time_zone: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub city: Option<String>,
    pub state: Option<String>,
    pub country: Option<String>,
    pub description: Option<String>,
    /// -1 (rejected) to 5.
    pub rating: Option<i8>,
    /// `EQUIRECTANGULAR` for a 360° photo.
    pub projection_type: Option<String>,
}

/// The stack an Asset belongs to, as the Asset shows it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct StackSummary {
    pub id: String,
    pub primary_asset_id: String,
    pub asset_count: u64,
}

/// A piece of text found in a picture, with its box (corners clockwise from top left, as fractions of the
/// picture's width and height).
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct OcrText {
    pub id: String,
    pub asset_id: String,
    pub text: String,
    /// How sure the detector is that the box holds text, 0 to 1.
    pub box_score: f64,
    /// How sure the recogniser is of the text, 0 to 1.
    pub text_score: f64,
    pub x1: f64,
    pub y1: f64,
    pub x2: f64,
    pub y2: f64,
    pub x3: f64,
    pub y3: f64,
    pub x4: f64,
    pub y4: f64,
}

/// A key-value entry an app attached to an Asset (`AssetMetadataResponseDto`). The value is any JSON object.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MetadataEntry {
    /// Present in bulk answers, which span several Assets.
    pub asset_id: Option<String>,
    pub key: String,
    pub value: serde_json::Value,
    #[serde(deserialize_with = "crate::dates::serde_dates::optional_utc")]
    pub updated_at: Option<DateTime<Utc>>,
}

/// How many Assets match (`GET /assets/statistics`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(default)]
pub struct AssetStatistics {
    pub images: u64,
    pub videos: u64,
    pub total: u64,
}

/// Which Assets [`Client::asset_statistics`] counts; `None` does not filter.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StatisticsFilter {
    pub favorite: Option<bool>,
    pub trashed: Option<bool>,
    pub visibility: Option<Visibility>,
}

impl Client {
    /// One Asset, read whole (`GET /assets/{id}`). Needs `asset.read`.
    pub async fn asset_details(&self, id: &str) -> Result<AssetDetails, ApiError> {
        self.call(Method::GET, &["assets", id], &[]).await
    }

    /// How many images and videos match (`GET /assets/statistics`). Needs `asset.statistics`.
    pub async fn asset_statistics(&self, filter: &StatisticsFilter) -> Result<AssetStatistics, ApiError> {
        let mut query: Vec<(&str, &str)> = Vec::new();
        if let Some(favorite) = filter.favorite {
            query.push(("isFavorite", if favorite { "true" } else { "false" }));
        }
        if let Some(trashed) = filter.trashed {
            query.push(("isTrashed", if trashed { "true" } else { "false" }));
        }
        if let Some(visibility) = &filter.visibility {
            query.push(("visibility", visibility.as_str()));
        }
        self.call(Method::GET, &["assets", "statistics"], &query).await
    }

    /// The text found in a picture (`GET /assets/{id}/ocr`); empty until machine learning has read it. Needs
    /// `asset.read`.
    pub async fn asset_ocr(&self, id: &str) -> Result<Vec<OcrText>, ApiError> {
        self.call(Method::GET, &["assets", id, "ocr"], &[]).await
    }

    /// Every key-value entry on an Asset (`GET /assets/{id}/metadata`). Needs `asset.read`.
    pub async fn asset_metadata(&self, id: &str) -> Result<Vec<MetadataEntry>, ApiError> {
        self.call(Method::GET, &["assets", id, "metadata"], &[]).await
    }

    /// One key-value entry on an Asset (`GET /assets/{id}/metadata/{key}`). Needs `asset.read`.
    pub async fn asset_metadata_entry(&self, id: &str, key: &str) -> Result<MetadataEntry, ApiError> {
        self.call(Method::GET, &["assets", id, "metadata", key], &[]).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::SEARCH_ITEM;

    #[test]
    fn a_search_item_reads_whole() {
        let details: AssetDetails = crate::client::decode_json(SEARCH_ITEM.as_bytes(), "assets").unwrap();
        assert_eq!(details.id, "12ecb329-c96f-41b1-8406-e5b8fe134454");
        assert_eq!(details.kind, AssetKind::Image);
        assert_eq!(details.visibility, Visibility::Timeline);
        assert_eq!(details.duration_ms, None);
        let exif = details.exif.unwrap();
        assert_eq!(exif.make.as_deref(), Some("samsung"));
        assert_eq!(exif.file_size, Some(1_012_520));
        assert_eq!(exif.city.as_deref(), Some("Reykjavík"));
        assert_eq!(exif.time_zone.as_deref(), Some("Atlantic/Reykjavik"));
    }

    #[test]
    fn a_video_duration_reads_in_either_spelling() {
        let new: AssetDetails = crate::client::decode_json(br#"{"id":"v","type":"VIDEO","duration":62500}"#, "a").unwrap();
        let old: AssetDetails =
            crate::client::decode_json(br#"{"id":"v","type":"VIDEO","duration":"0:01:02.500000"}"#, "a").unwrap();
        assert_eq!(new.duration_ms, Some(62_500));
        assert_eq!(old.duration_ms, Some(62_500));
    }
}
