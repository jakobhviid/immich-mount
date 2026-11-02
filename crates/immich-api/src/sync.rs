//! The sync stream: exact, resumable change records for a session (`POST /sync/stream`, `POST /sync/ack`).
//!
//! Only a session gets it: every API key is refused with 403 "Sync endpoints cannot be used with API keys"
//! (seen on v3.3.5). The answer is JSON lines, `{"type": ..., "data": ..., "ack": ...}`, read one at a
//! time so a first sync of a large library never sits in memory whole. After applying records, the caller acks
//! the last `ack` of each type; the next stream then starts after it. Trashed and archived Assets are included,
//! and a permanent delete arrives as `AssetDeleteV1`.
//!
//! The record types the mount needs are typed; the rest arrive as [`SyncRecord::Other`] with their raw JSON.
//! Request types are passed as their wire names (`"AssetsV2"`, `"AssetExifsV1"`, `"AlbumsV2"`,
//! `"AlbumToAssetsV1"`, `"PeopleV1"`, `"AssetFacesV1"`, ...), so a newer server's types need no new code.

use bytes::Bytes;
use chrono::{DateTime, NaiveDateTime, Utc};
use futures_util::stream::BoxStream;
use futures_util::StreamExt;
use reqwest::Method;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::asset::{non_blank, AssetKind, DurationWire, Visibility};
use crate::client::{check, Client};
use crate::dates::{optional_utc, parse_wall_clock};
use crate::error::{from_transport, ApiError, RequestShape};

/// One line of the stream: a record and the ack that confirms it.
#[derive(Debug, Clone, PartialEq)]
pub struct SyncLine {
    pub ack: String,
    pub record: SyncRecord,
}

/// A change record.
#[derive(Debug, Clone, PartialEq)]
pub enum SyncRecord {
    /// An Asset was created or changed (including trashed, archived and restored).
    AssetV2(SyncAsset),
    /// An Asset was deleted for good.
    AssetDeleteV1 { asset_id: String },
    /// An Asset's EXIF changed; this carries its file size and place.
    AssetExifV1(SyncAssetExif),
    /// The server wants the client to discard its state and sync from scratch.
    SyncResetV1,
    /// The end of this stream's changes.
    SyncCompleteV1,
    /// Any other record type, with its wire name and raw data.
    Other { kind: String, data: Value },
}

/// An Asset as the sync stream describes it.
#[derive(Debug, Clone, PartialEq)]
pub struct SyncAsset {
    pub id: String,
    pub owner_id: String,
    pub original_file_name: String,
    /// Base64 SHA-1.
    pub checksum: String,
    pub file_created_at: Option<DateTime<Utc>>,
    pub file_modified_at: Option<DateTime<Utc>>,
    pub created_at: Option<DateTime<Utc>>,
    /// Wall clock where it was taken; the fake `Z` is dropped.
    pub local_date_time: Option<NaiveDateTime>,
    pub kind: AssetKind,
    pub visibility: Visibility,
    /// Set when the Asset is in the trash.
    pub deleted_at: Option<DateTime<Utc>>,
    pub is_favorite: bool,
    pub duration_ms: Option<u64>,
    pub stack_id: Option<String>,
    pub live_photo_video_id: Option<String>,
}

/// The parts of an Asset's EXIF the mount uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncAssetExif {
    pub asset_id: String,
    pub file_size: Option<u64>,
    pub city: Option<String>,
    pub country: Option<String>,
    pub date_time_original: Option<DateTime<Utc>>,
    pub time_zone: Option<String>,
}

#[derive(Deserialize)]
struct LineWire {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    data: Value,
    #[serde(default)]
    ack: String,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct SyncAssetWire {
    id: Option<String>,
    owner_id: Option<String>,
    original_file_name: Option<String>,
    checksum: Option<String>,
    file_created_at: Option<String>,
    file_modified_at: Option<String>,
    created_at: Option<String>,
    local_date_time: Option<String>,
    #[serde(rename = "type")]
    kind: Option<AssetKind>,
    visibility: Option<Visibility>,
    deleted_at: Option<String>,
    is_favorite: Option<bool>,
    duration: Option<DurationWire>,
    stack_id: Option<String>,
    live_photo_video_id: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct SyncAssetExifWire {
    asset_id: Option<String>,
    file_size_in_byte: Option<u64>,
    city: Option<String>,
    country: Option<String>,
    date_time_original: Option<String>,
    time_zone: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct AssetDeleteWire {
    asset_id: Option<String>,
}

fn decode_data<T: serde::de::DeserializeOwned>(kind: &str, data: Value) -> Result<T, ApiError> {
    serde_json::from_value(data).map_err(|error| ApiError::Protocol(format!("a {kind} sync record: {error}")))
}

fn required_id(kind: &str, id: Option<String>) -> Result<String, ApiError> {
    non_blank(id).ok_or_else(|| ApiError::Protocol(format!("a {kind} sync record without an id")))
}

/// Decode one JSON line.
fn parse_line(line: &[u8]) -> Result<SyncLine, ApiError> {
    let wire: LineWire = serde_json::from_slice(line)
        .map_err(|error| ApiError::Protocol(format!("an unreadable sync line: {error}")))?;
    let kind = wire.kind;
    let record = match kind.as_str() {
        "AssetV2" => {
            let asset: SyncAssetWire = decode_data(&kind, wire.data)?;
            SyncRecord::AssetV2(SyncAsset {
                id: required_id(&kind, asset.id)?,
                owner_id: asset.owner_id.unwrap_or_default(),
                original_file_name: asset.original_file_name.unwrap_or_default(),
                checksum: asset.checksum.unwrap_or_default(),
                file_created_at: optional_utc(asset.file_created_at.as_deref()),
                file_modified_at: optional_utc(asset.file_modified_at.as_deref()),
                created_at: optional_utc(asset.created_at.as_deref()),
                local_date_time: asset.local_date_time.as_deref().and_then(parse_wall_clock),
                kind: asset.kind.unwrap_or(AssetKind::Image),
                visibility: asset.visibility.unwrap_or_default(),
                deleted_at: optional_utc(asset.deleted_at.as_deref()),
                is_favorite: asset.is_favorite.unwrap_or(false),
                duration_ms: asset.duration.as_ref().and_then(DurationWire::to_milliseconds),
                stack_id: non_blank(asset.stack_id),
                live_photo_video_id: non_blank(asset.live_photo_video_id),
            })
        }
        "AssetDeleteV1" => {
            let deleted: AssetDeleteWire = decode_data(&kind, wire.data)?;
            SyncRecord::AssetDeleteV1 {
                asset_id: required_id(&kind, deleted.asset_id)?,
            }
        }
        "AssetExifV1" => {
            let exif: SyncAssetExifWire = decode_data(&kind, wire.data)?;
            SyncRecord::AssetExifV1(SyncAssetExif {
                asset_id: required_id(&kind, exif.asset_id)?,
                file_size: exif.file_size_in_byte,
                city: non_blank(exif.city),
                country: non_blank(exif.country),
                date_time_original: optional_utc(exif.date_time_original.as_deref()),
                time_zone: non_blank(exif.time_zone),
            })
        }
        "SyncResetV1" => SyncRecord::SyncResetV1,
        "SyncCompleteV1" => SyncRecord::SyncCompleteV1,
        _ => SyncRecord::Other {
            kind,
            data: wire.data,
        },
    };
    Ok(SyncLine { ack: wire.ack, record })
}

/// An open sync stream; read it with [`SyncStream::next_line`] until it returns `None`.
pub struct SyncStream {
    chunks: BoxStream<'static, reqwest::Result<Bytes>>,
    buffer: Vec<u8>,
    finished: bool,
}

impl std::fmt::Debug for SyncStream {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SyncStream")
            .field("buffered_bytes", &self.buffer.len())
            .field("finished", &self.finished)
            .finish()
    }
}

impl SyncStream {
    fn new(chunks: BoxStream<'static, reqwest::Result<Bytes>>) -> SyncStream {
        SyncStream {
            chunks,
            buffer: Vec::new(),
            finished: false,
        }
    }

    /// The next record, or `None` at the end of the stream. A dropped connection is [`ApiError::Offline`].
    pub async fn next_line(&mut self) -> Result<Option<SyncLine>, ApiError> {
        loop {
            if let Some(newline) = self.buffer.iter().position(|byte| *byte == b'\n') {
                let line: Vec<u8> = self.buffer.drain(..=newline).collect();
                if line.iter().all(u8::is_ascii_whitespace) {
                    continue;
                }
                return parse_line(&line).map(Some);
            }
            if self.finished {
                if self.buffer.iter().all(u8::is_ascii_whitespace) {
                    return Ok(None);
                }
                let line = std::mem::take(&mut self.buffer);
                return parse_line(&line).map(Some);
            }
            match self.chunks.next().await {
                Some(Ok(chunk)) => self.buffer.extend_from_slice(&chunk),
                Some(Err(error)) => return Err(from_transport(&error)),
                None => self.finished = true,
            }
        }
    }
}

impl Client {
    /// Open the sync stream for `types` (wire names such as `"AssetsV2"`). `reset` starts over from scratch.
    ///
    /// Needs a session; API keys get [`ApiError::Forbidden`]. No request timeout applies; bound the reading.
    pub async fn sync_stream(&self, types: &[&str], reset: bool) -> Result<SyncStream, ApiError> {
        let mut body = json!({ "types": types });
        if reset {
            body["reset"] = Value::Bool(true);
        }
        let builder = self.request(Method::POST, &["sync", "stream"]).json(&body);
        let response = check(self.execute(builder).await?, RequestShape::Plain).await?;
        Ok(SyncStream::new(response.bytes_stream().boxed()))
    }

    /// Confirm records up to these acks (`POST /sync/ack`), so the next stream starts after them.
    pub async fn sync_ack(&self, acks: &[String]) -> Result<(), ApiError> {
        self.post_without_answer(&["sync", "ack"], Some(&json!({ "acks": acks })))
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_server::{CannedResponse, FakeServer};

    const ASSET_LINE: &str = r#"{"type":"AssetV2","data":{"id":"a1","ownerId":"u1","originalFileName":"IMG_0001.JPG",
        "thumbhash":null,"checksum":"JpqXUgvoX45cHzen+eI2fGVg89U=","fileCreatedAt":"2018-11-17T11:31:54.000Z",
        "fileModifiedAt":"2018-11-17T11:31:54.000Z","createdAt":"2026-11-01T21:01:50.365Z",
        "localDateTime":"2018-11-17T12:31:54.000Z","duration":null,"type":"IMAGE","deletedAt":null,
        "isFavorite":false,"visibility":"timeline","livePhotoVideoId":null,"stackId":null,"libraryId":null,
        "width":1920,"height":1440,"isEdited":false},"ack":"AssetV2|019a4f..."}"#;

    #[test]
    fn asset_lines_decode() {
        let line = parse_line(ASSET_LINE.as_bytes()).unwrap();
        assert_eq!(line.ack, "AssetV2|019a4f...");
        let SyncRecord::AssetV2(asset) = line.record else {
            panic!("not an AssetV2: {:?}", line.record);
        };
        assert_eq!(asset.id, "a1");
        assert_eq!(asset.original_file_name, "IMG_0001.JPG");
        assert_eq!(asset.kind, AssetKind::Image);
        assert_eq!(asset.deleted_at, None);
        assert_eq!(
            asset.local_date_time.unwrap().to_string(),
            "2018-11-17 12:31:54",
            "wall clock, fake Z dropped"
        );
    }

    #[test]
    fn delete_exif_complete_and_unknown_lines_decode() {
        let delete = parse_line(br#"{"type":"AssetDeleteV1","data":{"assetId":"a1"},"ack":"d"}"#).unwrap();
        assert_eq!(delete.record, SyncRecord::AssetDeleteV1 { asset_id: "a1".to_owned() });

        let exif = parse_line(
            br#"{"type":"AssetExifV1","data":{"assetId":"a1","fileSizeInByte":1012520,"city":"Lyon","country":"France",
                "dateTimeOriginal":"2018-11-17T12:31:54+00:00","timeZone":"Europe/Paris","fNumber":1.7},"ack":"e"}"#,
        )
        .unwrap();
        let SyncRecord::AssetExifV1(exif) = exif.record else {
            panic!("not an AssetExifV1");
        };
        assert_eq!(exif.file_size, Some(1_012_520));
        assert_eq!(exif.city.as_deref(), Some("Lyon"));

        let complete = parse_line(br#"{"type":"SyncCompleteV1","data":{},"ack":"c"}"#).unwrap();
        assert_eq!(complete.record, SyncRecord::SyncCompleteV1);

        let album = parse_line(br#"{"type":"AlbumV2","data":{"id":"x","name":"Holiday Mix"},"ack":"al"}"#).unwrap();
        assert_eq!(
            album.record,
            SyncRecord::Other {
                kind: "AlbumV2".to_owned(),
                data: json!({ "id": "x", "name": "Holiday Mix" })
            }
        );
    }

    #[test]
    fn garbage_lines_are_protocol_errors() {
        assert!(matches!(parse_line(b"not json"), Err(ApiError::Protocol(_))));
        assert!(matches!(
            parse_line(br#"{"type":"AssetDeleteV1","data":{},"ack":"d"}"#),
            Err(ApiError::Protocol(_))
        ));
    }

    #[tokio::test]
    async fn stream_reads_lines_across_chunks_and_a_missing_final_newline() {
        let body = format!(
            "{}\n\n{}",
            ASSET_LINE.replace('\n', ""),
            r#"{"type":"SyncCompleteV1","data":{},"ack":"c"}"#
        );
        let server = FakeServer::start(vec![CannedResponse::bytes(200, "application/jsonlines+json", body.as_bytes())]).await;
        let client = Client::new(&server.url, Some(crate::Credential::Session("token".into())), "test").unwrap();
        let mut stream = client.sync_stream(&["AssetsV2", "AssetExifsV1"], false).await.unwrap();
        let first = stream.next_line().await.unwrap().unwrap();
        assert!(matches!(first.record, SyncRecord::AssetV2(_)));
        let second = stream.next_line().await.unwrap().unwrap();
        assert_eq!(second.record, SyncRecord::SyncCompleteV1);
        assert_eq!(stream.next_line().await.unwrap(), None);
        let request = &server.requests()[0];
        assert_eq!(
            serde_json::from_str::<Value>(&request.body_text()).unwrap(),
            json!({ "types": ["AssetsV2", "AssetExifsV1"] })
        );
    }

    #[tokio::test]
    async fn api_keys_are_refused() {
        let body = r#"{"message":"Sync endpoints cannot be used with API keys"}"#;
        let server = FakeServer::start(vec![CannedResponse::json(403, body)]).await;
        let client = Client::new(&server.url, Some(crate::Credential::ApiKey("key".into())), "test").unwrap();
        let error = client.sync_stream(&["AssetsV2"], false).await.unwrap_err();
        assert_eq!(error, ApiError::Forbidden { missing_permission: None });
    }
}
