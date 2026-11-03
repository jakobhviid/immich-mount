//! The daemon's D-Bus interface (SPEC.md, "The daemon's D-Bus interface"): `dev.hviid.ImmichMount.Daemon1` at
//! `/dev/hviid/ImmichMount/Daemon` on the session bus, owned as `dev.hviid.ImmichMount.Daemon`. The app reads the
//! daemon's state from it and asks it to refresh, clear its cache or unmount. The credential never crosses it.
//!
//! The daemon lives while it is mounted: unmounting (here or by eject) stops it, and the user service or the app
//! starts it again (ARCHITECTURE.md, "The daemon's lifecycle").

use std::sync::{Arc, Mutex};

use zbus::object_server::SignalEmitter;

pub const BUS_NAME: &str = "dev.hviid.ImmichMount.Daemon";
pub const OBJECT_PATH: &str = "/dev/hviid/ImmichMount/Daemon";
const INTERFACE: &str = "dev.hviid.ImmichMount.Daemon1";

/// The files the running daemon is copying in or uploading (name, what, size); empty when no daemon answers.
pub async fn uploads_in_flight() -> Vec<(String, String, u64)> {
    let Ok(connection) = zbus::Connection::session().await else { return Vec::new() };
    let Ok(proxy) = zbus::Proxy::new(&connection, BUS_NAME, OBJECT_PATH, INTERFACE).await else { return Vec::new() };
    proxy.get_property("Uploads").await.unwrap_or_default()
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

/// What the daemon can do on request; the daemon wires each to its tree, cache and mount.
pub struct Actions {
    /// Bytes the content cache holds, and its limit.
    pub cache_usage: Box<dyn Fn() -> (u64, u64) + Send + Sync>,
    /// Uploads in flight: name, phase, bytes.
    pub uploads: Box<dyn Fn() -> Vec<(String, String, u64)> + Send + Sync>,
    /// Forget every listing, so the next visit reads Immich again.
    pub refresh: Box<dyn Fn() + Send + Sync>,
    /// Empty the content cache, except what is being read.
    pub clear_cache: Box<dyn Fn() + Send + Sync>,
    /// Unmount, which stops the daemon.
    pub unmount: Box<dyn Fn() -> Result<(), String> + Send + Sync>,
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
