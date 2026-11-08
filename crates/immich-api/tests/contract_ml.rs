//! Contract tests that need machine learning: smart search, detected faces, OCR text, duplicate groups and the
//! explore page with real results. They run against a server built with `up.sh --ml` (its env file sets
//! `IMMICH_ML=1`) and pass without checking anything elsewhere. See `tests/common/mod.rs` to run them.

mod common;

use common::{asset_named, full, readonly};
use immich_api::{AssetQuery, SmartQuery};

/// Whether the server has machine learning; the tests print why they check nothing when it has not.
fn machine_learning() -> bool {
    let on = std::env::var("IMMICH_ML").is_ok_and(|value| value == "1");
    if !on {
        println!("IMMICH_ML is not 1: this server has no machine learning, nothing to check");
    }
    on
}

#[tokio::test]
#[ignore = "needs the lab server; see tests/common/mod.rs"]
async fn smart_search_finds_by_meaning_and_by_likeness() {
    if !machine_learning() {
        return;
    }
    let client = readonly();
    let churches = client
        .smart_search(&SmartQuery { text: Some("a church".to_owned()), ..SmartQuery::default() }, 5)
        .await
        .unwrap();
    assert_eq!(churches.len(), 5);
    let names: Vec<&str> = churches.iter().map(|asset| asset.original_file_name.as_str()).collect();
    assert!(
        names.iter().any(|name| name.contains("Dom") || name.contains("Pietro") || name.contains("Petersdom")),
        "{names:?}"
    );

    let first = churches[0].id.clone();
    let alike = client
        .smart_search(&SmartQuery { similar_to: Some(first.clone()), ..SmartQuery::default() }, 3)
        .await
        .unwrap();
    assert_eq!(alike[0].id, first, "an Asset is most like itself");

    let in_denmark = client
        .smart_search(
            &SmartQuery {
                text: Some("a church".to_owned()),
                filter: AssetQuery { country: Some("Denmark".to_owned()), ..AssetQuery::default() },
                ..SmartQuery::default()
            },
            10,
        )
        .await
        .unwrap();
    assert!(!in_denmark.is_empty());
    assert!(in_denmark
        .iter()
        .all(|asset| asset.exif.as_ref().and_then(|exif| exif.country.as_deref()) == Some("Denmark")));
}

#[tokio::test]
#[ignore = "needs the lab server; see tests/common/mod.rs"]
async fn text_in_pictures_is_read() {
    if !machine_learning() {
        return;
    }
    let client = full();
    let id = asset_named(&client, "Tjarnarbíó 2025.jpg").await;
    let texts = client.asset_ocr(&id).await.unwrap();
    assert!(texts.iter().any(|text| text.text.contains("TJARNARBIO")), "{texts:?}");
    assert!(texts.iter().all(|text| text.asset_id == id && (0.0..=1.0).contains(&text.text_score)));
}

#[tokio::test]
#[ignore = "needs the lab server; see tests/common/mod.rs"]
async fn faces_are_detected_and_duplicates_grouped() {
    if !machine_learning() {
        return;
    }
    let client = full();
    let library = client.search_all_assets(&AssetQuery::default(), 1000).await.unwrap();
    let mut detected = 0;
    for asset in &library {
        let faces = client.faces(&asset.id).await.unwrap();
        for face in &faces {
            assert!(face.left < face.right && face.top < face.bottom, "{face:?}");
            assert!(face.right <= face.image_width as i32 && face.bottom <= face.image_height as i32);
        }
        detected += faces.len();
    }
    assert!(detected >= 10, "faces detected in the seed: {detected}");

    let groups = client.duplicates().await.unwrap();
    assert!(!groups.is_empty(), "the seed has look-alike photos");
    for group in &groups {
        assert!(group.assets.len() >= 2, "{}", group.duplicate_id);
        assert!(group
            .suggested_keep_asset_ids
            .iter()
            .all(|keep| group.assets.iter().any(|asset| &asset.id == keep)));
    }

    let explore = client.explore().await.unwrap();
    assert!(explore.iter().any(|group| group.field_name == "exifInfo.city" && !group.items.is_empty()));
}
