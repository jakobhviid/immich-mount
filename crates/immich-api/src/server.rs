//! What the server is and what the credential may do: version, features, public config, API key permissions.
//!
//! `/server/version`, `/server/features` and `/server/config` answer without authentication on v3.3.5, so the
//! sign-in screen can read them before there is a credential (ARCHITECTURE.md, "Signing in").
//! `/api-keys/me` needs no permission, which makes it the connection probe for API keys; a session gets 403
//! there ("Not authenticated with an API Key").

use serde::Deserialize;

use crate::client::Client;
use crate::error::ApiError;
use crate::version::ServerVersion;

/// The sign-in and capability switches from `/server/features`. A switch the server does not send reads as off.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ServerFeatures {
    /// Browser sign-in (OIDC) is configured.
    pub oauth: bool,
    /// The server wants OIDC sign-in started without asking.
    pub oauth_auto_launch: bool,
    /// Email and password sign-in is allowed.
    pub password_login: bool,
    /// The trash is on; when off, deletes are permanent.
    pub trash: bool,
    /// Places (country and city) are filled in from GPS.
    pub reverse_geocoding: bool,
}

/// The public settings from `/server/config` that the mount uses.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ServerConfig {
    /// The label of the server's OIDC button ("Login with OAuth" by default).
    pub oauth_button_text: String,
    /// Days before trashed Assets are deleted for good. Immich's default, 30, when the server does not say.
    pub trash_days: u32,
}

impl Default for ServerConfig {
    fn default() -> Self {
        ServerConfig {
            oauth_button_text: String::new(),
            trash_days: 30,
        }
    }
}

#[derive(Deserialize)]
struct VersionWire {
    major: u32,
    minor: u32,
    #[serde(default)]
    patch: u32,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ApiKeyWire {
    permissions: Vec<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct MediaTypesWire {
    image: Vec<String>,
    video: Vec<String>,
}

pub(crate) const ENDPOINTS: &[crate::wire::Endpoint] = crate::wire::endpoints![
    GET "/server/version" -,
    GET "/server/features" -,
    GET "/server/config" -,
    GET "/server/media-types" -,
    GET "/api-keys/me" -,
];

impl Client {
    /// The server's version (`GET /server/version`). Needs no credential.
    pub async fn server_version(&self) -> Result<ServerVersion, ApiError> {
        let wire: VersionWire = self.get_json(&["server", "version"], &[]).await?;
        Ok(ServerVersion {
            major: wire.major,
            minor: wire.minor,
            patch: wire.patch,
        })
    }

    /// Which sign-in methods and features the server offers (`GET /server/features`). Needs no credential.
    pub async fn server_features(&self) -> Result<ServerFeatures, ApiError> {
        self.get_json(&["server", "features"], &[]).await
    }

    /// The server's public settings (`GET /server/config`). Needs no credential.
    pub async fn server_config(&self) -> Result<ServerConfig, ApiError> {
        self.get_json(&["server", "config"], &[]).await
    }

    /// The file extensions the server accepts (`GET /server/media-types`), as `.jpg` and the like, lower case.
    /// Images and videos only; sidecars are left out, because the mount does not upload them.
    /// Needs no credential.
    pub async fn server_media_types(&self) -> Result<Vec<String>, ApiError> {
        let wire: MediaTypesWire = self.get_json(&["server", "media-types"], &[]).await?;
        Ok(wire.image.into_iter().chain(wire.video).map(|extension| extension.to_ascii_lowercase()).collect())
    }

    /// The permissions of the API key in use (`GET /api-keys/me`), such as `["all"]` or `["asset.read", ...]`.
    ///
    /// Works for API keys only; a session gets [`ApiError::Forbidden`].
    /// Needs an API key; a session is refused.
    pub async fn api_key_permissions(&self) -> Result<Vec<String>, ApiError> {
        let wire: ApiKeyWire = self.get_json(&["api-keys", "me"], &[]).await?;
        Ok(wire.permissions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_types_join_images_and_videos_in_lower_case() {
        let wire: MediaTypesWire =
            serde_json::from_str(r#"{"image":[".JPG",".heic"],"video":[".mp4"],"sidecar":[".xmp"]}"#).unwrap();
        let extensions: Vec<String> = wire.image.into_iter().chain(wire.video).map(|extension| extension.to_ascii_lowercase()).collect();
        assert_eq!(extensions, [".jpg", ".heic", ".mp4"]);
    }

    #[test]
    fn features_decode_and_ignore_unknown_switches() {
        let body = r#"{"smartSearch":false,"facialRecognition":false,"duplicateDetection":false,"map":true,
            "reverseGeocoding":true,"importFaces":false,"sidecar":true,"search":true,"trash":true,"oauth":false,
            "oauthAutoLaunch":false,"ocr":false,"passwordLogin":true,"configFile":false,"email":false,
            "realtimeTranscoding":false,"someFutureSwitch":true}"#;
        let features: ServerFeatures = serde_json::from_str(body).unwrap();
        assert_eq!(
            features,
            ServerFeatures {
                oauth: false,
                oauth_auto_launch: false,
                password_login: true,
                trash: true,
                reverse_geocoding: true,
            }
        );
    }

    #[test]
    fn missing_feature_switches_read_as_off() {
        let features: ServerFeatures = serde_json::from_str(r#"{"oauth":true}"#).unwrap();
        assert!(features.oauth);
        assert!(!features.password_login);
    }

    #[test]
    fn config_decodes_the_fields_we_use() {
        let body = r#"{"loginPageMessage":"","trashDays":30,"userDeleteDelay":7,"oauthButtonText":"Login with OAuth",
            "oauthAccountManagementUrl":"","isInitialized":true,"isOnboarded":false,"externalDomain":"",
            "publicUsers":true,"maintenanceMode":false,"minFaces":3}"#;
        let config: ServerConfig = serde_json::from_str(body).unwrap();
        assert_eq!(config.oauth_button_text, "Login with OAuth");
        assert_eq!(config.trash_days, 30);
    }

    #[test]
    fn config_defaults_when_fields_are_missing() {
        let config: ServerConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(config, ServerConfig::default());
    }

    #[test]
    fn version_tolerates_prerelease_and_new_fields() {
        let wire: VersionWire =
            serde_json::from_str(r#"{"major":3,"minor":3,"patch":5,"prerelease":null,"build":"x"}"#).unwrap();
        assert_eq!((wire.major, wire.minor, wire.patch), (3, 3, 5));
    }

    #[test]
    fn api_key_permissions_decode() {
        let body = r#"{"id":"k","name":"lab-readonly","createdAt":"2026-11-01T21:01:49.000Z",
            "updatedAt":"2026-11-01T21:01:49.000Z","permissions":["album.read","asset.read","asset.view",
            "asset.download","person.read"]}"#;
        let wire: ApiKeyWire = serde_json::from_str(body).unwrap();
        assert_eq!(wire.permissions.len(), 5);
        assert_eq!(wire.permissions[0], "album.read");
    }
}
