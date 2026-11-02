//! KDE's settings that decide how the mount looks in Dolphin.
//!
//! KIO counts a FUSE mount as remote, and Dolphin shows no previews of remote files until
//! `[PreviewSettings] MaximumRemoteSize` in `kdeglobals` allows them. The setting is
//! KDE's, for every remote location, so it is changed only when the user asks. And Solid lists only the FUSE mounts
//! it knows (sshfs, rclone, the encrypted ones), so the mount reaches Dolphin's Places as a place of its own.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::paths::Paths;

/// What turning previews on allows: files up to 100 MB, enough for photos and short videos.
pub const PREVIEW_LIMIT: u64 = 104_857_600;

/// Whether KDE is the running desktop, by this process's environment.
pub fn is_running() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|desktop| desktop.split(':').any(|name| name == "KDE"))
}

/// Whether this user's Plasma session runs: KDE in the environment, or Plasma's shell on the session bus. The daemon
/// is a user service that can start at login before Plasma hands the session its environment, so it asks the bus
/// too, at most once a minute.
pub async fn plasma_is_running() -> bool {
    static LAST_ANSWER: std::sync::Mutex<Option<(std::time::Instant, bool)>> = std::sync::Mutex::new(None);
    if is_running() {
        return true;
    }
    if let Some((asked, answer)) = *LAST_ANSWER.lock().expect("answer lock") {
        if asked.elapsed() < std::time::Duration::from_secs(60) {
            return answer;
        }
    }
    let answer = async {
        let connection = zbus::Connection::session().await.ok()?;
        let bus = zbus::fdo::DBusProxy::new(&connection).await.ok()?;
        bus.name_has_owner(zbus::names::BusName::try_from("org.kde.plasmashell").ok()?).await.ok()
    }
    .await
    .unwrap_or(false);
    *LAST_ANSWER.lock().expect("answer lock") = Some((std::time::Instant::now(), answer));
    answer
}

fn kdeglobals(paths: &Paths) -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| paths.home.join(".config")).join("kdeglobals")
}

/// The largest remote file Dolphin previews, in bytes: 0 when previews of remote files are off (KDE's default).
/// `None` when there is no `kdeglobals`, which on a desktop other than KDE means Dolphin was never set up.
pub fn remote_preview_limit(paths: &Paths) -> Option<u64> {
    let text = std::fs::read_to_string(kdeglobals(paths)).ok()?;
    Some(maximum_remote_size(&text).unwrap_or(0))
}

/// Let Dolphin preview remote files up to `PREVIEW_LIMIT`, or return to KDE's default of none, through KDE's own
/// `kwriteconfig6` so a running Plasma notices.
pub fn allow_remote_previews(allow: bool) -> Result<(), String> {
    let mut command = Command::new("kwriteconfig6");
    command.args(["--file", "kdeglobals", "--group", "PreviewSettings", "--key", "MaximumRemoteSize"]);
    if allow {
        command.arg(PREVIEW_LIMIT.to_string());
    } else {
        command.arg("--delete");
    }
    let outcome = command.output().map_err(|error| format!("kwriteconfig6: {error}"))?;
    if outcome.status.success() {
        Ok(())
    } else {
        Err(format!("kwriteconfig6: {}", String::from_utf8_lossy(&outcome.stderr).trim()))
    }
}

/// Dolphin's Places panel: KIO's bookmarks of places, the file "Add to Places" writes.
fn user_places(paths: &Paths) -> PathBuf {
    std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| paths.home.join(".local/share")).join("user-places.xbel")
}

/// The Places file KIO starts from when there is none: no `kde_places_version`, so KIO still adds its own default
/// places (Home, Trash, Network) the first time it reads it.
const EMPTY_PLACES: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE xbel>\n<xbel \
    xmlns:bookmark=\"http://www.freedesktop.org/standards/desktop-bookmarks\" \
    xmlns:mime=\"http://www.freedesktop.org/standards/shared-mime-info\" xmlns:kdepriv=\"http://www.kde.org/kdepriv\">\n</xbel>\n";

/// Show the mount in Dolphin's Places, as "Add to Places" would, unless it is there already. KIO counts a FUSE mount
/// it does not know as neither a device nor a network share, so without this Dolphin lists it nowhere. Called once,
/// by the KDE app's first-run setup, so a place the user removed stays removed. `Ok(true)` when it was added.
pub fn add_place(paths: &Paths, mount_point: &Path, title: &str) -> Result<bool, String> {
    let file = user_places(paths);
    let text = match std::fs::read_to_string(&file) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => EMPTY_PLACES.to_owned(),
        Err(error) => return Err(format!("{}: {error}", file.display())),
    };
    let Some(updated) = with_place(&text, &crate::thumbnails::file_uri(mount_point), title) else {
        return Ok(false);
    };
    write_places(&file, &updated).map(|()| true)
}

/// Take the mount out of Dolphin's Places (at logout). Nothing to remove is not an error.
pub fn remove_place(paths: &Paths, mount_point: &Path) -> Result<bool, String> {
    let file = user_places(paths);
    let Ok(text) = std::fs::read_to_string(&file) else { return Ok(false) };
    let Some(updated) = without_place(&text, &crate::thumbnails::file_uri(mount_point)) else {
        return Ok(false);
    };
    write_places(&file, &updated).map(|()| true)
}

/// Replace the file whole (KIO watches it and reloads), so a reader never sees half of it.
fn write_places(file: &Path, text: &str) -> Result<(), String> {
    let describe = |error: std::io::Error| format!("{}: {error}", file.display());
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).map_err(describe)?;
    }
    let temporary = file.with_extension("xbel.immich-mount");
    std::fs::write(&temporary, text).map_err(describe)?;
    std::fs::rename(&temporary, file).map_err(describe)
}

/// The opening tag of the place for `uri`, as KIO writes it. A file URI keeps `&` and `'` as they are, so the
/// attribute is escaped: an unescaped `&` would make the whole file unreadable to KIO, which then writes its
/// defaults over the user's places.
fn place_tag(uri: &str) -> String {
    format!("<bookmark href=\"{}\">", xml_escaped(uri))
}

fn xml_escaped(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// `places` with a place for `uri` added at the end, or `None` when it has one, or is not a Places file.
fn with_place(places: &str, uri: &str, title: &str) -> Option<String> {
    if places.contains(&place_tag(uri)) {
        return None;
    }
    let end = places.rfind("</xbel>")?;
    let place = format!(
        " {}\n  <title>{}</title>\n  <info>\n   <metadata owner=\"http://freedesktop.org\">\n    \
         <bookmark:icon name=\"dev.hviid.ImmichMount\"/>\n   </metadata>\n   <metadata owner=\"http://www.kde.org\">\n    \
         <ID>immich-mount</ID>\n   </metadata>\n  </info>\n </bookmark>\n",
        place_tag(uri),
        xml_escaped(title)
    );
    Some(format!("{}{place}{}", &places[..end], &places[end..]))
}

/// `places` without the place for `uri`, or `None` when it has none.
fn without_place(places: &str, uri: &str) -> Option<String> {
    let start = places.find(&place_tag(uri))?;
    let end = start + places[start..].find("</bookmark>")? + "</bookmark>".len();
    // The line the place stood on goes too, when the place had it to itself.
    let line_start = places[..start].rfind('\n').map_or(0, |newline| newline + 1);
    let (start, end) = if places[line_start..start].trim().is_empty() && places[end..].starts_with('\n') {
        (line_start, end + 1)
    } else {
        (start, end)
    };
    Some(format!("{}{}", &places[..start], &places[end..]))
}

/// `MaximumRemoteSize` from `kdeglobals`' `[PreviewSettings]` group.
fn maximum_remote_size(kdeglobals: &str) -> Option<u64> {
    let mut in_group = false;
    for line in kdeglobals.lines().map(str::trim) {
        if line.starts_with('[') {
            in_group = line == "[PreviewSettings]";
        } else if in_group {
            if let Some(value) = line.strip_prefix("MaximumRemoteSize=") {
                return value.trim().parse().ok();
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_remote_preview_limit_is_read_from_its_group_only() {
        assert_eq!(maximum_remote_size("[General]\nMaximumRemoteSize=5\n[PreviewSettings]\nMaximumRemoteSize=104857600\n"), Some(104_857_600));
        assert_eq!(maximum_remote_size("[General]\nMaximumRemoteSize=5\n"), None);
    }

    #[test]
    fn a_place_is_added_once_and_removed_whole() {
        let original = "<xbel>\n <bookmark href=\"file:///home/me\">\n  <title>Home</title>\n </bookmark>\n</xbel>\n";
        let added = with_place(original, "file:///home/me/Immich", "Immich").expect("added");
        assert!(added.contains("<bookmark href=\"file:///home/me/Immich\">") && added.contains("<title>Immich</title>"));
        assert!(added.ends_with("</xbel>\n"));
        assert_eq!(with_place(&added, "file:///home/me/Immich", "Immich"), None);
        assert_eq!(without_place(&added, "file:///home/me/Immich").as_deref(), Some(original));
        assert_eq!(without_place(original, "file:///home/me/Immich"), None);
        assert_eq!(with_place("not places", "file:///x", "x"), None);
    }

    #[test]
    fn a_path_with_an_ampersand_stays_valid_xml_and_a_one_line_file_keeps_its_other_places() {
        let added = with_place("<xbel>\n</xbel>\n", "file:///home/me/Photos&Videos", "Immich").expect("added");
        assert!(added.contains("href=\"file:///home/me/Photos&amp;Videos\""));
        assert_eq!(without_place(&added, "file:///home/me/Photos&Videos").as_deref(), Some("<xbel>\n</xbel>\n"));

        let one_line = "<xbel><bookmark href=\"file:///home/me\"><title>Home</title></bookmark>\
                        <bookmark href=\"file:///home/me/Immich\"><title>Immich</title></bookmark></xbel>";
        let kept = without_place(one_line, "file:///home/me/Immich").expect("removed");
        assert_eq!(kept, "<xbel><bookmark href=\"file:///home/me\"><title>Home</title></bookmark></xbel>");
    }
}
