//! The settings pages every desktop app shows (ARCHITECTURE.md, "One layout for every settings window"): designed here,
//! once, over the core's settings table, the same one behind `immich-mount configure`. Each setting's pane comes from
//! the core (`settings::PANES`); this says how each page reads. The apps differ only in their widgets.

use immich_mount_core::settings::{self, Pane, Setting, SETTINGS};
use immich_mount_linux::i18n::tr;
use immich_mount_linux::paths::Paths;

/// One row of a settings page: a setting, its title and the line under it, in English (`tr` translates them), and,
/// for a section, the setting shown with it (its date folders, or what Upload does with folders).
pub struct Row {
    pub key: &'static str,
    pub title: &'static str,
    pub subtitle: Option<&'static str>,
    pub inner: Option<(&'static str, &'static str)>,
}

/// A group of rows under a title. `apps`: the apps that take part in Immich Crosslink follow this group.
pub struct Group {
    pub title: Option<&'static str>,
    pub description: Option<&'static str>,
    pub rows: &'static [Row],
    pub apps: bool,
}

/// A page of settings, by the pane it shows, its title, its icon and its groups.
pub struct Page {
    pub pane: Pane,
    pub id: &'static str,
    pub title: &'static str,
    pub icon: &'static str,
    pub groups: &'static [Group],
}

const fn row(key: &'static str, title: &'static str, subtitle: Option<&'static str>) -> Row {
    Row { key, title, subtitle, inner: None }
}

const fn section(key: &'static str, title: &'static str, subtitle: Option<&'static str>, inner: (&'static str, &'static str)) -> Row {
    Row { key, title, subtitle, inner: Some(inner) }
}

/// The settings pages, in the sidebar's order after Status (and before Account), as the Mac's Settings window has them.
pub const PAGES: &[Page] = &[
    Page {
        pane: Pane::General,
        id: "general",
        title: "General",
        icon: "preferences-system-symbolic",
        groups: &[
            Group {
                title: Some("Drive"),
                description: None,
                rows: &[
                    row("mount.at_login", "Mount at Login", None),
                    row("mount.path", "Location", None),
                    row("mount.name", "Name in the Sidebar", None),
                ],
                apps: false,
            },
            Group {
                title: Some("Storage"),
                description: None,
                rows: &[
                    row("cache.limit", "Cache Size", Some("Opened originals are kept on this computer up to this size.")),
                    row("thumbnails.sharp", "Sharp Thumbnails", Some("Also download larger previews, for big icon sizes. Uses more data.")),
                ],
                apps: false,
            },
            Group {
                title: Some("Updates"),
                description: None,
                rows: &[row("updates.check", "Check for Updates", Some("Tell me when a new version is out."))],
                apps: false,
            },
        ],
    },
    Page {
        pane: Pane::Folders,
        id: "folders",
        title: "Folders",
        icon: "folder-symbolic",
        groups: &[
            Group {
                title: Some("Sections"),
                description: Some("The folders the drive shows, and how each splits its photos by date."),
                rows: &[
                    section("sections.photos", "Photos", Some("The timeline, by year and month"), ("grouping.photos", "Day Folders")),
                    section("sections.albums", "Albums", None, ("grouping.albums", "Year and Month Folders")),
                    section("sections.people", "People", None, ("grouping.people", "Year and Month Folders")),
                    section("sections.places", "Places", None, ("grouping.places", "Year and Month Folders")),
                    section("sections.favorites", "Favorites", None, ("grouping.favorites", "Year and Month Folders")),
                    section("sections.archive", "Archive", None, ("grouping.archive", "Year and Month Folders")),
                    section("uploads.enabled", "Upload", Some("What is copied into it is uploaded"), ("uploads.folders", "Folders Copied Into Upload")),
                ],
                apps: false,
            },
            Group {
                title: Some("Names"),
                description: None,
                rows: &[row("albums.layout", "Album Folders", None), row("files.naming", "File Names", None)],
                apps: false,
            },
        ],
    },
    Page {
        pane: Pane::OtherApps,
        id: "other-apps",
        title: "Other Apps",
        icon: "send-to-symbolic",
        groups: &[
            Group { title: None, description: None, rows: &[row("crosslink.enabled", "Immich Crosslink", None)], apps: true },
            Group {
                title: Some("Immich Web App"),
                description: None,
                rows: &[row("web.links", "Open in Immich and Copy Immich Link", None)],
                apps: false,
            },
        ],
    },
];

/// A folder as the location setting is written: `~/…` under home, which may be reached through a symlink (Fedora's
/// atomic desktops, Bazzite: a folder chooser may give the resolved path), else the path as it is.
pub fn home_relative(path: &std::path::Path) -> String {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_default();
    let homes = [home.clone(), home.canonicalize().unwrap_or(home)];
    for home in homes.iter().filter(|home| !home.as_os_str().is_empty()) {
        if let Ok(rest) = path.strip_prefix(home) {
            return if rest.as_os_str().is_empty() { "~".to_owned() } else { format!("~/{}", rest.display()) };
        }
    }
    path.display().to_string()
}

/// A setting of the table, by key.
pub fn setting(key: &str) -> &'static Setting {
    SETTINGS.iter().find(|setting| setting.key == key).expect("every row names a setting of the table")
}

/// The cache size a row shows: one of the sizes offered, or, set by hand to another, itself.
pub fn cache_sizes() -> Vec<String> {
    let current = current_value(setting("cache.limit"));
    let mut sizes: Vec<String> = settings::CACHE_SIZES.iter().map(|size| (*size).to_owned()).collect();
    if !settings::CACHE_SIZES.contains(&current.as_str()) {
        sizes.push(current);
    }
    sizes
}

/// A size in decimal units, as Files and Dolphin count, the same in both apps: whole where it is ("10 GB"), else to a
/// tenth ("1.4 GB").
pub fn format_size(bytes: u64) -> String {
    let (value, unit) = match bytes {
        1_000_000_000_000.. => (bytes as f64 / 1e12, "TB"),
        1_000_000_000.. => (bytes as f64 / 1e9, "GB"),
        _ => (bytes as f64 / 1e6, "MB"),
    };
    if (value - value.round()).abs() < 0.05 { format!("{value:.0} {unit}") } else { format!("{value:.1} {unit}") }
}

/// How a cache size reads, marked when it was set by hand to one the apps do not offer.
pub fn cache_size_label(size: &str) -> String {
    let bytes = format_size(settings::parse_size(size).unwrap_or_default());
    if settings::CACHE_SIZES.contains(&size) {
        bytes
    } else {
        tr("Custom ({size})").replace("{size}", &bytes)
    }
}

/// The protocol behind `crosslink.enabled`, linked beside its switch so it is clear what it is (immich-crosslink,
/// "Settings": the same label and link in every app that takes part).
pub const CROSSLINK_PAGE: &str = "https://jakob.hviid.phd/immich-crosslink/";

/// The protocol's list of the apps that take part, linked when none is found.
pub const PARTICIPANTS_PAGE: &str = "https://jakob.hviid.phd/immich-crosslink/participants.html";

/// What the switches and Forget of the apps that take part do, under their list.
pub fn apps_description() -> &'static str {
    tr("Apps that take part are found by themselves. Turn one off to leave it out of Show in. One not found stays listed for 30 days, so it stays off if it comes back; Forget removes it now.")
}

/// The title of the list of apps under Immich Crosslink's switch, in the user's language.
pub fn apps_heading() -> &'static str {
    tr("Apps That Take Part")
}

/// A line under a setting's title, in the user's language, where one says more than the title: the row's own, or
/// one of these.
pub fn caption(setting: &Setting) -> Option<&'static str> {
    if let Some(subtitle) = PAGES.iter().flat_map(|page| page.groups).flat_map(|group| group.rows).find(|row| row.key == setting.key).and_then(|row| row.subtitle) {
        return Some(tr(subtitle));
    }
    match setting.key {
        "web.links" => Some(tr("Open photos, albums, people and places in Immich's web app in your browser, or copy their address.")),
        "crosslink.enabled" => Some(tr("Show photos, albums, people and places in other apps that take part, and show the items they send in the file manager.")),
        _ => None,
    }
}

/// The page a setting's row links to, if any.
pub fn link(setting: &Setting) -> Option<&'static str> {
    (setting.key == "crosslink.enabled").then_some(CROSSLINK_PAGE)
}

/// A setting's title in the user's language: its row's title, or, for one shown inside a section, the title given
/// there, translated. Immich Crosslink's switch carries the protocol's name, in English in every language.
pub fn title(setting: &Setting) -> &'static str {
    if setting.key == "crosslink.enabled" {
        return "Immich Crosslink";
    }
    let rows = || PAGES.iter().flat_map(|page| page.groups).flat_map(|group| group.rows);
    if let Some(row) = rows().find(|row| row.key == setting.key) {
        return tr(row.title);
    }
    if let Some((_, title)) = rows().filter_map(|row| row.inner).find(|(key, _)| *key == setting.key) {
        return tr(title);
    }
    tr(setting.description)
}

/// How a choice reads in a dialog, in the user's language. The names the drive may have stay as they are.
pub fn choice_label(word: &str) -> &str {
    tr(settings::choice_label(word))
}

/// The other apps that take crosslinks, installed now or seen in the last 30 days, each with whether Show in offers
/// it (Immich Crosslink, "Settings"). Apps forgotten now lose their switch too.
pub fn crosslink_apps() -> Vec<(immich_mount_linux::crosslink_apps::ListedApp, bool)> {
    use immich_mount_linux::crosslink_apps;
    let (listed, forgotten) = crosslink_apps::listed(&seen_file(), crosslink_apps::installed(), chrono::Utc::now());
    for id in &forgotten {
        let _ = show_app(id, true);
    }
    let hidden = stored("crosslink.hidden");
    listed
        .into_iter()
        .map(|listed| {
            let shown = !crosslink_apps::is_hidden(&hidden, &listed.app.id);
            (listed, shown)
        })
        .collect()
}

/// Forget an app not found, and its switch: should it come back, it is new, and on.
pub fn forget_app(id: &str) -> Result<bool, String> {
    immich_mount_linux::crosslink_apps::forget(&seen_file(), id);
    show_app(id, true)
}

/// Where the apps seen are remembered: the app's state, not a setting.
fn seen_file() -> std::path::PathBuf {
    Paths::from_environment().state_directory.join("crosslink-seen.json")
}

/// Offer an app in Show in, or leave it out. `Ok(true)`: the daemon restarts to see it, as for any setting.
pub fn show_app(id: &str, shown: bool) -> Result<bool, String> {
    let hidden = immich_mount_linux::crosslink_apps::set_hidden(&stored("crosslink.hidden"), id, !shown);
    if hidden.is_empty() {
        // No app left out: the key goes, since a text setting takes no empty value.
        unset("crosslink.hidden")?;
        return Ok(true);
    }
    change("crosslink.hidden", &hidden)
}

/// One setting's stored value, or empty.
fn stored(key: &str) -> String {
    settings::stored_values(&config_text(&Paths::from_environment())).unwrap_or_default().get(key).cloned().unwrap_or_default()
}

fn config_text(paths: &Paths) -> String {
    std::fs::read_to_string(paths.config_file()).unwrap_or_default()
}

/// Every setting's current value as stored, or its default.
pub fn current_value(setting: &Setting) -> String {
    let stored = settings::stored_values(&config_text(&Paths::from_environment())).unwrap_or_default();
    stored.get(setting.key).cloned().unwrap_or_else(|| setting.default.to_owned())
}

/// The drive's name in the sidebar (`mount.name`), for the entry the setup adds.
pub fn drive_name() -> &'static str {
    SETTINGS.iter().find(|setting| setting.key == "mount.name").map_or(settings::DEFAULT_MOUNT_NAME, |setting| {
        settings::mount_name(&current_value(setting))
    })
}

/// Store one value, comments in `config.toml` kept. The message says why when it is refused.
pub fn store(key: &str, value: &str) -> Result<(), String> {
    let paths = Paths::from_environment();
    let updated = settings::set_value(&config_text(&paths), key, value).map_err(|error| error.to_string())?;
    write_config(&paths, updated)
}

/// Remove one value, so its default applies again.
fn unset(key: &str) -> Result<(), String> {
    let paths = Paths::from_environment();
    let updated = settings::unset_value(&config_text(&paths), key).map_err(|error| error.to_string())?;
    write_config(&paths, updated)
}

fn write_config(paths: &Paths, updated: String) -> Result<(), String> {
    std::fs::create_dir_all(&paths.config_directory).map_err(|error| error.to_string())?;
    let temporary = paths.config_file().with_extension("toml.tmp");
    std::fs::write(&temporary, updated).map_err(|error| error.to_string())?;
    std::fs::rename(&temporary, paths.config_file()).map_err(|error| error.to_string())
}

/// Store one value, and act on the one that is not read from the file: mounting at login is the user service being
/// enabled. `Ok(true)` when the daemon must restart to see the change (`restart_daemon`, once the page closes).
pub fn change(key: &str, value: &str) -> Result<bool, String> {
    store(key, value)?;
    if key == "mount.at_login" {
        let verb = if value == "true" { "enable" } else { "disable" };
        std::thread::spawn(move || {
            let _ = std::process::Command::new("systemctl").args(["--user", verb, immich_mount_linux::service::user_service()]).status();
        });
        return Ok(false);
    }
    Ok(true)
}

/// The daemon reads its settings at start: restart it, if it is running, off the caller's thread.
pub fn restart_daemon() {
    std::thread::spawn(|| {
        let _ = std::process::Command::new("systemctl").args(["--user", "try-restart", immich_mount_linux::service::user_service()]).status();
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use immich_mount_core::settings::Kind;

    #[test]
    fn every_setting_a_window_shows_is_on_its_pane_page_once() {
        let mut shown: Vec<&str> = Vec::new();
        for page in PAGES {
            for row in page.groups.iter().flat_map(|group| group.rows) {
                for key in std::iter::once(row.key).chain(row.inner.map(|(key, _)| key)) {
                    assert_eq!(settings::pane(key), Some(page.pane), "{key} is on the {} page, its pane says otherwise", page.id);
                    assert!(!shown.contains(&key), "{key} is shown twice");
                    shown.push(key);
                }
            }
        }
        for setting in SETTINGS {
            let placed = settings::pane(setting.key) != Some(Pane::CommandLine);
            assert_eq!(placed, shown.contains(&setting.key), "{}: placed on a pane, it must be shown; on the command line only, never", setting.key);
        }
    }

    #[test]
    fn sizes_read_as_the_file_managers_count() {
        assert_eq!(format_size(10_000_000_000), "10 GB");
        assert_eq!(format_size(1_400_000_000), "1.4 GB");
        assert_eq!(format_size(0), "0 MB");
        assert_eq!(cache_size_label("25GB"), "25 GB");
    }

    #[test]
    fn every_choice_shown_has_a_label() {
        for row in PAGES.iter().flat_map(|page| page.groups).flat_map(|group| group.rows) {
            for key in std::iter::once(row.key).chain(row.inner.map(|(key, _)| key)) {
                if let Kind::Choice(words) = setting(key).kind {
                    for word in words {
                        assert_ne!(choice_label(word), *word, "{key} has no label for {word}");
                    }
                }
            }
        }
    }
}
