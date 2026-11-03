//! Mount bookkeeping: is the mount there, is it a dead mount left by a crash, and how to take it down.
//!
//! Unmounting goes through `fusermount3`, the only host program the daemon needs (Flatpak readiness rules,
//! ARCHITECTURE.md). Every call has a timeout, so a stuck mount never hangs the command line.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// The state of a mount point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum MountState {
    /// An Immich mount is there and answers.
    Mounted,
    /// An Immich mount is listed, but its daemon is gone ("Transport endpoint is not connected").
    Stale,
    /// Nothing is mounted there.
    NotMounted,
}

/// The path as the mount table lists it: no trailing slash, and symlinks in its parent resolved (`/home` is a link
/// to `/var/home` on Fedora Atomic). Only the parent is resolved, so this never touches the mount itself, which may
/// be stale.
pub fn canonical_mount_point(path: &Path) -> PathBuf {
    let cleaned: PathBuf = path.components().collect();
    match (cleaned.parent(), cleaned.file_name()) {
        (Some(parent), Some(name)) => std::fs::canonicalize(parent).map(|parent| parent.join(name)).unwrap_or(cleaned),
        _ => cleaned,
    }
}

/// Whether an Immich FUSE mount is listed at `path` in this process's mount table.
pub fn is_listed(path: &Path) -> bool {
    let Ok(table) = std::fs::read_to_string("/proc/self/mountinfo") else {
        return false;
    };
    listed_in(&table, &canonical_mount_point(path).to_string_lossy())
}

/// Whether `mountinfo` lists an Immich mount at `mount_point`. Mounted through `fusermount3` it is `fuse.immich`;
/// mounted by root, fuser calls `mount(2)` itself and the type is plain `fuse`, with our source name `immich`
/// (lab/packaging, containers as root).
fn listed_in(mountinfo: &str, mount_point: &str) -> bool {
    let wanted = mount_point.replace(' ', "\\040");
    mountinfo.lines().any(|line| {
        let fields: Vec<&str> = line.split(' ').collect();
        let listed_point = fields.get(4).copied().unwrap_or_default();
        let mut after = line.split(" - ").nth(1).unwrap_or_default().split(' ');
        let (filesystem_type, source) = (after.next().unwrap_or_default(), after.next().unwrap_or_default());
        listed_point == wanted && (filesystem_type == "fuse.immich" || (filesystem_type == "fuse" && source == "immich"))
    })
}

/// The state of the mount at `path`.
pub fn mount_state(path: &Path) -> MountState {
    if !is_listed(path) {
        return MountState::NotMounted;
    }
    // statfs always reaches the daemon, where the kernel answers stat from the attributes it keeps for a minute: a
    // dead mount then looks alive, and a daemon restarted after a crash took it for another daemon's (seen on Bazzite).
    let Ok(path_text) = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()) else { return MountState::Mounted };
    let mut space = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    let failed = unsafe { libc::statvfs(path_text.as_ptr(), space.as_mut_ptr()) } != 0;
    if failed && std::io::Error::last_os_error().raw_os_error() == Some(libc::ENOTCONN) {
        MountState::Stale
    } else {
        MountState::Mounted
    }
}

/// Unmount through `fusermount3`; `lazy` detaches a stale mount even while something holds it.
pub fn unmount(path: &Path, lazy: bool) -> Result<(), String> {
    let mut command = Command::new("fusermount3");
    command.arg(if lazy { "-uz" } else { "-u" }).arg(path);
    run_with_timeout(command, Duration::from_secs(10))
}

/// Whether an unmount failed because something still uses the mount (a file manager showing it, an open file).
pub fn is_busy(message: &str) -> bool {
    message.contains("busy")
}

fn run_with_timeout(mut command: Command, timeout: Duration) -> Result<(), String> {
    let mut child = command
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|error| format!("could not run fusermount3: {error}"))?;
    let started = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => {
                let mut said = String::new();
                if let Some(mut stderr) = child.stderr.take() {
                    let _ = std::io::Read::read_to_string(&mut stderr, &mut said);
                }
                let reason = said.trim().rsplit(": ").next().unwrap_or_default().to_owned();
                return Err(if reason.is_empty() { format!("fusermount3 failed ({status})") } else { format!("fusermount3: {reason}") });
            }
            Ok(None) if started.elapsed() > timeout => {
                let _ = child.kill();
                return Err("fusermount3 did not finish in time".to_string());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(error) => return Err(error.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mount_by_fusermount3_and_one_by_root_both_count() {
        let by_user = "61 30 0:52 / /home/me/Immich rw,nosuid,nodev,relatime shared:1 - fuse.immich immich rw,user_id=1000\n";
        let by_root = "61 30 0:52 / /root/Immich rw,nosuid,nodev,noexec,relatime - fuse immich rw,user_id=0,group_id=0\n";
        let someone_else = "61 30 0:52 / /root/Immich rw,relatime - fuse sshfs rw,user_id=0\n";
        assert!(listed_in(by_user, "/home/me/Immich"));
        assert!(listed_in(by_root, "/root/Immich"));
        assert!(!listed_in(someone_else, "/root/Immich"), "another FUSE filesystem there is not ours");
        assert!(!listed_in(by_user, "/root/Immich"));
    }

    #[test]
    fn the_mount_point_is_compared_without_trailing_slashes_or_parent_links() {
        let directory = tempfile::tempdir().unwrap();
        let real = directory.path().join("real");
        std::fs::create_dir(&real).unwrap();
        std::os::unix::fs::symlink(&real, directory.path().join("link")).unwrap();
        let expected = std::fs::canonicalize(&real).unwrap().join("Immich");
        assert_eq!(canonical_mount_point(&directory.path().join("link/Immich/")), expected);
    }
}
