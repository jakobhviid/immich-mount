//! Uploads: the SHA-1 checksum, the duplicate check, and the streamed multipart upload.
//!
//! The sequence the mount runs (ARCHITECTURE.md, "Uploads"): SHA-1 over the staging file, then
//! [`Client::bulk_upload_check`] (a trashed duplicate fails there, before any bytes move), then
//! [`Client::upload_asset`]. Immich identifies an Asset by its checksum, so an upload retried after a lost
//! answer comes back as a duplicate of the first, never as a second Asset; that is why uploads are retried.
//!
//! Wire details (seen on v3.3.5; adapted from immich-rs `upload.rs`):
//! - required multipart fields are `assetData`, `fileCreatedAt` and `fileModifiedAt`; the text fields go first
//! - the `filename` text field carries the real UTF-8 name, because a multipart part's own file name does not
//!   survive non-ASCII characters reliably; the server lets the field override the part
//! - `deviceAssetId` and `deviceId` are ignored by v3.3.5 and sent anyway, for older servers
//! - the file is streamed with its length, never read into memory; the checksum goes in `x-immich-checksum`
//! - 201 `created` is a new Asset; 200 `duplicate` returns the existing id, and if that Asset is in the trash it
//!   **stays** trashed, so the outcome says so
//! - checksums are accepted as hex or base64; Assets carry base64

use std::path::Path;

use base64::Engine;
use bytes::Bytes;
use chrono::{DateTime, Utc};
use reqwest::multipart::{Form, Part};
use reqwest::{Method, Request, StatusCode};
use serde::Deserialize;
use serde_json::json;
use sha1::{Digest, Sha1};
use tokio::io::AsyncReadExt;

use crate::asset::non_blank;
use crate::client::{check, read_json, Client};
use crate::dates::format_utc;
use crate::error::{from_transport, ApiError, RequestShape};

/// How much of the file is read per step, both for hashing and for streaming.
const CHUNK_SIZE: usize = 256 * 1024;

/// What `deviceId` says on servers that still read it.
const DEVICE_ID: &str = "immich-mount";

/// A SHA-1 digest, as Immich uses to recognise identical files.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Sha1Checksum([u8; 20]);

impl std::fmt::Debug for Sha1Checksum {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "Sha1Checksum({})", self.to_hex())
    }
}

impl Sha1Checksum {
    /// The checksum of some bytes.
    pub fn of_bytes(bytes: &[u8]) -> Sha1Checksum {
        Sha1Checksum(Sha1::digest(bytes).into())
    }

    /// The checksum of a file, read in chunks.
    pub async fn of_file(path: &Path) -> Result<Sha1Checksum, ApiError> {
        let mut file = tokio::fs::File::open(path)
            .await
            .map_err(|error| local_file_error(path, &error))?;
        let mut hasher = Sha1::new();
        let mut buffer = vec![0u8; CHUNK_SIZE];
        loop {
            let read = file
                .read(&mut buffer)
                .await
                .map_err(|error| local_file_error(path, &error))?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
        Ok(Sha1Checksum(hasher.finalize().into()))
    }

    /// The form Immich reports on Assets: base64, 28 characters.
    pub fn to_base64(&self) -> String {
        base64::engine::general_purpose::STANDARD.encode(self.0)
    }

    /// Lowercase hex, 40 characters.
    pub fn to_hex(&self) -> String {
        self.0.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    /// The raw digest.
    pub fn as_bytes(&self) -> &[u8; 20] {
        &self.0
    }
}

fn local_file_error(path: &Path, error: &std::io::Error) -> ApiError {
    ApiError::LocalFile(format!("{}: {error}", path.display()))
}

/// How an upload ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UploadOutcome {
    /// A new Asset, with its id.
    Created(String),
    /// The library already had these bytes ("Already in library"). `trashed` is true when that Asset is in the
    /// trash, where it stays: the user must restore it to see it.
    Duplicate { id: String, trashed: bool },
}

/// What the server would do with a file, judged by its checksum alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UploadCheck {
    /// Not in the library yet: upload it.
    Accept,
    /// Already in the library, possibly in the trash.
    Duplicate { asset_id: String, trashed: bool },
    /// Refused for another reason, verbatim (`unsupported-format`, or something newer).
    Reject { reason: String },
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct UploadAnswerWire {
    id: Option<String>,
    status: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct BulkCheckAnswerWire {
    results: Vec<BulkCheckResultWire>,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct BulkCheckResultWire {
    id: Option<String>,
    action: Option<String>,
    reason: Option<String>,
    asset_id: Option<String>,
    is_trashed: Option<bool>,
}

impl BulkCheckResultWire {
    fn into_check(self) -> Result<UploadCheck, ApiError> {
        match (self.action.as_deref(), self.reason.as_deref()) {
            (Some("accept"), _) => Ok(UploadCheck::Accept),
            (Some("reject"), Some("duplicate")) => Ok(UploadCheck::Duplicate {
                asset_id: non_blank(self.asset_id).ok_or_else(|| {
                    ApiError::Protocol("a duplicate in the upload check without an asset id".to_owned())
                })?,
                trashed: self.is_trashed.unwrap_or(false),
            }),
            (Some("reject"), reason) => Ok(UploadCheck::Reject {
                reason: reason.unwrap_or("unknown").to_owned(),
            }),
            (action, _) => Err(ApiError::Protocol(format!(
                "an upload check result with action {action:?}"
            ))),
        }
    }
}

/// Match bulk-check results to the checksums sent, by the index we used as each item's id.
fn into_checks(answer: BulkCheckAnswerWire, sent: usize) -> Result<Vec<UploadCheck>, ApiError> {
    let mut checks: Vec<Option<UploadCheck>> = vec![None; sent];
    for result in answer.results {
        let index = result
            .id
            .as_deref()
            .and_then(|id| id.parse::<usize>().ok())
            .filter(|index| *index < sent)
            .ok_or_else(|| ApiError::Protocol(format!("an upload check result for unknown item {:?}", result.id)))?;
        checks[index] = Some(result.into_check()?);
    }
    checks
        .into_iter()
        .enumerate()
        .map(|(index, check)| {
            check.ok_or_else(|| ApiError::Protocol(format!("the upload check left out item {index}")))
        })
        .collect()
}

pub(crate) const ENDPOINTS: &[crate::wire::Endpoint] = crate::wire::endpoints![
    POST "/assets/bulk-upload-check" "asset.upload",
    POST "/assets" "asset.upload",
];

impl Client {
    /// Ask whether files with these checksums (hex or base64) are already in the library
    /// (`POST /assets/bulk-upload-check`). Results come back in the order given. Needs `asset.upload`.
    pub async fn bulk_upload_check(&self, checksums: &[String]) -> Result<Vec<UploadCheck>, ApiError> {
        if checksums.is_empty() {
            return Ok(Vec::new());
        }
        let items: Vec<_> = checksums
            .iter()
            .enumerate()
            .map(|(index, checksum)| json!({ "id": index.to_string(), "checksum": checksum }))
            .collect();
        let answer: BulkCheckAnswerWire = self
            .post_json(&["assets", "bulk-upload-check"], &json!({ "assets": items }))
            .await?;
        into_checks(answer, checksums.len())
    }

    /// Upload the file at `path` as a new Asset named `file_name` (`POST /assets`, streamed multipart).
    ///
    /// `file_created_at` is the capture date (the server re-reads EXIF anyway); `file_modified_at` becomes the
    /// original's `Last-Modified`. No request timeout applies; bound the call. Needs `asset.upload`.
    ///
    /// A retried attempt whose first try did reach the server comes back as [`UploadOutcome::Duplicate`] of the
    /// Asset that try created. A caller that checked with [`Client::bulk_upload_check`] first (and got
    /// `Accept`) can read a non-trashed duplicate as its own upload.
    pub async fn upload_asset(
        &self,
        path: &Path,
        file_name: &str,
        file_created_at: DateTime<Utc>,
        file_modified_at: DateTime<Utc>,
    ) -> Result<UploadOutcome, ApiError> {
        let checksum = Sha1Checksum::of_file(path).await?;
        let created = format_utc(&file_created_at);
        let modified = format_utc(&file_modified_at);
        let response = self
            .execute_with(|| self.upload_request(path, file_name, &created, &modified, &checksum))
            .await?;
        let response = check(response, RequestShape::Plain).await?;
        let status = response.status();
        let answer: UploadAnswerWire = read_json(response, "assets").await?;
        let id = non_blank(answer.id).ok_or_else(|| ApiError::Protocol("an upload answer without an id".to_owned()))?;
        match (status, answer.status.as_deref()) {
            (_, Some("duplicate")) => {
                let trashed = self.is_trashed_duplicate(&checksum).await?;
                Ok(UploadOutcome::Duplicate { id, trashed })
            }
            (StatusCode::CREATED, _) | (_, Some("created")) => Ok(UploadOutcome::Created(id)),
            (status, other) => Err(ApiError::Protocol(format!(
                "an upload answered HTTP {} with status {other:?}",
                status.as_u16()
            ))),
        }
    }

    /// Whether the Asset with this checksum is in the trash, from the bulk check (same permission as uploading).
    async fn is_trashed_duplicate(&self, checksum: &Sha1Checksum) -> Result<bool, ApiError> {
        let checks = self.bulk_upload_check(&[checksum.to_base64()]).await?;
        Ok(matches!(checks.first(), Some(UploadCheck::Duplicate { trashed: true, .. })))
    }

    /// One attempt's upload request, with the file opened afresh so a retry streams it from the start.
    async fn upload_request(
        &self,
        path: &Path,
        file_name: &str,
        file_created_at: &str,
        file_modified_at: &str,
        checksum: &Sha1Checksum,
    ) -> Result<Request, ApiError> {
        let file = tokio::fs::File::open(path)
            .await
            .map_err(|error| local_file_error(path, &error))?;
        let length = file
            .metadata()
            .await
            .map_err(|error| local_file_error(path, &error))?
            .len();
        let chunks = futures_util::stream::try_unfold(file, |mut file| async move {
            let mut buffer = vec![0u8; CHUNK_SIZE];
            let read = file.read(&mut buffer).await?;
            if read == 0 {
                return Ok::<_, std::io::Error>(None);
            }
            buffer.truncate(read);
            Ok(Some((Bytes::from(buffer), file)))
        });
        let part = Part::stream_with_length(reqwest::Body::wrap_stream(chunks), length)
            .file_name(file_name.to_owned())
            .mime_str("application/octet-stream")
            .map_err(|error| from_transport(&error))?;
        let form = Form::new()
            .text("deviceAssetId", format!("{file_name}-{length}"))
            .text("deviceId", DEVICE_ID)
            .text("fileCreatedAt", file_created_at.to_owned())
            .text("fileModifiedAt", file_modified_at.to_owned())
            .text("filename", file_name.to_owned())
            .part("assetData", part);
        self.request(Method::POST, &["assets"])
            .header("x-immich-checksum", checksum.to_base64())
            .multipart(form)
            .build()
            .map_err(|error| from_transport(&error))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksums_in_both_encodings() {
        let checksum = Sha1Checksum::of_bytes(b"abc");
        assert_eq!(checksum.to_hex(), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(checksum.to_base64(), "qZk+NkcGgWq6PiVxeFDCbJzQ2J0=");
        assert_eq!(checksum.to_base64().len(), 28);
    }

    #[tokio::test]
    async fn file_checksum_matches_bytes_checksum() {
        let path = std::env::temp_dir().join(format!("immich-api-checksum-{}", std::process::id()));
        let content: Vec<u8> = (0..(CHUNK_SIZE * 2 + 17)).map(|index| (index % 251) as u8).collect();
        tokio::fs::write(&path, &content).await.unwrap();
        let from_file = Sha1Checksum::of_file(&path).await.unwrap();
        tokio::fs::remove_file(&path).await.unwrap();
        assert_eq!(from_file, Sha1Checksum::of_bytes(&content));
    }

    #[tokio::test]
    async fn a_missing_file_is_a_local_file_error() {
        let error = Sha1Checksum::of_file(Path::new("/nonexistent/immich-api/x.jpg"))
            .await
            .unwrap_err();
        assert!(matches!(error, ApiError::LocalFile(_)), "{error:?}");
    }

    #[test]
    fn bulk_check_results_map_back_in_order() {
        let answer = r#"{"results":[
            {"id":"1","action":"reject","reason":"duplicate","assetId":"a-trashed","isTrashed":true},
            {"id":"0","action":"accept"},
            {"id":"2","action":"reject","reason":"duplicate","assetId":"a-live","isTrashed":false},
            {"id":"3","action":"reject","reason":"unsupported-format"}]}"#;
        let checks = into_checks(serde_json::from_str(answer).unwrap(), 4).unwrap();
        assert_eq!(
            checks,
            [
                UploadCheck::Accept,
                UploadCheck::Duplicate {
                    asset_id: "a-trashed".to_owned(),
                    trashed: true
                },
                UploadCheck::Duplicate {
                    asset_id: "a-live".to_owned(),
                    trashed: false
                },
                UploadCheck::Reject {
                    reason: "unsupported-format".to_owned()
                },
            ]
        );
    }

    #[test]
    fn bulk_check_with_a_missing_result_is_an_error() {
        let answer = r#"{"results":[{"id":"0","action":"accept"}]}"#;
        assert!(matches!(
            into_checks(serde_json::from_str(answer).unwrap(), 2),
            Err(ApiError::Protocol(_))
        ));
    }
}
