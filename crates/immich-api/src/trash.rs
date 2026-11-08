//! The trash: restoring Assets from it, and emptying it. Moving Assets into it is [`Client::delete_assets`].
//!
//! Each call answers with how many Assets it affected. Emptying and restoring everything run as server jobs, so
//! the count is what was queued, and the trash may take a moment to show the result.

use reqwest::Method;
use serde::Deserialize;

use crate::client::Client;
use crate::error::ApiError;
use crate::wire::{endpoints, BulkIds, Endpoint};

pub(crate) const ENDPOINTS: &[Endpoint] = endpoints![
    POST "/trash/empty" "asset.delete",
    POST "/trash/restore" "asset.delete",
    POST "/trash/restore/assets" "asset.delete",
];

#[derive(Deserialize, Default)]
#[serde(default)]
struct CountWire {
    count: u64,
}

impl Client {
    /// Delete everything in the trash for good (`POST /trash/empty`); returns how many Assets. Needs
    /// `asset.delete`.
    pub async fn empty_trash(&self) -> Result<u64, ApiError> {
        let answer: CountWire = self.call(Method::POST, &["trash", "empty"], &[]).await?;
        Ok(answer.count)
    }

    /// Restore everything in the trash (`POST /trash/restore`); returns how many Assets. Needs `asset.delete`.
    pub async fn restore_trash(&self) -> Result<u64, ApiError> {
        let answer: CountWire = self.call(Method::POST, &["trash", "restore"], &[]).await?;
        Ok(answer.count)
    }

    /// Restore these Assets from the trash (`POST /trash/restore/assets`); returns how many. Needs `asset.delete`.
    pub async fn restore_assets(&self, ids: &[String]) -> Result<u64, ApiError> {
        let answer: CountWire = self
            .call_with(Method::POST, &["trash", "restore", "assets"], &[], &BulkIds { ids })
            .await?;
        Ok(answer.count)
    }
}
