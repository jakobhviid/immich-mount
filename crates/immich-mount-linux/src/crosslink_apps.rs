//! The other apps that take part in immich-crosslink, as their desktop entries announce them (the protocol's
//! "Announcing": `X-Immich-Crosslink-Version` and `X-Immich-Crosslink-Shows`, with the scheme in `MimeType`). The
//! settings list them; the file managers' menus find them through their own toolkits, by the same rules.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The draft Immich Mount sends; an app announcing another is treated as not announcing at all.
pub const DRAFT: &str = "0.2";
/// The scheme's MIME type in a desktop entry.
pub const SCHEME_TYPE: &str = "x-scheme-handler/immich-crosslink";
/// Immich Mount's own entry, which is never listed.
pub const OWN_ENTRY: &str = "dev.hviid.ImmichMount.desktop";

/// An app found, named as its desktop entry names it, never by anything in a link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrosslinkApp {
    /// Its desktop entry id (`org.example.Photos.desktop`), by which the settings leave it out.
    pub id: String,
    pub name: String,
    pub icon: Option<String>,
    /// The kinds it shows (`asset`, `album`, `person`, `place`, `page`).
    pub shows: Vec<String>,
}

/// The apps installed for the user, by name.
pub fn installed() -> Vec<CrosslinkApp> {
    let languages = crate::i18n::languages(&|name| std::env::var(name).ok());
    let mut apps: Vec<CrosslinkApp> = entries(&application_folders()).into_iter().filter_map(|(id, text)| read(&id, &text, &languages)).collect();
    apps.sort_by_key(|app| app.name.to_lowercase());
    apps
}

/// The folders desktop entries are found in, the user's first: `XDG_DATA_HOME`, then `XDG_DATA_DIRS`.
fn application_folders() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let data_home = std::env::var_os("XDG_DATA_HOME").filter(|value| !value.is_empty()).map_or_else(|| home.join(".local/share"), PathBuf::from);
    let data_dirs = std::env::var("XDG_DATA_DIRS").ok().filter(|value| !value.is_empty()).unwrap_or_else(|| "/usr/local/share:/usr/share".to_owned());
    std::iter::once(data_home).chain(data_dirs.split(':').map(PathBuf::from)).map(|folder| folder.join("applications")).collect()
}

/// Every desktop entry's id and text, the first folder's winning where two hold one id, as the specification says. A
/// subfolder's name joins the id with `-`.
fn entries(folders: &[PathBuf]) -> BTreeMap<String, String> {
    fn walk(folder: &Path, prefix: &str, found: &mut BTreeMap<String, String>) {
        let Ok(listing) = std::fs::read_dir(folder) else { return };
        for entry in listing.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|name| name.to_str()).map(str::to_owned) else { continue };
            if path.is_dir() {
                walk(&path, &format!("{prefix}{name}-"), found);
            } else if name.ends_with(".desktop") {
                if let std::collections::btree_map::Entry::Vacant(slot) = found.entry(format!("{prefix}{name}")) {
                    if let Ok(text) = std::fs::read_to_string(&path) {
                        slot.insert(text);
                    }
                }
            }
        }
    }
    let mut found = BTreeMap::new();
    for folder in folders {
        walk(folder, "", &mut found);
    }
    found
}

/// The app an entry announces, if it takes part in the draft Immich Mount sends and is not Immich Mount.
fn read(id: &str, text: &str, languages: &[String]) -> Option<CrosslinkApp> {
    if id == OWN_ENTRY {
        return None;
    }
    let mut values: BTreeMap<String, String> = BTreeMap::new();
    let mut in_main_group = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_main_group = line == "[Desktop Entry]";
        } else if in_main_group {
            if let Some((key, value)) = line.split_once('=') {
                values.entry(key.trim().to_owned()).or_insert_with(|| value.trim().to_owned());
            }
        }
    }
    let flag = |key: &str| values.get(key).is_some_and(|value| value == "true");
    let list = |key: &str| values.get(key).map(|value| value.split(';').filter(|item| !item.is_empty()).map(str::to_owned).collect::<Vec<_>>()).unwrap_or_default();
    if flag("Hidden") || !list("MimeType").iter().any(|mime| mime == SCHEME_TYPE) || values.get("X-Immich-Crosslink-Version").map(String::as_str) != Some(DRAFT) {
        return None;
    }
    let name = languages.iter().find_map(|language| values.get(&format!("Name[{language}]"))).or_else(|| values.get("Name"))?.clone();
    Some(CrosslinkApp { id: id.to_owned(), name, icon: values.get("Icon").cloned(), shows: list("X-Immich-Crosslink-Shows") })
}

/// Whether the user left `app` out of the menus (`crosslink.hidden`, desktop entry ids separated by `;`).
pub fn is_hidden(hidden: &str, app: &str) -> bool {
    hidden.split(';').any(|id| id.trim() == app)
}

/// `hidden` with `app` left out or put back.
pub fn set_hidden(hidden: &str, app: &str, hide: bool) -> String {
    let mut ids: Vec<&str> = hidden.split(';').map(str::trim).filter(|id| !id.is_empty() && *id != app).collect();
    if hide {
        ids.push(app);
    }
    ids.join(";")
}

#[cfg(test)]
mod tests {
    use super::*;

    const GALLERY: &str = "[Desktop Entry]\nName=Gallery\nName[da]=Galleri\nIcon=org.example.Gallery\nExec=gallery %u\n\
        MimeType=image/jpeg;x-scheme-handler/immich-crosslink;\nX-Immich-Crosslink-Version=0.2\n\
        X-Immich-Crosslink-Shows=asset;album;\n\n[Desktop Action new]\nName=New Window\n";

    #[test]
    fn an_announced_app_is_read_by_its_entry() {
        let app = read("org.example.Gallery.desktop", GALLERY, &[]).unwrap();
        assert_eq!(app.name, "Gallery");
        assert_eq!(app.icon.as_deref(), Some("org.example.Gallery"));
        assert_eq!(app.shows, ["asset", "album"]);
        assert_eq!(read("org.example.Gallery.desktop", GALLERY, &["da_DK".into(), "da".into()]).unwrap().name, "Galleri");
    }

    #[test]
    fn apps_that_do_not_take_part_are_left_out() {
        assert_eq!(read(OWN_ENTRY, GALLERY, &[]), None, "never itself");
        assert_eq!(read("a.desktop", &GALLERY.replace("Version=0.2", "Version=0.1"), &[]), None, "another draft");
        assert_eq!(read("a.desktop", &GALLERY.replace("X-Immich-Crosslink-Version=0.2\n", ""), &[]), None, "no announcement");
        assert_eq!(read("a.desktop", &GALLERY.replace(";x-scheme-handler/immich-crosslink;", ";"), &[]), None, "no scheme");
        assert_eq!(read("a.desktop", &GALLERY.replace("Icon=", "Hidden=true\nIcon="), &[]), None, "deleted by the user");
    }

    #[test]
    fn the_users_entry_wins_and_subfolders_join_the_id() {
        let folders = tempfile::tempdir().unwrap();
        let (user, system) = (folders.path().join("user"), folders.path().join("system"));
        std::fs::create_dir_all(system.join("kde")).unwrap();
        std::fs::create_dir_all(&user).unwrap();
        std::fs::write(system.join("org.example.Gallery.desktop"), GALLERY).unwrap();
        std::fs::write(user.join("org.example.Gallery.desktop"), GALLERY.replace("Name=Gallery", "Name=Mine")).unwrap();
        std::fs::write(system.join("kde/viewer.desktop"), GALLERY).unwrap();
        let found = entries(&[user, system]);
        assert!(found["org.example.Gallery.desktop"].contains("Name=Mine"));
        assert!(found.contains_key("kde-viewer.desktop"));
    }

    #[test]
    fn hidden_apps_are_a_list_of_ids() {
        assert!(is_hidden("a.desktop; b.desktop", "b.desktop"));
        assert!(!is_hidden("", "a.desktop"));
        assert_eq!(set_hidden("a.desktop;b.desktop", "a.desktop", false), "b.desktop");
        assert_eq!(set_hidden("b.desktop", "a.desktop", true), "b.desktop;a.desktop");
        assert_eq!(set_hidden("a.desktop", "a.desktop", true), "a.desktop");
    }
}
