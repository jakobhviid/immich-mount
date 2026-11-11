//! How the mount shows in GNOME Files' sidebar, and in the other file managers that list GVfs's mounts.
//!
//! GVfs offers a FUSE mount to the sidebar by its path alone (`should_include` in its udisks2 volume monitor): under
//! `$HOME`, or under `/media` or `/run/media/$USER`, and never inside a hidden folder. It compares the path the kernel
//! reports, which is always resolved, with `$HOME` as written. So where home is reached through a symlink, as on
//! Fedora's atomic desktops and Bazzite (`HOME=/home/me`, with `/home` a link to `/var/home`), no mount in it ever
//! matches. The daemon then gives the volume monitor, and only it, the resolved home through a drop-in for its user
//! service, so the mount is listed as a drive. Where that cannot be done, the app's setup bookmarks the mount.
//!
//! The sidebar draws a mount with a symbolic icon GVfs takes from the mount's volume, and a FUSE mount has a volume
//! only through an fstab entry. The same drop-in points the volume monitor at a private copy of `/etc/fstab`
//! (`LIBMOUNT_FSTAB`), where the daemon puts the mount's line, with our icon, only while it is mounted
//! (ARCHITECTURE.md, "The drive's icon").

use std::path::{Path, PathBuf};

use immich_mount_core::settings::mount_names;

use crate::mounts::canonical_mount_point;
use crate::paths::Paths;
use crate::thumbnails::file_uri;

/// Whether GVfs lists a mount at `mount_point` (resolved, as the kernel reports it) in the sidebar, for a user `user`
/// whose `$HOME` is `home`, by GVfs's rule for a mount with no fstab entry and no `x-gvfs-show`.
fn gvfs_lists(mount_point: &Path, home: &Path, user: &str) -> bool {
    let path = mount_point.to_string_lossy();
    if path.contains("/.") {
        return false;
    }
    let home = home.to_string_lossy();
    if path.strip_prefix(home.as_ref()).is_some_and(|rest| rest.starts_with('/')) {
        return true;
    }
    let below_media = path.strip_prefix("/run").unwrap_or(&path).strip_prefix("/media/");
    below_media.is_some_and(|rest| rest.strip_prefix(user).is_some_and(|after| after.starts_with('/')) || !rest.contains('/'))
}

/// Whether GVfs lists the mount at `mount_point` in Files' sidebar, as a drive.
pub fn listed_in_sidebar(paths: &Paths, mount_point: &Path) -> bool {
    let user = std::env::var("USER").unwrap_or_default();
    gvfs_lists(&canonical_mount_point(mount_point), &gvfs_home(paths), &user)
}

/// GVfs's volume monitor: the user service that decides which mounts Files' sidebar lists.
const VOLUME_MONITOR: &str = "gvfs-udisks2-volume-monitor.service";

/// Our drop-in for the volume monitor's user service.
fn volume_monitor_drop_in(paths: &Paths) -> PathBuf {
    config_home(paths).join("systemd").join("user").join(format!("{VOLUME_MONITOR}.d")).join("immich-mount.conf")
}

/// The drop-in: the private fstab, and, where home is reached through a symlink, the resolved home.
fn drop_in_text(resolved_home: Option<&Path>, private_fstab: &Path) -> String {
    let mut text = String::from(
        "# Immich Mount: the volume monitor reads a copy of /etc/fstab where the daemon lists its mount, with its icon,\n\
         # while it is mounted.\n",
    );
    if resolved_home.is_some() {
        text.push_str("# GVfs compares mounts with $HOME as written, and this home is reached through a symlink.\n");
    }
    text.push_str("[Service]\n");
    if let Some(home) = resolved_home {
        text.push_str(&format!("Environment=HOME={}\n", home.display()));
    }
    text.push_str(&format!("Environment=LIBMOUNT_FSTAB={}\n", private_fstab.display()));
    text
}

/// The volume monitor's copy of `/etc/fstab`, with the mount's line while it is mounted: the daemon's state.
fn private_fstab(paths: &Paths) -> PathBuf {
    paths.state_directory.join("gvfs-fstab")
}

/// A path as fstab writes it: spaces, tabs, newlines and backslashes escaped in octal.
fn fstab_path(path: &Path) -> String {
    let mut written = String::new();
    for character in path.to_string_lossy().chars() {
        match character {
            ' ' => written.push_str("\\040"),
            '\t' => written.push_str("\\011"),
            '\n' => written.push_str("\\012"),
            '\\' => written.push_str("\\134"),
            other => written.push(other),
        }
    }
    written
}

/// The mount's fstab line: shown in the sidebar (`x-gvfs-show`, on its resolved path), with the drive's name and our
/// icons, and never mounted from fstab (`noauto`).
fn fstab_line(mount_point: &Path, drive_name: &str) -> String {
    format!(
        "immich {} fuse.immich noauto,x-gvfs-show,x-gvfs-name={},x-gvfs-icon=dev.hviid.ImmichMount,x-gvfs-symbolic-icon=dev.hviid.ImmichMount-symbolic 0 0\n",
        fstab_path(&canonical_mount_point(mount_point)),
        fstab_path(Path::new(drive_name)).replace(',', "\\054"),
    )
}

/// Write the volume monitor's fstab: the system's, as it is now, and the mount's line while `mounted` names it. Only
/// where the drop-in points the monitor at it. A line left by a daemon that stopped without its unmount (killed,
/// crashed) is gone at the next start, which writes it without.
pub fn list_mount_in_fstab(paths: &Paths, mounted: Option<(&Path, &str)>) -> Result<(), String> {
    let drop_in = std::fs::read_to_string(volume_monitor_drop_in(paths)).unwrap_or_default();
    let file = private_fstab(paths);
    if !drop_in.contains(&format!("LIBMOUNT_FSTAB={}", file.display())) {
        return Ok(());
    }
    let mut text = std::fs::read_to_string("/etc/fstab").unwrap_or_default();
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    if let Some((mount_point, drive_name)) = mounted {
        text.push_str("# Immich Mount, while it is mounted (the drive's icon in Files' sidebar)\n");
        text.push_str(&fstab_line(mount_point, drive_name));
    }
    let describe = |error: std::io::Error| format!("{}: {error}", file.display());
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).map_err(describe)?;
    }
    // GLib watches the file, so it is replaced whole, never seen half-written.
    let temporary = file.with_extension("tmp");
    std::fs::write(&temporary, text).map_err(describe)?;
    std::fs::rename(&temporary, &file).map_err(describe)
}

/// The mount point's folder icon in Files, ours instead of a plain folder: GNOME's per-file metadata, kept by
/// gvfsd-metadata for the folder as it is seen mounted and unmounted. Through `gio`, where GNOME has it; nothing
/// elsewhere.
pub fn set_folder_icon(mount_point: &Path) {
    let outcome = std::process::Command::new("gio")
        .args(["set", "-t", "string"])
        .arg(mount_point)
        .args(["metadata::custom-icon-name", "dev.hviid.ImmichMount"])
        .output();
    match outcome {
        Ok(output) if !output.status.success() => {
            log::debug!("the folder icon was not set: {}", String::from_utf8_lossy(&output.stderr).trim());
        }
        Ok(_) => {}
        Err(error) => log::debug!("the folder icon was not set (no gio): {error}"),
    }
}

/// The home the volume monitor compares with: the one our drop-in gives it, else `$HOME`.
fn gvfs_home(paths: &Paths) -> PathBuf {
    let text = std::fs::read_to_string(volume_monitor_drop_in(paths)).unwrap_or_default();
    text.lines().find_map(|line| line.strip_prefix("Environment=HOME=")).map(PathBuf::from).unwrap_or_else(|| paths.home.clone())
}

/// Give the volume monitor the private fstab and, where home is reached through a symlink, the resolved home, and
/// restart it, so it lists the mount as a drive with an eject button and our icon; the setup's bookmark of the mount
/// (titled with the drive's name) then goes, as the drive takes its place. Called by the daemon before it mounts.
/// `Ok(true)` when it changed something.
pub fn show_mounts_in_sidebar(paths: &Paths, mount_point: &Path) -> Result<bool, String> {
    if !crate::service::is_installed(VOLUME_MONITOR) {
        return Ok(false);
    }
    let resolved = std::fs::canonicalize(&paths.home).ok().filter(|resolved| *resolved != paths.home);
    let file = volume_monitor_drop_in(paths);
    let text = drop_in_text(resolved.as_deref(), &private_fstab(paths));
    if std::fs::read_to_string(&file).is_ok_and(|current| current == text) {
        return Ok(false);
    }
    let describe = |error: std::io::Error| format!("{}: {error}", file.display());
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).map_err(describe)?;
    }
    std::fs::write(&file, text).map_err(describe)?;
    for arguments in [["--user", "daemon-reload", ""], ["--user", "try-restart", VOLUME_MONITOR]] {
        let _ = std::process::Command::new("systemctl").args(arguments.iter().filter(|argument| !argument.is_empty())).output();
    }
    for name in mount_names() {
        remove_bookmark_titled(paths, mount_point, Some(name))?;
    }
    Ok(true)
}

/// Take our drop-in away again (at logout); the volume monitor goes back to `$HOME` and `/etc/fstab` at its next start.
pub fn remove_sidebar_drop_in(paths: &Paths) -> Result<bool, String> {
    let _ = std::fs::remove_file(private_fstab(paths));
    let file = volume_monitor_drop_in(paths);
    match std::fs::remove_file(&file) {
        Ok(()) => {
            let _ = std::fs::remove_dir(file.parent().unwrap_or(&file));
            let _ = std::process::Command::new("systemctl").args(["--user", "daemon-reload"]).output();
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("{}: {error}", file.display())),
    }
}

fn config_home(paths: &Paths) -> &Path {
    paths.config_directory.parent().unwrap_or(&paths.home)
}

/// The sidebar's bookmarks, which GTK keeps (Files' Ctrl+D writes here).
fn bookmarks(paths: &Paths) -> PathBuf {
    config_home(paths).join("gtk-3.0").join("bookmarks")
}

/// The URIs a bookmark of the mount may carry: the mount point as configured, and resolved.
fn uris(mount_point: &Path) -> Vec<String> {
    let mut uris = vec![file_uri(mount_point)];
    let resolved = file_uri(&canonical_mount_point(mount_point));
    if !uris.contains(&resolved) {
        uris.push(resolved);
    }
    uris
}

/// Whether the sidebar has a bookmark of the mount, ours or the user's own.
pub fn has_bookmark(paths: &Paths, mount_point: &Path) -> bool {
    let text = std::fs::read_to_string(bookmarks(paths)).unwrap_or_default();
    uris(mount_point).iter().any(|uri| bookmark_line(&text, uri).is_some())
}

/// Bookmark the mount in the sidebar, titled `title`, unless it has a bookmark already. Called once, by the GNOME
/// app's setup and only where GVfs does not list the mount, so a bookmark the user removed stays removed. `Ok(true)`
/// when it was added.
pub fn add_bookmark(paths: &Paths, mount_point: &Path, title: &str) -> Result<bool, String> {
    if has_bookmark(paths, mount_point) {
        return Ok(false);
    }
    let file = bookmarks(paths);
    let text = match std::fs::read_to_string(&file) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(format!("{}: {error}", file.display())),
    };
    write_bookmarks(&file, &with_bookmark(&text, &file_uri(mount_point), title)).map(|()| true)
}

/// Give the setup's bookmark of the mount the drive's name, `title`, after it changed. A bookmark titled anything but
/// one of the drive's names is the user's, and keeps its title. Called by the daemon as it starts. `Ok(true)` when it
/// changed something.
pub fn retitle_bookmark(paths: &Paths, mount_point: &Path, title: &str) -> Result<bool, String> {
    let file = bookmarks(paths);
    let Ok(text) = std::fs::read_to_string(&file) else { return Ok(false) };
    let updated = retitled(&text, &uris(mount_point), title);
    if updated == text {
        return Ok(false);
    }
    write_bookmarks(&file, &updated).map(|()| true)
}

/// Take the mount's bookmark out of the sidebar (at logout). Nothing to remove is not an error.
pub fn remove_bookmark(paths: &Paths, mount_point: &Path) -> Result<bool, String> {
    remove_bookmark_titled(paths, mount_point, None)
}

/// Take out the mount's bookmarks, or only the one titled `title` (the setup's, not one the user made).
fn remove_bookmark_titled(paths: &Paths, mount_point: &Path, title: Option<&str>) -> Result<bool, String> {
    let file = bookmarks(paths);
    let Ok(mut text) = std::fs::read_to_string(&file) else { return Ok(false) };
    let mut removed = false;
    for uri in uris(mount_point) {
        let bookmark = match title {
            Some(title) => format!("{uri} {title}"),
            None => uri,
        };
        if let Some(updated) = without_bookmark(&text, &bookmark) {
            text = updated;
            removed = true;
        }
    }
    if removed {
        write_bookmarks(&file, &text)?;
    }
    Ok(removed)
}

/// Replace the file whole (GTK watches it and reloads), so a reader never sees half of it.
fn write_bookmarks(file: &Path, text: &str) -> Result<(), String> {
    let describe = |error: std::io::Error| format!("{}: {error}", file.display());
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).map_err(describe)?;
    }
    let temporary = file.with_extension("immich-mount");
    std::fs::write(&temporary, text).map_err(describe)?;
    std::fs::rename(&temporary, file).map_err(describe)
}

/// The index of the line bookmarking `uri`: the URI, then optionally a space and its title. `uri` with a title
/// matches only that title.
fn bookmark_line(bookmarks: &str, uri: &str) -> Option<usize> {
    if uri.contains(' ') {
        return bookmarks.lines().position(|line| line == uri);
    }
    bookmarks.lines().position(|line| line.split(' ').next() == Some(uri))
}

/// `bookmarks` with one for `uri`, titled `title`, at the end.
fn with_bookmark(bookmarks: &str, uri: &str, title: &str) -> String {
    let separator = if bookmarks.is_empty() || bookmarks.ends_with('\n') { "" } else { "\n" };
    format!("{bookmarks}{separator}{uri} {title}\n")
}

/// `bookmarks` with each bookmark of `uris` titled by one of the drive's names titled `title` instead.
fn retitled(bookmarks: &str, uris: &[String], title: &str) -> String {
    bookmarks
        .split_inclusive('\n')
        .map(|line| {
            let entry = line.trim_end_matches('\n');
            let ours = uris.iter().find_map(|uri| {
                let old = entry.strip_prefix(uri.as_str())?.strip_prefix(' ')?;
                (old != title && mount_names().any(|name| name == old)).then_some(uri)
            });
            match ours {
                Some(uri) => format!("{uri} {title}{}", &line[entry.len()..]),
                None => line.to_owned(),
            }
        })
        .collect()
}

/// `bookmarks` without the one for `uri`, or `None` when it has none.
fn without_bookmark(bookmarks: &str, uri: &str) -> Option<String> {
    let index = bookmark_line(bookmarks, uri)?;
    Some(bookmarks.split_inclusive('\n').enumerate().filter(|(line, _)| *line != index).map(|(_, text)| text).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gvfs_lists_a_mount_in_home_only_where_home_is_the_real_folder() {
        let mount = Path::new("/var/home/me/Immich");
        assert!(gvfs_lists(mount, Path::new("/var/home/me"), "me"));
        // An atomic desktop: HOME=/home/me, while the kernel reports the resolved path.
        assert!(!gvfs_lists(mount, Path::new("/home/me"), "me"));
        assert!(!gvfs_lists(Path::new("/var/home/me/.hidden/Immich"), Path::new("/var/home/me"), "me"));
        assert!(!gvfs_lists(Path::new("/var/home/meadow/Immich"), Path::new("/var/home/me"), "me"));
        assert!(gvfs_lists(Path::new("/run/media/me/Immich"), Path::new("/home/me"), "me"));
        assert!(gvfs_lists(Path::new("/media/Immich"), Path::new("/home/me"), "me"));
        assert!(!gvfs_lists(Path::new("/run/user/1000/Immich"), Path::new("/home/me"), "me"));
    }

    #[test]
    fn a_bookmark_takes_the_drives_new_name_unless_the_user_titled_it() {
        let uris = ["file:///home/me/Immich".to_owned()];
        let ours = "file:///home/me/Documents\nfile:///home/me/Immich Immich\n";
        assert_eq!(retitled(ours, &uris, "Immich Drive"), "file:///home/me/Documents\nfile:///home/me/Immich Immich Drive\n");
        assert_eq!(retitled("file:///home/me/Immich Immich Drive", &uris, "Immich Mount"), "file:///home/me/Immich Immich Mount");
        let theirs = "file:///home/me/Immich Photos\nfile:///home/me/Immich\n";
        assert_eq!(retitled(theirs, &uris, "Immich Mount"), theirs);
    }

    #[test]
    fn a_bookmark_is_added_once_and_removed_with_its_line() {
        let uri = "file:///home/me/Immich";
        let mine = "file:///home/me/Projects Projects\nfile:///home/me/Music";
        let added = with_bookmark(mine, uri, "Immich");
        assert_eq!(added, "file:///home/me/Projects Projects\nfile:///home/me/Music\nfile:///home/me/Immich Immich\n");
        assert!(bookmark_line(&added, uri).is_some());
        assert_eq!(bookmark_line(&added, "file:///home/me/Immich2"), None);
        assert_eq!(without_bookmark(&added, uri).as_deref(), Some("file:///home/me/Projects Projects\nfile:///home/me/Music\n"));
        assert_eq!(without_bookmark(mine, uri), None);
        assert_eq!(with_bookmark("", uri, "Immich"), "file:///home/me/Immich Immich\n");
        // Only the setup's bookmark, by its title; one the user made without a title stays.
        let both = "file:///home/me/Immich\nfile:///home/me/Immich Immich\n";
        assert_eq!(without_bookmark(both, "file:///home/me/Immich Immich").as_deref(), Some("file:///home/me/Immich\n"));
    }

    #[test]
    fn the_drop_in_gives_the_volume_monitor_the_resolved_home_and_the_private_fstab() {
        let fstab = Path::new("/home/me/.local/state/immich-mount/gvfs-fstab");
        let text = drop_in_text(Some(Path::new("/var/home/me")), fstab);
        assert!(text.contains("[Service]\nEnvironment=HOME=/var/home/me\n"), "{text}");
        assert_eq!(text.lines().find_map(|line| line.strip_prefix("Environment=HOME=")), Some("/var/home/me"));
        assert!(text.ends_with("Environment=LIBMOUNT_FSTAB=/home/me/.local/state/immich-mount/gvfs-fstab\n"), "{text}");
        let plain = drop_in_text(None, fstab);
        assert!(!plain.contains("HOME="), "a home not reached through a symlink keeps its $HOME: {plain}");
    }

    #[test]
    fn the_fstab_line_shows_the_mount_with_its_name_and_icons_and_never_mounts_it() {
        let line = fstab_line(Path::new("/nonexistent/My Photos"), "Immich Mount");
        assert_eq!(
            line,
            "immich /nonexistent/My\\040Photos fuse.immich noauto,x-gvfs-show,x-gvfs-name=Immich\\040Mount,\
             x-gvfs-icon=dev.hviid.ImmichMount,x-gvfs-symbolic-icon=dev.hviid.ImmichMount-symbolic 0 0\n"
        );
        assert_eq!(line.split_whitespace().count(), 6, "six fields: {line}");
    }
}
