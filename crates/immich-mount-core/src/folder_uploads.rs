//! Folders copied into Upload (SPEC.md, "Uploads"; ARCHITECTURE.md, "Folders copied into Upload").
//!
//! Each photo and video in a folder copied into Upload uploads to the timeline as it arrives, exactly as a single
//! file does: nothing here holds an upload back, so a move still never loses a photo (principle 6). What this keeps
//! is what albums need: each folder's uploads, by the folder's path below Upload. No platform sees a copy coming (a
//! file manager sends a folder one file at a time), so a copy counts as finished once it has settled: nothing new for
//! `SETTLE_SECONDS`, and nothing in it being written or uploaded. Then, as `uploads.folders` says, its folders become
//! albums, are asked about, or stay in the timeline only. The albums come after the uploads, since Immich adds an
//! Asset to an album at any time. The state is plain data, which the macOS edition keeps in its store: the system
//! stops its extension between requests.

use std::collections::HashSet;
use std::future::Future;
use std::sync::Arc;

use chrono::NaiveDate;

use crate::naming::clash_key;
use crate::source::SourceError;

/// How long a copy must be quiet, with nothing in it being written or uploaded, before it counts as finished.
pub const SETTLE_SECONDS: u64 = 10;
/// How long a question waits for its answer; unanswered, the copy stays in the timeline only.
pub const QUESTION_SECONDS: u64 = 24 * 60 * 60;
/// How long a finished copy is remembered: a folder of the same name copied in meanwhile joins it, as the rest of a
/// copy that paused.
pub const REMEMBER_SECONDS: u64 = 10 * 60;
/// How long album work that failed (the server unreachable) waits before it is tried again.
pub const RETRY_SECONDS: u64 = 60;
/// Between the folders of an album's name: `Trip › Day 1`. Unlike `/`, which file names cannot hold, it reads the
/// same in Immich and in the mount's Albums folder.
pub const SEPARATOR: &str = " › ";

/// What a folder copied into Upload becomes besides its timeline uploads (`uploads.folders`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderChoice {
    /// Ask once the copy has settled: albums, or the timeline only.
    Ask,
    /// The timeline only, as single files.
    Timeline,
    /// An album of each folder that holds photos or videos, named by its path.
    Albums,
}

impl FolderChoice {
    /// The choice a setting's word stands for; `ask` for any other.
    pub fn from_setting(word: &str) -> FolderChoice {
        match word {
            "timeline" => FolderChoice::Timeline,
            "albums" => FolderChoice::Albums,
            _ => FolderChoice::Ask,
        }
    }
}

/// The album a folder becomes, before any clash is settled: its path below Upload, the copied folder first.
pub fn album_name(path: &[String]) -> String {
    path.join(SEPARATOR)
}

/// The folders copied into Upload that are being followed.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FolderUploads {
    copies: Vec<FolderCopy>,
    next_id: u64,
}

/// One folder copied into Upload, with everything inside it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct FolderCopy {
    id: u64,
    /// The copied folder's name.
    top: String,
    /// It and the folders inside it, in the order they arrived.
    folders: Vec<CopiedFolder>,
    /// Seconds since the epoch.
    last_activity: u64,
    state: CopyState,
    /// Seconds since the epoch before which failed album work is not tried again.
    #[serde(default)]
    retry_after: u64,
    /// Albums are being made from it now. Not kept: work a stopped process cut short is taken up again.
    #[serde(skip)]
    making: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct CopiedFolder {
    /// Below Upload, the copied folder first.
    path: Vec<String>,
    /// The Assets uploaded directly in it, in order.
    assets: Vec<String>,
    /// Files in it that did not upload: refused, or failed.
    #[serde(default)]
    not_uploaded: u32,
    /// The album made of it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    album: Option<String>,
    /// How many of `assets` the album holds.
    #[serde(default)]
    in_album: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
enum CopyState {
    /// Still arriving.
    Copying,
    /// Settled, and asked about since then (seconds since the epoch).
    Asking { since: u64 },
    /// Albums wanted: made of each folder with uploads, and added to as more arrive.
    Albums,
    /// The timeline only.
    Timeline,
}

/// A copy that settled, to ask about.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FolderQuestion {
    pub id: u64,
    /// The copied folder's name.
    pub folder: String,
    /// Photos and videos uploaded from it.
    pub uploaded: u32,
    /// Files in it that did not upload.
    pub not_uploaded: u32,
    /// The albums it would make: one for each folder holding uploads, by `album_name`.
    pub albums: Vec<String>,
}

/// What a platform does next, from `FolderUploads::tick`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FolderEvent {
    /// A copy settled and `uploads.folders` asks: ask this, and pass the answer to `FolderUploads::answer`.
    Ask(FolderQuestion),
    /// Albums are wanted and work waits: take it with `FolderUploads::album_work`.
    MakeAlbums(u64),
    /// A question went unanswered for `QUESTION_SECONDS`; the copy stays in the timeline only.
    Unanswered(u64),
}

/// Albums to make or add to, taken from a copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlbumWork {
    pub id: u64,
    /// The copied folder's name.
    pub folder: String,
    pub items: Vec<AlbumItem>,
}

/// One folder's album work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlbumItem {
    pub path: Vec<String>,
    /// The album made of the folder before, to add to; `None` makes one.
    pub album: Option<String>,
    /// The Assets to put in it.
    pub assets: Vec<String>,
    /// How many of the folder's Assets the album holds once these are in.
    pub through: usize,
}

/// One folder's album, made or added to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MadeAlbum {
    pub path: Vec<String>,
    pub album_id: String,
    /// The name it was made with; `None` when it was added to, or was gone.
    pub name: Option<String>,
    pub through: usize,
}

impl FolderCopy {
    /// Whether it still takes arrivals under its folder's name, rather than a new copy starting.
    fn is_active(&self, now: u64) -> bool {
        match self.state {
            CopyState::Copying | CopyState::Asking { .. } => true,
            CopyState::Albums if self.has_album_work() => now < self.last_activity.saturating_add(QUESTION_SECONDS),
            CopyState::Albums | CopyState::Timeline => now < self.last_activity.saturating_add(REMEMBER_SECONDS),
        }
    }

    fn uploaded(&self) -> u32 {
        self.folders.iter().map(|folder| folder.assets.len() as u32).sum()
    }

    fn has_album_work(&self) -> bool {
        self.folders.iter().any(|folder| folder.assets.len() > folder.in_album)
    }

    fn folder(&mut self, path: &[String]) -> &mut CopiedFolder {
        let index = match self.folders.iter().position(|folder| folder.path == path) {
            Some(index) => index,
            None => {
                self.folders.push(CopiedFolder { path: path.to_vec(), assets: Vec::new(), not_uploaded: 0, album: None, in_album: 0 });
                self.folders.len() - 1
            }
        };
        &mut self.folders[index]
    }

    fn question(&self) -> FolderQuestion {
        FolderQuestion {
            id: self.id,
            folder: self.top.clone(),
            uploaded: self.uploaded(),
            not_uploaded: self.folders.iter().map(|folder| folder.not_uploaded).sum(),
            albums: self.folders.iter().filter(|folder| !folder.assets.is_empty()).map(|folder| album_name(&folder.path)).collect(),
        }
    }
}

impl FolderUploads {
    /// The copy that a folder under `top` belongs to: the one still taking arrivals under that name, or a new one.
    fn copy_for(&mut self, top: &str, now: u64) -> &mut FolderCopy {
        let index = match self.copies.iter().position(|copy| copy.top == top && copy.is_active(now)) {
            Some(index) => index,
            None => {
                self.next_id += 1;
                self.copies.push(FolderCopy {
                    id: self.next_id,
                    top: top.to_owned(),
                    folders: Vec::new(),
                    last_activity: now,
                    state: CopyState::Copying,
                    retry_after: 0,
                    making: false,
                });
                self.copies.len() - 1
            }
        };
        &mut self.copies[index]
    }

    /// The folder at `path` below Upload, and its copy, noting that something arrived now. `None` for Upload itself,
    /// whose single files are no copy of a folder.
    fn arrived(&mut self, path: &[String], now: u64) -> Option<&mut CopiedFolder> {
        let top = path.first()?;
        let copy = self.copy_for(top, now);
        copy.last_activity = now;
        Some(copy.folder(path))
    }

    /// A folder was made at `path` below Upload.
    pub fn folder_created(&mut self, path: &[String], now: u64) {
        self.arrived(path, now);
    }

    /// Something is being written in the folder at `path`: the copy is not finished.
    pub fn touched(&mut self, path: &[String], now: u64) {
        self.arrived(path, now);
    }

    /// A file in the folder at `path` was uploaded as `asset_id`, or found in the library already.
    pub fn uploaded(&mut self, path: &[String], asset_id: &str, now: u64) {
        if let Some(folder) = self.arrived(path, now) {
            if !folder.assets.iter().any(|known| known == asset_id) {
                folder.assets.push(asset_id.to_owned());
            }
        }
    }

    /// A file in the folder at `path` did not upload: refused, or failed.
    pub fn not_uploaded(&mut self, path: &[String], now: u64) {
        if let Some(folder) = self.arrived(path, now) {
            folder.not_uploaded += 1;
        }
    }

    /// The folder at `path` was removed while it held nothing that uploaded: a cancelled copy cleaning up.
    pub fn folder_removed(&mut self, path: &[String]) {
        for copy in &mut self.copies {
            copy.folders.retain(|folder| folder.path != path || !folder.assets.is_empty());
        }
        self.copies.retain(|copy| !copy.folders.is_empty() || copy.state != CopyState::Copying);
    }

    /// Move every copy along at `now`. `busy` says whether anything under a copied folder of that name is being
    /// written or uploaded. A copy that settled with nothing uploaded (an empty folder, or only refused files) is
    /// not asked about: it has nothing to make an album of.
    pub fn tick(&mut self, now: u64, choice: FolderChoice, busy: impl Fn(&str) -> bool) -> Vec<FolderEvent> {
        let mut events = Vec::new();
        for copy in &mut self.copies {
            let settled = now >= copy.last_activity.saturating_add(SETTLE_SECONDS) && !busy(&copy.top);
            match copy.state {
                CopyState::Copying if settled && copy.uploaded() == 0 => copy.state = CopyState::Timeline,
                CopyState::Copying if settled => match choice {
                    FolderChoice::Ask => {
                        copy.state = CopyState::Asking { since: now };
                        events.push(FolderEvent::Ask(copy.question()));
                    }
                    FolderChoice::Albums => {
                        copy.state = CopyState::Albums;
                        events.push(FolderEvent::MakeAlbums(copy.id));
                    }
                    FolderChoice::Timeline => copy.state = CopyState::Timeline,
                },
                CopyState::Asking { since } if now >= since.saturating_add(QUESTION_SECONDS) => {
                    copy.state = CopyState::Timeline;
                    events.push(FolderEvent::Unanswered(copy.id));
                }
                CopyState::Albums if settled && !copy.making && now >= copy.retry_after && copy.has_album_work() => {
                    events.push(FolderEvent::MakeAlbums(copy.id));
                }
                _ => {}
            }
        }
        self.copies.retain(|copy| copy.making || copy.is_active(now));
        events
    }

    /// The questions waiting for an answer.
    pub fn questions(&self) -> Vec<FolderQuestion> {
        self.copies.iter().filter(|copy| matches!(copy.state, CopyState::Asking { .. })).map(FolderCopy::question).collect()
    }

    /// The question about copy `id`, as it stands now, while it waits for its answer.
    pub fn question(&self, id: u64) -> Option<FolderQuestion> {
        self.copies.iter().find(|copy| copy.id == id && matches!(copy.state, CopyState::Asking { .. })).map(FolderCopy::question)
    }

    /// The answer to the question about copy `id`: albums too, or the timeline only. Returns whether it was waiting
    /// for one; an answer that comes too late, or twice, changes nothing.
    pub fn answer(&mut self, id: u64, albums: bool) -> bool {
        match self.copies.iter_mut().find(|copy| copy.id == id) {
            Some(copy) if matches!(copy.state, CopyState::Asking { .. }) => {
                copy.state = if albums { CopyState::Albums } else { CopyState::Timeline };
                true
            }
            _ => false,
        }
    }

    /// The album work waiting in copy `id`, taken: until `albums_made` reports on it, nothing else takes it.
    pub fn album_work(&mut self, id: u64) -> Option<AlbumWork> {
        let copy = self.copies.iter_mut().find(|copy| copy.id == id && copy.state == CopyState::Albums && !copy.making)?;
        let items: Vec<AlbumItem> = copy
            .folders
            .iter()
            .filter(|folder| folder.assets.len() > folder.in_album)
            .map(|folder| AlbumItem {
                path: folder.path.clone(),
                album: folder.album.clone(),
                // Added to an album made before: only what came since. Made now: everything.
                assets: if folder.album.is_some() { folder.assets[folder.in_album..].to_vec() } else { folder.assets.clone() },
                through: folder.assets.len(),
            })
            .collect();
        if items.is_empty() {
            return None;
        }
        copy.making = true;
        Some(AlbumWork { id, folder: copy.top.clone(), items })
    }

    /// Record what `make_albums` did with copy `id`'s work. `failure` says why the rest was not done: a credential
    /// that may not make albums ends the work, and the copy stays in the timeline only; anything else is tried again
    /// after `RETRY_SECONDS`.
    pub fn albums_made(&mut self, id: u64, made: &[MadeAlbum], failure: Option<&SourceError>, now: u64) {
        let Some(copy) = self.copies.iter_mut().find(|copy| copy.id == id) else { return };
        copy.making = false;
        for album in made {
            let folder = copy.folder(&album.path);
            folder.album = Some(album.album_id.clone());
            folder.in_album = folder.in_album.max(album.through);
        }
        match failure {
            Some(SourceError::MissingPermission(_) | SourceError::SignedOut) => copy.state = CopyState::Timeline,
            Some(_) => copy.retry_after = now.saturating_add(RETRY_SECONDS),
            None => {}
        }
    }
}

/// Where albums are made: Immich, or a fake in tests.
pub trait AlbumTarget: Send + Sync {
    /// The names of every album the account sees, its own and those shared with it.
    fn album_names(&self) -> impl Future<Output = Result<Vec<String>, SourceError>> + Send;
    /// Make an album named `name` holding `asset_ids`, and return its id.
    fn create_album(&self, name: &str, asset_ids: &[String]) -> impl Future<Output = Result<String, SourceError>> + Send;
    /// Put `asset_ids` in album `album_id`; any already there stay as they are.
    fn add_to_album(&self, album_id: &str, asset_ids: &[String]) -> impl Future<Output = Result<(), SourceError>> + Send;
}

impl<Shared: AlbumTarget> AlbumTarget for Arc<Shared> {
    async fn album_names(&self) -> Result<Vec<String>, SourceError> {
        self.as_ref().album_names().await
    }

    async fn create_album(&self, name: &str, asset_ids: &[String]) -> Result<String, SourceError> {
        self.as_ref().create_album(name, asset_ids).await
    }

    async fn add_to_album(&self, album_id: &str, asset_ids: &[String]) -> Result<(), SourceError> {
        self.as_ref().add_to_album(album_id, asset_ids).await
    }
}

/// The first of `base`, `base - <today>`, `base - <today> (2)`, … that no album has, compared as the mount compares
/// names (`clash_key`); taken from then on.
fn free_name(base: &str, taken: &mut HashSet<String>, today: NaiveDate) -> String {
    let dated = format!("{base} - {}", today.format("%Y-%m-%d"));
    let name = [base.to_owned(), dated.clone()]
        .into_iter()
        .chain((2..).map(|counter| format!("{dated} ({counter})")))
        .find(|candidate| !taken.contains(&clash_key(candidate)))
        .expect("there is always a free name");
    taken.insert(clash_key(&name));
    name
}

/// Do album work: make each folder's album, named by its path and dated where an album has that name already, or add
/// to the one made before. Returns what was done, for `FolderUploads::albums_made`, and the failure that stopped the
/// rest. An album made before and deleted since is not made again: someone removed it on purpose.
pub async fn make_albums(target: &impl AlbumTarget, work: &AlbumWork, today: NaiveDate) -> (Vec<MadeAlbum>, Option<SourceError>) {
    let mut made = Vec::new();
    let mut taken = HashSet::new();
    if work.items.iter().any(|item| item.album.is_none()) {
        match target.album_names().await {
            Ok(names) => taken.extend(names.iter().map(|name| clash_key(name))),
            Err(error) => return (made, Some(error)),
        }
    }
    for item in &work.items {
        let outcome = match &item.album {
            Some(album_id) => match target.add_to_album(album_id, &item.assets).await {
                Ok(()) | Err(SourceError::NotFound) => Ok((album_id.clone(), None)),
                Err(error) => Err(error),
            },
            None => {
                let name = free_name(&album_name(&item.path), &mut taken, today);
                target.create_album(&name, &item.assets).await.map(|album_id| (album_id, Some(name)))
            }
        };
        match outcome {
            Ok((album_id, name)) => made.push(MadeAlbum { path: item.path.clone(), album_id, name, through: item.through }),
            Err(error) => return (made, Some(error)),
        }
    }
    (made, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn path(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|part| part.to_string()).collect()
    }

    const IDLE: fn(&str) -> bool = |_| false;

    /// A copy of `Trip` with two photos in it and one in `Trip/Day 1`, at time 100.
    fn trip() -> FolderUploads {
        let mut uploads = FolderUploads::default();
        uploads.folder_created(&path(&["Trip"]), 100);
        uploads.uploaded(&path(&["Trip"]), "a1", 100);
        uploads.uploaded(&path(&["Trip"]), "a2", 101);
        uploads.folder_created(&path(&["Trip", "Day 1"]), 101);
        uploads.uploaded(&path(&["Trip", "Day 1"]), "b1", 102);
        uploads.folder_created(&path(&["Trip", "Empty"]), 102);
        uploads.not_uploaded(&path(&["Trip", "Day 1"]), 102);
        uploads
    }

    #[test]
    fn a_copy_is_asked_about_once_it_has_settled() {
        let mut uploads = trip();
        assert!(uploads.tick(105, FolderChoice::Ask, IDLE).is_empty(), "not quiet long enough");
        assert!(uploads.tick(200, FolderChoice::Ask, |top| top == "Trip").is_empty(), "something in it is still uploading");
        let events = uploads.tick(200, FolderChoice::Ask, IDLE);
        let question = FolderQuestion {
            id: 1,
            folder: "Trip".into(),
            uploaded: 3,
            not_uploaded: 1,
            albums: vec!["Trip".into(), "Trip › Day 1".into()],
        };
        assert_eq!(events, [FolderEvent::Ask(question.clone())], "a folder without uploads makes no album");
        assert!(uploads.tick(300, FolderChoice::Ask, IDLE).is_empty(), "asked once");
        assert_eq!(uploads.questions(), [question]);
    }

    #[test]
    fn single_files_in_upload_are_no_copy_of_a_folder() {
        let mut uploads = FolderUploads::default();
        uploads.uploaded(&[], "a1", 100);
        uploads.not_uploaded(&[], 100);
        assert_eq!(uploads, FolderUploads::default());
    }

    #[test]
    fn a_copy_with_nothing_uploaded_is_not_asked_about() {
        let mut uploads = FolderUploads::default();
        uploads.folder_created(&path(&["Notes"]), 100);
        uploads.not_uploaded(&path(&["Notes"]), 100);
        assert!(uploads.tick(200, FolderChoice::Ask, IDLE).is_empty());
        assert!(uploads.questions().is_empty());
    }

    #[test]
    fn an_answer_counts_once_and_only_while_asked() {
        let mut uploads = trip();
        assert!(!uploads.answer(1, true), "not asked yet");
        uploads.tick(200, FolderChoice::Ask, IDLE);
        assert!(uploads.answer(1, true));
        assert!(!uploads.answer(1, false), "answered already");
        assert_eq!(uploads.tick(201, FolderChoice::Ask, IDLE), [FolderEvent::MakeAlbums(1)]);
    }

    #[test]
    fn an_unanswered_question_leaves_the_copy_in_the_timeline() {
        let mut uploads = trip();
        uploads.tick(200, FolderChoice::Ask, IDLE);
        assert_eq!(uploads.tick(200 + QUESTION_SECONDS, FolderChoice::Ask, IDLE), [FolderEvent::Unanswered(1)]);
        assert!(!uploads.answer(1, true), "too late");
        assert!(uploads.tick(200 + QUESTION_SECONDS + REMEMBER_SECONDS, FolderChoice::Ask, IDLE).is_empty());
        assert_eq!(uploads.copies.len(), 0, "forgotten in the end");
    }

    #[test]
    fn the_timeline_choice_asks_nothing_and_the_albums_choice_makes_them() {
        let mut timeline = trip();
        assert!(timeline.tick(200, FolderChoice::Timeline, IDLE).is_empty());
        assert!(timeline.questions().is_empty());
        let mut albums = trip();
        assert_eq!(albums.tick(200, FolderChoice::Albums, IDLE), [FolderEvent::MakeAlbums(1)]);
    }

    /// Albums made in memory; `refuse` makes every call fail with it.
    #[derive(Default)]
    struct FakeAlbums {
        albums: Mutex<Vec<(String, String, Vec<String>)>>,
        refuse: Mutex<Option<SourceError>>,
    }

    impl AlbumTarget for FakeAlbums {
        async fn album_names(&self) -> Result<Vec<String>, SourceError> {
            if let Some(error) = self.refuse.lock().unwrap().clone() {
                return Err(error);
            }
            Ok(self.albums.lock().unwrap().iter().map(|(_, name, _)| name.clone()).collect())
        }

        async fn create_album(&self, name: &str, asset_ids: &[String]) -> Result<String, SourceError> {
            if let Some(error) = self.refuse.lock().unwrap().clone() {
                return Err(error);
            }
            let mut albums = self.albums.lock().unwrap();
            let id = format!("album-{}", albums.len() + 1);
            albums.push((id.clone(), name.to_owned(), asset_ids.to_vec()));
            Ok(id)
        }

        async fn add_to_album(&self, album_id: &str, asset_ids: &[String]) -> Result<(), SourceError> {
            let mut albums = self.albums.lock().unwrap();
            let album = albums.iter_mut().find(|(id, _, _)| id == album_id).ok_or(SourceError::NotFound)?;
            album.2.extend(asset_ids.iter().cloned());
            Ok(())
        }
    }

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, 9).unwrap()
    }

    async fn work(uploads: &mut FolderUploads, target: &FakeAlbums, id: u64, now: u64) -> Vec<MadeAlbum> {
        let work = uploads.album_work(id).expect("work waits");
        assert!(uploads.album_work(id).is_none(), "taken: nothing else takes it meanwhile");
        let (made, failure) = make_albums(target, &work, today()).await;
        uploads.albums_made(id, &made, failure.as_ref(), now);
        made
    }

    #[tokio::test]
    async fn each_folder_with_uploads_becomes_an_album_named_by_its_path() {
        let target = FakeAlbums::default();
        let mut uploads = trip();
        uploads.tick(200, FolderChoice::Albums, IDLE);
        work(&mut uploads, &target, 1, 200).await;
        let albums = target.albums.lock().unwrap().clone();
        assert_eq!(
            albums,
            [("album-1".into(), "Trip".into(), path(&["a1", "a2"])), ("album-2".into(), "Trip › Day 1".into(), path(&["b1"]))]
        );
        assert!(uploads.tick(300, FolderChoice::Albums, IDLE).is_empty(), "nothing left to do");
    }

    #[tokio::test]
    async fn a_taken_name_is_dated_and_then_numbered() {
        let target = FakeAlbums::default();
        for name in ["trip", "Trip › Day 1", "Trip › Day 1 - 2026-11-09"] {
            target.create_album(name, &[]).await.unwrap();
        }
        let mut uploads = trip();
        uploads.tick(200, FolderChoice::Albums, IDLE);
        let made = work(&mut uploads, &target, 1, 200).await;
        let names: Vec<_> = made.iter().map(|album| album.name.clone().unwrap()).collect();
        assert_eq!(names, ["Trip - 2026-11-09", "Trip › Day 1 - 2026-11-09 (2)"], "compared ignoring case");
    }

    #[tokio::test]
    async fn what_arrives_later_joins_the_albums_made() {
        let target = FakeAlbums::default();
        let mut uploads = trip();
        uploads.tick(200, FolderChoice::Albums, IDLE);
        work(&mut uploads, &target, 1, 200).await;
        uploads.uploaded(&path(&["Trip"]), "a3", 250);
        uploads.uploaded(&path(&["Trip", "Day 2"]), "c1", 250);
        assert!(uploads.tick(255, FolderChoice::Albums, IDLE).is_empty(), "settling again first");
        assert_eq!(uploads.tick(300, FolderChoice::Albums, IDLE), [FolderEvent::MakeAlbums(1)]);
        work(&mut uploads, &target, 1, 300).await;
        let albums = target.albums.lock().unwrap().clone();
        assert_eq!(albums[0].2, path(&["a1", "a2", "a3"]), "added to, not made again");
        assert_eq!((albums[2].1.as_str(), albums[2].2.clone()), ("Trip › Day 2", path(&["c1"])));
    }

    #[tokio::test]
    async fn a_copy_finished_long_ago_does_not_take_a_new_one() {
        let target = FakeAlbums::default();
        let mut uploads = trip();
        uploads.tick(200, FolderChoice::Albums, IDLE);
        work(&mut uploads, &target, 1, 200).await;
        let later = 102 + REMEMBER_SECONDS + 1;
        uploads.tick(later, FolderChoice::Albums, IDLE);
        uploads.uploaded(&path(&["Trip"]), "d1", later);
        uploads.tick(later + SETTLE_SECONDS, FolderChoice::Albums, IDLE);
        work(&mut uploads, &target, 2, later).await;
        assert_eq!(target.albums.lock().unwrap()[2].1, "Trip - 2026-11-09", "a second copy of Trip: an album of its own");
    }

    #[tokio::test]
    async fn without_permission_the_copy_stays_in_the_timeline_and_other_failures_wait() {
        let target = FakeAlbums::default();
        *target.refuse.lock().unwrap() = Some(SourceError::Offline("unreachable".into()));
        let mut uploads = trip();
        uploads.tick(200, FolderChoice::Albums, IDLE);
        work(&mut uploads, &target, 1, 200).await;
        assert!(uploads.tick(210, FolderChoice::Albums, IDLE).is_empty(), "waits before trying again");
        assert_eq!(uploads.tick(200 + RETRY_SECONDS, FolderChoice::Albums, IDLE), [FolderEvent::MakeAlbums(1)]);
        *target.refuse.lock().unwrap() = Some(SourceError::MissingPermission(Some("album.create".into())));
        work(&mut uploads, &target, 1, 300).await;
        assert!(uploads.tick(300 + RETRY_SECONDS, FolderChoice::Albums, IDLE).is_empty(), "never tried again");
        assert!(uploads.album_work(1).is_none());
    }

    #[tokio::test]
    async fn an_album_deleted_since_is_not_made_again() {
        let target = FakeAlbums::default();
        let mut uploads = trip();
        uploads.tick(200, FolderChoice::Albums, IDLE);
        work(&mut uploads, &target, 1, 200).await;
        target.albums.lock().unwrap().remove(0);
        uploads.uploaded(&path(&["Trip"]), "a3", 250);
        uploads.tick(300, FolderChoice::Albums, IDLE);
        work(&mut uploads, &target, 1, 300).await;
        assert_eq!(target.albums.lock().unwrap().len(), 1, "only Day 1's");
        assert!(uploads.tick(400, FolderChoice::Albums, IDLE).is_empty());
    }

    #[test]
    fn work_cut_short_by_a_stopped_process_is_taken_up_again() {
        let mut uploads = trip();
        uploads.tick(200, FolderChoice::Albums, IDLE);
        uploads.album_work(1).unwrap();
        let kept: FolderUploads = serde_json::from_str(&serde_json::to_string(&uploads).unwrap()).unwrap();
        let mut again = kept;
        assert!(again.album_work(1).is_some(), "the work in progress is not kept");
    }

    #[test]
    fn a_folder_removed_empty_is_forgotten() {
        let mut uploads = FolderUploads::default();
        uploads.folder_created(&path(&["Trip"]), 100);
        uploads.folder_created(&path(&["Trip", "Day 1"]), 100);
        uploads.folder_removed(&path(&["Trip", "Day 1"]));
        uploads.folder_removed(&path(&["Trip"]));
        assert!(uploads.copies.is_empty(), "a cancelled copy leaves nothing behind");
    }

    #[test]
    fn the_choice_comes_from_the_setting() {
        assert_eq!(FolderChoice::from_setting("albums"), FolderChoice::Albums);
        assert_eq!(FolderChoice::from_setting("timeline"), FolderChoice::Timeline);
        assert_eq!(FolderChoice::from_setting("ask"), FolderChoice::Ask);
    }
}
