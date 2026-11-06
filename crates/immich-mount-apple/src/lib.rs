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
use immich_mount_core::immich::ImmichSource;
use immich_mount_core::settings::{self, choice_label, Config, Kind, SETTINGS};
use immich_mount_core::sign_in::{self, normalised_address, BrowserSignIn};
use immich_mount_core::source::{LibrarySource, SourceError, ThumbnailKind, ThumbnailSource};
use immich_mount_core::store::{Anchor, Change, JournalRead, MetadataStore};
use immich_mount_core::tree::{is_touched, Entry, EntryKind, NameMemory, NodeKey, Tree};
use immich_mount_core::uploads::{completeness, Admission, Completeness, UploadError, UploadTarget, Uploaded};
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
    /// Whether new files may be added: the Upload folder's, once the macOS edition uploads (ROADMAP, M7).
    pub accepts_new_files: bool,
    /// The original's size in bytes, when Immich said.
    pub size: Option<u64>,
    /// When the photo or video was taken: the file's creation and modification time.
    pub taken: Option<SystemTime>,
    /// The content's version: an Asset's original never changes, so its id; `folder` for folders.
    pub content_version: String,
    /// Changes whenever anything shown about the item (its name, size or time) changes, so the system updates it.
    pub version: String,
}

impl Item {
    fn versioned(mut self) -> Item {
        let mut digest = Sha256::new();
        digest.update(self.name.as_bytes());
        digest.update([0]);
        digest.update(self.size.unwrap_or(u64::MAX).to_le_bytes());
        let taken = self.taken.and_then(|taken| taken.duration_since(SystemTime::UNIX_EPOCH).ok()).map_or(0, |since| since.as_secs());
        digest.update(taken.to_le_bytes());
        self.version = digest.finalize()[..8].iter().map(|byte| format!("{byte:02x}")).collect();
        self
    }
}

fn root_item() -> Item {
    let root = NodeKey::Root.storage_key();
    Item {
        identifier: root.clone(),
        parent_identifier: root,
        name: "Immich".to_owned(),
        is_folder: true,
        accepts_new_files: false,
        size: None,
        taken: None,
        content_version: "folder".to_owned(),
        version: String::new(),
    }
    .versioned()
}

fn item_for(entry: &Entry, folder: &NodeKey) -> Item {
    let (size, taken, content_version) = match &entry.kind {
        EntryKind::Folder => (None, None, "folder".to_owned()),
        EntryKind::File { asset_id, size, modified } => (*size, Some(SystemTime::from(*modified)), asset_id.clone()),
    };
    Item {
        identifier: entry.node.storage_key(),
        parent_identifier: folder.storage_key(),
        name: entry.name.clone(),
        is_folder: matches!(entry.kind, EntryKind::Folder),
        accepts_new_files: entry.node == NodeKey::Upload,
        size,
        taken,
        content_version,
        version: String::new(),
    }
    .versioned()
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
}

impl<Source, Names> Answers<Source, Names>
where
    Source: LibrarySource + ThumbnailSource + MacSource + 'static,
    Names: NameMemory + 'static,
{
    fn new(source: Arc<Source>, names: Names, config: Config) -> Self {
        Answers { tree: Tree::new(source.clone(), names, config), source, deadline: LISTING_DEADLINE, kept_poll: Mutex::new(None) }
    }

    async fn item(&self, identifier: &str) -> Result<Item, BridgeError> {
        let node = node_for(identifier)?;
        if node == NodeKey::Root {
            return Ok(root_item());
        }
        let found = tokio::time::timeout(self.deadline, self.tree.entry_of(&node)).await.map_err(|_| too_slow())??;
        let (folder, entry) = found.ok_or(BridgeError::NoSuchItem)?;
        Ok(item_for(&entry, &folder))
    }

    async fn list(&self, identifier: &str) -> Result<Vec<Item>, BridgeError> {
        let folder = node_for(identifier)?;
        let entries = match tokio::time::timeout(self.deadline, self.tree.list(&folder)).await {
            Ok(listed) => listed?,
            Err(_) => self.tree.stale_listing(&folder).ok_or_else(too_slow)?,
        };
        Ok(entries.iter().map(|entry| item_for(entry, &folder)).collect())
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
    async fn poll(&self) -> Result<bool, BridgeError> {
        let before = self.store().journal_anchor().map_err(failed)?;
        self.record_departures(seconds_since_epoch(SystemTime::now()))?;
        let kept = self.kept_poll.lock().expect("kept poll lock").take();
        if let Some(memory) = kept {
            self.source.recall_poll(&memory).await;
        }
        let changes = self.source.changes().await?;
        if let Some(memory) = self.source.poll_memory().await {
            self.store().keep_state(POLL_MEMORY, &memory);
        }
        if !changes.is_empty() {
            self.tree.apply_changes(&changes);
            for folder in self.store().kept_folders().into_iter().filter(|folder| is_touched(folder, &changes)) {
                // A folder that went away with its parent, listed earlier in this loop, is not asked for again.
                if self.store().kept_listing(&folder).is_none() {
                    continue;
                }
                match tokio::time::timeout(self.deadline, self.tree.list(&folder)).await {
                    Ok(Ok(_)) => {}
                    Ok(Err(error)) => log::info!("a changed folder could not be listed again ({error}); the next change or visit lists it"),
                    Err(_) => log::info!("a changed folder was not listed again in time; the next change or visit lists it"),
                }
            }
        }
        Ok(self.store().journal_anchor().map_err(failed)? != before)
    }

    /// Upload a finished file added to the Upload folder (ARCHITECTURE.md, "The macOS edition", "Uploads"), and
    /// return its item there. It leaves the folder at the first poll after `UPLOAD_STAY`, and shows in its month once
    /// the poll sees the server's change.
    async fn upload(&self, folder: &str, name: &str, path: &Path, modified: SystemTime) -> Result<Item, BridgeError>
    where
        Source: UploadTarget,
    {
        if node_for(folder)? != NodeKey::Upload || !self.tree.config().uploads_enabled {
            return Err(failed("Only the Upload folder takes new files; every other folder is a view of the library."));
        }
        let media = self.source.media_types().await.unwrap_or_default();
        if media.admission(name) != Admission::Upload {
            return Err(failed(format!("{name} is not a photo or video Immich takes.")));
        }
        let owned = path.to_owned();
        let judged = tokio::task::spawn_blocking(move || completeness(&owned)).await.map_err(failed)?;
        if matches!(judged.map_err(failed)?, Completeness::Truncated) {
            return Err(failed(format!("{name} is not whole: its copy may have been cut short.")));
        }
        let asset_id = match self.source.upload(path, name, modified.into()).await {
            Ok(Uploaded::Created(asset_id) | Uploaded::AlreadyInLibrary(asset_id)) => asset_id,
            Err(UploadError::Source(error)) => return Err(error.into()),
            Err(error) => return Err(failed(error)),
        };
        let node = NodeKey::Asset { parent: Box::new(NodeKey::Upload), asset_id: asset_id.clone() };
        let mut leaving = self.leaving_upload();
        leaving.push((node.storage_key(), seconds_since_epoch(SystemTime::now() + UPLOAD_STAY)));
        self.keep_leaving_upload(&leaving)?;
        self.tree.forget_timeline();
        let size = std::fs::metadata(path).ok().map(|metadata| metadata.len());
        Ok(Item {
            identifier: node.storage_key(),
            parent_identifier: NodeKey::Upload.storage_key(),
            name: name.to_owned(),
            is_folder: false,
            accepts_new_files: false,
            size,
            taken: Some(modified),
            content_version: asset_id,
            version: String::new(),
        }
        .versioned())
    }

    /// Uploaded files waiting to leave the Upload folder, with when they leave (seconds since the epoch).
    fn leaving_upload(&self) -> Vec<(String, u64)> {
        self.store().kept_state(LEAVING_UPLOAD).and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
    }

    fn keep_leaving_upload(&self, leaving: &[(String, u64)]) -> Result<(), BridgeError> {
        self.store().keep_state(LEAVING_UPLOAD, &serde_json::to_string(leaving).map_err(failed)?);
        Ok(())
    }

    /// Record in the journal the departures that are due at `now`.
    fn record_departures(&self, now: u64) -> Result<(), BridgeError> {
        let (due, waiting): (Vec<_>, Vec<_>) = self.leaving_upload().into_iter().partition(|(_, at)| *at <= now);
        if due.is_empty() {
            return Ok(());
        }
        let changes: Vec<Change> = due.iter().filter_map(|(key, _)| NodeKey::from_storage_key(key)).map(Change::Deleted).collect();
        self.store().record_changes(&changes).map_err(failed)?;
        self.keep_leaving_upload(&waiting)
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
                Change::Updated { folder, entry } => (entry.node.storage_key(), Some(item_for(&entry, &folder))),
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

    /// Every item the system holds: the entries of every kept listing.
    fn working_set(&self) -> Vec<Item> {
        let store = self.store();
        store
            .kept_folders()
            .into_iter()
            .flat_map(|folder| store.kept_listing(&folder).unwrap_or_default().iter().map(|entry| item_for(entry, &folder)).collect::<Vec<_>>())
            .collect()
    }
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
        let source = Arc::new(ImmichSource::new(client));
        let answers = Answers::new(source.clone(), store.clone(), parsed);
        *answers.kept_poll.lock().expect("kept poll lock") = store.kept_state(POLL_MEMORY);
        let library = Arc::new(Library { answers: RwLock::new(Arc::new(answers)), source, store, config: Mutex::new(config) });
        open.insert(key, Arc::downgrade(&library));
        Ok(library)
    }

    /// The item behind an identifier.
    pub async fn item(&self, identifier: String) -> Result<Item, BridgeError> {
        let answers = self.answers();
        on_runtime(async move { answers.item(&identifier).await }).await
    }

    /// The items a folder lists.
    pub async fn list(&self, identifier: String) -> Result<Vec<Item>, BridgeError> {
        let answers = self.answers();
        on_runtime(async move { answers.list(&identifier).await }).await
    }

    /// Download a file's original into `destination`, and return the item as it is now. On failure no file is left.
    pub async fn fetch(&self, identifier: String, destination: String) -> Result<Item, BridgeError> {
        let answers = self.answers();
        on_runtime(async move { answers.fetch(&identifier, Path::new(&destination)).await }).await
    }

    /// A file's thumbnail for a size in pixels (its longer side), as image bytes: WebP or JPEG, as Immich made it.
    /// `None` for a folder, which keeps the system's own icon.
    pub async fn thumbnail(&self, identifier: String, pixels: u32) -> Result<Option<Vec<u8>>, BridgeError> {
        let answers = self.answers();
        on_runtime(async move { answers.thumbnail(&identifier, pixels).await }).await
    }

    /// Ask the server what changed, and record in the journal what changed for the folders the system holds.
    /// Returns whether anything did.
    pub async fn poll(&self) -> Result<bool, BridgeError> {
        let answers = self.answers();
        on_runtime(async move { answers.poll().await }).await
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
    pub async fn upload(&self, folder: String, name: String, path: String, modified: SystemTime) -> Result<Item, BridgeError> {
        let answers = self.answers();
        on_runtime(async move { answers.upload(&folder, &name, Path::new(&path), modified).await }).await
    }

    /// Follow the settings in `config` (the text of `config.toml`) from now on, when they differ from those in use:
    /// folders are listed by the new rules on their next visit. Returns whether they differed.
    pub fn apply_config(&self, config: String) -> Result<bool, BridgeError> {
        let mut current = self.config.lock().expect("config lock");
        if *current == config {
            return Ok(false);
        }
        let answers = Answers::new(self.source.clone(), self.store.clone(), parse_config(&config)?);
        *self.answers.write().expect("answers lock") = Arc::new(answers);
        *current = config;
        Ok(true)
    }
}

/// The settings groups the macOS app shows: those that mean something there. The mount's path, the cache and
/// thumbnail seeding are Linux's.
const PREFERENCE_GROUPS: &[(&str, &str)] =
    &[("sections", "Sections"), ("grouping", "Date Folders"), ("albums", "Albums"), ("files", "Files"), ("uploads", "Uploads")];

/// A group of settings, as the app shows it.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PreferenceGroup {
    pub title: String,
    pub preferences: Vec<Preference>,
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

/// The preferences the app shows, with their values in `config`, the text of `config.toml`.
#[uniffi::export]
pub fn preferences(config: String) -> Vec<PreferenceGroup> {
    let stored = settings::stored_values(&config).unwrap_or_default();
    PREFERENCE_GROUPS
        .iter()
        .map(|(prefix, title)| PreferenceGroup {
            title: (*title).to_owned(),
            preferences: SETTINGS
                .iter()
                .filter(|setting| setting.key.split('.').next() == Some(*prefix))
                .filter_map(|setting| {
                    let kind = match setting.kind {
                        Kind::Bool => PreferenceKind::Toggle,
                        Kind::Choice(words) => PreferenceKind::Choice {
                            options: words
                                .iter()
                                .map(|word| PreferenceOption { value: (*word).to_owned(), label: choice_label(word).to_owned() })
                                .collect(),
                        },
                        Kind::Text | Kind::Path | Kind::Size => return None,
                    };
                    let value = stored.get(setting.key).cloned().unwrap_or_else(|| setting.default.to_owned());
                    Some(Preference { key: setting.key.to_owned(), title: setting.description.to_owned(), kind, value })
                })
                .collect(),
        })
        .collect()
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

/// Run `work` on the bridge's runtime, from whatever executor polls the result. When the result is dropped (the
/// system cancelled the request), the work stops too.
async fn on_runtime<Output: Send + 'static>(
    work: impl Future<Output = Result<Output, BridgeError>> + Send + 'static,
) -> Result<Output, BridgeError> {
    match Cancelling(runtime().spawn(work)).await {
        Ok(result) => result,
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
    use immich_mount_core::model::{Album, Asset, Person, YearMonth};
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
        poll_told: AtomicBool,
        thumbnails_asked: Mutex<Vec<ThumbnailKind>>,
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
        async fn changes(&self) -> Result<immich_mount_core::source::LibraryChanges, SourceError> {
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
        assert!(!answers.poll().await.unwrap(), "nothing changed yet");
        assert_eq!(answers.store().kept_state(POLL_MEMORY).as_deref(), Some("seen"), "the poll's memory is kept");

        answers.source.second_photo.store(true, Ordering::SeqCst);
        assert!(answers.poll().await.unwrap(), "the month changed, and the system holds it");
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
    fn the_preferences_are_the_settings_that_mean_something_on_macos() {
        let groups = preferences("[sections]\npeople = false\n".into());
        let titles: Vec<&str> = groups.iter().map(|group| group.title.as_str()).collect();
        assert_eq!(titles, ["Sections", "Date Folders", "Albums", "Files", "Uploads"]);
        let people = groups[0].preferences.iter().find(|preference| preference.key == "sections.people").unwrap();
        assert_eq!((&people.kind, people.value.as_str()), (&PreferenceKind::Toggle, "false"));
        let layout = &groups[2].preferences[0];
        assert_eq!((layout.key.as_str(), layout.value.as_str()), ("albums.layout", "year-prefix"), "the default");
        assert!(matches!(&layout.kind, PreferenceKind::Choice { options } if options[1].label == "Year Folders (2023/Trip)"));
        let changed = set_preference("# mine\n".into(), "albums.layout".into(), "flat".into()).unwrap();
        assert!(changed.contains("# mine") && preferences(changed)[2].preferences[0].value == "flat");
        assert!(set_preference(String::new(), "albums.layout".into(), "sideways".into()).is_err());
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

        let before = answers.current_anchor().unwrap();
        answers.record_departures(seconds_since_epoch(SystemTime::now())).unwrap();
        assert_eq!(answers.current_anchor().unwrap(), before, "it stays a while, so the system takes it first");
        answers.record_departures(seconds_since_epoch(SystemTime::now() + UPLOAD_STAY)).unwrap();
        let batch = answers.changes_since(&before, 100).unwrap();
        assert_eq!(batch.deleted, ["asset|upload|asset-uploaded"]);
        assert!(answers.leaving_upload().is_empty(), "and leaves once");

        let refused = |result: Result<Item, BridgeError>| matches!(result, Err(BridgeError::Failed { .. }));
        assert!(refused(answers.upload("section|Photos", "IMG_1.png", &photo, taken).await), "only Upload takes files");
        assert!(refused(answers.upload("upload", "._IMG_1.png", &photo, taken).await), "AppleDouble companions");
        assert!(refused(answers.upload("upload", "notes.txt", &photo, taken).await), "not a photo");
        let cut = directory.path().join("cut.png");
        std::fs::write(&cut, b"\x89PNG\r\n\x1a\n....").unwrap();
        assert!(refused(answers.upload("upload", "IMG_2.png", &cut, taken).await), "a copy cut short");
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
        let item = root_item();
        let renamed = Item { name: "Other".into(), ..item.clone() }.versioned();
        assert_ne!(item.version, renamed.version);
        assert_eq!(item.version, root_item().version, "and only then");
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
}
