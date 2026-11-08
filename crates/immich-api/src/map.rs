//! The map: one marker per Asset with GPS, and reverse geocoding of a point.

use chrono::{DateTime, Utc};
use reqwest::Method;
use serde::Deserialize;

use crate::client::Client;
use crate::dates::format_utc;
use crate::error::ApiError;
use crate::wire::{endpoints, Endpoint};

pub(crate) const ENDPOINTS: &[Endpoint] = endpoints![
    GET "/map/markers" "map.read",
    GET "/map/reverse-geocode" "map.search",
];

/// One Asset on the map.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MapMarker {
    /// The Asset's id.
    pub id: String,
    pub lat: f64,
    pub lon: f64,
    pub city: Option<String>,
    pub state: Option<String>,
    pub country: Option<String>,
}

/// A place a point falls in.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Place {
    pub city: Option<String>,
    pub state: Option<String>,
    pub country: Option<String>,
}

/// Which Assets [`Client::map_markers`] returns. The default is the user's own, archived ones included.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MapQuery {
    /// `Some(false)` leaves archived Assets out; `Some(true)` returns only them.
    pub archived: Option<bool>,
    pub favorite: Option<bool>,
    pub created_after: Option<DateTime<Utc>>,
    pub created_before: Option<DateTime<Utc>>,
    /// Include partners' Assets.
    pub with_partners: bool,
    /// Include Assets of albums shared with the user.
    pub with_shared_albums: bool,
}

impl Client {
    /// Every Asset with GPS coordinates that matches (`GET /map/markers`). Needs `map.read`.
    pub async fn map_markers(&self, query: &MapQuery) -> Result<Vec<MapMarker>, ApiError> {
        let mut parameters: Vec<(&str, String)> = Vec::new();
        if let Some(archived) = query.archived {
            parameters.push(("isArchived", archived.to_string()));
        }
        if let Some(favorite) = query.favorite {
            parameters.push(("isFavorite", favorite.to_string()));
        }
        if let Some(after) = &query.created_after {
            parameters.push(("fileCreatedAfter", format_utc(after)));
        }
        if let Some(before) = &query.created_before {
            parameters.push(("fileCreatedBefore", format_utc(before)));
        }
        if query.with_partners {
            parameters.push(("withPartners", "true".to_owned()));
        }
        if query.with_shared_albums {
            parameters.push(("withSharedAlbums", "true".to_owned()));
        }
        let borrowed: Vec<(&str, &str)> = parameters.iter().map(|(name, value)| (*name, value.as_str())).collect();
        self.call(Method::GET, &["map", "markers"], &borrowed).await
    }

    /// The places a point falls in, nearest first (`GET /map/reverse-geocode`). Needs `map.search`.
    pub async fn reverse_geocode(&self, latitude: f64, longitude: f64) -> Result<Vec<Place>, ApiError> {
        let (latitude, longitude) = (latitude.to_string(), longitude.to_string());
        self.call(Method::GET, &["map", "reverse-geocode"], &[("lat", &latitude), ("lon", &longitude)])
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_server::{CannedResponse, FakeServer};

    #[tokio::test]
    async fn map_markers_send_only_the_filters_that_are_set() {
        let answer = r#"[{"id":"a","lat":55.6,"lon":12.5,"city":"Copenhagen","state":null,"country":"Denmark"}]"#;
        let server = FakeServer::start(vec![CannedResponse::json(200, answer)]).await;
        let client = Client::new(&server.url, None, "test").unwrap();
        let query = MapQuery { archived: Some(false), with_partners: true, ..MapQuery::default() };
        let markers = client.map_markers(&query).await.unwrap();
        assert_eq!(markers[0].city.as_deref(), Some("Copenhagen"));
        assert_eq!(markers[0].state, None);
        assert_eq!(server.requests()[0].target, "/api/map/markers?isArchived=false&withPartners=true");
    }
}
