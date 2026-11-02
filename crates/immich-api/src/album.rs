//! Albums: the list of albums the credential can see, owned and shared.
//!
//! `GET /albums` without `isOwned`/`isShared` returns both the user's own albums and those shared with them.
//! The answer has no asset list (seen on v3.3.5): an album's contents come from search by `albumIds`, or
//! from its timeline buckets. `assetCount` counts archived members; `updatedAt` changes when Assets are added,
//! which makes it, with the list's ETag, the cheap change signal for a scoped API key.

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::asset::non_blank;
use crate::client::{Client, Fetched};
use crate::dates::{optional_utc, parse_utc};
use crate::error::ApiError;

/// An album.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Album {
    pub id: String,
    pub name: String,
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
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct AlbumWire {
    id: Option<String>,
    album_name: Option<String>,
    start_date: Option<String>,
    end_date: Option<String>,
    created_at: Option<String>,
    updated_at: Option<String>,
    asset_count: Option<u64>,
    shared: Option<bool>,
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
            start_date: optional_utc(self.start_date.as_deref()),
            end_date: optional_utc(self.end_date.as_deref()),
            updated_at: optional_utc(self.updated_at.as_deref()).unwrap_or(created_at),
            created_at,
            asset_count: self.asset_count.unwrap_or(0),
            shared: self.shared.unwrap_or(false),
            id,
        })
    }
}

fn into_albums(wire: Vec<AlbumWire>) -> Result<Vec<Album>, ApiError> {
    wire.into_iter().map(AlbumWire::into_album).collect()
}

impl Client {
    /// Every album the credential can see, owned and shared (`GET /albums`).
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::ALBUMS;

    #[test]
    fn albums_decode() {
        let wire: Vec<AlbumWire> = serde_json::from_str(ALBUMS).unwrap();
        let albums = into_albums(wire).unwrap();
        assert_eq!(
            albums,
            [Album {
                id: "81d8ee12-2679-4d47-b903-a1b1ebc1d533".to_owned(),
                name: "Persons".to_owned(),
                start_date: parse_utc("2014-08-15T00:00:00.000Z"),
                end_date: parse_utc("2019-12-12T00:00:00.000Z"),
                created_at: parse_utc("2026-11-01T21:02:04.055Z").unwrap(),
                updated_at: parse_utc("2026-11-01T21:02:04.055Z").unwrap(),
                asset_count: 3,
                shared: false,
            }]
        );
    }

    #[test]
    fn an_empty_album_has_no_dates() {
        let json = r#"[{"id":"a","albumName":"Empty","createdAt":"2026-11-01T21:02:04.055Z","assetCount":0}]"#;
        let albums = into_albums(serde_json::from_str(json).unwrap()).unwrap();
        assert_eq!(albums[0].start_date, None);
        assert_eq!(albums[0].end_date, None);
        assert_eq!(albums[0].updated_at, albums[0].created_at);
    }
}
