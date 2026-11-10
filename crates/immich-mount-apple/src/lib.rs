//! The bridge between the macOS edition's Swift code and the portable core (ARCHITECTURE.md, "The macOS edition").
//!
//! The File Provider extension and the app call it through uniffi. It answers in the system's terms (item
//! identifiers, whole originals, thumbnails at a size) from the core's tree, so Finder shows the names Files and
//! Dolphin show. An item's identifier is its `NodeKey`'s storage key; the root's is `root`. Nothing here needs a Mac:
//! it builds and is tested on Linux too, and only the Swift side needs Xcode.

use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex, OnceLock, RwLock, Weak};
use std::task::{Context, Poll};
use std::time::{Duration, Instant, SystemTime};

use immich_api::{ApiError, Client, Credential as ApiCredential};
use immich_mount_core::folder_uploads::{make_albums, tried_again, AlbumProgress, AlbumTarget, FolderEvent, FolderUploads};
use immich_mount_core::immich::ImmichSource;
use immich_mount_core::links;
use immich_mount_core::settings::{self, choice_label, Config, Kind, SETTINGS};
use immich_mount_core::sign_in::{self, normalised_address, BrowserSignIn};
use immich_mount_core::source::{LibrarySource, SourceError, ThumbnailKind, ThumbnailSource};
use immich_mount_core::store::{Anchor, Change, JournalRead, MetadataStore};
use immich_mount_core::tree::{is_touched, Entry, EntryKind, NameMemory, NodeKey, Tree};
use immich_mount_core::uploads::{completeness, Admission, Completeness, UploadError, UploadTarget};
use sha2::{Digest, Sha256};

uniffi::setup_scaffolding!();

const USER_AGENT: &str = concat!("immich-mount/", env!("CARGO_PKG_VERSION"), " (macOS)");
/// How long a listing or an item may take before the system hears that the server cannot be reached. A listing kept
/// from before answers instead, when there is one.
const LISTING_DEADLINE: Duration = Duration::from_secs(20);
/// Immich's small thumbnail is 250 pixels on its short side. Up to this size it is upscaled a little; above it, the
/// preview (1440 pixels) is sent.
const SMALL_THUMBNAIL_UP_TO: u32 = 320;
/// How many journal records the store keeps; an anchor older than that is expired, and the system enumerates again.
const JOURNAL_LIMIT: u64 = 20_000;
/// The name the change poll's memory is kept under in the store.
const POLL_MEMORY: &str = "change-poll";
/// The name uploaded files waiting to leave the Upload folder are kept under in the store.
const LEAVING_UPLOAD: &str = "leaving-upload";
/// How long an uploaded file stays in the Upload folder, as on Linux: long enough for the system to take the item
/// before it is told the item went away.
const UPLOAD_STAY: Duration = Duration::from_secs(10);
/// The name the folders copied into Upload are kept under in the store, as the system's items.
const UPLOAD_FOLDERS: &str = "upload-folders";
/// The name the record of each copied folder is kept under in the store (`FolderUploads`).
const FOLDER_COPIES: &str = "folder-copies";
/// How long a folder copied into Upload stays once it is empty and untouched, when macOS says it holds nothing the
/// system has not handed over (its pending set): long enough for the last files to leave. Never shorter than the time
/// macOS takes to bring that list up to date, and a margin.
const FOLDER_STAY: Duration = Duration::from_secs(20);
/// The margin above the pending set's refresh time.
const PENDING_MARGIN: Duration = Duration::from_secs(10);
/// How long it stays when macOS does not say: as long as its copy is remembered, so a copy Finder is still writing
/// (a large file, which the extension hears of only once it is whole) is never taken from under it.
const FOLDER_STAY_UNSURE: Duration = Duration::from_secs(immich_mount_core::folder_uploads::REMEMBER_SECONDS);
/// Where the extension leaves the questions about settled copies for the app, in the library's directory.
const QUESTIONS_FILE: &str = "folder-questions.json";
/// The app's answers, one file each (`folder-answer-<copy>.json`), which the extension takes and removes.
const ANSWER_FILE_PREFIX: &str = "folder-answer-";
/// What came of album work, for the app to tell, one file each (`folder-notice-<n>.json`); the app removes them.
const NOTICE_FILE_PREFIX: &str = "folder-notice-";
/// Why a file or folder added anywhere but Upload is refused.
const ONLY_UPLOAD: &str = "Only the Upload folder takes new files; every other folder is a view of the library.";
/// How long one poll lists folders again; what is left waits for the next poll, so a change's report never waits
/// on a library-wide relisting.
const RELIST_BUDGET: Duration = Duration::from_secs(20);
/// What the extension's items carry (their capabilities and fields). The system keeps an item's details until its
/// version changes, so the format is part of every version: raising it, with the app's reimport, has the system take
/// every item again (ARCHITECTURE.md, "The macOS edition", "Content").
/// 4: folders carry the time the location was set up, where they carried none (Finder showed 1 January 1970).
/// 5: items say whether they have a web address (`has_web_link`), which Finder's Open in Immich and Copy Immich Link
/// are offered by.
const ITEM_FORMAT: u32 = 5;
/// How long the server check keeps trying a connection that fails: macOS may refuse sockets while its Local Network
/// prompt waits for an answer (Apple's TN3179).
const LOCAL_NETWORK_PATIENCE: Duration = Duration::from_secs(30);
/// How long one request of signing in may take; the client's own retries would wait minutes on a server that drops
/// requests.
const SIGN_IN_LIMIT: Duration = Duration::from_secs(10);

/// Why a request failed, in the kinds the system tells apart.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum BridgeError {
    /// The credential was refused: sign in again.
    #[error("signed out of Immich")]
    NotAuthenticated,
    /// The server cannot be reached, or is busy for now; the system tries again later.
    #[error("the Immich server cannot be reached: {message}")]
    ServerUnreachable { message: String },
    /// No such item, or not any more.
    #[error("no such item")]
    NoSuchItem,
    /// A sync anchor from before the journal's oldest record, or from another store: the system enumerates again.
    #[error("the sync anchor expired")]
    AnchorExpired,
    /// The system cancelled the request.
    #[error("cancelled")]
    Cancelled,
    /// The credential lacks a permission the request needs.
    #[error("{message}")]
    NotPermitted { message: String },
    /// Anything else: an unexpected answer, a file that cannot be written, a setting that does not parse.
    #[error("{message}")]
    Failed { message: String },
}

impl From<SourceError> for BridgeError {
    fn from(error: SourceError) -> BridgeError {
        match error {
            SourceError::Offline(_) | SourceError::Busy => BridgeError::ServerUnreachable { message: error.to_string() },
            SourceError::SignedOut => BridgeError::NotAuthenticated,
            SourceError::MissingPermission(_) => BridgeError::NotPermitted { message: error.to_string() },
            SourceError::NotFound => BridgeError::NoSuchItem,
            SourceError::Protocol(_) | SourceError::Changed => BridgeError::Failed { message: error.to_string() },
        }
    }
}

fn failed(error: impl std::fmt::Display) -> BridgeError {
    BridgeError::Failed { message: error.to_string() }
}

/// One item, as the system sees it.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Item {
    /// The item's identifier: its `NodeKey`'s storage key. The root's is `root`.
    pub identifier: String,
    /// The identifier of the folder that lists it; the root's is its own.
    pub parent_identifier: String,
    pub name: String,
    pub is_folder: bool,
    /// Whether new files may be added: the Upload folder's.
    pub accepts_new_files: bool,
    /// The original's size in bytes, when Immich said.
    pub size: Option<u64>,
    /// When the photo or video was taken: the file's creation and modification time.
    pub taken: Option<SystemTime>,
    /// The content's version: an Asset's original never changes, so its id; `folder` for folders.
    pub content_version: String,
    /// Whether the item has a web address in Immich (`links::immich_path`), so Finder offers Open in Immich and Copy
    /// Immich Link on it. It follows from the identifier alone, so it never changes for an item.
    pub has_web_link: bool,
    /// Changes whenever anything shown about the item (its name, size or time) changes, so the system updates it.
    pub version: String,
}

impl Item {
    fn versioned(mut self) -> Item {
        let mut digest = Sha256::new();
        digest.update(ITEM_FORMAT.to_le_bytes());
        digest.update(self.name.as_bytes());
        digest.update([0]);
        digest.update(self.size.unwrap_or(u64::MAX).to_le_bytes());
        let taken = self.taken.and_then(|taken| taken.duration_since(SystemTime::UNIX_EPOCH).ok()).map_or(0, |since| since.as_secs());
        digest.update(taken.to_le_bytes());
        self.version = digest.finalize()[..8].iter().map(|byte| format!("{byte:02x}")).collect();
        self
    }
}

/// The root, whose time is the folders' (`Answers::set_up`).
fn root_item(set_up: SystemTime) -> Item {
    let root = NodeKey::Root.storage_key();
    Item {
        identifier: root.clone(),
        parent_identifier: root,
        name: "Immich".to_owned(),
        is_folder: true,
        accepts_new_files: false,
        size: None,
        taken: Some(set_up),
        content_version: "folder".to_owned(),
        has_web_link: false,
        version: String::new(),
    }
    .versioned()
}

/// An entry as the system sees it. A file has its capture time; a folder the time the location was set up, as on
/// Linux a folder has the mount's (SPEC.md, "Attributes").
fn item_for(entry: &Entry, folder: &NodeKey, set_up: SystemTime) -> Item {
    let (size, taken, content_version) = match &entry.kind {
        EntryKind::Folder => (None, Some(set_up), "folder".to_owned()),
        EntryKind::File { asset_id, size, modified } => (*size, Some(SystemTime::from(*modified)), asset_id.clone()),
    };
    Item {
        identifier: entry.node.storage_key(),
        parent_identifier: folder.storage_key(),
        name: entry.name.clone(),
        is_folder: matches!(entry.kind, EntryKind::Folder),
        accepts_new_files: entry.node.upload_path().is_some(),
        size,
        taken,
        content_version,
        has_web_link: links::immich_path(&entry.node).is_some(),
        version: String::new(),
    }
    .versioned()
}

/// When the location was set up: the store's creation, which signing in makes and signing out removes. Its
/// modification time where the file system keeps no creation time.
fn set_up_time(store: &Path) -> SystemTime {
    std::fs::metadata(store).and_then(|metadata| metadata.created().or_else(|_| metadata.modified())).unwrap_or_else(|_| SystemTime::now())
}

fn node_for(identifier: &str) -> Result<NodeKey, BridgeError> {
    NodeKey::from_storage_key(identifier).ok_or(BridgeError::NoSuchItem)
}

/// What the macOS edition needs of a library beyond the tree's source: whole originals, which the system asks for
/// where Linux reads ranges, and the change poll's memory, kept between runs of an extension the system stops.
pub trait MacSource: Send + Sync {
    /// Write an Asset's whole original to `destination`, and return the bytes written.
    fn download(&self, asset_id: &str, destination: &Path) -> impl Future<Output = Result<u64, SourceError>> + Send;
    /// What the change poll has seen, to keep; `None` before its first poll.
    fn poll_memory(&self) -> impl Future<Output = Option<String>> + Send;
    /// Start from a kept memory.
    fn recall_poll(&self, memory: &str) -> impl Future<Output = ()> + Send;
}

impl MacSource for ImmichSource {
    async fn download(&self, asset_id: &str, destination: &Path) -> Result<u64, SourceError> {
        self.download_original(asset_id, destination).await
    }
    async fn poll_memory(&self) -> Option<String> {
        ImmichSource::poll_memory(self).await
    }
    async fn recall_poll(&self, memory: &str) {
        ImmichSource::recall_poll(self, memory).await
    }
}

/// What changed for the system since an anchor.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ChangeBatch {
    /// Items that appeared, or whose name, size or time changed.
    pub updated: Vec<Item>,
    /// Identifiers of items that went away.
    pub deleted: Vec<String>,
    /// Where these changes end: the anchor to ask from next.
    pub anchor: String,
    /// Whether more changes follow.
    pub more: bool,
}

/// The answers to the system's requests, over any source: Immich in the extension, a fake in tests.
struct Answers<Source, Names> {
    tree: Tree<Arc<Source>, Names>,
    source: Arc<Source>,
    deadline: Duration,
    /// The change poll's memory from an earlier run, recalled at the first poll.
    kept_poll: Mutex<Option<String>>,
    /// When the location was set up: every folder's time, stable from run to run, since it is part of the folders'
    /// versions.
    set_up: SystemTime,
    folders: Arc<FolderFollowing>,
    /// The library's directory, where the questions, answers and notices about copied folders pass between the
    /// extension and the app; `None` in tests that need none.
    files: Option<PathBuf>,
}

/// An uploaded file waiting to leave the Upload folder, which no listing shows: the system may still ask about it,
/// and fetch it, until it leaves.
#[derive(serde::Serialize, serde::Deserialize)]
struct Leaving {
    key: String,
    /// The folder it was added to: Upload, or a folder copied into it.
    #[serde(default = "upload_identifier")]
    parent: String,
    /// Seconds since the epoch.
    leaves_at: u64,
    name: String,
    size: Option<u64>,
    /// Seconds since the epoch.
    taken: u64,
}

impl Leaving {
    fn item(&self) -> Item {
        let asset_id = match NodeKey::from_storage_key(&self.key) {
            Some(NodeKey::Asset { asset_id, .. }) => asset_id,
            _ => String::new(),
        };
        Item {
            identifier: self.key.clone(),
            parent_identifier: self.parent.clone(),
            name: self.name.clone(),
            is_folder: false,
            accepts_new_files: false,
            size: self.size,
            taken: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(self.taken)),
            content_version: asset_id,
            has_web_link: false,
            version: String::new(),
        }
        .versioned()
    }
}

fn upload_identifier() -> String {
    NodeKey::Upload.storage_key()
}

/// A folder copied into Upload, as the system's item: no listing shows it, since it holds only what is being
/// uploaded, so it is kept until it leaves (ARCHITECTURE.md, "The macOS edition", "Uploads").
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct UploadFolder {
    key: String,
    parent: String,
    name: String,
    /// Seconds since the epoch.
    made: u64,
    /// When something last arrived in it, in seconds since the epoch.
    touched: u64,
    /// Files added to it that have not uploaded (yet), from the moment they arrive, and folders inside it that were
    /// refused: the system keeps them in the folder, so it stays.
    #[serde(default)]
    waiting: Vec<String>,
    /// Files in it that failed, each counted once as not uploaded, however often the system tries again.
    #[serde(default)]
    failed: Vec<String>,
}

impl UploadFolder {
    fn item(&self) -> Item {
        Item {
            identifier: self.key.clone(),
            parent_identifier: self.parent.clone(),
            name: self.name.clone(),
            is_folder: true,
            accepts_new_files: true,
            size: None,
            taken: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(self.made)),
            content_version: "folder".to_owned(),
            has_web_link: false,
            version: String::new(),
        }
        .versioned()
    }
}

/// The folders copied into Upload, as kept.
fn upload_folders(kept: Option<&str>) -> Vec<UploadFolder> {
    kept.and_then(|text| serde_json::from_str(text).ok()).unwrap_or_default()
}

/// Where the uploaded files waiting to leave Upload, and the folders copied into it, are kept: the metadata store,
/// which keeps them across runs.
trait Uploads {
    fn uploaded(&self, key: &str) -> Option<Item>;
    /// The copied folders: all of them, or those inside the folder `parent`.
    fn upload_folders(&self, parent: Option<&str>) -> Vec<Item>;
}

impl Uploads for MetadataStore {
    fn uploaded(&self, key: &str) -> Option<Item> {
        leaving_upload(self.kept_state(LEAVING_UPLOAD).as_deref()).iter().find(|leaving| leaving.key == key).map(Leaving::item)
    }

    fn upload_folders(&self, parent: Option<&str>) -> Vec<Item> {
        upload_folders(self.kept_state(UPLOAD_FOLDERS).as_deref())
            .iter()
            .filter(|folder| parent.is_none_or(|parent| folder.parent == parent))
            .map(UploadFolder::item)
            .collect()
    }
}

impl<Shared: Uploads> Uploads for Arc<Shared> {
    fn uploaded(&self, key: &str) -> Option<Item> {
        self.as_ref().uploaded(key)
    }

    fn upload_folders(&self, parent: Option<&str>) -> Vec<Item> {
        self.as_ref().upload_folders(parent)
    }
}

impl Uploads for immich_mount_core::tree::InMemoryNames {
    fn uploaded(&self, _key: &str) -> Option<Item> {
        None
    }

    fn upload_folders(&self, _parent: Option<&str>) -> Vec<Item> {
        Vec::new()
    }
}

/// Runs its closure when dropped: album work's task dropped midway frees the work for a later poll.
struct OnDrop<Work: FnMut()>(Option<Work>);

impl<Work: FnMut()> OnDrop<Work> {
    /// Done: nothing to run.
    fn defuse(&mut self) {
        self.0 = None;
    }
}

impl<Work: FnMut()> Drop for OnDrop<Work> {
    fn drop(&mut self) {
        if let Some(work) = self.0.as_mut() {
            work();
        }
    }
}

/// The folders copied into Upload (ARCHITECTURE.md, "Folders copied into Upload"): the core's record of each copy,
/// kept in the store after every change, and the uploads running under each copied folder, which only this process
/// knows. Shared by the answers that new settings replace.
#[derive(Default)]
struct FolderFollowing {
    copies: Mutex<FolderUploads>,
    /// Uploads running, by the copied folder they are in.
    in_flight: Mutex<HashMap<String, usize>>,
    /// Copies whose albums are due, for the library to make in a task of their own, outside the poll the system may
    /// cancel.
    albums_due: Mutex<Vec<u64>>,
}

impl FolderFollowing {
    /// The copies kept in a store, or none yet, numbered from now: a store made anew by signing in again never takes
    /// an answer the app gave about a copy of before.
    fn from_kept(kept: Option<&str>) -> FolderFollowing {
        let copies = kept
            .and_then(|text| serde_json::from_str(text).ok())
            .unwrap_or_else(|| FolderUploads::numbered_from(seconds_since_epoch(SystemTime::now())));
        FolderFollowing { copies: Mutex::new(copies), in_flight: Mutex::default(), albums_due: Mutex::default() }
    }

    fn busy(&self, top: &str) -> bool {
        self.in_flight.lock().expect("in flight lock").get(top).is_some_and(|count| *count > 0)
    }
}

/// An upload running under a copied folder, counted while it lives.
struct InFlight<'following> {
    following: &'following FolderFollowing,
    top: Option<String>,
}

impl<'following> InFlight<'following> {
    fn start(following: &'following FolderFollowing, folder: &NodeKey) -> InFlight<'following> {
        let top = folder.upload_path().and_then(<[String]>::first).cloned();
        if let Some(top) = &top {
            *following.in_flight.lock().expect("in flight lock").entry(top.clone()).or_default() += 1;
        }
        InFlight { following, top }
    }
}

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        if let Some(top) = &self.top {
            let mut in_flight = self.following.in_flight.lock().expect("in flight lock");
            if let Some(count) = in_flight.get_mut(top) {
                *count = count.saturating_sub(1);
            }
        }
    }
}

impl<Source, Names> Answers<Source, Names>
where
    Source: LibrarySource + ThumbnailSource + MacSource + 'static,
    Names: NameMemory + Uploads + 'static,
{
    fn new(source: Arc<Source>, names: Names, config: Config) -> Self {
        Answers {
            tree: Tree::new(source.clone(), names, config),
            source,
            deadline: LISTING_DEADLINE,
            kept_poll: Mutex::new(None),
            set_up: SystemTime::UNIX_EPOCH,
            folders: Arc::default(),
            files: None,
        }
    }

    fn set_up_at(mut self, set_up: SystemTime) -> Self {
        self.set_up = set_up;
        self
    }

    /// Follow the copied folders in `folders`, passing questions and answers through files in `files`.
    fn following(mut self, folders: Arc<FolderFollowing>, files: Option<PathBuf>) -> Self {
        self.folders = folders;
        self.files = files;
        self
    }

    async fn item(&self, identifier: &str) -> Result<Item, BridgeError> {
        let node = node_for(identifier)?;
        if node == NodeKey::Root {
            return Ok(root_item(self.set_up));
        }
        if matches!(&node, NodeKey::Asset { parent, .. } if parent.upload_path().is_some()) {
            return self.tree.names().uploaded(identifier).ok_or(BridgeError::NoSuchItem);
        }
        if matches!(node, NodeKey::UploadFolder(_)) {
            let folders = self.tree.names().upload_folders(None);
            return folders.into_iter().find(|folder| folder.identifier == identifier).ok_or(BridgeError::NoSuchItem);
        }
        let found = tokio::time::timeout(self.deadline, self.tree.entry_of(&node)).await.map_err(|_| too_slow())??;
        let (folder, entry) = found.ok_or(BridgeError::NoSuchItem)?;
        Ok(item_for(&entry, &folder, self.set_up))
    }

    async fn list(&self, identifier: &str) -> Result<Vec<Item>, BridgeError> {
        let folder = node_for(identifier)?;
        let entries = match tokio::time::timeout(self.deadline, self.tree.list(&folder)).await {
            Ok(listed) => listed?,
            Err(_) => self.tree.stale_listing(&folder).ok_or_else(too_slow)?,
        };
        let mut items: Vec<Item> = entries.iter().map(|entry| item_for(entry, &folder, self.set_up)).collect();
        // The folders copied into it, which no listing of the library holds: listed, the system keeps them.
        if folder.upload_path().is_some() {
            items.extend(self.tree.names().upload_folders(Some(identifier)));
        }
        Ok(items)
    }

    async fn fetch(&self, identifier: &str, destination: &Path) -> Result<Item, BridgeError> {
        let node = node_for(identifier)?;
        let NodeKey::Asset { asset_id, .. } = &node else { return Err(BridgeError::NoSuchItem) };
        let mut item = self.item(identifier).await?;
        match self.source.download(asset_id, destination).await {
            Ok(written) => {
                item.size = Some(written);
                Ok(item.versioned())
            }
            Err(error) => {
                let _ = std::fs::remove_file(destination);
                Err(error.into())
            }
        }
    }

    async fn thumbnail(&self, identifier: &str, pixels: u32) -> Result<Option<Vec<u8>>, BridgeError> {
        let node = node_for(identifier)?;
        let NodeKey::Asset { asset_id, .. } = &node else { return Ok(None) };
        let kind = if pixels <= SMALL_THUMBNAIL_UP_TO { ThumbnailKind::Small } else { ThumbnailKind::Preview };
        Ok(Some(self.source.thumbnail(asset_id, kind).await?))
    }
}

/// Live changes (ARCHITECTURE.md, "The macOS edition", "Changes"): they need the journal, which only the metadata
/// store keeps.
impl<Source> Answers<Source, Arc<MetadataStore>>
where
    Source: LibrarySource + ThumbnailSource + MacSource + 'static,
{
    fn store(&self) -> &MetadataStore {
        self.tree.names()
    }

    /// Ask the server what changed, and list again the folders it touched that the system holds, which records the
    /// differences in the journal. Returns whether the journal grew.
    ///
    /// The touched folders are marked in the store before the poll's memory is kept, so a folder that cannot be
    /// listed now (the server busy, or the budget spent) is listed at a later poll; a change is never seen once and
    /// then lost. Deletions are settled once every touched folder was listed again, so a node that only moved, to a
    /// folder not listed yet, is not deleted.
    async fn poll(&self, pending: Option<&PendingFolders>) -> Result<bool, BridgeError>
    where
        Source: AlbumTarget,
    {
        let before = self.store().journal_anchor().map_err(failed)?;
        self.record_departures(seconds_since_epoch(SystemTime::now()))?;
        let kept = self.kept_poll.lock().expect("kept poll lock").take();
        if let Some(memory) = kept {
            self.source.recall_poll(&memory).await;
        }
        let changes = self.source.changes(self.tree.watched()).await?;
        if !changes.is_empty() {
            self.tree.apply_changes(&changes);
            let touched: Vec<NodeKey> = self.store().kept_folders().into_iter().filter(|folder| is_touched(folder, &changes)).collect();
            self.store().mark_for_relisting(&touched).map_err(failed)?;
        }
        if let Some(memory) = self.source.poll_memory().await {
            self.store().keep_state(POLL_MEMORY, &memory);
        }
        let started = Instant::now();
        for folder in self.store().folders_to_relist() {
            let Some(left) = RELIST_BUDGET.checked_sub(started.elapsed()) else { break };
            // A folder that went away with its parent, listed earlier, is not asked for again.
            if self.store().kept_listing(&folder).is_none() {
                self.store().relisted(&folder).map_err(failed)?;
                continue;
            }
            match tokio::time::timeout(left, self.tree.relist(&folder)).await {
                // A folder gone from the server has nothing to list: its parent's listing records its departure.
                Ok(Ok(_) | Err(SourceError::NotFound)) => self.store().relisted(&folder).map_err(failed)?,
                Ok(Err(error)) => log::info!("a changed folder could not be listed again ({error}); the next poll tries again"),
                Err(_) => log::info!("a changed folder was not listed again in time; the next poll tries again"),
            }
        }
        if self.store().folders_to_relist().is_empty() {
            self.store().settle_departures().map_err(failed)?;
        } else {
            log::info!("deletions wait until every changed folder is listed again");
        }
        self.follow_folder_copies(pending).await?;
        Ok(self.store().journal_anchor().map_err(failed)? != before)
    }

    /// Upload a finished file added to the Upload folder, or to a folder copied into it (ARCHITECTURE.md, "The macOS
    /// edition", "Uploads"), and return its item there. It leaves the folder at the first poll after `UPLOAD_STAY`, and
    /// shows in its month once the poll sees the server's change.
    async fn upload(&self, folder: &str, name: &str, path: &Path, modified: SystemTime) -> Result<Item, BridgeError>
    where
        Source: UploadTarget,
    {
        let folder_node = node_for(folder)?;
        if folder_node.upload_path().is_none() || !self.tree.config().uploads_enabled {
            return Err(failed(ONLY_UPLOAD));
        }
        let _running = InFlight::start(&self.folders, &folder_node);
        // Finder's own record of a folder's view is no photo, and keeps no copied folder from leaving.
        if name == ".DS_Store" {
            return Err(failed(format!("{name} is not a photo or video Immich takes.")));
        }
        self.folder_waits_for(&folder_node, name)?;
        self.folder_touched(&folder_node, None)?;
        let refused = |message: String| {
            self.not_uploaded(&folder_node, name)?;
            Err(failed(message))
        };
        let media = self.source.media_types().await.unwrap_or_default();
        if media.admission(name) != Admission::Upload {
            return refused(format!("{name} is not a photo or video Immich takes."));
        }
        let owned = path.to_owned();
        let judged = tokio::task::spawn_blocking(move || completeness(&owned)).await.map_err(failed)?;
        if matches!(judged.map_err(failed)?, Completeness::Truncated) {
            return refused(format!("{name} is not whole: its copy may have been cut short."));
        }
        let asset_id = match self.source.upload(path, name, modified.into()).await {
            Ok(uploaded) => uploaded.asset_id().to_owned(),
            Err(error) => {
                self.not_uploaded(&folder_node, name)?;
                return Err(match error {
                    UploadError::Source(error) => error.into(),
                    error => failed(error),
                });
            }
        };
        // The same bytes added twice are one Asset, so one item: the system merges the two.
        let node = NodeKey::Asset { parent: Box::new(folder_node.clone()), asset_id: asset_id.clone() };
        let leaving = Leaving {
            key: node.storage_key(),
            parent: folder.to_owned(),
            leaves_at: seconds_since_epoch(SystemTime::now() + UPLOAD_STAY),
            name: name.to_owned(),
            size: std::fs::metadata(path).ok().map(|metadata| metadata.len()),
            taken: seconds_since_epoch(modified),
        };
        let item = leaving.item();
        self.store()
            .update_state(LEAVING_UPLOAD, |kept| {
                let mut waiting = leaving_upload(kept);
                waiting.retain(|earlier| earlier.key != leaving.key);
                waiting.push(leaving);
                (Some(serde_json::to_string(&waiting).unwrap_or_default()), Vec::new(), ())
            })
            .map_err(failed)?;
        self.folder_touched(&folder_node, Some(name))?;
        if let Some(path) = folder_node.upload_path().filter(|path| !path.is_empty()) {
            self.with_copies(|copies| copies.uploaded(path, &asset_id, seconds_since_epoch(SystemTime::now())));
        }
        self.tree.forget_timeline();
        Ok(item)
    }

    /// Change the record of the copied folders, and keep it: the system stops the extension between requests.
    fn with_copies<Answer>(&self, change: impl FnOnce(&mut FolderUploads) -> Answer) -> Answer {
        let mut copies = self.folders.copies.lock().expect("folder copies lock");
        let answer = change(&mut copies);
        self.store().keep_state(FOLDER_COPIES, &serde_json::to_string(&*copies).unwrap_or_default());
        answer
    }

    /// Note that something arrived in a copied folder, so it stays; with `uploaded`, that the file of that name is
    /// in Immich now, so the folder no longer waits for it. Nothing for Upload itself.
    fn folder_touched(&self, folder: &NodeKey, uploaded: Option<&str>) -> Result<(), BridgeError> {
        let Some(path) = folder.upload_path().filter(|path| !path.is_empty()) else { return Ok(()) };
        let (key, now) = (folder.storage_key(), seconds_since_epoch(SystemTime::now()));
        self.store()
            .update_state(UPLOAD_FOLDERS, |kept| {
                let mut folders = upload_folders(kept);
                if let Some(record) = folders.iter_mut().find(|record| record.key == key) {
                    record.touched = now;
                    record.waiting.retain(|name| Some(name.as_str()) != uploaded);
                }
                (Some(serde_json::to_string(&folders).unwrap_or_default()), Vec::new(), ())
            })
            .map_err(failed)?;
        if uploaded.is_none() {
            self.with_copies(|copies| copies.touched(path, now));
        }
        Ok(())
    }

    /// Something named `name` arrived in the copied folder `folder`: until it uploads (or is deleted with its folder),
    /// the system holds it there, so the folder stays. Nothing for Upload itself.
    fn folder_waits_for(&self, folder: &NodeKey, name: &str) -> Result<(), BridgeError> {
        if !matches!(folder, NodeKey::UploadFolder(_)) {
            return Ok(());
        }
        let key = folder.storage_key();
        self.store()
            .update_state(UPLOAD_FOLDERS, |kept| {
                let mut folders = upload_folders(kept);
                if let Some(record) = folders.iter_mut().find(|record| record.key == key) {
                    if !record.waiting.iter().any(|waiting| waiting == name) {
                        record.waiting.push(name.to_owned());
                    }
                }
                (Some(serde_json::to_string(&folders).unwrap_or_default()), Vec::new(), ())
            })
            .map_err(failed)
    }

    /// A file added to `folder` as `name` did not upload. In a copied folder the system keeps it there, refused with
    /// its reason, so the folder stays until it uploads; the copy counts it once.
    fn not_uploaded(&self, folder: &NodeKey, name: &str) -> Result<(), BridgeError> {
        let Some(path) = folder.upload_path().filter(|path| !path.is_empty()) else { return Ok(()) };
        let key = folder.storage_key();
        let first_time = self
            .store()
            .update_state(UPLOAD_FOLDERS, |kept| {
                let mut folders = upload_folders(kept);
                let mut first_time = false;
                if let Some(record) = folders.iter_mut().find(|record| record.key == key) {
                    if !record.waiting.iter().any(|waiting| waiting == name) {
                        record.waiting.push(name.to_owned());
                    }
                    if !record.failed.iter().any(|failed| failed == name) {
                        record.failed.push(name.to_owned());
                        first_time = true;
                    }
                }
                (Some(serde_json::to_string(&folders).unwrap_or_default()), Vec::new(), first_time)
            })
            .map_err(failed)?;
        if first_time {
            self.with_copies(|copies| copies.not_uploaded(path, seconds_since_epoch(SystemTime::now())));
        }
        Ok(())
    }

    /// Make the folder `name` inside `parent` (Upload, or a folder copied into it), as Finder copies a folder in, and
    /// return its item. Made again (the system reimporting), it is the one there.
    fn create_folder(&self, parent: &str, name: &str) -> Result<Item, BridgeError> {
        let parent_node = node_for(parent)?;
        let key = parent_node.upload_child(name).filter(|_| self.tree.config().uploads_enabled).ok_or_else(|| failed(ONLY_UPLOAD))?;
        if name.is_empty() || name.starts_with('.') || name.contains('/') {
            // The system keeps the refused folder in its parent, so the parent stays with it.
            self.folder_waits_for(&parent_node, name)?;
            return Err(failed("Hidden folders are not uploaded."));
        }
        let (identifier, now) = (key.storage_key(), seconds_since_epoch(SystemTime::now()));
        let record = self
            .store()
            .update_state(UPLOAD_FOLDERS, |kept| {
                let mut folders = upload_folders(kept);
                let record = match folders.iter_mut().find(|record| record.key == identifier) {
                    Some(record) => {
                        record.touched = now;
                        record.clone()
                    }
                    None => {
                        let record = UploadFolder {
                            key: identifier.clone(),
                            parent: parent.to_owned(),
                            name: name.to_owned(),
                            made: now,
                            touched: now,
                            waiting: Vec::new(),
                            failed: Vec::new(),
                        };
                        folders.push(record.clone());
                        record
                    }
                };
                (Some(serde_json::to_string(&folders).unwrap_or_default()), Vec::new(), record)
            })
            .map_err(failed)?;
        if let Some(path) = key.upload_path() {
            self.with_copies(|copies| copies.folder_created(path, now));
        }
        Ok(record.item())
    }

    /// Someone deleted a copied folder in Finder (its files that did not upload with it); it is forgotten, with the
    /// folders inside it. Only these folders may be deleted: everything else is a view of the library.
    fn delete_folder(&self, identifier: &str) -> Result<(), BridgeError> {
        let Some(path) = node_for(identifier)?.upload_path().filter(|path| !path.is_empty()).map(<[String]>::to_vec) else {
            return Err(failed(ONLY_UPLOAD));
        };
        let gone = self
            .store()
            .update_state(UPLOAD_FOLDERS, |kept| {
                let (gone, folders): (Vec<UploadFolder>, Vec<UploadFolder>) = upload_folders(kept).into_iter().partition(|record| {
                    NodeKey::from_storage_key(&record.key).and_then(|key| key.upload_path().map(|inside| inside.starts_with(&path))).unwrap_or(false)
                });
                ((!folders.is_empty()).then(|| serde_json::to_string(&folders).unwrap_or_default()), Vec::new(), gone)
            })
            .map_err(failed)?;
        if gone.is_empty() {
            return Err(BridgeError::NoSuchItem);
        }
        self.with_copies(|copies| {
            for record in &gone {
                if let Some(key) = NodeKey::from_storage_key(&record.key) {
                    copies.folder_removed(key.upload_path().unwrap_or_default());
                }
            }
        });
        Ok(())
    }

    /// The copied folders' part of a poll: take the app's answers, move the copies along, make their albums, leave the
    /// app the questions and what came of the albums, and record the folders that are done leaving.
    async fn follow_folder_copies(&self, pending: Option<&PendingFolders>) -> Result<(), BridgeError>
    where
        Source: AlbumTarget,
    {
        for (copy, albums) in self.take_answers() {
            self.with_copies(|copies| copies.answer(copy, albums));
        }
        let now = seconds_since_epoch(SystemTime::now());
        let choice = self.tree.config().upload_folders;
        let events = self.with_copies(|copies| copies.tick(now, choice, |top| self.folders.busy(top)));
        for event in events {
            if let FolderEvent::MakeAlbums(copy) = event {
                let mut due = self.folders.albums_due.lock().expect("albums due lock");
                if !due.contains(&copy) {
                    due.push(copy);
                }
            }
        }
        self.leave_questions();
        self.record_folder_departures(now, pending)
    }

    /// The copies whose albums are due, taken.
    fn take_albums_due(&self) -> Vec<u64> {
        std::mem::take(&mut *self.folders.albums_due.lock().expect("albums due lock"))
    }

    /// Make a copy's albums, recording each step as it happens. Dropped midway, the work is freed for a later poll,
    /// and what it did is in the record.
    async fn make_albums(&self, copy: u64)
    where
        Source: AlbumTarget,
    {
        let Some(work) = self.with_copies(|copies| copies.album_work(copy)) else { return };
        let mut freed = OnDrop(Some(|| self.with_copies(|copies| copies.abandon_work(copy))));
        let mut albums = Vec::new();
        let outcome = make_albums(self.source.as_ref(), &work, chrono::Local::now().date_naive(), |step| {
            if let AlbumProgress::Made { name: Some(name), .. } = &step {
                albums.push(name.clone());
            }
            self.with_copies(|copies| copies.album_progress(copy, &step));
        })
        .await;
        freed.defuse();
        let failure = outcome.err();
        self.with_copies(|copies| copies.albums_done(copy, failure.as_ref(), seconds_since_epoch(SystemTime::now())));
        if !albums.is_empty() {
            log::info!("made {} albums from a copied folder", albums.len());
            self.leave_notice(&FolderNotice::AlbumsMade { folder: work.folder.clone(), albums });
        }
        let folder = work.folder.clone();
        match failure {
            None => {}
            Some(error) if tried_again(&error) => log::info!("albums from a copied folder wait: {error}"),
            Some(SourceError::MissingPermission(permission)) => self.leave_notice(&FolderNotice::AlbumsNotAllowed {
                folder,
                permission: permission.unwrap_or_else(|| "album.create".to_owned()),
            }),
            Some(error) => {
                log::warn!("no albums from a copied folder: {error}");
                self.leave_notice(&FolderNotice::AlbumsFailed { folder, reason: error.to_string() });
            }
        }
    }

    /// The answers the app left, taken: each file is removed once read.
    fn take_answers(&self) -> Vec<(u64, bool)> {
        let Some(files) = &self.files else { return Vec::new() };
        let Ok(entries) = std::fs::read_dir(files) else { return Vec::new() };
        let mut answers = Vec::new();
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(copy) = name.strip_prefix(ANSWER_FILE_PREFIX).and_then(|rest| rest.strip_suffix(".json")).and_then(|copy| copy.parse().ok()) else {
                continue;
            };
            if let Ok(answer) = std::fs::read_to_string(entry.path()) {
                answers.push((copy, answer.trim() == "\"albums\""));
            }
            let _ = std::fs::remove_file(entry.path());
        }
        answers
    }

    /// Leave the app the questions waiting for an answer, in place of those left before.
    fn leave_questions(&self) {
        let Some(files) = &self.files else { return };
        let questions: Vec<FolderQuestion> = self.folders.copies.lock().expect("folder copies lock").questions().into_iter().map(FolderQuestion::from).collect();
        write_whole(&files.join(QUESTIONS_FILE), &serde_json::to_string(&questions).unwrap_or_default());
    }

    fn leave_notice(&self, notice: &FolderNotice) {
        let Some(files) = &self.files else { return };
        let stamp = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |since| since.as_nanos());
        write_whole(&files.join(format!("{NOTICE_FILE_PREFIX}{stamp}.json")), &serde_json::to_string(notice).unwrap_or_default());
    }

    /// Record in the journal the copied folders that are done: empty, untouched for a while, and with nothing in them
    /// uploading, leaving, kept by the system after it did not upload, or pending in the system. `pending` holds the
    /// folders macOS still holds items in that it has not handed over; when it is not known, they stay as long as
    /// their copy is remembered. The deepest go first.
    fn record_folder_departures(&self, now: u64, pending: Option<&PendingFolders>) -> Result<(), BridgeError> {
        let leaving: Vec<String> = leaving_upload(self.store().kept_state(LEAVING_UPLOAD).as_deref()).into_iter().map(|leaving| leaving.parent).collect();
        let stay = match pending {
            Some(pending) => FOLDER_STAY.max(Duration::from_secs(pending.refresh_seconds.into()) + PENDING_MARGIN),
            None => FOLDER_STAY_UNSURE,
        }
        .as_secs();
        let pending: &[String] = pending.map(|pending| pending.folders.as_slice()).unwrap_or_default();
        self.store()
            .update_state(UPLOAD_FOLDERS, |kept| {
                let mut folders = upload_folders(kept);
                let mut gone = Vec::new();
                loop {
                    let done: Vec<String> = folders
                        .iter()
                        .filter(|record| {
                            let top = NodeKey::from_storage_key(&record.key).and_then(|key| key.upload_path().and_then(<[String]>::first).cloned());
                            now >= record.touched.saturating_add(stay)
                                && record.waiting.is_empty()
                                && !pending.contains(&record.key)
                                && !folders.iter().any(|other| other.parent == record.key)
                                && !leaving.contains(&record.key)
                                && !top.is_some_and(|top| self.folders.busy(&top))
                        })
                        .map(|record| record.key.clone())
                        .collect();
                    if done.is_empty() {
                        break;
                    }
                    folders.retain(|record| !done.contains(&record.key));
                    gone.extend(done);
                }
                let changes = gone.iter().filter_map(|key| NodeKey::from_storage_key(key)).map(Change::Deleted).collect();
                ((!folders.is_empty()).then(|| serde_json::to_string(&folders).unwrap_or_default()), changes, ())
            })
            .map_err(failed)
    }

    /// Record in the journal the uploaded files due to leave the Upload folder at `now`, in the same transaction that
    /// takes them off the waiting list: an upload finishing meanwhile is neither lost nor recorded twice.
    fn record_departures(&self, now: u64) -> Result<(), BridgeError> {
        self.store()
            .update_state(LEAVING_UPLOAD, |kept| {
                let (due, waiting): (Vec<_>, Vec<_>) = leaving_upload(kept).into_iter().partition(|leaving| leaving.leaves_at <= now);
                let changes = due.iter().filter_map(|leaving| NodeKey::from_storage_key(&leaving.key)).map(Change::Deleted).collect();
                ((!waiting.is_empty()).then(|| serde_json::to_string(&waiting).unwrap_or_default()), changes, ())
            })
            .map_err(failed)
    }

    fn current_anchor(&self) -> Result<String, BridgeError> {
        Ok(self.store().journal_anchor().map_err(failed)?.to_text())
    }

    fn changes_since(&self, anchor: &str, limit: u32) -> Result<ChangeBatch, BridgeError> {
        let anchor = Anchor::from_text(anchor).ok_or(BridgeError::AnchorExpired)?;
        let (changes, anchor, more) = match self.store().journal_since(anchor, limit.max(1) as usize).map_err(failed)? {
            JournalRead::Changes { changes, anchor, more } => (changes, anchor, more),
            JournalRead::Expired => return Err(BridgeError::AnchorExpired),
        };
        // The latest record of each item wins: one that went away and came back in a batch is simply updated.
        let mut latest: Vec<(String, Option<Item>)> = Vec::new();
        for change in changes {
            let (identifier, item) = match change {
                Change::Updated { folder, entry } => (entry.node.storage_key(), Some(item_for(&entry, &folder, self.set_up))),
                Change::Deleted(node) => (node.storage_key(), None),
            };
            latest.retain(|(earlier, _)| *earlier != identifier);
            latest.push((identifier, item));
        }
        let (mut updated, mut deleted) = (Vec::new(), Vec::new());
        for (identifier, item) in latest {
            match item {
                Some(item) => updated.push(item),
                None => deleted.push(identifier),
            }
        }
        Ok(ChangeBatch { updated, deleted, anchor: anchor.to_text(), more })
    }

    /// Every item the system holds: the entries of every kept listing, and the folders copied into Upload.
    fn working_set(&self) -> Vec<Item> {
        let store = self.store();
        let mut items: Vec<Item> = store
            .kept_folders()
            .into_iter()
            .flat_map(|folder| store.kept_listing(&folder).unwrap_or_default().iter().map(|entry| item_for(entry, &folder, self.set_up)).collect::<Vec<_>>())
            .collect();
        items.extend(store.upload_folders(None));
        items
    }
}

/// Write a file whole, through a temporary one beside it, so a reader never sees half of it.
fn write_whole(path: &Path, contents: &str) {
    // A name of its own for each write: two writers at once (a poll and a look after an upload) never share one.
    static WRITES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let write = WRITES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let temporary = path.with_extension(format!("{}-{write}.partial", std::process::id()));
    if std::fs::write(&temporary, contents).and_then(|()| std::fs::rename(&temporary, path)).is_err() {
        log::info!("could not write {}", path.display());
        let _ = std::fs::remove_file(&temporary);
    }
}

/// What macOS says it holds back (its pending set): the folders holding items it has not handed over yet, and how often
/// it brings that list up to date.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PendingFolders {
    pub folders: Vec<String>,
    pub refresh_seconds: u32,
}

/// A copied folder that settled, for the app to ask about (ARCHITECTURE.md, "Folders copied into Upload").
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record, serde::Serialize, serde::Deserialize)]
pub struct FolderQuestion {
    /// Which copy it is, for the answer.
    pub copy: u64,
    /// The copied folder's name.
    pub folder: String,
    /// Photos and videos uploaded from it.
    pub uploaded: u32,
    /// Files in it that did not upload.
    pub not_uploaded: u32,
    /// The albums it would make, by their names before any clash is settled.
    pub albums: Vec<String>,
}

impl From<immich_mount_core::folder_uploads::FolderQuestion> for FolderQuestion {
    fn from(question: immich_mount_core::folder_uploads::FolderQuestion) -> FolderQuestion {
        FolderQuestion {
            copy: question.id,
            folder: question.folder,
            uploaded: question.uploaded,
            not_uploaded: question.not_uploaded,
            albums: question.albums,
        }
    }
}

/// What came of a copied folder's albums, for the app to tell.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum, serde::Serialize, serde::Deserialize)]
pub enum FolderNotice {
    /// Albums were made, with these names.
    AlbumsMade { folder: String, albums: Vec<String> },
    /// The account may not make albums: the copy stays in the timeline only.
    AlbumsNotAllowed { folder: String, permission: String },
    /// The albums could not be made, for this reason (as the server or the system gave it): the copy stays in the
    /// timeline only.
    AlbumsFailed { folder: String, reason: String },
}

/// The questions about copied folders waiting for an answer, as the extension left them in the library's
/// `directory`.
#[uniffi::export]
pub fn folder_questions(directory: String) -> Vec<FolderQuestion> {
    std::fs::read_to_string(Path::new(&directory).join(QUESTIONS_FILE)).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
}

/// Answer the question about copy `copy`: albums too, or the timeline only. The extension takes the answer at its
/// next poll.
#[uniffi::export]
pub fn answer_folder_question(directory: String, copy: u64, make_albums: bool) {
    let answer = if make_albums { "\"albums\"" } else { "\"timeline\"" };
    write_whole(&Path::new(&directory).join(format!("{ANSWER_FILE_PREFIX}{copy}.json")), answer);
}

/// What came of copied folders' albums since the app last asked, taken: each is told once.
#[uniffi::export]
pub fn take_folder_notices(directory: String) -> Vec<FolderNotice> {
    let Ok(entries) = std::fs::read_dir(&directory) else { return Vec::new() };
    let mut found: Vec<(String, PathBuf)> = entries
        .flatten()
        .map(|entry| (entry.file_name().to_string_lossy().into_owned(), entry.path()))
        .filter(|(name, _)| name.starts_with(NOTICE_FILE_PREFIX) && name.ends_with(".json"))
        .collect();
    found.sort();
    found
        .into_iter()
        .filter_map(|(_, path)| {
            let notice = std::fs::read_to_string(&path).ok().and_then(|text| serde_json::from_str(&text).ok());
            let _ = std::fs::remove_file(&path);
            notice
        })
        .collect()
}

/// Uploaded files waiting to leave the Upload folder, as kept.
fn leaving_upload(kept: Option<&str>) -> Vec<Leaving> {
    kept.and_then(|text| serde_json::from_str(text).ok()).unwrap_or_default()
}

fn too_slow() -> BridgeError {
    BridgeError::ServerUnreachable { message: "the server did not answer in time".to_owned() }
}

/// The library behind the Finder location: one account on one server.
#[derive(uniffi::Object)]
pub struct Library {
    /// Replaced when the settings change; requests in flight finish with the one they started with.
    answers: RwLock<Arc<Answers<ImmichSource, Arc<MetadataStore>>>>,
    source: Arc<ImmichSource>,
    store: Arc<MetadataStore>,
    /// The text of `config.toml` the answers follow.
    config: Mutex<String>,
    /// When the location was set up (`Answers::set_up`).
    set_up: SystemTime,
    /// The folders copied into Upload, followed across new settings.
    folders: Arc<FolderFollowing>,
    /// The library's directory, where the store is and where the questions about copied folders pass.
    directory: PathBuf,
    /// The server's address as signed in, which web addresses start with.
    server: String,
    /// The signed-in user's Immich id, which crosslinks carry, once the server told it.
    user_id: Mutex<Option<String>>,
}

/// Look at the copied folders again once a copy could have settled, rather than at the next poll, up to ten seconds
/// later: the question reaches the app sooner. The poll still does the same, for an extension stopped meanwhile.
fn look_again_once_settled(answers: Arc<Answers<ImmichSource, Arc<MetadataStore>>>) {
    runtime().spawn(async move {
        tokio::time::sleep(Duration::from_secs(immich_mount_core::folder_uploads::SETTLE_SECONDS + 1)).await;
        if let Err(error) = answers.follow_folder_copies(None).await {
            log::info!("the copied folders were not looked at: {error}");
        }
        for copy in answers.take_albums_due() {
            let answers = answers.clone();
            runtime().spawn(async move { answers.make_albums(copy).await });
        }
    });
}

/// The settings in `config.toml`, as the macOS edition follows them.
fn parse_config(text: &str) -> Result<Config, BridgeError> {
    Config::from_toml(text).map_err(failed)
}

fn seconds_since_epoch(time: SystemTime) -> u64 {
    time.duration_since(SystemTime::UNIX_EPOCH).map_or(0, |since| since.as_secs())
}

impl Library {
    fn answers(&self) -> Arc<Answers<ImmichSource, Arc<MetadataStore>>> {
        self.answers.read().expect("answers lock").clone()
    }
}

/// What an account signs in with.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum Credential {
    /// An API key, made in Immich's account settings.
    ApiKey { key: String },
    /// A session from a password or browser sign-in.
    Session { token: String },
}

impl Credential {
    fn for_client(&self) -> ApiCredential {
        match self {
            Credential::ApiKey { key } => ApiCredential::ApiKey(key.trim().to_owned()),
            Credential::Session { token } => ApiCredential::Session(token.trim().to_owned()),
        }
    }

    fn secret(&self) -> &str {
        match self {
            Credential::ApiKey { key } => key.trim(),
            Credential::Session { token } => token.trim(),
        }
    }
}

/// The libraries open in this process, by directory and credential: the system may make a new extension object
/// before it lets go of the old one, and the metadata store can be opened only once.
type OpenLibraries = Mutex<HashMap<(PathBuf, String, String), Weak<Library>>>;

fn open_libraries() -> &'static OpenLibraries {
    static OPEN: OnceLock<OpenLibraries> = OnceLock::new();
    OPEN.get_or_init(OpenLibraries::default)
}

#[uniffi::export]
impl Library {
    /// Open one account's library. `directory` holds its metadata store (in the App Group container), and `config`
    /// is the text of `config.toml`, empty for the defaults.
    #[uniffi::constructor]
    pub fn open(server: String, credential: Credential, directory: String, config: String) -> Result<Arc<Library>, BridgeError> {
        let (server, directory) = (normalised_address(&server), PathBuf::from(directory));
        let mut open = open_libraries().lock().expect("open libraries lock");
        open.retain(|_, library| library.strong_count() > 0);
        let key = (directory.clone(), server.clone(), credential.secret().to_owned());
        if let Some(library) = open.get(&key).and_then(Weak::upgrade) {
            return Ok(library);
        }
        let parsed = parse_config(&config)?;
        let client = Client::new(&server, Some(credential.for_client()), USER_AGENT).map_err(failed)?;
        std::fs::create_dir_all(&directory).map_err(|error| failed(format!("{}: {error}", directory.display())))?;
        let store = MetadataStore::open(&directory.join("metadata.redb")).map_err(failed)?.with_journal(JOURNAL_LIMIT).map_err(failed)?;
        let store = Arc::new(store);
        let set_up = set_up_time(&directory.join("metadata.redb"));
        let source = Arc::new(ImmichSource::new(client));
        let folders = Arc::new(FolderFollowing::from_kept(store.kept_state(FOLDER_COPIES).as_deref()));
        let answers = Answers::new(source.clone(), store.clone(), parsed).set_up_at(set_up).following(folders.clone(), Some(directory.clone()));
        *answers.kept_poll.lock().expect("kept poll lock") = store.kept_state(POLL_MEMORY);
        let library = Arc::new(Library {
            answers: RwLock::new(Arc::new(answers)),
            source,
            store,
            config: Mutex::new(config),
            set_up,
            folders,
            directory: directory.clone(),
            server,
            user_id: Mutex::new(None),
        });
        open.insert(key, Arc::downgrade(&library));
        Ok(library)
    }

    /// The item behind an identifier. Each request takes an optional `Cancel`, which stops it.
    pub async fn item(&self, identifier: String, cancel: Option<Arc<Cancel>>) -> Result<Item, BridgeError> {
        let answers = self.answers();
        cancellable(cancel, async move { answers.item(&identifier).await }).await
    }

    /// The items a folder lists.
    pub async fn list(&self, identifier: String, cancel: Option<Arc<Cancel>>) -> Result<Vec<Item>, BridgeError> {
        let answers = self.answers();
        cancellable(cancel, async move { answers.list(&identifier).await }).await
    }

    /// Download a file's original into `destination`, and return the item as it is now. On failure no file is left.
    pub async fn fetch(&self, identifier: String, destination: String, cancel: Option<Arc<Cancel>>) -> Result<Item, BridgeError> {
        let answers = self.answers();
        let (stopped, path) = (destination.clone(), destination);
        let result = cancellable(cancel, async move { answers.fetch(&identifier, Path::new(&path)).await }).await;
        if result.is_err() {
            let _ = std::fs::remove_file(stopped);
        }
        result
    }

    /// A file's thumbnail for a size in pixels (its longer side), as image bytes: WebP or JPEG, as Immich made it.
    /// `None` for a folder, which keeps the system's own icon.
    pub async fn thumbnail(&self, identifier: String, pixels: u32, cancel: Option<Arc<Cancel>>) -> Result<Option<Vec<u8>>, BridgeError> {
        let answers = self.answers();
        cancellable(cancel, async move { answers.thumbnail(&identifier, pixels).await }).await
    }

    /// Ask the server what changed, and record in the journal what changed for the folders the system holds.
    /// `pending` is macOS's pending set, `None` when it did not say, or said its list was full: a copied folder leaves
    /// Upload soon only once it holds none of it. Returns whether anything changed.
    pub async fn poll(&self, pending: Option<PendingFolders>, cancel: Option<Arc<Cancel>>) -> Result<bool, BridgeError> {
        let answers = self.answers();
        let polling = answers.clone();
        let result = cancellable(cancel, async move { polling.poll(pending.as_ref()).await }).await;
        // Albums are made in tasks of their own: the system cancels a poll it no longer waits for, and album work cut
        // short there would wait for the next one.
        for copy in answers.take_albums_due() {
            let answers = answers.clone();
            runtime().spawn(async move { answers.make_albums(copy).await });
        }
        result
    }

    /// The item's address in Immich's web app, for Finder's Open in Immich and Copy Immich Link; `None` where Immich has
    /// no page for it (ROADMAP.md, "Open in Immich", "The item map").
    pub fn web_link(&self, identifier: String) -> Option<String> {
        links::web_address(&self.server, &NodeKey::from_storage_key(&identifier)?)
    }

    /// The item's crosslink, for Finder's Show in actions (immich-crosslink, "Sending"), with the signed-in user's id
    /// once the server tells it (an API key without `user.read` does not); `None` where Immich has no page for it.
    pub async fn crosslink(&self, identifier: String) -> Option<String> {
        let node = NodeKey::from_storage_key(&identifier)?;
        let known = self.user_id.lock().expect("user id lock").clone();
        let user_id = match known {
            Some(user_id) => Some(user_id),
            None => {
                let source = self.source.clone();
                let asked = on_runtime(async move {
                    match tokio::time::timeout(SIGN_IN_LIMIT, source.client().me()).await {
                        Ok(Ok(me)) => Ok(me.id),
                        _ => Err(too_slow()),
                    }
                })
                .await
                .ok();
                if let Some(user_id) = &asked {
                    *self.user_id.lock().expect("user id lock") = Some(user_id.clone());
                }
                asked
            }
        };
        links::crosslink(&self.server, user_id.as_deref(), &node)
    }

    /// The journal's anchor now, as the system's sync anchor.
    pub fn current_anchor(&self) -> Result<String, BridgeError> {
        self.answers().current_anchor()
    }

    /// The changes after `anchor`, at most `limit` records of them; `AnchorExpired` when they are no longer kept.
    pub fn changes_since(&self, anchor: String, limit: u32) -> Result<ChangeBatch, BridgeError> {
        self.answers().changes_since(&anchor, limit)
    }

    /// Every item the system holds, for when its anchor expired and it enumerates the working set again.
    pub fn working_set(&self) -> Vec<Item> {
        self.answers().working_set()
    }

    /// The server's Immich version, as `v3.3.5`, for the app's status.
    pub async fn server_version(&self) -> Result<String, BridgeError> {
        let source = self.source.clone();
        on_runtime(async move {
            let version = source.client().server_version().await.map_err(immich_mount_core::immich::source_error)?;
            Ok(version.to_string())
        })
        .await
    }

    /// Upload the finished file at `path`, added to `folder` (only the Upload folder takes files) as `name`, and
    /// return its item there. Refused, with a message, when it is no photo or video Immich takes, or not whole.
    pub async fn upload(
        &self,
        folder: String,
        name: String,
        path: String,
        modified: SystemTime,
        cancel: Option<Arc<Cancel>>,
    ) -> Result<Item, BridgeError> {
        let answers = self.answers();
        let copied = matches!(NodeKey::from_storage_key(&folder), Some(NodeKey::UploadFolder(_)));
        let uploading = answers.clone();
        let result = cancellable(cancel, async move { uploading.upload(&folder, &name, Path::new(&path), modified).await }).await;
        if copied {
            look_again_once_settled(answers);
        }
        result
    }

    /// Make the folder `name` inside `parent`, as Finder copies a folder into Upload or into a folder copied there,
    /// and return its item. Refused anywhere else, and for a hidden folder.
    pub fn create_folder(&self, parent: String, name: String) -> Result<Item, BridgeError> {
        self.answers().create_folder(&parent, &name)
    }

    /// Forget a copied folder Finder deleted. Refused for every other folder, which is a view of the library.
    pub fn delete_folder(&self, identifier: String) -> Result<(), BridgeError> {
        self.answers().delete_folder(&identifier)
    }

    /// Follow the settings in `config` (the text of `config.toml`) from now on, when they differ from those in use:
    /// folders are listed by the new rules on their next visit. Returns whether they differed.
    pub fn apply_config(&self, config: String) -> Result<bool, BridgeError> {
        let mut current = self.config.lock().expect("config lock");
        if *current == config {
            return Ok(false);
        }
        let answers = Answers::new(self.source.clone(), self.store.clone(), parse_config(&config)?)
            .set_up_at(self.set_up)
            .following(self.folders.clone(), Some(self.directory.clone()));
        *self.answers.write().expect("answers lock") = Arc::new(answers);
        *current = config;
        Ok(true)
    }
}

/// The sections, in the order the Folders pane lists them: the key of the switch that shows each, the key of its
/// date folders where it has them, and the folder name it shows in Finder (never translated: the drive's folders
/// keep their names in every language, SPEC.md, "Names"). Upload is one of them: its switch shows or hides the
/// folder, and with it what is copied there being uploaded; it has no date folders.
const SECTIONS: &[(&str, Option<&str>, &str)] = &[
    ("sections.photos", Some("grouping.photos"), "Photos"),
    ("sections.albums", Some("grouping.albums"), "Albums"),
    ("sections.people", Some("grouping.people"), "People"),
    ("sections.places", Some("grouping.places"), "Places"),
    ("sections.favorites", Some("grouping.favorites"), "Favorites"),
    ("sections.archive", Some("grouping.archive"), "Archive"),
    ("uploads.enabled", None, "Upload"),
];

/// What the Settings window's Folders pane shows (ARCHITECTURE.md, "The macOS edition", "Settings"): the settings
/// table's entries that mean something on macOS. The mount's group is Linux's: on macOS Finder's sidebar
/// shows the app's name for a provider's only location, so its name (`mount.name`) would change nothing there, and
/// its path and mounting at login are Linux's too; so are the cache and thumbnail seeding.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SettingsPanes {
    /// Each section's switch beside its date folders, Upload last.
    pub sections: Vec<SectionSettings>,
    /// How album folders are named and grouped (`albums.layout`).
    pub album_layout: Preference,
    /// How files are named (`files.naming`).
    pub file_naming: Preference,
    /// What a folder copied into Upload becomes (`uploads.folders`).
    pub upload_folders: Preference,
    /// Immich crosslink's switch (`crosslink.enabled`), in the General pane.
    pub crosslink: Preference,
}

/// One section of the drive, as a row of the Folders pane.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SectionSettings {
    /// The section's folder name in Finder, untranslated.
    pub name: String,
    /// Whether Finder shows it (`sections.<section>`, or `uploads.enabled`).
    pub shown: Preference,
    /// Its date folders (`grouping.<section>`): day folders inside months for Photos, year and month folders for
    /// the others; none for Upload.
    pub date_folders: Option<Preference>,
}

/// One setting of the core's settings table (SPEC.md, "Configuration"), with its value.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Preference {
    /// Its key in `config.toml`.
    pub key: String,
    pub title: String,
    pub kind: PreferenceKind,
    /// As stored, or the default.
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum PreferenceKind {
    /// `true` or `false`.
    Toggle,
    /// One of these words, each with how it reads.
    Choice { options: Vec<PreferenceOption> },
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PreferenceOption {
    pub value: String,
    pub label: String,
}

/// The Settings window's settings, with their values in `config`, the text of `config.toml`.
#[uniffi::export]
pub fn settings_panes(config: String) -> Result<SettingsPanes, BridgeError> {
    let stored = settings::stored_values(&config).unwrap_or_default();
    let preference = |key: &str| -> Result<Preference, BridgeError> {
        let setting = SETTINGS
            .iter()
            .find(|setting| setting.key == key)
            .ok_or_else(|| BridgeError::Failed { message: format!("the settings table has no `{key}`") })?;
        let kind = match setting.kind {
            Kind::Bool => PreferenceKind::Toggle,
            Kind::Choice(words) => PreferenceKind::Choice {
                options: words
                    .iter()
                    .map(|word| PreferenceOption { value: (*word).to_owned(), label: choice_label(word).to_owned() })
                    .collect(),
            },
            Kind::Text | Kind::Path | Kind::Size => {
                return Err(BridgeError::Failed { message: format!("`{key}` is not a switch or a choice") })
            }
        };
        let value = stored.get(setting.key).cloned().unwrap_or_else(|| setting.default.to_owned());
        Ok(Preference { key: setting.key.to_owned(), title: setting.description.to_owned(), kind, value })
    };
    let sections = SECTIONS
        .iter()
        .map(|(shown, date_folders, name)| {
            Ok(SectionSettings {
                name: (*name).to_owned(),
                shown: preference(shown)?,
                date_folders: date_folders.map(preference).transpose()?,
            })
        })
        .collect::<Result<Vec<_>, BridgeError>>()?;
    Ok(SettingsPanes {
        sections,
        album_layout: preference("albums.layout")?,
        file_naming: preference("files.naming")?,
        upload_folders: preference("uploads.folders")?,
        crosslink: preference("crosslink.enabled")?,
    })
}

/// The Finder location's name in the sidebar (`mount.name`), the default when `config` does not say or cannot be read.
#[uniffi::export]
pub fn drive_name(config: String) -> String {
    settings::Config::from_toml(&config).map_or(settings::DEFAULT_MOUNT_NAME, |config| config.mount_name).to_owned()
}

/// `config` with one setting changed, its comments kept; refused with a message when the value does not fit.
#[uniffi::export]
pub fn set_preference(config: String, key: String, value: String) -> Result<String, BridgeError> {
    settings::set_value(&config, &key, &value).map_err(failed)
}

// Signing in, as on Linux: the core's `sign_in` (ARCHITECTURE.md, "Signing in"). Its errors are messages for people.

/// What a server offers before anyone signs in.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ServerOffer {
    /// The address as it is used: `https://` added when none was typed.
    pub address: String,
    /// Immich's version, as `v3.3.5`.
    pub version: String,
    /// The text of the server's single sign-on button, when it offers browser sign-in.
    pub browser_button: Option<String>,
    /// Whether it takes a password.
    pub password: bool,
}

/// A finished sign-in: the credential to keep, and the account's email when the sign-in told it.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SignedIn {
    pub credential: Credential,
    pub account: Option<String>,
}

fn anonymous(server: &str) -> Result<Client, BridgeError> {
    Client::new(server, None, USER_AGENT).map_err(failed)
}

/// A step of signing in, with its own limit.
async fn in_time<Output>(step: impl Future<Output = Result<Output, String>>) -> Result<Output, BridgeError> {
    match tokio::time::timeout(SIGN_IN_LIMIT * 3, step).await {
        Ok(result) => result.map_err(failed),
        Err(_) => Err(too_slow()),
    }
}

/// Check the server at an address, and what sign-in methods it offers. The app calls this before the extension ever
/// runs, so macOS's Local Network prompt names the app (ARCHITECTURE.md, "The macOS edition"); a connection that
/// fails is tried again while that prompt may be open.
#[uniffi::export]
pub async fn probe(address: String) -> Result<ServerOffer, BridgeError> {
    on_runtime(async move {
        let address = normalised_address(&address);
        let client = anonymous(&address)?;
        let started = Instant::now();
        while started.elapsed() < LOCAL_NETWORK_PATIENCE {
            match tokio::time::timeout(SIGN_IN_LIMIT, client.server_version()).await {
                Ok(Err(ApiError::Offline(_))) | Err(_) => tokio::time::sleep(Duration::from_secs(1)).await,
                _ => break,
            }
        }
        let offer = in_time(sign_in::probe(&client, &address)).await?;
        Ok(ServerOffer { address: offer.url, version: offer.version, browser_button: offer.browser_button, password: offer.password })
    })
    .await
}

#[uniffi::export]
pub async fn sign_in_with_api_key(server: String, key: String) -> Result<SignedIn, BridgeError> {
    on_runtime(async move {
        let credential = Credential::ApiKey { key: key.trim().to_owned() };
        let client = Client::new(&normalised_address(&server), Some(credential.for_client()), USER_AGENT).map_err(failed)?;
        in_time(sign_in::check_api_key(&client)).await?;
        Ok(SignedIn { credential, account: None })
    })
    .await
}

#[uniffi::export]
pub async fn sign_in_with_password(server: String, email: String, password: String) -> Result<SignedIn, BridgeError> {
    on_runtime(async move {
        let client = anonymous(&normalised_address(&server))?;
        let session = in_time(sign_in::password_session(&client, &email, &password)).await?;
        Ok(SignedIn { credential: Credential::Session { token: session.token }, account: Some(session.account) })
    })
    .await
}

/// One browser sign-in. Its secrets stay here; the app opens `address()` in a web authentication session that ends
/// at the address Immich's mobile app registers (`app.immich:///oauth-callback`), and hands that back to `finish`.
#[derive(uniffi::Object)]
pub struct BrowserAttempt {
    server: String,
    secrets: BrowserSignIn,
    address: String,
}

#[uniffi::export]
pub async fn begin_browser_sign_in(server: String) -> Result<Arc<BrowserAttempt>, BridgeError> {
    on_runtime(async move {
        let server = normalised_address(&server);
        let secrets = BrowserSignIn::new().map_err(failed)?;
        let address = in_time(sign_in::begin_browser(&anonymous(&server)?, &secrets.state, &secrets.code_challenge)).await?;
        Ok(Arc::new(BrowserAttempt { server, secrets, address }))
    })
    .await
}

#[uniffi::export]
impl BrowserAttempt {
    /// The identity provider's address to open.
    pub fn address(&self) -> String {
        self.address.clone()
    }

    /// The scheme the sign-in ends at, which the web authentication session waits for.
    pub fn callback_scheme(&self) -> String {
        sign_in::REDIRECT_URI.split("://").next().unwrap_or_default().to_owned()
    }

    /// Finish with the address the browser was sent to.
    pub async fn finish(&self, redirect: String) -> Result<SignedIn, BridgeError> {
        if let Some(refusal) = sign_in::provider_error(&redirect, &self.secrets.state) {
            return Err(failed(refusal));
        }
        let (server, secrets) = (self.server.clone(), self.secrets.clone());
        on_runtime(async move {
            let session = in_time(sign_in::finish_browser(&anonymous(&server)?, &redirect, &secrets)).await?;
            Ok(SignedIn { credential: Credential::Session { token: session.token }, account: Some(session.account) })
        })
        .await
    }
}

/// End a session on the server, as signing out does; an API key has nothing to end. Signing out goes on whatever the
/// server says.
#[uniffi::export]
pub async fn end_session(server: String, credential: Credential) {
    let ended = on_runtime(async move {
        if matches!(credential, Credential::Session { .. }) {
            let client = Client::new(&normalised_address(&server), Some(credential.for_client()), USER_AGENT).map_err(failed)?;
            let _ = tokio::time::timeout(SIGN_IN_LIMIT, client.logout()).await;
        }
        Ok(())
    })
    .await;
    if let Err(error) = ended {
        log::info!("the session was not ended on the server: {error}");
    }
}

// Received links (ROADMAP.md, "Open in Immich, and links between apps", step 2): the app takes a crosslink, or a web
// address on the signed-in server, and has Finder show its item. The app has no metadata store (only the extension
// opens it), so it finds the item's key with a tree of its own; keys do not depend on the names kept there, and the
// key's storage key is the item's identifier in the extension.

/// What came of a received link: Linux's results, with the item to show or the host the link names.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ShowLinkOutcome {
    /// `show`, `invalid`, `cannot-show`, `other-server`, `other-user`, `not-found`, `off` or `unreachable`, as Linux's
    /// daemon answers.
    pub result: String,
    /// The item's identifier, for `show`.
    pub identifier: Option<String>,
    /// The host (and port) the link names, for `other-server`; empty otherwise.
    pub detail: String,
}

impl ShowLinkOutcome {
    fn refused(result: &str) -> ShowLinkOutcome {
        ShowLinkOutcome { result: result.to_owned(), identifier: None, detail: String::new() }
    }
}

/// How long finding a received link's item may take, as on Linux.
const SHOW_LINK_LIMIT: Duration = Duration::from_secs(20);

/// Where the account's library shows what `link` names, read and checked as immich-crosslink's receivers must: the
/// server must be the signed-in one, and so must the user when the link names one and the account may read its own
/// id. `config` is the text of `config.toml`: with `crosslink.enabled` off, nothing is read. Only ever finds: nothing is
/// fetched but listings and, for a photo, its details.
#[uniffi::export]
pub async fn show_link(server: String, credential: Credential, config: String, link: String) -> ShowLinkOutcome {
    let finding = on_runtime(async move {
        let server = normalised_address(&server);
        let config = parse_config(&config)?;
        let enabled = config.crosslink_enabled;
        let client = Client::new(&server, Some(credential.for_client()), USER_AGENT).map_err(failed)?;
        let source = Arc::new(ImmichSource::new(client));
        let asking = source.clone();
        // An API key without `user.read` cannot tell its id: the link's user is then not checked.
        let user_id = async move { tokio::time::timeout(SIGN_IN_LIMIT, asking.client().me()).await.ok()?.ok().map(|me| me.id) };
        let tree = Tree::new(source, immich_mount_core::tree::InMemoryNames::default(), config);
        Ok(follow_link(&tree, &link, &server, enabled, user_id).await)
    })
    .await;
    finding.unwrap_or_else(|error| {
        log::info!("a received link was not followed: {error}");
        ShowLinkOutcome::refused("unreachable")
    })
}

/// `show_link`'s work, on any tree. `user_id` is asked only for a link that names a user.
async fn follow_link<Source, Names>(
    tree: &Tree<Source, Names>,
    link: &str,
    server: &str,
    enabled: bool,
    user_id: impl Future<Output = Option<String>>,
) -> ShowLinkOutcome
where
    Source: LibrarySource,
    Names: NameMemory,
{
    if !enabled {
        return ShowLinkOutcome::refused("off");
    }
    let refused = |refusal: links::Refusal| match refusal {
        links::Refusal::OtherServer(host) => ShowLinkOutcome { detail: host, ..ShowLinkOutcome::refused("other-server") },
        refusal => ShowLinkOutcome::refused(refusal.as_str()),
    };
    let mut item = match links::receive(link, server, None) {
        Ok(item) => item,
        Err(refusal) => return refused(refusal),
    };
    if matches!(links::read_crosslink(link.trim()), links::Reading::Show(links::Crosslink { user_id: Some(_), .. })) {
        if let Some(user_id) = user_id.await {
            item = match links::receive(link, server, Some(&user_id)) {
                Ok(item) => item,
                Err(refusal) => return refused(refusal),
            };
        }
    }
    // Its path from the root as well, as Linux finds it: a folder the root no longer lists (a section switched off)
    // still has its key. The system finds the item's place itself (`getUserVisibleURL`), listing the folders above
    // it where they were never listed.
    let found = async {
        let Some(key) = tree.locate(&item).await? else { return Ok(None) };
        Ok(tree.path_of(&key).await?.map(|_| key))
    };
    match tokio::time::timeout(SHOW_LINK_LIMIT, found).await {
        Ok(Ok(Some(key))) => ShowLinkOutcome { result: "show".to_owned(), identifier: Some(key.storage_key()), detail: String::new() },
        Ok(Ok(None)) | Ok(Err(SourceError::NotFound)) => ShowLinkOutcome::refused("not-found"),
        Ok(Err(error)) => {
            log::info!("a received link could not be followed: {error}");
            ShowLinkOutcome::refused("unreachable")
        }
        Err(_) => ShowLinkOutcome::refused("unreachable"),
    }
}

// Show in other apps (ROADMAP.md, "Open in Immich, and links between apps", step 3): the app finds the apps that take
// crosslinks and reads their announcements; which of them Finder offers, and on what, is decided here.

/// The draft Immich Mount sends: an app announcing another is treated as not announcing at all.
const CROSSLINK_DRAFT: &str = "0.2";
/// The kinds of item the protocol names; others in an announcement are ignored.
const CROSSLINK_KINDS: &[&str] = &["asset", "album", "person", "place", "page"];
/// Immich Mount's own builds, never offered.
const OWN_BUNDLES: &[&str] = &["dev.hviid.ImmichMount", "dev.hviid.ImmichMount.Dev"];
/// The known apps (ROADMAP.md, step 3, "Known apps"): Immich Gallery, by its bundle ids, release first, each with the
/// scheme of its own links. It opens `<scheme>://photo/<id>` for a photo, as its desktop widget does
/// (`Shared/WidgetFeed.swift` in IGOLz/ImmichGallery).
const GALLERY_BUNDLES: &[(&str, &str)] = &[("dev.igol.ImmichGallery", "immichgallery"), ("dev.igol.ImmichGallery.Dev", "immichgallery-dev")];

/// An app the system knows: one registered for `immich-crosslink`, or a known app.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FoundApp {
    pub bundle_id: String,
    /// Where it is installed.
    pub path: String,
    /// Whether it is registered for the scheme `immich-crosslink`.
    pub takes_crosslinks: bool,
    /// Its announcement (`ImmichCrosslink` in its Info.plist): the draft, and the kinds it shows.
    pub version: Option<String>,
    pub shows: Vec<String>,
}

/// An app Finder offers.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct OfferedApp {
    pub bundle_id: String,
    pub path: String,
    /// The kinds of item it is offered on.
    pub kinds: Vec<String>,
    /// For a known app that does not announce itself: the scheme of its own links, which it is sent instead of a
    /// crosslink, and only for a photo in Photos.
    pub own_scheme: Option<String>,
}

/// An app the Settings window lists, with its switch.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ListedApp {
    pub bundle_id: String,
    pub path: String,
    /// Whether Show in offers it (it is not in `crosslink.hidden`).
    pub shown: bool,
}

/// What Finder offers: Show in Immich Gallery, and the apps of Show in Another App…; and what Settings lists.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CrosslinkTargets {
    pub gallery: Option<OfferedApp>,
    pub others: Vec<OfferedApp>,
    pub listed: Vec<ListedApp>,
}

/// Which of the apps found Finder offers, by `config` (`crosslink.enabled`, `crosslink.hidden`). `development`: this
/// is Immich Mount Dev, which prefers Immich Gallery's Dev build where both are installed. An app installed in several
/// places is taken from Applications where it is there.
#[uniffi::export]
pub fn crosslink_targets(found: Vec<FoundApp>, config: String, development: bool) -> CrosslinkTargets {
    let config = parse_config(&config).unwrap_or_else(|_| Config::from_toml("").expect("the defaults parse"));
    let hidden = |bundle_id: &str| config.crosslink_hidden.split(';').any(|hidden| hidden.trim() == bundle_id);
    let mut apps: Vec<FoundApp> = Vec::new();
    for app in found {
        if OWN_BUNDLES.contains(&app.bundle_id.as_str()) {
            continue;
        }
        match apps.iter_mut().find(|kept| kept.bundle_id == app.bundle_id) {
            Some(kept) => {
                kept.takes_crosslinks |= app.takes_crosslinks;
                if !kept.path.starts_with("/Applications/") && app.path.starts_with("/Applications/") {
                    kept.path = app.path;
                }
            }
            None => apps.push(app),
        }
    }
    let announced = |app: &FoundApp| -> Option<Vec<String>> {
        (app.takes_crosslinks && app.version.as_deref() == Some(CROSSLINK_DRAFT))
            .then(|| app.shows.iter().filter(|kind| CROSSLINK_KINDS.contains(&kind.as_str())).cloned().collect())
    };
    let mut galleries: Vec<&(&str, &str)> = GALLERY_BUNDLES.iter().collect();
    if development {
        galleries.reverse();
    }
    let gallery = galleries.into_iter().find_map(|(bundle_id, scheme)| {
        let app = apps.iter().find(|app| app.bundle_id == *bundle_id)?;
        Some(match announced(app) {
            Some(kinds) => OfferedApp { bundle_id: app.bundle_id.clone(), path: app.path.clone(), kinds, own_scheme: None },
            None => OfferedApp { bundle_id: app.bundle_id.clone(), path: app.path.clone(), kinds: vec!["asset".to_owned()], own_scheme: Some((*scheme).to_owned()) },
        })
    });
    let listed = apps
        .iter()
        .filter(|app| announced(app).is_some() || gallery.as_ref().is_some_and(|gallery| gallery.bundle_id == app.bundle_id))
        .map(|app| ListedApp { bundle_id: app.bundle_id.clone(), path: app.path.clone(), shown: !hidden(&app.bundle_id) })
        .collect();
    if !config.crosslink_enabled {
        return CrosslinkTargets { gallery: None, others: Vec::new(), listed };
    }
    let others = apps
        .iter()
        .filter(|app| !hidden(&app.bundle_id) && gallery.as_ref().is_none_or(|gallery| gallery.bundle_id != app.bundle_id))
        .filter_map(|app| {
            let kinds = announced(app).filter(|kinds| !kinds.is_empty())?;
            Some(OfferedApp { bundle_id: app.bundle_id.clone(), path: app.path.clone(), kinds, own_scheme: None })
        })
        .collect();
    CrosslinkTargets { gallery: gallery.filter(|gallery| !hidden(&gallery.bundle_id) && !gallery.kinds.is_empty()), others, listed }
}

/// `config` with the app `bundle_id` left out of Show in (`crosslink.hidden`), or put back.
#[uniffi::export]
pub fn set_app_shown(config: String, bundle_id: String, shown: bool) -> Result<String, BridgeError> {
    let stored = settings::stored_values(&config).map_err(failed)?.get("crosslink.hidden").cloned().unwrap_or_default();
    let mut hidden: Vec<&str> = stored.split(';').map(str::trim).filter(|id| !id.is_empty() && *id != bundle_id).collect();
    if !shown {
        hidden.push(&bundle_id);
    }
    if hidden.is_empty() {
        // A text setting takes no empty value: with no app left out, the key goes.
        return settings::unset_value(&config, "crosslink.hidden").map_err(failed);
    }
    settings::set_value(&config, "crosslink.hidden", &hidden.join(";")).map_err(failed)
}

/// The kind of item behind an identifier, as immich-crosslink names it (`asset`, `album`, `person`, `place`, `page`);
/// `None` where Immich has no page for it.
#[uniffi::export]
pub fn item_kind(identifier: String) -> Option<String> {
    links::kind(&NodeKey::from_storage_key(&identifier)?).map(|kind| kind.as_str().to_owned())
}

/// The Asset's id when the item is a photo in Photos, which Immich Gallery's own link opens.
#[uniffi::export]
pub fn photo_in_photos(identifier: String) -> Option<String> {
    match NodeKey::from_storage_key(&identifier)? {
        NodeKey::Asset { parent, asset_id } if matches!(*parent, NodeKey::TimelineMonth(_) | NodeKey::TimelineDay(..)) => Some(asset_id),
        _ => None,
    }
}

/// The kind of item a crosslink names, when it is a valid link the protocol defines; for the menu of Show in Another
/// App…, which lists the apps that show that kind.
#[uniffi::export]
pub fn crosslink_kind(link: String) -> Option<String> {
    match links::read_crosslink(&link) {
        links::Reading::Show(crosslink) => Some(crosslink.item.kind().as_str().to_owned()),
        _ => None,
    }
}

/// The items' format (`ITEM_FORMAT`): when it differs from the one the app last saw, the app has the system read every
/// item again.
#[uniffi::export]
pub fn item_format() -> u32 {
    ITEM_FORMAT
}

/// Send this process's log lines to macOS's unified log, under the subsystem `dev.hviid.ImmichMount`. Elsewhere it
/// does nothing. Called once, as the app or the extension starts.
#[uniffi::export]
pub fn start_logging() {
    #[cfg(target_os = "macos")]
    {
        static STARTED: std::sync::Once = std::sync::Once::new();
        STARTED.call_once(|| {
            let _ = oslog::OsLogger::new("dev.hviid.ImmichMount").level_filter(log::LevelFilter::Info).init();
        });
    }
}

/// The bridge's tokio runtime, which reqwest needs; Swift's executor only polls the results.
fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .thread_name("immich-mount")
            .enable_all()
            .build()
            .expect("the bridge's tokio runtime starts")
    })
}

/// A way to stop a request the system no longer wants. uniffi's Swift does not pass a task's cancellation on to the
/// Rust future, so the extension hands a long request one of these, and cancels it from the request's progress. One
/// request may run several pieces of work under it (a batch of thumbnails): cancelling stops them all.
#[derive(Default, uniffi::Object)]
pub struct Cancel {
    state: Mutex<(bool, Vec<tokio::task::AbortHandle>)>,
}

#[uniffi::export]
impl Cancel {
    #[uniffi::constructor]
    pub fn new() -> Arc<Cancel> {
        Arc::new(Cancel::default())
    }

    /// Stop the request's work, now or as soon as it starts.
    pub fn cancel(&self) {
        let mut state = self.state.lock().expect("cancel lock");
        state.0 = true;
        for work in state.1.drain(..) {
            work.abort();
        }
    }
}

/// Run `work` on the bridge's runtime, from whatever executor polls the result.
async fn on_runtime<Output: Send + 'static>(
    work: impl Future<Output = Result<Output, BridgeError>> + Send + 'static,
) -> Result<Output, BridgeError> {
    cancellable(None, work).await
}

/// `on_runtime`, stopped by `cancel` or when the result is dropped.
async fn cancellable<Output: Send + 'static>(
    cancel: Option<Arc<Cancel>>,
    work: impl Future<Output = Result<Output, BridgeError>> + Send + 'static,
) -> Result<Output, BridgeError> {
    let task = runtime().spawn(work);
    if let Some(cancel) = &cancel {
        let mut state = cancel.state.lock().expect("cancel lock");
        if state.0 {
            task.abort();
        } else {
            state.1.retain(|work| !work.is_finished());
            state.1.push(task.abort_handle());
        }
    }
    match Cancelling(task).await {
        Ok(result) => result,
        Err(error) if error.is_cancelled() => Err(BridgeError::Cancelled),
        Err(error) => Err(failed(format!("internal: {error}"))),
    }
}

/// A spawned task that is aborted when its handle is dropped.
struct Cancelling<Output>(tokio::task::JoinHandle<Output>);

impl<Output> Future for Cancelling<Output> {
    type Output = Result<Output, tokio::task::JoinError>;
    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.0).poll(context)
    }
}

impl<Output> Drop for Cancelling<Output> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveDate, TimeZone, Utc};
    use immich_mount_core::uploads::Uploaded;
    use immich_mount_core::model::{Album, Asset, Person, YearMonth};
    use immich_mount_core::source::Watched;
    use immich_mount_core::tree::{InMemoryNames, SectionKey};
    use std::sync::atomic::{AtomicBool, Ordering};

    /// One photo in March 2024, in one album.
    #[derive(Default)]
    struct FakeLibrary {
        /// Listings take an hour, as on a server that drops requests.
        silent: AtomicBool,
        /// Downloads break off midway.
        downloads_fail: AtomicBool,
        /// A second photo arrived in March, and the next poll says so.
        second_photo: AtomicBool,
        /// Month listings fail, as on a server answering 503.
        months_fail: AtomicBool,
        poll_told: AtomicBool,
        thumbnails_asked: Mutex<Vec<ThumbnailKind>>,
        /// Albums made, by name, with their Assets.
        albums_made: Mutex<Vec<(String, Vec<String>)>>,
    }

    fn photo() -> Asset {
        let local = NaiveDate::from_ymd_opt(2024, 3, 15).unwrap().and_hms_opt(14, 32, 7).unwrap();
        Asset {
            id: "asset-1".into(),
            original_file_name: "IMG_1.JPG".into(),
            local_capture_time: local,
            captured_at: Utc.from_utc_datetime(&local),
            uploaded_at: Utc.from_utc_datetime(&local),
            size: Some(11),
            city: None,
            country: None,
        }
    }

    impl FakeLibrary {
        async fn answer(&self) {
            if self.silent.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_secs(3600)).await;
            }
        }
    }

    impl LibrarySource for FakeLibrary {
        async fn timeline_months(&self) -> Result<Vec<(YearMonth, u32)>, SourceError> {
            self.answer().await;
            Ok(vec![(YearMonth { year: 2024, month: 3 }, 1)])
        }
        async fn month_assets(&self, _month: YearMonth) -> Result<Vec<Asset>, SourceError> {
            self.answer().await;
            if self.months_fail.load(Ordering::SeqCst) {
                return Err(SourceError::Busy);
            }
            if !self.second_photo.load(Ordering::SeqCst) {
                return Ok(vec![photo()]);
            }
            let first = photo();
            let second = Asset {
                id: "asset-2".into(),
                original_file_name: "IMG_2.JPG".into(),
                local_capture_time: first.local_capture_time + chrono::Duration::seconds(1),
                ..photo()
            };
            Ok(vec![first, second])
        }
        async fn albums(&self) -> Result<Vec<Album>, SourceError> {
            Ok(vec![Album { id: "album-1".into(), name: "Trip".into(), year: Some(2024), created_at: Utc::now() }])
        }
        async fn album_assets(&self, _album_id: &str) -> Result<Vec<Asset>, SourceError> {
            Ok(vec![photo()])
        }
        async fn people(&self) -> Result<Vec<Person>, SourceError> {
            Ok(vec![])
        }
        async fn person_assets(&self, _person_id: &str) -> Result<Vec<Asset>, SourceError> {
            Ok(vec![])
        }
        async fn countries(&self) -> Result<Vec<String>, SourceError> {
            Ok(vec![])
        }
        async fn cities(&self, _country: &str) -> Result<Vec<String>, SourceError> {
            Ok(vec![])
        }
        async fn city_assets(&self, _country: &str, _city: &str) -> Result<Vec<Asset>, SourceError> {
            Ok(vec![])
        }
        async fn archive_assets(&self) -> Result<Vec<Asset>, SourceError> {
            Ok(vec![])
        }
        async fn favorite_assets(&self) -> Result<Vec<Asset>, SourceError> {
            Ok(vec![])
        }
        async fn changes(&self, _watched: Watched) -> Result<immich_mount_core::source::LibraryChanges, SourceError> {
            let arrived = self.second_photo.load(Ordering::SeqCst) && !self.poll_told.swap(true, Ordering::SeqCst);
            let months = if arrived { vec![YearMonth { year: 2024, month: 3 }] } else { vec![] };
            Ok(immich_mount_core::source::LibraryChanges { months, ..Default::default() })
        }
    }

    impl ThumbnailSource for FakeLibrary {
        async fn thumbnail(&self, _asset_id: &str, kind: ThumbnailKind) -> Result<Vec<u8>, SourceError> {
            self.thumbnails_asked.lock().unwrap().push(kind);
            Ok(b"RIFF".to_vec())
        }
    }

    impl UploadTarget for FakeLibrary {
        async fn media_types(&self) -> Result<immich_mount_core::uploads::MediaTypes, SourceError> {
            Ok(immich_mount_core::uploads::MediaTypes::default())
        }
        async fn upload(&self, _path: &Path, _file_name: &str, _modified: chrono::DateTime<Utc>) -> Result<Uploaded, UploadError> {
            Ok(Uploaded::Created("asset-uploaded".into()))
        }
    }

    impl AlbumTarget for FakeLibrary {
        async fn album_names(&self) -> Result<Vec<String>, SourceError> {
            let made = self.albums_made.lock().unwrap();
            Ok(std::iter::once("Trip".to_owned()).chain(made.iter().map(|(name, _)| name.clone())).collect())
        }
        async fn create_album(&self, name: &str, asset_ids: &[String]) -> Result<String, SourceError> {
            let mut made = self.albums_made.lock().unwrap();
            made.push((name.to_owned(), asset_ids.to_vec()));
            Ok(format!("made-{}", made.len()))
        }
        async fn add_to_album(&self, _album_id: &str, _asset_ids: &[String]) -> Result<(), SourceError> {
            Ok(())
        }
        async fn album_with(&self, _name: &str, _asset_id: &str) -> Result<Option<String>, SourceError> {
            Ok(None)
        }
    }

    impl MacSource for FakeLibrary {
        async fn poll_memory(&self) -> Option<String> {
            Some("seen".into())
        }
        async fn recall_poll(&self, _memory: &str) {}

        async fn download(&self, _asset_id: &str, destination: &Path) -> Result<u64, SourceError> {
            if self.downloads_fail.load(Ordering::SeqCst) {
                std::fs::write(destination, b"the ph").unwrap();
                return Err(SourceError::Offline("the connection dropped".into()));
            }
            std::fs::write(destination, b"the photo!!").unwrap();
            Ok(11)
        }
    }

    fn answers() -> Answers<FakeLibrary, InMemoryNames> {
        Answers::new(Arc::new(FakeLibrary::default()), InMemoryNames::default(), Config::from_toml("").unwrap())
    }

    fn march_photo() -> String {
        NodeKey::Asset { parent: Box::new(NodeKey::TimelineMonth(YearMonth { year: 2024, month: 3 })), asset_id: "asset-1".into() }.storage_key()
    }

    #[tokio::test]
    async fn the_root_lists_the_sections_as_folders() {
        let answers = answers();
        let root = answers.item("root").await.unwrap();
        assert!(root.is_folder && root.parent_identifier == "root");
        let sections = answers.list("root").await.unwrap();
        let names: Vec<&str> = sections.iter().map(|item| item.name.as_str()).collect();
        assert_eq!(names, ["Photos", "Albums", "People", "Places", "Upload"]);
        assert!(sections.iter().all(|item| item.is_folder && item.parent_identifier == "root"));
        assert!(sections.iter().all(|item| item.accepts_new_files == (item.name == "Upload")));
    }

    #[tokio::test]
    async fn folders_carry_the_time_the_location_was_set_up() {
        let set_up = SystemTime::UNIX_EPOCH + Duration::from_secs(1_762_560_000);
        let answers = answers().set_up_at(set_up);
        assert_eq!(answers.item("root").await.unwrap().taken, Some(set_up));
        let sections = answers.list("root").await.unwrap();
        assert!(sections.iter().all(|folder| folder.taken == Some(set_up)), "not 1 January 1970 in Finder");
        let photos = answers.list("section|Photos").await.unwrap();
        assert!(photos.iter().all(|year| year.taken == Some(set_up)));
    }

    #[tokio::test]
    async fn a_photo_is_the_same_item_whether_listed_or_asked_for() {
        let answers = answers();
        let years = answers.list(&NodeKey::Section(SectionKey::Photos).storage_key()).await.unwrap();
        let month = answers.list(&years[0].identifier).await.unwrap();
        let listed = answers.list(&month[0].identifier).await.unwrap();
        assert_eq!(listed.len(), 1);
        let photo = &listed[0];
        assert_eq!(photo.identifier, march_photo());
        assert_eq!((photo.is_folder, photo.size, photo.content_version.as_str()), (false, Some(11), "asset-1"));
        assert_eq!(answers.item(&photo.identifier).await.unwrap(), *photo);
    }

    #[tokio::test]
    async fn items_say_whether_they_have_a_web_link() {
        let answers = answers();
        assert!(!answers.item("root").await.unwrap().has_web_link, "Immich has no page for the root");
        let sections = answers.list("root").await.unwrap();
        let linked: Vec<(&str, bool)> = sections.iter().map(|item| (item.name.as_str(), item.has_web_link)).collect();
        assert_eq!(linked, [("Photos", true), ("Albums", true), ("People", true), ("Places", true), ("Upload", false)]);
        let years = answers.list(&NodeKey::Section(SectionKey::Photos).storage_key()).await.unwrap();
        assert!(!years[0].has_web_link, "a year folder");
        let months = answers.list(&years[0].identifier).await.unwrap();
        assert!(!months[0].has_web_link, "a month folder");
        let photos = answers.list(&months[0].identifier).await.unwrap();
        assert!(photos[0].has_web_link, "a photo in Photos");
    }

    #[test]
    fn a_web_link_is_the_items_address_on_the_signed_in_server() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().display().to_string();
        let library = Library::open("lab.example/".into(), Credential::ApiKey { key: "key".into() }, path, String::new()).unwrap();
        let album = NodeKey::Album("b1e2c3d4-0000-4000-8000-000000000001".into()).storage_key();
        assert_eq!(library.web_link(album).as_deref(), Some("https://lab.example/albums/b1e2c3d4-0000-4000-8000-000000000001"));
        assert_eq!(library.web_link(march_photo()).as_deref(), Some("https://lab.example/photos/asset-1"));
        assert_eq!(library.web_link("root".into()), None);
        assert_eq!(library.web_link("upload".into()), None);
        assert_eq!(library.web_link(NodeKey::TimelineMonth(YearMonth { year: 2024, month: 3 }).storage_key()), None);
        assert_eq!(library.web_link("not a key".into()), None);
    }

    #[tokio::test]
    async fn fetching_writes_the_original_and_a_failure_leaves_no_file() {
        let answers = answers();
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("original");
        let item = answers.fetch(&march_photo(), &destination).await.unwrap();
        assert_eq!(std::fs::read(&destination).unwrap(), b"the photo!!");
        assert_eq!(item.size, Some(11));
        assert_eq!(answers.fetch("root", &destination).await, Err(BridgeError::NoSuchItem), "a folder has no original");
        let gone = NodeKey::Asset { parent: Box::new(NodeKey::Album("album-1".into())), asset_id: "asset-2".into() };
        assert_eq!(answers.fetch(&gone.storage_key(), &destination).await, Err(BridgeError::NoSuchItem));
        answers.source.downloads_fail.store(true, Ordering::SeqCst);
        let broken = directory.path().join("broken");
        assert!(matches!(answers.fetch(&march_photo(), &broken).await, Err(BridgeError::ServerUnreachable { .. })));
        assert!(!broken.exists(), "half an original is never handed to the system");
    }

    #[tokio::test]
    async fn small_sizes_get_the_small_thumbnail_and_large_ones_the_preview() {
        let answers = answers();
        assert_eq!(answers.thumbnail(&march_photo(), 256).await.unwrap().as_deref(), Some(&b"RIFF"[..]));
        answers.thumbnail(&march_photo(), 1024).await.unwrap();
        assert_eq!(*answers.source.thumbnails_asked.lock().unwrap(), [ThumbnailKind::Small, ThumbnailKind::Preview]);
        assert_eq!(answers.thumbnail("root", 256).await, Ok(None), "a folder keeps the system's icon");
        assert_eq!(answers.thumbnail("month|2024-3", 256).await, Err(BridgeError::NoSuchItem));
    }

    #[tokio::test]
    async fn identifiers_no_key_writes_are_no_such_item() {
        let answers = answers();
        for identifier in ["", "NSFileProviderRootContainerItemIdentifier", "month|2024-3"] {
            assert_eq!(answers.item(identifier).await, Err(BridgeError::NoSuchItem), "{identifier}");
            assert_eq!(answers.list(identifier).await, Err(BridgeError::NoSuchItem), "{identifier}");
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_silent_server_answers_from_the_kept_listing_or_says_unreachable() {
        let directory = tempfile::tempdir().unwrap();
        let store = MetadataStore::open(&directory.path().join("metadata.redb")).unwrap();
        let answers = Answers::new(Arc::new(FakeLibrary::default()), store, Config::from_toml("").unwrap());
        let photos = NodeKey::Section(SectionKey::Photos);
        let years = answers.list(&photos.storage_key()).await.unwrap();
        answers.tree.invalidate(&photos);
        answers.source.silent.store(true, Ordering::SeqCst);
        // Forgotten as fresh, but kept: the folder still opens.
        assert_eq!(answers.list(&photos.storage_key()).await.unwrap(), years);
        let month = NodeKey::TimelineMonth(YearMonth { year: 2023, month: 1 });
        assert!(matches!(answers.list(&month.storage_key()).await, Err(BridgeError::ServerUnreachable { .. })));
    }

    fn journaled() -> (tempfile::TempDir, Answers<FakeLibrary, Arc<MetadataStore>>) {
        let directory = tempfile::tempdir().unwrap();
        let store = MetadataStore::open(&directory.path().join("metadata.redb")).unwrap().with_journal(JOURNAL_LIMIT).unwrap();
        (directory, Answers::new(Arc::new(FakeLibrary::default()), Arc::new(store), Config::from_toml("").unwrap()))
    }

    #[tokio::test]
    async fn a_photo_added_on_the_server_reaches_the_system_through_the_journal() {
        let (_directory, answers) = journaled();
        let month = NodeKey::TimelineMonth(YearMonth { year: 2024, month: 3 });
        for folder in ["root", "section|Photos", "year|2024", "month|2024-03"] {
            answers.list(folder).await.unwrap();
        }
        let shown = answers.current_anchor().unwrap();
        assert!(!answers.poll(None).await.unwrap(), "nothing changed yet");
        assert_eq!(answers.store().kept_state(POLL_MEMORY).as_deref(), Some("seen"), "the poll's memory is kept");

        answers.source.second_photo.store(true, Ordering::SeqCst);
        assert!(answers.poll(None).await.unwrap(), "the month changed, and the system holds it");
        let batch = answers.changes_since(&shown, 100).unwrap();
        assert_eq!(batch.deleted, Vec::<String>::new());
        assert_eq!(batch.updated.len(), 1);
        assert_eq!(batch.updated[0].parent_identifier, month.storage_key());
        assert_eq!(batch.updated[0].content_version, "asset-2");
        assert_eq!((batch.anchor.clone(), batch.more), (answers.current_anchor().unwrap(), false));
        assert_eq!(answers.changes_since(&batch.anchor, 100).unwrap().updated, vec![], "caught up");

        let held: Vec<String> = answers.working_set().into_iter().map(|item| item.identifier).collect();
        assert!(held.contains(&batch.updated[0].identifier) && held.contains(&"section|Photos".to_owned()));
        assert_eq!(answers.changes_since("not an anchor", 100), Err(BridgeError::AnchorExpired));
    }

    #[test]
    fn the_settings_panes_are_the_settings_that_mean_something_on_macos() {
        let panes = settings_panes("[sections]\npeople = false\n".into()).unwrap();
        let names: Vec<&str> = panes.sections.iter().map(|section| section.name.as_str()).collect();
        assert_eq!(names, ["Photos", "Albums", "People", "Places", "Favorites", "Archive", "Upload"]);
        let people = &panes.sections[2];
        assert_eq!((people.shown.key.as_str(), &people.shown.kind, people.shown.value.as_str()), ("sections.people", &PreferenceKind::Toggle, "false"));
        let people_dates = people.date_folders.as_ref().unwrap();
        assert_eq!((people_dates.key.as_str(), people_dates.value.as_str()), ("grouping.people", "atLeast500"));
        let upload = &panes.sections[6];
        assert_eq!((upload.shown.key.as_str(), upload.shown.value.as_str(), &upload.date_folders), ("uploads.enabled", "true", &None));
        assert_eq!(panes.sections[4].shown.value, "false", "Favorites is off by default");
        assert_eq!((panes.album_layout.key.as_str(), panes.album_layout.value.as_str()), ("albums.layout", "year-prefix"));
        assert!(matches!(&panes.album_layout.kind, PreferenceKind::Choice { options } if options[1].label == "Year Folders (2023/Trip)"));
        assert_eq!(panes.file_naming.key, "files.naming");
        assert_eq!((panes.crosslink.key.as_str(), &panes.crosslink.kind, panes.crosslink.value.as_str()), ("crosslink.enabled", &PreferenceKind::Toggle, "true"));
        let changed = set_preference("# mine\n".into(), "albums.layout".into(), "flat".into()).unwrap();
        assert!(changed.contains("# mine") && settings_panes(changed).unwrap().album_layout.value == "flat");
        assert!(set_preference(String::new(), "albums.layout".into(), "sideways".into()).is_err());
    }

    #[test]
    fn the_location_is_named_by_the_setting() {
        assert_eq!(drive_name(String::new()), "Immich Mount");
        assert_eq!(drive_name("[mount]\nname = \"immich\"\n".into()), "Immich");
        assert_eq!(drive_name("not toml [".into()), "Immich Mount");
    }

    #[test]
    fn new_settings_are_followed_without_opening_the_store_again() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().display().to_string();
        let library = Library::open("lab.example".into(), Credential::ApiKey { key: "key".into() }, path, String::new()).unwrap();
        assert!(!library.apply_config(String::new()).unwrap(), "the same settings");
        assert!(library.apply_config("[sections]\nplaces = false\n".into()).unwrap());
        assert!(!library.answers().tree.config().show_places);
        assert!(library.apply_config("[albums]\nlayout = \"sideways\"\n".into()).is_err(), "and the old ones stay");
        assert!(!library.answers().tree.config().show_places);
    }

    #[tokio::test]
    async fn a_photo_added_to_upload_is_uploaded_and_leaves_the_folder_later() {
        let (directory, answers) = journaled();
        let photo = directory.path().join("photo.png");
        std::fs::write(&photo, b"\x89PNG\r\n\x1a\n....\0\0\0\0IEND\xaeB`\x82").unwrap();
        let taken = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let item = answers.upload("upload", "IMG_1.png", &photo, taken).await.unwrap();
        assert_eq!((item.identifier.as_str(), item.parent_identifier.as_str()), ("asset|upload|asset-uploaded", "upload"));
        assert_eq!(item.taken, Some(taken));
        assert_eq!(answers.item(&item.identifier).await, Ok(item.clone()), "the system may ask about it while it waits");
        let fetched = directory.path().join("fetched");
        assert_eq!(answers.fetch(&item.identifier, &fetched).await.map(|item| item.size), Ok(Some(11)), "or fetch it");

        let before = answers.current_anchor().unwrap();
        answers.record_departures(seconds_since_epoch(SystemTime::now())).unwrap();
        assert_eq!(answers.current_anchor().unwrap(), before, "it stays a while, so the system takes it first");
        answers.record_departures(seconds_since_epoch(SystemTime::now() + UPLOAD_STAY)).unwrap();
        let batch = answers.changes_since(&before, 100).unwrap();
        assert_eq!(batch.deleted, ["asset|upload|asset-uploaded"]);
        assert!(leaving_upload(answers.store().kept_state(LEAVING_UPLOAD).as_deref()).is_empty(), "and leaves once");
        assert_eq!(answers.item(&item.identifier).await, Err(BridgeError::NoSuchItem), "and is gone after");

        let refused = |result: Result<Item, BridgeError>| matches!(result, Err(BridgeError::Failed { .. }));
        assert!(refused(answers.upload("section|Photos", "IMG_1.png", &photo, taken).await), "only Upload takes files");
        assert!(refused(answers.upload("upload", "._IMG_1.png", &photo, taken).await), "AppleDouble companions");
        assert!(refused(answers.upload("upload", "notes.txt", &photo, taken).await), "not a photo");
        let cut = directory.path().join("cut.png");
        std::fs::write(&cut, b"\x89PNG\r\n\x1a\n....").unwrap();
        assert!(refused(answers.upload("upload", "IMG_2.png", &cut, taken).await), "a copy cut short");
    }

    #[tokio::test]
    async fn a_folder_copied_into_upload_uploads_its_files_and_asks_about_albums() {
        use immich_mount_core::folder_uploads::FolderChoice;
        let (directory, answers) = journaled();
        let files = directory.path().join("library");
        std::fs::create_dir(&files).unwrap();
        let shared = files.display().to_string();
        let answers = answers.following(Arc::default(), Some(files));
        let trip = answers.create_folder("upload", "Trip").unwrap();
        assert_eq!((trip.identifier.as_str(), trip.parent_identifier.as_str()), ("upload-folder|Trip", "upload"));
        assert!(trip.is_folder && trip.accepts_new_files);
        let day = answers.create_folder(&trip.identifier, "Day 1").unwrap();
        assert_eq!(answers.create_folder(&trip.identifier, "Day 1").unwrap(), day, "made again, it is the one there");
        assert!(answers.create_folder("upload", ".thumbnails").is_err(), "hidden folders are not uploaded");
        assert!(answers.create_folder("section|Photos", "Trip").is_err(), "only Upload takes folders");
        assert_eq!(answers.item(&day.identifier).await, Ok(day.clone()), "the system may ask about it");
        assert_eq!(answers.list("upload").await.unwrap(), std::slice::from_ref(&trip), "and listed, the system keeps it");
        assert_eq!(answers.list(&trip.identifier).await.unwrap(), std::slice::from_ref(&day));
        assert!(answers.working_set().contains(&day));

        let photo = directory.path().join("photo.png");
        std::fs::write(&photo, b"\x89PNG\r\n\x1a\n....\0\0\0\0IEND\xaeB`\x82").unwrap();
        let taken = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let item = answers.upload(&day.identifier, "IMG_1.png", &photo, taken).await.unwrap();
        assert_eq!(item.parent_identifier, day.identifier);
        assert_eq!(answers.item(&item.identifier).await, Ok(item.clone()));
        assert!(answers.upload(&trip.identifier, "notes.txt", &photo, taken).await.is_err());

        // Settled, the copy is asked about, through a file the app reads.
        let later = seconds_since_epoch(SystemTime::now()) + 60;
        let events = answers.with_copies(|copies| copies.tick(later, FolderChoice::Ask, |_| false));
        assert!(matches!(events.as_slice(), [FolderEvent::Ask(_)]));
        answers.leave_questions();
        let question = FolderQuestion { copy: 1, folder: "Trip".into(), uploaded: 1, not_uploaded: 1, albums: vec!["Trip › Day 1".into()] };
        assert_eq!(folder_questions(shared.clone()), [question]);
        answer_folder_question(shared.clone(), 1, true);
        assert_eq!(answers.take_answers(), [(1, true)]);
        assert!(answers.take_answers().is_empty(), "an answer is taken once");
        assert!(answers.with_copies(|copies| copies.answer(1, true)));
        answers.make_albums(1).await;
        assert_eq!(*answers.source.albums_made.lock().unwrap(), [("Trip › Day 1".to_owned(), vec!["asset-uploaded".to_owned()])]);
        assert_eq!(take_folder_notices(shared.clone()), [FolderNotice::AlbumsMade { folder: "Trip".into(), albums: vec!["Trip › Day 1".into()] }]);
        assert!(take_folder_notices(shared).is_empty(), "told once");

        // Day 1 leaves once its photo left and it stayed untouched; Trip stays, holding the refused file.
        let before = answers.current_anchor().unwrap();
        answers.record_departures(seconds_since_epoch(SystemTime::now() + UPLOAD_STAY)).unwrap();
        let soon = seconds_since_epoch(SystemTime::now()) + FOLDER_STAY.as_secs();
        answers.record_folder_departures(soon, None).unwrap();
        assert_eq!(answers.changes_since(&before, 100).unwrap().deleted, std::slice::from_ref(&item.identifier), "unsure: the folder stays");
        let holding = |folders: &[String]| PendingFolders { folders: folders.to_vec(), refresh_seconds: 0 };
        answers.record_folder_departures(soon, Some(&holding(std::slice::from_ref(&day.identifier)))).unwrap();
        assert_eq!(answers.changes_since(&before, 100).unwrap().deleted.len(), 1, "macOS holds something in it: it stays");
        let slow = PendingFolders { folders: Vec::new(), refresh_seconds: 60 };
        answers.record_folder_departures(soon, Some(&slow)).unwrap();
        assert_eq!(answers.changes_since(&before, 100).unwrap().deleted.len(), 1, "a list a minute old: it waits that long");
        answers.record_folder_departures(soon, Some(&holding(&[]))).unwrap();
        let deleted = answers.changes_since(&before, 100).unwrap().deleted;
        assert_eq!(deleted, [item.identifier.clone(), day.identifier.clone()], "nothing pending: it goes soon");
        assert_eq!(answers.item(&trip.identifier).await, Ok(trip.clone()));
        // Deleted in Finder, with the file that did not upload, it is forgotten.
        answers.delete_folder(&trip.identifier).unwrap();
        assert_eq!(answers.item(&trip.identifier).await, Err(BridgeError::NoSuchItem));
        assert!(answers.delete_folder("section|Photos").is_err(), "the library's folders are views");
    }

    #[tokio::test]
    async fn a_change_whose_folder_cannot_be_listed_now_reaches_the_system_at_a_later_poll() {
        let (_directory, answers) = journaled();
        for folder in ["root", "section|Photos", "year|2024", "month|2024-03"] {
            answers.list(folder).await.unwrap();
        }
        answers.poll(None).await.unwrap();
        let shown = answers.current_anchor().unwrap();
        answers.source.second_photo.store(true, Ordering::SeqCst);
        answers.source.months_fail.store(true, Ordering::SeqCst);
        assert!(!answers.poll(None).await.unwrap(), "the month could not be listed again");
        assert!(answers.store().folders_to_relist().contains(&NodeKey::TimelineMonth(YearMonth { year: 2024, month: 3 })));
        answers.source.months_fail.store(false, Ordering::SeqCst);
        assert!(answers.poll(None).await.unwrap(), "the server tells of the change only once, and the next poll still lists it");
        assert_eq!(answers.changes_since(&shown, 100).unwrap().updated[0].content_version, "asset-2");
        assert!(answers.store().folders_to_relist().is_empty());
    }

    #[test]
    fn the_kinds_of_failure_map_onto_what_the_system_tells_apart() {
        assert_eq!(BridgeError::from(SourceError::SignedOut), BridgeError::NotAuthenticated);
        assert_eq!(BridgeError::from(SourceError::NotFound), BridgeError::NoSuchItem);
        assert!(matches!(BridgeError::from(SourceError::Busy), BridgeError::ServerUnreachable { .. }));
        assert!(matches!(BridgeError::from(SourceError::MissingPermission(None)), BridgeError::NotPermitted { .. }));
    }

    #[test]
    fn a_version_changes_with_what_is_shown() {
        let item = root_item(SystemTime::UNIX_EPOCH);
        let renamed = Item { name: "Other".into(), ..item.clone() }.versioned();
        assert_ne!(item.version, renamed.version);
        assert_eq!(item.version, root_item(SystemTime::UNIX_EPOCH).version, "and only then");
    }

    #[tokio::test]
    async fn dropping_a_request_stops_its_work() {
        let finished = Arc::new(AtomicBool::new(false));
        let flag = finished.clone();
        let request = on_runtime(async move {
            tokio::time::sleep(Duration::from_millis(200)).await;
            flag.store(true, Ordering::SeqCst);
            Ok(())
        });
        drop(tokio::time::timeout(Duration::from_millis(20), request).await);
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert!(!finished.load(Ordering::SeqCst), "the system cancelled it, so it never finished");
    }

    #[tokio::test]
    async fn a_cancelled_request_stops_its_work() {
        let finished = Arc::new(AtomicBool::new(false));
        let flag = finished.clone();
        let cancel = Cancel::new();
        let stopper = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            stopper.cancel();
        });
        let result = cancellable(Some(cancel.clone()), async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            flag.store(true, Ordering::SeqCst);
            Ok(())
        })
        .await;
        assert_eq!(result, Err(BridgeError::Cancelled));
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert!(!finished.load(Ordering::SeqCst), "the work stopped, though the caller never dropped it");
        assert_eq!(cancellable(Some(cancel), async { Ok(()) }).await, Err(BridgeError::Cancelled), "cancelled before it began");
    }

    #[tokio::test]
    async fn one_cancel_stops_every_piece_of_work_under_it() {
        let cancel = Cancel::new();
        let stopper = cancel.clone();
        let piece = || {
            cancellable(Some(cancel.clone()), async {
                tokio::time::sleep(Duration::from_millis(300)).await;
                Ok(())
            })
        };
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            stopper.cancel();
        });
        let (first, second) = tokio::join!(piece(), piece());
        assert_eq!((first, second), (Err(BridgeError::Cancelled), Err(BridgeError::Cancelled)), "a batch of thumbnails");
    }

    #[test]
    fn a_library_is_opened_once_per_directory_and_credential() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().display().to_string();
        let key = |key: &str| Credential::ApiKey { key: key.into() };
        let first = Library::open("lab.example".into(), key("key"), path.clone(), String::new()).unwrap();
        let again = Library::open("https://lab.example/".into(), key(" key "), path.clone(), String::new()).unwrap();
        assert!(Arc::ptr_eq(&first, &again), "the store is opened once");
        drop((first, again));
        assert!(Library::open("lab.example".into(), key("key"), path, "bad = [".into()).is_err(), "a config that does not parse");
    }

    // Received links: a library with one photo in March 2024, in the album Trip, in Aarhus, Denmark, and one person,
    // with the UUIDs links carry.
    const LINKED_SERVER: &str = "https://photos.example.com";
    const LINKED_PHOTO: &str = "4f3c2b1a-0000-4000-8000-000000000001";
    const LINKED_ALBUM: &str = "4f3c2b1a-0000-4000-8000-000000000002";
    const LINKED_PERSON: &str = "4f3c2b1a-0000-4000-8000-000000000003";
    const SIGNED_IN_USER: &str = "4f3c2b1a-0000-4000-8000-0000000000aa";

    #[derive(Default)]
    struct LinkedLibrary {
        /// Every request takes an hour, as on a server that drops requests.
        silent: bool,
    }

    fn linked_photo() -> Asset {
        Asset { id: LINKED_PHOTO.into(), city: Some("Aarhus".into()), country: Some("Denmark".into()), ..photo() }
    }

    impl LinkedLibrary {
        async fn answer<Answer>(&self, answer: Answer) -> Result<Answer, SourceError> {
            if self.silent {
                tokio::time::sleep(Duration::from_secs(3600)).await;
            }
            Ok(answer)
        }
    }

    impl LibrarySource for LinkedLibrary {
        async fn timeline_months(&self) -> Result<Vec<(YearMonth, u32)>, SourceError> {
            self.answer(vec![(YearMonth { year: 2024, month: 3 }, 1)]).await
        }
        async fn month_assets(&self, _month: YearMonth) -> Result<Vec<Asset>, SourceError> {
            self.answer(vec![linked_photo()]).await
        }
        async fn albums(&self) -> Result<Vec<Album>, SourceError> {
            self.answer(vec![Album { id: LINKED_ALBUM.into(), name: "Trip".into(), year: Some(2024), created_at: Utc::now() }]).await
        }
        async fn album_assets(&self, _album_id: &str) -> Result<Vec<Asset>, SourceError> {
            self.answer(vec![linked_photo()]).await
        }
        async fn people(&self) -> Result<Vec<Person>, SourceError> {
            self.answer(vec![Person { id: LINKED_PERSON.into(), name: "Ada".into(), is_hidden: false, created_at: Utc::now() }]).await
        }
        async fn person_assets(&self, _person_id: &str) -> Result<Vec<Asset>, SourceError> {
            self.answer(vec![linked_photo()]).await
        }
        async fn countries(&self) -> Result<Vec<String>, SourceError> {
            self.answer(vec!["Denmark".into()]).await
        }
        async fn cities(&self, _country: &str) -> Result<Vec<String>, SourceError> {
            self.answer(vec!["Aarhus".into()]).await
        }
        async fn city_assets(&self, _country: &str, _city: &str) -> Result<Vec<Asset>, SourceError> {
            self.answer(vec![linked_photo()]).await
        }
        async fn archive_assets(&self) -> Result<Vec<Asset>, SourceError> {
            self.answer(vec![]).await
        }
        async fn favorite_assets(&self) -> Result<Vec<Asset>, SourceError> {
            self.answer(vec![]).await
        }
        async fn asset(&self, asset_id: &str) -> Result<Asset, SourceError> {
            if asset_id == LINKED_PHOTO { self.answer(linked_photo()).await } else { Err(SourceError::NotFound) }
        }
    }

    fn linked_tree(config: &str) -> Tree<LinkedLibrary, InMemoryNames> {
        Tree::new(LinkedLibrary::default(), InMemoryNames::default(), Config::from_toml(config).unwrap())
    }

    /// `show_link`'s outcome on the linked library, for the signed-in user.
    async fn follow(tree: &Tree<LinkedLibrary, InMemoryNames>, link: &str) -> ShowLinkOutcome {
        follow_link(tree, link, LINKED_SERVER, true, async { Some(SIGNED_IN_USER.to_owned()) }).await
    }

    fn crosslink_to(node: &NodeKey) -> String {
        links::crosslink(LINKED_SERVER, Some(SIGNED_IN_USER), node).unwrap()
    }

    fn shows(node: NodeKey) -> ShowLinkOutcome {
        ShowLinkOutcome { result: "show".into(), identifier: Some(node.storage_key()), detail: String::new() }
    }

    #[tokio::test]
    async fn a_received_link_names_the_item_where_finder_shows_it() {
        let tree = linked_tree("");
        let album = NodeKey::Album(LINKED_ALBUM.into());
        let person = NodeKey::Person(LINKED_PERSON.into());
        let city = NodeKey::City { country: "Denmark".into(), city: "Aarhus".into() };
        let march = NodeKey::TimelineMonth(YearMonth { year: 2024, month: 3 });
        let in_folder = |folder: &NodeKey| NodeKey::Asset { parent: Box::new(folder.clone()), asset_id: LINKED_PHOTO.into() };
        for (node, said) in [
            (album.clone(), "an album"),
            (person.clone(), "a person"),
            (city.clone(), "a city"),
            (NodeKey::Section(SectionKey::Albums), "a section"),
            (in_folder(&march), "a photo in Photos"),
            (in_folder(&album), "a photo in an album"),
            (in_folder(&person), "a photo of a person"),
            (in_folder(&city), "a photo in a city"),
        ] {
            assert_eq!(follow(&tree, &crosslink_to(&node)).await, shows(node), "{said}");
        }
        let web = links::web_address(LINKED_SERVER, &album).unwrap();
        assert_eq!(follow(&tree, &web).await, shows(album), "a web address on the signed-in server");
    }

    #[tokio::test]
    async fn a_received_link_that_shows_nothing_says_why() {
        let tree = linked_tree("[sections]\npeople = false\n");
        let album = NodeKey::Album(LINKED_ALBUM.into());
        let elsewhere = links::crosslink("https://other.example:8443", None, &album).unwrap();
        assert_eq!(
            follow(&tree, &elsewhere).await,
            ShowLinkOutcome { detail: "other.example:8443".into(), ..ShowLinkOutcome::refused("other-server") }
        );
        let refused = |result: &str| ShowLinkOutcome::refused(result);
        assert_eq!(follow(&tree, "immich-crosslink://host/albums/x").await, refused("invalid"));
        assert_eq!(follow(&tree, "https://example.com/albums").await.result, "other-server", "a web address elsewhere");
        let search = format!("immich-crosslink:///search?query=%7B%22make%22%3A%22Canon%22%7D&server={}", links::encode_component(LINKED_SERVER));
        assert_eq!(follow(&tree, &search).await, refused("cannot-show"));
        let someone_else = links::crosslink(LINKED_SERVER, Some(LINKED_PERSON), &album).unwrap();
        assert_eq!(follow(&tree, &someone_else).await, refused("other-user"));
        let unknown_id = follow_link(&tree, &someone_else, LINKED_SERVER, true, async { None }).await;
        assert_eq!(unknown_id, shows(album), "the user is not checked when the account cannot read its id");
        let march = NodeKey::TimelineMonth(YearMonth { year: 2024, month: 3 });
        let gone = NodeKey::Asset { parent: Box::new(march), asset_id: LINKED_ALBUM.into() };
        assert_eq!(follow(&tree, &crosslink_to(&gone)).await, refused("not-found"), "a photo the account cannot see");
        let hidden = crosslink_to(&NodeKey::Person(LINKED_PERSON.into()));
        assert_eq!(follow(&tree, &hidden).await, refused("not-found"), "a section that is off");
    }

    #[tokio::test]
    async fn with_crosslink_off_a_link_is_not_read() {
        let tree = linked_tree("");
        let link = crosslink_to(&NodeKey::Album(LINKED_ALBUM.into()));
        let outcome = follow_link(&tree, &link, LINKED_SERVER, false, async { panic!("the user was asked") }).await;
        assert_eq!(outcome, ShowLinkOutcome::refused("off"));
        assert!(!parse_config("[crosslink]\nenabled = false\n").unwrap().crosslink_enabled);
    }

    #[tokio::test(start_paused = true)]
    async fn a_received_link_waits_a_limited_time() {
        let tree = Tree::new(LinkedLibrary { silent: true }, InMemoryNames::default(), Config::from_toml("").unwrap());
        let link = crosslink_to(&NodeKey::Album(LINKED_ALBUM.into()));
        assert_eq!(follow(&tree, &link).await, ShowLinkOutcome::refused("unreachable"));
    }

    fn found(bundle_id: &str, path: &str, takes_crosslinks: bool, version: Option<&str>, shows: &[&str]) -> FoundApp {
        FoundApp {
            bundle_id: bundle_id.into(),
            path: path.into(),
            takes_crosslinks,
            version: version.map(str::to_owned),
            shows: shows.iter().map(|kind| (*kind).to_owned()).collect(),
        }
    }

    const EVERY_KIND: &[&str] = &["asset", "album", "person", "place", "page"];

    #[test]
    fn finder_offers_the_apps_that_announce_the_draft() {
        let apps = vec![
            found("dev.hviid.ImmichMount", "/Applications/Immich Mount.app", true, Some("0.2"), EVERY_KIND),
            found("org.example.Albums", "/Users/me/build/Albums.app", true, Some("0.2"), &["album", "spaceship"]),
            found("org.example.Albums", "/Applications/Albums.app", true, Some("0.2"), &["album", "spaceship"]),
            found("org.example.Old", "/Applications/Old.app", true, Some("0.1"), EVERY_KIND),
            found("org.example.Quiet", "/Applications/Quiet.app", true, None, &[]),
        ];
        let targets = crosslink_targets(apps.clone(), String::new(), false);
        assert_eq!(targets.gallery, None);
        let albums = OfferedApp { bundle_id: "org.example.Albums".into(), path: "/Applications/Albums.app".into(), kinds: vec!["album".into()], own_scheme: None };
        assert_eq!(targets.others, vec![albums.clone()], "itself, another draft and no announcement are left out");
        assert_eq!(targets.listed, vec![ListedApp { bundle_id: albums.bundle_id.clone(), path: albums.path.clone(), shown: true }]);

        let hidden = set_app_shown(String::new(), "org.example.Albums".into(), false).unwrap();
        let targets = crosslink_targets(apps.clone(), hidden.clone(), false);
        assert_eq!((targets.others.len(), targets.listed[0].shown), (0, false), "switched off in Settings");
        assert!(!set_app_shown(hidden, "org.example.Albums".into(), true).unwrap().contains("hidden"), "back, the key goes");

        let off = crosslink_targets(apps, "[crosslink]\nenabled = false\n".into(), false);
        assert_eq!((off.gallery, off.others.len(), off.listed.len()), (None, 0, 1), "Immich crosslink off: listed, offered nowhere");
    }

    #[test]
    fn immich_gallery_gets_its_own_action() {
        let release = found("dev.igol.ImmichGallery", "/Applications/ImmichGallery.app", false, None, &[]);
        let development = found("dev.igol.ImmichGallery.Dev", "/Applications/Immich Gallery Dev.app", true, Some("0.2"), EVERY_KIND);
        let targets = crosslink_targets(vec![release.clone()], String::new(), false);
        let own = OfferedApp { bundle_id: release.bundle_id.clone(), path: release.path.clone(), kinds: vec!["asset".into()], own_scheme: Some("immichgallery".into()) };
        assert_eq!(targets.gallery, Some(own.clone()), "a release that does not announce: its own link, on photos");
        assert_eq!(targets.listed.len(), 1);

        let both = vec![release, development.clone()];
        let targets = crosslink_targets(both.clone(), String::new(), true);
        let gallery = targets.gallery.unwrap();
        assert_eq!((gallery.bundle_id.as_str(), gallery.kinds.len(), gallery.own_scheme), ("dev.igol.ImmichGallery.Dev", 5, None), "announced: crosslinks");
        assert!(targets.others.is_empty(), "never offered twice");
        let targets = crosslink_targets(both, String::new(), false);
        assert_eq!(targets.gallery, Some(own), "the release prefers the release");
        assert_eq!(targets.others.iter().map(|app| app.bundle_id.as_str()).collect::<Vec<_>>(), ["dev.igol.ImmichGallery.Dev"]);
    }

    #[test]
    fn an_item_tells_its_kind_and_a_photo_in_photos_its_asset() {
        let march = NodeKey::TimelineMonth(YearMonth { year: 2024, month: 3 });
        let in_photos = NodeKey::Asset { parent: Box::new(march.clone()), asset_id: LINKED_PHOTO.into() };
        let in_album = NodeKey::Asset { parent: Box::new(NodeKey::Album(LINKED_ALBUM.into())), asset_id: LINKED_PHOTO.into() };
        assert_eq!(item_kind(in_photos.storage_key()).as_deref(), Some("asset"));
        assert_eq!(item_kind(NodeKey::Album(LINKED_ALBUM.into()).storage_key()).as_deref(), Some("album"));
        assert_eq!(item_kind(march.storage_key()), None, "Immich has no page for a month");
        assert_eq!(photo_in_photos(in_photos.storage_key()).as_deref(), Some(LINKED_PHOTO));
        assert_eq!(photo_in_photos(in_album.storage_key()), None);
        assert_eq!(crosslink_kind(crosslink_to(&in_album)).as_deref(), Some("asset"));
        assert_eq!(crosslink_kind("immich-crosslink://host/albums".into()), None);
    }
}
