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
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::task::{Context, Poll};
use std::time::{Duration, Instant, SystemTime};

use immich_api::{ApiError, Client, Credential};
use immich_mount_core::immich::{source_error, ImmichSource};
use immich_mount_core::settings::Config;
use immich_mount_core::sign_in::normalised_address;
use immich_mount_core::source::{LibrarySource, SourceError, ThumbnailKind, ThumbnailSource};
use immich_mount_core::store::MetadataStore;
use immich_mount_core::tree::{Entry, EntryKind, NameMemory, NodeKey, Tree};
use sha2::{Digest, Sha256};

uniffi::setup_scaffolding!();

const USER_AGENT: &str = concat!("immich-mount/", env!("CARGO_PKG_VERSION"), " (macOS)");
/// How long a listing or an item may take before the system hears that the server cannot be reached. A listing kept
/// from before answers instead, when there is one.
const LISTING_DEADLINE: Duration = Duration::from_secs(20);
/// Immich's small thumbnail is 250 pixels on its short side. Up to this size it is upscaled a little; above it, the
/// preview (1440 pixels) is sent.
const SMALL_THUMBNAIL_UP_TO: u32 = 320;
/// How long the server check keeps trying a connection that fails: macOS may refuse sockets while its Local Network
/// prompt waits for an answer (Apple's TN3179).
const LOCAL_NETWORK_PATIENCE: Duration = Duration::from_secs(30);
/// How long one attempt of the server check may take; the client's own retries would wait minutes on a server that
/// drops requests.
const CHECK_ATTEMPT_LIMIT: Duration = Duration::from_secs(10);

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

/// Whole originals, which the system asks for where Linux reads ranges.
pub trait Originals: Send + Sync {
    /// Write an Asset's whole original to `destination`, and return the bytes written.
    fn download(&self, asset_id: &str, destination: &Path) -> impl Future<Output = Result<u64, SourceError>> + Send;
}

impl Originals for ImmichSource {
    async fn download(&self, asset_id: &str, destination: &Path) -> Result<u64, SourceError> {
        self.download_original(asset_id, destination).await
    }
}

/// The answers to the system's requests, over any source: Immich in the extension, a fake in tests.
struct Answers<Source, Names> {
    tree: Tree<Arc<Source>, Names>,
    source: Arc<Source>,
    deadline: Duration,
}

impl<Source, Names> Answers<Source, Names>
where
    Source: LibrarySource + ThumbnailSource + Originals + 'static,
    Names: NameMemory + 'static,
{
    fn new(source: Source, names: Names, config: Config) -> Self {
        let source = Arc::new(source);
        Answers { tree: Tree::new(source.clone(), names, config), source, deadline: LISTING_DEADLINE }
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

fn too_slow() -> BridgeError {
    BridgeError::ServerUnreachable { message: "the server did not answer in time".to_owned() }
}

/// The library behind the Finder location: one account on one server.
#[derive(uniffi::Object)]
pub struct Library {
    answers: Arc<Answers<ImmichSource, MetadataStore>>,
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
    pub fn open(server: String, api_key: String, directory: String, config: String) -> Result<Arc<Library>, BridgeError> {
        let (server, api_key, directory) = (normalised_address(&server), api_key.trim().to_owned(), PathBuf::from(directory));
        let mut open = open_libraries().lock().expect("open libraries lock");
        open.retain(|_, library| library.strong_count() > 0);
        let key = (directory.clone(), server.clone(), api_key.clone());
        if let Some(library) = open.get(&key).and_then(Weak::upgrade) {
            return Ok(library);
        }
        let mut config = Config::from_toml(&config).map_err(failed)?;
        // Not before the macOS edition uploads (ROADMAP, M7): the folder would refuse every file.
        config.uploads_enabled = false;
        let client = Client::new(&server, Some(Credential::ApiKey(api_key)), USER_AGENT).map_err(failed)?;
        std::fs::create_dir_all(&directory).map_err(|error| failed(format!("{}: {error}", directory.display())))?;
        let store = MetadataStore::open(&directory.join("metadata.redb")).map_err(failed)?;
        let library = Arc::new(Library { answers: Arc::new(Answers::new(ImmichSource::new(client), store, config)) });
        open.insert(key, Arc::downgrade(&library));
        Ok(library)
    }

    /// The item behind an identifier.
    pub async fn item(&self, identifier: String) -> Result<Item, BridgeError> {
        let answers = self.answers.clone();
        on_runtime(async move { answers.item(&identifier).await }).await
    }

    /// The items a folder lists.
    pub async fn list(&self, identifier: String) -> Result<Vec<Item>, BridgeError> {
        let answers = self.answers.clone();
        on_runtime(async move { answers.list(&identifier).await }).await
    }

    /// Download a file's original into `destination`, and return the item as it is now. On failure no file is left.
    pub async fn fetch(&self, identifier: String, destination: String) -> Result<Item, BridgeError> {
        let answers = self.answers.clone();
        on_runtime(async move { answers.fetch(&identifier, Path::new(&destination)).await }).await
    }

    /// A file's thumbnail for a size in pixels (its longer side), as image bytes: WebP or JPEG, as Immich made it.
    /// `None` for a folder, which keeps the system's own icon.
    pub async fn thumbnail(&self, identifier: String, pixels: u32) -> Result<Option<Vec<u8>>, BridgeError> {
        let answers = self.answers.clone();
        on_runtime(async move { answers.thumbnail(&identifier, pixels).await }).await
    }
}

/// What a server check found.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Server {
    /// The address as it is used: `https://` added when none was typed.
    pub address: String,
    /// Immich's version, as `v3.3.5`.
    pub version: String,
}

/// Check a server and an API key before they are kept: the server answers, runs a supported Immich, and accepts the
/// key. The app calls this before the extension ever runs, so macOS's Local Network prompt names the app
/// (ARCHITECTURE.md, "The macOS edition"); a connection that fails is tried again while that prompt may be open.
#[uniffi::export]
pub async fn check_server(address: String, api_key: String) -> Result<Server, BridgeError> {
    on_runtime(async move {
        let address = normalised_address(&address);
        let client = Client::new(&address, Some(Credential::ApiKey(api_key.trim().to_owned())), USER_AGENT).map_err(failed)?;
        let started = Instant::now();
        let version = loop {
            let attempt = tokio::time::timeout(CHECK_ATTEMPT_LIMIT, client.server_version()).await;
            match attempt {
                Ok(Ok(version)) => break version,
                Ok(Err(ApiError::Offline(_))) | Err(_) if started.elapsed() < LOCAL_NETWORK_PATIENCE => {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
                Ok(Err(error)) => return Err(BridgeError::from(source_error(error))),
                Err(_) => return Err(too_slow()),
            }
        };
        if !version.is_supported() {
            return Err(failed(format!(
                "This server runs Immich {version}. Immich Mount needs {} or later.",
                immich_api::version::MINIMUM_SUPPORTED
            )));
        }
        client.api_key_permissions().await.map_err(|error| BridgeError::from(source_error(error)))?;
        Ok(Server { address, version: version.to_string() })
    })
    .await
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
            Ok(vec![photo()])
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
    }

    impl ThumbnailSource for FakeLibrary {
        async fn thumbnail(&self, _asset_id: &str, kind: ThumbnailKind) -> Result<Vec<u8>, SourceError> {
            self.thumbnails_asked.lock().unwrap().push(kind);
            Ok(b"RIFF".to_vec())
        }
    }

    impl Originals for FakeLibrary {
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
        Answers::new(FakeLibrary::default(), InMemoryNames::default(), Config::from_toml("").unwrap())
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
        let answers = Answers::new(FakeLibrary::default(), store, Config::from_toml("").unwrap());
        let photos = NodeKey::Section(SectionKey::Photos);
        let years = answers.list(&photos.storage_key()).await.unwrap();
        answers.tree.invalidate(&photos);
        answers.source.silent.store(true, Ordering::SeqCst);
        // Forgotten as fresh, but kept: the folder still opens.
        assert_eq!(answers.list(&photos.storage_key()).await.unwrap(), years);
        let month = NodeKey::TimelineMonth(YearMonth { year: 2023, month: 1 });
        assert!(matches!(answers.list(&month.storage_key()).await, Err(BridgeError::ServerUnreachable { .. })));
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
        let first = Library::open("lab.example".into(), "key".into(), path.clone(), String::new()).unwrap();
        let again = Library::open("https://lab.example/".into(), " key ".into(), path.clone(), String::new()).unwrap();
        assert!(Arc::ptr_eq(&first, &again), "the store is opened once");
        drop((first, again));
        assert!(Library::open("lab.example".into(), "key".into(), path, "bad = [".into()).is_err(), "a config that does not parse");
    }
}
