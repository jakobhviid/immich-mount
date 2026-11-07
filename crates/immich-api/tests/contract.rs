//! Contract tests: the client against a real Immich server, seeded by `lab/immich-server/seed.sh`.
//!
//! All `#[ignore]`d, so a plain `cargo test` never needs the network. Run them against a lab server with the
//! values from its `.env.local`, one at a time (the upload test adds and removes an Asset, which moves counts):
//!
//! ```sh
//! set -a; . lab/immich-server/.env.local; set +a
//! cargo test -p immich-api --test contract -- --ignored --test-threads=1
//! ```
//!
//! Environment: `IMMICH_URL`, `IMMICH_API_KEY_READONLY`, `IMMICH_API_KEY_FULL`, `IMMICH_ADMIN_EMAIL`,
//! `IMMICH_ADMIN_PASSWORD`. The expected numbers are the seeded library's (the `lab-immich-server`
//! skill, "What is in it"): 51 Assets not trashed, 50 on the timeline, one archived, one trashed, the albums
//! "Holiday Mix" (5) and "Persons" (3), one person, 7 countries.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use immich_api::{
    ApiError, AssetQuery, Client, Credential, Fetched, SyncRecord, ThumbnailSize, TimelineScope, UploadCheck,
    UploadOutcome, Visibility, YearMonth,
};

const USER_AGENT: &str = "ImmichMount/contract-tests (Linux; lab)";

/// The current time; chrono is built without its `clock` feature.
fn now() -> DateTime<Utc> {
    DateTime::<Utc>::from(std::time::SystemTime::now())
}

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is not set; source lab/immich-server/.env.local"))
}

fn client_with(credential: Option<Credential>) -> Client {
    Client::new(&env("IMMICH_URL"), credential, USER_AGENT).unwrap()
}

fn readonly() -> Client {
    client_with(Some(Credential::ApiKey(env("IMMICH_API_KEY_READONLY"))))
}

fn full() -> Client {
    client_with(Some(Credential::ApiKey(env("IMMICH_API_KEY_FULL"))))
}

async fn session() -> (Client, immich_api::LoginResponse) {
    let login = client_with(None)
        .login_password(&env("IMMICH_ADMIN_EMAIL"), &env("IMMICH_ADMIN_PASSWORD"))
        .await
        .unwrap();
    let client = client_with(Some(Credential::Session(login.access_token.clone())));
    (client, login)
}

/// An endpoint this crate does not wrap (trash, delete), called with the full-access key.
async fn admin_call(method: reqwest::Method, path: &str, body: serde_json::Value) -> reqwest::StatusCode {
    let url = format!("{}/api{path}", env("IMMICH_URL").trim_end_matches('/'));
    reqwest::Client::new()
        .request(method, url)
        .header("x-api-key", env("IMMICH_API_KEY_FULL"))
        .json(&body)
        .send()
        .await
        .unwrap()
        .status()
}

async fn album_id(client: &Client, name: &str) -> String {
    let albums = client.albums().await.unwrap();
    albums
        .into_iter()
        .find(|album| album.name == name)
        .unwrap_or_else(|| panic!("no album named {name}"))
        .id
}

#[tokio::test]
#[ignore = "needs the lab server; see the header of tests/contract.rs"]
async fn server_version_is_supported() {
    let version = client_with(None).server_version().await.unwrap();
    println!("server version {version}");
    assert!(version.is_supported(), "{version} is below the floor");
}

#[tokio::test]
#[ignore = "needs the lab server; see the header of tests/contract.rs"]
async fn server_features_without_a_credential() {
    let features = client_with(None).server_features().await.unwrap();
    assert!(features.password_login);
    assert!(features.trash);
    assert!(features.reverse_geocoding);
    assert!(!features.oauth, "the lab has no identity provider");
}

#[tokio::test]
#[ignore = "needs the lab server; see the header of tests/contract.rs"]
async fn server_config_without_a_credential() {
    let config = client_with(None).server_config().await.unwrap();
    assert_eq!(config.trash_days, 30);
    assert_eq!(config.oauth_button_text, "Login with OAuth");
}

#[tokio::test]
#[ignore = "needs the lab server; see the header of tests/contract.rs"]
async fn api_key_permissions_per_credential() {
    let readonly_permissions = readonly().api_key_permissions().await.unwrap();
    assert_eq!(
        readonly_permissions,
        ["album.read", "asset.read", "asset.view", "asset.download", "person.read"]
    );
    assert_eq!(full().api_key_permissions().await.unwrap(), ["all"]);

    let (session, _) = session().await;
    assert_eq!(
        session.api_key_permissions().await.unwrap_err(),
        ApiError::Forbidden { missing_permission: None }
    );
    session.logout().await.unwrap();

    let wrong_key = client_with(Some(Credential::ApiKey("not-a-real-key".into())));
    assert_eq!(wrong_key.api_key_permissions().await.unwrap_err(), ApiError::Unauthorized);
    assert_eq!(client_with(None).api_key_permissions().await.unwrap_err(), ApiError::Unauthorized);
}

#[tokio::test]
#[ignore = "needs the lab server; see the header of tests/contract.rs"]
async fn time_buckets_count_the_timeline() {
    let client = readonly();
    let buckets = client.time_buckets(&TimelineScope::Library).await.unwrap();
    let total: u32 = buckets.iter().map(|bucket| bucket.count).sum();
    assert_eq!(total, 50, "timeline visibility excludes the archived and trashed Assets");
    let archive_buckets = client.time_buckets(&TimelineScope::Archive).await.unwrap();
    assert_eq!(archive_buckets.iter().map(|bucket| bucket.count).sum::<u32>(), 1, "the one archived Asset");
    assert_eq!(buckets.len(), 40);
    assert!(buckets.windows(2).all(|pair| pair[0].month > pair[1].month), "newest first");
    assert!(buckets.iter().any(|bucket| bucket.month == YearMonth::new(1970, 1).unwrap()), "no-date.jpg");

    let holiday = client
        .time_buckets(&TimelineScope::Album(album_id(&client, "Holiday Mix").await))
        .await
        .unwrap();
    assert_eq!(holiday.iter().map(|bucket| bucket.count).sum::<u32>(), 5);

    let person = client.people().await.unwrap().remove(0);
    let person_buckets = client.time_buckets(&TimelineScope::Person(person.id)).await.unwrap();
    assert_eq!(person_buckets.iter().map(|bucket| bucket.count).sum::<u32>(), 1);
}

#[tokio::test]
#[ignore = "needs the lab server; see the header of tests/contract.rs"]
async fn time_bucket_ids_match_counts_and_local_months() {
    let client = readonly();
    let buckets = client.time_buckets(&TimelineScope::Library).await.unwrap();
    let mut all_ids = HashSet::new();
    for bucket in &buckets {
        let assets = client.time_bucket(&TimelineScope::Library, bucket.month).await.unwrap();
        assert_eq!(assets.len() as u32, bucket.count, "bucket {}", bucket.month);
        for asset in assets {
            assert_eq!(
                YearMonth::of(&asset.local_date_time()),
                bucket.month,
                "buckets group by local time ({})",
                asset.id
            );
            assert!(!asset.is_trashed);
            all_ids.insert(asset.id);
        }
    }
    assert_eq!(all_ids.len(), 50);
}

#[tokio::test]
#[ignore = "needs the lab server; see the header of tests/contract.rs"]
async fn search_finds_by_visibility_album_person_place_and_date() {
    let client = readonly();
    let timeline = client
        .search_all_assets(&AssetQuery { visibility: Some(Visibility::Timeline), ..AssetQuery::default() }, 1000)
        .await
        .unwrap();
    assert_eq!(timeline.len(), 50);
    assert!(timeline.iter().all(|asset| !asset.is_trashed && asset.file_size.is_some()));

    let everything = client.search_all_assets(&AssetQuery::default(), 1000).await.unwrap();
    assert_eq!(everything.len(), 51, "archived included, trashed excluded");

    // A read-only key may ask for the archive alone: only `ne: archive` needs elevated permission.
    let archive = client
        .search_all_assets(&AssetQuery { visibility: Some(Visibility::Archive), ..AssetQuery::default() }, 1000)
        .await
        .unwrap();
    assert_eq!(archive.len(), 1);
    assert_eq!(archive[0].visibility, Visibility::Archive);

    // Small pages walk the cursor and find the same set.
    let paged = client
        .search_all_assets(&AssetQuery { visibility: Some(Visibility::Timeline), ..AssetQuery::default() }, 7)
        .await
        .unwrap();
    let paged_ids: HashSet<_> = paged.iter().map(|asset| asset.id.clone()).collect();
    let single_page_ids: HashSet<_> = timeline.iter().map(|asset| asset.id.clone()).collect();
    assert_eq!(paged_ids, single_page_ids);
    let first_page = client
        .search_assets(&AssetQuery::default(), 7, None)
        .await
        .unwrap();
    assert_eq!(first_page.assets.len(), 7);
    assert!(first_page.next_cursor.is_some());

    let holiday = AssetQuery {
        album_ids: Some(vec![album_id(&client, "Holiday Mix").await]),
        ..AssetQuery::default()
    };
    assert_eq!(client.search_all_assets(&holiday, 100).await.unwrap().len(), 5);

    let person = client.people().await.unwrap().remove(0);
    let with_person = AssetQuery {
        person_ids: Some(vec![person.id]),
        ..AssetQuery::default()
    };
    assert_eq!(client.search_all_assets(&with_person, 100).await.unwrap().len(), 1);

    let lyon = AssetQuery {
        city: Some("Lyon".into()),
        country: Some("France".into()),
        ..AssetQuery::default()
    };
    let in_lyon = client.search_all_assets(&lyon, 100).await.unwrap();
    assert!(in_lyon.len() >= 3, "the three IMG_0001.JPG photos are in Lyon");
    assert!(in_lyon.iter().all(|asset| asset.city.as_deref() == Some("Lyon")));
    let same_name = in_lyon.iter().filter(|asset| asset.original_file_name == "IMG_0001.JPG").count();
    assert_eq!(same_name, 3, "duplicate names survive");

    let january_1970 = YearMonth::new(1970, 1).unwrap();
    let start = january_1970.first_day().and_hms_opt(0, 0, 0).unwrap().and_utc();
    let end = january_1970.next().first_day().and_hms_opt(0, 0, 0).unwrap().and_utc();
    let dateless = client
        .search_all_assets(&AssetQuery { taken: Some(start..end), ..AssetQuery::default() }, 100)
        .await
        .unwrap();
    assert!(dateless.iter().any(|asset| asset.original_file_name == "no-date.jpg"));
}

#[tokio::test]
#[ignore = "needs the lab server; see the header of tests/contract.rs"]
async fn asset_by_id_and_unknown_ids() {
    let client = readonly();
    let found = client.search_assets(&AssetQuery::default(), 1, None).await.unwrap().assets.remove(0);
    let asset = client.asset(&found.id).await.unwrap();
    assert_eq!(asset.id, found.id);
    assert_eq!(asset.original_file_name, found.original_file_name);
    assert_eq!(asset.file_size, found.file_size, "GET /assets/{{id}} includes exifInfo");
    assert_eq!(asset.checksum.len(), 28, "base64 SHA-1");
    assert_eq!(
        client.asset("00000000-0000-4000-8000-000000000000").await.unwrap_err(),
        ApiError::NotFound
    );
}

#[tokio::test]
#[ignore = "needs the lab server; see the header of tests/contract.rs"]
async fn albums_people_and_places() {
    let client = readonly();
    let albums = client.albums().await.unwrap();
    let by_name: HashMap<_, _> = albums.iter().map(|album| (album.name.as_str(), album)).collect();
    assert_eq!(by_name["Holiday Mix"].asset_count, 5);
    assert_eq!(by_name["Persons"].asset_count, 3);
    assert!(by_name["Persons"].start_date.is_some());

    let people = client.people().await.unwrap();
    assert_eq!(people.len(), 1);
    assert_eq!(people[0].name, "Lab Person");
    assert!(!people[0].is_hidden);

    let countries = client.countries().await.unwrap();
    assert_eq!(
        countries,
        ["Australia", "Denmark", "France", "Germany", "Iceland", "Japan", "United States"]
    );
    assert_eq!(client.cities("Denmark").await.unwrap(), ["Århus", "Copenhagen", "Odense"]);
    assert_eq!(client.cities("United States").await.unwrap(), ["Weehawken"]);
}

#[tokio::test]
#[ignore = "needs the lab server; see the header of tests/contract.rs"]
async fn shared_albums_are_listed_with_owned_ones() {
    // A second user shares an album with the admin; the admin's list must include it.
    let suffix = now().timestamp_millis();
    let email = format!("friend-{suffix}@example.com");
    let password = format!("friend-password-{suffix}");
    let (admin, admin_login) = session().await;
    let admin_user_id = admin_login.user_id;

    let url = format!("{}/api", env("IMMICH_URL").trim_end_matches('/'));
    let http = reqwest::Client::new();
    let created: serde_json::Value = http
        .post(format!("{url}/admin/users"))
        .header("x-api-key", env("IMMICH_API_KEY_FULL"))
        .json(&serde_json::json!({ "email": email, "password": password, "name": "Friend" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let friend_user_id = created["id"].as_str().unwrap().to_owned();
    let friend = client_with(None).login_password(&email, &password).await.unwrap();
    let album: serde_json::Value = http
        .post(format!("{url}/albums"))
        .bearer_auth(&friend.access_token)
        .json(&serde_json::json!({
            "albumName": format!("Shared by a friend {suffix}"),
            "albumUsers": [{ "userId": admin_user_id, "role": "viewer" }]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let shared_album_id = album["id"].as_str().unwrap().to_owned();

    let albums = admin.albums().await.unwrap();
    let shared = albums.iter().find(|album| album.id == shared_album_id);
    let owned = albums.iter().find(|album| album.name == "Holiday Mix");

    // Clean up before asserting, so a failure leaves the lab as seeded.
    http.delete(format!("{url}/albums/{shared_album_id}"))
        .bearer_auth(&friend.access_token)
        .send()
        .await
        .unwrap();
    admin_call(
        reqwest::Method::DELETE,
        &format!("/admin/users/{friend_user_id}"),
        serde_json::json!({ "force": true }),
    )
    .await;
    admin.logout().await.unwrap();

    assert!(owned.is_some(), "owned albums are listed");
    let shared = shared.expect("albums shared with the user are listed");
    assert!(shared.shared);
}

#[tokio::test]
#[ignore = "needs the lab server; see the header of tests/contract.rs"]
async fn original_ranges_are_byte_exact() {
    let client = readonly();
    let assets = client.search_all_assets(&AssetQuery::default(), 1000).await.unwrap();
    // One image and one video, read whole in blocks, must hash to the server's checksum.
    let image = assets.iter().find(|asset| asset.kind == immich_api::AssetKind::Image).unwrap();
    let video = assets.iter().find(|asset| asset.kind == immich_api::AssetKind::Video).unwrap();
    for asset in [image, video] {
        let size = asset.file_size.unwrap();
        let (head_size, head_etag) = client.original_head(&asset.id).await.unwrap();
        assert_eq!(head_size, size, "HEAD size of {}", asset.original_file_name);
        assert!(head_etag.as_deref().is_some_and(|etag| etag.starts_with("W/")));

        let block = 256 * 1024;
        let mut content = Vec::with_capacity(size as usize);
        let mut start = 0;
        while start < size {
            let range = client.original_range(&asset.id, start, start + block - 1).await.unwrap();
            assert_eq!(range.total_size, size);
            assert_eq!(range.etag, head_etag);
            content.extend_from_slice(&range.bytes);
            start += range.bytes.len() as u64;
        }
        assert_eq!(content.len() as u64, size);
        assert_eq!(
            immich_api::Sha1Checksum::of_bytes(&content).to_base64(),
            asset.checksum,
            "content of {}",
            asset.original_file_name
        );

        let mid = client.original_range(&asset.id, size / 2, size / 2 + 99).await.unwrap();
        assert_eq!(&mid.bytes[..], &content[(size / 2) as usize..(size / 2 + 100) as usize]);

        assert_eq!(
            client.original_range(&asset.id, size + 10, size + 20).await.unwrap_err(),
            ApiError::RangeNotSatisfiable
        );
    }
    assert_eq!(
        client
            .original_range("00000000-0000-4000-8000-000000000000", 0, 10)
            .await
            .unwrap_err(),
        ApiError::NotFound
    );
}

#[tokio::test]
#[ignore = "needs the lab server; see the header of tests/contract.rs"]
async fn thumbnails_in_both_sizes() {
    let client = readonly();
    let asset = client.search_assets(&AssetQuery::default(), 1, None).await.unwrap().assets.remove(0);
    let (small, small_type) = client.thumbnail(&asset.id, ThumbnailSize::Thumbnail).await.unwrap();
    assert_eq!(small_type, "image/webp");
    assert_eq!(&small[..4], b"RIFF");
    let (large, large_type) = client.thumbnail(&asset.id, ThumbnailSize::Preview).await.unwrap();
    assert_eq!(large_type, "image/jpeg");
    assert_eq!(&large[..2], b"\xff\xd8");
    assert!(large.len() > small.len());
    assert_eq!(
        client
            .thumbnail("00000000-0000-4000-8000-000000000000", ThumbnailSize::Thumbnail)
            .await
            .unwrap_err(),
        ApiError::NotFound
    );
}

#[tokio::test]
#[ignore = "needs the lab server; see the header of tests/contract.rs"]
async fn etags_give_not_modified() {
    let client = readonly();
    let Fetched::Changed { etag, .. } = client.albums_if_changed(None).await.unwrap() else {
        panic!("first fetch must return the albums");
    };
    assert!(etag.is_some());
    assert_eq!(client.albums_if_changed(etag.as_deref()).await.unwrap(), Fetched::NotModified);

    let Fetched::Changed { etag, value } = client.time_buckets_if_changed(&TimelineScope::Library, None).await.unwrap()
    else {
        panic!("first fetch must return the buckets");
    };
    assert_eq!(
        client
            .time_buckets_if_changed(&TimelineScope::Library, etag.as_deref())
            .await
            .unwrap(),
        Fetched::NotModified
    );

    let month = value[0].month;
    let Fetched::Changed { etag, .. } = client
        .time_bucket_if_changed(&TimelineScope::Library, month, None)
        .await
        .unwrap()
    else {
        panic!("first fetch must return the bucket");
    };
    assert_eq!(
        client
            .time_bucket_if_changed(&TimelineScope::Library, month, etag.as_deref())
            .await
            .unwrap(),
        Fetched::NotModified
    );
}

#[tokio::test]
#[ignore = "needs the lab server; see the header of tests/contract.rs"]
async fn sign_in_failures() {
    let anonymous = client_with(None);
    assert_eq!(
        anonymous
            .login_password(&env("IMMICH_ADMIN_EMAIL"), "certainly-wrong")
            .await
            .unwrap_err(),
        ApiError::Unauthorized
    );
    let error = anonymous
        .oauth_authorize("app.immich:///oauth-callback", "state", "challenge")
        .await
        .unwrap_err();
    assert!(
        matches!(&error, ApiError::Protocol(message) if message.contains("OAuth is not enabled")),
        "{error:?}"
    );
}

#[tokio::test]
#[ignore = "needs the lab server; see the header of tests/contract.rs"]
async fn missing_permissions_are_named() {
    let client = readonly();
    let error = client.bulk_upload_check(&["0".repeat(40)]).await.unwrap_err();
    assert_eq!(
        error,
        ApiError::Forbidden {
            missing_permission: Some("asset.upload".to_owned())
        }
    );
    assert!(matches!(
        client.sync_stream(&["AssetsV2"], false).await.unwrap_err(),
        ApiError::Forbidden { .. }
    ));
    assert!(matches!(
        full().sync_stream(&["AssetsV2"], false).await.unwrap_err(),
        ApiError::Forbidden { .. }
    ));
}

/// Read a sync stream to its end: the records, and the last ack per record type.
async fn drain_sync(client: &Client, reset: bool) -> (Vec<SyncRecord>, Vec<String>) {
    let mut stream = client.sync_stream(&["AssetsV2", "AssetExifsV1"], reset).await.unwrap();
    let mut records = Vec::new();
    let mut last_acks: HashMap<String, String> = HashMap::new();
    while let Some(line) = stream.next_line().await.unwrap() {
        let kind = line.ack.split('|').next().unwrap_or_default().to_owned();
        last_acks.insert(kind, line.ack);
        records.push(line.record);
    }
    (records, last_acks.into_values().collect())
}

/// A JPEG from the lab's fixture cache with a unique trailer, so its checksum is new to the server.
fn unique_jpeg(nonce: i64) -> PathBuf {
    let cache = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../lab/immich-server/fixtures/cache");
    let source = std::fs::read_dir(&cache)
        .expect("the lab fixture cache exists after seed.sh")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.extension().is_some_and(|extension| extension == "jpg"))
        .expect("a JPEG in the fixture cache");
    let mut content = std::fs::read(source).unwrap();
    content.extend_from_slice(format!("contract-test-{nonce}").as_bytes());
    let path = std::env::temp_dir().join(format!("immich-api-contract-{nonce}.jpg"));
    std::fs::write(&path, content).unwrap();
    path
}

#[tokio::test]
#[ignore = "needs the lab server; see the header of tests/contract.rs"]
async fn upload_duplicate_trash_delete_and_sync() {
    let uploader = full();
    let (session, login) = session().await;
    assert!(!login.user_id.is_empty());

    // A first sync carries the whole library, trashed and archived Assets included.
    let (records, acks) = drain_sync(&session, true).await;
    let assets_seen = records.iter().filter(|record| matches!(record, SyncRecord::AssetV2(_))).count();
    let exif_with_size = records
        .iter()
        .filter(|record| matches!(record, SyncRecord::AssetExifV1(exif) if exif.file_size.is_some()))
        .count();
    assert!(assets_seen >= 52, "saw {assets_seen} AssetV2 records");
    assert!(exif_with_size >= 52);
    assert_eq!(records.last(), Some(&SyncRecord::SyncCompleteV1));
    session.sync_ack(&acks).await.unwrap();
    let (records, _) = drain_sync(&session, false).await;
    assert_eq!(records, [SyncRecord::SyncCompleteV1], "nothing new after the ack");

    // Upload a file new to the server, under a Unicode name.
    let nonce = now().timestamp_millis();
    let path = unique_jpeg(nonce);
    let checksum = immich_api::Sha1Checksum::of_file(&path).await.unwrap();
    assert_eq!(
        uploader.bulk_upload_check(&[checksum.to_base64()]).await.unwrap(),
        [UploadCheck::Accept]
    );
    let name = format!("タテ弁 contract {nonce}.jpg");
    let taken = now();
    let outcome = uploader.upload_asset(&path, &name, taken, taken).await.unwrap();
    let UploadOutcome::Created(id) = outcome else {
        panic!("expected a new Asset, got {outcome:?}");
    };
    let uploaded = uploader.asset(&id).await.unwrap();
    assert_eq!(uploaded.original_file_name, name, "the UTF-8 name round-trips");
    assert_eq!(uploaded.checksum, checksum.to_base64());

    // The same bytes again are a duplicate, under any name.
    assert_eq!(
        uploader.upload_asset(&path, "other-name.jpg", taken, taken).await.unwrap(),
        UploadOutcome::Duplicate { id: id.clone(), trashed: false }
    );
    assert_eq!(
        uploader.bulk_upload_check(&[checksum.to_hex()]).await.unwrap(),
        [UploadCheck::Duplicate { asset_id: id.clone(), trashed: false }],
        "hex checksums are accepted too"
    );

    // The sync stream reports the new Asset.
    let (records, acks) = drain_sync(&session, false).await;
    assert!(records
        .iter()
        .any(|record| matches!(record, SyncRecord::AssetV2(asset) if asset.id == id)));
    session.sync_ack(&acks).await.unwrap();

    // Trashed, the duplicate says so, and the Asset stays trashed.
    let trashed = admin_call(reqwest::Method::DELETE, "/assets", serde_json::json!({ "ids": [id] })).await;
    assert!(trashed.is_success());
    assert_eq!(
        uploader.upload_asset(&path, &name, taken, taken).await.unwrap(),
        UploadOutcome::Duplicate { id: id.clone(), trashed: true }
    );
    assert!(uploader.asset(&id).await.unwrap().is_trashed);

    // Deleted for good, it is gone everywhere, and the sync stream says so.
    let deleted = admin_call(
        reqwest::Method::DELETE,
        "/assets",
        serde_json::json!({ "ids": [id], "force": true }),
    )
    .await;
    assert!(deleted.is_success());
    std::fs::remove_file(&path).unwrap();
    // A forced delete is a queued job: wait (bounded) until the Asset is gone.
    let mut lookup = uploader.asset(&id).await;
    for _ in 0..60 {
        if lookup.is_err() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        lookup = uploader.asset(&id).await;
    }
    assert_eq!(lookup.unwrap_err(), ApiError::NotFound);
    assert_eq!(uploader.original_range(&id, 0, 10).await.unwrap_err(), ApiError::NotFound);
    let (records, _) = drain_sync(&session, false).await;
    assert!(
        records
            .iter()
            .any(|record| matches!(record, SyncRecord::AssetDeleteV1 { asset_id } if *asset_id == id)),
        "{records:?}"
    );

    // Signing out ends the session.
    session.logout().await.unwrap();
    assert_eq!(session.albums().await.unwrap_err(), ApiError::Unauthorized);
}
