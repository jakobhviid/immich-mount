//! The preferences every desktop app shows, generated from the core's settings table, the same one behind
//! `immich-mount configure` (ARCHITECTURE.md, "The desktop app"). The apps differ only in their widgets.

use immich_mount_core::settings::{self, Setting, SETTINGS};
use immich_mount_linux::i18n::tr;
use immich_mount_linux::paths::Paths;

/// The groups the apps show, by the settings' key prefix, with their titles in English (`tr` translates them).
/// Settings outside these (the server, set by signing in; the headless credential file) are left to
/// `immich-mount configure`.
pub const GROUPS: &[(&str, &str)] = &[
    ("mount", "Drive"),
    ("sections", "Sections"),
    ("grouping", "Date Folders"),
    ("albums", "Albums"),
    ("files", "Files"),
    ("uploads", "Uploads"),
    ("thumbnails", "Thumbnails"),
    ("cache", "Cache"),
    ("crosslink", "Other Apps"),
    ("web", "Immich Web App"),
    ("updates", "Updates"),
];

/// The protocol behind `crosslink.enabled`, linked beside its switch so it is clear what it is (immich-crosslink,
/// "Settings": the same label and link in every app that takes part).
pub const CROSSLINK_PAGE: &str = "https://jakob.hviid.phd/immich-crosslink/";

/// The protocol's list of the apps that take part, linked when none is found.
pub const PARTICIPANTS_PAGE: &str = "https://jakob.hviid.phd/immich-crosslink/participants.html";

/// What the switches and Forget of the apps that take part do, under their list.
pub fn apps_description() -> &'static str {
    tr("Apps that take part are found by themselves. Turn one off to leave it out of Show in. Forget clears what Immich Mount remembers of an app: one that still takes part comes back, switched on.")
}

/// The title of the list of apps under Immich Crosslink's switch, in the user's language.
pub fn apps_heading() -> &'static str {
    tr("Apps That Take Part")
}

/// A line under a setting's title, in the user's language, where one says more than the title.
pub fn caption(setting: &Setting) -> Option<&'static str> {
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

/// A setting's title in the user's language: its description in the core's table, translated. Immich Crosslink's
/// switch carries the protocol's name, in English in every language.
pub fn title(setting: &Setting) -> &'static str {
    if setting.key == "crosslink.enabled" {
        return "Immich Crosslink";
    }
    tr(setting.description)
}

/// How a choice reads in a dialog, in the user's language. The names the drive may have stay as they are.
pub fn choice_label(word: &str) -> &str {
    tr(settings::choice_label(word))
}

/// The settings of one group, in table order. The apps left out of Show in are not a row of their own: each app found
/// has its switch (`crosslink_apps`).
pub fn settings_in(prefix: &str) -> impl Iterator<Item = &'static Setting> + '_ {
    SETTINGS.iter().filter(move |setting| setting.key.split('.').next() == Some(prefix) && setting.key != "crosslink.hidden")
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

/// Forget an app, found or not, and its switch: one that still takes part is found again, new and on.
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
    fn every_choice_shown_has_a_label() {
        for (prefix, _) in GROUPS {
            for setting in settings_in(prefix) {
                if let Kind::Choice(words) = setting.kind {
                    for word in words {
                        assert_ne!(choice_label(word), *word, "{} has no label for {word}", setting.key);
                    }
                }
            }
        }
    }
}
