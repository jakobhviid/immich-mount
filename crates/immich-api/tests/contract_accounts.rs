//! Contract tests for sharing, the account and administration: a second user made for each test (and deleted at its
//! end), albums shared with them, comments and likes, partners, shared links, the account's own settings,
//! password, PIN code, sessions and API keys, notifications, and the admin's queues, libraries and server details.
//! See `tests/common/mod.rs` to run them.

mod common;

use common::{assert_refused, client_with, env, full, readonly, timeline_ids};
use immich_api::{
    ActivityFilter, ActivityKind, AlbumFilter, AlbumRole, ApiError, Client, Credential, LibrarySettings, LinkAddress,
    ManualJob, NewAlbum, NewMember, NewSharedLink, NewUser, NotificationFilter, PartnerDirection, Proof, QueueName,
    SharedLinkChanges, ThumbnailSize, UserChanges,
};

/// A throwaway second user, signed in with a password session.
struct OtherUser {
    id: String,
    email: String,
    password: String,
    client: Client,
}

async fn other_user(label: &str) -> OtherUser {
    let admin = full();
    // A failed earlier run may have left its user behind.
    let stale_prefix = format!("contract-{label}-");
    for stale in admin.admin_users(true).await.unwrap() {
        if stale.email.starts_with(&stale_prefix) {
            admin.delete_user(&stale.id, true).await.unwrap();
        }
    }
    let suffix = common::now().timestamp_micros();
    let email = format!("contract-{label}-{suffix}@example.com");
    let password = format!("contract-{suffix}");
    let created = admin
        .create_user(&NewUser {
            email: email.clone(),
            name: format!("Contract {label}"),
            password: password.clone(),
            ..NewUser::default()
        })
        .await
        .unwrap();
    let login = client_with(None).login_password(&email, &password).await.unwrap();
    OtherUser {
        id: created.id,
        email,
        password,
        client: client_with(Some(Credential::Session(login.access_token))),
    }
}

async fn remove(user: OtherUser) {
    let deleted = full().delete_user(&user.id, true).await.unwrap();
    assert!(deleted.deleted_at.is_some());
}

#[tokio::test]
#[ignore = "needs the lab server; see tests/common/mod.rs"]
async fn an_album_is_shared_commented_and_left() {
    let admin = full();
    let user = other_user("album").await;
    let ids = timeline_ids(&admin).await;
    let album = admin
        .create_album(&NewAlbum { name: "Contract shared".to_owned(), asset_ids: ids[..2].to_vec(), ..NewAlbum::default() })
        .await
        .unwrap();

    let shared = admin
        .add_album_members(&album.id, &[NewMember { user_id: user.id.clone(), role: AlbumRole::Viewer }])
        .await
        .unwrap();
    assert!(shared.shared);
    assert!(shared.members.iter().any(|member| member.user.id == user.id && member.role == AlbumRole::Viewer));
    admin.update_album_member(&album.id, &user.id, &AlbumRole::Editor).await.unwrap();
    let members = admin.album(&album.id).await.unwrap().members;
    assert!(members.iter().any(|member| member.user.id == user.id && member.role == AlbumRole::Editor));

    let seen = user
        .client
        .albums_where(&AlbumFilter { shared: Some(true), ..AlbumFilter::default() })
        .await
        .unwrap();
    assert!(seen.iter().any(|candidate| candidate.id == album.id));

    let comment = user.client.comment(&album.id, Some(&ids[0]), "a contract comment").await.unwrap();
    assert_eq!(comment.kind, ActivityKind::Comment);
    assert_eq!(comment.user.id, user.id);
    let like = admin.like(&album.id, None).await.unwrap();
    assert_eq!(like.asset_id, None);
    let all = admin.activities(&album.id, &ActivityFilter::default()).await.unwrap();
    assert_eq!(all.len(), 2);
    let comments = admin
        .activities(&album.id, &ActivityFilter { kind: Some(ActivityKind::Comment), ..ActivityFilter::default() })
        .await
        .unwrap();
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0].comment.as_deref(), Some("a contract comment"));
    let counts = admin.activity_statistics(&album.id, Some(&ids[0])).await.unwrap();
    assert_eq!(counts.comments, 1);
    user.client.delete_activity(&comment.id).await.unwrap();
    admin.delete_activity(&like.id).await.unwrap();
    assert!(admin.activities(&album.id, &ActivityFilter::default()).await.unwrap().is_empty());

    // The invitation reached the user as a notification.
    let notifications = user.client.notifications(&NotificationFilter::default()).await.unwrap();
    if let Some(invite) = notifications.first() {
        let read = user.client.mark_notification(&invite.id, Some(common::now())).await.unwrap();
        assert!(read.read_at.is_some());
        assert_eq!(user.client.notification(&invite.id).await.unwrap_err(), ApiError::NotFound, "v3.3.6 finds none by id");
        let unread = user
            .client
            .notifications(&NotificationFilter { unread_only: true, ..NotificationFilter::default() })
            .await
            .unwrap();
        assert!(!unread.iter().any(|candidate| candidate.id == invite.id));
        user.client.mark_notifications(std::slice::from_ref(&invite.id), None).await.unwrap();
        user.client.delete_notifications(std::slice::from_ref(&invite.id)).await.unwrap();
    }
    println!("notifications seen by the invited user: {}", notifications.len());

    user.client.remove_album_member(&album.id, "me").await.unwrap();
    assert!(!admin.album(&album.id).await.unwrap().members.iter().any(|member| member.user.id == user.id));
    admin.delete_album(&album.id).await.unwrap();
    remove(user).await;
}

#[tokio::test]
#[ignore = "needs the lab server; see tests/common/mod.rs"]
async fn partners_share_their_libraries() {
    let admin = full();
    let user = other_user("partner").await;
    let partner = admin.add_partner(&user.id).await.unwrap();
    assert_eq!(partner.id, user.id);
    let shared_by = admin.partners(&PartnerDirection::SharedBy).await.unwrap();
    assert!(shared_by.iter().any(|candidate| candidate.id == user.id));
    let shared_with = user.client.partners(&PartnerDirection::SharedWith).await.unwrap();
    let admin_id = admin.me().await.unwrap().id;
    assert!(shared_with.iter().any(|candidate| candidate.id == admin_id));
    let shown = user.client.set_partner_in_timeline(&admin_id, true).await.unwrap();
    assert_eq!(shown.in_timeline, Some(true));
    admin.remove_partner(&user.id).await.unwrap();
    assert!(admin.partners(&PartnerDirection::SharedBy).await.unwrap().is_empty());
    remove(user).await;
}

#[tokio::test]
#[ignore = "needs the lab server; see tests/common/mod.rs"]
async fn shared_links_open_by_key_and_password() {
    let admin = full();
    let ids = timeline_ids(&admin).await;
    let link = admin.create_shared_link(&NewSharedLink::to_assets(&ids[..2])).await.unwrap();
    assert_eq!(link.assets.len(), 2);
    assert!(admin.shared_links(None).await.unwrap().iter().any(|candidate| candidate.id == link.id));
    assert_eq!(admin.shared_link(&link.id).await.unwrap().key, link.key);

    let anonymous = client_with(None);
    let opened = anonymous.shared_link_by_key(&LinkAddress::Key(link.key.clone())).await.unwrap();
    assert_eq!(opened.id, link.id);

    let added = admin.add_assets_to_shared_link(&link.id, &ids[2..3]).await.unwrap();
    assert!(added[0].success, "{added:?}");
    assert_eq!(added[0].asset_id, ids[2]);
    let removed = admin.remove_assets_from_shared_link(&link.id, &ids[2..3]).await.unwrap();
    assert!(removed[0].success);

    let changed = admin
        .update_shared_link(
            &link.id,
            &SharedLinkChanges {
                password: Some(Some("contract".to_owned())),
                description: Some(Some("for the contract".to_owned())),
                slug: Some(Some(format!("contract-{}", common::now().timestamp_micros()))),
                allow_download: Some(false),
                ..SharedLinkChanges::default()
            },
        )
        .await
        .unwrap();
    assert!(!changed.allow_download);
    assert_eq!(changed.description.as_deref(), Some("for the contract"));
    let slug = changed.slug.clone().expect("the slug");
    assert_eq!(
        anonymous.shared_link_by_key(&LinkAddress::Slug(slug)).await.unwrap_err(),
        ApiError::Unauthorized,
        "a link with a password needs it"
    );
    let unlocked = anonymous.shared_link_login(&LinkAddress::Key(link.key.clone()), "contract").await.unwrap();
    assert_eq!(unlocked.id, link.id);

    let holiday = common::album_id(&admin, "Holiday Mix").await;
    let album_link = admin.create_shared_link(&NewSharedLink::to_album(&holiday)).await.unwrap();
    assert_eq!(album_link.album.as_ref().map(|album| album.id.clone()), Some(holiday.clone()));
    assert_eq!(admin.shared_links(Some(&holiday)).await.unwrap().len(), 1);

    admin.delete_shared_link(&link.id).await.unwrap();
    admin.delete_shared_link(&album_link.id).await.unwrap();
    assert_eq!(
        anonymous.shared_link_by_key(&LinkAddress::Key(link.key)).await.unwrap_err(),
        ApiError::Unauthorized
    );
}

#[tokio::test]
#[ignore = "needs the lab server; see tests/common/mod.rs"]
async fn the_account_settings_password_and_pin_code() {
    let admin = full();
    let user = other_user("account").await;
    let me = user.client.me().await.unwrap();
    assert_eq!(me.email, user.email);
    assert!(!me.is_admin);
    let renamed = user
        .client
        .update_me(&immich_api::AccountChanges { name: Some("Contract renamed".to_owned()), ..Default::default() })
        .await
        .unwrap();
    assert_eq!(renamed.name, "Contract renamed");
    assert!(user.client.users().await.unwrap().iter().any(|other| other.email == env("IMMICH_ADMIN_EMAIL")));
    assert_eq!(user.client.user(&user.id).await.unwrap().name, "Contract renamed");

    let preferences = user.client.preferences().await.unwrap();
    let flipped = user
        .client
        .update_preferences(&serde_json::json!({ "memories": { "enabled": !preferences.memories.enabled } }))
        .await
        .unwrap();
    assert_eq!(flipped.memories.enabled, !preferences.memories.enabled);
    assert_eq!(admin.user_preferences(&user.id).await.unwrap().memories.enabled, flipped.memories.enabled);

    let id = timeline_ids(&admin).await.remove(0);
    let (picture, _) = admin.thumbnail(&id, ThumbnailSize::Preview).await.unwrap();
    let stored = user.client.set_profile_image(picture, "me.jpg", "image/jpeg").await.unwrap();
    assert_eq!(stored.user_id, user.id);
    let fetched = admin.profile_image(&user.id).await.unwrap();
    assert!(!fetched.bytes.is_empty());
    user.client.delete_profile_image().await.unwrap();

    let status = user.client.auth_status().await.unwrap();
    assert!(status.password);
    assert!(!status.pin_code);
    user.client.set_pin_code("135790").await.unwrap();
    assert!(user.client.auth_status().await.unwrap().pin_code);
    user.client.unlock_session(&Proof::PinCode("135790".to_owned())).await.unwrap();
    assert!(user.client.auth_status().await.unwrap().is_elevated);
    user.client.lock_session().await.unwrap();
    assert!(!user.client.auth_status().await.unwrap().is_elevated);
    user.client.change_pin_code(&Proof::PinCode("135790".to_owned()), "246801").await.unwrap();
    user.client.remove_pin_code(&Proof::Password(user.password.clone())).await.unwrap();
    assert!(!user.client.auth_status().await.unwrap().pin_code);

    assert!(user.client.validate_credential().await.unwrap());
    let new_password = format!("{}-new", user.password);
    user.client.change_password(&user.password, &new_password, false).await.unwrap();
    client_with(None).login_password(&user.email, &new_password).await.unwrap();
    assert_eq!(
        client_with(None).login_password(&user.email, &user.password).await.unwrap_err(),
        ApiError::Unauthorized
    );
    remove(user).await;
}

#[tokio::test]
#[ignore = "needs the lab server; see tests/common/mod.rs"]
async fn sessions_and_api_keys() {
    let user = other_user("keys").await;
    let sessions = user.client.sessions().await.unwrap();
    assert!(sessions.iter().any(|session| session.current));

    let device = user.client.create_session("Contract", "Linux", Some(3600)).await.unwrap();
    assert!(!device.token.is_empty());
    let device_client = client_with(Some(Credential::Session(device.token.clone())));
    assert_eq!(device_client.me().await.unwrap().id, user.id);
    user.client.lock_other_session(&device.session.id).await.unwrap();
    user.client.delete_session(&device.session.id).await.unwrap();
    assert_eq!(device_client.me().await.unwrap_err(), ApiError::Unauthorized);
    let second = user.client.create_session("Contract", "Linux", None).await.unwrap();
    user.client.delete_other_sessions().await.unwrap();
    assert_eq!(
        client_with(Some(Credential::Session(second.token))).me().await.unwrap_err(),
        ApiError::Unauthorized
    );
    assert!(full().user_sessions(&user.id).await.unwrap().len() == 1);

    let created = user.client.create_api_key("contract", &["asset.read".to_owned()]).await.unwrap();
    let key_client = client_with(Some(Credential::ApiKey(created.secret.clone())));
    assert_eq!(key_client.api_key_permissions().await.unwrap(), ["asset.read"]);
    let renamed = user
        .client
        .update_api_key(&created.key.id, Some("contract renamed"), Some(&["asset.read".to_owned(), "album.read".to_owned()]))
        .await
        .unwrap();
    assert_eq!(renamed.name, "contract renamed");
    assert_eq!(user.client.api_key(&created.key.id).await.unwrap().permissions.len(), 2);
    let rotated = user.client.rotate_api_key(&created.key.id).await.unwrap();
    assert_ne!(rotated.secret, created.secret);
    assert_eq!(key_client.api_key_permissions().await.unwrap_err(), ApiError::Unauthorized);
    assert!(user.client.api_keys().await.unwrap().iter().any(|key| key.id == created.key.id));
    user.client.delete_api_key(&created.key.id).await.unwrap();
    assert!(user.client.api_keys().await.unwrap().is_empty());
    remove(user).await;
}

#[tokio::test]
#[ignore = "needs the lab server; see tests/common/mod.rs"]
async fn a_user_empties_their_own_trash() {
    let admin = full();
    let user = other_user("trash").await;
    let id = timeline_ids(&admin).await.remove(0);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("copy.jpg");
    admin.download_original(&id, &path).await.unwrap();
    let outcome = user
        .client
        .upload_asset(&path, "contract-trash.jpg", common::now(), common::now())
        .await
        .unwrap();
    let uploaded = match outcome {
        immich_api::UploadOutcome::Created(id) => id,
        other => panic!("{other:?}"),
    };
    user.client.delete_assets(std::slice::from_ref(&uploaded), false).await.unwrap();
    assert_eq!(user.client.empty_trash().await.unwrap(), 1);
    remove(user).await;
}

#[tokio::test]
#[ignore = "needs the lab server; see tests/common/mod.rs"]
async fn users_are_deleted_and_restored() {
    let admin = full();
    let user = other_user("restore").await;
    let listed = admin.admin_users(false).await.unwrap();
    assert!(listed.iter().any(|candidate| candidate.id == user.id));
    assert_eq!(admin.user_statistics(&user.id, None).await.unwrap().total, 0);
    let changed = admin
        .update_user(
            &user.id,
            &UserChanges { quota_size_in_bytes: Some(Some(1 << 30)), should_change_password: Some(true), ..UserChanges::default() },
        )
        .await
        .unwrap();
    assert_eq!(changed.quota_size_in_bytes, Some(1 << 30));
    assert!(changed.should_change_password);

    let deleted = admin.delete_user(&user.id, false).await.unwrap();
    assert!(deleted.deleted_at.is_some());
    assert!(!admin.admin_users(false).await.unwrap().iter().any(|candidate| candidate.id == user.id));
    assert!(admin.admin_users(true).await.unwrap().iter().any(|candidate| candidate.id == user.id));
    let restored = admin.restore_user(&user.id).await.unwrap();
    assert_eq!(restored.deleted_at, None);
    assert_eq!(admin.admin_user(&user.id).await.unwrap().email, user.email);
    remove(user).await;
}

#[tokio::test]
#[ignore = "needs the lab server; see tests/common/mod.rs"]
async fn queues_libraries_and_the_server() {
    let admin = full();
    let queues = admin.queues().await.unwrap();
    assert!(queues.iter().any(|queue| queue.name == QueueName::ThumbnailGeneration));
    let paused = admin.set_queue_paused(&QueueName::Library, true).await.unwrap();
    assert!(paused.is_paused);
    assert!(admin.queue(&QueueName::Library).await.unwrap().is_paused);
    assert!(!admin.set_queue_paused(&QueueName::Library, false).await.unwrap().is_paused);
    admin.queued_jobs(&QueueName::ThumbnailGeneration).await.unwrap();
    admin.clear_queue(&QueueName::Library, false).await.unwrap();
    admin.start_job(&ManualJob::TagCleanup).await.unwrap();

    let owner = admin.me().await.unwrap().id;
    let library = admin
        .create_library(
            &owner,
            &LibrarySettings {
                name: Some("Contract library".to_owned()),
                import_paths: Some(vec!["/nonexistent-contract".to_owned()]),
                exclusion_patterns: Some(vec!["**/@eaDir/**".to_owned()]),
            },
        )
        .await
        .unwrap();
    assert_eq!(library.import_paths, ["/nonexistent-contract"]);
    let checks = admin
        .validate_library(&library.id, &LibrarySettings { import_paths: Some(library.import_paths.clone()), ..LibrarySettings::default() })
        .await
        .unwrap();
    assert!(!checks[0].is_valid, "{checks:?}");
    let renamed = admin
        .update_library(&library.id, &LibrarySettings { name: Some("Contract library 2".to_owned()), ..LibrarySettings::default() })
        .await
        .unwrap();
    assert_eq!(renamed.name, "Contract library 2");
    assert_eq!(admin.library_statistics(&library.id).await.unwrap().total, 0);
    assert!(admin.libraries().await.unwrap().iter().any(|candidate| candidate.id == library.id));
    assert_eq!(admin.library(&library.id).await.unwrap().owner_id, owner);
    admin.scan_library(&library.id).await.unwrap();
    admin.delete_library(&library.id).await.unwrap();

    admin.ping().await.unwrap();
    let version = admin.server_version().await.unwrap();
    let about = admin.server_about().await.unwrap();
    assert_eq!(about.version, version.to_string());
    assert!(!admin.version_history().await.unwrap().is_empty());
    let statistics = admin.server_statistics().await.unwrap();
    assert!(statistics.photos >= 47, "{statistics:?}");
    assert!(!statistics.usage_by_user.is_empty());
    assert!(admin.server_storage().await.unwrap().disk_size > 0);
}

#[tokio::test]
#[ignore = "needs the lab server; see tests/common/mod.rs"]
async fn a_read_only_key_is_refused_sharing_account_and_admin_calls() {
    let client = readonly();
    let album = common::album_id(&client, "Holiday Mix").await;
    assert_refused(client.shared_links(None).await, "GET", "/shared-links");
    assert_refused(client.create_shared_link(&NewSharedLink::to_album(&album)).await, "POST", "/shared-links");
    assert_refused(client.partners(&PartnerDirection::SharedBy).await, "GET", "/partners");
    assert_refused(client.activities(&album, &ActivityFilter::default()).await, "GET", "/activities");
    assert_refused(client.like(&album, None).await, "POST", "/activities");
    assert_refused(client.users().await, "GET", "/users");
    assert_refused(client.preferences().await, "GET", "/users/me/preferences");
    assert_refused(client.sessions().await, "GET", "/sessions");
    assert_refused(client.api_keys().await, "GET", "/api-keys");
    assert_refused(client.notifications(&NotificationFilter::default()).await, "GET", "/notifications");
    assert_refused(client.admin_users(false).await, "GET", "/admin/users");
    assert_refused(client.queues().await, "GET", "/queues");
    assert_refused(client.libraries().await, "GET", "/libraries");
    assert_refused(client.server_about().await, "GET", "/server/about");
    assert_refused(client.server_statistics().await, "GET", "/server/statistics");
    assert_refused(client.server_storage().await, "GET", "/server/storage");
    assert_refused(client.start_job(&ManualJob::TagCleanup).await, "POST", "/jobs");
    // What needs no permission still answers.
    client.ping().await.unwrap();
    client.version_history().await.unwrap();
    // A signed-in endpoint whose spec names no permission needs `all` from a key.
    assert_eq!(client.auth_status().await.unwrap_err(), ApiError::Forbidden { missing_permission: Some("all".to_owned()) });
}
