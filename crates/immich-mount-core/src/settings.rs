//! The settings table: one source of truth for every user setting.
//!
//! `immich-mount configure` and the desktop app's settings dialog both read this table, so they can never disagree
//! about which settings exist or what values are valid (ARCHITECTURE.md, "The desktop app"). Settings live in
//! `config.toml`. Reading is lenient: unknown keys are ignored. Writing goes through `toml_edit`, so comments and
//! the user's own layout survive.

use toml_edit::{value, DocumentMut, Item, Table};

use crate::folder_uploads::FolderChoice;
use crate::layout::{AlbumLayout, GroupingMode};
use crate::naming::FileNaming;

/// The config file format this build writes. Older files are migrated once on load.
pub const CONFIG_FORMAT_VERSION: i64 = 1;

/// The kind of value a setting takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Bool,
    /// Free text, such as a URL.
    Text,
    /// A file system path; `~` is expanded by the caller.
    Path,
    /// A size such as `10GB` or `10GiB`.
    Size,
    /// One of a fixed set of words.
    Choice(&'static [&'static str]),
}

/// One user setting.
#[derive(Debug, Clone, Copy)]
pub struct Setting {
    /// Dotted key, mirroring the TOML path.
    pub key: &'static str,
    pub kind: Kind,
    /// A one-line description, shown by `configure list` and as the settings dialog's subtitle.
    pub description: &'static str,
    /// The default, as text. Empty means "not set".
    pub default: &'static str,
}

const GROUPING_CHOICES: &[&str] = &["never", "atLeast500", "always"];
/// The names the drive may have in a file manager's sidebar, as `mount.name` stores them.
const MOUNT_NAMES: &[&str] = &["immich-mount", "immich-drive", "immich"];
/// The drive's name when none is chosen.
pub const DEFAULT_MOUNT_NAME: &str = "Immich Mount";

/// Every setting, in display order.
pub const SETTINGS: &[Setting] = &[
    Setting { key: "server.url", kind: Kind::Text, description: "The Immich server; /api is added", default: "" },
    Setting { key: "mount.path", kind: Kind::Path, description: "Where the mount appears", default: "~/Immich" },
    Setting { key: "mount.name", kind: Kind::Choice(MOUNT_NAMES), description: "The drive's name in the sidebar", default: "immich-mount" },
    Setting { key: "mount.at_login", kind: Kind::Bool, description: "Mount when the session starts", default: "true" },
    Setting { key: "sections.photos", kind: Kind::Bool, description: "Show the Photos section", default: "true" },
    Setting { key: "sections.albums", kind: Kind::Bool, description: "Show the Albums section", default: "true" },
    Setting { key: "sections.people", kind: Kind::Bool, description: "Show the People section", default: "true" },
    Setting { key: "sections.places", kind: Kind::Bool, description: "Show the Places section", default: "true" },
    Setting { key: "sections.favorites", kind: Kind::Bool, description: "Show the Favorites section", default: "false" },
    Setting { key: "sections.archive", kind: Kind::Bool, description: "Show the Archive section", default: "false" },
    Setting { key: "grouping.photos", kind: Kind::Choice(GROUPING_CHOICES), description: "Day folders inside months", default: "atLeast500" },
    Setting { key: "grouping.albums", kind: Kind::Choice(GROUPING_CHOICES), description: "Year and month folders inside albums", default: "atLeast500" },
    Setting { key: "grouping.people", kind: Kind::Choice(GROUPING_CHOICES), description: "Year and month folders inside people", default: "atLeast500" },
    Setting { key: "grouping.places", kind: Kind::Choice(GROUPING_CHOICES), description: "Year and month folders inside cities", default: "atLeast500" },
    Setting { key: "grouping.favorites", kind: Kind::Choice(GROUPING_CHOICES), description: "Year and month folders inside favorites", default: "always" },
    Setting { key: "grouping.archive", kind: Kind::Choice(GROUPING_CHOICES), description: "Year and month folders inside the archive", default: "always" },
    Setting { key: "albums.layout", kind: Kind::Choice(&["year-prefix", "year-folders", "flat"]), description: "How album folders are named and grouped", default: "year-prefix" },
    Setting { key: "files.naming", kind: Kind::Choice(&["date-time", "original"]), description: "How files are named", default: "date-time" },
    Setting { key: "cache.limit", kind: Kind::Size, description: "The content cache's size limit", default: "10GB" },
    Setting { key: "uploads.enabled", kind: Kind::Bool, description: "Show the Upload folder, and upload what is copied into it", default: "true" },
    Setting { key: "uploads.folders", kind: Kind::Choice(&["ask", "timeline", "albums"]), description: "Folders copied into Upload", default: "ask" },
    Setting { key: "thumbnails.sharp", kind: Kind::Bool, description: "Also seed 1024 px thumbnails from the preview", default: "true" },
    Setting { key: "thumbnails.safety_net", kind: Kind::Bool, description: "Refuse thumbnailer reads of originals", default: "true" },
    Setting { key: "web.links", kind: Kind::Bool, description: "Open in Immich and Copy Immich Link", default: "true" },
    Setting { key: "crosslink.enabled", kind: Kind::Bool, description: "Show what other apps link to (Immich Crosslink)", default: "true" },
    Setting { key: "crosslink.hidden", kind: Kind::Text, description: "Apps left out of Show in, by their desktop entry or bundle id, separated by ;", default: "" },
    Setting { key: "updates.check", kind: Kind::Bool, description: "Tell me when a new version is out", default: "true" },
    Setting { key: "credential.file", kind: Kind::Path, description: "Headless only: a 0600 file holding the credential", default: "" },
];

/// The cache sizes the settings windows offer, in decimal units as the file managers count; any other size set in
/// `config.toml` shows as itself.
pub const CACHE_SIZES: &[&str] = &["5GB", "10GB", "25GB", "50GB", "100GB"];

/// Where a setting is shown (ARCHITECTURE.md, "One layout for every settings window"). The apps lay their windows out
/// from this, each in its own platform's way; the Mac leaves out what means nothing there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    /// Opening at login, the drive's place and name, the cache, thumbnails and updates.
    General,
    /// The sections with their date folders, Upload, album and file names.
    Folders,
    /// Immich Crosslink, the apps that take part, and Immich's web app.
    OtherApps,
    /// Only in `config.toml` and `immich-mount configure`: set another way (the server, by signing in; the apps left
    /// out of Show in, by each app's switch), for headless machines only, or a safeguard no window should offer.
    CommandLine,
}

/// Every setting's pane. A test keeps it whole: a setting added to the table without a place here fails it.
pub const PANES: &[(&str, Pane)] = &[
    ("server.url", Pane::CommandLine),
    ("mount.path", Pane::General),
    ("mount.name", Pane::General),
    ("mount.at_login", Pane::General),
    ("sections.photos", Pane::Folders),
    ("sections.albums", Pane::Folders),
    ("sections.people", Pane::Folders),
    ("sections.places", Pane::Folders),
    ("sections.favorites", Pane::Folders),
    ("sections.archive", Pane::Folders),
    ("grouping.photos", Pane::Folders),
    ("grouping.albums", Pane::Folders),
    ("grouping.people", Pane::Folders),
    ("grouping.places", Pane::Folders),
    ("grouping.favorites", Pane::Folders),
    ("grouping.archive", Pane::Folders),
    ("albums.layout", Pane::Folders),
    ("files.naming", Pane::Folders),
    ("cache.limit", Pane::General),
    ("uploads.enabled", Pane::Folders),
    ("uploads.folders", Pane::Folders),
    ("thumbnails.sharp", Pane::General),
    // Off, a thumbnailer may download every original it is shown, gigabytes for icons: no window offers that.
    ("thumbnails.safety_net", Pane::CommandLine),
    ("web.links", Pane::OtherApps),
    ("crosslink.enabled", Pane::OtherApps),
    ("crosslink.hidden", Pane::CommandLine),
    ("updates.check", Pane::General),
    ("credential.file", Pane::CommandLine),
];

/// A setting's pane.
pub fn pane(key: &str) -> Option<Pane> {
    PANES.iter().find(|(placed, _)| *placed == key).map(|(_, pane)| *pane)
}

/// How a choice reads in a dialog, in every edition's app.
pub fn choice_label(word: &str) -> &str {
    match word {
        "never" => "Never",
        "atLeast500" => "From 500 Items",
        "always" => "Always",
        "year-prefix" => "Year in the Name (2023 - Trip)",
        "year-folders" => "Year Folders (2023/Trip)",
        "flat" => "Name Only (Trip)",
        "date-time" => "Date and Time Taken",
        "original" => "Original File Names",
        "ask" => "Ask Each Time",
        "timeline" => "Timeline Only",
        "albums" => "Make Albums",
        "immich-mount" | "immich-drive" | "immich" => mount_name(word),
        other => other,
    }
}

/// The name a `mount.name` choice shows in the sidebar. It names the drive only: the mount point is `mount.path`.
pub fn mount_name(choice: &str) -> &'static str {
    match choice {
        "immich-drive" => "Immich Drive",
        "immich" => "Immich",
        _ => DEFAULT_MOUNT_NAME,
    }
}

/// Every name the drive may have, so a sidebar entry under any of them is known as ours.
pub fn mount_names() -> impl Iterator<Item = &'static str> {
    MOUNT_NAMES.iter().map(|choice| mount_name(choice))
}

/// Why a setting could not be read or changed.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SettingError {
    #[error("unknown setting `{0}` (see `immich-mount configure keys`)")]
    UnknownKey(String),
    #[error("`{key}` must be {expected}, not `{given}`")]
    InvalidValue { key: String, expected: String, given: String },
    #[error("the config file is not valid TOML: {0}")]
    Unparsable(String),
}

/// Look a setting up by key.
pub fn setting(key: &str) -> Result<&'static Setting, SettingError> {
    SETTINGS.iter().find(|setting| setting.key == key).ok_or_else(|| SettingError::UnknownKey(key.to_string()))
}

/// Check that `given` is a valid value for `key`.
pub fn validate(key: &str, given: &str) -> Result<(), SettingError> {
    let setting = setting(key)?;
    let invalid = |expected: String| SettingError::InvalidValue { key: key.to_string(), expected, given: given.to_string() };
    match setting.kind {
        Kind::Bool => given.parse::<bool>().map(|_| ()).map_err(|_| invalid("`true` or `false`".into())),
        Kind::Text | Kind::Path => if given.trim().is_empty() { Err(invalid("non-empty".into())) } else { Ok(()) },
        Kind::Size => parse_size(given).map(|_| ()).ok_or_else(|| invalid("a size such as `10GB` or `500MiB`".into())),
        Kind::Choice(choices) => if choices.contains(&given) { Ok(()) } else { Err(invalid(format!("one of {}", choices.join(", ")))) },
    }
}

/// Parse a size: a whole number of bytes, or a number with a decimal unit (`KB`, `MB`, `GB`, `TB`, as file managers
/// count) or a binary one (`KiB`, `MiB`, `GiB`, `TiB`).
pub fn parse_size(text: &str) -> Option<u64> {
    let text = text.trim();
    let units = [
        ("TiB", 1u64 << 40),
        ("GiB", 1 << 30),
        ("MiB", 1 << 20),
        ("KiB", 1 << 10),
        ("TB", 1_000_000_000_000),
        ("GB", 1_000_000_000),
        ("MB", 1_000_000),
        ("KB", 1_000),
        ("B", 1),
    ];
    for (unit, multiplier) in units {
        if let Some(number) = text.strip_suffix(unit) {
            return number.trim().parse::<u64>().ok()?.checked_mul(multiplier);
        }
    }
    text.parse().ok()
}

/// The effective settings, typed.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub server_url: Option<String>,
    pub mount_path: String,
    /// The drive's name in the sidebar (`mount.name`).
    pub mount_name: &'static str,
    pub mount_at_login: bool,
    pub show_photos: bool,
    pub show_albums: bool,
    pub show_people: bool,
    pub show_places: bool,
    pub show_favorites: bool,
    pub show_archive: bool,
    pub grouping_photos: GroupingMode,
    pub grouping_albums: GroupingMode,
    pub grouping_people: GroupingMode,
    pub grouping_places: GroupingMode,
    pub grouping_favorites: GroupingMode,
    pub grouping_archive: GroupingMode,
    pub album_layout: AlbumLayout,
    pub file_naming: FileNaming,
    pub cache_limit_bytes: u64,
    pub uploads_enabled: bool,
    /// What a folder copied into Upload becomes besides its timeline uploads (`uploads.folders`).
    pub upload_folders: FolderChoice,
    pub sharp_thumbnails: bool,
    pub thumbnail_safety_net: bool,
    /// Linux packages from the download page: tell of a newer release (ARCHITECTURE.md, "Updates on Linux").
    pub update_check: bool,
    /// Immich Crosslink: show in the file manager what another app links to, and offer the other apps (ROADMAP.md,
    /// "Open in Immich, and links between apps").
    pub crosslink_enabled: bool,
    /// Open in Immich and Copy Immich Link in the file managers' menus (Immich Crosslink, "Copying links").
    pub web_links: bool,
    /// The apps left out of Show in *app*, by desktop entry or bundle id, separated by `;`.
    pub crosslink_hidden: String,
    pub credential_file: Option<String>,
}

impl Config {
    /// Read a config file's text. Unknown keys are ignored; a known key with an invalid value is an error.
    pub fn from_toml(text: &str) -> Result<Config, SettingError> {
        let document: DocumentMut = text.parse().map_err(|error: toml_edit::TomlError| SettingError::Unparsable(error.to_string()))?;
        let effective = |key: &str| -> Result<String, SettingError> {
            match read_value(&document, key) {
                Some(given) => {
                    validate(key, &given)?;
                    Ok(given)
                }
                None => Ok(setting(key)?.default.to_string()),
            }
        };
        let flag = |key: &str| -> Result<bool, SettingError> { Ok(effective(key)? == "true") };
        let optional = |key: &str| -> Result<Option<String>, SettingError> {
            let given = effective(key)?;
            Ok(if given.is_empty() { None } else { Some(given) })
        };
        Ok(Config {
            server_url: optional("server.url")?,
            mount_path: effective("mount.path")?,
            mount_name: mount_name(&effective("mount.name")?),
            mount_at_login: flag("mount.at_login")?,
            show_photos: flag("sections.photos")?,
            show_albums: flag("sections.albums")?,
            show_people: flag("sections.people")?,
            show_places: flag("sections.places")?,
            show_favorites: flag("sections.favorites")?,
            show_archive: flag("sections.archive")?,
            grouping_photos: grouping(&effective("grouping.photos")?),
            grouping_albums: grouping(&effective("grouping.albums")?),
            grouping_people: grouping(&effective("grouping.people")?),
            grouping_places: grouping(&effective("grouping.places")?),
            grouping_favorites: grouping(&effective("grouping.favorites")?),
            grouping_archive: grouping(&effective("grouping.archive")?),
            album_layout: match effective("albums.layout")?.as_str() {
                "year-folders" => AlbumLayout::YearFolders,
                "flat" => AlbumLayout::Flat,
                _ => AlbumLayout::YearPrefix,
            },
            file_naming: if effective("files.naming")? == "original" { FileNaming::Original } else { FileNaming::DateTime },
            cache_limit_bytes: parse_size(&effective("cache.limit")?).unwrap_or(10 << 30),
            uploads_enabled: flag("uploads.enabled")?,
            upload_folders: FolderChoice::from_setting(&effective("uploads.folders")?),
            sharp_thumbnails: flag("thumbnails.sharp")?,
            thumbnail_safety_net: flag("thumbnails.safety_net")?,
            update_check: flag("updates.check")?,
            crosslink_enabled: flag("crosslink.enabled")?,
            web_links: flag("web.links")?,
            crosslink_hidden: optional("crosslink.hidden")?.unwrap_or_default(),
            credential_file: optional("credential.file")?,
        })
    }
}

fn grouping(choice: &str) -> GroupingMode {
    match choice {
        "never" => GroupingMode::Never,
        "always" => GroupingMode::Always,
        _ => GroupingMode::AtLeast500,
    }
}

/// The value stored for `key`, as text, if the file sets it.
pub fn read_value(document: &DocumentMut, key: &str) -> Option<String> {
    let (table, leaf) = key.split_once('.')?;
    let item = document.get(table)?.get(leaf)?;
    if let Some(flag) = item.as_bool() {
        return Some(flag.to_string());
    }
    item.as_str().map(str::to_string)
}

/// The values a config file's text sets explicitly, by key. Keys the file does not set are absent.
pub fn stored_values(text: &str) -> Result<std::collections::HashMap<&'static str, String>, SettingError> {
    let document: DocumentMut = text.parse().map_err(|error: toml_edit::TomlError| SettingError::Unparsable(error.to_string()))?;
    Ok(SETTINGS.iter().filter_map(|setting| read_value(&document, setting.key).map(|value| (setting.key, value))).collect())
}

/// Set `key` to `given` in a config file's text, keeping everything else as it was.
pub fn set_value(text: &str, key: &str, given: &str) -> Result<String, SettingError> {
    validate(key, given)?;
    let mut document: DocumentMut = text.parse().map_err(|error: toml_edit::TomlError| SettingError::Unparsable(error.to_string()))?;
    let (table_name, leaf) = key.split_once('.').expect("every setting key is dotted");
    if document.get("version").is_none() {
        document["version"] = value(CONFIG_FORMAT_VERSION);
    }
    if !document.contains_table(table_name) {
        document[table_name] = Item::Table(Table::new());
    }
    document[table_name][leaf] = match setting(key)?.kind {
        Kind::Bool => value(given == "true"),
        _ => value(given),
    };
    Ok(document.to_string())
}

/// Remove `key` from a config file's text, so its default applies again.
pub fn unset_value(text: &str, key: &str) -> Result<String, SettingError> {
    setting(key)?;
    let mut document: DocumentMut = text.parse().map_err(|error: toml_edit::TomlError| SettingError::Unparsable(error.to_string()))?;
    let (table_name, leaf) = key.split_once('.').expect("every setting key is dotted");
    if let Some(table) = document.get_mut(table_name).and_then(Item::as_table_mut) {
        table.remove(leaf);
    }
    Ok(document.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_default_is_valid() {
        for setting in SETTINGS.iter().filter(|setting| !setting.default.is_empty()) {
            assert_eq!(validate(setting.key, setting.default), Ok(()), "{}", setting.key);
        }
    }

    #[test]
    fn an_empty_file_gives_the_defaults() {
        let config = Config::from_toml("").unwrap();
        assert_eq!(config.mount_path, "~/Immich");
        assert_eq!(config.mount_name, "Immich Mount");
        assert_eq!(config.album_layout, AlbumLayout::YearPrefix);
        assert_eq!(config.file_naming, FileNaming::DateTime);
        assert_eq!(config.grouping_photos, GroupingMode::AtLeast500);
        assert!(!config.show_archive && !config.show_favorites, "the archive and favorites are shown only when asked for");
        assert_eq!((config.grouping_archive, config.grouping_favorites), (GroupingMode::Always, GroupingMode::Always));
        assert_eq!(config.cache_limit_bytes, 10_000_000_000, "10 GB, as Files counts");
        assert!(config.server_url.is_none());
        assert_eq!(config.upload_folders, FolderChoice::Ask, "a folder copied into Upload asks before it makes albums");
    }

    #[test]
    fn the_drive_is_named_by_a_choice_and_the_mount_point_stays() {
        let config = Config::from_toml("[mount]\nname = \"immich\"\n").unwrap();
        assert_eq!((config.mount_name, config.mount_path.as_str()), ("Immich", "~/Immich"));
        assert_eq!(Config::from_toml("[mount]\nname = \"immich-drive\"\n").unwrap().mount_name, "Immich Drive");
        assert_eq!(choice_label("immich-mount"), "Immich Mount");
        assert!(Config::from_toml("[mount]\nname = \"Photos\"\n").is_err());
        assert_eq!(mount_names().collect::<Vec<_>>(), ["Immich Mount", "Immich Drive", "Immich"]);
    }

    #[test]
    fn unknown_keys_are_ignored_and_invalid_values_are_reported() {
        assert!(Config::from_toml("[old]\nthing = 1\n").is_ok());
        let invalid = Config::from_toml("[albums]\nlayout = \"sideways\"\n");
        assert!(matches!(invalid, Err(SettingError::InvalidValue { .. })));
    }

    #[test]
    fn setting_a_value_keeps_comments_and_unsetting_restores_the_default() {
        let original = "# my notes\n[server]\nurl = \"https://photos.example.org\" # home\n";
        let updated = set_value(original, "albums.layout", "flat").unwrap();
        assert!(updated.contains("# my notes"));
        assert!(updated.contains("# home"));
        assert_eq!(Config::from_toml(&updated).unwrap().album_layout, AlbumLayout::Flat);
        let restored = unset_value(&updated, "albums.layout").unwrap();
        assert_eq!(Config::from_toml(&restored).unwrap().album_layout, AlbumLayout::YearPrefix);
    }

    #[test]
    fn values_are_validated_before_writing() {
        assert!(matches!(set_value("", "files.naming", "camera"), Err(SettingError::InvalidValue { .. })));
        assert!(matches!(set_value("", "no.such", "x"), Err(SettingError::UnknownKey(_))));
        assert!(set_value("", "mount.at_login", "false").unwrap().contains("at_login = false"));
    }

    #[test]
    fn every_setting_has_one_pane_and_every_pane_entry_a_setting() {
        for setting in SETTINGS {
            assert_eq!(PANES.iter().filter(|(key, _)| *key == setting.key).count(), 1, "{} needs exactly one pane", setting.key);
        }
        for (key, _) in PANES {
            assert!(SETTINGS.iter().any(|setting| setting.key == *key), "{key} is placed but not a setting");
        }
        for size in CACHE_SIZES {
            assert!(validate("cache.limit", size).is_ok(), "{size}");
        }
    }

    #[test]
    fn sizes_parse_with_decimal_units() {
        assert_eq!(parse_size("10GB"), Some(10_000_000_000));
        assert_eq!(parse_size("500 MB"), Some(500_000_000));
        assert_eq!(parse_size("1TB"), Some(1_000_000_000_000));
    }

    #[test]
    fn sizes_parse_with_binary_units() {
        assert_eq!(parse_size("10GiB"), Some(10 << 30));
        assert_eq!(parse_size("500 MiB"), Some(500 << 20));
        assert_eq!(parse_size("4096"), Some(4096));
        assert_eq!(parse_size("ten"), None);
    }
}
