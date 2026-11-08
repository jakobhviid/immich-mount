//! Places: the countries and cities Immich has reverse-geocoded, from `GET /search/suggestions`.
//!
//! Names are GeoNames' nearest place, not always the obvious city (Times Square is "Weehawken"), and they are
//! the exact strings search filters on. Assets without a place are not listed here (`includeNull` is not sent).

use crate::client::Client;
use crate::error::ApiError;

/// Drop the `null` and blank entries a suggestion list can hold.
pub(crate) fn names(suggestions: Vec<Option<String>>) -> Vec<String> {
    suggestions
        .into_iter()
        .flatten()
        .filter(|name| !name.trim().is_empty())
        .collect()
}

pub(crate) const ENDPOINTS: &[crate::wire::Endpoint] = crate::wire::endpoints![
    GET "/search/suggestions" "asset.read",
];

impl Client {
    /// Every country with at least one Asset, sorted by the server.
    /// Needs `asset.read`.
    pub async fn countries(&self) -> Result<Vec<String>, ApiError> {
        let suggestions: Vec<Option<String>> = self
            .get_json(&["search", "suggestions"], &[("type", "country")])
            .await?;
        Ok(names(suggestions))
    }

    /// Every city in `country` with at least one Asset.
    /// Needs `asset.read`.
    pub async fn cities(&self, country: &str) -> Result<Vec<String>, ApiError> {
        let suggestions: Vec<Option<String>> = self
            .get_json(&["search", "suggestions"], &[("type", "city"), ("country", country)])
            .await?;
        Ok(names(suggestions))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::Credential;
    use crate::test_server::{CannedResponse, FakeServer};

    #[test]
    fn nulls_and_blanks_are_dropped() {
        let suggestions: Vec<Option<String>> = serde_json::from_str(r#"["Århus","Copenhagen",null,""]"#).unwrap();
        assert_eq!(names(suggestions), ["Århus", "Copenhagen"]);
    }

    #[tokio::test]
    async fn cities_encode_the_country() {
        let server = FakeServer::start(vec![CannedResponse::json(200, r#"["Weehawken"]"#)]).await;
        let client = Client::new(&server.url, None, "test").unwrap();
        let cities = client.cities("United States").await.unwrap();
        assert_eq!(cities, ["Weehawken"]);
        assert_eq!(
            server.requests()[0].target,
            "/api/search/suggestions?type=city&country=United+States"
        );
    }

    #[tokio::test]
    async fn a_redirect_within_the_server_is_followed() {
        let server = FakeServer::start(vec![
            CannedResponse::json(302, "").with_header("location", "/api/search/suggestions?type=city&country=Denmark"),
            CannedResponse::json(200, r#"["Odense"]"#),
        ])
        .await;
        let client = Client::new(&server.url, Some(Credential::ApiKey("key".into())), "test").unwrap();
        assert_eq!(client.cities("United States").await.unwrap(), ["Odense"]);
        assert_eq!(server.requests()[1].header("x-api-key").as_deref(), Some("key"));
    }

    #[tokio::test]
    async fn the_api_key_never_follows_a_redirect_elsewhere() {
        let elsewhere = FakeServer::start(vec![CannedResponse::json(200, r#"["Leaked"]"#)]).await;
        let server = FakeServer::start(vec![CannedResponse::json(302, "").with_header("location", &format!("{}/collect", elsewhere.url))]).await;
        let client = Client::new(&server.url, Some(Credential::ApiKey("key".into())), "test").unwrap();
        assert!(client.cities("Denmark").await.is_err());
        assert!(elsewhere.requests().is_empty(), "the other origin was asked: {:?}", elsewhere.requests());
    }
}
