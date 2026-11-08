//! Tags: a tree of labels the user puts on Assets. A tag's `value` is its full path (`Travel/Iceland`), its
//! `name` the last part.
//!
//! `PUT /tags` ("upsert") takes full paths and creates every missing level, so it is the way to make a nested tag
//! in one call; `POST /tags` makes one tag under an existing parent.
//!
//! Quirks, seen on v3.3.6:
//! - deleting a tag deletes the tags below it, and takes all of them off their Assets
//! - tagging an Asset queues a background job that writes its tags into its sidecar and reads them back; a tag
//!   deleted while that job is pending comes back a moment later (as a new tag, on no Asset). Delete tags once the
//!   server's queues are idle ([`Client::queues`]), or delete again

use chrono::{DateTime, Utc};
use reqwest::Method;
use serde::{Deserialize, Serialize};

use crate::client::Client;
use crate::error::ApiError;
use crate::wire::{endpoints, BulkIdResult, BulkIds, Endpoint};

pub(crate) const ENDPOINTS: &[Endpoint] = endpoints![
    GET "/tags" "tag.read",
    GET "/tags/{id}" "tag.read",
    POST "/tags" "tag.create",
    PUT "/tags" "tag.create",
    PUT "/tags/{id}" "tag.update",
    DELETE "/tags/{id}" "tag.delete",
    PUT "/tags/assets" "tag.asset",
    PUT "/tags/{id}/assets" "tag.asset",
    DELETE "/tags/{id}/assets" "tag.asset",
];

/// A tag.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Tag {
    pub id: String,
    /// The last part of its path.
    pub name: String,
    /// Its full path, `Parent/Child`.
    pub value: String,
    /// The parent tag's id; `None` at the top level.
    pub parent_id: Option<String>,
    /// CSS colour text, when one was chosen.
    pub color: Option<String>,
    #[serde(deserialize_with = "crate::dates::serde_dates::utc")]
    pub created_at: DateTime<Utc>,
    #[serde(deserialize_with = "crate::dates::serde_dates::utc")]
    pub updated_at: DateTime<Utc>,
}

/// A new tag under an existing parent, or at the top level.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewTag {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

/// Changes to a tag; `None` leaves a field as it is.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TagChanges {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// `Some(None)` removes the colour.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<Option<String>>,
}

impl Client {
    /// Every tag of the user (`GET /tags`). Needs `tag.read`.
    pub async fn tags(&self) -> Result<Vec<Tag>, ApiError> {
        self.call(Method::GET, &["tags"], &[]).await
    }

    /// One tag (`GET /tags/{id}`). Needs `tag.read`.
    pub async fn tag(&self, id: &str) -> Result<Tag, ApiError> {
        self.call(Method::GET, &["tags", id], &[]).await
    }

    /// Create one tag (`POST /tags`). Needs `tag.create`.
    pub async fn create_tag(&self, tag: &NewTag) -> Result<Tag, ApiError> {
        self.call_with(Method::POST, &["tags"], &[], tag).await
    }

    /// The tags with these full paths, created where missing, every level included (`PUT /tags`). Needs
    /// `tag.create`.
    pub async fn upsert_tags(&self, paths: &[String]) -> Result<Vec<Tag>, ApiError> {
        #[derive(Serialize)]
        struct Body<'a> {
            tags: &'a [String],
        }
        self.call_with(Method::PUT, &["tags"], &[], &Body { tags: paths }).await
    }

    /// Rename or recolour a tag (`PUT /tags/{id}`). Needs `tag.update`.
    pub async fn update_tag(&self, id: &str, changes: &TagChanges) -> Result<Tag, ApiError> {
        self.call_with(Method::PUT, &["tags", id], &[], changes).await
    }

    /// Delete a tag and the tags below it; the Assets stay (`DELETE /tags/{id}`). Needs `tag.delete`.
    pub async fn delete_tag(&self, id: &str) -> Result<(), ApiError> {
        self.call_unit(Method::DELETE, &["tags", id], &[]).await
    }

    /// Put every one of these tags on every one of these Assets; returns how many pairs were added
    /// (`PUT /tags/assets`). Needs `tag.asset`.
    pub async fn tag_assets_with(&self, tag_ids: &[String], asset_ids: &[String]) -> Result<u64, ApiError> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Body<'a> {
            tag_ids: &'a [String],
            asset_ids: &'a [String],
        }
        #[derive(Deserialize, Default)]
        #[serde(default)]
        struct Count {
            count: u64,
        }
        let answer: Count = self
            .call_with(Method::PUT, &["tags", "assets"], &[], &Body { tag_ids, asset_ids })
            .await?;
        Ok(answer.count)
    }

    /// Put a tag on Assets, one outcome per Asset (`PUT /tags/{id}/assets`). Needs `tag.asset`.
    pub async fn tag_assets(&self, id: &str, asset_ids: &[String]) -> Result<Vec<BulkIdResult>, ApiError> {
        self.call_with(Method::PUT, &["tags", id, "assets"], &[], &BulkIds { ids: asset_ids })
            .await
    }

    /// Take a tag off Assets, one outcome per Asset (`DELETE /tags/{id}/assets`). Needs `tag.asset`.
    pub async fn untag_assets(&self, id: &str, asset_ids: &[String]) -> Result<Vec<BulkIdResult>, ApiError> {
        self.call_with(Method::DELETE, &["tags", id, "assets"], &[], &BulkIds { ids: asset_ids })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_changes_can_remove_the_colour() {
        let changes = TagChanges { color: Some(None), ..TagChanges::default() };
        assert_eq!(serde_json::to_value(&changes).unwrap(), serde_json::json!({"color": null}));
    }

    #[test]
    fn a_tag_decodes_with_null_parent() {
        let json = r#"{"id":"t","name":"Iceland","value":"Travel/Iceland","parentId":null,
            "createdAt":"2026-11-08T10:00:00.000Z","updatedAt":"2026-11-08T10:00:00.000Z"}"#;
        let tag: Tag = crate::client::decode_json(json.as_bytes(), "tags").unwrap();
        assert_eq!(tag.value, "Travel/Iceland");
        assert_eq!(tag.parent_id, None);
    }
}
