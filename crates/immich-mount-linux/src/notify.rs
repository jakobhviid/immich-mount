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
/// The app's launcher entry, which Plasma heads the notification with.
const KDE_DESKTOP_ENTRY: &str = "dev.hviid.ImmichMount";

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
    show(&connection, summary, body, &[], -1, false).await?;
    Ok(())
}

/// Show a notification on `connection`, with `actions` as buttons (key and label), and return its id. `expire` is in
/// milliseconds: -1 for the desktop's default, 0 for never. An `urgent` one stays on screen until it is answered or
/// dismissed, where a normal one slides into the desktop's list, or never shows while another is up (GNOME 50).
async fn show(
    connection: &zbus::Connection,
    summary: &str,
    body: &str,
    actions: &[(&str, &str)],
    expire: i32,
    urgent: bool,
) -> zbus::Result<u32> {
    // Plasma heads a notification with the app its `desktop-entry` names, so the app's name and icon show and no
    // picture is needed. GNOME drops a notification whose `desktop-entry` names an app it did not come from, and the
    // daemon is not the app, so there it carries our symbolic icon and no entry.
    let (icon, mut hints): (&str, HashMap<&str, zbus::zvariant::Value<'_>>) = if crate::kde::plasma_is_running().await {
        ("", HashMap::from([("desktop-entry", KDE_DESKTOP_ENTRY.into())]))
    } else {
        (ICON, HashMap::new())
    };
    if urgent {
        /// The specification's "critical" urgency.
        const CRITICAL: u8 = 2;
        hints.insert("urgency", CRITICAL.into());
    }
    let actions: Vec<&str> = actions.iter().flat_map(|(key, label)| [*key, *label]).collect();
    let reply = connection
        .call_method(
            Some("org.freedesktop.Notifications"),
            "/org/freedesktop/Notifications",
            Some("org.freedesktop.Notifications"),
            "Notify",
            &(APP_NAME, 0u32, icon, summary, body, actions, hints, expire),
        )
        .await?;
    reply.body().deserialize::<u32>()
}

/// Whether the desktop's notifications can carry buttons (GNOME's and Plasma's can).
pub async fn has_buttons(connection: &zbus::Connection) -> bool {
    let capabilities = async {
        let reply = connection
            .call_method(
                Some("org.freedesktop.Notifications"),
                "/org/freedesktop/Notifications",
                Some("org.freedesktop.Notifications"),
                "GetCapabilities",
                &(),
            )
            .await?;
        reply.body().deserialize::<Vec<String>>()
    };
    match tokio::time::timeout(SEND_LIMIT, capabilities).await {
        Ok(Ok(capabilities)) => capabilities.iter().any(|capability| capability == "actions"),
        _ => false,
    }
}

/// Ask a question with buttons on `connection`, which is where the desktop sends the answer (`ActionInvoked`), and
/// return the notification's id. It is urgent, so it stays on screen until answered or dismissed: a question in the
/// desktop's list was easy to miss (GNOME 50 put it there at once while another notification was up).
pub async fn ask(connection: &zbus::Connection, summary: &str, body: &str, buttons: &[(&str, &str)]) -> Option<u32> {
    match tokio::time::timeout(SEND_LIMIT, show(connection, summary, body, buttons, 0, true)).await {
        Ok(Ok(id)) => Some(id),
        Ok(Err(error)) => {
            log::info!("could not ask ({error}): {summary}");
            None
        }
        Err(_) => None,
    }
}

/// Close a notification shown earlier: a question a restarted daemon asks again.
pub async fn close(connection: &zbus::Connection, id: u32) {
    let body = (id,);
    let closing = connection.call_method(
        Some("org.freedesktop.Notifications"),
        "/org/freedesktop/Notifications",
        Some("org.freedesktop.Notifications"),
        "CloseNotification",
        &body,
    );
    if let Ok(Err(error)) = tokio::time::timeout(SEND_LIMIT, closing).await {
        log::debug!("could not close notification {id}: {error}");
    }
}

/// What became of a question: a button pressed (its key), or dismissed (`None`).
pub type Answer = (u32, Option<String>);

/// Pass on the answers to questions asked on `connection` until it closes: a button pressed, or a notification the
/// user dismissed. One that only expired, or was replaced, is no answer.
pub async fn answers(connection: zbus::Connection, sender: tokio::sync::mpsc::UnboundedSender<Answer>) {
    use futures_util::StreamExt;
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface("org.freedesktop.Notifications")
        .and_then(|rule| rule.path("/org/freedesktop/Notifications"))
        .map(|rule| rule.build());
    let stream = match rule {
        Ok(rule) => zbus::MessageStream::for_match_rule(rule, &connection, None).await,
        Err(error) => Err(error),
    };
    let mut stream = match stream {
        Ok(stream) => stream,
        Err(error) => {
            log::info!("cannot hear answers to notifications: {error}");
            return;
        }
    };
    /// `NotificationClosed`'s reason when the user dismissed it.
    const DISMISSED: u32 = 2;
    while let Some(Ok(message)) = stream.next().await {
        let header = message.header();
        let answer = match header.member().map(|member| member.as_str()) {
            Some("ActionInvoked") => message.body().deserialize::<(u32, String)>().ok().map(|(id, key)| (id, Some(key))),
            Some("NotificationClosed") => match message.body().deserialize::<(u32, u32)>() {
                Ok((id, DISMISSED)) => Some((id, None)),
                _ => None,
            },
            _ => None,
        };
        if let Some(answer) = answer {
            if sender.send(answer).is_err() {
                return;
            }
        }
    }
}
