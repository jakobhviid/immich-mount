//! What the contract tests share: the lab server's credentials, clients, and lookups in the seeded library.
//!
//! Run against a lab server with the values from its `.env.local` (the `lab-immich-server` skill), one test at a
//! time, since tests that change the library put it back when they finish and must not overlap:
//!
//! ```sh
//! set -a; . lab/immich-server/.env.local; set +a
//! cargo test -p immich-api --tests -- --ignored --test-threads=1
//! ```

#![allow(dead_code)] // each test file uses its own part of this module

use chrono::{DateTime, Utc};
use immich_api::{ApiError, AssetQuery, Client, Credential, Visibility};

pub const USER_AGENT: &str = "ImmichMount/contract-tests (Linux; lab)";

/// The current time; chrono is built without its `clock` feature.
pub fn now() -> DateTime<Utc> {
    DateTime::<Utc>::from(std::time::SystemTime::now())
}

pub fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is not set; source lab/immich-server/.env.local"))
}

pub fn client_with(credential: Option<Credential>) -> Client {
    Client::new(&env("IMMICH_URL"), credential, USER_AGENT).unwrap()
}

/// The read-only key: `album.read`, `asset.read`, `asset.view`, `asset.download`, `person.read`.
pub fn readonly() -> Client {
    client_with(Some(Credential::ApiKey(env("IMMICH_API_KEY_READONLY"))))
}

/// The full-access key of the admin account.
pub fn full() -> Client {
    client_with(Some(Credential::ApiKey(env("IMMICH_API_KEY_FULL"))))
}

/// A password session of the admin account; sign it out when done.
pub async fn session() -> (Client, immich_api::LoginResponse) {
    let login = client_with(None)
        .login_password(&env("IMMICH_ADMIN_EMAIL"), &env("IMMICH_ADMIN_PASSWORD"))
        .await
        .unwrap();
    let client = client_with(Some(Credential::Session(login.access_token.clone())));
    (client, login)
}

pub async fn album_id(client: &Client, name: &str) -> String {
    client
        .albums()
        .await
        .unwrap()
        .into_iter()
        .find(|album| album.name == name)
        .unwrap_or_else(|| panic!("no album named {name}"))
        .id
}

/// The ids of the Assets on the timeline, newest first.
pub async fn timeline_ids(client: &Client) -> Vec<String> {
    let query = AssetQuery { visibility: Some(Visibility::Timeline), ..AssetQuery::default() };
    client
        .search_all_assets(&query, 1000)
        .await
        .unwrap()
        .into_iter()
        .map(|asset| asset.id)
        .collect()
}

/// The id of the Asset uploaded under this file name; panics unless there is exactly one.
pub async fn asset_named(client: &Client, file_name: &str) -> String {
    let all = client.search_all_assets(&AssetQuery::default(), 1000).await.unwrap();
    let mut matching = all.into_iter().filter(|asset| asset.original_file_name == file_name);
    let found = matching.next().unwrap_or_else(|| panic!("no Asset named {file_name}"));
    assert!(matching.next().is_none(), "more than one Asset named {file_name}");
    found.id
}

/// The permission Immich requires for an endpoint, from the crate's own table.
pub fn permission_of(method: &str, path: &str) -> &'static str {
    immich_api::endpoints()
        .into_iter()
        .find(|endpoint| endpoint.method == method && endpoint.path == path)
        .unwrap_or_else(|| panic!("{method} {path} is not in immich_api::endpoints()"))
        .permission
        .unwrap_or_else(|| panic!("{method} {path} needs no permission"))
}

/// Assert that a call was refused for lacking exactly the permission the table names for its endpoint.
#[track_caller]
pub fn assert_refused<T: std::fmt::Debug>(result: Result<T, ApiError>, method: &str, path: &str) {
    let expected = permission_of(method, path);
    match result {
        Err(ApiError::Forbidden { missing_permission }) => {
            assert_eq!(missing_permission.as_deref(), Some(expected), "{method} {path}")
        }
        other => panic!("{method} {path}: expected a refusal for `{expected}`, got {other:?}"),
    }
}

/// Wait, up to a minute, until the server's job queues are idle: changes that queue background work (tagging,
/// edits, uploads) are settled then.
pub async fn wait_for_idle_queues(admin: &Client) {
    for _ in 0..120 {
        if admin.queues().await.unwrap().iter().all(immich_api::Queue::is_idle) {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    panic!("the job queues did not go idle within a minute");
}
