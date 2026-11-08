//! The signed-in account: its details, preferences and profile picture, its password and PIN code, the locked
//! folder's unlock, and the other users it can see.
//!
//! The locked folder: Assets with `locked` visibility appear only to a session that was unlocked with the PIN code
//! (or the password) through [`Client::unlock_session`]; an API key cannot unlock. [`Client::auth_status`] says
//! whether the account has a PIN code and whether this session is unlocked (`is_elevated`).
//!
//! Quirks, seen on v3.3.6:
//! - `PUT /users/me` and `PUT /users/me/preferences` are deprecated with no replacement yet; they are the only way
//! - preferences differ between versions (`people.updateStrategy` is v3.3 only), so they are sent as a JSON patch
//!   and read into a model whose every part is optional

use bytes::Bytes;
use chrono::{DateTime, Utc};
use reqwest::multipart::{Form, Part};
use reqwest::Method;
use serde::{Deserialize, Serialize};

use crate::album::AssetOrder;
use crate::client::{check, read_json, Client, Media, REQUEST_TIMEOUT};
use crate::error::{from_transport, ApiError, RequestShape};
use crate::retry::Resend;
use crate::user::{AvatarColor, User, UserDetails};
use crate::wire::{endpoints, Endpoint};

pub(crate) const ENDPOINTS: &[Endpoint] = endpoints![
    PUT "/users/me" "user.update",
    GET "/users" "user.read",
    GET "/users/{id}" "user.read",
    GET "/users/me/preferences" "userPreference.read",
    PUT "/users/me/preferences" "userPreference.update",
    POST "/users/profile-image" "userProfileImage.update",
    DELETE "/users/profile-image" "userProfileImage.delete",
    GET "/users/{id}/profile-image" "userProfileImage.read",
    POST "/auth/change-password" "auth.changePassword",
    GET "/auth/status" -,
    POST "/auth/pin-code" "pinCode.create",
    PUT "/auth/pin-code" "pinCode.update",
    DELETE "/auth/pin-code" "pinCode.delete",
    POST "/auth/session/unlock" -,
    POST "/auth/session/lock" -,
    POST "/auth/validateToken" -,
];

/// Changes to the account; `None` leaves a field as it is.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountChanges {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_color: Option<AvatarColor>,
}

/// The account's preferences (`UserPreferencesResponseDto`), the parts a client acts on. Every part may be missing
/// on a version that lacks it.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Preferences {
    pub albums: AlbumPreferences,
    pub download: DownloadPreferences,
    pub email_notifications: EmailPreferences,
    pub folders: FeaturePreference,
    pub memories: MemoryPreferences,
    pub people: PeoplePreferences,
    pub ratings: FeaturePreference,
    pub shared_links: FeaturePreference,
    pub tags: FeaturePreference,
    /// The whole answer, for the parts the model leaves out.
    #[serde(skip)]
    pub raw: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AlbumPreferences {
    pub default_asset_order: AssetOrder,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DownloadPreferences {
    /// The size at which a download is split into several archives, in bytes.
    pub archive_size: u64,
    pub include_embedded_videos: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct EmailPreferences {
    pub enabled: bool,
    pub album_invite: bool,
    pub album_update: bool,
}

/// A feature the user can switch on or off, and show in the web app's sidebar.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct FeaturePreference {
    pub enabled: bool,
    pub sidebar_web: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MemoryPreferences {
    pub enabled: bool,
    /// Seconds each memory shows in a slideshow.
    pub duration: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PeoplePreferences {
    pub enabled: bool,
    pub sidebar_web: bool,
}

/// What the server knows about this sign-in (`GET /auth/status`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AuthStatus {
    /// The account has a password (an OIDC-only account has none).
    pub password: bool,
    /// The account has a PIN code for the locked folder.
    pub pin_code: bool,
    /// This session is unlocked and may see the locked folder.
    pub is_elevated: bool,
    #[serde(deserialize_with = "crate::dates::serde_dates::optional_utc")]
    pub expires_at: Option<DateTime<Utc>>,
    #[serde(deserialize_with = "crate::dates::serde_dates::optional_utc")]
    pub pin_expires_at: Option<DateTime<Utc>>,
}

/// What proves the user when unlocking, or changing or removing the PIN code. Its `Debug` output is redacted.
#[derive(Clone, PartialEq, Eq)]
pub enum Proof {
    PinCode(String),
    Password(String),
}

impl std::fmt::Debug for Proof {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Proof::PinCode(_) => formatter.write_str("PinCode(<redacted>)"),
            Proof::Password(_) => formatter.write_str("Password(<redacted>)"),
        }
    }
}

impl Proof {
    fn body(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut body = serde_json::Map::new();
        match self {
            Proof::PinCode(pin_code) => body.insert("pinCode".to_owned(), pin_code.clone().into()),
            Proof::Password(password) => body.insert("password".to_owned(), password.clone().into()),
        };
        body
    }
}

/// The profile picture as stored (`CreateProfileImageResponseDto`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ProfileImage {
    pub user_id: String,
    pub profile_image_path: String,
    #[serde(deserialize_with = "crate::dates::serde_dates::optional_utc")]
    pub profile_changed_at: Option<DateTime<Utc>>,
}

impl Client {
    /// The signed-in account with its private details (`GET /users/me`). Needs `user.read`.
    pub async fn me(&self) -> Result<UserDetails, ApiError> {
        self.call(Method::GET, &["users", "me"], &[]).await
    }

    /// Change the account's name, email or colour (`PUT /users/me`). Needs `user.update`.
    pub async fn update_me(&self, changes: &AccountChanges) -> Result<UserDetails, ApiError> {
        self.call_with(Method::PUT, &["users", "me"], &[], changes).await
    }

    /// The other users of the server, as this account sees them (`GET /users`). Needs `user.read`.
    pub async fn users(&self) -> Result<Vec<User>, ApiError> {
        self.call(Method::GET, &["users"], &[]).await
    }

    /// One user (`GET /users/{id}`). Needs `user.read`.
    pub async fn user(&self, id: &str) -> Result<User, ApiError> {
        self.call(Method::GET, &["users", id], &[]).await
    }

    /// The account's preferences (`GET /users/me/preferences`). Needs `userPreference.read`.
    pub async fn preferences(&self) -> Result<Preferences, ApiError> {
        let raw: serde_json::Value = self.call(Method::GET, &["users", "me", "preferences"], &[]).await?;
        preferences_from(raw)
    }

    /// Change preferences with a JSON patch of the parts to change, such as
    /// `{"memories": {"enabled": false}}` (`PUT /users/me/preferences`); returns them all. Needs
    /// `userPreference.update`.
    pub async fn update_preferences(&self, patch: &serde_json::Value) -> Result<Preferences, ApiError> {
        let raw: serde_json::Value = self.call_with(Method::PUT, &["users", "me", "preferences"], &[], patch).await?;
        preferences_from(raw)
    }

    /// Set the account's profile picture from an image file's bytes (`POST /users/profile-image`, multipart).
    /// Needs `userProfileImage.update`.
    pub async fn set_profile_image(&self, image: Bytes, file_name: &str, mime_type: &str) -> Result<ProfileImage, ApiError> {
        if Part::bytes(Vec::new()).mime_str(mime_type).is_err() {
            return Err(ApiError::Protocol(format!("`{mime_type}` is not a media type")));
        }
        // The form is built afresh for each attempt: a multipart body cannot be replayed.
        let response = self
            .execute_with_resending(Resend::OnlyUnsent, || {
                let part = Part::bytes(image.to_vec()).file_name(file_name.to_owned()).mime_str(mime_type);
                let builder = self.checked_request(Method::POST, &["users", "profile-image"]);
                async move {
                    let part = part.map_err(|_| ApiError::Protocol("internal: a media type was refused".to_owned()))?;
                    builder?
                        .timeout(REQUEST_TIMEOUT)
                        .multipart(Form::new().part("file", part))
                        .build()
                        .map_err(|error| from_transport(&error))
                }
            })
            .await?;
        let response = check(response, RequestShape::Plain).await?;
        read_json(response, "users/profile-image").await
    }

    /// Remove the account's profile picture (`DELETE /users/profile-image`). Needs `userProfileImage.delete`.
    pub async fn delete_profile_image(&self) -> Result<(), ApiError> {
        self.call_unit(Method::DELETE, &["users", "profile-image"], &[]).await
    }

    /// A user's profile picture (`GET /users/{id}/profile-image`); [`ApiError::NotFound`] when they have none.
    /// Needs `userProfileImage.read`.
    pub async fn profile_image(&self, user_id: &str) -> Result<Media, ApiError> {
        self.get_media(&["users", user_id, "profile-image"], &[]).await
    }

    /// Change the account's password, optionally signing out its other sessions (`POST /auth/change-password`).
    /// Needs `auth.changePassword`.
    pub async fn change_password(
        &self,
        password: &str,
        new_password: &str,
        sign_out_other_sessions: bool,
    ) -> Result<UserDetails, ApiError> {
        let body = serde_json::json!({
            "password": password,
            "newPassword": new_password,
            "invalidateSessions": sign_out_other_sessions,
        });
        self.call_with(Method::POST, &["auth", "change-password"], &[], &body).await
    }

    /// Whether the account has a password and a PIN code, and whether this session is unlocked
    /// (`GET /auth/status`). An API key needs `all` for it, as for every signed-in endpoint whose spec names no
    /// permission.
    pub async fn auth_status(&self) -> Result<AuthStatus, ApiError> {
        self.call(Method::GET, &["auth", "status"], &[]).await
    }

    /// Whether the credential is valid (`POST /auth/validateToken`).
    /// Any valid credential may ask, a scoped API key included.
    pub async fn validate_credential(&self) -> Result<bool, ApiError> {
        #[derive(Deserialize, Default)]
        #[serde(default, rename_all = "camelCase")]
        struct Answer {
            auth_status: bool,
        }
        let answer: Answer = self.call(Method::POST, &["auth", "validateToken"], &[]).await?;
        Ok(answer.auth_status)
    }

    /// Give the account its first PIN code (`POST /auth/pin-code`). Needs `pinCode.create`.
    pub async fn set_pin_code(&self, pin_code: &str) -> Result<(), ApiError> {
        self.call_unit_with(Method::POST, &["auth", "pin-code"], &[], &serde_json::json!({ "pinCode": pin_code }))
            .await
    }

    /// Change the PIN code, proven with the old one or the password (`PUT /auth/pin-code`). Needs `pinCode.update`.
    pub async fn change_pin_code(&self, proof: &Proof, new_pin_code: &str) -> Result<(), ApiError> {
        let mut body = proof.body();
        body.insert("newPinCode".to_owned(), new_pin_code.into());
        // Sent twice, the second attempt would prove itself with the old PIN code and fail.
        self.call_unit_with_once(Method::PUT, &["auth", "pin-code"], &[], &body).await
    }

    /// Remove the PIN code, proven with it or the password (`DELETE /auth/pin-code`). Needs `pinCode.delete`.
    pub async fn remove_pin_code(&self, proof: &Proof) -> Result<(), ApiError> {
        self.call_unit_with_once(Method::DELETE, &["auth", "pin-code"], &[], &proof.body()).await
    }

    /// Unlock this session for the locked folder (`POST /auth/session/unlock`). A session only; an API key cannot.
    /// Needs a session; an API key without `all` is refused.
    pub async fn unlock_session(&self, proof: &Proof) -> Result<(), ApiError> {
        self.call_unit_with(Method::POST, &["auth", "session", "unlock"], &[], &proof.body()).await
    }

    /// Lock this session again (`POST /auth/session/lock`).
    /// Needs a session; an API key without `all` is refused.
    pub async fn lock_session(&self) -> Result<(), ApiError> {
        self.call_unit(Method::POST, &["auth", "session", "lock"], &[]).await
    }
}

pub(crate) fn preferences_from(raw: serde_json::Value) -> Result<Preferences, ApiError> {
    let mut stripped = raw.clone();
    crate::wire::strip_nulls(&mut stripped);
    let mut preferences: Preferences = serde_json::from_value(stripped)
        .map_err(|error| ApiError::Protocol(format!("decoding the preferences: {error}")))?;
    preferences.raw = raw;
    Ok(preferences)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferences_keep_what_the_model_leaves_out() {
        let raw = serde_json::json!({"memories": {"enabled": true, "duration": 5, "sidebarWeb": false},
            "cast": {"gCastEnabled": true}, "people": null});
        let preferences = preferences_from(raw.clone()).unwrap();
        assert!(preferences.memories.enabled);
        assert_eq!(preferences.memories.duration, 5);
        assert!(!preferences.people.enabled);
        assert_eq!(preferences.raw["cast"]["gCastEnabled"], true);
    }

    #[test]
    fn a_proof_is_one_field() {
        assert_eq!(
            serde_json::Value::Object(Proof::PinCode("123456".to_owned()).body()),
            serde_json::json!({"pinCode": "123456"})
        );
    }
}
