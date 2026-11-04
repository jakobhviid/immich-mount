//! The content cache: originals fetched on demand in 2 MiB blocks with range requests (ARCHITECTURE.md, "Content").
//!
//! Each Asset's bytes live in one sparse file in the cache directory, `<id>.bin`, beside its block map, `<id>.blocks`
//! (one line per block: index and length; and a `version` line with the original's ETag).
//! - A block counts as present only after it is written and synced, and only then is it added to the map, so a
//!   crash never serves zeros. The map survives restarts.
//! - Each block is fetched once, however many reads wait for it.
//! - Past the size limit, whole Assets are evicted, least recently used first. An Asset being read is pinned and
//!   never evicted.
//! - Every block carries the version of the original it came from (its ETag). A block of another version means the
//!   original was replaced on the server: the old blocks are dropped, so a file never mixes the two, and a read
//!   that overlapped the change reads again.
//! - Peeks are served from an Asset's edges, `<id>.edges`: the start and end of the original, fetched together the
//!   first time a program peeks at either, and for a whole folder once a program peeks at its files one after
//!   another (`prefetch_edges`).
//!
//! Read-ahead comes on top of this (ROADMAP.md, M2).

use std::collections::{HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::future::Future;
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, FileExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::source::SourceError;

/// The size of one cached block.
pub const BLOCK_SIZE: u64 = 2 << 20;

/// How often the disk floor asks how much room the disk has.
const DISK_PROBE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);

/// Some bytes of an original, and which version of it they come from: the server's ETag, when it sends one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangeBytes {
    pub bytes: Vec<u8>,
    pub version: Option<String>,
}

/// Where original bytes come from.
pub trait ContentSource: Send + Sync {
    /// Bytes `start..=end_inclusive` of an Asset's original, with the original's version.
    fn read_range(&self, asset_id: &str, start: u64, end_inclusive: u64) -> impl Future<Output = Result<RangeBytes, SourceError>> + Send;
}

/// One source can serve several users through an `Arc`.
impl<Shared: ContentSource> ContentSource for Arc<Shared> {
    async fn read_range(&self, asset_id: &str, start: u64, end_inclusive: u64) -> Result<RangeBytes, SourceError> {
        self.as_ref().read_range(asset_id, start, end_inclusive).await
    }
}

/// How much room the cache's disk has, in bytes: what this user may still write, and the disk's size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiskRoom {
    pub available: u64,
    pub total: u64,
}

/// Asks the platform how much room the cache's disk has (`statvfs` on Linux). `None` when it cannot tell.
pub type DiskRoomProbe = Arc<dyn Fn() -> Option<DiskRoom> + Send + Sync>;

/// One gate per block being fetched, so concurrent reads wait for a single request.
type BlockGates = HashMap<(String, u64), Arc<tokio::sync::Mutex<()>>>;

/// The on-disk block cache in front of a `ContentSource`.
pub struct BlockCache<Source> {
    source: Source,
    directory: PathBuf,
    limit_bytes: u64,
    /// Lowers the limit so the cache never takes the disk's last tenth (`with_disk_floor`).
    disk_room: Option<DiskRoomProbe>,
    /// The disk floor's limit and when it was worked out (`effective_limit`).
    disk_limit: Mutex<Option<(std::time::Instant, u64)>>,
    state: Mutex<CacheState>,
    in_flight: Mutex<BlockGates>,
    /// One gate per Asset around writing a block, so its version check and its files change together.
    writes: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    read_ahead: Mutex<HashMap<String, SequentialRun>>,
    /// Read-ahead fetches at once. Small, so reads someone is waiting for go first.
    read_ahead_budget: Arc<tokio::sync::Semaphore>,
    /// One gate per Asset whose edges are being fetched, so concurrent peeks wait for a single fetch.
    edge_gates: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// Edge fetches at once for prefetches; a peek someone is waiting for does not wait for these.
    edge_prefetches: Arc<tokio::sync::Semaphore>,
    /// Bytes of originals fetched from the server, and in how many requests, since the cache was made.
    fetched_bytes: std::sync::atomic::AtomicU64,
    fetch_requests: std::sync::atomic::AtomicU64,
}

/// A run of sequential reads of one Asset, which read-ahead follows.
#[derive(Debug, Clone, Copy, Default)]
struct SequentialRun {
    /// Where the next read of the run would start.
    next_offset: u64,
    /// How much the run has read so far.
    bytes: u64,
    /// How many blocks ahead to fetch.
    window: u64,
    /// The first block not yet scheduled by this run, so each block is queued once however many reads follow.
    scheduled_until: u64,
}

/// Read-ahead starts only after this much sequential reading, so a program peeking at a file's header (a type
/// sniffer, a thumbnailer) never pulls a block, let alone the original (principle 2).
const READ_AHEAD_AFTER: u64 = 2 * BLOCK_SIZE;
/// A read into bytes the cache does not hold is a peek while the run of reads it belongs to stays within this much:
/// it is served from the Asset's edges (its first `HEAD_BYTES` and last `TAIL_BYTES`), or fetches just its own range,
/// never whole blocks, which for most photos are the whole original. A program peeking at every file of a folder
/// (rom-properties, which Bazzite's file managers load, reads each file's first 8 KiB and its last page) then costs
/// two small requests per file, once. The kernel's own read-ahead after a peek (16 to 48 KiB) stays within it; a copy
/// or a viewer passes it at its first or second read.
const PEEK_LIMIT: u64 = 64 * 1024;
/// The read-ahead window, in blocks: it starts at 2 and doubles up to 16 (32 MiB).
const READ_AHEAD_MIN: u64 = 2;
const READ_AHEAD_MAX: u64 = 16;
/// How much of an original's start its edges keep: a peek at a file's first 8 KiB reaches the daemon as its first
/// 48 KiB, after the kernel's own read-ahead.
const HEAD_BYTES: u64 = PEEK_LIMIT;
/// How much of an original's end its edges keep: the last page, with room to spare.
const TAIL_BYTES: u64 = 16 * 1024;
/// Files whose edges a folder's prefetch fetches at once (two requests each).
const PARALLEL_EDGE_PREFETCHES: usize = 8;
/// The most files one prefetch fetches the edges of, 80 MB: a folder of files holds fewer with the default date
/// groups (`grouping`), and a program peeking at more reads the rest itself.
const MOST_PREFETCHED: usize = 1000;

/// The lengths of an original's edges, its start and its end; a file too small to part is kept whole as its start.
fn edge_lengths(total_size: u64) -> (u64, u64) {
    if total_size <= HEAD_BYTES + TAIL_BYTES {
        (total_size, 0)
    } else {
        (HEAD_BYTES, TAIL_BYTES)
    }
}

/// Which original an Asset's edges file holds the start and end of.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Edges {
    size: u64,
    /// The original's version (its ETag), when the server sent one.
    version: Option<String>,
}

impl Edges {
    /// The bytes the edges hold.
    fn length(&self) -> u64 {
        let (head, tail) = edge_lengths(self.size);
        head + tail
    }

    /// The first line of an edges file: what it holds, and a checksum of the bytes after it.
    fn header(&self, payload: &[u8]) -> String {
        let (head, tail) = edge_lengths(self.size);
        format!("edges {} {head} {tail} {:016x} {}\n", self.size, fnv1a(payload), self.version.as_deref().unwrap_or("-"))
    }

    /// The edges and the checksum a header line names, if it is one this version writes.
    fn parse_header(line: &str) -> Option<(Edges, u64)> {
        let mut fields = line.trim_end_matches('\n').splitn(6, ' ');
        if fields.next()? != "edges" {
            return None;
        }
        let size: u64 = fields.next()?.parse().ok()?;
        let head: u64 = fields.next()?.parse().ok()?;
        let tail: u64 = fields.next()?.parse().ok()?;
        let checksum = u64::from_str_radix(fields.next()?, 16).ok()?;
        let version = match fields.next()? {
            "-" => None,
            version => Some(version.to_owned()),
        };
        ((head, tail) == edge_lengths(size)).then_some((Edges { size, version }, checksum))
    }
}

/// FNV-1a, 64-bit: tells a torn edges file from a whole one. Edges files are not synced, since a torn one is merely
/// fetched again.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3))
}

/// An edges file's record and the bytes after its header, if the file is whole.
fn read_edges_file(path: &Path) -> Option<(Edges, Vec<u8>)> {
    let mut contents = std::fs::read(path).ok()?;
    let newline = contents.iter().position(|byte| *byte == b'\n')?;
    let (edges, checksum) = Edges::parse_header(std::str::from_utf8(&contents[..newline]).ok()?)?;
    let payload = contents.split_off(newline + 1);
    (payload.len() as u64 == edges.length() && fnv1a(&payload) == checksum).then_some((edges, payload))
}

/// An edges file's record, from its header alone, if the file has the length the header promises.
fn edges_file_record(path: &Path) -> Option<Edges> {
    use std::io::{BufRead, Read};
    let file = File::open(path).ok()?;
    let length = file.metadata().ok()?.len();
    let mut header = String::new();
    std::io::BufReader::new(file).take(512).read_line(&mut header).ok()?;
    let (edges, _) = Edges::parse_header(&header)?;
    (length == header.len() as u64 + edges.length()).then_some(edges)
}

/// What the cache holds, kept in step with the files.
#[derive(Default)]
struct CacheState {
    entries: HashMap<String, CachedAsset>,
    /// The sum of every present block's length.
    bytes: u64,
    /// A counter that orders uses; higher is more recent.
    clock: u64,
}

#[derive(Default)]
struct CachedAsset {
    blocks: HashSet<u64>,
    bytes: u64,
    last_used: u64,
    /// Reads in progress. A pinned Asset is never evicted.
    pins: usize,
    /// The original's version (its ETag) the present blocks come from, once known.
    version: Option<String>,
    /// The original's size the present blocks belong to, once known, and whether the block map says it yet.
    size: Option<u64>,
    size_recorded: bool,
    /// The blocks were dropped: the Asset's files are emptied before the next block is written.
    rewrite: bool,
    /// Counts the times the blocks were dropped (a new version or size), so a read can tell it overlapped one.
    generation: u64,
    /// The generation at the last open, so the next open can tell whether the kernel's pages are still good.
    opened_generation: Option<u64>,
    /// The original whose start and end the edges file holds, if there is one. Its bytes count in `bytes`.
    edges: Option<Edges>,
}

impl CachedAsset {
    /// Forget the blocks and edges of an outdated original; returns the bytes they held. The caller removes the
    /// edges file (`BlockCache::drop_outdated`).
    fn drop_blocks(&mut self) -> u64 {
        let dropped = self.bytes;
        self.blocks.clear();
        self.edges = None;
        self.bytes = 0;
        self.generation += 1;
        self.rewrite = true;
        self.version = None;
        self.size_recorded = false;
        dropped
    }
}

/// How a fetched block is written.
struct WritePlan {
    /// Empty the Asset's files first: its cached blocks were dropped.
    rewrite: bool,
    /// Lines for the block map before the block's own: the version and the size, when the map lacks them.
    header: String,
}

/// Unpins an Asset when its read is over, however the read ends.
struct Pin<'cache, Source> {
    cache: &'cache BlockCache<Source>,
    asset_id: String,
}

impl<Source> Drop for Pin<'_, Source> {
    fn drop(&mut self) {
        let mut state = self.cache.state.lock().expect("cache state lock");
        if let Some(entry) = state.entries.get_mut(&self.asset_id) {
            entry.pins = entry.pins.saturating_sub(1);
        }
    }
}

impl<Source: ContentSource> BlockCache<Source> {
    /// A cache keeping its files in `directory` (created if missing), holding at most about `limit_bytes`. What an
    /// earlier run cached is kept, as far as its block maps say; files without a map are removed.
    pub fn new(source: Source, directory: &Path, limit_bytes: u64) -> std::io::Result<Self> {
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(directory)?;
        let cache = BlockCache {
            source,
            directory: directory.to_path_buf(),
            limit_bytes,
            disk_room: None,
            disk_limit: Mutex::new(None),
            state: Mutex::new(CacheState::default()),
            in_flight: Mutex::new(HashMap::new()),
            writes: Mutex::new(HashMap::new()),
            read_ahead: Mutex::new(HashMap::new()),
            read_ahead_budget: Arc::new(tokio::sync::Semaphore::new(2)),
            edge_gates: Mutex::new(HashMap::new()),
            edge_prefetches: Arc::new(tokio::sync::Semaphore::new(PARALLEL_EDGE_PREFETCHES)),
            fetched_bytes: Default::default(),
            fetch_requests: Default::default(),
        };
        cache.load()?;
        Ok(cache)
    }

    /// Keep the disk's last tenth free: past it the cache evicts as if over its limit, however large `cache.limit`
    /// is (SPEC.md, "Content"). The probe is asked once per eviction check, after each block written.
    pub fn with_disk_floor(mut self, probe: DiskRoomProbe) -> Self {
        self.disk_room = Some(probe);
        self
    }

    /// The limit to keep to now: `limit_bytes`, or less when the disk is short of room. What the cache could hold
    /// is what it held when the disk was last asked, plus what the disk had above its last tenth. Asked at most every
    /// 30 seconds: freed space can take a filesystem commit to show (btrfs), and asking after every eviction would
    /// then evict ever more.
    fn effective_limit(&self, cached: u64) -> u64 {
        let Some(probe) = self.disk_room.as_ref() else {
            return self.limit_bytes;
        };
        let mut remembered = self.disk_limit.lock().expect("disk limit lock");
        if let Some((asked, limit)) = *remembered {
            if asked.elapsed() < DISK_PROBE_INTERVAL {
                return self.limit_bytes.min(limit);
            }
        }
        let Some(room) = probe() else {
            return self.limit_bytes;
        };
        let limit = (cached + room.available).saturating_sub(room.total / 10);
        *remembered = Some((std::time::Instant::now(), limit));
        self.limit_bytes.min(limit)
    }

    /// How many bytes of originals the cache holds.
    pub fn used_bytes(&self) -> u64 {
        self.state.lock().expect("cache state lock").bytes
    }

    /// The size limit the cache keeps to.
    pub fn limit_bytes(&self) -> u64 {
        self.limit_bytes
    }

    /// Empty the cache, except Assets being read right now.
    pub fn clear(&self) {
        let mut state = self.state.lock().expect("cache state lock");
        let idle: Vec<String> = state.entries.iter().filter(|(_, entry)| entry.pins == 0).map(|(asset_id, _)| asset_id.clone()).collect();
        for asset_id in idle {
            if let Some(entry) = state.entries.remove(&asset_id) {
                state.bytes -= entry.bytes;
            }
            for path in [self.map_path(&asset_id), self.data_path(&asset_id), self.edges_path(&asset_id)] {
                let _ = std::fs::remove_file(path);
            }
        }
    }

    /// Read up to `length` bytes at `offset` of an Asset whose original is `total_size` bytes. Short only at the end.
    pub async fn read(&self, asset_id: &str, total_size: u64, offset: u64, length: u32) -> Result<Vec<u8>, SourceError> {
        if offset >= total_size || length == 0 {
            return Ok(Vec::new());
        }
        let _pin = self.pin(asset_id);
        self.check_size(asset_id, total_size);
        let end = (offset + u64::from(length)).min(total_size);
        let first_block = offset / BLOCK_SIZE;
        let last_block = (end - 1) / BLOCK_SIZE;
        // A file within the limit costs no more whole than peeked, and whole it is cached.
        if total_size > PEEK_LIMIT
            && self.is_peek(asset_id, offset, end - offset)
            && !(first_block..=last_block).all(|block| self.is_present(asset_id, block))
        {
            if let Some(bytes) = self.read_from_edges(asset_id, total_size, offset, end).await? {
                return Ok(bytes);
            }
            let RangeBytes { bytes, .. } = self.source.read_range(asset_id, offset, end - 1).await?;
            self.count_fetch(bytes.len());
            if bytes.len() as u64 != end - offset {
                log::info!("asked for {} bytes of {asset_id} at {offset}, got {}; the original changed", end - offset, bytes.len());
                return Err(SourceError::Changed);
            }
            return Ok(bytes);
        }
        // A read that overlapped a new version of the original may hold blocks of both, or find the file emptied
        // under it: it reads again.
        for _ in 0..3 {
            let generation = self.generation(asset_id);
            for block in first_block..=last_block {
                self.ensure_block(asset_id, total_size, block).await?;
            }
            let path = self.data_path(asset_id);
            // File I/O runs off the async workers, so a slow disk never stalls other FUSE replies or their deadlines.
            let outcome = tokio::task::spawn_blocking(move || {
                let file = File::open(path).map_err(|error| SourceError::Protocol(format!("cache file: {error}")))?;
                let mut buffer = vec![0u8; (end - offset) as usize];
                file.read_exact_at(&mut buffer, offset).map_err(|error| SourceError::Protocol(format!("cache read: {error}")))?;
                Ok::<_, SourceError>(buffer)
            })
            .await
            .map_err(|error| SourceError::Protocol(format!("cache read: {error}")))?;
            if self.generation(asset_id) == generation {
                return outcome;
            }
            log::info!("the original of {asset_id} changed on the server during a read; reading again");
        }
        Err(SourceError::Protocol(format!("the original of {asset_id} kept changing on the server")))
    }

    /// How many times the Asset's cached blocks were dropped for a new version or size; a caller compares it before
    /// and after a read to learn that the original changed on the server.
    pub fn generation(&self, asset_id: &str) -> u64 {
        self.state.lock().expect("cache state lock").entries.get(asset_id).map_or(0, |entry| entry.generation)
    }

    /// Whether the kernel may keep what it cached of this Asset at an earlier open: not when its blocks were dropped
    /// since (a new version or size), so a program never reads old pages beside new bytes.
    pub fn keep_page_cache(&self, asset_id: &str) -> bool {
        let mut state = self.state.lock().expect("cache state lock");
        let entry = state.entries.entry(asset_id.to_string()).or_default();
        let keep = entry.opened_generation.is_none_or(|opened| opened == entry.generation);
        entry.opened_generation = Some(entry.generation);
        keep
    }

    /// Drop the cached blocks when the listing gives another size than they were fetched for: the original was
    /// replaced, and its old blocks must not be served. A size first seen is adopted.
    fn check_size(&self, asset_id: &str, total_size: u64) {
        let mut state = self.state.lock().expect("cache state lock");
        let CacheState { entries, bytes, .. } = &mut *state;
        let entry = entries.entry(asset_id.to_string()).or_default();
        match entry.size {
            Some(size) if size != total_size => {
                log::info!("the original of {asset_id} changed size on the server; its cached blocks are dropped");
                *bytes -= self.drop_outdated(asset_id, entry);
                entry.size = Some(total_size);
            }
            None => entry.size = Some(total_size),
            Some(_) => {}
        }
    }

    /// Bytes of originals fetched from the server and the requests that fetched them, since the cache was made: what
    /// browsing cost the network, which `immich-mount status` shows.
    pub fn fetched(&self) -> (u64, u64) {
        use std::sync::atomic::Ordering::Relaxed;
        (self.fetched_bytes.load(Relaxed), self.fetch_requests.load(Relaxed))
    }

    fn count_fetch(&self, bytes: usize) {
        use std::sync::atomic::Ordering::Relaxed;
        self.fetched_bytes.fetch_add(bytes as u64, Relaxed);
        self.fetch_requests.fetch_add(1, Relaxed);
    }

    /// Forget the Asset's run of reads, when it is opened: a run belongs to one open, so the reads of an earlier peek
    /// never add up with the next one's past `PEEK_LIMIT` (a photo shown in a month and an album, a folder looked at
    /// again once its pages left memory).
    pub fn forget_run(&self, asset_id: &str) {
        self.read_ahead.lock().expect("read-ahead lock").remove(asset_id);
    }

    /// Whether a read about to be made is a peek (`PEEK_LIMIT`), which the caller watches for a program peeking at
    /// a folder's files one after another (`prefetch_edges`).
    pub fn is_peek_read(&self, asset_id: &str, total_size: u64, offset: u64, length: u32) -> bool {
        total_size > PEEK_LIMIT && offset < total_size && self.is_peek(asset_id, offset, u64::from(length))
    }

    /// Fetch the edges of a folder's files in the background, in the folder's order and a few at a time: a program
    /// that peeks at every file of a folder from its main thread (rom-properties, in Bazzite's file managers) then
    /// finds the bytes on disk instead of waiting for the server once per file. Files whose edges, or first and last
    /// blocks, are cached already are skipped; a peek at a file not reached yet fetches its edges itself.
    pub fn prefetch_edges(self: &Arc<Self>, files: Vec<(String, u64)>)
    where
        Source: 'static,
    {
        let cache = self.clone();
        tokio::spawn(async move {
            let mut fetches = tokio::task::JoinSet::new();
            'files: for (asset_id, total_size) in files.into_iter().take(MOST_PREFETCHED) {
                if total_size <= PEEK_LIMIT
                    || cache.current_edges(&asset_id, total_size).is_some()
                    || (cache.is_present(&asset_id, 0) && cache.is_present(&asset_id, (total_size - 1) / BLOCK_SIZE))
                {
                    continue;
                }
                // Taken in order, so the files are fetched in the order the program reads them.
                let Ok(permit) = cache.edge_prefetches.clone().acquire_owned().await else { return };
                let cache = cache.clone();
                fetches.spawn(async move {
                    let _permit = permit;
                    let _pin = cache.pin(&asset_id);
                    cache.check_size(&asset_id, total_size);
                    match cache.ensure_edges(&asset_id, total_size).await {
                        Ok(()) | Err(SourceError::NotFound | SourceError::Changed) => true,
                        Err(error) => {
                            log::debug!("prefetching the edges of {asset_id} failed: {error}");
                            false
                        }
                    }
                });
                // The server stopped answering: the rest would fail too. The fetches under way finish.
                while let Some(outcome) = fetches.try_join_next() {
                    if !outcome.unwrap_or(false) {
                        break 'files;
                    }
                }
            }
            while fetches.join_next().await.is_some() {}
        });
    }

    /// The Asset's edges, if it has them for an original of `total_size` bytes that its blocks do not contradict.
    fn current_edges(&self, asset_id: &str, total_size: u64) -> Option<Edges> {
        let state = self.state.lock().expect("cache state lock");
        let entry = state.entries.get(asset_id)?;
        let edges = entry.edges.as_ref().filter(|edges| edges.size == total_size)?;
        let contradicted = entry.version.as_ref().zip(edges.version.as_ref()).is_some_and(|(blocks, edges)| blocks != edges);
        (!contradicted).then(|| edges.clone())
    }

    /// Bytes `offset..end` of a peek from the Asset's edges, which are fetched first if missing. `None` when the range
    /// is not within them, or they could not be read: the peek then fetches its own range.
    async fn read_from_edges(&self, asset_id: &str, total_size: u64, offset: u64, end: u64) -> Result<Option<Vec<u8>>, SourceError> {
        let (head, tail) = edge_lengths(total_size);
        let tail_start = total_size - tail;
        let start_in_edges = if end <= head {
            offset
        } else if tail > 0 && offset >= tail_start {
            head + offset - tail_start
        } else {
            return Ok(None);
        };
        self.ensure_edges(asset_id, total_size).await?;
        let Some(wanted) = self.current_edges(asset_id, total_size) else { return Ok(None) };
        let path = self.edges_path(asset_id);
        let read = tokio::task::spawn_blocking(move || read_edges_file(&path)).await.ok().flatten();
        match read {
            // Still the same edges once read: a change of the original during the read would have dropped them.
            Some((edges, payload)) if edges == wanted && self.current_edges(asset_id, total_size).as_ref() == Some(&wanted) => {
                Ok(Some(payload[start_in_edges as usize..(start_in_edges + end - offset) as usize].to_vec()))
            }
            _ => {
                log::debug!("the edges of {asset_id} could not be read; they will be fetched again");
                self.forget_edges(asset_id, &wanted);
                Ok(None)
            }
        }
    }

    /// Forget an Asset's edges when they are the ones given (torn on disk, say).
    fn forget_edges(&self, asset_id: &str, edges: &Edges) {
        let mut state = self.state.lock().expect("cache state lock");
        let CacheState { entries, bytes, .. } = &mut *state;
        if let Some(entry) = entries.get_mut(asset_id).filter(|entry| entry.edges.as_ref() == Some(edges)) {
            entry.edges = None;
            entry.bytes -= edges.length();
            *bytes -= edges.length();
            let _ = std::fs::remove_file(self.edges_path(asset_id));
        }
    }

    /// Make sure the Asset has edges for an original of `total_size` bytes, fetching them once however many peeks
    /// wait for them.
    async fn ensure_edges(&self, asset_id: &str, total_size: u64) -> Result<(), SourceError> {
        if self.current_edges(asset_id, total_size).is_some() {
            return Ok(());
        }
        let gate = self.edge_gates.lock().expect("edge gates lock").entry(asset_id.to_string()).or_default().clone();
        let _turn = gate.lock().await;
        if self.current_edges(asset_id, total_size).is_some() {
            return Ok(());
        }
        let fetched = self.fetch_edges(asset_id, total_size).await;
        // The gate's work is done, whatever the outcome; failed edges are asked for afresh.
        self.edge_gates.lock().expect("edge gates lock").remove(asset_id);
        fetched?;
        self.evict_over_limit();
        Ok(())
    }

    /// Fetch an original's start and end at once, and keep them as the Asset's edges file.
    async fn fetch_edges(&self, asset_id: &str, total_size: u64) -> Result<(), SourceError> {
        let (head_length, tail_length) = edge_lengths(total_size);
        let head = self.source.read_range(asset_id, 0, head_length - 1);
        let (head, tail) = if tail_length == 0 {
            (head.await?, None)
        } else {
            let (head, tail) = futures_util::future::join(head, self.source.read_range(asset_id, total_size - tail_length, total_size - 1)).await;
            (head?, Some(tail?))
        };
        self.count_fetch(head.bytes.len());
        if let Some(tail) = &tail {
            self.count_fetch(tail.bytes.len());
        }
        let tail_bytes = tail.as_ref().map_or(0, |tail| tail.bytes.len() as u64);
        if head.bytes.len() as u64 != head_length || tail_bytes != tail_length {
            log::info!("the edges of {asset_id} came back shorter than its listed size allows; the original changed");
            return Err(SourceError::Changed);
        }
        if tail.as_ref().is_some_and(|tail| tail.version != head.version) {
            log::info!("the original of {asset_id} changed on the server between its start and its end");
            return Err(SourceError::Changed);
        }
        let edges = Edges { size: total_size, version: head.version.clone() };
        let mut payload = head.bytes;
        if let Some(tail) = tail {
            payload.extend_from_slice(&tail.bytes);
        }
        let write_gate = self.writes.lock().expect("write gates lock").entry(asset_id.to_string()).or_default().clone();
        let writing = write_gate.lock().await;
        {
            let mut state = self.state.lock().expect("cache state lock");
            let CacheState { entries, bytes, .. } = &mut *state;
            let entry = entries.entry(asset_id.to_string()).or_default();
            // The listing gave another size meanwhile: these edges are of an original no longer listed.
            if entry.size.is_some_and(|size| size != total_size) {
                return Ok(());
            }
            // Newer than the cached blocks: those go, map and all, so a restart cannot bring them back.
            if entry.version.as_ref().zip(edges.version.as_ref()).is_some_and(|(blocks, fetched)| blocks != fetched) {
                log::info!("the original of {asset_id} changed on the server; its cached blocks are dropped");
                *bytes -= self.drop_outdated(asset_id, entry);
                let _ = std::fs::remove_file(self.map_path(asset_id));
                let _ = std::fs::remove_file(self.data_path(asset_id));
            }
        }
        let path = self.edges_path(asset_id);
        let header = edges.header(&payload);
        tokio::task::spawn_blocking(move || {
            let temporary = path.with_extension("edges.tmp");
            let mut file = OpenOptions::new().create(true).write(true).truncate(true).mode(0o600).open(&temporary)?;
            file.write_all(header.as_bytes())?;
            file.write_all(&payload)?;
            drop(file);
            std::fs::rename(&temporary, &path)
        })
        .await
        .map_err(|error| SourceError::Protocol(format!("cache write: {error}")))?
        .map_err(|error| SourceError::Protocol(format!("cache edges: {error}")))?;
        {
            let mut state = self.state.lock().expect("cache state lock");
            let CacheState { entries, bytes, .. } = &mut *state;
            let entry = entries.entry(asset_id.to_string()).or_default();
            if let Some(previous) = entry.edges.replace(edges.clone()) {
                entry.bytes -= previous.length();
                *bytes -= previous.length();
            }
            entry.size = Some(total_size);
            entry.bytes += edges.length();
            *bytes += edges.length();
        }
        drop(writing);
        drop(write_gate);
        self.writes.lock().expect("write gates lock").retain(|_, gate| Arc::strong_count(gate) > 1);
        Ok(())
    }

    /// Whether a read of `length` bytes at `offset` is a peek (`PEEK_LIMIT`): small, and in a run of reads that
    /// stays small with it. The run is the one `read_ahead` keeps, with the same slack for reads out of order.
    fn is_peek(&self, asset_id: &str, offset: u64, length: u64) -> bool {
        if length > PEEK_LIMIT {
            return false;
        }
        let runs = self.read_ahead.lock().expect("read-ahead lock");
        let Some(run) = runs.get(asset_id) else { return true };
        let continues = run.bytes > 0 && offset + BLOCK_SIZE >= run.next_offset && offset <= run.next_offset + BLOCK_SIZE;
        !continues || run.bytes + length <= PEEK_LIMIT
    }

    /// Note a finished read; if it continues a long sequential run (video playback, a copy), fetch the blocks after
    /// it in the background (ARCHITECTURE.md, "Content"). Random access ends the run.
    pub fn read_ahead(self: &Arc<Self>, asset_id: &str, total_size: u64, offset: u64, length: u64) -> usize
    where
        Source: 'static,
    {
        let end = (offset + length).min(total_size);
        let (window, scheduled_until) = {
            let mut runs = self.read_ahead.lock().expect("read-ahead lock");
            let run = runs.entry(asset_id.to_string()).or_default();
            // The kernel may issue a few reads out of order, so "sequential" allows a block of slack.
            let continues = run.bytes > 0 && offset + BLOCK_SIZE >= run.next_offset && offset <= run.next_offset + BLOCK_SIZE;
            if continues {
                run.bytes += length;
            } else {
                *run = SequentialRun { bytes: length, ..SequentialRun::default() };
            }
            run.next_offset = run.next_offset.max(end);
            if run.bytes >= READ_AHEAD_AFTER {
                run.window = (run.window * 2).clamp(READ_AHEAD_MIN, READ_AHEAD_MAX);
            }
            let previously_scheduled = run.scheduled_until;
            if run.window > 0 && end < total_size {
                let last_block = (end / BLOCK_SIZE + run.window - 1).min((total_size - 1) / BLOCK_SIZE);
                run.scheduled_until = run.scheduled_until.max(last_block + 1);
            }
            (run.window, previously_scheduled)
        };
        if window == 0 || end >= total_size {
            return 0;
        }
        let first_block = (end / BLOCK_SIZE).max(scheduled_until);
        let last_block = (end / BLOCK_SIZE + window - 1).min((total_size - 1) / BLOCK_SIZE);
        let wanted: Vec<u64> = (first_block..=last_block).filter(|block| !self.is_present(asset_id, *block)).collect();
        if wanted.is_empty() {
            return 0;
        }
        let count = wanted.len();
        let cache = self.clone();
        let asset_id = asset_id.to_string();
        tokio::spawn(async move {
            let Ok(_permit) = cache.read_ahead_budget.clone().acquire_owned().await else { return };
            let _pin = cache.pin(&asset_id);
            for block in wanted {
                if cache.ensure_block(&asset_id, total_size, block).await.is_err() {
                    break;
                }
            }
        });
        count
    }

    fn file_stem(asset_id: &str) -> String {
        asset_id.chars().filter(|character| character.is_ascii_alphanumeric() || *character == '-').collect()
    }

    fn data_path(&self, asset_id: &str) -> PathBuf {
        self.directory.join(format!("{}.bin", Self::file_stem(asset_id)))
    }

    fn map_path(&self, asset_id: &str) -> PathBuf {
        self.directory.join(format!("{}.blocks", Self::file_stem(asset_id)))
    }

    fn edges_path(&self, asset_id: &str) -> PathBuf {
        self.directory.join(format!("{}.edges", Self::file_stem(asset_id)))
    }

    /// Forget the blocks and edges of an outdated original (`CachedAsset::drop_blocks`), removing its edges file at
    /// once: unlike the blocks, which the next write empties, nothing else would replace it. Returns the bytes held.
    fn drop_outdated(&self, asset_id: &str, entry: &mut CachedAsset) -> u64 {
        if entry.edges.is_some() {
            let _ = std::fs::remove_file(self.edges_path(asset_id));
        }
        entry.drop_blocks()
    }

    /// Mark an Asset as being read, and as the most recently used.
    fn pin(&self, asset_id: &str) -> Pin<'_, Source> {
        let mut state = self.state.lock().expect("cache state lock");
        state.clock += 1;
        let now = state.clock;
        let entry = state.entries.entry(asset_id.to_string()).or_default();
        entry.pins += 1;
        entry.last_used = now;
        Pin { cache: self, asset_id: asset_id.to_string() }
    }

    fn is_present(&self, asset_id: &str, block: u64) -> bool {
        self.state.lock().expect("cache state lock").entries.get(asset_id).is_some_and(|entry| entry.blocks.contains(&block))
    }

    async fn ensure_block(&self, asset_id: &str, total_size: u64, block: u64) -> Result<(), SourceError> {
        if self.is_present(asset_id, block) {
            return Ok(());
        }
        let gate = {
            let mut in_flight = self.in_flight.lock().expect("in-flight lock");
            in_flight.entry((asset_id.to_string(), block)).or_default().clone()
        };
        let _turn = gate.lock().await;
        if self.is_present(asset_id, block) {
            return Ok(());
        }
        let start = block * BLOCK_SIZE;
        let length = (start + BLOCK_SIZE).min(total_size) - start;
        let fetched = self.fetch_block(asset_id, total_size, block, start, length).await;
        // The gate's work is done, whatever the outcome; a failed block is asked for afresh.
        self.in_flight.lock().expect("in-flight lock").remove(&(asset_id.to_string(), block));
        fetched?;
        self.evict_over_limit();
        Ok(())
    }

    async fn fetch_block(&self, asset_id: &str, total_size: u64, block: u64, start: u64, length: u64) -> Result<(), SourceError> {
        let RangeBytes { bytes, version } = self.source.read_range(asset_id, start, start + length - 1).await?;
        self.count_fetch(bytes.len());
        if bytes.len() as u64 != length {
            // Fewer bytes than the listed size allows: the original is shorter now.
            log::info!("asked for {length} bytes of {asset_id} at {start}, got {}; the original changed", bytes.len());
            return Err(SourceError::Changed);
        }
        let write_gate = self.writes.lock().expect("write gates lock").entry(asset_id.to_string()).or_default().clone();
        let writing = write_gate.lock().await;
        let plan = self.plan_write(asset_id, version, total_size);
        let (data_path, map_path) = (self.data_path(asset_id), self.map_path(asset_id));
        tokio::task::spawn_blocking(move || {
            let cache_error = |what: &str, error: std::io::Error| SourceError::Protocol(format!("cache {what}: {error}"));
            let block_line = format!("{block} {length}\n");
            if plan.rewrite {
                // The map is emptied and synced before the data file, so a crash between the two leaves a map that
                // lists no block, never old blocks over an emptied file.
                let mut map =
                    OpenOptions::new().create(true).write(true).truncate(true).mode(0o600).open(&map_path).map_err(|error| cache_error("map", error))?;
                map.write_all(plan.header.as_bytes()).map_err(|error| cache_error("map", error))?;
                map.sync_all().map_err(|error| cache_error("map", error))?;
            }
            let file = OpenOptions::new()
                .create(true)
                .truncate(plan.rewrite)
                .write(true)
                .mode(0o600)
                .open(data_path)
                .map_err(|error| cache_error("file", error))?;
            file.write_all_at(&bytes, start).map_err(|error| cache_error("write", error))?;
            file.sync_data().map_err(|error| cache_error("sync", error))?;
            // One write per line: appends from concurrent blocks never interleave, and a crash cannot tear a line.
            let mut map = OpenOptions::new().create(true).append(true).mode(0o600).open(map_path).map_err(|error| cache_error("map", error))?;
            let line = if plan.rewrite { block_line } else { format!("{}{block_line}", plan.header) };
            map.write_all(line.as_bytes()).map_err(|error| cache_error("map", error))
        })
        .await
        .map_err(|error| SourceError::Protocol(format!("cache write: {error}")))??;
        {
            let mut state = self.state.lock().expect("cache state lock");
            let CacheState { entries, bytes: total, .. } = &mut *state;
            let entry = entries.entry(asset_id.to_string()).or_default();
            if entry.blocks.insert(block) {
                entry.bytes += length;
                *total += length;
            }
        }
        drop(writing);
        drop(write_gate);
        // Gates nobody holds go, so the map does not grow with every Asset ever read.
        self.writes.lock().expect("write gates lock").retain(|_, gate| Arc::strong_count(gate) > 1);
        Ok(())
    }

    /// Decide how a fetched block is written. A block of another version than the cached ones means the original was
    /// replaced: they are dropped from the state at once (and their generation counted), before the files change, so
    /// no read takes them for current. Blocks cached before versions were recorded adopt the first one seen.
    fn plan_write(&self, asset_id: &str, version: Option<String>, total_size: u64) -> WritePlan {
        let mut state = self.state.lock().expect("cache state lock");
        let CacheState { entries, bytes, .. } = &mut *state;
        let entry = entries.entry(asset_id.to_string()).or_default();
        // The blocks, or the edges when no block has a version yet: either way the file must not mix the two.
        let known = entry.version.as_ref().or_else(|| entry.edges.as_ref().and_then(|edges| edges.version.as_ref()));
        if known.zip(version.as_ref()).is_some_and(|(known, fetched)| known != fetched) {
            log::info!("the original of {asset_id} changed on the server; its cached blocks are dropped");
            *bytes -= self.drop_outdated(asset_id, entry);
        }
        let rewrite = std::mem::take(&mut entry.rewrite);
        let mut header = String::new();
        if let (None, Some(fetched)) = (&entry.version, version) {
            header.push_str(&format!("version {fetched}\n"));
            entry.version = Some(fetched);
        }
        if !entry.size_recorded {
            header.push_str(&format!("size {total_size}\n"));
            entry.size = Some(total_size);
            entry.size_recorded = true;
        }
        WritePlan { rewrite, header }
    }

    /// Remove whole Assets, least recently used first, until the cache is within its limit. Pinned Assets stay, so
    /// the cache can briefly exceed the limit while more than it can hold is being read.
    fn evict_over_limit(&self) {
        let cached = self.state.lock().expect("cache state lock").bytes;
        let limit = self.effective_limit(cached);
        let mut state = self.state.lock().expect("cache state lock");
        while state.bytes > limit {
            let victim = state
                .entries
                .iter()
                .filter(|(_, entry)| entry.pins == 0 && entry.bytes > 0)
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(asset_id, _)| asset_id.clone());
            let Some(asset_id) = victim else {
                break;
            };
            if let Some(entry) = state.entries.remove(&asset_id) {
                state.bytes -= entry.bytes;
            }
            for path in [self.map_path(&asset_id), self.data_path(&asset_id), self.edges_path(&asset_id)] {
                if let Err(error) = std::fs::remove_file(&path) {
                    if error.kind() != std::io::ErrorKind::NotFound {
                        log::warn!("could not evict {}: {error}", path.display());
                    }
                }
            }
        }
    }

    /// Rebuild the state from the block maps and edges files, oldest first by modification time, then evict down to
    /// the limit.
    fn load(&self) -> std::io::Result<()> {
        let mut found: HashMap<String, (std::time::SystemTime, CachedAsset)> = HashMap::new();
        let mut edges_found: Vec<(std::time::SystemTime, String, Edges)> = Vec::new();
        for item in std::fs::read_dir(&self.directory)? {
            let path = item?.path();
            let (Some(stem), Some(extension)) = (path.file_stem().and_then(|stem| stem.to_str()), path.extension()) else {
                continue;
            };
            let asset_id = stem.to_string();
            // An edges file a crash left half written.
            if extension == "tmp" {
                let _ = std::fs::remove_file(&path);
                continue;
            }
            if extension == "edges" {
                match edges_file_record(&path) {
                    Some(edges) => {
                        let modified = std::fs::metadata(&path)?.modified().unwrap_or(std::time::UNIX_EPOCH);
                        edges_found.push((modified, asset_id, edges));
                    }
                    None => {
                        let _ = std::fs::remove_file(&path);
                    }
                }
                continue;
            }
            if extension == "bin" && !self.map_path(&asset_id).exists() {
                let _ = std::fs::remove_file(&path);
                continue;
            }
            if extension != "blocks" {
                continue;
            }
            let Ok(data_length) = std::fs::metadata(self.data_path(&asset_id)).map(|metadata| metadata.len()) else {
                let _ = std::fs::remove_file(&path);
                continue;
            };
            let mut entry = CachedAsset::default();
            for line in std::fs::read_to_string(&path)?.lines() {
                if let Some(version) = line.strip_prefix("version ") {
                    entry.version = Some(version.to_owned());
                    continue;
                }
                if let Some(size) = line.strip_prefix("size ").and_then(|size| size.trim().parse().ok()) {
                    entry.size = Some(size);
                    entry.size_recorded = true;
                    continue;
                }
                let mut fields = line.split_whitespace().map(str::parse::<u64>);
                if let (Some(Ok(block)), Some(Ok(length)), None) = (fields.next(), fields.next(), fields.next()) {
                    // Only a block the data file can actually hold counts.
                    let fits = length > 0 && length <= BLOCK_SIZE && block.checked_mul(BLOCK_SIZE).and_then(|start| start.checked_add(length)).is_some_and(|end| end <= data_length);
                    if fits && entry.blocks.insert(block) {
                        entry.bytes += length;
                    }
                }
            }
            let modified = std::fs::metadata(&path)?.modified().unwrap_or(std::time::UNIX_EPOCH);
            found.insert(asset_id, (modified, entry));
        }
        for (modified, asset_id, edges) in edges_found {
            let (used, entry) = found.entry(asset_id.clone()).or_insert_with(|| (modified, CachedAsset::default()));
            // Edges of another original than the blocks beside them are outdated, or the blocks are: the edges go,
            // since only the blocks are checked against the server again.
            let agrees = entry.size.is_none_or(|size| size == edges.size)
                && !entry.version.as_ref().zip(edges.version.as_ref()).is_some_and(|(blocks, edges)| blocks != edges);
            if !agrees {
                let _ = std::fs::remove_file(self.edges_path(&asset_id));
                continue;
            }
            *used = (*used).max(modified);
            entry.size = Some(edges.size);
            entry.bytes += edges.length();
            entry.edges = Some(edges);
        }
        let mut found: Vec<(std::time::SystemTime, String, CachedAsset)> =
            found.into_iter().map(|(asset_id, (modified, entry))| (modified, asset_id, entry)).collect();
        found.sort_by_key(|(modified, _, _)| *modified);
        {
            let mut state = self.state.lock().expect("cache state lock");
            for (_, asset_id, mut entry) in found {
                state.clock += 1;
                entry.last_used = state.clock;
                state.bytes += entry.bytes;
                state.entries.insert(asset_id, entry);
            }
        }
        self.evict_over_limit();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountingSource {
        bytes: Vec<u8>,
        requests: AtomicUsize,
    }

    impl ContentSource for CountingSource {
        async fn read_range(&self, _asset_id: &str, start: u64, end_inclusive: u64) -> Result<RangeBytes, SourceError> {
            self.requests.fetch_add(1, Ordering::SeqCst);
            Ok(RangeBytes { bytes: self.bytes[start as usize..=end_inclusive as usize].to_vec(), version: None })
        }
    }

    /// An original replaced on the server once `replaced` is set: other bytes, another ETag.
    /// A read long enough not to be a peek, so it fetches and caches whole blocks.
    const BLOCK_READ: u32 = 128 * 1024;

    struct ReplacedSource {
        replaced: std::sync::atomic::AtomicBool,
    }

    impl ContentSource for ReplacedSource {
        async fn read_range(&self, _asset_id: &str, start: u64, end_inclusive: u64) -> Result<RangeBytes, SourceError> {
            let (fill, version) = if self.replaced.load(Ordering::SeqCst) { (2u8, "W/\"new\"") } else { (1u8, "W/\"old\"") };
            Ok(RangeBytes { bytes: vec![fill; (end_inclusive - start + 1) as usize], version: Some(version.to_owned()) })
        }
    }

    #[tokio::test]
    async fn a_new_size_in_the_listing_drops_even_fully_cached_blocks() {
        let directory = tempfile::tempdir().unwrap();
        let cache = BlockCache::new(ReplacedSource { replaced: std::sync::atomic::AtomicBool::new(false) }, directory.path(), u64::MAX).unwrap();
        assert_eq!(cache.read("asset", BLOCK_SIZE + 10, 0, BLOCK_READ).await.unwrap()[..10], [1u8; 10]);
        cache.source.replaced.store(true, Ordering::SeqCst);
        // Block 0 is cached, but the listing now gives another size: it is fetched again, from the new original.
        assert_eq!(cache.read("asset", BLOCK_SIZE * 2, 0, BLOCK_READ).await.unwrap()[..10], [2u8; 10]);
        assert_eq!(cache.generation("asset"), 1);
        let map = std::fs::read_to_string(directory.path().join("asset.blocks")).unwrap();
        assert!(map.contains(&format!("size {}\n", BLOCK_SIZE * 2)) && !map.contains("old"), "{map}");
        assert!(cache.keep_page_cache("asset"), "a first open keeps whatever the kernel holds");
        cache.source.replaced.store(false, Ordering::SeqCst);
        assert_eq!(cache.read("asset", BLOCK_SIZE + 10, 0, BLOCK_READ).await.unwrap()[..10], [1u8; 10]);
        assert!(!cache.keep_page_cache("asset"), "the original changed since the last open: the kernel's pages go");
        assert!(cache.keep_page_cache("asset"), "and nothing changed since this one");
    }

    #[tokio::test]
    async fn a_replaced_original_never_mixes_old_and_new_blocks() {
        let directory = tempfile::tempdir().unwrap();
        let size = BLOCK_SIZE * 2;
        let cache = BlockCache::new(ReplacedSource { replaced: std::sync::atomic::AtomicBool::new(false) }, directory.path(), u64::MAX).unwrap();
        assert_eq!(cache.read("asset", size, 0, BLOCK_READ).await.unwrap()[..10], [1u8; 10]);
        cache.source.replaced.store(true, Ordering::SeqCst);
        // The second block comes from the new version, so the first one is dropped and fetched again.
        assert_eq!(cache.read("asset", size, BLOCK_SIZE, BLOCK_READ).await.unwrap()[..10], [2u8; 10]);
        assert_eq!(cache.read("asset", size, 0, BLOCK_READ).await.unwrap()[..10], [2u8; 10]);
        assert_eq!(cache.used_bytes(), size);
        let map = std::fs::read_to_string(directory.path().join("asset.blocks")).unwrap();
        assert!(map.starts_with("version W/\"new\"\n") && !map.contains("old"), "{map}");
        drop(cache);
        let restarted = BlockCache::new(ReplacedSource { replaced: std::sync::atomic::AtomicBool::new(true) }, directory.path(), u64::MAX).unwrap();
        assert_eq!(restarted.generation("asset"), 0);
        assert_eq!(restarted.used_bytes(), size, "the version and both blocks survive a restart");
    }

    #[tokio::test]
    async fn a_block_of_a_new_version_drops_the_edges_of_the_old() {
        let directory = tempfile::tempdir().unwrap();
        let size = BLOCK_SIZE * 2;
        let cache = BlockCache::new(ReplacedSource { replaced: std::sync::atomic::AtomicBool::new(false) }, directory.path(), u64::MAX).unwrap();
        assert_eq!(cache.read("asset", size, 0, 4096).await.unwrap()[..10], [1u8; 10], "a peek keeps the old edges");
        cache.source.replaced.store(true, Ordering::SeqCst);
        // A copy's first block comes from the new original: the old edges must not stand beside it.
        assert_eq!(cache.read("asset", size, BLOCK_SIZE, BLOCK_READ).await.unwrap()[..10], [2u8; 10]);
        assert_eq!(cache.generation("asset"), 1, "a change, so the kernel's pages of the old one go too");
        assert!(!directory.path().join("asset.edges").exists());
        cache.forget_run("asset");
        assert_eq!(cache.read("asset", size, 0, 4096).await.unwrap()[..10], [2u8; 10]);
    }

    fn original(length: usize) -> Vec<u8> {
        (0..length).map(|index| (index % 251) as u8).collect()
    }

    #[tokio::test]
    async fn reads_return_exact_bytes_across_block_boundaries() {
        let bytes = original((BLOCK_SIZE * 2 + 1000) as usize);
        let directory = tempfile::tempdir().unwrap();
        let cache = BlockCache::new(CountingSource { bytes: bytes.clone(), requests: AtomicUsize::new(0) }, directory.path(), u64::MAX).unwrap();
        let size = bytes.len() as u64;
        let offset = BLOCK_SIZE - 10;
        let read = cache.read("asset-1", size, offset, 100).await.unwrap();
        assert_eq!(read, bytes[offset as usize..offset as usize + 100]);
        let tail = cache.read("asset-1", size, size - 500, 4096).await.unwrap();
        assert_eq!(tail, bytes[bytes.len() - 500..]);
        assert!(cache.read("asset-1", size, size, 10).await.unwrap().is_empty());
    }

    /// rom-properties' reads of one file after the kernel's read-ahead: the start, the last page, the start's
    /// continuation.
    fn rom_properties_reads(size: u64) -> [(u64, u32); 3] {
        [(0, 16384), (size - 3000, 4096), (16384, 32768)]
    }

    #[tokio::test]
    async fn a_peek_is_served_from_the_edges_and_a_run_fetches_blocks() {
        let bytes = original((BLOCK_SIZE + 3000) as usize);
        let size = bytes.len() as u64;
        let directory = tempfile::tempdir().unwrap();
        let cache =
            Arc::new(BlockCache::new(CountingSource { bytes: bytes.clone(), requests: AtomicUsize::new(0) }, directory.path(), u64::MAX).unwrap());
        for (offset, length) in rom_properties_reads(size) {
            let read = cache.read("peeked", size, offset, length).await.unwrap();
            assert_eq!(read, bytes[offset as usize..(offset + u64::from(length)).min(size) as usize]);
            cache.read_ahead("peeked", size, offset, read.len() as u64);
        }
        assert_eq!(cache.source.requests.load(Ordering::SeqCst), 2, "the start and the end, fetched together once");
        assert_eq!(cache.used_bytes(), HEAD_BYTES + TAIL_BYTES, "kept as the edges, no block");
        assert_eq!(cache.fetched(), (HEAD_BYTES + TAIL_BYTES, 2), "the counter sees what the server sent");
        // Opened again, it is peeked again, from the edges.
        cache.forget_run("peeked");
        for (offset, length) in rom_properties_reads(size) {
            let read = cache.read("peeked", size, offset, length).await.unwrap();
            assert_eq!(read, bytes[offset as usize..(offset + u64::from(length)).min(size) as usize]);
            cache.read_ahead("peeked", size, offset, read.len() as u64);
        }
        assert_eq!(cache.source.requests.load(Ordering::SeqCst), 2);
        // A copy goes on from there: past 64 KiB of run, whole blocks.
        cache.read("peeked", size, 49152, 131072).await.unwrap();
        assert!(cache.used_bytes() >= BLOCK_SIZE);
        // Bytes the cache holds are read from it, peek or not.
        let before = cache.source.requests.load(Ordering::SeqCst);
        cache.read("peeked", size, 100, 512).await.unwrap();
        assert_eq!(cache.source.requests.load(Ordering::SeqCst), before);
    }

    #[tokio::test]
    async fn edges_survive_a_restart_and_a_torn_file_is_fetched_again() {
        let bytes = original((BLOCK_SIZE * 3) as usize);
        let size = bytes.len() as u64;
        let directory = tempfile::tempdir().unwrap();
        let source = || CountingSource { bytes: bytes.clone(), requests: AtomicUsize::new(0) };
        let cache = BlockCache::new(source(), directory.path(), u64::MAX).unwrap();
        cache.read("photo", size, 0, 16384).await.unwrap();
        drop(cache);
        let restarted = BlockCache::new(source(), directory.path(), u64::MAX).unwrap();
        assert_eq!(restarted.used_bytes(), HEAD_BYTES + TAIL_BYTES);
        for (offset, length) in rom_properties_reads(size) {
            let read = restarted.read("photo", size, offset, length).await.unwrap();
            assert_eq!(read, bytes[offset as usize..(offset + u64::from(length)).min(size) as usize]);
        }
        assert_eq!(restarted.source.requests.load(Ordering::SeqCst), 0, "served from the kept edges");
        // A torn file (no fsync is spent on them) fails its checksum, and is fetched again.
        let path = directory.path().join("photo.edges");
        let mut torn = std::fs::read(&path).unwrap();
        let last = torn.len() - 1;
        torn[last] ^= 0xff;
        std::fs::write(&path, torn).unwrap();
        let read = restarted.read("photo", size, size - 100, 100).await.unwrap();
        assert_eq!(read, bytes[bytes.len() - 100..]);
        assert_eq!(restarted.read("photo", size, size - 100, 100).await.unwrap(), bytes[bytes.len() - 100..]);
        assert_eq!(restarted.source.requests.load(Ordering::SeqCst), 3, "one plain peek, then the edges again");
        assert_eq!(restarted.used_bytes(), HEAD_BYTES + TAIL_BYTES);
    }

    #[tokio::test]
    async fn a_new_size_drops_the_edges_and_eviction_takes_them() {
        let directory = tempfile::tempdir().unwrap();
        let bytes = original((BLOCK_SIZE * 2) as usize);
        let cache = BlockCache::new(CountingSource { bytes, requests: AtomicUsize::new(0) }, directory.path(), u64::MAX).unwrap();
        cache.read("photo", BLOCK_SIZE, 0, 4096).await.unwrap();
        assert!(directory.path().join("photo.edges").exists());
        // The listing gives another size: the original was replaced, and its edges go with its blocks.
        cache.read("photo", BLOCK_SIZE * 2, 0, BLOCK_READ).await.unwrap();
        assert!(!directory.path().join("photo.edges").exists());
        assert_eq!(cache.used_bytes(), BLOCK_SIZE);
        cache.read("other", BLOCK_SIZE * 2, 0, 4096).await.unwrap();
        cache.clear();
        assert_eq!(cache.used_bytes(), 0);
        assert!(!directory.path().join("other.edges").exists());
    }

    #[tokio::test]
    async fn a_folder_prefetch_fetches_each_files_edges_once() {
        let bytes = original((BLOCK_SIZE + 5000) as usize);
        let size = bytes.len() as u64;
        let directory = tempfile::tempdir().unwrap();
        let cache =
            Arc::new(BlockCache::new(CountingSource { bytes: bytes.clone(), requests: AtomicUsize::new(0) }, directory.path(), u64::MAX).unwrap());
        let files: Vec<(String, u64)> = (0..20).map(|index| (format!("photo-{index}"), size)).chain([("small".to_string(), 1000)]).collect();
        cache.prefetch_edges(files.clone());
        // Peeks while the prefetch runs wait for it, or fetch their own file's edges; either way once per file.
        for (asset_id, _) in files.iter().take(20) {
            for (offset, length) in rom_properties_reads(size) {
                let read = cache.read(asset_id, size, offset, length).await.unwrap();
                assert_eq!(read, bytes[offset as usize..(offset + u64::from(length)).min(size) as usize]);
            }
        }
        for _ in 0..100 {
            if cache.edge_prefetches.available_permits() == PARALLEL_EDGE_PREFETCHES {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert_eq!(cache.source.requests.load(Ordering::SeqCst), 40, "two requests per photo; the small file is left to its reads");
        cache.prefetch_edges(files);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(cache.source.requests.load(Ordering::SeqCst), 40, "a folder seen before costs nothing");
    }

    #[tokio::test]
    async fn each_block_is_fetched_once() {
        let bytes = original(4096);
        let directory = tempfile::tempdir().unwrap();
        let cache = BlockCache::new(CountingSource { bytes: bytes.clone(), requests: AtomicUsize::new(0) }, directory.path(), u64::MAX).unwrap();
        for offset in [0, 100, 2000] {
            cache.read("asset-2", 4096, offset, 512).await.unwrap();
        }
        assert_eq!(cache.source.requests.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn reading_the_header_of_a_large_file_fetches_only_its_edges() {
        let bytes = original((BLOCK_SIZE * 5) as usize);
        let directory = tempfile::tempdir().unwrap();
        let cache = BlockCache::new(CountingSource { bytes, requests: AtomicUsize::new(0) }, directory.path(), u64::MAX).unwrap();
        cache.read("video", BLOCK_SIZE * 5, 0, 65536).await.unwrap();
        assert_eq!(cache.fetched(), (HEAD_BYTES + TAIL_BYTES, 2));
    }

    #[tokio::test]
    async fn the_block_map_survives_a_restart() {
        let bytes = original(4096);
        let directory = tempfile::tempdir().unwrap();
        let source = || CountingSource { bytes: bytes.clone(), requests: AtomicUsize::new(0) };
        BlockCache::new(source(), directory.path(), u64::MAX).unwrap().read("asset-3", 4096, 0, 100).await.unwrap();
        let restarted = BlockCache::new(source(), directory.path(), u64::MAX).unwrap();
        assert_eq!(restarted.used_bytes(), 4096);
        assert_eq!(restarted.read("asset-3", 4096, 1000, 100).await.unwrap(), bytes[1000..1100]);
        assert_eq!(restarted.source.requests.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn past_the_limit_the_least_recently_used_asset_goes() {
        let bytes = original(1000);
        let directory = tempfile::tempdir().unwrap();
        let cache = BlockCache::new(CountingSource { bytes, requests: AtomicUsize::new(0) }, directory.path(), 2500).unwrap();
        for asset_id in ["old", "middle", "new"] {
            cache.read(asset_id, 1000, 0, 10).await.unwrap();
        }
        cache.read("old", 1000, 0, 10).await.unwrap(); // used again, so "middle" is now the oldest
        cache.read("fourth", 1000, 0, 10).await.unwrap();
        assert!(cache.used_bytes() <= 2500);
        assert!(!directory.path().join("middle.bin").exists() && !directory.path().join("middle.blocks").exists());
        assert!(directory.path().join("old.bin").exists() && directory.path().join("fourth.bin").exists());
    }

    #[tokio::test]
    async fn the_cache_never_takes_the_disks_last_tenth() {
        let directory = tempfile::tempdir().unwrap();
        let full_disk: DiskRoomProbe = Arc::new(|| Some(DiskRoom { available: 0, total: 10_000 }));
        let cache = BlockCache::new(CountingSource { bytes: original(600), requests: AtomicUsize::new(0) }, directory.path(), u64::MAX)
            .unwrap()
            .with_disk_floor(full_disk);
        cache.read("first", 600, 0, 10).await.unwrap();
        cache.read("second", 600, 0, 10).await.unwrap();
        assert!(!directory.path().join("first.bin").exists(), "with the disk at its last tenth, older Assets go");

        let directory = tempfile::tempdir().unwrap();
        let roomy_disk: DiskRoomProbe = Arc::new(|| Some(DiskRoom { available: 1_000_000, total: 10_000 }));
        let cache = BlockCache::new(CountingSource { bytes: original(600), requests: AtomicUsize::new(0) }, directory.path(), u64::MAX)
            .unwrap()
            .with_disk_floor(roomy_disk);
        cache.read("first", 600, 0, 10).await.unwrap();
        cache.read("second", 600, 0, 10).await.unwrap();
        assert!(directory.path().join("first.bin").exists(), "with room to spare, nothing goes");
    }

    #[tokio::test]
    async fn a_pinned_asset_is_never_evicted() {
        let bytes = original(1000);
        let directory = tempfile::tempdir().unwrap();
        let cache = BlockCache::new(CountingSource { bytes, requests: AtomicUsize::new(0) }, directory.path(), 500).unwrap();
        let pin = cache.pin("busy");
        cache.read("busy", 1000, 0, 10).await.unwrap();
        cache.read("other", 1000, 0, 10).await.unwrap();
        assert!(directory.path().join("busy.bin").exists(), "a pinned Asset stays even past the limit");
        drop(pin);
        cache.read("third", 1000, 0, 10).await.unwrap();
        assert!(!directory.path().join("busy.bin").exists());
    }

    #[test]
    fn files_without_a_block_map_are_removed_at_start() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("orphan.bin"), b"half written").unwrap();
        let cache = BlockCache::new(CountingSource { bytes: Vec::new(), requests: AtomicUsize::new(0) }, directory.path(), u64::MAX).unwrap();
        assert_eq!(cache.used_bytes(), 0);
        assert!(!directory.path().join("orphan.bin").exists());
    }

    #[test]
    fn a_map_line_the_data_file_cannot_hold_is_ignored() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("torn.bin"), vec![7u8; 1000]).unwrap();
        std::fs::write(directory.path().join("torn.blocks"), "0 1000\n12 2097152\n0 1000 extra\n").unwrap();
        let cache = BlockCache::new(CountingSource { bytes: Vec::new(), requests: AtomicUsize::new(0) }, directory.path(), u64::MAX).unwrap();
        assert_eq!(cache.used_bytes(), 1000, "block 12 lies past the end of the data, so it was never written");
    }

    #[tokio::test]
    async fn a_header_peek_reads_nothing_ahead_but_playback_does() {
        let size = BLOCK_SIZE * 20;
        let directory = tempfile::tempdir().unwrap();
        let cache = Arc::new(BlockCache::new(CountingSource { bytes: original(size as usize), requests: AtomicUsize::new(0) }, directory.path(), u64::MAX).unwrap());
        let chunk = 131_072u64;
        for index in 0..3 {
            cache.read("video", size, index * chunk, chunk as u32).await.unwrap();
            assert_eq!(cache.read_ahead("video", size, index * chunk, chunk), 0, "a peek at the header stays a peek");
        }
        let mut scheduled = 0;
        let mut offset = 3 * chunk;
        while offset < 3 * BLOCK_SIZE {
            cache.read("video", size, offset, chunk as u32).await.unwrap();
            scheduled += cache.read_ahead("video", size, offset, chunk);
            offset += chunk;
        }
        assert!(scheduled >= 2, "past 4 MiB of sequential reading, the next blocks are fetched ahead");
        cache.read_ahead("video", size, 17 * BLOCK_SIZE, chunk);
        assert_eq!(cache.read_ahead("video", size, 17 * BLOCK_SIZE + chunk, chunk), 0, "a jump starts a new run");
    }
}

