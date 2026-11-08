//! Users: the people who have accounts on the server, as other endpoints show them (album members, partners, link
//! owners).
//!
//! The account's own details, its preferences and the user list live here too; administration of other accounts
//! is in [`crate::admin`].

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::wire::wire_enum;

wire_enum! {
    /// The colour Immich draws a user's initials in when they have no profile picture.
    pub enum AvatarColor {
        Primary = "primary",
        Pink = "pink",
        Red = "red",
        Yellow = "yellow",
        Blue = "blue",
        Green = "green",
        Purple = "purple",
        Orange = "orange",
        Gray = "gray",
        Amber = "amber",
    }
}

/// Another account, as albums, partners and shared links show it (`UserResponseDto`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct User {
    pub id: String,
    pub name: String,
    pub email: String,
    pub avatar_color: AvatarColor,
    /// Empty when the user has no profile picture.
    pub profile_image_path: String,
    #[serde(deserialize_with = "crate::dates::serde_dates::optional_utc")]
    pub profile_changed_at: Option<DateTime<Utc>>,
}

wire_enum! {
    /// Whether an account is in use or on its way out.
    pub enum UserStatus {
        Active = "active",
        Removing = "removing",
        Deleted = "deleted",
    }
}

/// An account with its private details, as its owner or an admin sees it (`UserAdminResponseDto`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct UserDetails {
    pub id: String,
    pub name: String,
    pub email: String,
    pub is_admin: bool,
    pub avatar_color: AvatarColor,
    pub profile_image_path: String,
    #[serde(deserialize_with = "crate::dates::serde_dates::optional_utc")]
    pub profile_changed_at: Option<DateTime<Utc>>,
    /// The user must pick a new password at the next sign-in.
    pub should_change_password: bool,
    pub status: UserStatus,
    /// The folder name of the user's uploads in the server's storage template, when set.
    pub storage_label: Option<String>,
    /// `None` means no quota.
    pub quota_size_in_bytes: Option<u64>,
    pub quota_usage_in_bytes: Option<u64>,
    /// The user's id at the OIDC provider; empty when never linked.
    pub oauth_id: String,
    #[serde(deserialize_with = "crate::dates::serde_dates::utc")]
    pub created_at: DateTime<Utc>,
    #[serde(deserialize_with = "crate::dates::serde_dates::utc")]
    pub updated_at: DateTime<Utc>,
    /// When the account was deleted; it is removed for good after the server's grace period.
    #[serde(deserialize_with = "crate::dates::serde_dates::optional_utc")]
    pub deleted_at: Option<DateTime<Utc>>,
}
