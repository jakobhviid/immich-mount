//! `immich-mount doctor`: a read-only health check (WORKFLOWS.md, "Doctor"). It changes nothing.

use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use serde::Serialize;

use crate::gnome;
use crate::kde;
use crate::mounts::{mount_state, MountState};
use crate::paths::Paths;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Ok,
    Warning,
    Problem,
    Skipped,
}

#[derive(Debug, Clone, Serialize)]
pub struct Check {
    pub name: &'static str,
    pub status: Status,
    pub detail: String,
    /// What to do about it, when it is not ok.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DoctorReport {
    pub checks: Vec<Check>,
    pub problems: usize,
    pub warnings: usize,
}

fn check(name: &'static str, status: Status, detail: impl Into<String>, fix: Option<&str>) -> Check {
    Check { name, status, detail: detail.into(), fix: fix.map(str::to_string) }
}

/// The local checks: everything that does not need the server.
/// Dolphin shows no previews on the mount until KDE allows previews of remote files (`kde.rs`). Only reported where
/// KDE is in use.
fn dolphin_previews(paths: &Paths) -> Option<Check> {
    let limit = kde::remote_preview_limit(paths);
    if !kde::is_running() && limit.is_none() {
        return None;
    }
    let limit = limit.unwrap_or(0);
    Some(if limit > 0 {
        check("dolphin-previews", Status::Ok, format!("Dolphin previews files up to {} MB on the mount", limit / 1_000_000), None)
    } else {
        check(
            "dolphin-previews",
            Status::Warning,
            "Dolphin shows no thumbnails on the mount: KDE counts it as remote, where previews are off",
            Some("kwriteconfig6 --file kdeglobals --group PreviewSettings --key MaximumRemoteSize 104857600"),
        )
    })
}

pub fn local_checks(paths: &Paths, mount_point: &Path) -> Vec<Check> {
    let mut checks = Vec::new();

    let fuse_device = std::fs::OpenOptions::new().read(true).write(true).open("/dev/fuse");
    checks.push(match fuse_device {
        Ok(_) => check("fuse", Status::Ok, "/dev/fuse is accessible", None),
        Err(error) => check("fuse", Status::Problem, format!("/dev/fuse: {error}"), Some("Install fuse3 and make sure /dev/fuse exists")),
    });

    checks.push(match find_in_path("fusermount3") {
        Some(helper) => {
            let setuid = std::fs::metadata(&helper).map(|metadata| metadata.permissions().mode() & 0o4000 != 0).unwrap_or(false);
            if setuid {
                check("fusermount3", Status::Ok, format!("{} is setuid", helper.display()), None)
            } else {
                check("fusermount3", Status::Problem, format!("{} is not setuid", helper.display()), Some("Reinstall the fuse3 package"))
            }
        }
        None => check("fusermount3", Status::Problem, "fusermount3 is not installed", Some("Install the fuse3 package")),
    });

    let timeout = std::fs::read_to_string("/proc/sys/fs/fuse/default_request_timeout").ok().and_then(|text| text.trim().parse::<u64>().ok());
    checks.push(match timeout {
        Some(seconds) if seconds > 0 => check(
            "kernel-timeout",
            Status::Warning,
            format!("fs.fuse.default_request_timeout is {seconds} s; a long upload could abort the mount"),
            Some("Set the sysctl fs.fuse.default_request_timeout to 0"),
        ),
        Some(_) => check("kernel-timeout", Status::Ok, "FUSE request timeouts are off", None),
        None => check("kernel-timeout", Status::Ok, "this kernel has no FUSE request timeout", None),
    });

    checks.push(match mount_state(mount_point) {
        MountState::Mounted => check("mount", Status::Ok, format!("mounted at {}", mount_point.display()), None),
        MountState::NotMounted => check(
            "mount",
            Status::Warning,
            format!("nothing is mounted at {}", mount_point.display()),
            Some("Start it with `systemctl --user start immich-mount`"),
        ),
        MountState::Stale => check(
            "mount",
            Status::Problem,
            format!("{} is a dead mount (the daemon is gone)", mount_point.display()),
            Some("Run `systemctl --user restart immich-mount`; it detaches the dead mount first"),
        ),
    });

    checks.push(sidebar(paths, mount_point));

    let thumbnails_writable = is_writable(&paths.thumbnail_directory) || paths.thumbnail_directory.parent().is_some_and(is_writable);
    checks.push(if thumbnails_writable {
        check("thumbnails", Status::Ok, format!("{} is writable", paths.thumbnail_directory.display()), None)
    } else {
        check(
            "thumbnails",
            Status::Problem,
            format!("{} is not writable, so files would be thumbnailed from originals", paths.thumbnail_directory.display()),
            Some("Fix the permissions of ~/.cache/thumbnails"),
        )
    });
    if let Some(dolphin) = dolphin_previews(paths) {
        checks.push(dolphin);
    }
    checks
}

/// Whether the file manager's sidebar shows the mount. On KDE the app's setup gives it a place in Dolphin; elsewhere
/// GVfs lists it by its path, except in a home reached through a symlink, where a bookmark stands in (`gnome.rs`).
fn sidebar(paths: &Paths, mount_point: &Path) -> Check {
    if !paths.is_visible_under_home(mount_point) {
        return check(
            "location",
            Status::Warning,
            format!("{} is outside home or hidden, so the sidebar will not show it", mount_point.display()),
            Some("Use a visible folder in your home, such as ~/Immich"),
        );
    }
    if kde::is_running() || gnome::listed_in_sidebar(paths, mount_point) {
        return check("location", Status::Ok, "the mount point shows in the file manager's sidebar", None);
    }
    let resolved = std::fs::canonicalize(&paths.home).unwrap_or_else(|_| paths.home.clone());
    let reason = format!(
        "GVfs lists no mount in a home reached through a symlink ({} is {})",
        paths.home.display(),
        resolved.display()
    );
    if gnome::has_bookmark(paths, mount_point) {
        check("location", Status::Ok, format!("Files' sidebar has the mount as a bookmark: {reason}"), None)
    } else {
        check(
            "location",
            Status::Warning,
            format!("Files' sidebar will not show the mount: {reason}"),
            Some("Open the mount in Files and press Ctrl+D to bookmark it"),
        )
    }
}

/// Put checks together into a report, counting what needs attention.
pub fn report(checks: Vec<Check>) -> DoctorReport {
    let problems = checks.iter().filter(|check| check.status == Status::Problem).count();
    let warnings = checks.iter().filter(|check| check.status == Status::Warning).count();
    DoctorReport { checks, problems, warnings }
}

fn find_in_path(program: &str) -> Option<std::path::PathBuf> {
    let search_path = std::env::var_os("PATH")?;
    std::env::split_paths(&search_path).map(|directory| directory.join(program)).find(|candidate| candidate.is_file())
}

fn is_writable(path: &Path) -> bool {
    let Ok(text) = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()) else {
        return false;
    };
    unsafe { libc::access(text.as_ptr(), libc::W_OK) == 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_report_counts_problems_and_warnings() {
        let checks = vec![
            check("a", Status::Ok, "fine", None),
            check("b", Status::Warning, "hm", Some("do x")),
            check("c", Status::Problem, "bad", Some("do y")),
        ];
        let report = report(checks);
        assert_eq!((report.problems, report.warnings), (1, 1));
    }
}

