//! Folders copied into Upload, on Linux (ARCHITECTURE.md, "Folders copied into Upload"). The core follows each copy
//! (`immich_mount_core::folder_uploads`), fed by the filesystem as files arrive and upload; this moves it along, asks
//! its question as a desktop notification with two buttons, and makes the albums. The record is kept in the account's
//! cache, so a daemon restarted by a settings change asks again what it was asking.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use immich_mount_core::folder_uploads::{
    make_albums, tried_again, AlbumProgress, AlbumTarget, FolderChoice, FolderEvent, FolderQuestion, FolderUploads,
};
use immich_mount_core::source::SourceError;

use crate::fuse::seconds_now;
use crate::i18n::tr;
use crate::staging::Staging;

/// How often the copies are looked at: a settled copy is asked about within this of settling.
const TICK: Duration = Duration::from_secs(2);
/// The buttons' keys. `default` is a click on the notification itself, which answers nothing.
const MAKE_ALBUMS: &str = "albums";
const TIMELINE_ONLY: &str = "timeline";
const DEFAULT_ACTION: &str = "default";
/// How many album names a notification lists; the rest are an ellipsis.
const NAMES_SHOWN: usize = 4;

/// What is kept in the account's cache: the copies, and the notifications asking about them, to close once the
/// daemon that showed them is gone.
#[derive(Default, serde::Serialize, serde::Deserialize)]
struct Kept {
    copies: FolderUploads,
    #[serde(default)]
    notifications: Vec<u32>,
}

/// The record of the copies, from the account's cache, or a new one numbered from now.
pub fn kept_copies(file: &Path) -> FolderUploads {
    match std::fs::read_to_string(file).ok().and_then(|text| serde_json::from_str::<Kept>(&text).ok()) {
        Some(kept) => kept.copies,
        None => FolderUploads::numbered_from(seconds_now()),
    }
}

/// The desktop's notifications, reached once a question is due: at login the daemon may start before the desktop
/// serves notifications.
#[derive(Default)]
struct Desktop {
    connection: Option<zbus::Connection>,
    answers: Option<tokio::sync::mpsc::UnboundedReceiver<crate::notify::Answer>>,
}

impl Desktop {
    /// The connection to ask on, made (and its answers heard) the first time a desktop with buttons is there.
    async fn connection(&mut self) -> Option<&zbus::Connection> {
        if self.connection.is_none() {
            let connection = zbus::Connection::session().await.ok()?;
            if !crate::notify::has_buttons(&connection).await {
                return None;
            }
            let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
            // The desktop sends a button's answer to the connection that showed it, so one connection does both.
            tokio::spawn(crate::notify::answers(connection.clone(), sender));
            self.connection = Some(connection);
            self.answers = Some(receiver);
        }
        self.connection.as_ref()
    }
}

/// Follow the folders copied into Upload for as long as the daemon runs, keeping the record in `file`. `choice` is
/// `uploads.folders`; asking needs a desktop whose notifications carry buttons, and without one a copy stays in the
/// timeline only.
pub async fn follow<Albums>(folders: Arc<Mutex<FolderUploads>>, file: PathBuf, staging: Arc<Staging>, albums: Arc<Albums>, choice: FolderChoice)
where
    Albums: AlbumTarget + 'static,
{
    let mut desktop = Desktop::default();
    // Each question's notification, and the copy it asks about.
    let mut asked: HashMap<u32, u64> = HashMap::new();
    let mut kept_text = String::new();
    // Questions a daemon before this one asked: its notifications go, and they are asked again.
    let earlier: Vec<u32> = std::fs::read_to_string(&file).ok().and_then(|text| serde_json::from_str::<Kept>(&text).ok()).map(|kept| kept.notifications).unwrap_or_default();
    let mut pending: Vec<FolderQuestion> = folders.lock().expect("folder uploads lock").questions();
    if !earlier.is_empty() {
        if let Some(connection) = desktop.connection().await {
            for notification in earlier {
                crate::notify::close(connection, notification).await;
            }
        }
    }
    loop {
        tokio::time::sleep(TICK).await;
        let mut answered = Vec::new();
        if let Some(answers) = desktop.answers.as_mut() {
            while let Ok(answer) = answers.try_recv() {
                answered.push(answer);
            }
        }
        for (notification, answer) in answered {
            let Some(copy) = asked.remove(&notification) else { continue };
            match answer.as_deref() {
                // A click on the notification itself: asked again, since it answered nothing.
                Some(DEFAULT_ACTION) => pending.extend(folders.lock().expect("folder uploads lock").question(copy)),
                answer => {
                    let wanted = answer == Some(MAKE_ALBUMS);
                    if folders.lock().expect("folder uploads lock").answer(copy, wanted) {
                        log::info!("a folder copied into Upload: {}", if wanted { "albums wanted" } else { "the timeline only" });
                    }
                }
            }
        }
        let events = folders.lock().expect("folder uploads lock").tick(seconds_now(), choice, |top| staging.busy_under(top));
        for event in events {
            match event {
                FolderEvent::Ask(question) => pending.push(question),
                FolderEvent::MakeAlbums(copy) => make(&folders, albums.as_ref(), copy).await,
                FolderEvent::Unanswered(copy) => {
                    asked.retain(|_, asked_about| *asked_about != copy);
                    log::info!("a question about a folder copied into Upload went unanswered; it stays in the timeline only");
                }
            }
        }
        for question in pending.drain(..) {
            let shown = match desktop.connection().await {
                Some(connection) => ask(connection, &question).await,
                None => None,
            };
            match shown {
                Some(notification) => {
                    asked.insert(notification, question.id);
                }
                None => {
                    log::info!("{} was copied into Upload; with nobody to ask, it stays in the timeline only", question.folder);
                    folders.lock().expect("folder uploads lock").answer(question.id, false);
                }
            }
        }
        // Kept when it changed, so a restarted daemon asks again what this one asked.
        let kept = Kept { copies: folders.lock().expect("folder uploads lock").clone(), notifications: asked.keys().copied().collect() };
        let text = serde_json::to_string(&kept).unwrap_or_default();
        if text != kept_text {
            write_whole(&file, &text);
            kept_text = text;
        }
    }
}

/// Write a file whole, through a temporary one beside it.
fn write_whole(file: &Path, text: &str) {
    let temporary = file.with_extension("partial");
    if let Err(error) = std::fs::write(&temporary, text).and_then(|()| std::fs::rename(&temporary, file)) {
        log::debug!("could not keep the folders copied into Upload in {}: {error}", file.display());
    }
}

/// Names for a notification: the first few, then an ellipsis.
fn names(names: &[String]) -> String {
    let mut shown: Vec<&str> = names.iter().take(NAMES_SHOWN).map(String::as_str).collect();
    if names.len() > NAMES_SHOWN {
        shown.push("…");
    }
    shown.join(", ")
}

/// Ask whether a copy's folders become albums too.
async fn ask(connection: &zbus::Connection, question: &FolderQuestion) -> Option<u32> {
    let summary = tr("Uploaded to Immich: {folder}").replace("{folder}", &question.folder);
    let mut lines = vec![
        tr("Photos and videos in the timeline: {count}").replace("{count}", &question.uploaded.to_string()),
        tr("Albums it can make: {albums}").replace("{albums}", &names(&question.albums)),
    ];
    if question.not_uploaded > 0 {
        lines.push(tr("Files not uploaded: {count}").replace("{count}", &question.not_uploaded.to_string()));
    }
    // GNOME shows a notification's body on one line, line breaks as spaces, so the parts are set apart.
    let buttons = [(DEFAULT_ACTION, ""), (MAKE_ALBUMS, tr("Make Albums")), (TIMELINE_ONLY, tr("Timeline Only"))];
    crate::notify::ask(connection, &summary, &lines.join(" · "), &buttons).await
}

/// Make a copy's albums, recording each step as it happens, and say what came of it.
async fn make<Albums: AlbumTarget>(folders: &Mutex<FolderUploads>, albums: &Albums, copy: u64) {
    let Some(work) = folders.lock().expect("folder uploads lock").album_work(copy) else { return };
    let mut new = Vec::new();
    let outcome = make_albums(albums, &work, chrono::Local::now().date_naive(), |step| {
        if let AlbumProgress::Made { name: Some(name), .. } = &step {
            new.push(name.clone());
        }
        folders.lock().expect("folder uploads lock").album_progress(copy, &step);
    })
    .await;
    let failure = outcome.err();
    folders.lock().expect("folder uploads lock").albums_done(copy, failure.as_ref(), seconds_now());
    if !new.is_empty() {
        log::info!("made {} albums from {}", new.len(), work.folder);
        crate::notify::notify(&tr("Albums made from {folder}").replace("{folder}", &work.folder), &names(&new)).await;
    }
    let not_made = tr("No albums made from {folder}").replace("{folder}", &work.folder);
    match failure {
        None => {}
        Some(error) if tried_again(&error) => log::info!("albums from {} wait: {error}; tried again in a minute", work.folder),
        Some(SourceError::MissingPermission(permission)) => {
            log::warn!("no albums from {}: the credential lacks {}", work.folder, permission.as_deref().unwrap_or("a permission"));
            let body = tr("Immich does not let this account make albums ({permission}). The photos and videos are in the timeline.")
                .replace("{permission}", permission.as_deref().unwrap_or("album.create"));
            crate::notify::notify(&not_made, &body).await;
        }
        Some(SourceError::SignedOut) => {
            crate::notify::notify(&not_made, tr("Signed out of Immich. The photos and videos are in the timeline.")).await;
        }
        Some(error) => {
            log::warn!("no albums from {}: {error}", work.folder);
            crate::notify::notify(&not_made, &error.to_string()).await;
        }
    }
}
