//! `immich-mount app`: open the desktop app in the interface native to the running desktop (ARCHITECTURE.md, "One
//! app, native on each desktop"). Kirigami on KDE Plasma, GTK everywhere else; on Plasma the GTK one stands in only
//! when the Kirigami one cannot load, since it is tied to the Qt it was built against. Each interface answers
//! `--probe` before it touches its toolkit, so one whose libraries do not load here is passed over.

use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const GNOME: &str = "immich-mount-gnome";
const KDE: &str = "immich-mount-kde";

/// Whether there is a display to open a window on.
pub fn has_display() -> bool {
    ["WAYLAND_DISPLAY", "DISPLAY"].iter().any(|variable| std::env::var_os(variable).is_some_and(|value| !value.is_empty()))
}

/// The interfaces to try, in order, on a desktop named like `XDG_CURRENT_DESKTOP` (`KDE`, `GNOME`, `ubuntu:GNOME`).
fn interfaces(desktop: &str) -> &'static [&'static str] {
    if desktop.split(':').any(|name| name.eq_ignore_ascii_case("KDE")) {
        &[KDE, GNOME]
    } else {
        &[GNOME]
    }
}

/// Replace this process with the app, handing it `arguments` (a sign-in redirect, from the scheme handler). Returns
/// only when no interface can run here.
pub fn open(arguments: &[String]) -> anyhow::Error {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    let wanted = interfaces(&desktop);
    for name in wanted {
        let Some(program) = immich_mount_linux::programs::find(name) else { continue };
        if !loads(&program) {
            eprintln!("{} does not load on this system; trying the next interface", program.display());
            continue;
        }
        let error = Command::new(&program).args(arguments).exec();
        return anyhow::anyhow!("could not start {}: {error}", program.display());
    }
    anyhow::anyhow!(
        "no app for this desktop is installed (looked for {}); install it, or use the command line: immich-mount --help",
        wanted.join(" and ")
    )
}

/// Whether `program` loads here: `--probe` exits at once, before any window, unless its libraries are missing.
fn loads(program: &Path) -> bool {
    let Ok(mut child) = Command::new(program).arg("--probe").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()
    else {
        return false;
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            // Still running: a build that ignores `--probe` and opened a window; it loads.
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plasma_gets_kirigami_first_and_every_other_desktop_gtk_only() {
        assert_eq!(interfaces("KDE"), [KDE, GNOME]);
        assert_eq!(interfaces("GNOME"), [GNOME]);
        assert_eq!(interfaces("ubuntu:GNOME"), [GNOME]);
        assert_eq!(interfaces("X-Cinnamon"), [GNOME]);
        assert_eq!(interfaces(""), [GNOME]);
    }
}
