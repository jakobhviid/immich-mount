//! The look at the download page for a newer release (ARCHITECTURE.md, "Updates on Linux"), and which installs make
//! it: packages from the download page, which nothing else updates. Homebrew updates its own, a Flatpak's remote
//! would update it, and a development build has no release to compare (`immich_mount_linux::updates`).

use std::path::Path;
use std::time::Duration;

use immich_mount_core::settings::Config;
use immich_mount_linux::updates::{Check, Release};

/// Built by the download page for its release (site/build.py).
const LATEST: &str = "https://jakob.hviid.phd/immich-mount/latest.json";

#[derive(serde::Deserialize)]
struct Latest {
    version: String,
    page: String,
}

/// Whether this install looks: not with `updates.check` off, under Homebrew, or in a Flatpak.
pub fn wanted(config: &Config) -> bool {
    if !config.update_check || std::env::var_os("FLATPAK_ID").is_some() {
        return false;
    }
    let program = std::env::current_exe().and_then(|program| program.canonicalize()).unwrap_or_default();
    !installed_by_homebrew(&program)
}

/// A program in Homebrew's cellar or a cask's folder, which `brew upgrade` keeps current.
fn installed_by_homebrew(program: &Path) -> bool {
    let text = program.to_string_lossy();
    ["/Caskroom/", "/Cellar/", "/.linuxbrew/"].iter().any(|part| text.contains(part))
}

/// One look, as the daemon makes it once a day. Nothing is sent but the request and its User-Agent.
pub fn check() -> Check {
    Box::new(|| Box::pin(latest()))
}

async fn latest() -> Option<Release> {
    let client = reqwest::Client::builder()
        .user_agent(concat!("immich-mount/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(30))
        .build()
        .ok()?;
    let response = client.get(LATEST).send().await.ok()?.error_for_status().ok()?;
    let latest: Latest = response.json().await.ok()?;
    Some(Release { version: latest.version, page: latest.page })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn homebrew_installs_are_left_to_homebrew() {
        assert!(installed_by_homebrew(Path::new("/home/linuxbrew/.linuxbrew/Caskroom/immich-mount/2.19.1/immich-mount")));
        assert!(installed_by_homebrew(Path::new("/home/linuxbrew/.linuxbrew/Cellar/immich-mount-headless/2.19.1/bin/immich-mount")));
        assert!(!installed_by_homebrew(Path::new("/usr/bin/immich-mount")));
        assert!(!installed_by_homebrew(Path::new("/home/me/.local/bin/immich-mount")));
    }

    #[test]
    fn the_setting_turns_it_off() {
        let off = Config::from_toml("[updates]\ncheck = false\n").expect("the settings parse");
        assert!(!wanted(&off));
    }
}
