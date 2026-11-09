//! Folders copied into Upload, on Linux (ARCHITECTURE.md, "Folders copied into Upload"). The core follows each copy
//! (`immich_mount_core::folder_uploads`), fed by the filesystem as files arrive and upload; this moves it along, asks
//! its question as a desktop notification with two buttons, and makes the albums.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use immich_mount_core::folder_uploads::{make_albums, AlbumTarget, FolderChoice, FolderEvent, FolderQuestion, FolderUploads};
use immich_mount_core::source::SourceError;

use crate::fuse::seconds_now;
use crate::i18n::tr;
use crate::staging::Staging;

/// How often the copies are looked at: a settled copy is asked about within this of settling.
const TICK: Duration = Duration::from_secs(2);
/// The buttons' keys.
const MAKE_ALBUMS: &str = "albums";
const TIMELINE_ONLY: &str = "timeline";
/// How many album names a notification lists; the rest are an ellipsis.
const NAMES_SHOWN: usize = 4;

/// Follow the folders copied into Upload for as long as the daemon runs. `choice` is `uploads.folders`; asking needs a
/// desktop whose notifications carry buttons, and without one a copy stays in the timeline only.
pub async fn follow<Albums>(folders: Arc<Mutex<FolderUploads>>, staging: Arc<Staging>, albums: Arc<Albums>, choice: FolderChoice)
where
    Albums: AlbumTarget + 'static,
{
    // The desktop sends a button's answer to the connection that showed it, so one connection does both.
    let connection = match zbus::Connection::session().await {
        Ok(connection) if crate::notify::has_buttons(&connection).await => Some(connection),
        _ => None,
    };
    let (sender, mut answers) = tokio::sync::mpsc::unbounded_channel();
    if let Some(connection) = connection.clone() {
        tokio::spawn(crate::notify::answers(connection, sender));
    }
    // Each question's notification, and the copy it asks about.
    let mut asked: HashMap<u32, u64> = HashMap::new();
    loop {
        tokio::time::sleep(TICK).await;
        while let Ok((notification, answer)) = answers.try_recv() {
            let Some(copy) = asked.remove(&notification) else { continue };
            let wanted = answer.as_deref() == Some(MAKE_ALBUMS);
            if folders.lock().expect("folder uploads lock").answer(copy, wanted) {
                log::info!("a folder copied into Upload: {}", if wanted { "albums wanted" } else { "the timeline only" });
            }
        }
        let events = folders.lock().expect("folder uploads lock").tick(seconds_now(), choice, |top| staging.busy_under(top));
        for event in events {
            match event {
                FolderEvent::Ask(question) => match &connection {
                    Some(connection) => match ask(connection, &question).await {
                        Some(notification) => {
                            asked.insert(notification, question.id);
                        }
                        None => {
                            folders.lock().expect("folder uploads lock").answer(question.id, false);
                        }
                    },
                    None => {
                        log::info!("{} was copied into Upload; with no desktop to ask, it stays in the timeline only", question.folder);
                        folders.lock().expect("folder uploads lock").answer(question.id, false);
                    }
                },
                FolderEvent::MakeAlbums(copy) => make(&folders, albums.as_ref(), copy).await,
                FolderEvent::Unanswered(copy) => {
                    asked.retain(|_, asked_about| *asked_about != copy);
                    log::info!("a question about a folder copied into Upload went unanswered; it stays in the timeline only");
                }
            }
        }
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
    crate::notify::ask(connection, &summary, &lines.join("\n"), &[(MAKE_ALBUMS, tr("Make Albums")), (TIMELINE_ONLY, tr("Timeline Only"))]).await
}

/// Make a copy's albums, and say what came of it.
async fn make<Albums: AlbumTarget>(folders: &Mutex<FolderUploads>, albums: &Albums, copy: u64) {
    let Some(work) = folders.lock().expect("folder uploads lock").album_work(copy) else { return };
    let (made, failure) = make_albums(albums, &work, chrono::Local::now().date_naive()).await;
    folders.lock().expect("folder uploads lock").albums_made(copy, &made, failure.as_ref(), seconds_now());
    let new: Vec<String> = made.iter().filter_map(|album| album.name.clone()).collect();
    if !new.is_empty() {
        log::info!("made {} albums from {}", new.len(), work.folder);
        crate::notify::notify(&tr("Albums made from {folder}").replace("{folder}", &work.folder), &names(&new)).await;
    }
    let not_made = tr("No albums made from {folder}").replace("{folder}", &work.folder);
    match failure {
        None => {}
        Some(SourceError::MissingPermission(permission)) => {
            log::warn!("no albums from {}: the credential lacks {}", work.folder, permission.as_deref().unwrap_or("a permission"));
            let body = tr("Immich does not let this account make albums ({permission}). The photos and videos are in the timeline.")
                .replace("{permission}", permission.as_deref().unwrap_or("album.create"));
            crate::notify::notify(&not_made, &body).await;
        }
        Some(SourceError::SignedOut) => {
            crate::notify::notify(&not_made, tr("Signed out of Immich. The photos and videos are in the timeline.")).await;
        }
        Some(error) => log::info!("albums from {} wait: {error}; tried again in a minute", work.folder),
    }
}
