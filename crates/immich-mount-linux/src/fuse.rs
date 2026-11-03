//! The FUSE adapter: presents the core's tree as a filesystem, read-only except for new files in Photos, which are
//! staged and uploaded (ARCHITECTURE.md, "The tree", "Content", "Uploads").
//!
//! The kernel speaks inode numbers; the core speaks `NodeKey`s. The inode table translates, handing out numbers in
//! sequence for the life of the mount. Every request that needs the network runs on the tokio runtime with a
//! deadline, so a slow or unreachable server never hangs a program that touches the mount (principle 1).

use std::collections::HashMap;
use std::ffi::OsStr;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use fuser::{
    BackgroundSession, BsdFileFlags, Config, Errno, FileAttr, FileHandle, FileType, Filesystem, FopenFlags, Generation,
    INodeNo, LockOwner, MountOption, OpenFlags, RenameFlags, ReplyAttr, ReplyCreate, ReplyData, ReplyDirectory, ReplyEmpty,
    ReplyEntry, ReplyOpen, ReplyStatfs, ReplyWrite, ReplyXattr, Request, TimeOrNow, WriteFlags,
};
use immich_mount_core::content::{BlockCache, ContentSource};
use immich_mount_core::source::{LibrarySource, SourceError, ThumbnailSource};
use immich_mount_core::tree::{Entry, EntryKind, NameMemory, NodeKey, SectionKey, Tree};
use immich_mount_core::uploads::{completeness, Admission, Completeness, MediaTypes, UploadError, UploadTarget, Uploaded};

use crate::caller;
use crate::seeding::{SeedFile, Seeder};
use crate::staging::{wait_for, CloseAction, PendingUpload, StagedFile, Staging, UploadResult};

/// How long a listing may take before the caller gets `EIO`.
const LISTING_DEADLINE: Duration = Duration::from_secs(10);
/// How long a listing waits for its thumbnails to be seeded before it answers anyway (SPEC.md, "Thumbnails").
const SEEDING_BUDGET: Duration = Duration::from_secs(10);
/// How long an upload may take before `close()` fails (ARCHITECTURE.md, "Uploads").
const UPLOAD_LIMIT: Duration = Duration::from_secs(30 * 60);
/// After this long, an upload is announced on the desktop.
const LONG_UPLOAD: Duration = Duration::from_secs(10);
/// How long a read may wait for its bytes before the caller gets `EIO`.
const READ_DEADLINE: Duration = Duration::from_secs(30);
/// The kernel's limit on one FUSE request, read when the mount is made (`kernel_request_limit`).
static REQUEST_LIMIT: std::sync::OnceLock<Option<Duration>> = std::sync::OnceLock::new();

/// The kernel's limit on one FUSE request on this system, if it sets one (Linux 6.14 and later):
/// `fs.fuse.default_request_timeout`, capped by `fs.fuse.max_request_timeout`. fuser asks for no limit of its own,
/// so these apply, and past the limit the kernel aborts the whole connection: the mount dies (seen
/// in a GNOME VM on Linux 7.2).
pub fn kernel_request_limit() -> Option<Duration> {
    let read = |name: &str| {
        std::fs::read_to_string(format!("/proc/sys/fs/fuse/{name}")).ok()?.trim().parse::<u64>().ok().filter(|seconds| *seconds > 0)
    };
    let default = read("default_request_timeout")?;
    Some(Duration::from_secs(read("max_request_timeout").map_or(default, |max| default.min(max))))
}

/// `wanted`, or a `share` of the kernel's request limit less a second, whichever is shorter: a request that waits
/// in more than one place gives each its share, so no wait of ours outlasts the kernel's patience.
fn within(wanted: Duration, share: u32) -> Duration {
    match REQUEST_LIMIT.get().copied().flatten() {
        Some(limit) => wanted.min((limit.saturating_sub(Duration::from_secs(1)) / share).max(Duration::from_millis(500))),
        None => wanted,
    }
}

/// How long the kernel may cache entries and attributes.
/// Files get the same short timeout as folders: a name freed by a deletion can be given to another Asset, and the
/// kernel must not keep sending the old one's path to the old Asset for long (principle 8).
const FOLDER_TTL: Duration = Duration::from_secs(60);
const FILE_TTL: Duration = Duration::from_secs(60);
/// How long the kernel may remember that a probed name is absent (`is_probe_name`).
const NEGATIVE_TTL: Duration = Duration::from_secs(60);

/// Names tools probe for in every folder: Dolphin's `.directory`, GIO's `.hidden`, the trash, indexers' opt-outs,
/// and Windows' and macOS' leftovers. Their absence is cached by the kernel as a negative entry, so the next probe
/// never reaches the daemon. Only these: an album, a person or an original's file name may start with a dot too,
/// and must show as soon as Immich has it.
fn is_probe_name(name: &str) -> bool {
    const PROBED: &[&str] =
        &[".directory", ".hidden", ".Trash", ".DS_Store", ".nomedia", ".trackerignore", ".metadata_never_index", "autorun.inf", "desktop.ini", "Thumbs.db"];
    PROBED.contains(&name) || name.starts_with(".Trash-") || name.starts_with("._")
}

/// Files the mount answers itself, at its root.
const VIRTUAL_FILES: &[(&str, &str)] = &[
    (".xdg-volume-info", "[Volume Info]\nName=Immich\n"),
    (".nomedia", ""),
    (".trackerignore", ""),
];

/// What an inode stands for.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum MountNode {
    Tree(NodeKey),
    Virtual(&'static str),
    /// A file being written into Photos, by its staging id.
    Staged(u64),
}

#[derive(Debug, Clone)]
struct InodeRecord {
    node: MountNode,
    kind: EntryKind,
    /// Where it appears, relative to the mount point.
    path: PathBuf,
    lookups: u64,
}

/// The two-way table between inode numbers and nodes.
struct InodeTable {
    by_node: HashMap<MountNode, u64>,
    by_inode: HashMap<u64, InodeRecord>,
    next_inode: u64,
}

impl InodeTable {
    fn new() -> Self {
        let root = MountNode::Tree(NodeKey::Root);
        let mut table = InodeTable { by_node: HashMap::new(), by_inode: HashMap::new(), next_inode: 2 };
        table.by_node.insert(root.clone(), INodeNo::ROOT.0);
        table.by_inode.insert(INodeNo::ROOT.0, InodeRecord { node: root, kind: EntryKind::Folder, path: PathBuf::new(), lookups: 1 });
        table
    }

    /// The inode for a node, creating it if needed, and refreshing its attributes and path.
    fn assign(&mut self, node: MountNode, kind: EntryKind, path: PathBuf, add_lookup: bool) -> u64 {
        let inode = match self.by_node.get(&node) {
            Some(inode) => *inode,
            None => {
                let inode = self.next_inode;
                self.next_inode += 1;
                self.by_node.insert(node.clone(), inode);
                self.by_inode.insert(inode, InodeRecord { node, kind: kind.clone(), path: path.clone(), lookups: 0 });
                inode
            }
        };
        if let Some(record) = self.by_inode.get_mut(&inode) {
            record.kind = kind;
            record.path = path;
            if add_lookup {
                record.lookups += 1;
            }
        }
        inode
    }

    fn record(&self, inode: u64) -> Option<InodeRecord> {
        self.by_inode.get(&inode).cloned()
    }

    fn forget(&mut self, inode: u64, count: u64) {
        if inode == INodeNo::ROOT.0 {
            return;
        }
        let drop_it = match self.by_inode.get_mut(&inode) {
            Some(record) => {
                record.lookups = record.lookups.saturating_sub(count);
                record.lookups == 0
            }
            None => false,
        };
        if drop_it {
            if let Some(record) = self.by_inode.remove(&inode) {
                self.by_node.remove(&record.node);
            }
        }
    }
}

/// List a folder in a task of its own, waiting at most `LISTING_DEADLINE`. A listing that outlives the deadline
/// still finishes and fills the tree's cache, so a slow, large folder answers on a later visit instead of starting
/// over every time. A listing past its freshness window is answered at once and refreshed in the background
/// (ARCHITECTURE.md, "Freshness"): the next visit shows the fresh one.
async fn list_detached<Library, Names>(
    tree: Arc<Tree<Library, Names>>,
    key: NodeKey,
    deadline: Duration,
) -> Result<Result<Vec<Entry>, SourceError>, tokio::time::error::Elapsed>
where
    Library: LibrarySource + 'static,
    Names: NameMemory + 'static,
{
    let (fetching_tree, fetching_key) = (tree.clone(), key.clone());
    if let Some(entries) = tree.expired_listing(&key) {
        tokio::spawn(async move {
            if let Err(error) = fetching_tree.list(&fetching_key).await {
                log::debug!("refreshing a folder in the background failed: {error}");
            }
        });
        return Ok(Ok(entries));
    }
    let listing = tokio::spawn(async move { fetching_tree.list(&fetching_key).await });
    match tokio::time::timeout(deadline, listing).await {
        Ok(Ok(result)) => Ok(result),
        Ok(Err(failure)) => Ok(Err(SourceError::Protocol(format!("the listing task failed: {failure}")))),
        // Past the deadline an older listing, if there is one, beats an error (a server that drops packets).
        Err(elapsed) => tree.stale_listing(&key).map(Ok).ok_or(elapsed),
    }
}

/// The error code a program sees for each kind of source failure (SPEC.md, "Errors").
pub fn errno_for(error: &SourceError) -> Errno {
    match error {
        SourceError::NotFound => Errno::ENOENT,
        SourceError::SignedOut | SourceError::MissingPermission(_) => Errno::EACCES,
        SourceError::Offline(_) | SourceError::Busy | SourceError::Protocol(_) | SourceError::Changed => Errno::EIO,
    }
}

/// The filesystem served to the kernel.
pub struct ImmichFilesystem<Library, Names, Content> {
    tree: Arc<Tree<Library, Names>>,
    content: Arc<BlockCache<Content>>,
    inodes: Arc<Mutex<InodeTable>>,
    runtime: tokio::runtime::Handle,
    owner_uid: u32,
    owner_gid: u32,
    mounted_at: SystemTime,
    generation: Generation,
    thumbnails: Option<ThumbnailSetup<Content>>,
    uploads: Option<UploadSetup<Content>>,
    /// The listing each open folder reads its pages from, taken with its first page, so a change between pages never
    /// skips or repeats an entry (a `cp -r` of a large folder while Immich changes it).
    listings: Arc<Mutex<HashMap<u64, Arc<Listing>>>>,
    next_folder_handle: AtomicU64,
    /// How many library files are open, which the daemon waits on before it restarts into an upgrade.
    open_files: Arc<AtomicUsize>,
}

/// The handle of an open library file: one value for all of them, since reads go by inode, kept apart from the
/// staged files' handles so `release` can count it.
const LIBRARY_FILE_HANDLE: u64 = u64::MAX;

/// One folder's entries as `readdir` hands them out: inode, type and name, `.` and `..` first.
type Listing = Vec<(u64, FileType, String)>;

/// Uploads into Photos (SPEC.md, "Uploads").
struct UploadSetup<Content> {
    staging: Arc<Staging>,
    media: Arc<MediaTypes>,
    target: Arc<Content>,
}

/// Seeding and the safety net (SPEC.md, "Thumbnails").
struct ThumbnailSetup<Content> {
    seeder: Arc<Seeder<Content>>,
    mount_point: PathBuf,
    safety_net: bool,
}

impl<Library, Names, Content> ImmichFilesystem<Library, Names, Content>
where
    Library: LibrarySource + 'static,
    Names: NameMemory + 'static,
    Content: ContentSource + 'static,
{
    pub fn new(tree: Arc<Tree<Library, Names>>, content: Arc<BlockCache<Content>>, runtime: tokio::runtime::Handle) -> Self {
        let mounted_at = SystemTime::now();
        let generation = mounted_at.duration_since(SystemTime::UNIX_EPOCH).map(|elapsed| elapsed.as_secs()).unwrap_or(1).max(1);
        ImmichFilesystem {
            tree,
            content,
            inodes: Arc::new(Mutex::new(InodeTable::new())),
            runtime,
            owner_uid: unsafe { libc::getuid() },
            owner_gid: unsafe { libc::getgid() },
            mounted_at,
            generation: Generation(generation),
            thumbnails: None,
            uploads: None,
            listings: Arc::new(Mutex::new(HashMap::new())),
            next_folder_handle: AtomicU64::new(1),
            open_files: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// The count of open library files, for the daemon to read once the filesystem is mounted.
    pub fn open_files(&self) -> Arc<AtomicUsize> {
        self.open_files.clone()
    }

    /// Accept new files in Photos, staged in `staging` and uploaded to `target` on close.
    pub fn with_uploads(mut self, staging: Arc<Staging>, media: MediaTypes, target: Arc<Content>) -> Self {
        self.uploads = Some(UploadSetup { staging, media: Arc::new(media), target });
        self
    }

    /// Seed thumbnails for the folders listed, and keep thumbnailers away from originals when `safety_net` is on.
    pub fn with_thumbnails(mut self, seeder: Arc<Seeder<Content>>, mount_point: &Path, safety_net: bool) -> Self {
        self.thumbnails = Some(ThumbnailSetup { seeder, mount_point: mount_point.to_path_buf(), safety_net });
        self
    }

    fn attributes(&self, inode: u64, node: &MountNode, kind: &EntryKind) -> FileAttr {
        self.snapshot().attributes(inode, node, kind)
    }

    fn staged_attributes(&self, inode: u64, id: u64, file: &StagedFile) -> FileAttr {
        self.attributes(inode, &MountNode::Staged(id), &staged_kind(file))
    }

    /// The error for changing something outside the staged files: existing Assets cannot be changed (principle 4),
    /// and outside Photos the mount is read-only.
    fn refusal(&self, folder: &NodeKey) -> Errno {
        if self.uploads.is_some() && is_photos_folder(folder) { Errno::EPERM } else { Errno::EROFS }
    }

    fn parent_folder(&self, parent: INodeNo) -> Result<(NodeKey, PathBuf), Errno> {
        match self.inodes.lock().expect("inode table lock").record(parent.0) {
            Some(InodeRecord { node: MountNode::Tree(key), kind: EntryKind::Folder, path, .. }) => Ok((key, path)),
            Some(_) => Err(Errno::ENOTDIR),
            None => Err(Errno::ENOENT),
        }
    }

    fn spawn<Work>(&self, work: Work)
    where
        Work: Future<Output = ()> + Send + 'static,
    {
        self.runtime.spawn(work);
    }
}

fn virtual_content(name: &str) -> &'static str {
    VIRTUAL_FILES.iter().find(|(file_name, _)| *file_name == name).map(|(_, content)| *content).unwrap_or("")
}

/// Whether a folder belongs to Photos, the only place new files are accepted.
fn is_photos_folder(node: &NodeKey) -> bool {
    matches!(node, NodeKey::Section(SectionKey::Photos) | NodeKey::TimelineYear(_) | NodeKey::TimelineMonth(_) | NodeKey::TimelineDay(..))
}

fn staged_kind(file: &StagedFile) -> EntryKind {
    EntryKind::File { asset_id: String::new(), size: Some(file.size), modified: chrono::DateTime::<chrono::Utc>::from(file.modified) }
}

/// The error a program sees on `close()` for a failed upload.
fn errno_for_upload(error: &UploadError) -> Errno {
    match error {
        UploadError::Source(source) => errno_for(source),
        UploadError::TrashedDuplicate => Errno::EEXIST,
        UploadError::Rejected(_) => Errno::ENOTSUP,
        UploadError::LocalFile(_) => Errno::EIO,
    }
}

/// Upload a staged file if it has something to upload, within `UPLOAD_LIMIT`. On success the timeline is read again,
/// so the new Asset shows in its date folder.
/// Upload a staged file within `UPLOAD_LIMIT` and hand the result to every waiting close. A file whose own structure
/// says it was cut short (a cancelled copy) is refused before a byte is sent. On success the timeline is read again,
/// so the new Asset shows in its date folder; a long upload or a failure is told on the desktop.
async fn run_upload<Content, Library, Names>(
    target: Arc<Content>,
    staging: Arc<Staging>,
    tree: Arc<Tree<Library, Names>>,
    pending: PendingUpload,
) -> UploadResult
where
    Content: UploadTarget + 'static,
    Library: LibrarySource + 'static,
    Names: NameMemory + 'static,
{
    let name = pending.name.clone();
    let path = pending.path.clone();
    let judged = tokio::task::spawn_blocking(move || completeness(&path)).await;
    if matches!(judged, Ok(Ok(Completeness::Truncated))) {
        log::warn!("not uploading {name}: the file ends before its own structure does (a cancelled or broken copy)");
        crate::notify::notify_in_background(format!("Not uploaded: {name}"), "The file is incomplete, as if its copy was cancelled.".to_owned());
        staging.finish_cut_short(pending);
        return Err(libc::EIO);
    }
    let modified = chrono::DateTime::<chrono::Utc>::from(pending.modified);
    // The close waits for the upload, so the kernel's request limit, where set, bounds it too.
    let limit = within(UPLOAD_LIMIT, 1);
    let outcome = {
        let upload = target.upload(&pending.path, &pending.name, modified);
        tokio::pin!(upload);
        // Files waits with its progress bar at 100% while the upload runs; say why when it takes a while.
        tokio::select! {
            outcome = tokio::time::timeout(limit, &mut upload) => outcome,
            () = tokio::time::sleep(LONG_UPLOAD), if limit > LONG_UPLOAD => {
                crate::notify::notify_in_background(format!("Uploading {name} to Immich"), "The copy finishes once Immich has the file.".to_owned());
                tokio::time::timeout(limit - LONG_UPLOAD, &mut upload).await
            }
        }
    };
    let result = match outcome {
        Ok(Ok(uploaded)) => {
            match uploaded {
                Uploaded::Created(_) => log::info!("uploaded {name}"),
                Uploaded::AlreadyInLibrary(_) => log::info!("{name} is already in the library"),
            }
            tree.forget_timeline();
            Ok(())
        }
        Ok(Err(error)) => {
            log::warn!("upload of {name} failed: {error}");
            let body = format!("{error}. The file is kept in {}.", staging.kept_directory().display());
            crate::notify::notify_once_in_a_while(&format!("failed:{error}"), &format!("Not uploaded: {name}"), &body).await;
            Err(errno_for_upload(&error).code())
        }
        Err(_) if limit < UPLOAD_LIMIT => {
            log::warn!("upload of {name} stopped after {} s, the kernel's limit on one FUSE request", limit.as_secs());
            crate::notify::notify_in_background(
                format!("Not uploaded: {name}"),
                format!(
                    "This computer lets a file operation take at most {} seconds (fs.fuse.default_request_timeout). Set it to 0 for longer uploads. The file is kept in {}.",
                    REQUEST_LIMIT.get().copied().flatten().map_or(0, |limit| limit.as_secs()),
                    staging.kept_directory().display()
                ),
            );
            Err(libc::EIO)
        }
        Err(_) => {
            log::warn!("upload of {name} took longer than {} minutes", UPLOAD_LIMIT.as_secs() / 60);
            crate::notify::notify_in_background(
                format!("Not uploaded: {name}"),
                format!("The upload took too long. The file is kept in {}.", staging.kept_directory().display()),
            );
            Err(libc::EIO)
        }
    };
    staging.finish_upload(pending, result);
    result
}

fn ttl_for(kind: &EntryKind) -> Duration {
    match kind {
        EntryKind::Folder => FOLDER_TTL,
        EntryKind::File { .. } => FILE_TTL,
    }
}

impl<Library, Names, Content> Filesystem for ImmichFilesystem<Library, Names, Content>
where
    Library: LibrarySource + 'static,
    Names: NameMemory + 'static,
    Content: ContentSource + ThumbnailSource + UploadTarget + 'static,
{
    fn lookup(&self, _request: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEntry) {
        let Some(parent_record) = self.inodes.lock().expect("inode table lock").record(parent.0) else {
            return reply.error(Errno::ENOENT);
        };
        let MountNode::Tree(parent_key) = parent_record.node else {
            return reply.error(Errno::ENOTDIR);
        };
        let Some(name) = name.to_str().map(str::to_string) else {
            return reply.error(Errno::ENOENT);
        };
        if let Some((id, file)) = self.uploads.as_ref().and_then(|uploads| uploads.staging.find(&parent_key, &name)) {
            let node = MountNode::Staged(id);
            let inode = self.inodes.lock().expect("inode table lock").assign(node, staged_kind(&file), parent_record.path.join(&name), true);
            return reply.entry(&Duration::ZERO, &self.staged_attributes(inode, id, &file), self.generation);
        }
        if parent_key == NodeKey::Root {
            if let Some((virtual_name, _)) = VIRTUAL_FILES.iter().find(|(file_name, _)| *file_name == name) {
                let node = MountNode::Virtual(virtual_name);
                let kind = EntryKind::Folder;
                let inode = self.inodes.lock().expect("inode table lock").assign(node.clone(), kind.clone(), PathBuf::from(&name), true);
                return reply.entry(&FILE_TTL, &self.attributes(inode, &node, &kind), self.generation);
            }
        }
        let tree = self.tree.clone();
        let inodes = self.inodes.clone();
        let filesystem = self.snapshot();
        let path = parent_record.path.join(&name);
        let folder = MountNode::Tree(parent_key.clone());
        self.spawn(async move {
            let found = list_detached(tree, parent_key, within(LISTING_DEADLINE, 1)).await.map(|listing| listing.map(|entries| entries.into_iter().find(|entry| entry.name == name)));
            match found {
                Ok(Ok(Some(entry))) => {
                    let node = MountNode::Tree(entry.node.clone());
                    let inode = inodes.lock().expect("inode table lock").assign(node.clone(), entry.kind.clone(), path, true);
                    reply.entry(&ttl_for(&entry.kind), &filesystem.attributes(inode, &node, &entry.kind), filesystem.generation);
                }
                // A negative entry (node id 0) is `ENOENT` that the kernel remembers.
                Ok(Ok(None)) if is_probe_name(&name) => reply.entry(&NEGATIVE_TTL, &filesystem.attributes(0, &folder, &EntryKind::Folder), filesystem.generation),
                Ok(Ok(None)) => reply.error(Errno::ENOENT),
                Ok(Err(error)) => reply.error(errno_for(&error)),
                Err(_) => reply.error(Errno::EIO),
            }
        });
    }

    fn forget(&self, _request: &Request, inode: INodeNo, lookups: u64) {
        self.inodes.lock().expect("inode table lock").forget(inode.0, lookups);
    }

    fn getattr(&self, _request: &Request, inode: INodeNo, _handle: Option<FileHandle>, reply: ReplyAttr) {
        let record = self.inodes.lock().expect("inode table lock").record(inode.0);
        if let Some(InodeRecord { node: MountNode::Staged(id), .. }) = record {
            return match self.uploads.as_ref().and_then(|uploads| uploads.staging.get(id)) {
                Some(file) => reply.attr(&Duration::ZERO, &self.staged_attributes(inode.0, id, &file)),
                None => reply.error(Errno::ENOENT),
            };
        }
        match record {
            Some(record) => reply.attr(&ttl_for(&record.kind), &self.attributes(inode.0, &record.node, &record.kind)),
            None => reply.error(Errno::ENOENT),
        }
    }

    fn open(&self, request: &Request, inode: INodeNo, flags: OpenFlags, reply: ReplyOpen) {
        let record = self.inodes.lock().expect("inode table lock").record(inode.0);
        let access_mode = flags.0 & libc::O_ACCMODE;
        if let Some(InodeRecord { node: MountNode::Staged(id), .. }) = record {
            // Every open of a staged file gets its own handle, which knows whether it may write.
            return match self.uploads.as_ref().and_then(|uploads| uploads.staging.open(id, access_mode != libc::O_RDONLY)) {
                Some(handle) => reply.opened(FileHandle(handle), FopenFlags::empty()),
                None => reply.error(Errno::ENOENT),
            };
        }
        if access_mode != libc::O_RDONLY {
            let in_photos = record.as_ref().is_some_and(|record| matches!(&record.node, MountNode::Tree(NodeKey::Asset { parent, .. }) if is_photos_folder(parent)));
            return reply.error(if in_photos && self.uploads.is_some() { Errno::EPERM } else { Errno::EROFS });
        }
        if inode.0 != INodeNo::ROOT.0 {
            if let Some(indexer) = caller::indexer(request.pid()) {
                log::debug!("refusing an original to the indexer {indexer}");
                return reply.error(Errno::EIO);
            }
        }
        if self.thumbnails.as_ref().is_some_and(|thumbnails| thumbnails.safety_net) && inode.0 != INodeNo::ROOT.0 {
            if let Some(thumbnailer) = caller::thumbnailer(request.pid()) {
                log::debug!("refusing an original to the thumbnailer {thumbnailer}");
                return reply.error(Errno::EIO);
            }
        }
        match self.inodes.lock().expect("inode table lock").record(inode.0) {
            Some(record) if !matches!(record.kind, EntryKind::Folder) || matches!(record.node, MountNode::Virtual(_)) => {
                // The kernel keeps its pages of the file from an earlier open, unless the original changed since.
                let keep = match &record.kind {
                    EntryKind::File { asset_id, .. } => self.content.keep_page_cache(asset_id),
                    EntryKind::Folder => true,
                };
                self.open_files.fetch_add(1, Ordering::Relaxed);
                reply.opened(FileHandle(LIBRARY_FILE_HANDLE), if keep { FopenFlags::FOPEN_KEEP_CACHE } else { FopenFlags::empty() })
            }
            Some(_) => reply.error(Errno::EISDIR),
            None => reply.error(Errno::ENOENT),
        }
    }

    fn read(
        &self,
        _request: &Request,
        inode: INodeNo,
        _handle: FileHandle,
        offset: u64,
        size: u32,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        reply: ReplyData,
    ) {
        let Some(record) = self.inodes.lock().expect("inode table lock").record(inode.0) else {
            return reply.error(Errno::ENOENT);
        };
        match (&record.node, &record.kind) {
            (MountNode::Staged(id), _) => match self.uploads.as_ref().map(|uploads| uploads.staging.read(*id, offset, size)) {
                Some(Ok(bytes)) => reply.data(&bytes),
                _ => reply.error(Errno::EIO),
            },
            (MountNode::Virtual(name), _) => {
                let bytes = virtual_content(name).as_bytes();
                let start = (offset as usize).min(bytes.len());
                let end = (start + size as usize).min(bytes.len());
                reply.data(&bytes[start..end]);
            }
            (node, EntryKind::File { asset_id, size: Some(total_size), .. }) => {
                let content = self.content.clone();
                let asset_id = asset_id.clone();
                let total_size = *total_size;
                // The folder the file was listed in, to read again if the listing turns out stale.
                let folder = match node {
                    MountNode::Tree(NodeKey::Asset { parent, .. }) => Some((**parent).clone()),
                    _ => None,
                };
                let tree = self.tree.clone();
                self.spawn(async move {
                    let generation = content.generation(&asset_id);
                    // The fetch runs as a task of its own: past the deadline it goes on filling the cache, so on a slow
                    // link the next read finds the block instead of starting it again.
                    let fetching = {
                        let (content, asset_id) = (content.clone(), asset_id.clone());
                        tokio::spawn(async move { content.read(&asset_id, total_size, offset, size).await })
                    };
                    let outcome = match tokio::time::timeout(within(READ_DEADLINE, 1), fetching).await {
                        Ok(Ok(outcome)) => Ok(outcome),
                        Ok(Err(failure)) => Ok(Err(SourceError::Protocol(format!("the read task failed: {failure}")))),
                        Err(elapsed) => Err(elapsed),
                    };
                    match outcome {
                        Ok(Ok(bytes)) => {
                            reply.data(&bytes);
                            content.read_ahead(&asset_id, total_size, offset, bytes.len() as u64);
                            // The cache found a new version of the original: its folder may list an old size or date.
                            if let Some(folder) = folder.as_ref().filter(|_| content.generation(&asset_id) != generation) {
                                tree.invalidate(folder);
                            }
                        }
                        Ok(Err(error)) => {
                            // Gone from Immich, or another size than listed: the folder's next visit asks again
                            // (SPEC.md, "Errors seen by programs using the mount").
                            if let (SourceError::NotFound | SourceError::Changed, Some(folder)) = (&error, &folder) {
                                log::info!("{asset_id} is gone or changed on the server; its folder will be read again");
                                tree.invalidate(folder);
                            }
                            reply.error(errno_for(&error))
                        }
                        Err(_) => reply.error(Errno::EIO),
                    }
                });
            }
            // Without a known size the bytes cannot be served correctly; an empty answer would make copies silently
            // empty (principle 3).
            (_, EntryKind::File { size: None, .. }) => reply.error(Errno::EIO),
            (_, EntryKind::Folder) => reply.error(Errno::EISDIR),
        }
    }

    fn opendir(&self, _request: &Request, _inode: INodeNo, _flags: OpenFlags, reply: ReplyOpen) {
        reply.opened(FileHandle(self.next_folder_handle.fetch_add(1, Ordering::Relaxed)), FopenFlags::empty());
    }

    fn releasedir(&self, _request: &Request, _inode: INodeNo, handle: FileHandle, _flags: OpenFlags, reply: ReplyEmpty) {
        self.listings.lock().expect("listings lock").remove(&handle.0);
        reply.ok();
    }

    fn readdir(&self, request: &Request, inode: INodeNo, handle: FileHandle, offset: u64, mut reply: ReplyDirectory) {
        // Later pages come from the listing taken with the first.
        if offset > 0 {
            let taken = self.listings.lock().expect("listings lock").get(&handle.0).cloned();
            if let Some(listing) = taken {
                for (index, (child, file_type, name)) in listing.iter().enumerate().skip(offset as usize) {
                    if reply.add(INodeNo(*child), (index + 1) as u64, *file_type, name) {
                        break;
                    }
                }
                return reply.ok();
            }
        }
        let Some(record) = self.inodes.lock().expect("inode table lock").record(inode.0) else {
            return reply.error(Errno::ENOENT);
        };
        let MountNode::Tree(key) = record.node else {
            return reply.error(Errno::ENOTDIR);
        };
        let tree = self.tree.clone();
        let inodes = self.inodes.clone();
        let folder_path = record.path.clone();
        let staged = self.uploads.as_ref().map(|uploads| uploads.staging.in_folder(&key)).unwrap_or_default();
        let listings = self.listings.clone();
        // Only the first page of a listing seeds, and never for a search crawl.
        let seeding = match &self.thumbnails {
            Some(thumbnails) if offset == 0 => {
                if caller::is_search_crawl(request.pid()) {
                    log::debug!("not seeding {} for a search crawl", folder_path.display());
                    None
                } else {
                    Some((thumbnails.seeder.clone(), thumbnails.mount_point.join(&folder_path)))
                }
            }
            _ => None,
        };
        self.spawn(async move {
            // Listing and seeding share the request: under a kernel limit each gets half of it.
            let entries: Vec<Entry> = match list_detached(tree, key.clone(), within(LISTING_DEADLINE, 2)).await {
                Ok(Ok(entries)) => entries,
                Ok(Err(error)) => return reply.error(errno_for(&error)),
                Err(_) => return reply.error(Errno::EIO),
            };
            if let Some((seeder, folder)) = seeding {
                let files: Vec<SeedFile> = entries
                    .iter()
                    .filter_map(|entry| match &entry.kind {
                        EntryKind::File { asset_id, modified, .. } => {
                            Some(SeedFile { path: folder.join(&entry.name), asset_id: asset_id.clone(), modified: *modified })
                        }
                        EntryKind::Folder => None,
                    })
                    .collect();
                if !files.is_empty() {
                    // Past the budget the listing answers anyway; the seeding task carries on.
                    let _ = tokio::time::timeout(within(SEEDING_BUDGET, 2), seeder.seed(files)).await;
                }
            }
            let mut listing: Listing = vec![
                (inode.0, FileType::Directory, ".".to_string()),
                (inode.0, FileType::Directory, "..".to_string()),
            ];
            {
                let mut table = inodes.lock().expect("inode table lock");
                if key == NodeKey::Root {
                    for (name, _) in VIRTUAL_FILES {
                        let child = table.assign(MountNode::Virtual(name), EntryKind::Folder, folder_path.join(name), false);
                        listing.push((child, FileType::RegularFile, name.to_string()));
                    }
                }
                for entry in entries {
                    let file_type = if matches!(entry.kind, EntryKind::Folder) { FileType::Directory } else { FileType::RegularFile };
                    let child = table.assign(MountNode::Tree(entry.node), entry.kind, folder_path.join(&entry.name), false);
                    listing.push((child, file_type, entry.name));
                }
                for (id, file) in staged {
                    let child = table.assign(MountNode::Staged(id), staged_kind(&file), folder_path.join(&file.name), false);
                    listing.push((child, FileType::RegularFile, file.name));
                }
            }
            let listing = Arc::new(listing);
            listings.lock().expect("listings lock").insert(handle.0, listing.clone());
            for (index, (child, file_type, name)) in listing.iter().enumerate().skip(offset as usize) {
                if reply.add(INodeNo(*child), (index + 1) as u64, *file_type, name) {
                    break;
                }
            }
            reply.ok();
        });
    }

    /// `close()` waits for this, so the upload runs here: success only once the server has the file (principle 6).
    /// A reader's close does nothing, a close during an upload waits for it, and a close after a failed one fails.
    fn flush(&self, request: &Request, inode: INodeNo, handle: FileHandle, _lock_owner: LockOwner, reply: ReplyEmpty) {
        let record = self.inodes.lock().expect("inode table lock").record(inode.0);
        let (Some(InodeRecord { node: MountNode::Staged(_), .. }), Some(uploads)) = (record, &self.uploads) else {
            return reply.ok();
        };
        match uploads.staging.close(handle.0) {
            CloseAction::Done => reply.ok(),
            CloseAction::Fail(errno) => reply.error(Errno::from_i32(errno)),
            CloseAction::Wait(receiver) => self.spawn(async move {
                match wait_for(receiver).await {
                    Ok(()) => reply.ok(),
                    Err(errno) => reply.error(Errno::from_i32(errno)),
                }
            }),
            CloseAction::Upload(pending) if caller::was_killed(request.pid()) => {
                // Killed mid-copy (Ctrl+C): the bytes are partial, so they are not uploaded.
                log::warn!("not uploading {}: its writer was killed", pending.name);
                uploads.staging.finish_upload(pending, Err(libc::EIO));
                reply.error(Errno::EIO);
            }
            CloseAction::Upload(pending) => {
                let (target, staging, tree) = (uploads.target.clone(), uploads.staging.clone(), self.tree.clone());
                self.spawn(async move {
                    match run_upload(target, staging, tree, pending).await {
                        Ok(()) => reply.ok(),
                        Err(errno) => reply.error(Errno::from_i32(errno)),
                    }
                });
            }
        }
    }

    fn release(
        &self,
        _request: &Request,
        inode: INodeNo,
        handle: FileHandle,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        _flush: bool,
        reply: ReplyEmpty,
    ) {
        if handle.0 == LIBRARY_FILE_HANDLE {
            // Each such handle came from one counted open, and the kernel releases each open once.
            self.open_files.fetch_sub(1, Ordering::Relaxed);
            return reply.ok();
        }
        let record = self.inodes.lock().expect("inode table lock").record(inode.0);
        if let (Some(InodeRecord { node: MountNode::Staged(_), .. }), Some(uploads)) = (record, &self.uploads) {
            uploads.staging.release(handle.0);
        }
        reply.ok();
    }

    fn create(&self, _request: &Request, parent: INodeNo, name: &OsStr, _mode: u32, _umask: u32, _flags: i32, reply: ReplyCreate) {
        let (folder, folder_path) = match self.parent_folder(parent) {
            Ok(found) => found,
            Err(errno) => return reply.error(errno),
        };
        let Some(uploads) = self.uploads.as_ref().filter(|_| is_photos_folder(&folder)) else {
            return reply.error(Errno::EROFS);
        };
        let Some(name) = name.to_str().map(str::to_owned) else {
            return reply.error(Errno::EINVAL);
        };
        let admission = uploads.media.admission(&name);
        if admission == Admission::Refused {
            // Refused before a byte is copied: not an image or video Immich takes (SPEC.md, "Uploads").
            let name = name.clone();
            self.spawn(async move {
                crate::notify::notify_once_in_a_while("refused", &format!("Not uploaded: {name}"), "Immich takes photos and videos only.").await;
            });
            return reply.error(Errno::ENOTSUP);
        }
        let (staging, tree, inodes, filesystem) = (uploads.staging.clone(), self.tree.clone(), self.inodes.clone(), self.snapshot());
        self.spawn(async move {
            // A name an Asset already has in this folder is taken.
            match list_detached(tree, folder.clone(), within(LISTING_DEADLINE, 1)).await {
                Ok(Ok(entries)) if entries.iter().any(|entry| entry.name == name) => return reply.error(Errno::EEXIST),
                Ok(Ok(_)) => {}
                Ok(Err(error)) => return reply.error(errno_for(&error)),
                Err(_) => return reply.error(Errno::EIO),
            }
            let (id, handle) = match staging.create(&folder, &name, admission == Admission::Staged) {
                Ok(created) => created,
                Err(error) => {
                    log::warn!("could not stage {name}: {error}");
                    return reply.error(Errno::EIO);
                }
            };
            let Some(file) = staging.get(id) else { return reply.error(Errno::EIO) };
            let node = MountNode::Staged(id);
            let kind = staged_kind(&file);
            let inode = inodes.lock().expect("inode table lock").assign(node.clone(), kind.clone(), folder_path.join(&name), true);
            reply.created(&Duration::ZERO, &filesystem.attributes(inode, &node, &kind), filesystem.generation, FileHandle(handle), FopenFlags::empty());
        });
    }

    fn write(
        &self,
        _request: &Request,
        inode: INodeNo,
        handle: FileHandle,
        offset: u64,
        data: &[u8],
        _write_flags: WriteFlags,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        reply: ReplyWrite,
    ) {
        let record = self.inodes.lock().expect("inode table lock").record(inode.0);
        let (Some(InodeRecord { node: MountNode::Staged(_), .. }), Some(uploads)) = (record, &self.uploads) else {
            return reply.error(Errno::EROFS);
        };
        match uploads.staging.write(handle.0, offset, data) {
            Ok(written) => reply.written(written as u32),
            Err(errno) => reply.error(Errno::from_i32(errno)),
        }
    }

    fn setattr(
        &self,
        _request: &Request,
        inode: INodeNo,
        _mode: Option<u32>,
        _uid: Option<u32>,
        _gid: Option<u32>,
        size: Option<u64>,
        _atime: Option<TimeOrNow>,
        mtime: Option<TimeOrNow>,
        _ctime: Option<SystemTime>,
        handle: Option<FileHandle>,
        _crtime: Option<SystemTime>,
        _chgtime: Option<SystemTime>,
        _bkuptime: Option<SystemTime>,
        _flags: Option<BsdFileFlags>,
        reply: ReplyAttr,
    ) {
        let record = self.inodes.lock().expect("inode table lock").record(inode.0);
        match (record, &self.uploads) {
            (Some(InodeRecord { node: MountNode::Staged(id), .. }), Some(uploads)) => {
                let modified = mtime.map(|time| match time {
                    TimeOrNow::SpecificTime(time) => time,
                    TimeOrNow::Now => SystemTime::now(),
                });
                // Mode and owner changes (`cp -p`) are accepted and ignored: staged files are always the user's.
                match uploads.staging.set_attributes(id, handle.map(|handle| handle.0), size, modified) {
                    Ok(()) => match uploads.staging.get(id) {
                        Some(file) => reply.attr(&Duration::ZERO, &self.staged_attributes(inode.0, id, &file)),
                        None => reply.error(Errno::ENOENT),
                    },
                    Err(errno) => reply.error(Errno::from_i32(errno)),
                }
            }
            (Some(InodeRecord { node: MountNode::Tree(NodeKey::Asset { parent, .. }), .. }), _) => reply.error(self.refusal(&parent)),
            (Some(InodeRecord { node: MountNode::Tree(key), .. }), _) => reply.error(self.refusal(&key)),
            (Some(_), _) => reply.error(Errno::EROFS),
            (None, _) => reply.error(Errno::ENOENT),
        }
    }

    /// Only a staged file can be renamed, never an Asset (principle 4), and never onto an Asset's name. A temporary
    /// name made real uploads at the writer's close if it is still open (GIO renames before closing), so the folder
    /// is never locked for an upload; a file already closed (rsync) uploads here, and this rename reports the result.
    fn rename(&self, _request: &Request, parent: INodeNo, name: &OsStr, new_parent: INodeNo, new_name: &OsStr, flags: RenameFlags, reply: ReplyEmpty) {
        let ((folder, _), (new_folder, new_folder_path)) = match (self.parent_folder(parent), self.parent_folder(new_parent)) {
            (Ok(from), Ok(to)) => (from, to),
            (Err(errno), _) | (_, Err(errno)) => return reply.error(errno),
        };
        let (Some(name), Some(new_name)) = (name.to_str(), new_name.to_str().map(str::to_owned)) else {
            return reply.error(Errno::EINVAL);
        };
        let Some(uploads) = &self.uploads else { return reply.error(Errno::EROFS) };
        let Some((id, file)) = uploads.staging.find(&folder, name) else {
            return reply.error(self.refusal(&folder));
        };
        if file.is_receipt() {
            return reply.error(Errno::EPERM);
        }
        if flags.contains(RenameFlags::RENAME_EXCHANGE) || flags.contains(RenameFlags::RENAME_WHITEOUT) {
            return reply.error(Errno::EINVAL);
        }
        if !is_photos_folder(&new_folder) {
            return reply.error(Errno::EROFS);
        }
        let admission = uploads.media.admission(&new_name);
        if admission == Admission::Refused {
            return reply.error(Errno::ENOTSUP);
        }
        if flags.contains(RenameFlags::RENAME_NOREPLACE) && uploads.staging.find(&new_folder, &new_name).is_some() {
            return reply.error(Errno::EEXIST);
        }
        let (target, staging, tree, inodes) = (uploads.target.clone(), uploads.staging.clone(), self.tree.clone(), self.inodes.clone());
        self.spawn(async move {
            // An Asset's name in the target folder is taken (principle 8).
            match list_detached(tree.clone(), new_folder.clone(), within(LISTING_DEADLINE, 1)).await {
                Ok(Ok(entries)) if entries.iter().any(|entry| entry.name == new_name) => return reply.error(Errno::EEXIST),
                Ok(Ok(_)) => {}
                Ok(Err(error)) => return reply.error(errno_for(&error)),
                Err(_) => return reply.error(Errno::EIO),
            }
            if let Err(errno) = staging.rename(id, &new_folder, &new_name, admission == Admission::Staged) {
                return reply.error(Errno::from_i32(errno));
            }
            inodes.lock().expect("inode table lock").assign(MountNode::Staged(id), staged_kind(&file), new_folder_path.join(&new_name), false);
            match staging.upload_after_rename(id) {
                None => reply.ok(),
                Some(pending) => match run_upload(target, staging, tree, pending).await {
                    Ok(()) => reply.ok(),
                    Err(errno) => reply.error(Errno::from_i32(errno)),
                },
            }
        });
    }

    /// Removing a staged file that was not uploaded is allowed (a cancelled copy cleaning up); a receipt or an Asset
    /// cannot be removed, since that would change nothing on the server, or the library itself (principles 4, 5).
    fn unlink(&self, _request: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEmpty) {
        let (folder, _) = match self.parent_folder(parent) {
            Ok(found) => found,
            Err(errno) => return reply.error(errno),
        };
        let staged = name.to_str().and_then(|name| self.uploads.as_ref().and_then(|uploads| uploads.staging.find(&folder, name)));
        match (staged, &self.uploads) {
            (Some((id, _)), Some(uploads)) => match uploads.staging.remove(id) {
                Ok(()) => reply.ok(),
                Err(errno) => reply.error(Errno::from_i32(errno)),
            },
            _ => reply.error(self.refusal(&folder)),
        }
    }

    fn mkdir(&self, _request: &Request, parent: INodeNo, _name: &OsStr, _mode: u32, _umask: u32, reply: ReplyEntry) {
        match self.parent_folder(parent) {
            Ok((folder, _)) => reply.error(self.refusal(&folder)),
            Err(errno) => reply.error(errno),
        }
    }

    fn rmdir(&self, _request: &Request, parent: INodeNo, _name: &OsStr, reply: ReplyEmpty) {
        match self.parent_folder(parent) {
            Ok((folder, _)) => reply.error(self.refusal(&folder)),
            Err(errno) => reply.error(errno),
        }
    }

    // The next two tell the kernel once that the mount has no extended attributes, so it stops asking. fuser's
    // defaults answer the same, but log a warning each time.
    fn getxattr(&self, _request: &Request, _inode: INodeNo, _name: &OsStr, _size: u32, reply: ReplyXattr) {
        reply.error(Errno::ENOSYS);
    }

    fn listxattr(&self, _request: &Request, _inode: INodeNo, _size: u32, reply: ReplyXattr) {
        reply.error(Errno::ENOSYS);
    }

    /// With uploads on, the staging filesystem's free space, so a file manager's space check before a copy is
    /// meaningful (ARCHITECTURE.md, "Uploads").
    fn statfs(&self, _request: &Request, _inode: INodeNo, reply: ReplyStatfs) {
        if let Some(space) = self.uploads.as_ref().and_then(|uploads| free_space(uploads.staging.directory())) {
            return reply.statfs(space.blocks, space.free, space.available, 0, 0, space.block_size, 255, space.block_size);
        }
        reply.statfs(1 << 28, 0, 0, 0, 0, 4096, 255, 4096);
    }
}

pub(crate) struct FreeSpace {
    pub(crate) blocks: u64,
    free: u64,
    pub(crate) available: u64,
    pub(crate) block_size: u32,
}

pub(crate) fn free_space(directory: &Path) -> Option<FreeSpace> {
    let path = std::ffi::CString::new(directory.as_os_str().as_encoded_bytes()).ok()?;
    let mut stats: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(path.as_ptr(), &mut stats) } != 0 {
        return None;
    }
    Some(FreeSpace {
        blocks: stats.f_blocks as u64,
        free: stats.f_bfree as u64,
        available: stats.f_bavail as u64,
        block_size: stats.f_frsize as u32,
    })
}

impl<Library, Names, Content> ImmichFilesystem<Library, Names, Content> {
    /// The parts of the filesystem a spawned task needs to build attributes.
    fn snapshot(&self) -> AttributeMaker {
        AttributeMaker {
            owner_uid: self.owner_uid,
            owner_gid: self.owner_gid,
            mounted_at: self.mounted_at,
            generation: self.generation,
            photos_writable: self.uploads.is_some(),
        }
    }
}

/// Builds attributes inside spawned tasks, without holding the filesystem.
#[derive(Clone, Copy)]
struct AttributeMaker {
    owner_uid: u32,
    owner_gid: u32,
    mounted_at: SystemTime,
    generation: Generation,
    /// Photos' folders take new files (0755); everything else is read-only (0555, 0444).
    photos_writable: bool,
}

impl AttributeMaker {
    fn attributes(&self, inode: u64, node: &MountNode, kind: &EntryKind) -> FileAttr {
        let (file_type, permissions, size, modified) = match (node, kind) {
            (MountNode::Virtual(name), _) => (FileType::RegularFile, 0o444, virtual_content(name).len() as u64, self.mounted_at),
            (MountNode::Staged(_), EntryKind::File { size, modified, .. }) => {
                (FileType::RegularFile, 0o644, size.unwrap_or(0), SystemTime::from(*modified))
            }
            (MountNode::Tree(key), EntryKind::Folder) if self.photos_writable && is_photos_folder(key) => {
                (FileType::Directory, 0o755, 0, self.mounted_at)
            }
            (_, EntryKind::Folder) => (FileType::Directory, 0o555, 0, self.mounted_at),
            (_, EntryKind::File { size, modified, .. }) => (FileType::RegularFile, 0o444, size.unwrap_or(0), SystemTime::from(*modified)),
        };
        FileAttr {
            ino: INodeNo(inode),
            size,
            blocks: size.div_ceil(512),
            atime: modified,
            mtime: modified,
            ctime: modified,
            crtime: modified,
            kind: file_type,
            perm: permissions,
            nlink: if file_type == FileType::Directory { 2 } else { 1 },
            uid: self.owner_uid,
            gid: self.owner_gid,
            rdev: 0,
            blksize: 4096,
            flags: 0,
        }
    }
}

/// Mount the filesystem at `mountpoint`, serving requests on a background thread until the returned session is
/// dropped. Without uploads the whole mount is read-only at the kernel level too.
pub fn mount<Library, Names, Content>(
    filesystem: ImmichFilesystem<Library, Names, Content>,
    mountpoint: &Path,
) -> std::io::Result<BackgroundSession>
where
    Library: LibrarySource + 'static,
    Names: NameMemory + 'static,
    Content: ContentSource + ThumbnailSource + UploadTarget + 'static,
{
    let limit = *REQUEST_LIMIT.get_or_init(kernel_request_limit);
    if let Some(limit) = limit {
        log::warn!(
            "this system limits one FUSE request to {} s (fs.fuse.default_request_timeout); uploads and reads longer than that fail, so the mount is not aborted",
            limit.as_secs()
        );
    }
    let mut config = Config::default();
    config.mount_options = vec![
        MountOption::FSName("immich".to_string()),
        MountOption::Subtype("immich".to_string()),
        MountOption::NoSuid,
        MountOption::NoDev,
        MountOption::NoExec,
        MountOption::DefaultPermissions,
    ];
    if filesystem.uploads.is_none() {
        config.mount_options.push(MountOption::RO);
    }
    fuser::spawn_mount(filesystem, mountpoint, &config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_names_that_never_name_an_asset_are_remembered_as_absent() {
        for probe in [".directory", ".hidden", ".Trash-1000", "autorun.inf", "desktop.ini", "Thumbs.db"] {
            assert!(is_probe_name(probe), "{probe}");
        }
        for asset in ["2024-03-15 14.32.07.jpg", "IMG_1234.JPG", "2023 - Summer trip", ".Private", ".hidden photo.jpg"] {
            assert!(!is_probe_name(asset), "{asset}");
        }
    }
}
