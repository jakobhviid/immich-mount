//! The bridge against a real Immich server, seeded by `lab/immich-server/seed.sh`: what the extension asks of it, on
//! Linux, before a Mac is at hand.
//!
//! All `#[ignore]`d. Run them against a lab server with the values from its `.env.local`:
//!
//! ```sh
//! set -a; . lab/immich-server/.env.local; set +a
//! cargo test -p immich-mount-apple --test lab -- --ignored
//! ```
//!
//! Environment: `IMMICH_URL`, `IMMICH_API_KEY_READONLY`, `IMMICH_API_KEY_FULL`, `IMMICH_ADMIN_EMAIL`,
//! `IMMICH_ADMIN_PASSWORD`; the upload test also reads the fixtures `seed.sh` downloaded. The numbers are
//! the seeded library's (the `lab-immich-server` skill, "What is in it"): 50 Assets on the timeline, no month big
//! enough for day folders.

use base64::Engine;
use immich_api::{Client, Credential as ApiCredential};
use immich_mount_apple::{end_session, probe, sign_in_with_api_key, sign_in_with_password, Credential, Item, Library};
use sha1::{Digest, Sha1};

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is not set; source lab/immich-server/.env.local"))
}

#[tokio::test]
#[ignore = "needs the lab server"]
async fn signing_in_takes_a_key_or_a_password_and_refuses_a_wrong_key() {
    let offer = probe(env("IMMICH_URL")).await.unwrap();
    assert!(offer.version.starts_with('v') && offer.password, "{offer:?}");
    let key = sign_in_with_api_key(env("IMMICH_URL"), env("IMMICH_API_KEY_READONLY")).await.unwrap();
    assert_eq!(key.credential, Credential::ApiKey { key: env("IMMICH_API_KEY_READONLY") });
    let refused = sign_in_with_api_key(env("IMMICH_URL"), "not-a-key".into()).await.unwrap_err();
    assert_eq!(refused.to_string(), "The server did not accept this API key.");

    let session = sign_in_with_password(env("IMMICH_URL"), env("IMMICH_ADMIN_EMAIL"), env("IMMICH_ADMIN_PASSWORD")).await.unwrap();
    assert_eq!(session.account, Some(env("IMMICH_ADMIN_EMAIL")));
    let directory = tempfile::tempdir().unwrap();
    let library = Library::open(env("IMMICH_URL"), session.credential.clone(), directory.path().display().to_string(), String::new()).unwrap();
    assert_eq!(library.list("root".into(), None).await.unwrap().len(), 5, "a session browses as a key does");
    assert!(!library.poll(None, None).await.unwrap(), "a session's first poll, through the sync stream, takes note only");
    drop(library);
    end_session(env("IMMICH_URL"), session.credential.clone()).await;
    let ended = Library::open(env("IMMICH_URL"), session.credential, directory.path().join("again").display().to_string(), String::new()).unwrap();
    assert!(ended.list("root".into(), None).await.is_ok(), "the root needs no server");
    assert!(ended.list("section|Photos".into(), None).await.is_err(), "a session ended on the server is refused");
}

#[tokio::test]
#[ignore = "needs the lab server"]
async fn a_photo_added_to_upload_reaches_the_library_byte_for_byte() {
    let directory = tempfile::tempdir().unwrap();
    let key = env("IMMICH_API_KEY_FULL");
    let library = Library::open(env("IMMICH_URL"), Credential::ApiKey { key: key.clone() }, directory.path().join("library").display().to_string(), String::new()).unwrap();
    // A fixture with bytes appended after the image: whole, and unique, so Immich takes it as new.
    let fixture = concat!(env!("CARGO_MANIFEST_DIR"), "/../../lab/immich-server/fixtures/cache/001.jpg");
    let mut bytes = std::fs::read(fixture).expect("the fixtures are downloaded (lab/immich-server/seed.sh)");
    bytes.extend(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos().to_le_bytes());
    let photo = directory.path().join("MAC_UPLOAD.jpg");
    std::fs::write(&photo, &bytes).unwrap();

    let item = library.upload("upload".into(), "MAC_UPLOAD.jpg".into(), photo.display().to_string(), std::time::SystemTime::now(), None).await.unwrap();
    assert_eq!(item.parent_identifier, "upload");
    let asset_id = item.content_version.clone();
    let client = Client::new(&env("IMMICH_URL"), Some(ApiCredential::ApiKey(key.clone())), "ImmichMount/lab-tests (Linux; lab)").unwrap();
    let expected = base64::engine::general_purpose::STANDARD.encode(Sha1::digest(&bytes));
    assert_eq!(client.asset(&asset_id).await.unwrap().checksum, expected, "the original, byte for byte");
    let again = library.upload("upload".into(), "MAC_UPLOAD.jpg".into(), photo.display().to_string(), std::time::SystemTime::now(), None).await.unwrap();
    assert_eq!(again.content_version, asset_id, "the same bytes again are the same Asset");

    let deleted = reqwest::Client::new()
        .delete(format!("{}/api/assets", env("IMMICH_URL")))
        .header("x-api-key", key)
        .json(&serde_json::json!({ "ids": [asset_id], "force": true }))
        .send()
        .await
        .unwrap();
    assert!(deleted.status().is_success(), "the test's Asset is deleted again");
}

/// Every file under `folder`, walked the way Finder would, folder by folder.
async fn files_under(library: &Library, folder: &str) -> Vec<Item> {
    let mut folders = vec![folder.to_owned()];
    let mut files = Vec::new();
    while let Some(folder) = folders.pop() {
        for item in library.list(folder, None).await.unwrap() {
            if item.is_folder { folders.push(item.identifier) } else { files.push(item) }
        }
    }
    files
}

/// Change Assets on the server, as the Immich app would, with the full-access key.
async fn update_assets(ids: &[&str], change: serde_json::Value) {
    let mut body = change;
    body["ids"] = serde_json::json!(ids);
    let updated = reqwest::Client::new()
        .put(format!("{}/api/assets", env("IMMICH_URL")))
        .header("x-api-key", env("IMMICH_API_KEY_FULL"))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert!(updated.status().is_success(), "{}", updated.status());
}

#[tokio::test]
#[ignore = "needs the lab server"]
async fn the_archive_holds_what_immich_archived_and_follows_it() {
    let directory = tempfile::tempdir().unwrap();
    let key = env("IMMICH_API_KEY_READONLY");
    let config = "[sections]\narchive = true\n".to_owned();
    let library = Library::open(env("IMMICH_URL"), Credential::ApiKey { key: key.clone() }, directory.path().display().to_string(), config).unwrap();
    let client = Client::new(&env("IMMICH_URL"), Some(ApiCredential::ApiKey(key)), "ImmichMount/lab-tests (Linux; lab)").unwrap();

    let sections: Vec<String> = library.list("root".into(), None).await.unwrap().into_iter().map(|item| item.name).collect();
    assert_eq!(sections, ["Photos", "Albums", "People", "Places", "Archive", "Upload"]);
    let archived = files_under(&library, "section|Archive").await;
    assert_eq!(archived.len(), 1, "the seed's one archived photo");
    assert!(archived[0].parent_identifier.starts_with("group-month|section%7CArchive|"), "in a year and a month folder");
    assert!(!library.poll(None, None).await.unwrap(), "the first poll takes note only");

    // A photo archived in Immich arrives at the next poll, and leaves again when it is unarchived.
    let timeline = files_under(&library, "section|Photos").await;
    let photo = timeline[0].content_version.clone();
    update_assets(&[&photo], serde_json::json!({ "visibility": "archive" })).await;
    assert!(library.poll(None, None).await.unwrap(), "the poll sees the archive change");
    let archived: Vec<String> = files_under(&library, "section|Archive").await.into_iter().map(|item| item.content_version).collect();
    assert_eq!(archived.len(), 2);
    assert!(archived.contains(&photo));
    assert_eq!(client.asset(&photo).await.unwrap().visibility, immich_api::Visibility::Archive);
    update_assets(&[&photo], serde_json::json!({ "visibility": "timeline" })).await;
    assert!(library.poll(None, None).await.unwrap(), "and sees it unarchived");
    assert_eq!(files_under(&library, "section|Archive").await.len(), 1);
}

#[tokio::test]
#[ignore = "needs the lab server"]
async fn favorites_hold_what_immich_marked_archived_ones_too() {
    let directory = tempfile::tempdir().unwrap();
    let key = env("IMMICH_API_KEY_READONLY");
    let config = "[sections]\nfavorites = true\n".to_owned();
    let library = Library::open(env("IMMICH_URL"), Credential::ApiKey { key: key.clone() }, directory.path().display().to_string(), config).unwrap();
    let client = Client::new(&env("IMMICH_URL"), Some(ApiCredential::ApiKey(key)), "ImmichMount/lab-tests (Linux; lab)").unwrap();

    let sections: Vec<String> = library.list("root".into(), None).await.unwrap().into_iter().map(|item| item.name).collect();
    assert_eq!(sections, ["Photos", "Albums", "People", "Places", "Favorites", "Upload"]);
    assert!(files_under(&library, "section|Favorites").await.is_empty(), "the seed marks no favorites");
    assert!(!library.poll(None, None).await.unwrap(), "the first poll takes note only");

    // One photo from the timeline and the archived one, marked in Immich: both arrive at the next poll, as both show
    // on Immich's Favorites page.
    let photo = files_under(&library, "section|Photos").await[0].content_version.clone();
    let archived = client
        .search_all_assets(&immich_api::AssetQuery { visibility: Some(immich_api::Visibility::Archive), ..Default::default() }, 10)
        .await
        .unwrap()[0]
        .id
        .clone();
    update_assets(&[&photo, &archived], serde_json::json!({ "isFavorite": true })).await;
    assert!(library.poll(None, None).await.unwrap(), "the poll sees the favorites change");
    let mut favorites: Vec<String> = files_under(&library, "section|Favorites").await.into_iter().map(|item| item.content_version).collect();
    favorites.sort();
    let mut expected = vec![photo.clone(), archived.clone()];
    expected.sort();
    assert_eq!(favorites, expected);
    update_assets(&[&photo, &archived], serde_json::json!({ "isFavorite": false })).await;
    assert!(library.poll(None, None).await.unwrap(), "and sees them unmarked");
    assert!(files_under(&library, "section|Favorites").await.is_empty());
}

#[tokio::test]
#[ignore = "needs the lab server"]
async fn every_timeline_original_matches_the_servers_checksum() {
    let directory = tempfile::tempdir().unwrap();
    let library = Library::open(
        env("IMMICH_URL"),
        Credential::ApiKey { key: env("IMMICH_API_KEY_READONLY") },
        directory.path().join("library").display().to_string(),
        String::new(),
    )
    .unwrap();
    let client = Client::new(&env("IMMICH_URL"), Some(ApiCredential::ApiKey(env("IMMICH_API_KEY_READONLY"))), "ImmichMount/lab-tests (Linux; lab)").unwrap();

    let sections: Vec<String> = library.list("root".into(), None).await.unwrap().into_iter().map(|item| item.name).collect();
    assert_eq!(sections, ["Photos", "Albums", "People", "Places", "Upload"]);

    // Walk Photos to its files, the way Finder would, folder by folder.
    let mut folders = vec!["section|Photos".to_owned()];
    let mut files: Vec<Item> = Vec::new();
    while let Some(folder) = folders.pop() {
        for item in library.list(folder.clone(), None).await.unwrap() {
            assert_eq!(item.parent_identifier, folder);
            assert_eq!(library.item(item.identifier.clone(), None).await.unwrap(), item, "asked for, an item is as listed");
            if item.is_folder { folders.push(item.identifier) } else { files.push(item) }
        }
    }
    assert_eq!(files.len(), 50, "every Asset on the timeline");

    for file in &files {
        let destination = directory.path().join("original");
        let fetched = library.fetch(file.identifier.clone(), destination.display().to_string(), None).await.unwrap();
        let bytes = std::fs::read(&destination).unwrap();
        assert_eq!(fetched.size, Some(bytes.len() as u64), "{}", file.name);
        let expected = client.asset(&file.content_version).await.unwrap().checksum;
        let actual = base64::engine::general_purpose::STANDARD.encode(Sha1::digest(&bytes));
        assert_eq!(actual, expected, "{} is byte for byte the server's original", file.name);
    }

    let thumbnail = library.thumbnail(files[0].identifier.clone(), 256, None).await.unwrap().expect("a photo has a thumbnail");
    assert!(thumbnail.starts_with(b"RIFF"), "the small thumbnail is Immich's WebP");
    let preview = library.thumbnail(files[0].identifier.clone(), 1024, None).await.unwrap().expect("and a preview");
    assert!(preview.starts_with(&[0xff, 0xd8]), "the preview is Immich's JPEG");
}
