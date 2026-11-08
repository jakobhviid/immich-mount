//! Notifications: what the server tells the user in its web app's bell (album invites and updates, failed jobs,
//! messages from the admin).
//!
//! Quirks, seen on v3.3.6:
//! - `data` arrives as JSON text inside the JSON (`"data": "{\"albumId\": ...}"`); the model decodes it
//! - `GET /notifications/{id}` answers 400 `Notification not found` for every notification, read or not

use chrono::{DateTime, Utc};
use reqwest::Method;
use serde::{Deserialize, Serialize};

use crate::client::Client;
use crate::dates::serde_dates;
use crate::error::ApiError;
use crate::wire::{endpoints, wire_enum, Endpoint};

pub(crate) const ENDPOINTS: &[Endpoint] = endpoints![
    GET "/notifications" "notification.read",
    GET "/notifications/{id}" "notification.read",
    PUT "/notifications/{id}" "notification.update",
    PUT "/notifications" "notification.update",
    DELETE "/notifications/{id}" "notification.delete",
    DELETE "/notifications" "notification.delete",
];

wire_enum! {
    /// How a notification should look.
    pub enum NotificationLevel {
        Info = "info",
        Success = "success",
        Warning = "warning",
        Error = "error",
    }
}

wire_enum! {
    /// What a notification is about.
    pub enum NotificationKind {
        AlbumInvite = "AlbumInvite",
        AlbumUpdate = "AlbumUpdate",
        JobFailed = "JobFailed",
        BackupFailed = "BackupFailed",
        SystemMessage = "SystemMessage",
        Custom = "Custom",
    }
}

/// A notification (`NotificationDto`).
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Notification {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub level: NotificationLevel,
    #[serde(rename = "type")]
    pub kind: NotificationKind,
    /// What the notification refers to, such as `{"albumId": ...}`. The server sends it as JSON text; it is
    /// decoded here.
    #[serde(deserialize_with = "json_or_text")]
    pub data: serde_json::Value,
    #[serde(deserialize_with = "serde_dates::utc")]
    pub created_at: DateTime<Utc>,
    /// `None` while unread.
    #[serde(deserialize_with = "serde_dates::optional_utc")]
    pub read_at: Option<DateTime<Utc>>,
}

/// Which notifications [`Client::notifications`] lists.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NotificationFilter {
    pub unread_only: bool,
    pub level: Option<NotificationLevel>,
    pub kind: Option<NotificationKind>,
}

impl Client {
    /// The user's notifications, newest first (`GET /notifications`). Needs `notification.read`.
    pub async fn notifications(&self, filter: &NotificationFilter) -> Result<Vec<Notification>, ApiError> {
        let mut query: Vec<(&str, &str)> = Vec::new();
        if filter.unread_only {
            query.push(("unread", "true"));
        }
        if let Some(level) = &filter.level {
            query.push(("level", level.as_str()));
        }
        if let Some(kind) = &filter.kind {
            query.push(("type", kind.as_str()));
        }
        self.call(Method::GET, &["notifications"], &query).await
    }

    /// One notification (`GET /notifications/{id}`). Needs `notification.read`. On v3.3.6 this answers
    /// [`ApiError::NotFound`] for every notification (400 `Notification not found`); read them from
    /// [`Client::notifications`].
    pub async fn notification(&self, id: &str) -> Result<Notification, ApiError> {
        self.call(Method::GET, &["notifications", id], &[]).await
    }

    /// Mark a notification read at a time, or unread with `None` (`PUT /notifications/{id}`). Needs
    /// `notification.update`.
    pub async fn mark_notification(&self, id: &str, read_at: Option<DateTime<Utc>>) -> Result<Notification, ApiError> {
        self.call_with(Method::PUT, &["notifications", id], &[], &ReadAt { ids: None, read_at }).await
    }

    /// Mark notifications read at a time, or unread with `None` (`PUT /notifications`). Needs
    /// `notification.update`.
    pub async fn mark_notifications(&self, ids: &[String], read_at: Option<DateTime<Utc>>) -> Result<(), ApiError> {
        self.call_unit_with(Method::PUT, &["notifications"], &[], &ReadAt { ids: Some(ids), read_at }).await
    }

    /// Delete a notification (`DELETE /notifications/{id}`). Needs `notification.delete`.
    pub async fn delete_notification(&self, id: &str) -> Result<(), ApiError> {
        self.call_unit(Method::DELETE, &["notifications", id], &[]).await
    }

    /// Delete notifications (`DELETE /notifications`). Needs `notification.delete`.
    pub async fn delete_notifications(&self, ids: &[String]) -> Result<(), ApiError> {
        self.call_unit_with(Method::DELETE, &["notifications"], &[], &serde_json::json!({ "ids": ids }))
            .await
    }
}

/// `data` as the server sends it, JSON text, decoded; a value that is already JSON is kept.
fn json_or_text<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<serde_json::Value, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(match &value {
        serde_json::Value::String(text) => serde_json::from_str(text).unwrap_or(value),
        _ => value,
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReadAt<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    ids: Option<&'a [String]>,
    #[serde(serialize_with = "serde_dates::send_utc")]
    read_at: Option<DateTime<Utc>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_sent_as_text_is_decoded() {
        let json = br#"{"id":"n","title":"Shared Album Invitation","level":"success","type":"AlbumInvite",
            "data":"{\"albumId\":\"a\"}","createdAt":"2026-11-08T16:12:48.384Z"}"#;
        let notification: Notification = crate::client::decode_json(json, "notifications").unwrap();
        assert_eq!(notification.data["albumId"], "a");
        assert_eq!(notification.kind, NotificationKind::AlbumInvite);
        assert_eq!(notification.read_at, None);
    }
}
