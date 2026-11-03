//! The commands that need no server: settings, health, unmounting, signing out. Each returns a typed report; `main`
//! renders it for people or as JSON.

use std::path::PathBuf;

use anyhow::{Context, Result};
use immich_mount_core::settings::{self, Config, SETTINGS};
use immich_mount_linux::doctor::{self, DoctorReport};
use immich_mount_linux::mounts;
use immich_mount_linux::seeding;
use immich_mount_linux::service;
use immich_mount_linux::paths::Paths;
use serde::Serialize;

/// The config file's text, or empty when it does not exist yet.
pub fn config_text(paths: &Paths) -> Result<String> {
    match std::fs::read_to_string(paths.config_file()) {
        Ok(text) => Ok(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(error).with_context(|| format!("reading {}", paths.config_file().display())),
    }
}

pub fn load_config(paths: &Paths) -> Result<Config> {
    Ok(Config::from_toml(&config_text(paths)?)?)
}

fn write_config(paths: &Paths, text: &str) -> Result<()> {
    std::fs::create_dir_all(&paths.config_directory).with_context(|| format!("creating {}", paths.config_directory.display()))?;
    let temporary = paths.config_file().with_extension("toml.tmp");
    std::fs::write(&temporary, text).with_context(|| format!("writing {}", temporary.display()))?;
    std::fs::rename(&temporary, paths.config_file()).with_context(|| format!("replacing {}", paths.config_file().display()))
}

#[derive(Serialize)]
pub struct SettingRow {
    pub key: &'static str,
    pub value: String,
    pub default: &'static str,
    pub is_set: bool,
    pub description: &'static str,
}

/// Every setting with its effective value.
pub fn configure_list(paths: &Paths) -> Result<Vec<SettingRow>> {
    let stored_values = settings::stored_values(&config_text(paths)?)?;
    Ok(SETTINGS
        .iter()
        .map(|setting| {
            let stored = stored_values.get(setting.key).cloned();
            SettingRow {
                key: setting.key,
                is_set: stored.is_some(),
                value: stored.unwrap_or_else(|| setting.default.to_string()),
                default: setting.default,
                description: setting.description,
            }
        })
        .collect())
}

/// The value of one setting.
pub fn configure_get(paths: &Paths, key: &str) -> Result<SettingRow> {
    settings::setting(key)?;
    configure_list(paths)?.into_iter().find(|row| row.key == key).context("setting disappeared")
}

/// Set one setting, keeping the rest of the file as it was.
pub fn configure_set(paths: &Paths, key: &str, value: &str) -> Result<SettingRow> {
    // logout removes the credential file, so it must never name a folder.
    if key == "credential.file" && paths.expand(value).is_dir() {
        anyhow::bail!("credential.file must name a file, and {value} is a folder");
    }
    let updated = settings::set_value(&config_text(paths)?, key, value)?;
    write_config(paths, &updated)?;
    mount_at_login(paths, key, value);
    configure_get(paths, key)
}

/// Mounting at login is the user service being enabled, as the apps' switch does. systemctl's own messages are kept
/// off stdout, which `--json` reserves for one document.
fn mount_at_login(paths: &Paths, key: &str, value: &str) {
    if key == "mount.at_login" {
        let (verb, unit) = match value {
            "false" => ("disable", service::user_service()),
            _ => ("enable", service::ensure_user_service(paths).unwrap_or_else(|_| service::user_service())),
        };
        let _ = std::process::Command::new("systemctl").args(["--user", verb, unit]).output();
    }
}

/// Remove one setting, so its default applies again.
pub fn configure_unset(paths: &Paths, key: &str) -> Result<SettingRow> {
    let updated = settings::unset_value(&config_text(paths)?, key)?;
    write_config(paths, &updated)?;
    mount_at_login(paths, key, "true");
    configure_get(paths, key)
}

/// Where the mount goes, from the settings.
pub fn mount_point(paths: &Paths) -> Result<PathBuf> {
    Ok(paths.expand(&load_config(paths)?.mount_path))
}

pub fn doctor(paths: &Paths) -> Result<DoctorReport> {
    let mut checks = doctor::local_checks(paths, &mount_point(paths)?);
    checks.extend(online_checks(paths));
    Ok(doctor::report(checks))
}

/// The server and the credential, as `status` sees them: the checks that need the network. A locked or missing keyring
/// shows in the credential's check, since reading the credential is where it matters.
fn online_checks(paths: &Paths) -> Vec<doctor::Check> {
    use doctor::Status::{Ok as Fine, Problem, Skipped};
    let entry = |name, status, detail: String, fix: Option<&str>| doctor::Check { name, status, detail, fix: fix.map(str::to_owned) };
    let sign_in_again = Some("Sign in again with `immich-mount login`, or in the app");
    let report = match crate::online::status(paths) {
        Ok(report) => report,
        Err(error) => return vec![entry("server", Problem, format!("{error:#}"), None)],
    };
    let Some(server) = &report.server else {
        return vec![entry("server", Skipped, "no server is configured".to_owned(), Some("Sign in with `immich-mount login`, or in the app"))];
    };
    let detail = report.detail.clone().unwrap_or_default();
    let answered = match &server.version {
        Some(version) => entry("server", Fine, format!("{} answers (Immich {version})", server.url), None),
        None => entry("server", Skipped, format!("{} was not asked without a credential", server.url), None),
    };
    let unreachable = || entry("server", Problem, format!("{} did not answer: {detail}", server.url), Some("Check the address, the network, and that the server runs"));
    let unchecked = || entry("credential", Skipped, "not checked: the server did not answer".to_owned(), None);
    match (report.state, report.account.credential) {
        ("signed-out", None) => vec![answered, entry("credential", Problem, detail.clone(), sign_in_again)],
        ("signed-out", Some(_)) => vec![answered, entry("credential", Problem, "the server refused it".to_owned(), sign_in_again)],
        ("missing-permission", _) => vec![
            answered,
            entry("credential", Problem, detail.clone(), Some("Use an API key with the permissions in WORKFLOWS.md, section 2")),
        ],
        ("offline", _) if server.version.is_none() => vec![unreachable(), unchecked()],
        ("offline", _) => vec![answered, unchecked()],
        (_, kind) => vec![answered, entry("credential", Fine, format!("accepted ({})", kind.unwrap_or("none")), None)],
    }
}

#[derive(Serialize)]
pub struct MountReport {
    pub mount_point: PathBuf,
    pub was_mounted: bool,
}

/// How long `mount` waits for the daemon to mount.
const MOUNT_WAIT: std::time::Duration = std::time::Duration::from_secs(15);

/// Start the user service (the daemon), writing it first if no install brought one, enable it when `mount.at_login`
/// says so, and wait until the mount is up. Until the daemon has its D-Bus control, the service is how the mount is
/// started (ARCHITECTURE.md, "The daemon's lifecycle").
pub fn mount(paths: &Paths) -> Result<MountReport> {
    let config = load_config(paths)?;
    let mount_point = paths.expand(&config.mount_path);
    if mounts::mount_state(&mount_point) == mounts::MountState::Mounted {
        return Ok(MountReport { mount_point, was_mounted: true });
    }
    let unit = service::ensure_user_service(paths).context("setting up the user service")?;
    if config.mount_at_login {
        let _ = std::process::Command::new("systemctl").args(["--user", "enable", unit]).output();
    }
    let started = std::process::Command::new("systemctl")
        .args(["--user", "start", unit])
        .output()
        .context("running systemctl --user")?;
    if !started.status.success() {
        anyhow::bail!(
            "could not start the user service {unit}: {}",
            String::from_utf8_lossy(&started.stderr).trim()
        );
    }
    let deadline = std::time::Instant::now() + MOUNT_WAIT;
    while std::time::Instant::now() < deadline {
        if mounts::mount_state(&mount_point) == mounts::MountState::Mounted {
            return Ok(MountReport { mount_point, was_mounted: false });
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    anyhow::bail!("the service started but did not mount within 15 seconds; see `journalctl --user -u immich-mount`")
}

#[derive(Serialize)]
pub struct UnmountReport {
    pub mount_point: PathBuf,
    pub was: mounts::MountState,
}

/// Unmount the mount, lazily if it is a dead one.
pub fn unmount(paths: &Paths) -> Result<UnmountReport> {
    let mount_point = mount_point(paths)?;
    let was = mounts::mount_state(&mount_point);
    if was != mounts::MountState::NotMounted {
        match mounts::unmount(&mount_point, was == mounts::MountState::Stale) {
            // A file manager showing the mount keeps it busy. Detach it as an eject does, so the windows leave it,
            // unless a copy into Photos would be cut short.
            Err(message) if mounts::is_busy(&message) => {
                let uploads = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map(|runtime| runtime.block_on(immich_mount_linux::dbus::uploads_in_flight()))
                    .unwrap_or_default();
                if let Some((name, _, _)) = uploads.first() {
                    anyhow::bail!("{name} is still being copied into Photos; unmount once it has finished");
                }
                mounts::unmount(&mount_point, true).map_err(anyhow::Error::msg)?;
            }
            outcome => outcome.map_err(anyhow::Error::msg)?,
        }
    }
    Ok(UnmountReport { mount_point, was })
}

/// Take away what the mount left in the desktop's shared places: its place in Dolphin, its bookmark in Files'
/// sidebar and the GVfs drop-in, and every thumbnail under it, our seeds and anything a thumbnailer made or recorded
/// as failed there, since their URIs carry album and person names. At logout, and by the daemon once its program is
/// uninstalled. Returns how many thumbnails went.
pub fn remove_desktop_traces(paths: &Paths, mount_point: &std::path::Path) -> Result<usize> {
    if let Err(error) = immich_mount_linux::kde::remove_place(paths, mount_point) {
        eprintln!("warning: could not remove the mount from Dolphin's Places: {error}");
    }
    if let Err(error) = immich_mount_linux::gnome::remove_bookmark(paths, mount_point) {
        eprintln!("warning: could not remove the mount's bookmark from Files' sidebar: {error}");
    }
    if let Err(error) = immich_mount_linux::gnome::remove_sidebar_drop_in(paths) {
        eprintln!("warning: could not remove the GVfs drop-in: {error}");
    }
    // Under the resolved path, and the configured one where home is reached through a symlink (both are seeded).
    let mut mount_prefixes = vec![immich_mount_linux::thumbnails::file_uri(&mounts::canonical_mount_point(mount_point))];
    let configured_prefix = immich_mount_linux::thumbnails::file_uri(mount_point);
    if !mount_prefixes.contains(&configured_prefix) {
        mount_prefixes.push(configured_prefix);
    }
    let mut removed = 0;
    for prefix in &mount_prefixes {
        removed += immich_mount_linux::thumbnails::remove_thumbnails_under(&paths.thumbnail_directory, &format!("{}/", prefix.trim_end_matches('/')));
    }
    if let Ok(accounts) = std::fs::read_dir(&paths.cache_directory) {
        for account in accounts.flatten() {
            // The sweep above found them already, unless the mount point moved since; the ledger catches those.
            removed += seeding::remove_seeds(&account.path(), &paths.thumbnail_directory)
                .with_context(|| format!("removing the thumbnails seeded for {}", account.path().display()))?;
        }
    }
    Ok(removed)
}

#[derive(Serialize)]
pub struct LogoutReport {
    /// Whether the session was ended on the server (sessions only; an API key stays valid until deleted in Immich).
    pub session_ended: bool,
    /// Whether the mount had to be unmounted first.
    pub unmounted: bool,
    /// How many seeded thumbnails were removed from the shared thumbnail cache.
    pub seeded_thumbnails_removed: usize,
    /// Whether the keyring items were removed (or there were none to remove).
    pub keyring_item_removed: bool,
    pub removed: Vec<PathBuf>,
}

/// The user services that run the daemon: the packages' own, and Homebrew's (`brew services`).
const USER_SERVICES: [&str; 2] = [service::PACKAGED, service::HOMEBREW];

/// Unmount, remove every seeded thumbnail, then the credential and every cache. Settings are kept. The caller ends
/// the session on the server first, while the credential still exists.
///
/// Unmounting first ends a running daemon (it treats it as an eject), and the user service is stopped and waited for,
/// so its background seeding has ended before the caches and thumbnails are removed.
pub fn logout(paths: &Paths, session_ended: bool) -> Result<LogoutReport> {
    let config = load_config(paths)?;
    let mount_point = paths.expand(&config.mount_path);
    let state = mounts::mount_state(&mount_point);
    if state != mounts::MountState::NotMounted {
        mounts::unmount(&mount_point, state == mounts::MountState::Stale).map_err(anyhow::Error::msg)?;
    }
    for unit in USER_SERVICES {
        let _ = std::process::Command::new("systemctl").args(["--user", "stop", unit]).output();
    }

    let seeded_thumbnails_removed = remove_desktop_traces(paths, &mount_point)?;

    let keyring_item_removed = remove_from_keyring();

    // Both places a credential file can be: the configured one, and the default one (the fallback without a keyring).
    let mut credentials = vec![paths.default_credential_file()];
    if let Some(configured) = config.credential_file.map(|file| paths.expand(&file)).filter(|file| *file != credentials[0]) {
        credentials.insert(0, configured);
    }
    let mut removed = Vec::new();
    for credential in credentials {
        // A credential is one file, never a folder, whatever the setting says: a mistaken `credential.file = ~` must
        // not take the home folder with it. Only the cache directory, which is ours alone, is removed whole.
        if credential.is_dir() {
            eprintln!("warning: credential.file names a folder ({}); it is left alone", credential.display());
            continue;
        }
        match std::fs::remove_file(&credential) {
            Ok(()) => removed.push(credential),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).with_context(|| format!("removing {}", credential.display())),
        }
    }
    match std::fs::remove_dir_all(&paths.cache_directory) {
        Ok(()) => removed.push(paths.cache_directory.clone()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error).with_context(|| format!("removing {}", paths.cache_directory.display())),
    }
    // Signed out, the daemon can only fail at every start, and its unit restarts on failure, so the unit goes off too.
    // The next `immich-mount login` says how to turn it on again; the apps do it at their first-run setup.
    for unit in USER_SERVICES {
        let _ = std::process::Command::new("systemctl").args(["--user", "disable", unit]).output();
    }
    Ok(LogoutReport { session_ended, unmounted: state != mounts::MountState::NotMounted, seeded_thumbnails_removed, keyring_item_removed, removed })
}

/// Remove every keyring item Immich Mount made, for any server, if a keyring is reachable. A missing keyring is not
/// an error here.
fn remove_from_keyring() -> bool {
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread().enable_all().build() else {
        return false;
    };
    match runtime.block_on(immich_mount_linux::keyring::remove_all()) {
        Ok(()) => true,
        Err(immich_mount_linux::keyring::KeyringError::Unavailable(_)) => false,
        Err(error) => {
            crate::ui::warn(&format!("could not remove the keyring item: {error}"));
            false
        }
    }
}
