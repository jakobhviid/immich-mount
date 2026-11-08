//! Activity in shared albums: comments and likes, on the album or on one of its Assets.
//!
//! The album's owner can switch activity off ([`crate::AlbumChanges::activity_enabled`]); creating then answers
//! an error, and reading still works.

use chrono::{DateTime, Utc};
use reqwest::Method;
use serde::{Deserialize, Serialize};

use crate::client::Client;
use crate::dates::serde_dates;
use crate::error::ApiError;
use crate::user::User;
use crate::wire::{endpoints, wire_enum, Endpoint};

pub(crate) const ENDPOINTS: &[Endpoint] = endpoints![
    GET "/activities" "activity.read",
    POST "/activities" "activity.create",
    DELETE "/activities/{id}" "activity.delete",
    GET "/activities/statistics" "activity.statistics",
];

wire_enum! {
    /// A comment or a like.
    pub enum ActivityKind {
        Comment = "comment",
        Like = "like",
    }
}

/// A comment or a like (`ActivityResponseDto`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Activity {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: ActivityKind,
    /// The Asset it is about; `None` for the album as a whole.
    pub asset_id: Option<String>,
    /// The text of a comment.
    pub comment: Option<String>,
    pub user: User,
    #[serde(deserialize_with = "serde_dates::utc")]
    pub created_at: DateTime<Utc>,
}

/// Which activity of an album [`Client::activities`] lists.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ActivityFilter {
    /// Only the activity on this Asset.
    pub asset_id: Option<String>,
    /// Only comments, or only likes.
    pub kind: Option<ActivityKind>,
    /// Only this user's.
    pub user_id: Option<String>,
    /// Only the album's own activity (`true`) or only its Assets' (`false`).
    pub album_level: Option<bool>,
}

/// How many comments and likes (`GET /activities/statistics`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(default)]
pub struct ActivityStatistics {
    pub comments: u64,
    pub likes: u64,
}

impl Client {
    /// The comments and likes in an album, oldest first (`GET /activities`). Needs `activity.read`.
    pub async fn activities(&self, album_id: &str, filter: &ActivityFilter) -> Result<Vec<Activity>, ApiError> {
        let mut query: Vec<(&str, &str)> = vec![("albumId", album_id)];
        if let Some(asset_id) = &filter.asset_id {
            query.push(("assetId", asset_id));
        }
        if let Some(kind) = &filter.kind {
            query.push(("type", kind.as_str()));
        }
        if let Some(user_id) = &filter.user_id {
            query.push(("userId", user_id));
        }
        if let Some(album_level) = filter.album_level {
            query.push(("level", if album_level { "album" } else { "asset" }));
        }
        self.call(Method::GET, &["activities"], &query).await
    }

    /// Comment on an album, or on one of its Assets (`POST /activities`). Needs `activity.create`.
    pub async fn comment(&self, album_id: &str, asset_id: Option<&str>, text: &str) -> Result<Activity, ApiError> {
        self.create_activity(album_id, asset_id, &ActivityKind::Comment, Some(text)).await
    }

    /// Like an album, or one of its Assets (`POST /activities`). Needs `activity.create`.
    pub async fn like(&self, album_id: &str, asset_id: Option<&str>) -> Result<Activity, ApiError> {
        self.create_activity(album_id, asset_id, &ActivityKind::Like, None).await
    }

    async fn create_activity(
        &self,
        album_id: &str,
        asset_id: Option<&str>,
        kind: &ActivityKind,
        comment: Option<&str>,
    ) -> Result<Activity, ApiError> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Body<'a> {
            album_id: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            asset_id: Option<&'a str>,
            #[serde(rename = "type")]
            kind: &'a ActivityKind,
            #[serde(skip_serializing_if = "Option::is_none")]
            comment: Option<&'a str>,
        }
        let body = Body { album_id, asset_id, kind, comment };
        self.call_with(Method::POST, &["activities"], &[], &body).await
    }

    /// Delete a comment, or take back a like (`DELETE /activities/{id}`). Needs `activity.delete`.
    pub async fn delete_activity(&self, id: &str) -> Result<(), ApiError> {
        self.call_unit(Method::DELETE, &["activities", id], &[]).await
    }

    /// How many comments and likes an album, or one of its Assets, has (`GET /activities/statistics`). Needs
    /// `activity.statistics`.
    pub async fn activity_statistics(&self, album_id: &str, asset_id: Option<&str>) -> Result<ActivityStatistics, ApiError> {
        let mut query: Vec<(&str, &str)> = vec![("albumId", album_id)];
        if let Some(asset_id) = asset_id {
            query.push(("assetId", asset_id));
        }
        self.call(Method::GET, &["activities", "statistics"], &query).await
    }
}
