//! Telling of a newer release (ARCHITECTURE.md, "Updates on Linux"): for packages downloaded from the download page,
//! which no repository updates, the daemon looks once a day and says so once per version, in a notification and on
//! the tray's menu. The look itself is the caller's (`Check`), which has the HTTP client.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::time::Duration;

use crate::i18n::tr;

/// The newest release, as the download page names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: String,
    /// The page that offers it.
    pub page: String,
}

/// One look at the newest release; `None` when the page cannot be read.
pub type Check = Box<dyn Fn() -> Pin<Box<dyn Future<Output = Option<Release>> + Send>> + Send + Sync>;

const EVERY: Duration = Duration::from_secs(24 * 60 * 60);
/// The first look waits until the session has settled, so it never slows the login.
const FIRST: Duration = Duration::from_secs(5 * 60);

/// Whether `latest` is newer than `running`, both `major.minor.patch`. A development build (`0.0.0`) never is behind:
/// its version is not a release's.
pub fn is_newer(latest: &str, running: &str) -> bool {
    let parse = |text: &str| -> Option<(u64, u64, u64)> {
        let mut parts = text.trim().trim_start_matches('v').split('.').map(|part| part.parse::<u64>().ok());
        Some((parts.next()??, parts.next()??, parts.next()??))
    };
    match (parse(latest), parse(running)) {
        (Some(latest), Some(running)) => running != (0, 0, 0) && latest > running,
        _ => false,
    }
}

/// Look now and then once a day, for as long as the daemon runs. `told` keeps the last version told of, so each is
/// told once, across restarts.
pub async fn follow(check: Check, running: &'static str, connection: Option<zbus::Connection>, told: PathBuf) {
    tokio::time::sleep(FIRST).await;
    loop {
        if let Some(release) = check().await.filter(|release| is_newer(&release.version, running)) {
            log::info!("Immich Mount {} is out ({running} runs here)", release.version);
            if let Some(connection) = &connection {
                crate::tray::offer_update(connection, &release).await;
            }
            let already = std::fs::read_to_string(&told).ok().is_some_and(|version| version.trim() == release.version);
            if !already {
                crate::notify::notify_in_background(
                    tr("Immich Mount {version} is available").replace("{version}", &release.version),
                    tr("Get it from jakob.hviid.phd/immich-mount.").to_owned(),
                );
                if let Some(folder) = told.parent() {
                    let _ = std::fs::create_dir_all(folder);
                }
                let _ = std::fs::write(&told, &release.version);
            }
        }
        tokio::time::sleep(EVERY).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_by_their_numbers() {
        assert!(is_newer("2.20.0", "2.19.1"));
        assert!(is_newer("2.10.0", "2.9.9"), "numbers, not text");
        assert!(is_newer("v3.0.0", "2.99.0"));
        assert!(!is_newer("2.19.1", "2.19.1"));
        assert!(!is_newer("2.19.0", "2.19.1"));
        assert!(!is_newer("2.20.0", "0.0.0"), "a development build is never behind");
        assert!(!is_newer("not a version", "2.19.1"));
    }
}
