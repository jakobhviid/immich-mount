//! Where Immich Mount keeps its files on Linux (SPEC.md, "Files on disk"), following the XDG base directories.

use std::path::{Path, PathBuf};

/// The locations the daemon and the command line use.
#[derive(Debug, Clone)]
pub struct Paths {
    pub home: PathBuf,
    pub config_directory: PathBuf,
    pub cache_directory: PathBuf,
    pub state_directory: PathBuf,
    /// `~/.local/share/immich-mount`: what must outlive the caches, such as files that were not uploaded.
    pub data_directory: PathBuf,
    /// The freedesktop thumbnail cache. Always the host's own, never an app-private one.
    pub thumbnail_directory: PathBuf,
}

impl Paths {
    /// Resolve the locations from the environment.
    pub fn from_environment() -> Paths {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"));
        let from_xdg = |variable: &str, fallback: &str| {
            std::env::var_os(variable)
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .unwrap_or_else(|| home.join(fallback))
        };
        Paths {
            config_directory: from_xdg("XDG_CONFIG_HOME", ".config").join("immich-mount"),
            cache_directory: from_xdg("XDG_CACHE_HOME", ".cache").join("immich-mount"),
            state_directory: from_xdg("XDG_STATE_HOME", ".local/state").join("immich-mount"),
            data_directory: from_xdg("XDG_DATA_HOME", ".local/share").join("immich-mount"),
            thumbnail_directory: home.join(".cache").join("thumbnails"),
            home,
        }
    }

    pub fn config_file(&self) -> PathBuf {
        self.config_directory.join("config.toml")
    }

    /// The headless credential file, unless the config names another.
    pub fn not_uploaded_directory(&self) -> PathBuf {
        self.data_directory.join("not-uploaded")
    }

    pub fn default_credential_file(&self) -> PathBuf {
        self.config_directory.join("credential")
    }

    /// Expand a leading `~` in a configured path.
    pub fn expand(&self, configured: &str) -> PathBuf {
        match configured.strip_prefix("~/") {
            Some(rest) => self.home.join(rest),
            None if configured == "~" => self.home.clone(),
            None => PathBuf::from(configured),
        }
    }

    /// The cache directory of one account (`<account>` is the server's host and port, SPEC.md "Files on disk").
    pub fn account_cache(&self, account: &str) -> PathBuf {
        self.cache_directory.join(account)
    }

    /// Whether `path` lies under the home directory and outside hidden folders, where the file manager shows mounts.
    pub fn is_visible_under_home(&self, path: &Path) -> bool {
        match path.strip_prefix(&self.home) {
            Ok(relative) => relative.components().all(|component| !component.as_os_str().to_string_lossy().starts_with('.')),
            Err(_) => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths() -> Paths {
        let home = PathBuf::from("/home/me");
        Paths {
            config_directory: home.join(".config/immich-mount"),
            cache_directory: home.join(".cache/immich-mount"),
            state_directory: home.join(".local/state/immich-mount"),
            data_directory: home.join(".local/share/immich-mount"),
            thumbnail_directory: home.join(".cache/thumbnails"),
            home,
        }
    }

    #[test]
    fn a_leading_tilde_means_home() {
        assert_eq!(paths().expand("~/Immich"), PathBuf::from("/home/me/Immich"));
        assert_eq!(paths().expand("/mnt/photos"), PathBuf::from("/mnt/photos"));
    }

    #[test]
    fn only_visible_folders_under_home_show_in_the_sidebar() {
        assert!(paths().is_visible_under_home(Path::new("/home/me/Immich")));
        assert!(!paths().is_visible_under_home(Path::new("/home/me/.local/share/immich")));
        assert!(!paths().is_visible_under_home(Path::new("/run/user/1000/immich")));
    }
}
