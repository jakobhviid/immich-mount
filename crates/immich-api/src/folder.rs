//! Folders: the Assets by the folder their original is stored in on the server, as Immich's folder view shows them.
//!
//! The paths are the server's own (`/data/upload/<user id>/03/c0` for uploads, the library's path for an external
//! library), so they mean something only for external libraries, whose folders the user chose.
//!
//! Quirks, seen on v3.3.6:
//! - the list of paths is sorted, and holds only folders with Assets directly in them, not their parents
//! - a folder's Assets are those directly in it, not in folders below; a trailing `/` makes no difference, and an
//!   unknown path answers an empty list

use reqwest::Method;

use crate::asset_info::AssetDetails;
use crate::client::Client;
use crate::error::ApiError;
use crate::wire::{endpoints, Endpoint};

pub(crate) const ENDPOINTS: &[Endpoint] = endpoints![
    GET "/view/folder/unique-paths" "folder.read",
    GET "/view/folder" "folder.read",
];

impl Client {
    /// Every folder on the server that holds Assets of the user (`GET /view/folder/unique-paths`). Needs
    /// `folder.read`.
    pub async fn folder_paths(&self) -> Result<Vec<String>, ApiError> {
        self.call(Method::GET, &["view", "folder", "unique-paths"], &[]).await
    }

    /// The Assets directly in one folder (`GET /view/folder?path=`). Needs `folder.read`.
    pub async fn folder_assets(&self, path: &str) -> Result<Vec<AssetDetails>, ApiError> {
        self.call(Method::GET, &["view", "folder"], &[("path", path)]).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_server::{CannedResponse, FakeServer};

    #[tokio::test]
    async fn a_folder_path_goes_in_the_query() {
        let server = FakeServer::start(vec![CannedResponse::json(200, r#"[{"id":"a1","type":"IMAGE"}]"#)]).await;
        let client = Client::new(&server.url, None, "test").unwrap();
        let assets = client.folder_assets("/photos/2024 trip").await.unwrap();
        assert_eq!(assets[0].id, "a1");
        assert_eq!(server.requests()[0].target, "/api/view/folder?path=%2Fphotos%2F2024+trip");
    }
}
