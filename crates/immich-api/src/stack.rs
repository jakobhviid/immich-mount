//! Stacks: Assets grouped behind one of them, the primary (a burst, or a RAW and its JPEG). The timeline shows the
//! primary; an Asset's own [`crate::AssetDetails::stack`] says which stack it is in.
//!
//! Quirks, seen on v3.3.6:
//! - a stack needs at least two Assets to be made, and the first one given becomes the primary
//! - removing Assets can leave a stack of one, which stays until it is deleted
//! - the primary cannot be removed (400 `Cannot remove stack's primary asset`); make another Asset the primary
//!   first
//! - deleting a stack keeps its Assets, unstacked
//! - `PUT /stacks/{id}` is marked deprecated with no replacement yet; it is the only way on v3.2

use reqwest::Method;
use serde::{Deserialize, Serialize};

use crate::asset_info::AssetDetails;
use crate::client::Client;
use crate::error::ApiError;
use crate::wire::{endpoints, BulkIds, Endpoint};

pub(crate) const ENDPOINTS: &[Endpoint] = endpoints![
    GET "/stacks" "stack.read",
    GET "/stacks/{id}" "stack.read",
    POST "/stacks" "stack.create",
    PUT "/stacks/{id}" "stack.update",
    DELETE "/stacks/{id}" "stack.delete",
    DELETE "/stacks" "stack.delete",
    DELETE "/stacks/{id}/assets/{assetId}" "stack.update",
];

/// A stack and its Assets.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Stack {
    pub id: String,
    /// The Asset shown for the stack.
    pub primary_asset_id: String,
    /// Every Asset in it, the primary first.
    pub assets: Vec<AssetDetails>,
}

impl Client {
    /// The user's stacks, or only the one whose primary is `primary_asset_id` (`GET /stacks`). Needs `stack.read`.
    pub async fn stacks(&self, primary_asset_id: Option<&str>) -> Result<Vec<Stack>, ApiError> {
        let query: Vec<(&str, &str)> = primary_asset_id.map(|id| ("primaryAssetId", id)).into_iter().collect();
        self.call(Method::GET, &["stacks"], &query).await
    }

    /// One stack (`GET /stacks/{id}`). Needs `stack.read`.
    pub async fn stack(&self, id: &str) -> Result<Stack, ApiError> {
        self.call(Method::GET, &["stacks", id], &[]).await
    }

    /// Stack two or more Assets; the first becomes the primary (`POST /stacks`). Needs `stack.create`.
    pub async fn create_stack(&self, asset_ids: &[String]) -> Result<Stack, ApiError> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Body<'a> {
            asset_ids: &'a [String],
        }
        self.call_with(Method::POST, &["stacks"], &[], &Body { asset_ids }).await
    }

    /// Make another Asset of the stack its primary (`PUT /stacks/{id}`). Needs `stack.update`.
    pub async fn set_stack_primary(&self, id: &str, primary_asset_id: &str) -> Result<Stack, ApiError> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Body<'a> {
            primary_asset_id: &'a str,
        }
        self.call_with(Method::PUT, &["stacks", id], &[], &Body { primary_asset_id }).await
    }

    /// Delete a stack; its Assets stay, unstacked (`DELETE /stacks/{id}`). Needs `stack.delete`.
    pub async fn delete_stack(&self, id: &str) -> Result<(), ApiError> {
        self.call_unit(Method::DELETE, &["stacks", id], &[]).await
    }

    /// Delete several stacks (`DELETE /stacks`). Needs `stack.delete`.
    pub async fn delete_stacks(&self, ids: &[String]) -> Result<(), ApiError> {
        self.call_unit_with(Method::DELETE, &["stacks"], &[], &BulkIds { ids }).await
    }

    /// Take one Asset, not the primary, out of a stack (`DELETE /stacks/{id}/assets/{assetId}`). Needs
    /// `stack.update`.
    pub async fn remove_from_stack(&self, id: &str, asset_id: &str) -> Result<(), ApiError> {
        self.call_unit(Method::DELETE, &["stacks", id, "assets", asset_id], &[]).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::SEARCH_ITEM;
    use crate::test_server::{CannedResponse, FakeServer};

    #[tokio::test]
    async fn stacks_filter_by_primary_and_decode_their_assets() {
        let answer = format!(r#"[{{"id":"s1","primaryAssetId":"12ecb329-c96f-41b1-8406-e5b8fe134454","assets":[{SEARCH_ITEM}]}}]"#);
        let server = FakeServer::start(vec![CannedResponse::json(200, &answer), CannedResponse::json(200, "[]")]).await;
        let client = Client::new(&server.url, None, "test").unwrap();
        let stacks = client.stacks(Some("12ecb329-c96f-41b1-8406-e5b8fe134454")).await.unwrap();
        assert_eq!(stacks[0].assets[0].id, stacks[0].primary_asset_id);
        assert!(client.stacks(None).await.unwrap().is_empty());
        let requests = server.requests();
        assert_eq!(requests[0].target, "/api/stacks?primaryAssetId=12ecb329-c96f-41b1-8406-e5b8fe134454");
        assert_eq!(requests[1].target, "/api/stacks");
    }

    #[tokio::test]
    async fn removing_from_a_stack_names_both_ids_in_the_path() {
        let server = FakeServer::start(vec![CannedResponse::json(204, "")]).await;
        let client = Client::new(&server.url, None, "test").unwrap();
        client.remove_from_stack("s1", "a2").await.unwrap();
        let request = &server.requests()[0];
        assert_eq!((request.method.as_str(), request.target.as_str()), ("DELETE", "/api/stacks/s1/assets/a2"));
        assert!(matches!(client.remove_from_stack("s1", "").await, Err(ApiError::Protocol(_))));
    }
}
