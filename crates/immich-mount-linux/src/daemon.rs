//! The daemon's life (ARCHITECTURE.md, "The daemon's lifecycle"): detach a dead mount, mount, serve until stopped
//! or ejected, then unmount cleanly.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use immich_mount_core::content::{BlockCache, ContentSource};
use immich_mount_core::settings::Config;
use immich_mount_core::source::{LibrarySource, SourceError, ThumbnailSource};
use immich_mount_core::uploads::UploadTarget;
use immich_mount_core::store::MetadataStore;
use immich_mount_core::tree::Tree;

use crate::fuse::{mount, ImmichFilesystem};
use crate::mounts::{mount_state, unmount, MountState};
use crate::dbus::{Actions, State};
use crate::seeding::Seeder;
use crate::staging::Staging;

/// How often the daemon checks whether the user ejected the mount.
const EJECT_POLL: Duration = Duration::from_secs(2);
/// How often the daemon asks Immich what changed, so a change made elsewhere shows within seconds. Unchanged, that
/// costs two 304 answers, and for a session an empty sync stream.
const CHANGE_POLL: Duration = Duration::from_secs(10);

#[derive(Debug, thiserror::Error)]
pub enum DaemonError {
    #[error("{0} is already mounted, probably by another immich-mount daemon")]
    AlreadyMounted(PathBuf),
    #[error("could not prepare {path}: {message}")]
    Prepare { path: PathBuf, message: String },
    #[error("could not mount at {path}: {message}")]
    Mount { path: PathBuf, message: String },
}

/// Why the daemon stopped serving.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    /// SIGTERM or Ctrl-C (logout, `systemctl --user stop`).
    Signal,
    /// The user ejected the mount in the file manager. Respected until the next start.
    Ejected,
}

/// The server the daemon serves, as the app sees it over D-Bus.
pub struct ServerInfo {
    pub url: String,
    /// `None` when the server could not be reached at start.
    pub version: Option<String>,
    /// The signed-in account's email, when the credential may read it.
    pub account: Option<String>,
}

/// Mount the library at `mount_point` and serve it until stopped. `account_cache` holds this account's metadata store,
/// content cache and seeding ledger; `thumbnail_directory` is the freedesktop thumbnail cache seeds go into.
#[allow(clippy::too_many_arguments)]
pub fn serve<Source>(
    source: Arc<Source>,
    server: ServerInfo,
    config: Config,
    mount_point: &Path,
    account_cache: &Path,
    thumbnail_directory: &Path,
    not_uploaded_directory: &Path,
    runtime: &tokio::runtime::Runtime,
) -> Result<StopReason, DaemonError>
where
    Source: LibrarySource + ContentSource + ThumbnailSource + UploadTarget + 'static,
{
    let prepare_error = |path: &Path, message: String| DaemonError::Prepare { path: path.to_path_buf(), message };
    match mount_state(mount_point) {
        MountState::Mounted => return Err(DaemonError::AlreadyMounted(mount_point.to_path_buf())),
        MountState::Stale => {
            log::warn!("detaching a dead mount at {}", mount_point.display());
            unmount(mount_point, true).map_err(|message| prepare_error(mount_point, message))?;
        }
        MountState::NotMounted => {}
    }
    std::fs::create_dir_all(mount_point).map_err(|error| prepare_error(mount_point, error.to_string()))?;
    // Mounting over a folder with files would hide them (FUSE mounts over non-empty folders), and the home folder or a
    // folder holding the daemon's own cache would hide far more.
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    if home.as_deref().is_some_and(|home| crate::mounts::canonical_mount_point(home) == crate::mounts::canonical_mount_point(mount_point))
        || account_cache.starts_with(mount_point)
    {
        return Err(prepare_error(mount_point, "this folder cannot hold the mount; choose an empty folder such as ~/Immich".to_owned()));
    }
    if std::fs::read_dir(mount_point).map_err(|error| prepare_error(mount_point, error.to_string()))?.next().is_some() {
        return Err(prepare_error(mount_point, "the folder is not empty, and mounting over it would hide its files; choose an empty folder".to_owned()));
    }
    // The mount table, Files and the thumbnail URIs all use the resolved path; comparing against anything else
    // would read as an eject at once.
    let mount_point = &crate::mounts::canonical_mount_point(mount_point);

    let store = MetadataStore::open(&account_cache.join("metadata.redb")).map_err(|error| prepare_error(account_cache, error.to_string()))?;
    let (sharp_thumbnails, safety_net, cache_limit_bytes) = (config.sharp_thumbnails, config.thumbnail_safety_net, config.cache_limit_bytes);
    let uploads_enabled = config.uploads_enabled;
    let tree = Arc::new(Tree::new(source.clone(), store, config));
    let content_directory = account_cache.join("content");
    let seeder = Seeder::new(Arc::new(source.clone()), thumbnail_directory, account_cache, sharp_thumbnails)
        .map_err(|error| prepare_error(account_cache, error.to_string()))?;
    let disk = content_directory.clone();
    let disk_room: immich_mount_core::content::DiskRoomProbe = Arc::new(move || {
        let space = crate::fuse::free_space(&disk)?;
        let block_size = u64::from(space.block_size);
        Some(immich_mount_core::content::DiskRoom { available: space.available * block_size, total: space.blocks * block_size })
    });
    let content = Arc::new(
        BlockCache::new(source.clone(), &content_directory, cache_limit_bytes)
            .map_err(|error| prepare_error(&content_directory, error.to_string()))?
            .with_disk_floor(disk_room),
    );
    let mut filesystem = ImmichFilesystem::new(tree.clone(), content.clone(), runtime.handle().clone())
        .with_thumbnails(Arc::new(seeder), mount_point, safety_net);
    let mut staging = None;
    if uploads_enabled {
        let staging_directory = account_cache.join("staging");
        let area = Arc::new(Staging::new(&staging_directory, not_uploaded_directory).map_err(|error| prepare_error(&staging_directory, error.to_string()))?);
        // The server's own list of media types; the built-in one when it cannot be asked yet.
        let media = runtime.block_on(async { tokio::time::timeout(Duration::from_secs(10), source.media_types()).await }).ok().and_then(Result::ok);
        filesystem = filesystem.with_uploads(area.clone(), media.unwrap_or_default(), Arc::new(source.clone()));
        staging = Some(area);
    }
    let session = mount(filesystem, mount_point).map_err(|error| DaemonError::Mount { path: mount_point.to_path_buf(), message: error.to_string() })?;
    log::info!("mounted at {}", mount_point.display());

    let state = Arc::new(std::sync::Mutex::new(if server.version.is_some() { State::Online } else { State::Offline }));
    let actions = {
        let (usage_cache, clear_cache, refresh_tree, unmount_point) = (content.clone(), content.clone(), tree.clone(), mount_point.to_path_buf());
        Actions {
            cache_usage: Box::new(move || (usage_cache.used_bytes(), usage_cache.limit_bytes())),
            uploads: Box::new(move || staging.as_ref().map(|area| area.in_flight()).unwrap_or_default()),
            refresh: Box::new(move || refresh_tree.forget_all()),
            clear_cache: Box::new(move || clear_cache.clear()),
            unmount: Box::new(move || unmount(&unmount_point, false)),
        }
    };
    let mount_text = mount_point.display().to_string();
    let version = server.version.clone().unwrap_or_default();
    let account = server.account.clone().unwrap_or_default();
    let connection = runtime.block_on(crate::dbus::serve(state.clone(), &mount_text, &server.url, &version, &account, actions));
    runtime.spawn(follow_changes(source.clone(), tree.clone(), state, connection.clone()));

    let reason = runtime.block_on(wait_for_stop(mount_point));
    match reason {
        StopReason::Signal => log::info!("stopping"),
        StopReason::Ejected => log::info!("{} was unmounted from outside (eject); stopping", mount_point.display()),
    }
    drop(session);
    drop(connection);
    Ok(reason)
}

/// Poll for changes for as long as the daemon runs, and forget the listings they touch. The freshness windows stay
/// on underneath, for whatever polling cannot see.
/// It also tracks the connection state for D-Bus, telling the desktop once when the server becomes unreachable or
/// the credential stops working.
async fn follow_changes<Source, Names>(
    source: Arc<Source>,
    tree: Arc<Tree<Arc<Source>, Names>>,
    state: Arc<std::sync::Mutex<State>>,
    connection: Option<zbus::Connection>,
) where
    Source: LibrarySource + 'static,
    Names: immich_mount_core::tree::NameMemory + 'static,
{
    let mut interval = tokio::time::interval(CHANGE_POLL);
    // A poll can take up to its 25-second limit; the next one then waits a full interval, rather than following at
    // once to catch up.
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        interval.tick().await;
        let outcome = source.changes().await;
        let previous = *state.lock().expect("state lock");
        let current = match &outcome {
            Ok(_) => State::Online,
            Err(SourceError::Offline(_) | SourceError::Busy) => State::Offline,
            Err(SourceError::SignedOut) => State::SignedOut,
            Err(_) => previous,
        };
        if current != previous {
            *state.lock().expect("state lock") = current;
            let message = match current {
                State::Offline => "The Immich server cannot be reached. Files you opened before stay available.",
                State::SignedOut => "Immich no longer accepts this sign-in. Open Immich Mount to sign in again.",
                State::Online | State::Connecting => "Connected to Immich.",
            };
            log::info!("{}: {message}", current.as_str());
            if let Some(connection) = &connection {
                let _ = tokio::time::timeout(Duration::from_secs(3), crate::dbus::announce(connection, current, message)).await;
            }
            match current {
                State::Offline => crate::notify::notify_in_background("Immich is unreachable".to_owned(), message.to_owned()),
                State::SignedOut => crate::notify::notify_in_background("Signed out of Immich".to_owned(), message.to_owned()),
                State::Online | State::Connecting => {}
            }
        }
        match outcome {
            Ok(changes) if !changes.is_empty() => {
                let forgotten = tree.apply_changes(&changes);
                if changes.everything {
                    log::info!("Immich changed in a way that touches everything; {forgotten} folder(s) will be read again");
                } else {
                    log::info!(
                        "Immich changed: {} month(s), {} album(s); {forgotten} folder(s) will be read again",
                        changes.months.len(),
                        changes.albums.len()
                    );
                }
            }
            Ok(_) => {}
            Err(error) => log::debug!("change poll failed: {error}"),
        }
    }
}

async fn wait_for_stop(mount_point: &Path) -> StopReason {
    let mut terminate = match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
        Ok(signal) => signal,
        Err(error) => {
            log::error!("cannot listen for SIGTERM: {error}");
            return StopReason::Signal;
        }
    };
    let mut ticker = tokio::time::interval(EJECT_POLL);
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => return StopReason::Signal,
            _ = terminate.recv() => return StopReason::Signal,
            _ = ticker.tick() => {
                if mount_state(mount_point) == MountState::NotMounted {
                    return StopReason::Ejected;
                }
            }
        }
    }
}
