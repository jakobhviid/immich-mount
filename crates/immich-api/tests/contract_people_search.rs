//! Contract tests for people and faces, stacks, duplicates, memories, search beyond metadata, folders and
//! downloads. Each test that changes something puts it back before it ends, and first clears what a failed run of
//! it may have left. See `tests/common/mod.rs` to run them.
//!
//! The lab server has no machine learning, so there are no detected faces or duplicates and smart search is off:
//! the tests make people, faces and a duplicate group by hand, and check smart search's refusal.

mod common;

use chrono::NaiveDate;
use common::{album_id, assert_refused, asset_named, full, readonly, timeline_ids};
use immich_api::{
    ApiError, AssetChanges, AssetKind, AssetQuery, BulkIdError, Client, DownloadSource, DuplicateResolution,
    MemoryChanges, MemoryData, MemoryFilter, MemoryKind, NewFace, NewMemory, NewPerson, PersonChanges, PersonOnAsset,
    SmartQuery, SuggestionKind, SuggestionQuery, Visibility,
};

/// A well-formed id that names nothing on the server.
const UNKNOWN_ID: &str = "00000000-0000-4000-8000-000000000000";

/// The duplicate group the tests make by hand.
const DUPLICATE_GROUP: &str = "11111111-2222-4333-8444-555555555555";

/// The seeded photo with the lab person's face (2448 by 3264 pixels).
const FACE_PHOTO: &str = "2016-08 Iceland Parliamentarians Meeting (29203692615).jpg";

/// Delete the people a failed run left (named `Contract ...`), and faces left without a person on the face photo.
async fn clear_contract_people(client: &Client, photo: &str) {
    let leftovers: Vec<String> = client
        .people()
        .await
        .unwrap()
        .into_iter()
        .filter(|person| person.name.starts_with("Contract"))
        .map(|person| person.id)
        .collect();
    if !leftovers.is_empty() {
        client.delete_people(&leftovers).await.unwrap();
    }
    for face in client.faces(photo).await.unwrap() {
        if face.person.is_none() {
            client.delete_face(&face.id, true).await.unwrap();
        }
    }
}

#[tokio::test]
#[ignore = "needs the lab server; see tests/common/mod.rs"]
async fn people_are_made_named_merged_and_deleted() {
    let client = full();
    let photo = asset_named(&client, FACE_PHOTO).await;
    clear_contract_people(&client, &photo).await;
    let people_before = client.people().await.unwrap();
    let faces_before = client.faces(&photo).await.unwrap();

    // The seeded person.
    let lab = people_before.iter().find(|person| person.name == "Lab Person").expect("the seeded person").clone();
    assert_eq!(client.person(&lab.id).await.unwrap().name, "Lab Person");
    for prefix in ["lab", "Person"] {
        let found = client.search_people(prefix, false).await.unwrap();
        assert!(found.iter().any(|person| person.id == lab.id), "searching {prefix}");
    }
    assert_eq!(client.person_statistics(&lab.id).await.unwrap(), 1);
    let thumbnail = client.person_thumbnail(&lab.id).await.unwrap();
    assert_eq!(thumbnail.content_type, "image/jpeg");
    assert!(thumbnail.bytes.starts_with(b"\xff\xd8"), "a JPEG");

    // Two people made by hand.
    let first = client
        .create_person(&NewPerson {
            name: Some("Contract First".to_owned()),
            birth_date: NaiveDate::from_ymd_opt(1990, 5, 17),
            is_favorite: Some(true),
            color: Some("#ff0000".to_owned()),
            ..NewPerson::default()
        })
        .await
        .unwrap();
    assert_eq!(first.birth_date, NaiveDate::from_ymd_opt(1990, 5, 17));
    assert!(first.is_favorite);
    assert_eq!(first.color.as_deref(), Some("#ff0000"));
    let second = client.create_person(&NewPerson { name: Some("Contract Second".to_owned()), ..NewPerson::default() }).await.unwrap();
    assert_eq!(client.person_thumbnail(&second.id).await.unwrap_err(), ApiError::NotFound, "no face, no thumbnail");

    // A face drawn by hand, moved from one to the other and back.
    client
        .create_face(&NewFace {
            asset_id: photo.clone(),
            person_id: first.id.clone(),
            x: 100,
            y: 100,
            width: 200,
            height: 200,
            image_width: 2448,
            image_height: 3264,
        })
        .await
        .unwrap();
    let faces = client.faces(&photo).await.unwrap();
    assert_eq!(faces.len(), faces_before.len() + 1);
    let face = faces
        .iter()
        .find(|face| face.person.as_ref().is_some_and(|person| person.id == first.id))
        .expect("the new face")
        .clone();
    assert_eq!((face.left, face.top, face.right, face.bottom), (100, 100, 300, 300));
    assert_eq!(face.image_width, 2448);
    assert_eq!(face.source_type, Some(immich_api::FaceSource::Manual));
    assert_eq!(client.person_statistics(&first.id).await.unwrap(), 1);

    let moved_to = client.reassign_face(&face.id, &second.id).await.unwrap();
    assert_eq!(moved_to.id, second.id);
    assert_eq!(client.person_statistics(&second.id).await.unwrap(), 1);
    assert_eq!(client.person_statistics(&first.id).await.unwrap(), 0);
    let moved_back = client
        .reassign_faces(&first.id, &[PersonOnAsset { person_id: second.id.clone(), asset_id: photo.clone() }])
        .await
        .unwrap();
    assert_eq!(moved_back.iter().map(|person| person.id.as_str()).collect::<Vec<_>>(), [first.id.as_str()]);
    assert_eq!(client.person_statistics(&first.id).await.unwrap(), 1);

    // Changes, one at a time and in bulk.
    let changed = client
        .update_person(
            &first.id,
            &PersonChanges {
                name: Some("Contract Renamed".to_owned()),
                birth_date: Some(None),
                color: Some(None),
                is_hidden: Some(true),
                feature_face_asset_id: Some(photo.clone()),
                ..PersonChanges::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(changed.name, "Contract Renamed");
    assert_eq!(changed.birth_date, None);
    assert!(changed.is_hidden);
    assert!(!client.search_people("Contract", false).await.unwrap().iter().any(|person| person.id == first.id));
    assert!(client.search_people("Contract", true).await.unwrap().iter().any(|person| person.id == first.id));
    let results = client
        .update_people(&[
            (first.id.clone(), PersonChanges { is_hidden: Some(false), ..PersonChanges::default() }),
            (UNKNOWN_ID.to_owned(), PersonChanges { name: Some("x".to_owned()), ..PersonChanges::default() }),
        ])
        .await
        .unwrap();
    assert!(results[0].success, "{results:?}");
    assert!(!results[1].success, "{results:?}");
    assert!(!client.person(&first.id).await.unwrap().is_hidden);

    // Merging: the answer leads with the person merged into, and an unknown id fails on its own.
    let merged = client.merge_people(&first.id, &[second.id.clone(), UNKNOWN_ID.to_owned()]).await.unwrap();
    let version = client.server_version().await.unwrap();
    if (version.major, version.minor) == (3, 2) {
        // v3.2.0 merges by person group, and people made by hand merge nothing (person.rs).
        assert_eq!(merged, [], "{version}");
        client.delete_person(&second.id).await.unwrap();
    } else {
        assert_eq!(merged.len(), 3, "{merged:?}");
        assert_eq!(merged[0].id, first.id);
        assert!(merged.iter().any(|result| result.id == second.id && result.success), "{merged:?}");
        assert_eq!(merged[2].error, Some(BulkIdError::NoPermission));
    }
    assert_eq!(client.person(&second.id).await.unwrap_err(), ApiError::NotFound);

    // A face marked deleted leaves the list, and can then be removed for good.
    client.delete_face(&face.id, false).await.unwrap();
    assert_eq!(client.faces(&photo).await.unwrap().len(), faces_before.len());
    assert_eq!(client.person_statistics(&first.id).await.unwrap(), 0);
    client.delete_face(&face.id, true).await.unwrap();

    // Deleting, one and several.
    let third = client.create_person(&NewPerson { name: Some("Contract Third".to_owned()), ..NewPerson::default() }).await.unwrap();
    let fourth = client.create_person(&NewPerson::default()).await.unwrap();
    client.delete_person(&fourth.id).await.unwrap();
    client.delete_people(&[first.id.clone(), third.id.clone()]).await.unwrap();
    assert_eq!(client.person(&first.id).await.unwrap_err(), ApiError::NotFound);
    let mut people_after: Vec<String> = client.people().await.unwrap().into_iter().map(|person| person.id).collect();
    let mut people_expected: Vec<String> = people_before.iter().map(|person| person.id.clone()).collect();
    people_after.sort();
    people_expected.sort();
    assert_eq!(people_after, people_expected);
    assert_eq!(client.faces(&photo).await.unwrap(), faces_before);
}

#[tokio::test]
#[ignore = "needs the lab server; see tests/common/mod.rs"]
async fn a_deleted_person_leaves_their_faces_without_a_person() {
    let client = full();
    let photo = asset_named(&client, FACE_PHOTO).await;
    clear_contract_people(&client, &photo).await;
    let person = client.create_person(&NewPerson { name: Some("Contract Gone".to_owned()), ..NewPerson::default() }).await.unwrap();
    let face = NewFace {
        asset_id: photo.clone(),
        person_id: person.id.clone(),
        x: 10,
        y: 10,
        width: 50,
        height: 50,
        image_width: 2448,
        image_height: 3264,
    };
    client.create_face(&face).await.unwrap();
    client.delete_person(&person.id).await.unwrap();
    let orphans: Vec<_> = client.faces(&photo).await.unwrap().into_iter().filter(|face| face.person.is_none()).collect();
    assert_eq!(orphans.len(), 1);
    client.delete_face(&orphans[0].id, true).await.unwrap();
    assert!(client.faces(&photo).await.unwrap().iter().all(|face| face.person.is_some()));
}

#[tokio::test]
#[ignore = "needs the lab server; see tests/common/mod.rs"]
async fn stacks_are_made_changed_and_deleted() {
    let client = full();
    for leftover in client.stacks(None).await.unwrap() {
        client.delete_stack(&leftover.id).await.unwrap();
    }
    let ids = timeline_ids(&client).await;
    let (first, second, third, fourth) = (&ids[0], &ids[1], &ids[2], &ids[3]);

    let stack = client.create_stack(&[first.clone(), second.clone(), third.clone()]).await.unwrap();
    assert_eq!(&stack.primary_asset_id, first, "the first Asset given is the primary");
    assert_eq!(stack.assets.len(), 3);
    assert_eq!(client.stacks(Some(first)).await.unwrap().len(), 1);
    assert!(client.stacks(Some(second)).await.unwrap().is_empty(), "the filter is on the primary");
    let summary = client.asset_details(second).await.unwrap().stack.expect("the Asset knows its stack");
    assert_eq!((summary.id.as_str(), summary.asset_count), (stack.id.as_str(), 3));

    let changed = client.set_stack_primary(&stack.id, second).await.unwrap();
    assert_eq!(&changed.primary_asset_id, second);
    assert!(
        matches!(client.remove_from_stack(&stack.id, second).await, Err(ApiError::Protocol(_))),
        "the primary cannot be removed"
    );
    client.remove_from_stack(&stack.id, third).await.unwrap();
    client.remove_from_stack(&stack.id, first).await.unwrap();
    let alone = client.stack(&stack.id).await.unwrap();
    assert_eq!(alone.assets.len(), 1, "a stack of one stays");
    client.delete_stack(&stack.id).await.unwrap();
    assert_eq!(client.stack(&stack.id).await.unwrap_err(), ApiError::NotFound);
    assert_eq!(client.asset_details(second).await.unwrap().stack, None);

    assert!(matches!(client.create_stack(std::slice::from_ref(first)).await, Err(ApiError::Protocol(_))), "one is too few");
    let one = client.create_stack(&[first.clone(), second.clone()]).await.unwrap();
    let two = client.create_stack(&[third.clone(), fourth.clone()]).await.unwrap();
    client.delete_stacks(&[one.id, two.id]).await.unwrap();
    assert!(client.stacks(None).await.unwrap().is_empty());
}

#[tokio::test]
#[ignore = "needs the lab server; see tests/common/mod.rs"]
async fn duplicates_are_dismissed_and_resolved() {
    let client = full();
    let kept = asset_named(&client, "タテ弁 01.jpg").await;
    let other = asset_named(&client, "タテ弁 02.jpg").await;
    let pair = [kept.clone(), other.clone()];
    let group_them = || AssetChanges { duplicate_id: Some(Some(DUPLICATE_GROUP.to_owned())), ..AssetChanges::default() };
    // A failed run may have left the group, or the other Asset in the trash.
    let _ = client.dismiss_duplicate_group(DUPLICATE_GROUP).await;
    client.restore_assets(std::slice::from_ref(&other)).await.unwrap();
    assert!(client.duplicates().await.unwrap().is_empty(), "no detection without machine learning");

    client.update_assets(&pair, &group_them()).await.unwrap();
    let groups = client.duplicates().await.unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].duplicate_id, DUPLICATE_GROUP);
    assert_eq!(groups[0].assets.len(), 2);
    assert!(!groups[0].suggested_keep_asset_ids.is_empty());
    assert_eq!(client.asset_details(&kept).await.unwrap().duplicate_id.as_deref(), Some(DUPLICATE_GROUP));
    client.dismiss_duplicate_group(DUPLICATE_GROUP).await.unwrap();
    assert!(client.duplicates().await.unwrap().is_empty());
    assert_eq!(client.asset_details(&kept).await.unwrap().duplicate_id, None);

    client.update_assets(&pair, &group_them()).await.unwrap();
    client.dismiss_duplicate_groups(&[DUPLICATE_GROUP.to_owned()]).await.unwrap();
    assert!(client.duplicates().await.unwrap().is_empty());
    assert!(!client.asset_details(&other).await.unwrap().is_trashed, "dismissing keeps every Asset");

    client.update_assets(&pair, &group_them()).await.unwrap();
    let resolved = client
        .resolve_duplicates(&[DuplicateResolution {
            duplicate_id: DUPLICATE_GROUP.to_owned(),
            keep_asset_ids: vec![kept.clone()],
            trash_asset_ids: vec![other.clone()],
        }])
        .await
        .unwrap();
    assert!(resolved[0].success, "{resolved:?}");
    assert!(client.duplicates().await.unwrap().is_empty());
    assert!(client.asset_details(&other).await.unwrap().is_trashed);
    assert!(!client.asset_details(&kept).await.unwrap().is_trashed);
    assert_eq!(client.restore_assets(std::slice::from_ref(&other)).await.unwrap(), 1);
    assert_eq!(client.asset_details(&other).await.unwrap().duplicate_id, None);
}

#[tokio::test]
#[ignore = "needs the lab server; see tests/common/mod.rs"]
async fn memories_are_made_filled_and_deleted() {
    let client = full();
    for leftover in client.memories(&MemoryFilter::default()).await.unwrap() {
        client.delete_memory(&leftover.id).await.unwrap();
    }
    let ids = timeline_ids(&client).await;
    let (first, second) = (&ids[0], &ids[1]);
    let instant = |text: &str| chrono::DateTime::parse_from_rfc3339(text).unwrap().to_utc();
    let before = client.memory_statistics(&MemoryFilter::default()).await.unwrap();

    let memory = client
        .create_memory(&NewMemory {
            kind: MemoryKind::OnThisDay,
            data: MemoryData { year: 2024, ..MemoryData::default() },
            memory_at: instant("2024-06-25T00:00:00Z"),
            asset_ids: vec![first.clone()],
            is_saved: None,
            show_at: Some(instant("2026-06-25T00:00:00Z")),
            hide_at: Some(instant("2026-06-26T00:00:00Z")),
            seen_at: None,
        })
        .await
        .unwrap();
    assert_eq!(memory.kind, MemoryKind::OnThisDay);
    assert_eq!(memory.data.year, 2024);
    assert_eq!(memory.memory_at, instant("2024-06-25T00:00:00Z"));
    assert_eq!(memory.assets.len(), 1);
    assert!(!memory.is_saved);
    assert_eq!(memory.seen_at, None);

    let added = client.add_memory_assets(&memory.id, &[first.clone(), second.clone()]).await.unwrap();
    assert_eq!(added[0].error, Some(BulkIdError::Duplicate));
    assert!(added[1].success);
    assert_eq!(client.memory(&memory.id).await.unwrap().assets.len(), 2);
    assert_eq!(client.memory_statistics(&MemoryFilter::default()).await.unwrap(), before + 1);
    let shown_on = |day| MemoryFilter { shown_on: NaiveDate::from_ymd_opt(2026, 6, day), ..MemoryFilter::default() };
    assert!(client.memories(&shown_on(25)).await.unwrap().iter().any(|candidate| candidate.id == memory.id));
    assert!(!client.memories(&shown_on(24)).await.unwrap().iter().any(|candidate| candidate.id == memory.id));
    assert_eq!(client.memory_statistics(&shown_on(24)).await.unwrap(), before);

    let changed = client
        .update_memory(&memory.id, &MemoryChanges { is_saved: Some(true), seen_at: Some(instant("2026-11-08T12:00:00Z")), ..MemoryChanges::default() })
        .await
        .unwrap();
    assert!(changed.is_saved);
    assert_eq!(changed.seen_at, Some(instant("2026-11-08T12:00:00Z")));
    let saved = MemoryFilter { saved: Some(true), ..MemoryFilter::default() };
    assert!(client.memories(&saved).await.unwrap().iter().any(|candidate| candidate.id == memory.id));

    let removed = client.remove_memory_assets(&memory.id, &[second.clone(), UNKNOWN_ID.to_owned()]).await.unwrap();
    assert!(removed[0].success);
    assert_eq!(removed[1].error, Some(BulkIdError::NotFound));

    // Without Assets a memory leaves the list, though it can still be read.
    client.remove_memory_assets(&memory.id, std::slice::from_ref(first)).await.unwrap();
    assert!(!client.memories(&MemoryFilter::default()).await.unwrap().iter().any(|candidate| candidate.id == memory.id));
    assert!(client.memory(&memory.id).await.unwrap().assets.is_empty());

    client.delete_memory(&memory.id).await.unwrap();
    assert_eq!(client.memory(&memory.id).await.unwrap_err(), ApiError::NotFound);
    assert_eq!(client.memory_statistics(&MemoryFilter::default()).await.unwrap(), before);
}

#[tokio::test]
#[ignore = "needs the lab server; see tests/common/mod.rs"]
async fn search_beyond_metadata() {
    let client = full();
    let library = client.search_all_assets(&AssetQuery::default(), 1000).await.unwrap();

    let smart = client.smart_search(&SmartQuery { text: Some("a mountain".to_owned()), ..SmartQuery::default() }, 10).await;
    match smart {
        Err(ApiError::Protocol(message)) => assert!(message.contains("Smart search is not enabled"), "{message}"),
        other => panic!("smart search without machine learning answered {other:?}"),
    }

    // Counting and random picks use the same filter as metadata search.
    assert_eq!(client.count_assets(&AssetQuery::default()).await.unwrap(), library.len() as u64);
    let timeline = AssetQuery { visibility: Some(Visibility::Timeline), ..AssetQuery::default() };
    assert_eq!(client.count_assets(&timeline).await.unwrap(), library.len() as u64 - 1, "one is archived");
    let videos = AssetQuery { kind: Some(AssetKind::Video), ..AssetQuery::default() };
    assert_eq!(client.count_assets(&videos).await.unwrap(), 4);
    let everything = client.random_assets(&AssetQuery::default(), 1000).await.unwrap();
    assert_eq!(everything.len(), library.len());
    assert!(everything.iter().all(|asset| !asset.is_trashed));
    let canon = AssetQuery { camera_make: Some("Canon".to_owned()), ..AssetQuery::default() };
    let picked = client.random_assets(&canon, 3).await.unwrap();
    assert_eq!(picked.len(), 3);
    assert!(picked.iter().all(|asset| asset.exif.as_ref().and_then(|exif| exif.make.as_deref()) == Some("Canon")));
    let canon_count = client.count_assets(&canon).await.unwrap();
    assert_eq!(client.search_all_assets(&canon, 1000).await.unwrap().len() as u64, canon_count);

    // The largest first.
    let large = client.large_assets(1_000_000, None, 100).await.unwrap();
    let sizes: Vec<u64> = large.iter().map(|asset| asset.exif.as_ref().and_then(|exif| exif.file_size).unwrap_or(0)).collect();
    assert!(!sizes.is_empty());
    assert!(sizes.iter().all(|size| *size >= 1_000_000), "{sizes:?}");
    assert!(sizes.windows(2).all(|pair| pair[0] >= pair[1]), "{sizes:?}");
    let on_timeline = client.large_assets(0, Some(&Visibility::Timeline), 1000).await.unwrap();
    assert_eq!(on_timeline.len(), library.len() - 1);

    // Explore, cities, suggestions and place names.
    let explore = client.explore().await.unwrap();
    let cities = explore.iter().find(|group| group.field_name == "exifInfo.city").expect("a city group");
    assert!(!cities.items.is_empty());
    for item in &cities.items {
        assert_eq!(item.asset.exif.as_ref().and_then(|exif| exif.city.clone()), Some(item.value.clone()));
    }
    let city_suggestions = client
        .suggestions(&SuggestionQuery { kind: SuggestionKind::City, ..SuggestionQuery::default() })
        .await
        .unwrap();
    let mut from_assets: Vec<String> =
        client.city_assets().await.unwrap().into_iter().filter_map(|asset| asset.exif.and_then(|exif| exif.city)).collect();
    from_assets.sort();
    let mut sorted_suggestions = city_suggestions.clone();
    sorted_suggestions.sort();
    assert_eq!(from_assets, sorted_suggestions, "one Asset per city");

    let countries = client
        .suggestions(&SuggestionQuery { kind: SuggestionKind::Country, ..SuggestionQuery::default() })
        .await
        .unwrap();
    assert_eq!(countries, client.countries().await.unwrap());
    let in_denmark = |kind| SuggestionQuery { kind, country: Some("Denmark".to_owned()), ..SuggestionQuery::default() };
    assert_eq!(client.suggestions(&in_denmark(SuggestionKind::City)).await.unwrap(), client.cities("Denmark").await.unwrap());
    assert!(client.suggestions(&in_denmark(SuggestionKind::State)).await.unwrap().contains(&"South Denmark".to_owned()));
    let makes = client
        .suggestions(&SuggestionQuery { kind: SuggestionKind::CameraMake, ..SuggestionQuery::default() })
        .await
        .unwrap();
    assert!(makes.contains(&"Canon".to_owned()));
    let models = client
        .suggestions(&SuggestionQuery {
            kind: SuggestionKind::CameraModel,
            camera_make: Some("Canon".to_owned()),
            ..SuggestionQuery::default()
        })
        .await
        .unwrap();
    assert!(!models.is_empty() && models.iter().all(|model| model.starts_with("Canon")), "{models:?}");
    client
        .suggestions(&SuggestionQuery { kind: SuggestionKind::LensModel, ..SuggestionQuery::default() })
        .await
        .unwrap();

    let places = client.search_places("Odense").await.unwrap();
    let odense = places.iter().find(|place| place.name == "Odense").expect("Odense in GeoNames");
    assert_eq!(odense.region.as_deref(), Some("South Denmark"));
    assert!((odense.latitude - 55.4).abs() < 0.2, "{odense:?}");
    assert!(client.search_places("zzzzqq").await.unwrap().is_empty());
}

#[tokio::test]
#[ignore = "needs the lab server; see tests/common/mod.rs"]
async fn folders_and_downloads() {
    let client = full();
    let paths = client.folder_paths().await.unwrap();
    assert!(!paths.is_empty());
    let mut sorted = paths.clone();
    sorted.sort();
    assert_eq!(paths, sorted);
    let in_folder = client.folder_assets(&paths[0]).await.unwrap();
    assert!(!in_folder.is_empty());
    assert!(in_folder.iter().all(|asset| asset.original_path.starts_with(&format!("{}/", paths[0]))));
    assert!(client.folder_assets("/nowhere").await.unwrap().is_empty());

    // Downloads need only `asset.download`, which the read-only key has.
    let reader = readonly();
    let holiday = album_id(&reader, "Holiday Mix").await;
    let plan = reader.download_plan(&DownloadSource::Album(holiday), Some(2_000_000)).await.unwrap();
    assert!(plan.archives.len() > 1, "split at 2 MB: {plan:?}");
    assert_eq!(plan.archives.iter().map(|archive| archive.size).sum::<u64>(), plan.total_size);
    assert_eq!(plan.archives.iter().map(|archive| archive.asset_ids.len()).sum::<usize>(), 5);

    let pair = vec![asset_named(&reader, "タテ弁 01.jpg").await, asset_named(&reader, "タテ弁 02.jpg").await];
    let pair_plan = reader.download_plan(&DownloadSource::Assets(pair.clone()), None).await.unwrap();
    assert_eq!(pair_plan.archives.len(), 1);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("pair.zip");
    let written = reader.download_archive(&pair, false, &path).await.unwrap();
    let archive = std::fs::read(&path).unwrap();
    assert_eq!(archive.len() as u64, written);
    assert!(archive.starts_with(b"PK\x03\x04"), "a ZIP");
    assert!(written > pair_plan.total_size, "stored, with headers");
}

#[tokio::test]
#[ignore = "needs the lab server; see tests/common/mod.rs"]
async fn a_read_only_key_is_refused_people_faces_stacks_duplicates_memories_and_folders() {
    let client = readonly();
    let id = timeline_ids(&client).await.remove(0);
    let ids = vec![id.clone()];
    let unknown = vec![UNKNOWN_ID.to_owned()];
    let lab = client.people().await.unwrap().into_iter().find(|person| person.name == "Lab Person").unwrap();

    assert_refused(client.create_person(&NewPerson::default()).await, "POST", "/people");
    assert_refused(client.update_person(UNKNOWN_ID, &PersonChanges::default()).await, "PUT", "/people/{id}");
    assert_refused(client.update_people(&[(UNKNOWN_ID.to_owned(), PersonChanges::default())]).await, "PUT", "/people");
    assert_refused(client.delete_person(UNKNOWN_ID).await, "DELETE", "/people/{id}");
    assert_refused(client.delete_people(&unknown).await, "DELETE", "/people");
    assert_refused(client.merge_people(UNKNOWN_ID, &unknown).await, "POST", "/people/{id}/merge");
    let face_of = PersonOnAsset { person_id: UNKNOWN_ID.to_owned(), asset_id: id.clone() };
    assert_refused(client.reassign_faces(UNKNOWN_ID, &[face_of]).await, "PUT", "/people/{id}/reassign");
    assert_refused(client.person_statistics(&lab.id).await, "GET", "/people/{id}/statistics");
    assert_refused(client.faces(&id).await, "GET", "/faces");
    let face = NewFace {
        asset_id: id.clone(),
        person_id: UNKNOWN_ID.to_owned(),
        width: 10,
        height: 10,
        image_width: 100,
        image_height: 100,
        ..NewFace::default()
    };
    assert_refused(client.create_face(&face).await, "POST", "/faces");
    assert_refused(client.reassign_face(UNKNOWN_ID, UNKNOWN_ID).await, "PUT", "/faces/{id}");
    assert_refused(client.delete_face(UNKNOWN_ID, true).await, "DELETE", "/faces/{id}");

    assert_refused(client.stacks(None).await, "GET", "/stacks");
    assert_refused(client.stack(UNKNOWN_ID).await, "GET", "/stacks/{id}");
    assert_refused(client.create_stack(&[id.clone(), UNKNOWN_ID.to_owned()]).await, "POST", "/stacks");
    assert_refused(client.set_stack_primary(UNKNOWN_ID, &id).await, "PUT", "/stacks/{id}");
    assert_refused(client.delete_stack(UNKNOWN_ID).await, "DELETE", "/stacks/{id}");
    assert_refused(client.delete_stacks(&unknown).await, "DELETE", "/stacks");
    assert_refused(client.remove_from_stack(UNKNOWN_ID, &id).await, "DELETE", "/stacks/{id}/assets/{assetId}");

    assert_refused(client.duplicates().await, "GET", "/duplicates");
    assert_refused(client.dismiss_duplicate_group(UNKNOWN_ID).await, "DELETE", "/duplicates/{id}");
    assert_refused(client.dismiss_duplicate_groups(&unknown).await, "DELETE", "/duplicates");
    let resolution = DuplicateResolution { duplicate_id: UNKNOWN_ID.to_owned(), keep_asset_ids: ids.clone(), trash_asset_ids: Vec::new() };
    assert_refused(client.resolve_duplicates(&[resolution]).await, "POST", "/duplicates/resolve");

    assert_refused(client.memories(&MemoryFilter::default()).await, "GET", "/memories");
    assert_refused(client.memory(UNKNOWN_ID).await, "GET", "/memories/{id}");
    let memory = NewMemory {
        kind: MemoryKind::OnThisDay,
        data: MemoryData { year: 2024, ..MemoryData::default() },
        memory_at: chrono::DateTime::parse_from_rfc3339("2024-06-25T00:00:00Z").unwrap().to_utc(),
        asset_ids: Vec::new(),
        is_saved: None,
        show_at: None,
        hide_at: None,
        seen_at: None,
    };
    assert_refused(client.create_memory(&memory).await, "POST", "/memories");
    assert_refused(client.update_memory(UNKNOWN_ID, &MemoryChanges::default()).await, "PUT", "/memories/{id}");
    assert_refused(client.delete_memory(UNKNOWN_ID).await, "DELETE", "/memories/{id}");
    assert_refused(client.add_memory_assets(UNKNOWN_ID, &ids).await, "PUT", "/memories/{id}/assets");
    assert_refused(client.remove_memory_assets(UNKNOWN_ID, &ids).await, "DELETE", "/memories/{id}/assets");
    assert_refused(client.memory_statistics(&MemoryFilter::default()).await, "GET", "/memories/statistics");

    assert_refused(client.count_assets(&AssetQuery::default()).await, "POST", "/search/statistics");
    assert_refused(client.folder_paths().await, "GET", "/view/folder/unique-paths");
    assert_refused(client.folder_assets("/").await, "GET", "/view/folder");

    // What the key may do still works.
    assert_eq!(client.person(&lab.id).await.unwrap().name, "Lab Person");
    assert!(!client.search_people("Lab", false).await.unwrap().is_empty());
    client.person_thumbnail(&lab.id).await.unwrap();
    assert_eq!(client.random_assets(&AssetQuery::default(), 2).await.unwrap().len(), 2);
    client.explore().await.unwrap();
    client.city_assets().await.unwrap();
    client.large_assets(0, None, 1).await.unwrap();
    client.search_places("Odense").await.unwrap();
    client.suggestions(&SuggestionQuery::default()).await.unwrap();
}
