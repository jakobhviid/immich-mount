//! The tray icon (ARCHITECTURE.md, "The tray"): the Mac's menu bar item on Linux, served by the daemon itself, as a
//! StatusNotifierItem with a `com.canonical.dbusmenu` menu. Both are D-Bus alone, so the static daemon needs no
//! toolkit. KDE Plasma shows it in its system tray; GNOME shows it where the AppIndicator extension is on (Ubuntu,
//! Bazzite), and nowhere on stock GNOME, where the app and Files' sidebar stay the way in.
//!
//! The menu is the Mac's: the state on its first line, then grey lines saying what each is, then the way to the
//! drive, the settings and the app, and at the foot Sign Out… and Quit, which unmounts, as the tray menus of KDE and
//! the apps that use GNOME's AppIndicator end.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use zbus::object_server::SignalEmitter;
use zbus::zvariant::{ObjectPath, OwnedValue, Value};

use crate::dbus::State;
use crate::i18n::tr;

const ITEM_PATH: &str = "/StatusNotifierItem";
const MENU_PATH: &str = "/MenuBar";
const WATCHER: &str = "org.kde.StatusNotifierWatcher";
/// A menu item's properties, by dbusmenu's names (`label`, `enabled`, `type`).
type Properties = HashMap<String, OwnedValue>;
/// One item and its children, dbusmenu's `(ia{sv}av)`.
type Layout = (i32, Properties, Vec<OwnedValue>);
/// Icons as pixels, which the item never sends (it names a themed icon): StatusNotifierItem's `a(iiay)`.
type Pixmaps = Vec<(i32, i32, Vec<u8>)>;

/// The symbolic icon the `immich-mount` package installs (DESIGN.md, "The icon family"); themes recolour it.
const ICON: &str = "dev.hviid.ImmichMount-symbolic";

/// What the tray shows: the daemon's view of the server, kept up to date as the state changes.
#[derive(Debug, Clone)]
pub struct View {
    pub state: State,
    pub server_url: String,
    pub server_version: String,
    pub account: String,
    pub mount_point: String,
    /// Dolphin's name for the file manager on KDE Plasma, Files' elsewhere.
    pub on_kde: bool,
    /// A newer release, once the daemon has seen one (`crate::updates`).
    pub update: Option<crate::updates::Release>,
}

/// One line of the menu: its label, and what clicking it does (`None` for a grey line).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Line {
    Text(String),
    Separator,
    Action(String, Action),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    OpenDrive,
    OpenSettings,
    OpenApp,
    OpenUpdate,
    SignOut,
    Quit,
}

/// What `immich-mount app` is given to open the app on its first settings page, General: an address no other program
/// claims, which the GNOME app receives as a file to open (in the window already running, if one is) and the KDE app
/// as an argument.
pub const OPEN_SETTINGS: &str = "immich-mount:settings";
/// What `immich-mount app` is given to open the app on its Account page, asking whether to sign out: the daemon has
/// no window to ask in, and signing out is never done without asking.
pub const OPEN_SIGN_OUT: &str = "immich-mount:sign-out";

impl View {
    fn host(&self) -> String {
        url_host(&self.server_url)
    }

    /// The first line, as the Mac's menu has it.
    pub fn status_line(&self) -> String {
        match self.state {
            State::Online => tr("Connected to {server}").replace("{server}", &self.host()),
            State::Offline => tr("{server} is not reachable").replace("{server}", &self.host()),
            State::SignedOut => tr("Signed out of Immich").to_owned(),
            State::Connecting => tr("Signed in to {server}").replace("{server}", &self.host()),
        }
    }

    /// Every line of the menu, in order.
    pub fn lines(&self) -> Vec<Line> {
        let mut lines = vec![Line::Text(self.status_line())];
        if !self.server_version.is_empty() {
            lines.push(Line::Text(tr("Server version: Immich {version}").replace("{version}", &self.server_version)));
        }
        if !self.account.is_empty() {
            lines.push(Line::Text(tr("Signed in as: {account}").replace("{account}", &self.account)));
        }
        lines.push(Line::Separator);
        let open = if self.on_kde { tr("Open in Dolphin") } else { tr("Open in Files") };
        lines.push(Line::Action(open.to_owned(), Action::OpenDrive));
        lines.push(Line::Action(tr("Settings…").to_owned(), Action::OpenSettings));
        lines.push(Line::Action(tr("Open Immich Mount").to_owned(), Action::OpenApp));
        if let Some(release) = &self.update {
            lines.push(Line::Separator);
            lines.push(Line::Action(tr("Get Immich Mount {version}…").replace("{version}", &release.version), Action::OpenUpdate));
        }
        lines.push(Line::Separator);
        lines.push(Line::Action(tr("Sign Out…").to_owned(), Action::SignOut));
        lines.push(Line::Action(tr("Quit Immich Mount").to_owned(), Action::Quit));
        lines
    }
}

/// `photos.example.org` from `https://photos.example.org:443/`, as the Mac's menu names the server.
fn url_host(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let host = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    host.rsplit_once(':').filter(|(_, port)| port.chars().all(|c| c.is_ascii_digit())).map_or(host, |(host, _)| host).to_owned()
}

/// The menu's layout as dbusmenu wants it: the root (id 0) with one child per line, ids from 1 in order.
fn layout(lines: &[Line]) -> Layout {
    let children = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| {
            let item: Layout = (index as i32 + 1, properties(line), Vec::new());
            OwnedValue::try_from(Value::from(item)).ok()
        })
        .collect();
    let mut root = HashMap::new();
    root.insert("children-display".to_owned(), owned("submenu"));
    (0, root, children)
}

fn properties(line: &Line) -> Properties {
    let mut properties = HashMap::new();
    match line {
        Line::Text(label) => {
            properties.insert("label".to_owned(), owned(label.as_str()));
            properties.insert("enabled".to_owned(), OwnedValue::from(false));
        }
        Line::Separator => {
            properties.insert("type".to_owned(), owned("separator"));
        }
        Line::Action(label, _) => {
            properties.insert("label".to_owned(), owned(label.as_str()));
        }
    }
    properties
}

fn owned(text: &str) -> OwnedValue {
    OwnedValue::try_from(Value::from(text)).expect("a string is a value")
}

struct Shared {
    view: Mutex<View>,
    revision: Mutex<u32>,
}

impl Shared {
    fn lines(&self) -> Vec<Line> {
        self.view.lock().expect("tray view lock").lines()
    }
}

/// `org.kde.StatusNotifierItem`: the icon itself.
struct Item {
    shared: Arc<Shared>,
}

#[zbus::interface(name = "org.kde.StatusNotifierItem")]
impl Item {
    #[zbus(property)]
    fn category(&self) -> &str {
        "ApplicationStatus"
    }

    #[zbus(property)]
    fn id(&self) -> &str {
        "immich-mount"
    }

    #[zbus(property)]
    fn title(&self) -> String {
        "Immich Mount".to_owned()
    }

    #[zbus(property)]
    fn status(&self) -> &str {
        "Active"
    }

    #[zbus(property)]
    fn window_id(&self) -> i32 {
        0
    }

    #[zbus(property)]
    fn icon_name(&self) -> &str {
        ICON
    }

    #[zbus(property)]
    fn icon_pixmap(&self) -> Pixmaps {
        Vec::new()
    }

    #[zbus(property)]
    fn overlay_icon_name(&self) -> &str {
        ""
    }

    #[zbus(property)]
    fn attention_icon_name(&self) -> &str {
        ""
    }

    #[zbus(property)]
    fn tool_tip(&self) -> (String, Pixmaps, String, String) {
        let status = self.shared.view.lock().expect("tray view lock").status_line();
        (ICON.to_owned(), Vec::new(), "Immich Mount".to_owned(), status)
    }

    /// The menu is all there is, as on the Mac: a click shows it rather than calling `Activate`.
    #[zbus(property)]
    fn item_is_menu(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn menu(&self) -> ObjectPath<'_> {
        ObjectPath::from_static_str_unchecked(MENU_PATH)
    }

    fn activate(&self, _x: i32, _y: i32) {
        run(Action::OpenApp, &self.shared.view.lock().expect("tray view lock"));
    }

    fn secondary_activate(&self, _x: i32, _y: i32) {}

    fn context_menu(&self, _x: i32, _y: i32) {}

    fn scroll(&self, _delta: i32, _orientation: &str) {}

    #[zbus(signal)]
    async fn new_tool_tip(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

/// `com.canonical.dbusmenu`: the menu, flat, built anew from the view at each change.
struct Menu {
    shared: Arc<Shared>,
}

#[zbus::interface(name = "com.canonical.dbusmenu")]
impl Menu {
    #[zbus(property)]
    fn version(&self) -> u32 {
        3
    }

    #[zbus(property)]
    fn text_direction(&self) -> &str {
        "ltr"
    }

    #[zbus(property)]
    fn status(&self) -> &str {
        "normal"
    }

    #[zbus(property)]
    fn icon_theme_path(&self) -> Vec<String> {
        Vec::new()
    }

    fn get_layout(
        &self,
        _parent_id: i32,
        _recursion_depth: i32,
        _property_names: Vec<String>,
    ) -> (u32, Layout) {
        (*self.shared.revision.lock().expect("tray revision lock"), layout(&self.shared.lines()))
    }

    fn get_group_properties(&self, ids: Vec<i32>, _property_names: Vec<String>) -> Vec<(i32, Properties)> {
        let lines = self.shared.lines();
        let wanted = |id: &i32| ids.is_empty() || ids.contains(id);
        lines.iter().enumerate().map(|(index, line)| (index as i32 + 1, properties(line))).filter(|(id, _)| wanted(id)).collect()
    }

    fn get_property(&self, id: i32, name: &str) -> zbus::fdo::Result<OwnedValue> {
        let lines = self.shared.lines();
        let line = lines.get((id - 1).max(0) as usize).ok_or_else(|| zbus::fdo::Error::InvalidArgs(format!("no item {id}")))?;
        properties(line).remove(name).ok_or_else(|| zbus::fdo::Error::InvalidArgs(format!("no property {name}")))
    }

    fn event(&self, id: i32, event_id: &str, _data: OwnedValue, _timestamp: u32) {
        if event_id != "clicked" {
            return;
        }
        let view = self.shared.view.lock().expect("tray view lock").clone();
        if let Some(Line::Action(_, action)) = view.lines().get((id - 1).max(0) as usize) {
            run(*action, &view);
        }
    }

    fn event_group(&self, events: Vec<(i32, String, OwnedValue, u32)>) -> Vec<i32> {
        for (id, event_id, data, timestamp) in events {
            self.event(id, &event_id, data, timestamp);
        }
        Vec::new()
    }

    fn about_to_show(&self, _id: i32) -> bool {
        false
    }

    fn about_to_show_group(&self, _ids: Vec<i32>) -> (Vec<i32>, Vec<i32>) {
        (Vec::new(), Vec::new())
    }

    #[zbus(signal)]
    async fn layout_updated(emitter: &SignalEmitter<'_>, revision: u32, parent: i32) -> zbus::Result<()>;
}

/// Do what a line of the menu says: open the drive in the file manager, the app or its settings, or quit.
fn run(action: Action, view: &View) {
    let outcome = match action {
        Action::OpenDrive => std::process::Command::new("xdg-open").arg(&view.mount_point).spawn(),
        Action::OpenUpdate => match &view.update {
            Some(release) => std::process::Command::new("xdg-open").arg(&release.page).spawn(),
            None => return,
        },
        Action::OpenApp | Action::OpenSettings | Action::SignOut => match std::env::current_exe() {
            Ok(program) => {
                let mut command = std::process::Command::new(program);
                command.arg("app");
                match action {
                    Action::OpenSettings => _ = command.arg(OPEN_SETTINGS),
                    Action::SignOut => _ = command.arg(OPEN_SIGN_OUT),
                    _ => {}
                }
                command.spawn()
            }
            Err(error) => Err(error),
        },
        Action::Quit => return quit(),
    };
    if let Err(error) = outcome {
        log::warn!("the tray's {action:?} did not start: {error}");
    }
}

/// Quit, which on Linux is unmounting (the daemon serves the drive and the tray icon alike): `immich-mount unmount`,
/// as the app's Unmount and Files' eject, so the daemon leaves cleanly and its service is not started again until the
/// next login or Mount. It refuses while a file is uploading, and says why in a notification.
fn quit() {
    let Ok(program) = std::env::current_exe() else { return };
    std::thread::spawn(move || {
        let outcome = std::process::Command::new(program).arg("unmount").output();
        let refusal = match outcome {
            Ok(output) if output.status.success() => return,
            Ok(output) => String::from_utf8_lossy(&output.stderr).trim().trim_start_matches("error:").trim().to_owned(),
            Err(error) => error.to_string(),
        };
        log::warn!("the tray's Quit did not unmount: {refusal}");
        if let Ok(runtime) = tokio::runtime::Builder::new_current_thread().enable_all().build() {
            runtime.block_on(crate::notify::notify("Immich Mount", &refusal));
        }
    });
}

/// Show the tray icon on `connection`, and keep it registered with the desktop's tray, which may start after the
/// daemon or start again. Nothing shows where no tray watches (stock GNOME).
pub async fn show(connection: &zbus::Connection, view: View) {
    let shared = Arc::new(Shared { view: Mutex::new(view), revision: Mutex::new(1) });
    let server = connection.object_server();
    if server.at(ITEM_PATH, Item { shared: shared.clone() }).await.is_err() || server.at(MENU_PATH, Menu { shared }).await.is_err() {
        log::info!("no tray icon: its D-Bus objects could not be served");
        return;
    }
    let connection = connection.clone();
    tokio::spawn(async move {
        register(&connection).await;
        let Ok(bus) = zbus::fdo::DBusProxy::new(&connection).await else { return };
        let Ok(mut owners) = bus.receive_name_owner_changed_with_args(&[(0, WATCHER)]).await else { return };
        use futures_util::StreamExt;
        while let Some(change) = owners.next().await {
            if change.args().is_ok_and(|args| args.new_owner().is_some()) {
                register(&connection).await;
            }
        }
    });
}

async fn register(connection: &zbus::Connection) {
    let outcome = connection
        .call_method(Some(WATCHER), "/StatusNotifierWatcher", Some(WATCHER), "RegisterStatusNotifierItem", &(ITEM_PATH,))
        .await;
    match outcome {
        Ok(_) => log::info!("the tray icon is shown"),
        Err(error) => log::debug!("no tray to show the icon in ({error})"),
    }
}

/// The state changed: the menu and the tooltip follow.
pub async fn update(connection: &zbus::Connection, state: State) {
    change(connection, |view| view.state = state).await;
}

/// A newer release is out: the menu offers it.
pub async fn offer_update(connection: &zbus::Connection, release: &crate::updates::Release) {
    change(connection, |view| view.update = Some(release.clone())).await;
}

async fn change(connection: &zbus::Connection, edit: impl FnOnce(&mut View)) {
    let Ok(menu) = connection.object_server().interface::<_, Menu>(MENU_PATH).await else { return };
    let revision = {
        let menu = menu.get().await;
        edit(&mut menu.shared.view.lock().expect("tray view lock"));
        let mut revision = menu.shared.revision.lock().expect("tray revision lock");
        *revision += 1;
        *revision
    };
    let _ = Menu::layout_updated(menu.signal_emitter(), revision, 0).await;
    if let Ok(item) = connection.object_server().interface::<_, Item>(ITEM_PATH).await {
        let _ = Item::new_tool_tip(item.signal_emitter()).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(state: State) -> View {
        View {
            state,
            server_url: "https://photos.example.org:443/".to_owned(),
            server_version: "v3.3.5".to_owned(),
            account: "admin@example.com".to_owned(),
            mount_point: "/home/tester/Immich".to_owned(),
            on_kde: false,
            update: None,
        }
    }

    #[test]
    fn the_menu_reads_as_the_macs() {
        let lines = view(State::Online).lines();
        assert_eq!(lines[0], Line::Text("Connected to photos.example.org".to_owned()));
        assert_eq!(lines[1], Line::Text("Server version: Immich v3.3.5".to_owned()));
        assert_eq!(lines[2], Line::Text("Signed in as: admin@example.com".to_owned()));
        assert_eq!(lines[3], Line::Separator);
        assert_eq!(lines[4], Line::Action("Open in Files".to_owned(), Action::OpenDrive));
        assert_eq!(lines[5], Line::Action("Settings…".to_owned(), Action::OpenSettings));
        assert_eq!(lines[6], Line::Action("Open Immich Mount".to_owned(), Action::OpenApp));
        assert_eq!(lines[7], Line::Separator);
        assert_eq!(lines[8], Line::Action("Sign Out…".to_owned(), Action::SignOut));
        assert_eq!(lines.last(), Some(&Line::Action("Quit Immich Mount".to_owned(), Action::Quit)), "Quit at the foot");
        let offline = view(State::Offline).lines();
        assert_eq!(offline[0], Line::Text("photos.example.org is not reachable".to_owned()));
        let on_kde = View { on_kde: true, ..view(State::Online) };
        assert!(on_kde.lines().contains(&Line::Action("Open in Dolphin".to_owned(), Action::OpenDrive)));
        let release = crate::updates::Release { version: "2.21.0".to_owned(), page: "https://example.org/".to_owned() };
        let behind = View { update: Some(release), ..view(State::Online) };
        let behind = behind.lines();
        assert_eq!(behind[behind.len() - 4], Line::Action("Get Immich Mount 2.21.0…".to_owned(), Action::OpenUpdate));
        assert_eq!(behind.last(), Some(&Line::Action("Quit Immich Mount".to_owned(), Action::Quit)), "Quit stays last");
    }

    #[test]
    fn hosts_are_named_as_on_the_mac() {
        assert_eq!(url_host("https://photos.example.org:443/"), "photos.example.org");
        assert_eq!(url_host("http://192.168.1.10:2283"), "192.168.1.10");
        assert_eq!(url_host("photos.example.org"), "photos.example.org");
    }

    #[test]
    fn the_layout_has_one_child_per_line() {
        let (root, properties, children) = layout(&view(State::Online).lines());
        assert_eq!(root, 0);
        assert!(properties.contains_key("children-display"));
        assert_eq!(children.len(), 10, "three lines, a separator, three ways in, a separator, Sign Out… and Quit");
        // Each child is the structure dbusmenu wants, (ia{sv}av), inside a variant.
        assert_eq!(children[0].value_signature().to_string(), "(ia{sv}av)");
    }
}
