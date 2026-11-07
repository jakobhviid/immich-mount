//! The commands that talk to the server: signing in, status, and the daemon itself.

use std::io::{BufRead, IsTerminal, Write};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use immich_api::{Client, TimelineScope};
use immich_mount_core::immich::ImmichSource;
use immich_mount_core::settings::Config;
use immich_mount_core::sign_in::{self, BrowserSignIn};
use immich_mount_linux::credentials::{read_credential_file, write_credential_file, Credential};
use immich_mount_linux::keyring::{self, KeyringError};
use immich_mount_linux::daemon::{self, StopReason};
use immich_mount_linux::mounts::{mount_state, MountState};
use immich_mount_linux::paths::Paths;
use serde::Serialize;

use crate::commands::{configure_set, load_config};

const USER_AGENT: &str = concat!("immich-mount/", env!("CARGO_PKG_VERSION"));

/// How `login` signs in.
#[derive(Debug, Clone, Copy)]
pub enum LoginMethod {
    /// The server's OIDC sign-in, in the user's own browser.
    Browser,
    ApiKey,
    Password,
}

/// Where a credential is kept (SPEC.md, "Signing in and credentials").
enum CredentialStore {
    Keyring,
    File(PathBuf),
}

fn configured_file(paths: &Paths, config: &Config) -> Option<PathBuf> {
    config.credential_file.as_deref().map(|file| paths.expand(file))
}

/// Whether this runs inside a desktop session, where a keyring can show its unlock prompt.
fn in_graphical_session() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some() || std::env::var_os("DISPLAY").is_some()
}

/// Where a new credential goes: a configured file always; else the keyring in a desktop session; else the 0600 file.
fn store_for_new_credential(paths: &Paths, config: &Config) -> CredentialStore {
    match configured_file(paths, config) {
        Some(file) => CredentialStore::File(file),
        None if in_graphical_session() => CredentialStore::Keyring,
        None => CredentialStore::File(paths.default_credential_file()),
    }
}

/// The stored credential: from a configured file; else from the default file if a headless sign-in left one; else
/// from the keyring, which may bring up its unlock prompt.
fn load_credential(paths: &Paths, config: &Config, server_url: &str, runtime: &tokio::runtime::Runtime) -> Result<Credential> {
    if let Some(file) = configured_file(paths, config) {
        return Ok(read_credential_file(&file)?);
    }
    let default_file = paths.default_credential_file();
    if default_file.exists() {
        return Ok(read_credential_file(&default_file)?);
    }
    match runtime.block_on(keyring::load(server_url)) {
        Ok(Some(credential)) => Ok(credential),
        Ok(None) => bail!("not signed in to {server_url}; sign in with `immich-mount login`"),
        Err(KeyringError::Unavailable(reason)) => {
            bail!("not signed in: no credential file, and no keyring ({reason}); sign in with `immich-mount login`")
        }
        Err(error) => Err(error).context("reading the credential from the keyring"),
    }
}

fn configured_server(config: &Config) -> Result<String> {
    config
        .server_url
        .clone()
        .filter(|url| !url.trim().is_empty())
        .context("no server is configured; sign in with `immich-mount login --server URL`")
}

fn client(server_url: &str, credential: Option<&Credential>) -> Result<Client> {
    let credential = credential.map(|credential| match credential {
        Credential::ApiKey(key) => immich_api::Credential::ApiKey(key.clone()),
        Credential::Session(token) => immich_api::Credential::Session(token.clone()),
    });
    Client::new(server_url, credential, USER_AGENT).with_context(|| format!("using the server URL {server_url}"))
}

fn runtime() -> Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_multi_thread().enable_all().build().context("starting the async runtime")
}

/// The name of an account's cache directory: the server's host and port, so it reads well in a file listing.
/// Signing out removes every cache. Signing in as another account without signing out shares the directory: Asset ids
/// are global, so cached bytes stay right, but kept listings show the earlier account's folders until refreshed (SPEC.md,
/// "Files on disk").
fn account_name(client: &Client) -> String {
    let url = client.api_url();
    let host = url.host_str().unwrap_or("server");
    let name = match url.port_or_known_default() {
        Some(port) => format!("{host}-{port}"),
        None => host.to_owned(),
    };
    name.chars().map(|character| if character.is_ascii_alphanumeric() || ".-".contains(character) { character } else { '_' }).collect()
}

/// A line from stdin, prompting only on a terminal.
fn read_line(prompt: &str) -> Result<String> {
    if std::io::stdin().is_terminal() {
        eprint!("{prompt}");
        std::io::stderr().flush().ok();
    }
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line).context("reading stdin")?;
    Ok(line.trim().to_owned())
}

/// A secret from stdin: without echo on a terminal, else the first line.
fn read_secret(prompt: &str) -> Result<String> {
    let secret = if std::io::stdin().is_terminal() {
        rpassword::prompt_password(prompt).context("reading from the terminal")?
    } else {
        read_line(prompt)?
    };
    let secret = secret.trim().to_owned();
    if secret.is_empty() {
        bail!("nothing was entered");
    }
    Ok(secret)
}

#[derive(Serialize)]
pub struct LoginReport {
    pub server: String,
    pub server_version: String,
    pub method: &'static str,
    pub user: Option<String>,
    /// `the keyring`, or the credential file's path.
    pub stored_in: String,
}

/// Sign in, check the credential against the server, and store it with the server's URL.
pub fn login(paths: &Paths, method: LoginMethod, server: Option<String>) -> Result<LoginReport> {
    let config = load_config(paths)?;
    let server_url = match server {
        Some(server) => server,
        None => configured_server(&config)?,
    };
    let runtime = runtime()?;
    let anonymous = client(&server_url, None)?;
    let version = runtime.block_on(anonymous.server_version()).with_context(|| format!("reaching {server_url}"))?;
    if !version.is_supported() {
        bail!("{server_url} runs Immich {version}; Immich Mount needs {} or later", immich_api::version::MINIMUM_SUPPORTED);
    }

    let (credential, user) = match method {
        LoginMethod::ApiKey => {
            let credential = Credential::ApiKey(read_secret("API key: ")?);
            runtime
                .block_on(client(&server_url, Some(&credential))?.api_key_permissions())
                .context("the server did not accept the API key")?;
            (credential, None)
        }
        LoginMethod::Browser => {
            let features = runtime.block_on(anonymous.server_features()).context("asking the server how it signs in")?;
            if !features.oauth {
                bail!("{server_url} has no OIDC sign-in; use --password or --api-key");
            }
            let attempt = BrowserSignIn::new().context("making the sign-in secrets")?;
            let address = runtime
                .block_on(anonymous.oauth_authorize(sign_in::REDIRECT_URI, &attempt.state, &attempt.code_challenge))
                .context("starting the browser sign-in")?;
            eprintln!("Open this address in your browser and sign in:\n\n  {address}\n");
            eprintln!("The browser then ends on an address starting with app.immich:// (it may show an error page).");
            let redirect = read_line("Copy that whole address and paste it here: ")?;
            if !sign_in::is_redirect_for(&redirect, &attempt.state) {
                bail!("that is not the address this sign-in ends on; run `immich-mount login` again");
            }
            let answer = runtime
                .block_on(anonymous.oauth_callback(&redirect, &attempt.state, &attempt.code_verifier))
                .context("finishing the browser sign-in")?;
            (Credential::Session(answer.access_token), Some(answer.user_email))
        }
        LoginMethod::Password => {
            if server_url.trim_start().to_ascii_lowercase().starts_with("http://") {
                crate::ui::warn(&format!("{server_url} is plain http: the password crosses the network unencrypted"));
            }
            let email = read_line("Email: ")?;
            let password = read_secret("Password: ")?;
            let answer = runtime.block_on(anonymous.login_password(&email, &password)).context("signing in")?;
            (Credential::Session(answer.access_token), Some(answer.user_email))
        }
    };

    let stored_in = match store_for_new_credential(paths, &config) {
        CredentialStore::File(file) => {
            write_credential_file(&file, &credential)?;
            file.display().to_string()
        }
        CredentialStore::Keyring => match runtime.block_on(keyring::store(&server_url, &credential)) {
            Ok(()) => {
                // A credential file from an earlier headless sign-in would be read first; this one replaces it.
                let _ = std::fs::remove_file(paths.default_credential_file());
                "the keyring".to_owned()
            }
            Err(KeyringError::Unavailable(reason)) => {
                crate::ui::warn(&format!("no keyring ({reason}); keeping the credential in a file readable only by you"));
                let file = paths.default_credential_file();
                write_credential_file(&file, &credential)?;
                file.display().to_string()
            }
            Err(error) => return Err(error).context("storing the credential in the keyring"),
        },
    };
    configure_set(paths, "server.url", &server_url)?;
    Ok(LoginReport { server: server_url, server_version: version.to_string(), method: credential.kind(), user, stored_in })
}

/// The shape WORKFLOWS.md documents for `immich-mount status --json`.
#[derive(Serialize)]
pub struct StatusReport {
    /// `unconfigured`, `online`, `offline`, `signed-out`, `missing-permission` or `unmounted`.
    pub state: &'static str,
    /// Why, when the state is not `online`.
    pub detail: Option<String>,
    pub server: Option<ServerStatus>,
    pub account: AccountStatus,
    /// How the daemon learns about changes: `sync-stream` with a session, `polling` with an API key.
    pub change_feed: &'static str,
    pub mount: MountStatus,
    pub cache: CacheStatus,
    /// Originals fetched from the server since the mount started, from the running daemon.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fetched: Option<FetchedStatus>,
}

#[derive(Serialize)]
pub struct FetchedStatus {
    pub bytes: u64,
    pub requests: u64,
}

#[derive(Serialize)]
pub struct ServerStatus {
    pub url: String,
    pub version: Option<String>,
}

#[derive(Serialize)]
pub struct AccountStatus {
    pub email: Option<String>,
    /// `session` or `api-key`, when signed in.
    pub credential: Option<&'static str>,
}

#[derive(Serialize)]
pub struct MountStatus {
    pub path: PathBuf,
    pub mounted: bool,
    /// The mount's process is gone and the mount must be detached (`immich-mount unmount`).
    pub stale: bool,
}

#[derive(Serialize)]
pub struct CacheStatus {
    pub used_bytes: u64,
    pub limit_bytes: u64,
}

/// How long the daemon's start and `status` wait for the server's version (ARCHITECTURE.md, "The daemon's lifecycle").
const VERSION_WAIT: std::time::Duration = std::time::Duration::from_secs(10);

/// The server's version, or an error once `VERSION_WAIT` has passed without one.
async fn server_version_soon(client: &Client) -> Result<immich_api::version::ServerVersion, immich_api::ApiError> {
    match tokio::time::timeout(VERSION_WAIT, client.server_version()).await {
        Ok(outcome) => outcome,
        Err(_) => Err(immich_api::ApiError::Offline(format!("no answer within {} s", VERSION_WAIT.as_secs()))),
    }
}

/// What the daemon would see: the server, the credential, the mount and the cache.
pub fn status(paths: &Paths) -> Result<StatusReport> {
    let config = load_config(paths)?;
    let path = paths.expand(&config.mount_path);
    let state_of_mount = mount_state(&path);
    let mut report = StatusReport {
        state: "unconfigured",
        detail: None,
        server: config.server_url.clone().filter(|url| !url.trim().is_empty()).map(|url| ServerStatus { url, version: None }),
        account: AccountStatus { email: None, credential: None },
        change_feed: "polling",
        // Set below once the credential is read: a session also follows the sync stream.
        mount: MountStatus { path, mounted: state_of_mount == MountState::Mounted, stale: state_of_mount == MountState::Stale },
        cache: CacheStatus { used_bytes: disk_usage(&paths.cache_directory), limit_bytes: config.cache_limit_bytes },
        fetched: runtime()
            .ok()
            .and_then(|runtime| runtime.block_on(immich_mount_linux::dbus::fetched()))
            .map(|(bytes, requests)| FetchedStatus { bytes, requests }),
    };
    let Some(server_url) = report.server.as_ref().map(|server| server.url.clone()) else {
        return Ok(report);
    };
    let runtime = runtime()?;
    let credential = match load_credential(paths, &config, &server_url, &runtime) {
        Ok(credential) => credential,
        Err(error) => {
            report.state = "signed-out";
            report.detail = Some(format!("{error:#}"));
            return Ok(report);
        }
    };
    report.account.credential = Some(credential.kind());
    if matches!(credential, Credential::Session(_)) {
        report.change_feed = "sync-stream";
    }
    let client = client(&server_url, Some(&credential))?;
    match runtime.block_on(server_version_soon(&client)) {
        Ok(version) => {
            if let Some(server) = report.server.as_mut() {
                server.version = Some(version.to_string());
            }
            report.account.email = runtime.block_on(client.my_email()).ok();
        }
        Err(error) => {
            report.state = "offline";
            report.detail = Some(error.to_string());
            return Ok(report);
        }
    }
    // The timeline is what the mount reads first, so it is the honest probe of the credential.
    report.state = match runtime.block_on(client.time_buckets(&TimelineScope::Library)) {
        Ok(_) if report.mount.mounted => "online",
        Ok(_) => "unmounted",
        Err(immich_api::ApiError::Unauthorized) => "signed-out",
        Err(immich_api::ApiError::Forbidden { missing_permission }) => {
            report.detail = missing_permission.map(|permission| format!("the credential lacks `{permission}`"));
            "missing-permission"
        }
        Err(error) => {
            report.detail = Some(error.to_string());
            "offline"
        }
    };
    Ok(report)
}

/// End the session on the server, so the device leaves Immich's "Authorized devices". Best effort: signing out
/// locally never depends on reaching the server. Returns whether a session was ended.
pub fn revoke_session(paths: &Paths) -> bool {
    let Ok(config) = load_config(paths) else { return false };
    let Ok(server_url) = configured_server(&config) else { return false };
    let Ok(runtime) = runtime() else { return false };
    let Ok(credential @ Credential::Session(_)) = load_credential(paths, &config, &server_url, &runtime) else { return false };
    let Ok(client) = client(&server_url, Some(&credential)) else { return false };
    // The timeout must be created inside the runtime, which owns the timer.
    let ended = runtime.block_on(async { tokio::time::timeout(std::time::Duration::from_secs(5), client.logout()).await });
    matches!(ended, Ok(Ok(())))
}

/// Bytes actually used under `directory`. Content cache files are sparse, so this counts blocks, not lengths.
fn disk_usage(directory: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| match entry.metadata() {
            Ok(metadata) if metadata.is_dir() => disk_usage(&entry.path()),
            Ok(metadata) => metadata.blocks() * 512,
            Err(_) => 0,
        })
        .sum()
}

/// Run the daemon in the foreground until it is stopped or the mount is ejected. `None` when it never started because
/// the user dismissed the keyring's unlock prompt: that ends the run cleanly, so the service is not restarted into
/// prompting again.
pub fn daemon(paths: &Paths) -> Result<Option<StopReason>> {
    let config = load_config(paths)?;
    let server_url = configured_server(&config)?;
    let runtime = runtime()?;
    let credential = match load_credential(paths, &config, &server_url, &runtime) {
        Ok(credential) => credential,
        Err(error) if matches!(error.downcast_ref::<KeyringError>(), Some(KeyringError::Dismissed)) => {
            log::error!("the keyring stayed locked, so Immich is not mounted; run `systemctl --user start immich-mount` after unlocking it");
            runtime.block_on(immich_mount_linux::notify::notify(
                "Immich is not mounted",
                "The keyring stayed locked. Open Immich Mount, or run “immich-mount mount”, to try again.",
            ));
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    let client = client(&server_url, Some(&credential))?;
    // The mount comes first: a server that drops requests would otherwise hold it back for the client's retries (four
    // minutes, seen on Bazzite), with the mount point an empty folder all along.
    let version = match runtime.block_on(server_version_soon(&client)) {
        Ok(version) if !version.is_supported() => {
            bail!("{server_url} runs Immich {version}; Immich Mount needs {} or later", immich_api::version::MINIMUM_SUPPORTED)
        }
        Ok(version) => {
            log::info!("Immich {version} at {server_url}");
            Some(version.to_string())
        }
        Err(error) => {
            log::warn!("cannot reach {server_url} yet ({error}); mounting anyway");
            None
        }
    };
    let mount_point = paths.expand(&config.mount_path);
    // GVfs lists no mount in a home reached through a symlink until its volume monitor has the resolved home.
    match immich_mount_linux::gnome::show_mounts_in_sidebar(paths, &mount_point) {
        Ok(true) => log::info!("gave GVfs's volume monitor the resolved home, so Files lists the mount as a drive"),
        Ok(false) => {}
        Err(error) => log::warn!("could not set GVfs up for this home, so Files may not list the mount: {error}"),
    }
    // The drive's name, in the sidebar entries the apps' setup made, after it changed (`mount.name`).
    let retitled = [
        immich_mount_linux::gnome::retitle_bookmark(paths, &mount_point, config.mount_name),
        immich_mount_linux::kde::retitle_place(paths, &mount_point, config.mount_name),
    ];
    for result in retitled {
        match result {
            Ok(true) => log::info!("named the mount's sidebar entry \"{}\"", config.mount_name),
            Ok(false) => {}
            Err(error) => log::warn!("could not name the mount's sidebar entry \"{}\": {error}", config.mount_name),
        }
    }
    let account_cache = paths.account_cache(&account_name(&client));
    // Shown by the apps; an API key without `user.read` is simply not named.
    let account = version.is_some().then(|| runtime.block_on(client.my_email()).ok()).flatten();
    let source = Arc::new(ImmichSource::new(client));
    let server = daemon::ServerInfo { url: server_url, version, account };
    match daemon::serve(source, server, config, &mount_point, &account_cache, &paths.thumbnail_directory, &paths.not_uploaded_directory(), &runtime) {
        // Another daemon serves it (the packages' service and Homebrew's both on): this one leaves cleanly, so its
        // service is not restarted every few seconds.
        Err(daemon::DaemonError::AlreadyMounted(path)) => {
            log::warn!("{} is already mounted by another immich-mount daemon; this one stops", path.display());
            Ok(None)
        }
        // Its program is gone: turn off the service that started it, so it does not fail at every login, and take
        // away what points at a mount that will not come back (the credential and the caches stay for `logout`).
        Ok(StopReason::Uninstalled) => {
            if let Some(unit) = immich_mount_linux::service::own_unit() {
                let _ = std::process::Command::new("systemctl").args(["--user", "disable", &unit]).output();
            }
            if let Err(error) = crate::commands::remove_desktop_traces(paths, &mount_point) {
                log::warn!("could not take the mount out of the desktop: {error:#}");
            }
            Ok(Some(StopReason::Uninstalled))
        }
        outcome => Ok(Some(outcome?)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_names_are_the_servers_host_and_port() {
        let named = |url: &str| account_name(&client(url, None).unwrap());
        assert_eq!(named("http://192.168.1.20:2283"), "192.168.1.20-2283");
        assert_eq!(named("https://photos.example.org"), "photos.example.org-443");
    }
}
