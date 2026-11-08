//! Memories: "on this day" collections of Assets (and, from v3.3, birthdays), made by the server each night or by
//! hand, shown between their `show_at` and `hide_at`.
//!
//! Quirks, seen on v3.3.6:
//! - a memory without Assets is left out of [`Client::memories`] and of the statistics, though it exists
//!   ([`Client::memory`] reads it)
//! - `for` (a date) keeps the memories whose show window holds that date, and those without a window
//! - adding an Asset that is already in a memory answers `duplicate` for it; removing an unknown one `not_found`
//! - `PUT /memories/{id}` is marked deprecated with no replacement yet; it is the only way on v3.2
//! - [`MemoryKind::Birthday`] and [`MemoryData`]'s person fields are v3.3 additions; v3.2 knows only on-this-day

use chrono::{DateTime, NaiveDate, Utc};
use reqwest::Method;
use serde::{Deserialize, Serialize};

use crate::asset_info::AssetDetails;
use crate::client::Client;
use crate::error::ApiError;
use crate::wire::{endpoints, wire_enum, BulkIdResult, BulkIds, Endpoint};

pub(crate) const ENDPOINTS: &[Endpoint] = endpoints![
    GET "/memories" "memory.read",
    GET "/memories/{id}" "memory.read",
    POST "/memories" "memory.create",
    PUT "/memories/{id}" "memory.update",
    DELETE "/memories/{id}" "memory.delete",
    PUT "/memories/{id}/assets" "memoryAsset.create",
    DELETE "/memories/{id}/assets" "memoryAsset.delete",
    GET "/memories/statistics" "memory.statistics",
];

wire_enum! {
    /// What a memory is about.
    pub enum MemoryKind {
        OnThisDay = "on_this_day",
        Birthday = "birthday",
    }
}

wire_enum! {
    /// The order [`Client::memories`] lists in.
    pub enum MemoryOrder {
        Desc = "desc",
        Asc = "asc",
        Random = "random",
    }
}

/// What a memory recalls: the year, and for a birthday, whose.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MemoryData {
    pub year: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub person_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub person_name: Option<String>,
}

/// A memory (`MemoryResponseDto`).
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Memory {
    pub id: String,
    pub owner_id: String,
    #[serde(rename = "type")]
    pub kind: MemoryKind,
    pub data: MemoryData,
    /// The day it recalls.
    #[serde(deserialize_with = "crate::dates::serde_dates::utc")]
    pub memory_at: DateTime<Utc>,
    /// When it starts to be shown; `None` for always.
    #[serde(deserialize_with = "crate::dates::serde_dates::optional_utc")]
    pub show_at: Option<DateTime<Utc>>,
    /// When it stops being shown.
    #[serde(deserialize_with = "crate::dates::serde_dates::optional_utc")]
    pub hide_at: Option<DateTime<Utc>>,
    /// When the user looked at it; `None` if not yet.
    #[serde(deserialize_with = "crate::dates::serde_dates::optional_utc")]
    pub seen_at: Option<DateTime<Utc>>,
    #[serde(deserialize_with = "crate::dates::serde_dates::optional_utc")]
    pub deleted_at: Option<DateTime<Utc>>,
    /// Kept by the user, so the server does not replace it.
    pub is_saved: bool,
    pub assets: Vec<AssetDetails>,
    #[serde(deserialize_with = "crate::dates::serde_dates::utc")]
    pub created_at: DateTime<Utc>,
    #[serde(deserialize_with = "crate::dates::serde_dates::utc")]
    pub updated_at: DateTime<Utc>,
}

/// Which memories [`Client::memories`] lists and [`Client::memory_statistics`] counts; `None` does not filter.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemoryFilter {
    /// Only those shown on this day.
    pub shown_on: Option<NaiveDate>,
    pub saved: Option<bool>,
    pub trashed: Option<bool>,
    /// Only those whose show window has not started yet.
    pub upcoming: Option<bool>,
    pub kind: Option<MemoryKind>,
    pub order: Option<MemoryOrder>,
    /// Memories per page, with `page` (from 1).
    pub size: Option<u32>,
    pub page: Option<u32>,
}

impl MemoryFilter {
    fn to_query(&self) -> Vec<(&'static str, String)> {
        let mut query = Vec::new();
        if let Some(day) = &self.shown_on {
            query.push(("for", day.format("%Y-%m-%d").to_string()));
        }
        if let Some(saved) = self.saved {
            query.push(("isSaved", saved.to_string()));
        }
        if let Some(trashed) = self.trashed {
            query.push(("isTrashed", trashed.to_string()));
        }
        if let Some(upcoming) = self.upcoming {
            query.push(("isUpcoming", upcoming.to_string()));
        }
        if let Some(kind) = &self.kind {
            query.push(("type", kind.as_str().to_owned()));
        }
        if let Some(order) = &self.order {
            query.push(("order", order.as_str().to_owned()));
        }
        if let Some(size) = self.size {
            query.push(("size", size.to_string()));
        }
        if let Some(page) = self.page {
            query.push(("page", page.to_string()));
        }
        query
    }
}

/// A memory made by hand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewMemory {
    #[serde(rename = "type")]
    pub kind: MemoryKind,
    pub data: MemoryData,
    #[serde(serialize_with = "crate::dates::serde_dates::send_instant")]
    pub memory_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub asset_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_saved: Option<bool>,
    #[serde(serialize_with = "crate::dates::serde_dates::send_utc", skip_serializing_if = "Option::is_none")]
    pub show_at: Option<DateTime<Utc>>,
    #[serde(serialize_with = "crate::dates::serde_dates::send_utc", skip_serializing_if = "Option::is_none")]
    pub hide_at: Option<DateTime<Utc>>,
    #[serde(serialize_with = "crate::dates::serde_dates::send_utc", skip_serializing_if = "Option::is_none")]
    pub seen_at: Option<DateTime<Utc>>,
}

/// Changes to a memory; `None` leaves a field as it is.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryChanges {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_saved: Option<bool>,
    #[serde(serialize_with = "crate::dates::serde_dates::send_utc", skip_serializing_if = "Option::is_none")]
    pub memory_at: Option<DateTime<Utc>>,
    /// Mark it seen.
    #[serde(serialize_with = "crate::dates::serde_dates::send_utc", skip_serializing_if = "Option::is_none")]
    pub seen_at: Option<DateTime<Utc>>,
}

impl Client {
    /// The memories that match, each with its Assets (`GET /memories`). Needs `memory.read`.
    pub async fn memories(&self, filter: &MemoryFilter) -> Result<Vec<Memory>, ApiError> {
        let query = filter.to_query();
        let borrowed: Vec<(&str, &str)> = query.iter().map(|(name, value)| (*name, value.as_str())).collect();
        self.call(Method::GET, &["memories"], &borrowed).await
    }

    /// How many memories match (`GET /memories/statistics`). Needs `memory.statistics`.
    pub async fn memory_statistics(&self, filter: &MemoryFilter) -> Result<u64, ApiError> {
        #[derive(Deserialize, Default)]
        #[serde(default)]
        struct Statistics {
            total: u64,
        }
        let query = filter.to_query();
        let borrowed: Vec<(&str, &str)> = query.iter().map(|(name, value)| (*name, value.as_str())).collect();
        let answer: Statistics = self.call(Method::GET, &["memories", "statistics"], &borrowed).await?;
        Ok(answer.total)
    }

    /// One memory (`GET /memories/{id}`). Needs `memory.read`.
    pub async fn memory(&self, id: &str) -> Result<Memory, ApiError> {
        self.call(Method::GET, &["memories", id], &[]).await
    }

    /// Make a memory (`POST /memories`). Needs `memory.create`.
    pub async fn create_memory(&self, memory: &NewMemory) -> Result<Memory, ApiError> {
        self.call_with(Method::POST, &["memories"], &[], memory).await
    }

    /// Save a memory, mark it seen, or move its day (`PUT /memories/{id}`). Needs `memory.update`.
    pub async fn update_memory(&self, id: &str, changes: &MemoryChanges) -> Result<Memory, ApiError> {
        self.call_with(Method::PUT, &["memories", id], &[], changes).await
    }

    /// Delete a memory; its Assets stay (`DELETE /memories/{id}`). Needs `memory.delete`.
    pub async fn delete_memory(&self, id: &str) -> Result<(), ApiError> {
        self.call_unit(Method::DELETE, &["memories", id], &[]).await
    }

    /// Add Assets to a memory, one outcome per Asset (`PUT /memories/{id}/assets`). Needs `memoryAsset.create`.
    pub async fn add_memory_assets(&self, id: &str, asset_ids: &[String]) -> Result<Vec<BulkIdResult>, ApiError> {
        self.call_with(Method::PUT, &["memories", id, "assets"], &[], &BulkIds { ids: asset_ids })
            .await
    }

    /// Take Assets out of a memory, one outcome per Asset (`DELETE /memories/{id}/assets`). Needs
    /// `memoryAsset.delete`.
    pub async fn remove_memory_assets(&self, id: &str, asset_ids: &[String]) -> Result<Vec<BulkIdResult>, ApiError> {
        self.call_with(Method::DELETE, &["memories", id, "assets"], &[], &BulkIds { ids: asset_ids })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dates::parse_utc;
    use crate::test_server::{CannedResponse, FakeServer};

    #[test]
    fn a_new_memory_sends_dates_in_our_format_and_leaves_out_what_is_unset() {
        let memory = NewMemory {
            kind: MemoryKind::OnThisDay,
            data: MemoryData { year: 2024, ..MemoryData::default() },
            memory_at: parse_utc("2024-06-25T00:00:00Z").unwrap(),
            asset_ids: vec!["a1".to_owned()],
            is_saved: None,
            show_at: parse_utc("2026-06-25T00:00:00Z"),
            hide_at: None,
            seen_at: None,
        };
        assert_eq!(
            serde_json::to_value(&memory).unwrap(),
            serde_json::json!({
                "type": "on_this_day",
                "data": {"year": 2024},
                "memoryAt": "2024-06-25T00:00:00.000Z",
                "assetIds": ["a1"],
                "showAt": "2026-06-25T00:00:00.000Z",
            })
        );
    }

    #[tokio::test]
    async fn the_filter_becomes_the_query() {
        let answer = r#"[{"id":"m1","type":"birthday","data":{"year":1990,"personId":"p1","personName":"Ada"},
            "memoryAt":"2026-05-17T00:00:00.000Z","isSaved":false,"assets":[],"showAt":null}]"#;
        let server = FakeServer::start(vec![CannedResponse::json(200, answer)]).await;
        let client = Client::new(&server.url, None, "test").unwrap();
        let filter = MemoryFilter {
            shown_on: NaiveDate::from_ymd_opt(2026, 6, 25),
            saved: Some(true),
            order: Some(MemoryOrder::Random),
            size: Some(10),
            ..MemoryFilter::default()
        };
        let memories = client.memories(&filter).await.unwrap();
        assert_eq!(memories[0].kind, MemoryKind::Birthday);
        assert_eq!(memories[0].data.person_name.as_deref(), Some("Ada"));
        assert_eq!(memories[0].show_at, None);
        assert_eq!(server.requests()[0].target, "/api/memories?for=2026-06-25&isSaved=true&order=random&size=10");
    }
}
