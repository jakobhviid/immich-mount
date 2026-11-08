//! Albums: listing, reading, creating and editing them, their Assets and their members.
//!
//! `GET /albums` without `isOwned`/`isShared` returns both the user's own albums and those shared with them.
//! No album answer has an asset list (seen on v3.3.5): an album's contents come from search by `albumIds`, or
//! from its timeline buckets. `assetCount` counts archived members; `updatedAt` changes when Assets are added,
//! which makes it, with the list's ETag, the cheap change signal for a scoped API key.
//!
//! Quirks, seen on v3.3.6:
//! - an album has no owner field; the owner is the member whose role is `owner`
//! - adding an Asset that is already in the album is not an error for the call: its [`BulkIdResult`] says
//!   `duplicate`
//! - `DELETE /albums/{id}/user/me` is how a member leaves an album shared with them

use chrono::{DateTime, Utc};
use reqwest::Method;
use serde::{Deserialize, Serialize};

use crate::asset::non_blank;
use crate::client::{Client, Fetched};
use crate::dates::{optional_utc, parse_utc};
use crate::error::ApiError;
use crate::map::MapMarker;
use crate::user::User;
use crate::wire::{endpoints, wire_enum, BulkIdError, BulkIdResult, BulkIds, Endpoint};

pub(crate) const ENDPOINTS: &[Endpoint] = endpoints![
    GET "/albums" "album.read",
    GET "/albums/{id}" "album.read",
    POST "/albums" "album.create",
    PATCH "/albums/{id}" "album.update",
    DELETE "/albums/{id}" "album.delete",
    PUT "/albums/{id}/assets" "albumAsset.create",
    DELETE "/albums/{id}/assets" "albumAsset.delete",
    PUT "/albums/assets" "albumAsset.create",
    PUT "/albums/{id}/users" "albumUser.create",
    PUT "/albums/{id}/user/{userId}" "albumUser.update",
    DELETE "/albums/{id}/user/{userId}" "albumUser.delete",
    GET "/albums/statistics" "album.statistics",
    GET "/albums/{id}/map-markers" "album.read",
];

wire_enum! {
    /// What a member may do in an album.
    pub enum AlbumRole {
        Owner = "owner",
        Editor = "editor",
        Viewer = "viewer",
    }
}

wire_enum! {
    /// Which way an album, or a search, sorts by capture date.
    pub enum AssetOrder {
        Desc = "desc",
        Asc = "asc",
    }
}

/// A member of an album: its owner, or a user it is shared with.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AlbumMember {
    pub role: AlbumRole,
    pub user: User,
}

/// An album.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Album {
    pub id: String,
    pub name: String,
    /// Empty when the album has none.
    pub description: String,
    /// The capture date of its oldest Asset; `None` for an empty album.
    pub start_date: Option<DateTime<Utc>>,
    /// The capture date of its newest Asset; `None` for an empty album.
    pub end_date: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    /// Changes when the album is edited or Assets are added.
    pub updated_at: DateTime<Utc>,
    pub asset_count: u64,
    /// Whether the album is shared with other users (in either direction).
    pub shared: bool,
    /// The Asset whose thumbnail stands for the album; `None` for an empty album.
    pub thumbnail_asset_id: Option<String>,
    /// The owner and everyone it is shared with. The owner's role is [`AlbumRole::Owner`].
    pub members: Vec<AlbumMember>,
    /// Whether comments and likes are on.
    pub activity_enabled: bool,
    /// Whether a shared link points at the album.
    pub has_shared_link: bool,
    /// The sort order chosen for the album, if any.
    pub order: Option<AssetOrder>,
}

impl Album {
    /// The member who owns the album, when the server listed them.
    pub fn owner(&self) -> Option<&User> {
        self.members
            .iter()
            .find(|member| member.role == AlbumRole::Owner)
            .map(|member| &member.user)
    }
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct AlbumWire {
    id: Option<String>,
    album_name: Option<String>,
    description: Option<String>,
    start_date: Option<String>,
    end_date: Option<String>,
    created_at: Option<String>,
    updated_at: Option<String>,
    asset_count: Option<u64>,
    shared: Option<bool>,
    album_thumbnail_asset_id: Option<String>,
    album_users: Vec<AlbumMember>,
    is_activity_enabled: Option<bool>,
    has_shared_link: Option<bool>,
    order: Option<AssetOrder>,
}

impl AlbumWire {
    fn into_album(self) -> Result<Album, ApiError> {
        let id = non_blank(self.id).ok_or_else(|| ApiError::Protocol("an album without an id".to_owned()))?;
        let created_at = self
            .created_at
            .as_deref()
            .and_then(parse_utc)
            .ok_or_else(|| ApiError::Protocol(format!("album {id} has no usable createdAt")))?;
        Ok(Album {
            name: self.album_name.unwrap_or_default(),
            description: self.description.unwrap_or_default(),
            start_date: optional_utc(self.start_date.as_deref()),
            end_date: optional_utc(self.end_date.as_deref()),
            updated_at: optional_utc(self.updated_at.as_deref()).unwrap_or(created_at),
            created_at,
            asset_count: self.asset_count.unwrap_or(0),
            shared: self.shared.unwrap_or(false),
            thumbnail_asset_id: non_blank(self.album_thumbnail_asset_id),
            members: self.album_users,
            activity_enabled: self.is_activity_enabled.unwrap_or(true),
            has_shared_link: self.has_shared_link.unwrap_or(false),
            order: self.order,
            id,
        })
    }
}

fn into_albums(wire: Vec<AlbumWire>) -> Result<Vec<Album>, ApiError> {
    wire.into_iter().map(AlbumWire::into_album).collect()
}

/// Which albums [`Client::albums_where`] lists. The default lists every album, as [`Client::albums`] does.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AlbumFilter {
    /// Only the albums that hold this Asset.
    pub containing_asset: Option<String>,
    /// `Some(true)`: only shared albums (either direction); `Some(false)`: only albums shared with nobody.
    pub shared: Option<bool>,
    /// `Some(true)`: only the user's own albums.
    pub owned: Option<bool>,
}

/// A new album.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewAlbum {
    #[serde(rename = "albumName")]
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Assets to put in it at once.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub asset_ids: Vec<String>,
    /// Users to share it with at once.
    #[serde(rename = "albumUsers", skip_serializing_if = "Vec::is_empty")]
    pub members: Vec<NewMember>,
}

/// A user to share an album with, and their role.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewMember {
    pub user_id: String,
    pub role: AlbumRole,
}

/// Changes to an album; `None` leaves a field as it is.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlbumChanges {
    #[serde(rename = "albumName", skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// An empty description removes it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The Asset whose thumbnail stands for the album; it must be in the album.
    #[serde(rename = "albumThumbnailAssetId", skip_serializing_if = "Option::is_none")]
    pub thumbnail_asset_id: Option<String>,
    #[serde(rename = "isActivityEnabled", skip_serializing_if = "Option::is_none")]
    pub activity_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order: Option<AssetOrder>,
}

/// How many albums the user has (`GET /albums/statistics`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AlbumStatistics {
    pub owned: u64,
    pub shared: u64,
    pub not_shared: u64,
}

/// The outcome of adding Assets to several albums at once (`PUT /albums/assets`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BulkAlbumOutcome {
    pub success: bool,
    pub error: Option<BulkIdError>,
}

impl Client {
    /// Every album the credential can see, owned and shared (`GET /albums`). Needs `album.read`.
    pub async fn albums(&self) -> Result<Vec<Album>, ApiError> {
        let wire: Vec<AlbumWire> = self.get_json(&["albums"], &[]).await?;
        into_albums(wire)
    }

    /// [`Client::albums`], unless nothing changed since `etag` (Immich answers 304 then).
    pub async fn albums_if_changed(&self, etag: Option<&str>) -> Result<Fetched<Vec<Album>>, ApiError> {
        let fetched: Fetched<Vec<AlbumWire>> = self.get_json_if_changed(&["albums"], &[], etag).await?;
        match fetched {
            Fetched::NotModified => Ok(Fetched::NotModified),
            Fetched::Changed { value, etag } => Ok(Fetched::Changed {
                value: into_albums(value)?,
                etag,
            }),
        }
    }

    /// The albums that match a filter (`GET /albums` with `assetId`, `isShared`, `isOwned`). Needs `album.read`.
    pub async fn albums_where(&self, filter: &AlbumFilter) -> Result<Vec<Album>, ApiError> {
        let mut query: Vec<(&str, &str)> = Vec::new();
        if let Some(asset_id) = &filter.containing_asset {
            query.push(("assetId", asset_id));
        }
        if let Some(shared) = filter.shared {
            query.push(("isShared", if shared { "true" } else { "false" }));
        }
        if let Some(owned) = filter.owned {
            query.push(("isOwned", if owned { "true" } else { "false" }));
        }
        let wire: Vec<AlbumWire> = self.call(Method::GET, &["albums"], &query).await?;
        into_albums(wire)
    }

    /// One album (`GET /albums/{id}`). Needs `album.read`.
    pub async fn album(&self, id: &str) -> Result<Album, ApiError> {
        let wire: AlbumWire = self.call(Method::GET, &["albums", id], &[]).await?;
        wire.into_album()
    }

    /// Create an album (`POST /albums`). Needs `album.create`.
    pub async fn create_album(&self, album: &NewAlbum) -> Result<Album, ApiError> {
        let wire: AlbumWire = self.call_with(Method::POST, &["albums"], &[], album).await?;
        wire.into_album()
    }

    /// Change an album's name, description, cover, activity switch or order (`PATCH /albums/{id}`). Needs
    /// `album.update`.
    pub async fn update_album(&self, id: &str, changes: &AlbumChanges) -> Result<Album, ApiError> {
        let wire: AlbumWire = self.call_with(Method::PATCH, &["albums", id], &[], changes).await?;
        wire.into_album()
    }

    /// Delete an album; its Assets stay in the library (`DELETE /albums/{id}`). Needs `album.delete`.
    pub async fn delete_album(&self, id: &str) -> Result<(), ApiError> {
        self.call_unit(Method::DELETE, &["albums", id], &[]).await
    }

    /// Add Assets to an album, one outcome per id (`PUT /albums/{id}/assets`). Needs `albumAsset.create`.
    pub async fn add_assets_to_album(&self, id: &str, asset_ids: &[String]) -> Result<Vec<BulkIdResult>, ApiError> {
        self.call_with(Method::PUT, &["albums", id, "assets"], &[], &BulkIds { ids: asset_ids })
            .await
    }

    /// Remove Assets from an album, one outcome per id (`DELETE /albums/{id}/assets`). Needs `albumAsset.delete`.
    pub async fn remove_assets_from_album(&self, id: &str, asset_ids: &[String]) -> Result<Vec<BulkIdResult>, ApiError> {
        self.call_with(Method::DELETE, &["albums", id, "assets"], &[], &BulkIds { ids: asset_ids })
            .await
    }

    /// Add the same Assets to several albums (`PUT /albums/assets`). Needs `albumAsset.create`.
    pub async fn add_assets_to_albums(&self, album_ids: &[String], asset_ids: &[String]) -> Result<BulkAlbumOutcome, ApiError> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Body<'a> {
            album_ids: &'a [String],
            asset_ids: &'a [String],
        }
        self.call_with(Method::PUT, &["albums", "assets"], &[], &Body { album_ids, asset_ids })
            .await
    }

    /// Share an album with users (`PUT /albums/{id}/users`). Needs `albumUser.create`.
    pub async fn add_album_members(&self, id: &str, members: &[NewMember]) -> Result<Album, ApiError> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Body<'a> {
            album_users: &'a [NewMember],
        }
        let wire: AlbumWire = self
            .call_with(Method::PUT, &["albums", id, "users"], &[], &Body { album_users: members })
            .await?;
        wire.into_album()
    }

    /// Change a member's role (`PUT /albums/{id}/user/{userId}`). Needs `albumUser.update`.
    pub async fn update_album_member(&self, id: &str, user_id: &str, role: &AlbumRole) -> Result<(), ApiError> {
        #[derive(Serialize)]
        struct Body<'a> {
            role: &'a AlbumRole,
        }
        self.call_unit_with(Method::PUT, &["albums", id, "user", user_id], &[], &Body { role })
            .await
    }

    /// Stop sharing an album with a user, or leave it with `user_id` `"me"` (`DELETE /albums/{id}/user/{userId}`).
    /// Needs `albumUser.delete`.
    pub async fn remove_album_member(&self, id: &str, user_id: &str) -> Result<(), ApiError> {
        self.call_unit(Method::DELETE, &["albums", id, "user", user_id], &[]).await
    }

    /// How many albums the user owns and shares (`GET /albums/statistics`). Needs `album.statistics`.
    pub async fn album_statistics(&self) -> Result<AlbumStatistics, ApiError> {
        self.call(Method::GET, &["albums", "statistics"], &[]).await
    }

    /// The album's Assets that have GPS coordinates (`GET /albums/{id}/map-markers`). Needs `album.read`.
    pub async fn album_map_markers(&self, id: &str) -> Result<Vec<MapMarker>, ApiError> {
        self.call(Method::GET, &["albums", id, "map-markers"], &[]).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::ALBUMS;
    use crate::test_server::{CannedResponse, FakeServer};

    #[test]
    fn albums_decode() {
        let wire: Vec<AlbumWire> = serde_json::from_str(ALBUMS).unwrap();
        let albums = into_albums(wire).unwrap();
        assert_eq!(albums.len(), 1);
        let album = &albums[0];
        assert_eq!(album.id, "81d8ee12-2679-4d47-b903-a1b1ebc1d533");
        assert_eq!(album.name, "Persons");
        assert_eq!(album.description, "");
        assert_eq!(album.start_date, parse_utc("2014-08-15T00:00:00.000Z"));
        assert_eq!(album.end_date, parse_utc("2019-12-12T00:00:00.000Z"));
        assert_eq!(album.created_at, parse_utc("2026-11-01T21:02:04.055Z").unwrap());
        assert_eq!(album.asset_count, 3);
        assert!(!album.shared);
        assert_eq!(album.thumbnail_asset_id.as_deref(), Some("e8b4a739-4120-4af2-b22f-6c337a14119a"));
        assert_eq!(album.owner().map(|owner| owner.email.as_str()), Some("admin@example.com"));
    }

    #[test]
    fn an_empty_album_has_no_dates() {
        let json = r#"[{"id":"a","albumName":"Empty","createdAt":"2026-11-01T21:02:04.055Z","assetCount":0}]"#;
        let albums = into_albums(serde_json::from_str(json).unwrap()).unwrap();
        assert_eq!(albums[0].start_date, None);
        assert_eq!(albums[0].end_date, None);
        assert_eq!(albums[0].updated_at, albums[0].created_at);
        assert_eq!(albums[0].owner(), None);
    }

    #[test]
    fn album_changes_send_only_what_changes() {
        let changes = AlbumChanges {
            name: Some("Trip".to_owned()),
            order: Some(AssetOrder::Asc),
            ..AlbumChanges::default()
        };
        assert_eq!(
            serde_json::to_value(&changes).unwrap(),
            serde_json::json!({"albumName": "Trip", "order": "asc"})
        );
        let album = NewAlbum {
            name: "New".to_owned(),
            members: vec![NewMember { user_id: "u".to_owned(), role: AlbumRole::Viewer }],
            ..NewAlbum::default()
        };
        assert_eq!(
            serde_json::to_value(&album).unwrap(),
            serde_json::json!({"albumName": "New", "albumUsers": [{"userId": "u", "role": "viewer"}]})
        );
    }

    #[tokio::test]
    async fn adding_assets_reports_each_id() {
        let answer = r#"[{"id":"a","success":true},{"id":"b","success":false,"error":"duplicate"}]"#;
        let server = FakeServer::start(vec![CannedResponse::json(200, answer)]).await;
        let client = Client::new(&server.url, None, "test").unwrap();
        let results = client
            .add_assets_to_album("album-1", &["a".to_owned(), "b".to_owned()])
            .await
            .unwrap();
        assert!(results[0].success);
        assert_eq!(results[1].error, Some(BulkIdError::Duplicate));
        let request = &server.requests()[0];
        assert_eq!((request.method.as_str(), request.target.as_str()), ("PUT", "/api/albums/album-1/assets"));
        assert_eq!(request.body_text(), r#"{"ids":["a","b"]}"#);
    }

    #[tokio::test]
    async fn an_empty_id_is_refused_before_sending() {
        let server = FakeServer::start(vec![]).await;
        let client = Client::new(&server.url, None, "test").unwrap();
        assert!(matches!(client.delete_album("").await, Err(ApiError::Protocol(_))));
        assert!(server.requests().is_empty());
    }
}
