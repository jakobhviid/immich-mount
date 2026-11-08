//! Contract tests for organising the library: albums, editing Assets, the trash, edits, key-value metadata, tags,
//! the map and video playback. Each test that changes something puts it back before it ends. See
//! `tests/common/mod.rs` to run them.

mod common;

use chrono::DateTime;
use common::{album_id, assert_refused, asset_named, full, readonly, timeline_ids};
use immich_api::{
    AlbumChanges, AlbumFilter, ApiError, AssetChanges, AssetEdit, AssetJob, AssetKind, AssetOrder, AssetQuery,
    CopyOptions, MapQuery, NewAlbum, NewTag, StatisticsFilter, TagChanges, Visibility,
};

#[tokio::test]
#[ignore = "needs the lab server; see tests/common/mod.rs"]
async fn albums_are_created_edited_filled_and_deleted() {
    let client = full();
    let ids = timeline_ids(&client).await;
    let before = client.album_statistics().await.unwrap();

    let album = client
        .create_album(&NewAlbum {
            name: "Contract album".to_owned(),
            description: Some("made by a test".to_owned()),
            asset_ids: ids[..2].to_vec(),
            ..NewAlbum::default()
        })
        .await
        .unwrap();
    assert_eq!(album.asset_count, 2);
    assert_eq!(album.description, "made by a test");
    assert_eq!(album.owner().map(|owner| owner.email.clone()), Some(common::env("IMMICH_ADMIN_EMAIL")));
    assert_eq!(client.album_statistics().await.unwrap().owned, before.owned + 1);

    let read = client.album(&album.id).await.unwrap();
    assert_eq!(read.name, "Contract album");
    assert!(read.thumbnail_asset_id.is_some());

    let changed = client
        .update_album(
            &album.id,
            &AlbumChanges {
                name: Some("Contract album, renamed".to_owned()),
                description: Some(String::new()),
                thumbnail_asset_id: Some(ids[1].clone()),
                order: Some(AssetOrder::Asc),
                activity_enabled: Some(false),
            },
        )
        .await
        .unwrap();
    assert_eq!(changed.name, "Contract album, renamed");
    assert_eq!(changed.description, "");
    assert_eq!(changed.thumbnail_asset_id.as_deref(), Some(ids[1].as_str()));
    assert_eq!(changed.order, Some(AssetOrder::Asc));
    assert!(!changed.activity_enabled);

    let added = client.add_assets_to_album(&album.id, &ids[1..3]).await.unwrap();
    assert_eq!(added.len(), 2);
    assert!(!added[0].success, "already in the album");
    assert_eq!(added[0].error, Some(immich_api::BulkIdError::Duplicate));
    assert!(added[1].success);
    assert_eq!(client.album(&album.id).await.unwrap().asset_count, 3);

    let containing = client
        .albums_where(&AlbumFilter { containing_asset: Some(ids[2].clone()), ..AlbumFilter::default() })
        .await
        .unwrap();
    assert!(containing.iter().any(|candidate| candidate.id == album.id));
    let not_shared = client
        .albums_where(&AlbumFilter { shared: Some(false), ..AlbumFilter::default() })
        .await
        .unwrap();
    assert!(not_shared.iter().any(|candidate| candidate.id == album.id));

    let removed = client.remove_assets_from_album(&album.id, std::slice::from_ref(&ids[0])).await.unwrap();
    assert!(removed[0].success);

    let second = client
        .create_album(&NewAlbum { name: "Contract album 2".to_owned(), ..NewAlbum::default() })
        .await
        .unwrap();
    assert_eq!(second.asset_count, 0);
    assert_eq!(second.start_date, None);
    let both = client
        .add_assets_to_albums(&[album.id.clone(), second.id.clone()], std::slice::from_ref(&ids[3]))
        .await
        .unwrap();
    assert!(both.success, "{both:?}");
    assert_eq!(client.album(&second.id).await.unwrap().asset_count, 1);

    let markers = client.album_map_markers(&album.id).await.unwrap();
    assert!(markers.len() <= 3);

    client.delete_album(&album.id).await.unwrap();
    client.delete_album(&second.id).await.unwrap();
    assert_eq!(client.album(&album.id).await.unwrap_err(), ApiError::NotFound);
    assert_eq!(client.album_statistics().await.unwrap(), before);
}

#[tokio::test]
#[ignore = "needs the lab server; see tests/common/mod.rs"]
async fn asset_details_agree_with_the_slim_asset() {
    let client = readonly();
    let holiday = album_id(&client, "Holiday Mix").await;
    let query = AssetQuery { album_ids: Some(vec![holiday]), ..AssetQuery::default() };
    for asset in client.search_all_assets(&query, 100).await.unwrap() {
        let details = client.asset_details(&asset.id).await.unwrap();
        assert_eq!(details.original_file_name, asset.original_file_name);
        assert_eq!(details.checksum, asset.checksum);
        assert_eq!(details.file_created_at, asset.file_created_at);
        assert_eq!(details.local_date_time, Some(asset.local_date_time));
        assert_eq!(details.exif.as_ref().and_then(|exif| exif.file_size), asset.file_size);
        assert_eq!(details.duration_ms, asset.duration_ms);
        assert_eq!(details.kind, asset.kind);
        assert!(details.width.is_some() && details.height.is_some(), "{}", asset.id);
        assert!(!details.owner_id.is_empty());
        assert!(details.thumbhash.is_some());
    }

    let person = client.people().await.unwrap().remove(0);
    let with_person = client
        .search_all_assets(&AssetQuery { person_ids: Some(vec![person.id.clone()]), ..AssetQuery::default() }, 10)
        .await
        .unwrap();
    let details = client.asset_details(&with_person[0].id).await.unwrap();
    assert!(details.people.iter().any(|candidate| candidate.id == person.id));

    assert_eq!(client.asset_ocr(&with_person[0].id).await.unwrap(), [], "machine learning is off in the lab");
}

#[tokio::test]
#[ignore = "needs the lab server; see tests/common/mod.rs"]
async fn an_asset_is_edited_one_at_a_time_and_in_bulk_then_put_back() {
    let client = full();
    let id = asset_named(&client, "Loew's State Theatre.jpg").await;
    let original = client.asset_details(&id).await.unwrap();
    let original_exif = original.exif.clone().unwrap_or_default();

    let changed = client
        .update_asset(
            &id,
            &AssetChanges {
                description: Some("contract description".to_owned()),
                rating: Some(Some(4)),
                favorite: Some(true),
                latitude: Some(55.6761),
                longitude: Some(12.5683),
                ..AssetChanges::default()
            },
        )
        .await
        .unwrap();
    assert!(changed.is_favorite);
    let exif = changed.exif.unwrap();
    assert_eq!(exif.description.as_deref(), Some("contract description"));
    assert_eq!(exif.rating, Some(4));
    assert!((exif.latitude.unwrap() - 55.6761).abs() < 1e-4);

    let taken = DateTime::parse_from_rfc3339("2001-02-03T04:05:06+02:00").unwrap();
    client
        .update_assets(
            std::slice::from_ref(&id),
            &AssetChanges {
                date_time_original: Some(taken),
                rating: Some(None),
                favorite: Some(false),
                description: Some(String::new()),
                ..AssetChanges::default()
            },
        )
        .await
        .unwrap();
    let read = client.asset_details(&id).await.unwrap();
    assert!(!read.is_favorite);
    assert_eq!(read.exif.as_ref().and_then(|exif| exif.rating), None, "null clears the rating");
    assert_eq!(read.exif.as_ref().and_then(|exif| exif.date_time_original), Some(taken.to_utc()));

    client
        .update_assets(std::slice::from_ref(&id), &AssetChanges { date_time_relative: Some(60), ..AssetChanges::default() })
        .await
        .unwrap();
    let shifted = client.asset_details(&id).await.unwrap();
    assert_eq!(
        shifted.exif.as_ref().and_then(|exif| exif.date_time_original),
        Some(taken.to_utc() + chrono::Duration::minutes(60))
    );

    // Back as it was. A date and place from the file are put back as they were read.
    let restore = AssetChanges {
        date_time_original: original_exif.date_time_original.map(|instant| instant.fixed_offset()),
        latitude: original_exif.latitude,
        longitude: original_exif.longitude,
        description: Some(original_exif.description.clone().unwrap_or_default()),
        rating: Some(original_exif.rating),
        favorite: Some(original.is_favorite),
        ..AssetChanges::default()
    };
    client.update_assets(std::slice::from_ref(&id), &restore).await.unwrap();
    let back = client.asset_details(&id).await.unwrap();
    assert_eq!(back.is_favorite, original.is_favorite);
    assert_eq!(back.exif.as_ref().and_then(|exif| exif.date_time_original), original_exif.date_time_original);
    assert_eq!(back.exif.as_ref().and_then(|exif| exif.rating), original_exif.rating);
}

#[tokio::test]
#[ignore = "needs the lab server; see tests/common/mod.rs"]
async fn archive_trash_and_restore_move_counts() {
    let client = full();
    let id = asset_named(&client, "Loew's State Theatre.jpg").await;
    let archived = |visibility| StatisticsFilter { visibility: Some(visibility), ..StatisticsFilter::default() };
    let trashed = StatisticsFilter { trashed: Some(true), ..StatisticsFilter::default() };
    let archive_before = client.asset_statistics(&archived(Visibility::Archive)).await.unwrap();
    let trash_before = client.asset_statistics(&trashed).await.unwrap();
    assert_eq!(trash_before.total, 1, "the seeded trashed Asset");

    client
        .update_assets(std::slice::from_ref(&id), &AssetChanges { visibility: Some(Visibility::Archive), ..AssetChanges::default() })
        .await
        .unwrap();
    assert_eq!(client.asset_statistics(&archived(Visibility::Archive)).await.unwrap().total, archive_before.total + 1);
    client
        .update_assets(std::slice::from_ref(&id), &AssetChanges { visibility: Some(Visibility::Timeline), ..AssetChanges::default() })
        .await
        .unwrap();
    assert_eq!(client.asset_statistics(&archived(Visibility::Archive)).await.unwrap(), archive_before);

    client.delete_assets(std::slice::from_ref(&id), false).await.unwrap();
    assert_eq!(client.asset_statistics(&trashed).await.unwrap().total, 2);
    assert!(client.asset_details(&id).await.unwrap().is_trashed);
    assert_eq!(client.restore_assets(std::slice::from_ref(&id)).await.unwrap(), 1);
    assert!(!client.asset_details(&id).await.unwrap().is_trashed);

    // Restoring the whole trash brings the seeded Asset back too; trash it again.
    let seeded = client
        .search_all_assets(&AssetQuery::default(), 1000)
        .await
        .unwrap()
        .len();
    assert_eq!(client.restore_trash().await.unwrap(), 1);
    let restored = wait_until(|| async { client.search_all_assets(&AssetQuery::default(), 1000).await.unwrap().len() == seeded + 1 }).await;
    assert!(restored, "the restore job finished");
    let back_id = client
        .search_all_assets(&AssetQuery::default(), 1000)
        .await
        .unwrap()
        .into_iter()
        .find(|asset| asset.original_file_name.starts_with("Besættelsesmuseet"))
        .unwrap()
        .id;
    client.delete_assets(&[back_id], false).await.unwrap();
    assert_eq!(client.asset_statistics(&trashed).await.unwrap(), trash_before);
}

/// Poll a condition for up to 20 s; server jobs finish shortly after the call that queued them.
async fn wait_until<Check, Pending>(mut check: Check) -> bool
where
    Check: FnMut() -> Pending,
    Pending: std::future::Future<Output = bool>,
{
    for _ in 0..40 {
        if check().await {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    false
}

#[tokio::test]
#[ignore = "needs the lab server; see tests/common/mod.rs"]
async fn edits_metadata_copies_and_jobs() {
    let client = full();
    let id = asset_named(&client, "Loew's State Theatre.jpg").await;
    let other = asset_named(&client, "no-date.jpg").await;

    let stored = client
        .set_asset_edits(&id, &[AssetEdit::Crop { x: 0, y: 0, width: 100, height: 80 }, AssetEdit::Rotate { angle: 90 }])
        .await
        .unwrap();
    assert_eq!(stored.len(), 2);
    assert_eq!(client.asset_edits(&id).await.unwrap(), stored);
    assert!(client.asset_details(&id).await.unwrap().is_edited);
    client.delete_asset_edits(&id).await.unwrap();
    assert_eq!(client.asset_edits(&id).await.unwrap(), []);

    let value = serde_json::json!({ "note": "contract" });
    let entries = client.set_asset_metadata(&id, &[("mobile-app".to_owned(), value.clone())]).await;
    match entries {
        Ok(entries) => {
            assert_eq!(entries[0].value, value);
            assert_eq!(client.asset_metadata_entry(&id, "mobile-app").await.unwrap().value, value);
            assert!(client.asset_metadata(&id).await.unwrap().iter().any(|entry| entry.key == "mobile-app"));
            client.delete_asset_metadata(&id, "mobile-app").await.unwrap();
            let bulk = client
                .set_assets_metadata(&[(id.clone(), "mobile-app".to_owned(), value.clone())])
                .await
                .unwrap();
            assert_eq!(bulk[0].asset_id.as_deref(), Some(id.as_str()));
            client.delete_assets_metadata(&[(id.clone(), "mobile-app".to_owned())]).await.unwrap();
            assert!(!client.asset_metadata(&id).await.unwrap().iter().any(|entry| entry.key == "mobile-app"));
        }
        Err(error) => panic!("setting metadata: {error:?}"),
    }

    client.update_asset(&id, &AssetChanges { favorite: Some(true), ..AssetChanges::default() }).await.unwrap();
    client
        .copy_asset_properties(&id, &other, &CopyOptions { favorite: true, ..CopyOptions::default() })
        .await
        .unwrap();
    assert!(client.asset_details(&other).await.unwrap().is_favorite);
    client
        .update_assets(&[id.clone(), other.clone()], &AssetChanges { favorite: Some(false), ..AssetChanges::default() })
        .await
        .unwrap();

    client.run_asset_job(std::slice::from_ref(&id), &AssetJob::RefreshMetadata).await.unwrap();
}

#[tokio::test]
#[ignore = "needs the lab server; see tests/common/mod.rs"]
async fn tags_nest_and_go_on_assets() {
    let client = full();
    let id = asset_named(&client, "no-date.jpg").await;
    let tags = client.upsert_tags(&["Contract/Nested".to_owned()]).await.unwrap();
    let nested = tags.iter().find(|tag| tag.value == "Contract/Nested").expect("the nested tag").clone();
    let parent = client.tags().await.unwrap().into_iter().find(|tag| tag.value == "Contract").expect("its parent");
    assert_eq!(nested.parent_id.as_deref(), Some(parent.id.as_str()));
    assert_eq!(nested.name, "Nested");

    let sibling = client
        .create_tag(&NewTag { name: "Sibling".to_owned(), parent_id: Some(parent.id.clone()), color: None })
        .await
        .unwrap();
    assert_eq!(sibling.value, "Contract/Sibling");
    let recoloured = client
        .update_tag(&sibling.id, &TagChanges { color: Some(Some("#ff0000".to_owned())), ..TagChanges::default() })
        .await
        .unwrap();
    assert_eq!(recoloured.color.as_deref(), Some("#ff0000"));
    assert_eq!(client.tag(&sibling.id).await.unwrap().color.as_deref(), Some("#ff0000"));

    let tagged = client.tag_assets(&nested.id, std::slice::from_ref(&id)).await.unwrap();
    assert!(tagged[0].success, "{tagged:?}");
    assert!(client.asset_details(&id).await.unwrap().tags.iter().any(|tag| tag.id == nested.id));
    assert_eq!(client.tag_assets_with(std::slice::from_ref(&sibling.id), std::slice::from_ref(&id)).await.unwrap(), 1);
    // Tagging queues a sidecar job that undoes changes made before it runs (tag.rs).
    common::wait_for_idle_queues(&client).await;
    let untagged = client.untag_assets(&nested.id, std::slice::from_ref(&id)).await.unwrap();
    assert!(untagged[0].success, "{untagged:?}");

    client.untag_assets(&sibling.id, std::slice::from_ref(&id)).await.unwrap();
    let untagged_everywhere = wait_until(|| async { client.asset_details(&id).await.unwrap().tags.is_empty() }).await;
    assert!(untagged_everywhere, "the tags came off the Asset");

    client.delete_tag(&parent.id).await.unwrap();
    let contract_tags = || async {
        let tags = client.tags().await.unwrap();
        tags.into_iter().filter(|tag| tag.value.starts_with("Contract")).collect::<Vec<_>>()
    };
    assert!(contract_tags().await.is_empty(), "deleting a tag deletes those below it");
    // The sidecar job of the tagging above can bring deleted tags back a moment later (tag.rs): delete them until
    // none come back.
    for _ in 0..20 {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let returned = contract_tags().await;
        if returned.is_empty() {
            continue;
        }
        println!("tags came back after the delete: {:?}", returned.iter().map(|tag| &tag.value).collect::<Vec<_>>());
        for tag in returned.iter().filter(|tag| tag.parent_id.is_none()) {
            client.delete_tag(&tag.id).await.unwrap();
        }
    }
    assert!(contract_tags().await.is_empty());
}

#[tokio::test]
#[ignore = "needs the lab server; see tests/common/mod.rs"]
async fn the_map_and_reverse_geocoding() {
    let client = full();
    let markers = client.map_markers(&MapQuery::default()).await.unwrap();
    let library: std::collections::HashSet<String> = client
        .search_all_assets(&AssetQuery::default(), 1000)
        .await
        .unwrap()
        .into_iter()
        .map(|asset| asset.id)
        .collect();
    assert!(!markers.is_empty());
    assert!(markers.iter().all(|marker| library.contains(&marker.id)));
    assert!(markers.len() <= library.len() - 10, "ten seeded photos have no GPS");

    let places = client.reverse_geocode(55.6761, 12.5683).await.unwrap();
    assert_eq!(places[0].country.as_deref(), Some("Denmark"));
}

#[tokio::test]
#[ignore = "needs the lab server; see tests/common/mod.rs"]
async fn video_playback_by_range_and_whole() {
    let client = readonly();
    let videos: Vec<_> = client
        .search_all_assets(&AssetQuery::default(), 1000)
        .await
        .unwrap()
        .into_iter()
        .filter(|asset| asset.kind == AssetKind::Video)
        .collect();
    assert_eq!(videos.len(), 4);
    let video = &videos[0];
    let range = client.video_playback_range(&video.id, 0, 1023).await.unwrap();
    assert_eq!(range.bytes.len(), 1024);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("video");
    let written = client.download_video_playback(&video.id, &path).await.unwrap();
    assert_eq!(written, range.total_size);
    assert_eq!(std::fs::metadata(&path).unwrap().len(), written);
}

#[tokio::test]
#[ignore = "needs the lab server; see tests/common/mod.rs"]
async fn a_read_only_key_is_refused_each_change_with_the_permission_named() {
    let client = readonly();
    let id = timeline_ids(&client).await.remove(0);
    let album = album_id(&client, "Holiday Mix").await;
    let ids = vec![id.clone()];

    assert_refused(client.create_album(&NewAlbum { name: "x".to_owned(), ..NewAlbum::default() }).await, "POST", "/albums");
    assert_refused(client.update_album(&album, &AlbumChanges::default()).await, "PATCH", "/albums/{id}");
    assert_refused(client.delete_album(&album).await, "DELETE", "/albums/{id}");
    assert_refused(client.add_assets_to_album(&album, &ids).await, "PUT", "/albums/{id}/assets");
    assert_refused(client.remove_assets_from_album(&album, &ids).await, "DELETE", "/albums/{id}/assets");
    assert_refused(client.add_assets_to_albums(std::slice::from_ref(&album), &ids).await, "PUT", "/albums/assets");
    assert_refused(client.album_statistics().await, "GET", "/albums/statistics");
    assert_refused(client.update_asset(&id, &AssetChanges::default()).await, "PUT", "/assets/{id}");
    assert_refused(client.update_assets(&ids, &AssetChanges::default()).await, "PUT", "/assets");
    assert_refused(client.delete_assets(&ids, false).await, "DELETE", "/assets");
    assert_refused(client.asset_statistics(&StatisticsFilter::default()).await, "GET", "/assets/statistics");
    assert_refused(client.asset_edits(&id).await, "GET", "/assets/{id}/edits");
    assert_refused(client.set_asset_edits(&id, &[AssetEdit::Rotate { angle: 90 }]).await, "PUT", "/assets/{id}/edits");
    assert_refused(client.delete_asset_edits(&id).await, "DELETE", "/assets/{id}/edits");
    assert_refused(
        client.set_asset_metadata(&id, &[("mobile-app".to_owned(), serde_json::json!({}))]).await,
        "PUT",
        "/assets/{id}/metadata",
    );
    assert_refused(client.copy_asset_properties(&id, &id, &CopyOptions::default()).await, "PUT", "/assets/copy");
    assert_refused(client.run_asset_job(&ids, &AssetJob::RefreshMetadata).await, "POST", "/assets/jobs");
    assert_refused(client.restore_assets(&ids).await, "POST", "/trash/restore/assets");
    assert_refused(client.empty_trash().await, "POST", "/trash/empty");
    assert_refused(client.tags().await, "GET", "/tags");
    assert_refused(client.upsert_tags(&["x".to_owned()]).await, "PUT", "/tags");
    assert_refused(client.map_markers(&MapQuery::default()).await, "GET", "/map/markers");
    assert_refused(client.reverse_geocode(0.0, 0.0).await, "GET", "/map/reverse-geocode");
    // What the key may do still works.
    client.asset_details(&id).await.unwrap();
    client.album(&album).await.unwrap();
    client.album_map_markers(&album).await.unwrap();
}
