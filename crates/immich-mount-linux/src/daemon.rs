//! The daemon's life (ARCHITECTURE.md, "The daemon's lifecycle"): detach a dead mount, mount, serve until stopped
//! or ejected, then unmount cleanly.

use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

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
/// How often the daemon looks at the program it was started from, which an upgrade replaces and an uninstall removes.
const PROGRAM_POLL: Duration = Duration::from_secs(30);
/// How long the program may be missing before the daemon takes it as uninstalled: an upgrade removes the old version
/// before it puts the new one in place.
const UNINSTALL_GRACE: Duration = Duration::from_secs(120);
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
    /// The program was upgraded: the daemon left once nothing was open or uploading, so its service starts the new one.
    Upgraded,
    /// The program was uninstalled: the daemon left once idle, and its service is to be turned off.
    Uninstalled,
}

/// What became of the program the daemon was started from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProgramChange {
    Unchanged,
    Upgraded,
    Uninstalled,
}

/// The program the user service started: the path it ran (`argv[0]`, which systemd gives absolute) and the file this
/// process runs. A package manager replaces a program by putting a new file in its place, so the path then names a
/// file other than the running one; an uninstall leaves nothing there.
struct ProgramWatch {
    path: PathBuf,
    running: (u64, u64),
    missing_since: Option<Instant>,
}

impl ProgramWatch {
    /// `None` when started by a relative name (from a terminal), where there is no path to watch.
    fn new() -> Option<ProgramWatch> {
        let path = PathBuf::from(std::env::args_os().next()?);
        if !path.is_absolute() {
            return None;
        }
        let running = std::fs::metadata("/proc/self/exe").ok()?;
        Some(ProgramWatch { path, running: (running.dev(), running.ino()), missing_since: None })
    }

    fn check(&mut self, now: Instant) -> ProgramChange {
        let found = std::fs::metadata(&self.path).ok().map(|metadata| (metadata.dev(), metadata.ino()));
        self.judge(found, now)
    }

    /// The change, given the file now at the path (device and inode), if any.
    fn judge(&mut self, found: Option<(u64, u64)>, now: Instant) -> ProgramChange {
        match found {
            Some(file) if file == self.running => {
                self.missing_since = None;
                ProgramChange::Unchanged
            }
            Some(_) => {
                self.missing_since = None;
                ProgramChange::Upgraded
            }
            None => {
                let since = *self.missing_since.get_or_insert(now);
                if now.duration_since(since) >= UNINSTALL_GRACE {
                    ProgramChange::Uninstalled
                } else {
                    ProgramChange::Unchanged
                }
            }
        }
    }
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
    // The mount table and GVfs use the resolved path; comparing against anything else would read as an eject at once.
    // Where home is reached through a symlink, file managers browsing from home use the configured one, so
    // thumbnails are seeded under both.
    let configured_mount_point = mount_point.to_path_buf();
    let mount_point = &crate::mounts::canonical_mount_point(mount_point);
    let mut thumbnail_roots = vec![mount_point.clone()];
    if configured_mount_point != *mount_point {
        thumbnail_roots.push(configured_mount_point);
    }

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
        .with_thumbnails(Arc::new(seeder), &thumbnail_roots, safety_net);
    let mut staging = None;
    if uploads_enabled {
        let staging_directory = account_cache.join("staging");
        let area = Arc::new(Staging::new(&staging_directory, not_uploaded_directory).map_err(|error| prepare_error(&staging_directory, error.to_string()))?);
        // The server's own list of media types; the built-in one when it cannot be asked yet.
        let media = runtime.block_on(async { tokio::time::timeout(Duration::from_secs(10), source.media_types()).await }).ok().and_then(Result::ok);
        filesystem = filesystem.with_uploads(area.clone(), media.unwrap_or_default(), Arc::new(source.clone()));
        staging = Some(area);
    }
    // The daemon leaves for an upgrade or an uninstall only when nothing is open or uploading.
    let open_files = filesystem.open_files();
    let staging_for_idle = staging.clone();
    let idle = move || open_files.load(Ordering::Relaxed) == 0 && staging_for_idle.as_ref().is_none_or(|area| area.is_idle());
    let session = mount(filesystem, mount_point).map_err(|error| DaemonError::Mount { path: mount_point.to_path_buf(), message: error.to_string() })?;
    log::info!("mounted at {}", mount_point.display());

    let state = Arc::new(std::sync::Mutex::new(if server.version.is_some() { State::Online } else { State::Offline }));
    let actions = {
        let (usage_cache, clear_cache, refresh_tree, unmount_point) = (content.clone(), content.clone(), tree.clone(), mount_point.to_path_buf());
        Actions {
            cache_usage: Box::new(move || (usage_cache.used_bytes(), usage_cache.limit_bytes())),
            fetched: Box::new({
                let content = content.clone();
                move || content.fetched()
            }),
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

    let reason = runtime.block_on(wait_for_stop(mount_point, idle));
    match reason {
        StopReason::Signal => log::info!("stopping"),
        StopReason::Ejected => log::info!("{} was unmounted from outside (eject); stopping", mount_point.display()),
        StopReason::Upgraded => log::info!("Immich Mount was upgraded and nothing is open or uploading; stopping so the new version starts"),
        StopReason::Uninstalled => log::info!("Immich Mount was uninstalled and nothing is open or uploading; stopping"),
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

async fn wait_for_stop(mount_point: &Path, idle: impl Fn() -> bool) -> StopReason {
    let mut terminate = match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
        Ok(signal) => signal,
        Err(error) => {
            log::error!("cannot listen for SIGTERM: {error}");
            return StopReason::Signal;
        }
    };
    let mut ticker = tokio::time::interval(EJECT_POLL);
    let mut program = ProgramWatch::new();
    let mut last_look = Instant::now();
    let mut change = ProgramChange::Unchanged;
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => return StopReason::Signal,
            _ = terminate.recv() => return StopReason::Signal,
            _ = ticker.tick() => {
                if mount_state(mount_point) == MountState::NotMounted {
                    return StopReason::Ejected;
                }
                if let Some(watch) = program.as_mut().filter(|_| last_look.elapsed() >= PROGRAM_POLL) {
                    last_look = Instant::now();
                    let seen = watch.check(last_look);
                    // An upgrade seen once stays seen; a program missing then back counts as an upgrade.
                    if change != ProgramChange::Upgraded {
                        if change == ProgramChange::Unchanged && seen != ProgramChange::Unchanged {
                            log::info!("{} was {}; leaving once nothing is open or uploading", watch.path.display(), match seen {
                                ProgramChange::Upgraded => "replaced by an upgrade",
                                _ => "uninstalled",
                            });
                        }
                        change = seen;
                    }
                }
                match change {
                    ProgramChange::Upgraded if idle() => return StopReason::Upgraded,
                    ProgramChange::Uninstalled if idle() => return StopReason::Uninstalled,
                    _ => {}
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_replaced_program_is_an_upgrade_and_a_long_missing_one_an_uninstall() {
        let start = Instant::now();
        let mut watch = ProgramWatch { path: PathBuf::from("/usr/bin/immich-mount"), running: (1, 10), missing_since: None };
        assert_eq!(watch.judge(Some((1, 10)), start), ProgramChange::Unchanged);
        assert_eq!(watch.judge(Some((1, 11)), start), ProgramChange::Upgraded);
        // Missing for a while is an upgrade in progress, not yet an uninstall.
        assert_eq!(watch.judge(None, start), ProgramChange::Unchanged);
        assert_eq!(watch.judge(None, start + UNINSTALL_GRACE / 2), ProgramChange::Unchanged);
        assert_eq!(watch.judge(None, start + UNINSTALL_GRACE), ProgramChange::Uninstalled);
        // Back in place, as a new file: the upgrade finished.
        assert_eq!(watch.judge(Some((1, 12)), start + UNINSTALL_GRACE), ProgramChange::Upgraded);
        assert_eq!(watch.missing_since, None);
    }
}
