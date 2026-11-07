//! The preferences every desktop app shows, generated from the core's settings table, the same one behind
//! `immich-mount configure` (ARCHITECTURE.md, "The desktop app"). The apps differ only in their widgets.

use immich_mount_core::settings::{self, Setting, SETTINGS};
use immich_mount_linux::paths::Paths;

/// The groups the apps show, by the settings' key prefix, with their titles. Settings outside these (the server, set
/// by signing in; the headless credential file; uploads) are left to `immich-mount configure`.
pub const GROUPS: &[(&str, &str)] = &[
    ("mount", "Mount"),
    ("sections", "Sections"),
    ("grouping", "Date Folders"),
    ("albums", "Albums"),
    ("files", "Files"),
    ("thumbnails", "Thumbnails"),
    ("cache", "Cache"),
];

/// The settings of one group, in table order.
pub fn settings_in(prefix: &str) -> impl Iterator<Item = &'static Setting> + '_ {
    SETTINGS.iter().filter(move |setting| setting.key.split('.').next() == Some(prefix))
}

pub use immich_mount_core::settings::choice_label;

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
