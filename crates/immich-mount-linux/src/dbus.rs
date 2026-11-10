//! The daemon's D-Bus interface (SPEC.md, "The daemon's D-Bus interface"): `dev.hviid.ImmichMount.Daemon1` at
//! `/dev/hviid/ImmichMount/Daemon` on the session bus, owned as `dev.hviid.ImmichMount.Daemon`. The app reads the
//! daemon's state from it and asks it to refresh, clear its cache or unmount. The credential never crosses it.
//!
//! The daemon lives while it is mounted: unmounting (here or by eject) stops it, and the user service or the app
//! starts it again (ARCHITECTURE.md, "The daemon's lifecycle").

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use zbus::object_server::SignalEmitter;

pub const BUS_NAME: &str = "dev.hviid.ImmichMount.Daemon";
pub const OBJECT_PATH: &str = "/dev/hviid/ImmichMount/Daemon";
const INTERFACE: &str = "dev.hviid.ImmichMount.Daemon1";

/// Bytes of originals the running daemon fetched since it mounted, and in how many requests; `None` when no daemon
/// answers.
pub async fn fetched() -> Option<(u64, u64)> {
    let connection = zbus::Connection::session().await.ok()?;
    let proxy = zbus::Proxy::new(&connection, BUS_NAME, OBJECT_PATH, INTERFACE).await.ok()?;
    proxy.get_property("Fetched").await.ok()
}

/// The files the running daemon is copying in or uploading (name, what, size); empty when no daemon answers.
pub async fn uploads_in_flight() -> Vec<(String, String, u64)> {
    let Ok(connection) = zbus::Connection::session().await else { return Vec::new() };
    let Ok(proxy) = zbus::Proxy::new(&connection, BUS_NAME, OBJECT_PATH, INTERFACE).await else { return Vec::new() };
    proxy.get_property("Uploads").await.unwrap_or_default()
}

/// The running daemon's web address for the item at `path`, empty when it has none; `None` when no daemon answers.
pub async fn web_link(path: &str) -> Option<String> {
    let connection = zbus::Connection::session().await.ok()?;
    let proxy = zbus::Proxy::new(&connection, BUS_NAME, OBJECT_PATH, INTERFACE).await.ok()?;
    proxy.call("WebLink", &(path,)).await.ok()
}

/// What the running daemon makes of a received link (`ShowLink`); `None` when no daemon answers.
pub async fn show_link(link: &str, select: bool, notify: bool) -> Option<(String, String)> {
    let connection = zbus::Connection::session().await.ok()?;
    let proxy = zbus::Proxy::new(&connection, BUS_NAME, OBJECT_PATH, INTERFACE).await.ok()?;
    proxy.call("ShowLink", &(link, select, notify)).await.ok()
}

/// Select `path` in the file manager, through the freedesktop interface Files and Dolphin both answer.
pub async fn show_in_file_manager(path: &std::path::Path) -> Result<(), String> {
    let connection = zbus::Connection::session().await.map_err(|error| error.to_string())?;
    let proxy = zbus::Proxy::new(&connection, "org.freedesktop.FileManager1", "/org/freedesktop/FileManager1", "org.freedesktop.FileManager1")
        .await
        .map_err(|error| error.to_string())?;
    let uri = file_uri(path).ok_or("the path is not UTF-8")?;
    proxy.call::<_, _, ()>("ShowItems", &(vec![uri], "")).await.map_err(|error| error.to_string())
}

/// A local path as a `file://` URI, each name encoded.
fn file_uri(path: &std::path::Path) -> Option<String> {
    let text = path.to_str()?;
    Some(format!("file://{}", text.split('/').map(immich_mount_core::links::encode_component).collect::<Vec<_>>().join("/")))
}

/// The connection state, as SPEC names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Connecting,
    Online,
    Offline,
    SignedOut,
}

impl State {
    pub fn as_str(self) -> &'static str {
        match self {
            State::Connecting => "connecting",
            State::Online => "online",
            State::Offline => "offline",
            State::SignedOut => "signed-out",
        }
    }
}

/// The answer to a web link request, which may list a folder to find the item.
pub type WebLinkFuture = Pin<Box<dyn Future<Output = Option<String>> + Send>>;

/// The answer to a received link: its result, and the path shown (or the server's host, for another server).
pub type ShowLinkFuture = Pin<Box<dyn Future<Output = (String, String)> + Send>>;

/// An item's links for the menus: its web address, its crosslink (empty while Immich crosslink is off), its kind, and
/// the apps left out of Show in; `None` for a path outside the mount or an item Immich has no page for.
pub type LinksFuture = Pin<Box<dyn Future<Output = Option<(String, String, String, String)>> + Send>>;

/// What the daemon can do on request; the daemon wires each to its tree, cache and mount.
pub struct Actions {
    /// Bytes the content cache holds, and its limit.
    pub cache_usage: Box<dyn Fn() -> (u64, u64) + Send + Sync>,
    /// Bytes of originals fetched since the mount started, and in how many requests.
    pub fetched: Box<dyn Fn() -> (u64, u64) + Send + Sync>,
    /// Uploads in flight: name, phase, bytes.
    pub uploads: Box<dyn Fn() -> Vec<(String, String, u64)> + Send + Sync>,
    /// Forget every listing, so the next visit reads Immich again.
    pub refresh: Box<dyn Fn() + Send + Sync>,
    /// Empty the content cache, except what is being read.
    pub clear_cache: Box<dyn Fn() + Send + Sync>,
    /// Unmount, which stops the daemon.
    pub unmount: Box<dyn Fn() -> Result<(), String> + Send + Sync>,
    /// The web address in Immich of the item at a path, `None` for a path outside the mount or an item Immich has no
    /// page for (ROADMAP.md, "Open in Immich").
    pub web_link: Box<dyn Fn(std::path::PathBuf) -> WebLinkFuture + Send + Sync>,
    /// Find a received link's item in the mount, and with `select` show it in the file manager or say why not
    /// (ROADMAP.md, "Show in Immich Mount").
    pub show_link: Box<dyn Fn(String, bool, bool) -> ShowLinkFuture + Send + Sync>,
    /// An item's links for Show in *app* (ROADMAP.md, "Show in other apps").
    pub links: Box<dyn Fn(std::path::PathBuf) -> LinksFuture + Send + Sync>,
}

/// The served object.
pub struct Daemon {
    state: Arc<Mutex<State>>,
    mount_point: String,
    server_url: String,
    server_version: String,
    account: String,
    actions: Actions,
}

#[zbus::interface(name = "dev.hviid.ImmichMount.Daemon1")]
impl Daemon {
    #[zbus(property)]
    fn state(&self) -> String {
        self.state.lock().expect("state lock").as_str().to_owned()
    }

    #[zbus(property)]
    fn mount_point(&self) -> String {
        self.mount_point.clone()
    }

    #[zbus(property)]
    fn server_url(&self) -> String {
        self.server_url.clone()
    }

    #[zbus(property)]
    fn server_version(&self) -> String {
        self.server_version.clone()
    }

    /// The signed-in account's email; empty when the credential may not read it.
    #[zbus(property)]
    fn account(&self) -> String {
        self.account.clone()
    }

    #[zbus(property)]
    fn cache_usage(&self) -> (u64, u64) {
        (self.actions.cache_usage)()
    }

    #[zbus(property)]
    fn fetched(&self) -> (u64, u64) {
        (self.actions.fetched)()
    }

    #[zbus(property)]
    fn uploads(&self) -> Vec<(String, String, u64)> {
        (self.actions.uploads)()
    }

    /// Read every folder from Immich again on its next visit. `path` is accepted for the interface's sake; the whole
    /// tree is refreshed, which costs one listing per folder visited.
    fn refresh(&self, _path: String) {
        (self.actions.refresh)();
    }

    fn clear_cache(&self) {
        (self.actions.clear_cache)();
    }

    fn unmount(&self) -> zbus::fdo::Result<()> {
        (self.actions.unmount)().map_err(zbus::fdo::Error::Failed)
    }

    /// The web address in Immich of the item at `path`; empty when the path is outside the mount or Immich has no page
    /// for the item. The menu items in Files and Dolphin ask this.
    async fn web_link(&self, path: String) -> String {
        (self.actions.web_link)(path.into()).await.unwrap_or_default()
    }

    /// Show the item a crosslink or an Immich web address names: `(result, detail)`, the result `show` with the
    /// item's path, or why not (`invalid`, `cannot-show`, `other-server` with its host, `other-user`, `not-found`,
    /// `off`, `unreachable`). With `select`, the file manager shows the item selected; with `notify`, a notification
    /// says why not, for a link opened from another app.
    async fn show_link(&self, link: String, select: bool, notify: bool) -> (String, String) {
        (self.actions.show_link)(link, select, notify).await
    }

    /// The links of the item at `path`, for the menus' Show in *app*: `(web address, crosslink, kind, hidden apps)`,
    /// the crosslink empty while Immich crosslink is off and the hidden apps separated by `;`; all empty for a path
    /// outside the mount or an item Immich has no page for.
    async fn links(&self, path: String) -> (String, String, String, String) {
        (self.actions.links)(path.into()).await.unwrap_or_default()
    }

    /// `StateChanged(state, message)`; named apart in Rust from the `State` property's own change signal.
    #[zbus(signal, name = "StateChanged")]
    async fn state_signal(emitter: &SignalEmitter<'_>, state: &str, message: &str) -> zbus::Result<()>;
}

/// Serve the interface for as long as the returned connection lives. `None` without a session bus (a headless
/// machine), or when another daemon owns the name.
pub async fn serve(
    state: Arc<Mutex<State>>,
    mount_point: &str,
    server_url: &str,
    server_version: &str,
    account: &str,
    actions: Actions,
) -> Option<zbus::Connection> {
    let daemon = Daemon {
        state,
        mount_point: mount_point.to_owned(),
        server_url: server_url.to_owned(),
        server_version: server_version.to_owned(),
        account: account.to_owned(),
        actions,
    };
    let built = async { zbus::connection::Builder::session()?.name(BUS_NAME)?.serve_at(OBJECT_PATH, daemon)?.build().await };
    match built.await {
        Ok(connection) => Some(connection),
        Err(error) => {
            log::info!("no D-Bus interface ({error}); the app will not see this daemon's state");
            None
        }
    }
}

/// Tell listeners the state changed: the property's change signal and `StateChanged` with a message.
pub async fn announce(connection: &zbus::Connection, state: State, message: &str) {
    let Ok(interface) = connection.object_server().interface::<_, Daemon>(OBJECT_PATH).await else { return };
    let emitter = interface.signal_emitter();
    let _ = interface.get().await.state_changed(emitter).await;
    let _ = Daemon::state_signal(emitter, state.as_str(), message).await;
}
