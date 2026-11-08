//! Sessions and API keys: the account's sign-ins on devices, and the keys apps use instead of a password.
//!
//! Quirks, seen on v3.3.6:
//! - a key's secret is shown once, when it is created or rotated; listing never shows it
//! - a key cannot be given a permission its creator lacks, and a key made by a key can hold at most that key's
//!   permissions
//! - `DELETE /sessions` signs out every session except the current one

use chrono::{DateTime, Utc};
use reqwest::Method;
use serde::{Deserialize, Serialize};

use crate::client::Client;
use crate::dates::serde_dates;
use crate::error::ApiError;
use crate::wire::{endpoints, Endpoint, Secret};

pub(crate) const ENDPOINTS: &[Endpoint] = endpoints![
    GET "/sessions" "session.read",
    POST "/sessions" "session.create",
    DELETE "/sessions/{id}" "session.delete",
    DELETE "/sessions" "session.delete",
    POST "/sessions/{id}/lock" "session.lock",
    GET "/api-keys" "apiKey.read",
    GET "/api-keys/{id}" "apiKey.read",
    POST "/api-keys" "apiKey.create",
    PUT "/api-keys/{id}" "apiKey.update",
    POST "/api-keys/{id}/rotate" "apiKey.rotate",
    DELETE "/api-keys/{id}" "apiKey.delete",
];

/// A sign-in on a device (`SessionResponseDto`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Session {
    pub id: String,
    /// The session making this request.
    pub current: bool,
    /// Such as `iOS` or `Linux`.
    #[serde(rename = "deviceOS")]
    pub device_os: String,
    /// Such as `Firefox` or `Immich Mount`.
    pub device_type: String,
    pub app_version: Option<String>,
    #[serde(deserialize_with = "serde_dates::utc")]
    pub created_at: DateTime<Utc>,
    #[serde(deserialize_with = "serde_dates::utc")]
    pub updated_at: DateTime<Utc>,
    #[serde(deserialize_with = "serde_dates::optional_utc")]
    pub expires_at: Option<DateTime<Utc>>,
    /// The session's sync stream starts over at its next sync.
    pub is_pending_sync_reset: bool,
}

/// A new session, as made for another device ([`Client::create_session`]), with its token.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct NewSession {
    #[serde(flatten)]
    pub session: Session,
    /// The bearer token; shown only now.
    pub token: Secret,
}

/// An API key (`ApiKeyResponseDto`); its secret is never shown again after creation.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ApiKey {
    pub id: String,
    pub name: String,
    /// Permission names such as `asset.read`, or `all`.
    pub permissions: Vec<String>,
    #[serde(deserialize_with = "serde_dates::utc")]
    pub created_at: DateTime<Utc>,
    #[serde(deserialize_with = "serde_dates::utc")]
    pub updated_at: DateTime<Utc>,
}

/// A key just created or rotated, with its secret.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct NewApiKey {
    #[serde(rename = "apiKey")]
    pub key: ApiKey,
    /// The value to send as `x-api-key`; shown only now.
    pub secret: Secret,
}

impl Client {
    /// The account's sessions (`GET /sessions`). Needs `session.read`.
    pub async fn sessions(&self) -> Result<Vec<Session>, ApiError> {
        self.call(Method::GET, &["sessions"], &[]).await
    }

    /// A session for another device, such as a TV signed in from a phone (`POST /sessions`). `duration` is in
    /// seconds; `None` lets the server choose. Needs `session.create`.
    pub async fn create_session(
        &self,
        device_type: &str,
        device_os: &str,
        duration_seconds: Option<u64>,
    ) -> Result<NewSession, ApiError> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Body<'a> {
            device_type: &'a str,
            #[serde(rename = "deviceOS")]
            device_os: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            duration: Option<u64>,
        }
        let body = Body { device_type, device_os, duration: duration_seconds };
        self.call_with(Method::POST, &["sessions"], &[], &body).await
    }

    /// Sign out one session (`DELETE /sessions/{id}`). Needs `session.delete`.
    pub async fn delete_session(&self, id: &str) -> Result<(), ApiError> {
        self.call_unit(Method::DELETE, &["sessions", id], &[]).await
    }

    /// Sign out every other session (`DELETE /sessions`). Needs `session.delete`.
    pub async fn delete_other_sessions(&self) -> Result<(), ApiError> {
        self.call_unit(Method::DELETE, &["sessions"], &[]).await
    }

    /// Lock a session out of the locked folder (`POST /sessions/{id}/lock`). Needs `session.lock`.
    pub async fn lock_other_session(&self, id: &str) -> Result<(), ApiError> {
        self.call_unit(Method::POST, &["sessions", id, "lock"], &[]).await
    }

    /// The account's API keys, without their secrets (`GET /api-keys`). Needs `apiKey.read`.
    pub async fn api_keys(&self) -> Result<Vec<ApiKey>, ApiError> {
        self.call(Method::GET, &["api-keys"], &[]).await
    }

    /// One API key (`GET /api-keys/{id}`). Needs `apiKey.read`.
    pub async fn api_key(&self, id: &str) -> Result<ApiKey, ApiError> {
        self.call(Method::GET, &["api-keys", id], &[]).await
    }

    /// Create an API key with these permissions (`POST /api-keys`); its secret comes back once. Needs
    /// `apiKey.create`.
    pub async fn create_api_key(&self, name: &str, permissions: &[String]) -> Result<NewApiKey, ApiError> {
        let body = serde_json::json!({ "name": name, "permissions": permissions });
        self.call_with(Method::POST, &["api-keys"], &[], &body).await
    }

    /// Rename a key, or change its permissions (`PUT /api-keys/{id}`). Needs `apiKey.update`.
    pub async fn update_api_key(&self, id: &str, name: Option<&str>, permissions: Option<&[String]>) -> Result<ApiKey, ApiError> {
        let mut body = serde_json::Map::new();
        if let Some(name) = name {
            body.insert("name".to_owned(), name.into());
        }
        if let Some(permissions) = permissions {
            body.insert("permissions".to_owned(), serde_json::json!(permissions));
        }
        self.call_with(Method::PUT, &["api-keys", id], &[], &body).await
    }

    /// Give a key a new secret; the old one stops working (`POST /api-keys/{id}/rotate`). Needs `apiKey.rotate`.
    pub async fn rotate_api_key(&self, id: &str) -> Result<NewApiKey, ApiError> {
        self.call(Method::POST, &["api-keys", id, "rotate"], &[]).await
    }

    /// Delete an API key (`DELETE /api-keys/{id}`). Needs `apiKey.delete`.
    pub async fn delete_api_key(&self, id: &str) -> Result<(), ApiError> {
        self.call_unit(Method::DELETE, &["api-keys", id], &[]).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_key_carries_its_secret_and_the_key() {
        let json = br#"{"apiKey":{"id":"k","name":"lab","permissions":["all"],"createdAt":"2026-11-08T10:00:00Z",
            "updatedAt":"2026-11-08T10:00:00Z"},"secret":"s3cret"}"#;
        let created: NewApiKey = crate::client::decode_json(json, "api-keys").unwrap();
        assert_eq!(created.secret.expose(), "s3cret");
        assert_eq!(created.key.permissions, ["all"]);
    }
}
