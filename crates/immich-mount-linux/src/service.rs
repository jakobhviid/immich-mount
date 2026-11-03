//! The user service that runs the daemon (ARCHITECTURE.md, "The daemon's lifecycle"): the packages' own
//! `immich-mount.service`, or Homebrew's `homebrew.immich-mount-headless.service`, which `brew services` writes.

/// The unit the .rpm, the .deb and the tarball install.
pub const PACKAGED: &str = "immich-mount.service";
/// Homebrew's unit for the headless formula, there once `brew services start immich-mount-headless` has run.
pub const HOMEBREW: &str = "homebrew.immich-mount-headless.service";

use std::path::Path;

use crate::paths::Paths;

/// The installed unit, if any: the packages' first, else Homebrew's.
pub fn installed_user_service() -> Option<&'static str> {
    [PACKAGED, HOMEBREW].into_iter().find(|unit| installed(unit))
}

/// The unit to enable, start and restart: the installed one, else the packages' name, so an error names the unit the
/// packages provide.
pub fn user_service() -> &'static str {
    installed_user_service().unwrap_or(PACKAGED)
}

/// The packages' unit, running `daemon`.
pub fn unit_text(daemon: &Path) -> String {
    include_str!("../../../packaging/systemd/immich-mount.service.in")
        .replace("@BINDIR@/immich-mount", &format!("\"{}\"", daemon.display()))
}

/// The installed unit, else the packages' unit written for the daemon found on this system: what the app and `mount`
/// start, so an install that brought no unit (Homebrew, a build from source) mounts all the same.
pub fn ensure_user_service(paths: &Paths) -> std::io::Result<&'static str> {
    if let Some(unit) = installed_user_service() {
        return Ok(unit);
    }
    let daemon = crate::programs::find("immich-mount")
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "immich-mount is not installed"))?;
    write_user_service(paths, &daemon)
}

/// Write the packages' unit into the user's own systemd folder, running `daemon`, and have systemd read it. Returns the
/// unit's name.
pub fn write_user_service(paths: &Paths, daemon: &Path) -> std::io::Result<&'static str> {
    let config_home = paths.config_directory.parent().unwrap_or(&paths.home);
    let directory = config_home.join("systemd").join("user");
    std::fs::create_dir_all(&directory)?;
    std::fs::write(directory.join(PACKAGED), unit_text(daemon))?;
    let _ = std::process::Command::new("systemctl").args(["--user", "daemon-reload"]).output();
    Ok(PACKAGED)
}

/// The user service this process runs in, from its control group (`.../app.slice/immich-mount.service`), so the
/// daemon turns off the unit that started it and no other. `None` outside a service.
pub fn own_unit() -> Option<String> {
    unit_of_cgroup(&std::fs::read_to_string("/proc/self/cgroup").ok()?)
}

fn unit_of_cgroup(cgroup: &str) -> Option<String> {
    let last = cgroup.lines().last()?.rsplit('/').next()?;
    (last.ends_with(".service") && !last.starts_with("user@")).then(|| last.to_owned())
}

fn installed(unit: &str) -> bool {
    std::process::Command::new("systemctl").args(["--user", "cat", "--", unit]).output().is_ok_and(|outcome| outcome.status.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_unit_is_read_from_the_control_group() {
        let service = "0::/user.slice/user-1000.slice/user@1000.service/app.slice/immich-mount.service\n";
        assert_eq!(unit_of_cgroup(service).as_deref(), Some("immich-mount.service"));
        let terminal = "0::/user.slice/user-1000.slice/user@1000.service/app.slice/app-org.gnome.Terminal.slice/vte-spawn-1.scope\n";
        assert_eq!(unit_of_cgroup(terminal), None);
        assert_eq!(unit_of_cgroup("0::/user.slice/user-1000.slice/user@1000.service\n"), None);
    }

    #[test]
    fn the_written_unit_runs_the_daemon_it_was_given() {
        let unit = unit_text(Path::new("/home/linuxbrew/.linuxbrew/bin/immich-mount"));
        assert!(unit.contains("ExecStart=\"/home/linuxbrew/.linuxbrew/bin/immich-mount\" daemon"), "{unit}");
        assert!(unit.contains("Restart=on-failure"));
        assert!(unit.contains("RestartForceExitStatus=75"));
        assert!(!unit.contains('@'));
    }
}
