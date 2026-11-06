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
//! Environment: `IMMICH_URL`, `IMMICH_API_KEY_READONLY`, `IMMICH_ADMIN_EMAIL`, `IMMICH_ADMIN_PASSWORD`. The numbers are
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
    assert_eq!(library.list("root".into()).await.unwrap().len(), 4, "a session browses as a key does");
    assert!(!library.poll().await.unwrap(), "a session's first poll, through the sync stream, takes note only");
    drop(library);
    end_session(env("IMMICH_URL"), session.credential.clone()).await;
    let ended = Library::open(env("IMMICH_URL"), session.credential, directory.path().join("again").display().to_string(), String::new()).unwrap();
    assert!(ended.list("root".into()).await.is_ok(), "the root needs no server");
    assert!(ended.list("section|Photos".into()).await.is_err(), "a session ended on the server is refused");
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

    let sections: Vec<String> = library.list("root".into()).await.unwrap().into_iter().map(|item| item.name).collect();
    assert_eq!(sections, ["Photos", "Albums", "People", "Places"], "no Upload folder before the macOS edition uploads");

    // Walk Photos to its files, the way Finder would, folder by folder.
    let mut folders = vec!["section|Photos".to_owned()];
    let mut files: Vec<Item> = Vec::new();
    while let Some(folder) = folders.pop() {
        for item in library.list(folder.clone()).await.unwrap() {
            assert_eq!(item.parent_identifier, folder);
            assert_eq!(library.item(item.identifier.clone()).await.unwrap(), item, "asked for, an item is as listed");
            if item.is_folder { folders.push(item.identifier) } else { files.push(item) }
        }
    }
    assert_eq!(files.len(), 50, "every Asset on the timeline");

    for file in &files {
        let destination = directory.path().join("original");
        let fetched = library.fetch(file.identifier.clone(), destination.display().to_string()).await.unwrap();
        let bytes = std::fs::read(&destination).unwrap();
        assert_eq!(fetched.size, Some(bytes.len() as u64), "{}", file.name);
        let expected = client.asset(&file.content_version).await.unwrap().checksum;
        let actual = base64::engine::general_purpose::STANDARD.encode(Sha1::digest(&bytes));
        assert_eq!(actual, expected, "{} is byte for byte the server's original", file.name);
    }

    let thumbnail = library.thumbnail(files[0].identifier.clone(), 256).await.unwrap().expect("a photo has a thumbnail");
    assert!(thumbnail.starts_with(b"RIFF"), "the small thumbnail is Immich's WebP");
    let preview = library.thumbnail(files[0].identifier.clone(), 1024).await.unwrap().expect("and a preview");
    assert!(preview.starts_with(&[0xff, 0xd8]), "the preview is Immich's JPEG");
}
