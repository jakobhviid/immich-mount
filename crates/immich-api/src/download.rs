//! Downloads of many Assets at once, as ZIP archives: first a plan that splits them into archives of a size, then
//! each archive streamed into a file.
//!
//! One original is [`Client::download_original`]; this is for "download these 300 photos" or a whole album.
//!
//! Quirks, seen on v3.3.6:
//! - the plan needs Assets, an album or a user (400 `assetIds, albumId, or userId is required` otherwise), and
//!   splits at `archive_size` bytes, or at the server's own default without one
//! - the archive is a ZIP whose entries are stored, not compressed, under the original file names; it is sent
//!   chunked, without a length, so a short archive is noticed only by reading it

use std::path::Path;

use reqwest::Method;
use serde::{Deserialize, Serialize};

use crate::client::Client;
use crate::error::ApiError;
use crate::wire::{endpoints, Endpoint};

pub(crate) const ENDPOINTS: &[Endpoint] = endpoints![
    POST "/download/info" "asset.download",
    POST "/download/archive" "asset.download",
];

/// What to download.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownloadSource {
    Assets(Vec<String>),
    /// Every Asset of an album.
    Album(String),
    /// Every Asset of a user (the credential's own, or a partner's).
    User(String),
}

/// How a download splits into archives (`DownloadResponseDto`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DownloadPlan {
    /// The size of all originals, in bytes.
    pub total_size: u64,
    pub archives: Vec<PlannedArchive>,
}

/// One archive of a plan: download it with [`Client::download_archive`].
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PlannedArchive {
    pub asset_ids: Vec<String>,
    /// The size of its originals, in bytes.
    pub size: u64,
}

impl Client {
    /// Plan a download: the Assets split into archives of at most `archive_size` bytes, or the server's default
    /// (`POST /download/info`). Needs `asset.download`.
    pub async fn download_plan(&self, source: &DownloadSource, archive_size: Option<u64>) -> Result<DownloadPlan, ApiError> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Body<'a> {
            #[serde(skip_serializing_if = "Option::is_none")]
            asset_ids: Option<&'a [String]>,
            #[serde(skip_serializing_if = "Option::is_none")]
            album_id: Option<&'a str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            user_id: Option<&'a str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            archive_size: Option<u64>,
        }
        let mut body = Body { asset_ids: None, album_id: None, user_id: None, archive_size };
        match source {
            DownloadSource::Assets(ids) => body.asset_ids = Some(ids),
            DownloadSource::Album(id) => body.album_id = Some(id),
            DownloadSource::User(id) => body.user_id = Some(id),
        }
        self.call_with(Method::POST, &["download", "info"], &[], &body).await
    }

    /// Download Assets as one ZIP archive into `destination`, as it arrives (`POST /download/archive`); returns the
    /// bytes written. With `edited`, an Asset edited in Immich comes as edited. Failures as for
    /// [`Client::download_original`]. Needs `asset.download`.
    pub async fn download_archive(&self, asset_ids: &[String], edited: bool, destination: &Path) -> Result<u64, ApiError> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Body<'a> {
            asset_ids: &'a [String],
            #[serde(skip_serializing_if = "std::ops::Not::not")]
            edited: bool,
        }
        let builder = self
            .checked_request(Method::POST, &["download", "archive"])?
            .json(&Body { asset_ids, edited });
        self.download_to(builder, destination).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_server::{CannedResponse, FakeServer};

    #[tokio::test]
    async fn a_plan_names_its_source() {
        let answer = r#"{"totalSize":30,"archives":[{"size":10,"assetIds":["a1"]},{"size":20,"assetIds":["a2"]}]}"#;
        let server = FakeServer::start(vec![CannedResponse::json(201, answer)]).await;
        let client = Client::new(&server.url, None, "test").unwrap();
        let plan = client.download_plan(&DownloadSource::Album("album-1".to_owned()), Some(15)).await.unwrap();
        assert_eq!(plan.total_size, 30);
        assert_eq!(plan.archives[1].asset_ids, ["a2"]);
        assert_eq!(server.requests()[0].body_text(), r#"{"albumId":"album-1","archiveSize":15}"#);
    }

    #[tokio::test]
    async fn an_archive_is_written_to_the_file() {
        let server = FakeServer::start(vec![CannedResponse::bytes(200, "application/octet-stream", b"PK\x03\x04zip")]).await;
        let client = Client::new(&server.url, None, "test").unwrap();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("archive.zip");
        let written = client.download_archive(&["a1".to_owned()], false, &path).await.unwrap();
        assert_eq!(written, 7);
        assert_eq!(std::fs::read(&path).unwrap(), b"PK\x03\x04zip");
        assert_eq!(server.requests()[0].body_text(), r#"{"assetIds":["a1"]}"#);
    }
}
