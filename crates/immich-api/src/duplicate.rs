//! Duplicates: groups of Assets that look alike, found by duplicate detection, and resolving them.
//!
//! Detection needs machine learning; without it there are no groups, unless an Asset is put into one by hand
//! ([`crate::AssetChanges::duplicate_id`]), which is how the contract tests make one.
//!
//! Quirks, seen on v3.3.6:
//! - dismissing a group (`DELETE /duplicates/{id}`, or several with `DELETE /duplicates`, whose ids are group ids)
//!   takes its Assets out of the group and keeps them all
//! - resolving moves the Assets named to trash to the trash (not deleted for good) and takes every Asset of the
//!   group out of it; it answers one outcome per group

use reqwest::Method;
use serde::{Deserialize, Serialize};

use crate::asset_info::AssetDetails;
use crate::client::Client;
use crate::error::ApiError;
use crate::wire::{endpoints, BulkIdResult, BulkIds, Endpoint};

pub(crate) const ENDPOINTS: &[Endpoint] = endpoints![
    GET "/duplicates" "duplicate.read",
    DELETE "/duplicates/{id}" "duplicate.delete",
    DELETE "/duplicates" "duplicate.delete",
    POST "/duplicates/resolve" "duplicate.delete",
];

/// A group of Assets that look alike.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DuplicateGroup {
    pub duplicate_id: String,
    pub assets: Vec<AssetDetails>,
    /// The Assets the server suggests keeping (the largest, usually).
    pub suggested_keep_asset_ids: Vec<String>,
}

/// How to resolve one group: which Assets to keep and which to move to the trash.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateResolution {
    pub duplicate_id: String,
    pub keep_asset_ids: Vec<String>,
    pub trash_asset_ids: Vec<String>,
}

impl Client {
    /// Every group of duplicates of the user (`GET /duplicates`). Needs `duplicate.read`.
    pub async fn duplicates(&self) -> Result<Vec<DuplicateGroup>, ApiError> {
        self.call(Method::GET, &["duplicates"], &[]).await
    }

    /// Dismiss a group: its Assets are no longer duplicates, and all stay (`DELETE /duplicates/{id}`). Needs
    /// `duplicate.delete`.
    pub async fn dismiss_duplicate_group(&self, duplicate_id: &str) -> Result<(), ApiError> {
        self.call_unit(Method::DELETE, &["duplicates", duplicate_id], &[]).await
    }

    /// Dismiss several groups (`DELETE /duplicates`). Needs `duplicate.delete`.
    pub async fn dismiss_duplicate_groups(&self, duplicate_ids: &[String]) -> Result<(), ApiError> {
        self.call_unit_with(Method::DELETE, &["duplicates"], &[], &BulkIds { ids: duplicate_ids })
            .await
    }

    /// Resolve groups, one outcome per group (`POST /duplicates/resolve`). Needs `duplicate.delete`.
    pub async fn resolve_duplicates(&self, resolutions: &[DuplicateResolution]) -> Result<Vec<BulkIdResult>, ApiError> {
        #[derive(Serialize)]
        struct Body<'a> {
            groups: &'a [DuplicateResolution],
        }
        self.call_with(Method::POST, &["duplicates", "resolve"], &[], &Body { groups: resolutions })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_server::{CannedResponse, FakeServer};

    #[tokio::test]
    async fn resolving_sends_each_group() {
        let server = FakeServer::start(vec![CannedResponse::json(200, r#"[{"id":"g1","success":true}]"#)]).await;
        let client = Client::new(&server.url, None, "test").unwrap();
        let resolution = DuplicateResolution {
            duplicate_id: "g1".to_owned(),
            keep_asset_ids: vec!["a1".to_owned()],
            trash_asset_ids: vec!["a2".to_owned()],
        };
        let results = client.resolve_duplicates(&[resolution]).await.unwrap();
        assert!(results[0].success);
        assert_eq!(
            server.requests()[0].body_text(),
            r#"{"groups":[{"duplicateId":"g1","keepAssetIds":["a1"],"trashAssetIds":["a2"]}]}"#
        );
    }

    #[test]
    fn a_group_decodes() {
        let json = r#"[{"duplicateId":"g1","assets":[{"id":"a1","type":"IMAGE"},{"id":"a2","type":"IMAGE"}],
            "suggestedKeepAssetIds":["a1"]}]"#;
        let groups: Vec<DuplicateGroup> = crate::client::decode_json(json.as_bytes(), "duplicates").unwrap();
        assert_eq!(groups[0].assets.len(), 2);
        assert_eq!(groups[0].suggested_keep_asset_ids, ["a1"]);
    }
}
