//! The crate's endpoint table against the server's own OpenAPI document (`/api/spec.json`): every endpoint the
//! crate calls must exist on the server, with the permission the table (and the method's documentation) names.
//! Run it against each supported version (`IMMICH_VERSION=v3.2.0 lab/immich-server/up.sh`, and the current one);
//! it is how a renamed or removed endpoint shows up before a user meets it. See `tests/common/mod.rs` to run.

mod common;

use common::env;

#[tokio::test]
#[ignore = "needs the lab server; see tests/common/mod.rs"]
async fn every_endpoint_exists_with_its_permission() {
    let url = format!("{}/api/spec.json", env("IMMICH_URL").trim_end_matches('/'));
    let spec: serde_json::Value = reqwest::get(url).await.unwrap().json().await.unwrap();
    let version = spec["info"]["version"].as_str().unwrap_or("?").to_owned();
    let mut problems = Vec::new();
    let mut deprecated = Vec::new();
    for endpoint in immich_api::endpoints() {
        let operation = &spec["paths"][endpoint.path][endpoint.method.to_ascii_lowercase()];
        if operation.is_null() {
            problems.push(format!("{} {} is missing", endpoint.method, endpoint.path));
            continue;
        }
        let permission = operation["x-immich-permission"].as_str();
        if permission != endpoint.permission {
            problems.push(format!(
                "{} {} needs {permission:?}, the table says {:?}",
                endpoint.method, endpoint.path, endpoint.permission
            ));
        }
        if operation["deprecated"].as_bool() == Some(true) {
            deprecated.push(format!("{} {}", endpoint.method, endpoint.path));
        }
    }
    println!("server spec {version}: {} endpoints checked", immich_api::endpoints().len());
    println!("deprecated on this server, still used: {}", deprecated.join(", "));
    assert!(problems.is_empty(), "against {version}:\n{}", problems.join("\n"));
}
