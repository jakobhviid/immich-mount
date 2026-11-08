//! Shared links: public links to an album or to chosen Assets, optionally with a password, an expiry date, a
//! custom slug, and permission to download or upload.
//!
//! A link is addressed in two ways: by its `id` for its owner (list, edit, delete), and by its `key` or `slug` for
//! whoever holds the link ([`Client::shared_link_by_key`], which needs no credential). A link with a password
//! answers 401 to the key alone until [`Client::shared_link_login`] is called.
//!
//! Quirks, seen on v3.3.6:
//! - adding an Asset to a link answers with `assetId` per item, not `id` as other bulk answers do
//! - only links of type `INDIVIDUAL` take Assets; an album link follows its album
//! - the answer to creating an album link has `album: null`; [`Client::create_shared_link`] reads it back

use chrono::{DateTime, Utc};
use reqwest::Method;
use serde::{Deserialize, Serialize};

use crate::album::Album;
use crate::asset_info::AssetDetails;
use crate::client::Client;
use crate::dates::serde_dates;
use crate::error::ApiError;
use crate::wire::{endpoints, wire_enum, BulkIdError, Endpoint};

pub(crate) const ENDPOINTS: &[Endpoint] = endpoints![
    GET "/shared-links" "sharedLink.read",
    GET "/shared-links/{id}" "sharedLink.read",
    GET "/shared-links/me" -,
    POST "/shared-links" "sharedLink.create",
    PATCH "/shared-links/{id}" "sharedLink.update",
    DELETE "/shared-links/{id}" "sharedLink.delete",
    PUT "/shared-links/{id}/assets" "sharedLink.update",
    DELETE "/shared-links/{id}/assets" "sharedLink.update",
    POST "/shared-links/login" -,
];

wire_enum! {
    /// What a shared link points at.
    pub enum SharedLinkKind {
        /// An album, and whatever it holds.
        Album = "ALBUM",
        /// Chosen Assets.
        Individual = "INDIVIDUAL",
    }
}

/// A shared link (`SharedLinkResponseDto`).
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SharedLink {
    pub id: String,
    /// The secret part of the link's URL (`/share/<key>`).
    pub key: String,
    /// A readable alternative to the key (`/s/<slug>`), when one was chosen.
    pub slug: Option<String>,
    #[serde(rename = "type")]
    pub kind: SharedLinkKind,
    /// The owner's user id.
    pub user_id: String,
    pub description: Option<String>,
    /// Sent to the owner only.
    pub password: Option<String>,
    #[serde(deserialize_with = "serde_dates::optional_utc")]
    pub expires_at: Option<DateTime<Utc>>,
    #[serde(deserialize_with = "serde_dates::utc")]
    pub created_at: DateTime<Utc>,
    pub allow_download: bool,
    pub allow_upload: bool,
    /// Whether viewers see camera and place details.
    pub show_metadata: bool,
    /// For an album link.
    pub album: Option<Album>,
    /// For a link to chosen Assets.
    pub assets: Vec<AssetDetails>,
}

/// A new shared link: to an album (`album_id`) or to chosen Assets (`asset_ids`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewSharedLink {
    #[serde(rename = "type")]
    pub kind: SharedLinkKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub album_id: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub asset_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,
    #[serde(serialize_with = "serde_dates::send_utc", skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
    pub allow_download: bool,
    pub allow_upload: bool,
    pub show_metadata: bool,
}

impl NewSharedLink {
    /// A link to an album, viewers allowed to download and see details, as Immich's web app makes it.
    pub fn to_album(album_id: &str) -> NewSharedLink {
        NewSharedLink {
            kind: SharedLinkKind::Album,
            album_id: Some(album_id.to_owned()),
            asset_ids: Vec::new(),
            description: None,
            password: None,
            slug: None,
            expires_at: None,
            allow_download: true,
            allow_upload: false,
            show_metadata: true,
        }
    }

    /// A link to chosen Assets, with the same defaults.
    pub fn to_assets(asset_ids: &[String]) -> NewSharedLink {
        NewSharedLink {
            kind: SharedLinkKind::Individual,
            album_id: None,
            asset_ids: asset_ids.to_vec(),
            ..NewSharedLink::to_album("")
        }
    }
}

/// Changes to a shared link; `None` leaves a field as it is.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedLinkChanges {
    /// `Some(None)` removes the description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Option<String>>,
    /// `Some(None)` removes the password.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<Option<String>>,
    /// `Some(None)` removes the slug.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slug: Option<Option<String>>,
    /// `Some(None)` makes the link last for ever.
    #[serde(serialize_with = "send_change_of_date", skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<Option<DateTime<Utc>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_download: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_upload: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_metadata: Option<bool>,
}

fn send_change_of_date<S: serde::Serializer>(
    value: &Option<Option<DateTime<Utc>>>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serde_dates::send_utc(&value.flatten(), serializer)
}

/// How whoever holds a link names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkAddress {
    /// The secret key of `/share/<key>`.
    Key(String),
    /// The slug of `/s/<slug>`.
    Slug(String),
}

impl LinkAddress {
    fn query(&self) -> [(&str, &str); 1] {
        match self {
            LinkAddress::Key(key) => [("key", key.as_str())],
            LinkAddress::Slug(slug) => [("slug", slug.as_str())],
        }
    }
}

/// The outcome for one Asset added to or removed from a link.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AssetIdResult {
    pub asset_id: String,
    pub success: bool,
    pub error: Option<BulkIdError>,
}

impl Client {
    /// The user's shared links, optionally only those of one album (`GET /shared-links`). Needs `sharedLink.read`.
    pub async fn shared_links(&self, album_id: Option<&str>) -> Result<Vec<SharedLink>, ApiError> {
        let query: Vec<(&str, &str)> = album_id.map(|album| ("albumId", album)).into_iter().collect();
        self.call(Method::GET, &["shared-links"], &query).await
    }

    /// One of the user's shared links (`GET /shared-links/{id}`). Needs `sharedLink.read`.
    pub async fn shared_link(&self, id: &str) -> Result<SharedLink, ApiError> {
        self.call(Method::GET, &["shared-links", id], &[]).await
    }

    /// A link as whoever holds it sees it, by key or slug (`GET /shared-links/me`). Needs no credential; a link
    /// with a password answers [`ApiError::Unauthorized`] until [`Client::shared_link_login`].
    pub async fn shared_link_by_key(&self, address: &LinkAddress) -> Result<SharedLink, ApiError> {
        self.call(Method::GET, &["shared-links", "me"], &address.query()).await
    }

    /// Open a link that has a password (`POST /shared-links/login`). The server answers with the link and a cookie
    /// for the browser; this client keeps no cookies. Needs no credential.
    pub async fn shared_link_login(&self, address: &LinkAddress, password: &str) -> Result<SharedLink, ApiError> {
        #[derive(Serialize)]
        struct Body<'a> {
            password: &'a str,
        }
        self.call_with(Method::POST, &["shared-links", "login"], &address.query(), &Body { password })
            .await
    }

    /// Create a shared link (`POST /shared-links`), and get it back whole. Needs `sharedLink.create`.
    pub async fn create_shared_link(&self, link: &NewSharedLink) -> Result<SharedLink, ApiError> {
        let created: SharedLink = self.call_with(Method::POST, &["shared-links"], &[], link).await?;
        if created.kind == SharedLinkKind::Album && created.album.is_none() {
            // The answer to creating an album link leaves the album out; reading the link back has it.
            return self.shared_link(&created.id).await;
        }
        Ok(created)
    }

    /// Change a shared link (`PATCH /shared-links/{id}`). Needs `sharedLink.update`.
    pub async fn update_shared_link(&self, id: &str, changes: &SharedLinkChanges) -> Result<SharedLink, ApiError> {
        self.call_with(Method::PATCH, &["shared-links", id], &[], changes).await
    }

    /// Delete a shared link; whoever holds it loses access (`DELETE /shared-links/{id}`). Needs `sharedLink.delete`.
    pub async fn delete_shared_link(&self, id: &str) -> Result<(), ApiError> {
        self.call_unit(Method::DELETE, &["shared-links", id], &[]).await
    }

    /// Add Assets to a link of chosen Assets (`PUT /shared-links/{id}/assets`). Needs `sharedLink.update`.
    pub async fn add_assets_to_shared_link(&self, id: &str, asset_ids: &[String]) -> Result<Vec<AssetIdResult>, ApiError> {
        self.call_with(Method::PUT, &["shared-links", id, "assets"], &[], &AssetIds { asset_ids })
            .await
    }

    /// Remove Assets from a link of chosen Assets (`DELETE /shared-links/{id}/assets`). Needs `sharedLink.update`.
    pub async fn remove_assets_from_shared_link(
        &self,
        id: &str,
        asset_ids: &[String],
    ) -> Result<Vec<AssetIdResult>, ApiError> {
        self.call_with(Method::DELETE, &["shared-links", id, "assets"], &[], &AssetIds { asset_ids })
            .await
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AssetIds<'a> {
    asset_ids: &'a [String],
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_link_sends_its_kind_and_target() {
        let link = NewSharedLink::to_assets(&["a".to_owned()]);
        assert_eq!(
            serde_json::to_value(&link).unwrap(),
            serde_json::json!({"type": "INDIVIDUAL", "assetIds": ["a"], "allowDownload": true,
                "allowUpload": false, "showMetadata": true})
        );
    }

    #[test]
    fn changes_can_remove_the_password_and_expiry() {
        let changes = SharedLinkChanges { password: Some(None), expires_at: Some(None), ..SharedLinkChanges::default() };
        assert_eq!(
            serde_json::to_value(&changes).unwrap(),
            serde_json::json!({"password": null, "expiresAt": null})
        );
    }
}
