//! Partners: users who share their whole library with each other. Sharing goes one way per partnership: the user
//! shares with someone (`shared-by` from their side), and sees the libraries of those who share with them
//! (`shared-with`), optionally on their own timeline.
//!
//! Quirks, seen on v3.3.6:
//! - `POST /partners` takes `{sharedWithId}`; the older `POST /partners/{id}` is deprecated, and v3.2 has both
//! - `inTimeline` is set by the user who receives the sharing, on the partner who shares

use reqwest::Method;
use serde::{Deserialize, Serialize};

use crate::client::Client;
use crate::error::ApiError;
use crate::user::AvatarColor;
use crate::wire::{endpoints, wire_enum, Endpoint};

pub(crate) const ENDPOINTS: &[Endpoint] = endpoints![
    GET "/partners" "partner.read",
    POST "/partners" "partner.create",
    PUT "/partners/{id}" "partner.update",
    DELETE "/partners/{id}" "partner.delete",
];

wire_enum! {
    /// Which side of a partnership to list.
    pub enum PartnerDirection {
        /// The users this user shares with.
        SharedBy = "shared-by",
        /// The users who share with this user.
        SharedWith = "shared-with",
    }
}

/// A partner (`PartnerResponseDto`): the other user, and whether their library shows on this user's timeline.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Partner {
    /// The other user's id.
    pub id: String,
    pub name: String,
    pub email: String,
    pub avatar_color: AvatarColor,
    pub profile_image_path: String,
    /// Sent for partners who share with this user.
    pub in_timeline: Option<bool>,
}

impl Client {
    /// The user's partners on one side (`GET /partners?direction=`). Needs `partner.read`.
    pub async fn partners(&self, direction: &PartnerDirection) -> Result<Vec<Partner>, ApiError> {
        self.call(Method::GET, &["partners"], &[("direction", direction.as_str())]).await
    }

    /// Share this user's library with another user (`POST /partners`). Needs `partner.create`.
    pub async fn add_partner(&self, user_id: &str) -> Result<Partner, ApiError> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Body<'a> {
            shared_with_id: &'a str,
        }
        self.call_with(Method::POST, &["partners"], &[], &Body { shared_with_id: user_id }).await
    }

    /// Show or hide a sharing partner's Assets on this user's timeline (`PUT /partners/{id}`). Needs
    /// `partner.update`.
    pub async fn set_partner_in_timeline(&self, user_id: &str, in_timeline: bool) -> Result<Partner, ApiError> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Body {
            in_timeline: bool,
        }
        self.call_with(Method::PUT, &["partners", user_id], &[], &Body { in_timeline }).await
    }

    /// Stop sharing with a user (`DELETE /partners/{id}`). Needs `partner.delete`.
    pub async fn remove_partner(&self, user_id: &str) -> Result<(), ApiError> {
        self.call_unit(Method::DELETE, &["partners", user_id], &[]).await
    }
}
