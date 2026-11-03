//! The user service that runs the daemon (ARCHITECTURE.md, "The daemon's lifecycle"): the packages' own
//! `immich-mount.service`, or Homebrew's `homebrew.immich-mount.service`, which `brew services` writes.

/// The unit the .rpm, the .deb and the tarball install.
pub const PACKAGED: &str = "immich-mount.service";
/// Homebrew's unit, there once `brew services start immich-mount` has run.
pub const HOMEBREW: &str = "homebrew.immich-mount.service";

/// The unit to enable, start and restart: the packages' when it is installed, else Homebrew's when that is, else the
/// packages' name, so an error names the unit the packages provide.
pub fn user_service() -> &'static str {
    if installed(PACKAGED) {
        PACKAGED
    } else if installed(HOMEBREW) {
        HOMEBREW
    } else {
        PACKAGED
    }
}

/// Whether this binary came from Homebrew (it runs from a keg in the Cellar).
pub fn from_homebrew() -> bool {
    std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .is_ok_and(|executable| executable.to_string_lossy().contains("/Cellar/immich-mount/"))
}

fn installed(unit: &str) -> bool {
    std::process::Command::new("systemctl").args(["--user", "cat", "--", unit]).output().is_ok_and(|outcome| outcome.status.success())
}
