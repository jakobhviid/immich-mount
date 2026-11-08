//! Metadata search: the Assets of an album, a person, a place, a camera, a tag or a time range, with names and
//! sizes. [`AssetQuery`] is also the filter of smart, random and counting search ([`crate::search_more`]).
//!
//! Only the v3.2 dialect is spoken: `filter`, `orderBy` and `cursor` (ARCHITECTURE.md, "Version policy"). The
//! deprecated fields (`page`, `visibility`, `takenAfter`, `withDeleted`, ...) are never sent, because mixing the
//! two dialects is a 400. Every search:
//! - sets `filter.trashedAt: {eq: null}`, because this dialect includes trashed Assets by default
//! - sets `withExif: true`, because without it there is no `exifInfo`, so no file size
//! - orders by `fileCreatedAt`, newest first, so cursor pages are stable
//!
//! `takenAt` compares the **UTC** `fileCreatedAt`, not the local month a timeline bucket uses (seen on
//! v3.3.5). `nextCursor` is opaque (today base64 `{"offset":N}`); `null` means the last page.

use std::collections::HashSet;
use std::ops::Range;

use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::asset::{Asset, AssetKind, AssetWire, Visibility};
use crate::client::Client;
use crate::dates::format_utc;
use crate::error::ApiError;

/// The largest page Immich serves.
pub const MAX_PAGE_SIZE: u32 = 1000;

/// What to search for. Every condition set must hold; an empty query finds every non-trashed Asset.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AssetQuery {
    /// In any of these albums.
    pub album_ids: Option<Vec<String>>,
    /// Showing any of these people.
    pub person_ids: Option<Vec<String>>,
    /// In this city (reverse-geocoded name, exact).
    pub city: Option<String>,
    /// In this country (reverse-geocoded name, exact).
    pub country: Option<String>,
    /// Taken in this half-open UTC range (`start` included, `end` excluded), on `fileCreatedAt`.
    pub taken: Option<Range<DateTime<Utc>>>,
    /// Changed by Immich after this instant (archive, trash, edits; not album membership).
    pub updated_after: Option<DateTime<Utc>>,
    /// Only Assets of this visibility: `Timeline` for the main timeline (no archived, hidden or locked ones),
    /// `Archive` for the archive. Without it, a search finds timeline and archived Assets.
    pub visibility: Option<Visibility>,
    /// Only the Assets marked as favorites.
    pub favorites_only: bool,
    /// In this state or region (reverse-geocoded name, exact).
    pub state: Option<String>,
    /// Tagged with any of these tags.
    pub tag_ids: Option<Vec<String>>,
    /// Only images, or only videos.
    pub kind: Option<AssetKind>,
    /// Taken with a camera of this make (as [`Client::suggestions`] lists them, exact).
    pub camera_make: Option<String>,
    /// Taken with this camera model (exact).
    pub camera_model: Option<String>,
    /// Taken with this lens (exact).
    pub lens_model: Option<String>,
}

impl AssetQuery {
    /// The request body for one page.
    pub(crate) fn to_body(&self, page_size: u32, cursor: Option<&str>) -> Value {
        let mut body = json!({
            "filter": self.to_filter(),
            "orderBy": { "field": "fileCreatedAt", "direction": "desc" },
            "size": page_size.clamp(1, MAX_PAGE_SIZE),
            "withExif": true,
        });
        if let Some(cursor) = cursor {
            body["cursor"] = Value::String(cursor.to_owned());
        }
        body
    }

    /// The `filter` object of the v3.2 dialect, which smart, random and statistics searches take too.
    pub(crate) fn to_filter(&self) -> Map<String, Value> {
        let mut filter = Map::new();
        filter.insert("trashedAt".to_owned(), json!({ "eq": null }));
        if let Some(album_ids) = &self.album_ids {
            filter.insert("albumIds".to_owned(), json!({ "any": album_ids }));
        }
        if let Some(person_ids) = &self.person_ids {
            filter.insert("personIds".to_owned(), json!({ "any": person_ids }));
        }
        if let Some(city) = &self.city {
            filter.insert("city".to_owned(), json!({ "eq": city }));
        }
        if let Some(country) = &self.country {
            filter.insert("country".to_owned(), json!({ "eq": country }));
        }
        if let Some(taken) = &self.taken {
            filter.insert(
                "takenAt".to_owned(),
                json!({ "gte": format_utc(&taken.start), "lt": format_utc(&taken.end) }),
            );
        }
        if let Some(updated_after) = &self.updated_after {
            filter.insert("updatedAt".to_owned(), json!({ "gt": format_utc(updated_after) }));
        }
        if let Some(visibility) = &self.visibility {
            filter.insert("visibility".to_owned(), json!({ "eq": visibility.as_str() }));
        }
        if self.favorites_only {
            filter.insert("isFavorite".to_owned(), json!({ "eq": true }));
        }
        if let Some(state) = &self.state {
            filter.insert("state".to_owned(), json!({ "eq": state }));
        }
        if let Some(tag_ids) = &self.tag_ids {
            filter.insert("tagIds".to_owned(), json!({ "any": tag_ids }));
        }
        if let Some(kind) = &self.kind {
            filter.insert("type".to_owned(), json!({ "eq": kind.as_str() }));
        }
        if let Some(make) = &self.camera_make {
            filter.insert("make".to_owned(), json!({ "eq": make }));
        }
        if let Some(model) = &self.camera_model {
            filter.insert("model".to_owned(), json!({ "eq": model }));
        }
        if let Some(lens) = &self.lens_model {
            filter.insert("lensModel".to_owned(), json!({ "eq": lens }));
        }
        filter
    }
}

/// One page of search results.
#[derive(Debug, Clone, PartialEq)]
pub struct AssetPage {
    pub assets: Vec<Asset>,
    /// Pass this to get the next page; `None` on the last page.
    pub next_cursor: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct SearchAnswerWire {
    assets: SearchAssetsWire,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct SearchAssetsWire {
    items: Vec<AssetWire>,
    next_cursor: Option<String>,
}

fn into_page(wire: SearchAnswerWire) -> Result<AssetPage, ApiError> {
    let assets = wire
        .assets
        .items
        .into_iter()
        .map(AssetWire::into_asset)
        .collect::<Result<Vec<_>, _>>()?;
    let next_cursor = wire.assets.next_cursor.filter(|cursor| !cursor.is_empty());
    Ok(AssetPage { assets, next_cursor })
}

pub(crate) const ENDPOINTS: &[crate::wire::Endpoint] = crate::wire::endpoints![
    POST "/search/metadata" "asset.read",
];

impl Client {
    /// One page of Assets matching `query` (`POST /search/metadata`). `page_size` is clamped to 1..=1000.
    pub async fn search_assets(
        &self,
        query: &AssetQuery,
        page_size: u32,
        cursor: Option<&str>,
    ) -> Result<AssetPage, ApiError> {
        let body = query.to_body(page_size, cursor);
        let wire: SearchAnswerWire = self.post_json(&["search", "metadata"], &body).await?;
        into_page(wire)
    }

    /// Every Asset matching `query`, paging through to the end.
    ///
    /// FUSE listings must be complete, so this fetches all pages before returning. An Asset seen twice (the
    /// library changed between pages) is kept once; a cursor that repeats is a protocol error rather than a loop.
    pub async fn search_all_assets(&self, query: &AssetQuery, page_size: u32) -> Result<Vec<Asset>, ApiError> {
        let mut assets = Vec::new();
        let mut seen_ids = HashSet::new();
        let mut seen_cursors = HashSet::new();
        let mut cursor: Option<String> = None;
        loop {
            let page = self.search_assets(query, page_size, cursor.as_deref()).await?;
            for asset in page.assets {
                if seen_ids.insert(asset.id.clone()) {
                    assets.push(asset);
                }
            }
            match page.next_cursor {
                None => return Ok(assets),
                Some(next) => {
                    if !seen_cursors.insert(next.clone()) {
                        return Err(ApiError::Protocol(format!("search returned the cursor `{next}` twice")));
                    }
                    cursor = Some(next);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dates::parse_utc;
    use crate::fixtures::SEARCH_ITEM;
    use crate::test_server::{CannedResponse, FakeServer};

    #[test]
    fn empty_query_still_excludes_trash_and_asks_for_exif() {
        let body = AssetQuery::default().to_body(250, None);
        assert_eq!(
            body,
            json!({
                "filter": { "trashedAt": { "eq": null } },
                "orderBy": { "field": "fileCreatedAt", "direction": "desc" },
                "size": 250,
                "withExif": true,
            })
        );
    }

    #[test]
    fn every_condition_becomes_a_filter() {
        let query = AssetQuery {
            album_ids: Some(vec!["album-1".into()]),
            person_ids: Some(vec!["person-1".into(), "person-2".into()]),
            city: Some("Lyon".into()),
            country: Some("France".into()),
            taken: Some(parse_utc("2018-11-01T00:00:00Z").unwrap()..parse_utc("2018-12-01T00:00:00Z").unwrap()),
            updated_after: Some(parse_utc("2026-01-01T00:00:00Z").unwrap()),
            visibility: Some(Visibility::Archive),
            favorites_only: true,
            state: Some("Auvergne-Rhône-Alpes".into()),
            tag_ids: Some(vec!["tag-1".into()]),
            kind: Some(AssetKind::Video),
            camera_make: Some("Apple".into()),
            camera_model: Some("iPhone 6".into()),
            lens_model: Some("iPhone 6 back camera".into()),
        };
        let body = query.to_body(5000, Some("eyJvZmZzZXQiOjJ9"));
        assert_eq!(
            body["filter"],
            json!({
                "trashedAt": { "eq": null },
                "albumIds": { "any": ["album-1"] },
                "personIds": { "any": ["person-1", "person-2"] },
                "city": { "eq": "Lyon" },
                "country": { "eq": "France" },
                "takenAt": { "gte": "2018-11-01T00:00:00.000Z", "lt": "2018-12-01T00:00:00.000Z" },
                "updatedAt": { "gt": "2026-01-01T00:00:00.000Z" },
                "visibility": { "eq": "archive" },
                "isFavorite": { "eq": true },
                "state": { "eq": "Auvergne-Rhône-Alpes" },
                "tagIds": { "any": ["tag-1"] },
                "type": { "eq": "VIDEO" },
                "make": { "eq": "Apple" },
                "model": { "eq": "iPhone 6" },
                "lensModel": { "eq": "iPhone 6 back camera" },
            })
        );
        assert_eq!(body["size"], json!(1000), "clamped to the server's maximum");
        assert_eq!(body["cursor"], json!("eyJvZmZzZXQiOjJ9"));
    }

    #[test]
    fn deprecated_fields_are_never_sent() {
        let body = AssetQuery {
            visibility: Some(Visibility::Timeline),
            ..AssetQuery::default()
        }
        .to_body(10, None);
        for deprecated in ["page", "order", "visibility", "takenAfter", "takenBefore", "updatedAfter", "withDeleted"] {
            assert!(body.get(deprecated).is_none(), "sent {deprecated}");
        }
    }

    fn search_answer(items: &[&str], next_cursor: Option<&str>) -> String {
        format!(
            r#"{{"albums":{{"total":0,"count":0,"items":[],"facets":[]}},
                "assets":{{"total":{count},"count":{count},"items":[{items}],"facets":[],"nextPage":null,
                "nextCursor":{cursor}}}}}"#,
            count = items.len(),
            items = items.join(","),
            cursor = next_cursor.map_or("null".to_owned(), |cursor| format!("\"{cursor}\"")),
        )
    }

    #[test]
    fn page_decodes_with_next_cursor() {
        let wire: SearchAnswerWire = serde_json::from_str(&search_answer(&[SEARCH_ITEM], Some("eyJvZmZzZXQiOjF9"))).unwrap();
        let page = into_page(wire).unwrap();
        assert_eq!(page.assets.len(), 1);
        assert_eq!(page.assets[0].file_size, Some(1_012_520));
        assert_eq!(page.next_cursor.as_deref(), Some("eyJvZmZzZXQiOjF9"));
    }

    #[test]
    fn last_page_has_no_cursor() {
        let wire: SearchAnswerWire = serde_json::from_str(&search_answer(&[], None)).unwrap();
        let page = into_page(wire).unwrap();
        assert!(page.assets.is_empty());
        assert_eq!(page.next_cursor, None);
    }

    #[tokio::test]
    async fn search_all_follows_cursors_and_drops_repeats() {
        let other_item = SEARCH_ITEM.replace("12ecb329-c96f-41b1-8406-e5b8fe134454", "second-id");
        let server = FakeServer::start(vec![
            CannedResponse::json(200, &search_answer(&[SEARCH_ITEM], Some("c1"))),
            CannedResponse::json(200, &search_answer(&[SEARCH_ITEM, &other_item], None)),
        ])
        .await;
        let client = Client::new(&server.url, None, "test").unwrap();
        let assets = client.search_all_assets(&AssetQuery::default(), 1).await.unwrap();
        let ids: Vec<&str> = assets.iter().map(|asset| asset.id.as_str()).collect();
        assert_eq!(ids, ["12ecb329-c96f-41b1-8406-e5b8fe134454", "second-id"]);
        let requests = server.requests();
        assert_eq!(requests[0].target, "/api/search/metadata");
        assert!(requests[1].body_text().contains(r#""cursor":"c1""#));
    }

    #[tokio::test]
    async fn a_repeating_cursor_is_an_error_not_a_loop() {
        let server = FakeServer::start(vec![
            CannedResponse::json(200, &search_answer(&[], Some("same"))),
            CannedResponse::json(200, &search_answer(&[], Some("same"))),
        ])
        .await;
        let client = Client::new(&server.url, None, "test").unwrap();
        let error = client.search_all_assets(&AssetQuery::default(), 10).await.unwrap_err();
        assert!(matches!(error, ApiError::Protocol(_)), "{error:?}");
    }
}
