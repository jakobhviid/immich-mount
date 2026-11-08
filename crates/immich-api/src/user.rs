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
