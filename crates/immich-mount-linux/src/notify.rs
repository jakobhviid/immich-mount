//! Desktop notifications, for what the user cannot otherwise see (ARCHITECTURE.md, "GNOME integration"): a refused
//! or failed upload, a long upload, and the mount stopping on a locked keyring.
//!
//! Sent over the session bus to `org.freedesktop.Notifications`, which GNOME, KDE and most desktops provide. Without
//! a session bus or a notification service the message is only logged, so a headless daemon behaves the same.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The name notifications appear under.
const APP_NAME: &str = "Immich Mount";
/// Our own symbolic icon, which the `immich-mount` package installs (ARCHITECTURE.md, "The icon"). Never the Immich
/// logo (AGENTS.md, "Hard rules").
const ICON: &str = "dev.hviid.ImmichMount-symbolic";
/// The KDE app's launcher entry, which Plasma heads the notification with.
const KDE_DESKTOP_ENTRY: &str = "dev.hviid.ImmichMount.Kde";

/// How long one kind of notification stays quiet after it was shown, so a folder of refused files gives one
/// notification, not one per file.
const QUIET_PERIOD: Duration = Duration::from_secs(20);

/// Show a notification unless one of the same `kind` was shown within the quiet period.
pub async fn notify_once_in_a_while(kind: &str, summary: &str, body: &str) {
    static LAST_SHOWN: Mutex<Option<HashMap<String, Instant>>> = Mutex::new(None);
    {
        let mut last_shown = LAST_SHOWN.lock().expect("notification lock");
        let shown = last_shown.get_or_insert_with(HashMap::new);
        let now = Instant::now();
        if shown.get(kind).is_some_and(|at| now.duration_since(*at) < QUIET_PERIOD) {
            return;
        }
        shown.insert(kind.to_owned(), now);
    }
    notify_in_background(summary.to_owned(), body.to_owned());
}

/// How long a notification may take to send: a stalled notification service must never hold anything up.
const SEND_LIMIT: Duration = Duration::from_secs(3);

/// Show a notification, waiting at most a few seconds. Failures are logged, never raised: a notification is a
/// courtesy, not part of an operation.
pub async fn notify(summary: &str, body: &str) {
    match tokio::time::timeout(SEND_LIMIT, send(summary, body)).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => log::debug!("no desktop notification ({error}): {summary}: {body}"),
        Err(_) => log::debug!("no desktop notification (timed out): {summary}: {body}"),
    }
}

/// Show a notification from a task of its own, so the caller (a file's close, the change poll) never waits on it.
pub fn notify_in_background(summary: String, body: String) {
    tokio::spawn(async move { notify(&summary, &body).await });
}

async fn send(summary: &str, body: &str) -> zbus::Result<()> {
    let connection = zbus::Connection::session().await?;
    // Plasma heads a notification with the app its `desktop-entry` names, so the KDE app's name and icon show and no
    // picture is needed. GNOME drops a notification whose `desktop-entry` names an app it did not come from, and the
    // daemon is not the app, so there it carries our symbolic icon and no entry.
    let (icon, hints): (&str, HashMap<&str, zbus::zvariant::Value<'_>>) = if crate::kde::plasma_is_running().await {
        ("", HashMap::from([("desktop-entry", KDE_DESKTOP_ENTRY.into())]))
    } else {
        (ICON, HashMap::new())
    };
    connection
        .call_method(
            Some("org.freedesktop.Notifications"),
            "/org/freedesktop/Notifications",
            Some("org.freedesktop.Notifications"),
            "Notify",
            &(APP_NAME, 0u32, icon, summary, body, Vec::<&str>::new(), hints, -1i32),
        )
        .await?;
    Ok(())
}
