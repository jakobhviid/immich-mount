//! Seeding the freedesktop thumbnail cache, so the file manager never reads an original to draw a thumbnail
//! (ARCHITECTURE.md, "Seeding the freedesktop cache"; SPEC.md, "Thumbnails").
//!
//! - Every file in a listed folder gets an `x-large` seed from Immich's small thumbnail, before the listing answers
//!   (within a budget the caller enforces).
//! - With sharp thumbnails on, an `xx-large` seed from Immich's preview follows in the background.
//! - Server thumbnails are kept by Asset id, so another Appearance of the same Asset costs no network.
//! - Every seeded URI is appended to a ledger, so signing out can remove them all again.

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use immich_mount_core::source::{SourceError, ThumbnailKind, ThumbnailSource};
use tokio::sync::Semaphore;

use crate::thumbnails::{decode, encode_png, file_uri, remove_thumbnails, scale_for, thumbnail_path, write_thumbnail, Bucket};

/// How many small thumbnails are fetched at once for a folder being shown. Its listing waits for these.
const PARALLEL_SMALL_FETCHES: usize = 12;
/// How many previews are fetched at once, in the background. Their own budget, so they never hold up a listing.
const PARALLEL_PREVIEW_FETCHES: usize = 4;
/// How many previews are rendered at once. They come after the folder is shown, while the file manager draws it and
/// its plugins read it, so they never take more than this of the CPU.
const PARALLEL_PREVIEW_RENDERS: usize = 2;
/// How many small thumbnails are fetched, and rendered, at once for folders seeded ahead (`Seeder::seed_ahead`).
const PARALLEL_AHEAD_FETCHES: usize = 4;
const PARALLEL_AHEAD_RENDERS: usize = 1;

/// The ledger of seeded URIs, in the account's cache directory.
const LEDGER_FILE: &str = "seeded-thumbnails";
/// Server thumbnails by Asset id, in the account's cache directory.
const SERVER_THUMBNAILS_DIRECTORY: &str = "server-thumbnails";

/// The paths one Asset appears at in a folder being seeded, each with the buckets it lacks.
type Appearances = Vec<(SeedFile, Vec<Bucket>)>;

/// A file to seed: where it appears in the mount, which Asset it is, and its mtime.
#[derive(Debug, Clone)]
pub struct SeedFile {
    pub path: PathBuf,
    pub asset_id: String,
    pub modified: DateTime<Utc>,
}

/// Where the server's small thumbnail goes. GNOME's Files asks for `x-large` at its usual zoom levels; KDE's Dolphin
/// asks for `normal` and `large` at its smaller ones, and makes a thumbnail from the
/// original when the asked-for size is missing, so on KDE all three are written. Asked at each seeding, since the
/// daemon can start before the desktop does.
async fn small_buckets() -> Vec<Bucket> {
    if crate::kde::plasma_is_running().await {
        vec![Bucket::Normal, Bucket::Large, Bucket::XLarge]
    } else {
        vec![Bucket::XLarge]
    }
}

/// One kind of seeding work, with its own budgets, so a slower kind never holds up a faster one.
#[derive(Debug, Clone, Copy)]
enum Lane {
    /// Small seeds for a folder being shown, which its listing waits for.
    Shown,
    /// Small seeds for a folder a file manager is counting the items of, in the background.
    Ahead,
    /// Sharp seeds from previews, in the background.
    Sharp,
}

/// How many server thumbnails a lane fetches at once, and how many it renders.
struct Budget {
    fetches: Semaphore,
    renders: Semaphore,
}

/// Writes seeds for one account.
pub struct Seeder<Thumbs> {
    source: Arc<Thumbs>,
    thumbnail_root: PathBuf,
    server_thumbnails: PathBuf,
    ledger_path: PathBuf,
    ledger_lock: Mutex<()>,
    sharp: bool,
    shown: Budget,
    ahead: Budget,
    sharp_budget: Budget,
    /// One gate per server thumbnail being fetched, so lanes wanting the same one at once fetch it once.
    fetch_gates: Mutex<HashMap<PathBuf, Arc<tokio::sync::Mutex<()>>>>,
}

impl<Thumbs: ThumbnailSource + 'static> Seeder<Thumbs> {
    pub fn new(source: Arc<Thumbs>, thumbnail_root: &Path, account_cache: &Path, sharp: bool) -> std::io::Result<Seeder<Thumbs>> {
        let server_thumbnails = account_cache.join(SERVER_THUMBNAILS_DIRECTORY);
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(&server_thumbnails)?;
        Ok(Seeder {
            source,
            thumbnail_root: thumbnail_root.to_path_buf(),
            server_thumbnails,
            ledger_path: account_cache.join(LEDGER_FILE),
            ledger_lock: Mutex::new(()),
            sharp,
            // A folder being shown renders on every core: someone is waiting for it.
            shown: Budget {
                fetches: Semaphore::new(PARALLEL_SMALL_FETCHES),
                renders: Semaphore::new(std::thread::available_parallelism().map_or(2, usize::from)),
            },
            ahead: Budget { fetches: Semaphore::new(PARALLEL_AHEAD_FETCHES), renders: Semaphore::new(PARALLEL_AHEAD_RENDERS) },
            sharp_budget: Budget { fetches: Semaphore::new(PARALLEL_PREVIEW_FETCHES), renders: Semaphore::new(PARALLEL_PREVIEW_RENDERS) },
            fetch_gates: Mutex::new(HashMap::new()),
        })
    }

    fn budget(&self, lane: Lane) -> &Budget {
        match lane {
            Lane::Shown => &self.shown,
            Lane::Ahead => &self.ahead,
            Lane::Sharp => &self.sharp_budget,
        }
    }

    /// Seed `files`. The returned task finishes once every file has an `x-large` seed or has failed; dropping it
    /// does not stop the work. Sharp seeds are written afterwards, in a task of their own.
    pub fn seed(self: &Arc<Self>, files: Vec<SeedFile>) -> tokio::task::JoinHandle<()> {
        let seeder = self.clone();
        tokio::spawn(async move {
            let wanted = small_buckets().await;
            let (files, missing) = seeder.lacking(files, wanted).await;
            seeder.seed_all(&missing, Lane::Shown).await;
            if seeder.sharp {
                let seeder = seeder.clone();
                tokio::spawn(async move {
                    // Every file without a sharp seed, also those that had a small one already (sharp thumbnails
                    // switched on later, or a pass cut short by a restart).
                    let (_, missing) = seeder.lacking(files, vec![Bucket::XxLarge]).await;
                    seeder.seed_all(&missing, Lane::Sharp).await;
                });
            }
        })
    }

    /// Seed the small thumbnails of a folder a file manager is only counting the items of, in the background and
    /// behind the folders being shown, and fetch no previews for it: its visit, often the next click, then finds its
    /// small seeds waiting, and a folder nobody opens costs no preview.
    pub fn seed_ahead(self: &Arc<Self>, files: Vec<SeedFile>) {
        let seeder = self.clone();
        tokio::spawn(async move {
            let wanted = small_buckets().await;
            let (_, missing) = seeder.lacking(files, wanted).await;
            seeder.seed_all(&missing, Lane::Ahead).await;
        });
    }

    /// Whether a seed exists and still matches the file's mtime. A stale one (the date was edited in Immich) would
    /// make the file manager thumbnail the original instead, so it is written again.
    fn has_seed(&self, file: &SeedFile, bucket: Bucket) -> bool {
        use std::io::Read;
        let path = thumbnail_path(&self.thumbnail_root, bucket, &file_uri(&file.path));
        let Ok(seed) = std::fs::File::open(path) else { return false };
        let mut head = Vec::with_capacity(8192);
        if seed.take(8192).read_to_end(&mut head).is_err() {
            return false;
        }
        crate::thumbnails::thumbnail_text(&head, "Thumb::MTime").is_some_and(|mtime| mtime == file.modified.timestamp().to_string())
    }

    /// Each file with the buckets of `wanted` it has no current seed in; files with all of them are left out. The
    /// seeds are read off the async workers; `files` is handed back for a later pass.
    async fn lacking(self: &Arc<Self>, files: Vec<SeedFile>, wanted: Vec<Bucket>) -> (Vec<SeedFile>, Vec<(SeedFile, Vec<Bucket>)>) {
        let seeder = self.clone();
        tokio::task::spawn_blocking(move || {
            let missing = files
                .iter()
                .filter_map(|file| {
                    let buckets: Vec<Bucket> = wanted.iter().copied().filter(|bucket| !seeder.has_seed(file, *bucket)).collect();
                    (!buckets.is_empty()).then(|| (file.clone(), buckets))
                })
                .collect();
            (files, missing)
        })
        .await
        .unwrap_or_default()
    }

    /// Write each file's seed into the buckets it lacks, from one server thumbnail per Asset, in the files' order.
    async fn seed_all(self: &Arc<Self>, files: &[(SeedFile, Vec<Bucket>)], lane: Lane) {
        // One fetch per Asset, however many of its Appearances the folder holds.
        let mut by_asset: Vec<(String, Appearances)> = Vec::new();
        let mut position: HashMap<&str, usize> = HashMap::new();
        for (file, buckets) in files {
            let index = *position.entry(file.asset_id.as_str()).or_insert_with(|| {
                by_asset.push((file.asset_id.clone(), Vec::new()));
                by_asset.len() - 1
            });
            by_asset[index].1.push((file.clone(), buckets.clone()));
        }
        let mut tasks = tokio::task::JoinSet::new();
        for (asset_id, appearances) in by_asset {
            let seeder = self.clone();
            tasks.spawn(async move {
                let image = match seeder.server_thumbnail(&asset_id, lane).await {
                    Ok(image) => image,
                    Err(error) => return log::debug!("no seed for Asset {asset_id}: {error}"),
                };
                let Ok(_render) = seeder.budget(lane).renders.acquire().await else { return };
                let thumbnail_root = seeder.thumbnail_root.clone();
                match tokio::task::spawn_blocking(move || write_seeds(&thumbnail_root, &image, &appearances)).await {
                    Ok(written) => seeder.record(&written),
                    Err(error) => log::warn!("seeding Asset {asset_id} failed: {error}"),
                }
            });
        }
        while tasks.join_next().await.is_some() {}
    }

    /// The server's thumbnail of an Asset for `lane`, from the account's cache when it is there.
    async fn server_thumbnail(&self, asset_id: &str, lane: Lane) -> Result<Vec<u8>, SourceError> {
        let kind = match lane {
            Lane::Shown | Lane::Ahead => ThumbnailKind::Small,
            Lane::Sharp => ThumbnailKind::Preview,
        };
        let suffix = match kind {
            ThumbnailKind::Small => "small",
            ThumbnailKind::Preview => "preview",
        };
        let cached = self.server_thumbnails.join(format!("{}.{suffix}", sanitised_id(asset_id)));
        if let Ok(bytes) = tokio::fs::read(&cached).await {
            return Ok(bytes);
        }
        let _permit = self.budget(lane).fetches.acquire().await.map_err(|_| SourceError::Busy)?;
        // Taken after the permit, so a folder being shown never waits behind a lane's queue, only behind a fetch of
        // the same thumbnail already under way.
        let gate = self.fetch_gates.lock().expect("fetch gates lock").entry(cached.clone()).or_default().clone();
        let fetched = {
            let _turn = gate.lock().await;
            match tokio::fs::read(&cached).await {
                Ok(bytes) => Ok(bytes),
                Err(_) => match self.source.thumbnail(asset_id, kind).await {
                    Ok(bytes) => {
                        let temporary = cached.with_extension(format!("{suffix}.tmp"));
                        if tokio::fs::write(&temporary, &bytes).await.is_ok() {
                            let _ = tokio::fs::rename(&temporary, &cached).await;
                        }
                        Ok(bytes)
                    }
                    Err(error) => Err(error),
                },
            }
        };
        drop(gate);
        // Gates nobody holds go, so the map does not grow with every Asset ever seeded.
        self.fetch_gates.lock().expect("fetch gates lock").retain(|_, gate| Arc::strong_count(gate) > 1);
        fetched
    }

    /// Append seeded URIs to the ledger.
    fn record(&self, uris: &[String]) {
        if uris.is_empty() {
            return;
        }
        let lines: String = uris.iter().map(|uri| format!("{uri}\n")).collect();
        let _guard = self.ledger_lock.lock().expect("ledger lock");
        let appended = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(&self.ledger_path)
            .and_then(|mut ledger| ledger.write_all(lines.as_bytes()));
        if let Err(error) = appended {
            log::warn!("could not record a seeded thumbnail in {}: {error}", self.ledger_path.display());
        }
    }
}

/// Write one Asset's seeds: its server thumbnail decoded once, scaled once per bucket, and encoded for each path that
/// lacks that bucket. Returns the URIs written, for the ledger.
fn write_seeds(thumbnail_root: &Path, image_bytes: &[u8], appearances: &[(SeedFile, Vec<Bucket>)]) -> Vec<String> {
    let image = match decode(image_bytes) {
        Ok(image) => image,
        Err(error) => {
            log::debug!("no seed for Asset {}: {error}", appearances.first().map_or("", |(file, _)| file.asset_id.as_str()));
            return Vec::new();
        }
    };
    let mut written = Vec::new();
    for bucket in Bucket::ALL {
        if !appearances.iter().any(|(_, buckets)| buckets.contains(&bucket)) {
            continue;
        }
        let scaled = scale_for(&image, bucket);
        for (file, _) in appearances.iter().filter(|(_, buckets)| buckets.contains(&bucket)) {
            let uri = file_uri(&file.path);
            let outcome = encode_png(scaled.as_ref().unwrap_or(&image), &uri, file.modified.timestamp())
                .and_then(|png| write_thumbnail(thumbnail_root, bucket, &uri, &png));
            match outcome {
                Ok(_) => written.push(uri),
                Err(error) => log::debug!("no {} seed for {}: {error}", bucket.directory_name(), file.path.display()),
            }
        }
    }
    written
}

/// Asset ids are UUIDs; anything else is made safe as a file name.
fn sanitised_id(asset_id: &str) -> String {
    asset_id.chars().map(|character| if character.is_ascii_alphanumeric() || character == '-' { character } else { '_' }).collect()
}

/// Remove every thumbnail seeded for the account whose cache is `account_cache`, with any failure records for the
/// same URIs, then the ledger. Returns how many URIs still had a thumbnail to remove.
pub fn remove_seeds(account_cache: &Path, thumbnail_root: &Path) -> std::io::Result<usize> {
    let ledger_path = account_cache.join(LEDGER_FILE);
    let ledger = match std::fs::read_to_string(&ledger_path) {
        Ok(ledger) => ledger,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error),
    };
    let uris: HashSet<&str> = ledger.lines().filter(|line| line.starts_with("file://")).collect();
    let mut removed = 0;
    for uri in &uris {
        if Bucket::ALL.iter().any(|bucket| thumbnail_path(thumbnail_root, *bucket, uri).exists()) {
            removed += 1;
        }
        if let Err(error) = remove_thumbnails(thumbnail_root, uri) {
            log::warn!("could not remove the thumbnails of {uri}: {error}");
        }
    }
    std::fs::remove_file(&ledger_path)?;
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Serves one generated JPEG for every Asset, and counts the requests, and the previews among them.
    struct FakeThumbnails {
        jpeg: Vec<u8>,
        requests: AtomicUsize,
        previews: AtomicUsize,
    }

    impl FakeThumbnails {
        fn new() -> FakeThumbnails {
            let image = image::RgbImage::from_pixel(375, 250, image::Rgb([200, 120, 40]));
            let mut jpeg = Vec::new();
            image::DynamicImage::ImageRgb8(image)
                .write_to(&mut std::io::Cursor::new(&mut jpeg), image::ImageFormat::Jpeg)
                .expect("encode a test JPEG");
            FakeThumbnails { jpeg, requests: AtomicUsize::new(0), previews: AtomicUsize::new(0) }
        }
    }

    impl ThumbnailSource for FakeThumbnails {
        async fn thumbnail(&self, asset_id: &str, kind: ThumbnailKind) -> Result<Vec<u8>, SourceError> {
            self.requests.fetch_add(1, Ordering::SeqCst);
            if matches!(kind, ThumbnailKind::Preview) {
                self.previews.fetch_add(1, Ordering::SeqCst);
            }
            // Long enough for two lanes asking for one thumbnail to overlap.
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            if asset_id == "missing" {
                return Err(SourceError::NotFound);
            }
            Ok(self.jpeg.clone())
        }
    }

    fn file(path: &str, asset_id: &str) -> SeedFile {
        SeedFile { path: PathBuf::from(path), asset_id: asset_id.into(), modified: DateTime::from_timestamp(1_700_000_000, 0).unwrap() }
    }

    #[tokio::test]
    async fn a_seed_with_another_mtime_is_written_again() {
        let directory = tempfile::tempdir().unwrap();
        let thumbnail_root = directory.path().join("thumbnails");
        let seeder = Arc::new(Seeder::new(Arc::new(FakeThumbnails::new()), &thumbnail_root, &directory.path().join("account"), false).unwrap());
        let original = file("/home/me/Immich/Photos/2024/03 March/a.jpg", "asset-1");
        seeder.seed(vec![original.clone()]).await.unwrap();
        assert!(seeder.has_seed(&original, Bucket::XLarge));
        let edited = SeedFile { modified: DateTime::from_timestamp(1_800_000_000, 0).unwrap(), ..original };
        assert!(!seeder.has_seed(&edited, Bucket::XLarge), "the date changed in Immich");
        seeder.seed(vec![edited.clone()]).await.unwrap();
        assert!(seeder.has_seed(&edited, Bucket::XLarge));
    }

    #[tokio::test]
    async fn seeding_ahead_writes_small_seeds_only_and_shares_each_fetch() {
        let directory = tempfile::tempdir().unwrap();
        let thumbnail_root = directory.path().join("thumbnails");
        let source = Arc::new(FakeThumbnails::new());
        let seeder = Arc::new(Seeder::new(source.clone(), &thumbnail_root, &directory.path().join("account"), true).unwrap());
        let files: Vec<SeedFile> = (0..20).map(|index| file(&format!("/home/me/Immich/Photos/2024/03 March/{index}.jpg"), &format!("asset-{index}"))).collect();
        // A count seeds ahead while the folder is opened: each Asset is fetched once between them.
        seeder.seed_ahead(files.clone());
        seeder.seed(files.clone()).await.unwrap();
        for _ in 0..200 {
            if files.iter().all(|file| seeder.has_seed(file, Bucket::XLarge)) && source.previews.load(Ordering::SeqCst) == 20 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert_eq!(source.requests.load(Ordering::SeqCst) - source.previews.load(Ordering::SeqCst), 20, "one small fetch per Asset");

        // Ahead alone: small seeds, no previews.
        let ahead_only: Vec<SeedFile> = (0..5).map(|index| file(&format!("/home/me/Immich/Photos/2024/04 April/{index}.jpg"), &format!("april-{index}"))).collect();
        let previews_before = source.previews.load(Ordering::SeqCst);
        seeder.seed_ahead(ahead_only.clone());
        for _ in 0..200 {
            if ahead_only.iter().all(|file| seeder.has_seed(file, Bucket::XLarge)) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(ahead_only.iter().all(|file| seeder.has_seed(file, Bucket::XLarge)));
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(ahead_only.iter().all(|file| !seeder.has_seed(file, Bucket::XxLarge)), "no sharp seed for a folder only counted");
        assert_eq!(source.previews.load(Ordering::SeqCst), previews_before);
    }

    #[tokio::test]
    async fn seeds_are_written_recorded_reused_and_removed() {
        let directory = tempfile::tempdir().unwrap();
        let thumbnail_root = directory.path().join("thumbnails");
        let account_cache = directory.path().join("account");
        let source = Arc::new(FakeThumbnails::new());
        let seeder = Arc::new(Seeder::new(source.clone(), &thumbnail_root, &account_cache, false).unwrap());

        let files = vec![
            file("/home/me/Immich/Photos/2024/03 March/2024-03-15 14.32.07.jpg", "asset-1"),
            file("/home/me/Immich/Albums/2024 - Trip/2024-03-15 14.32.07.jpg", "asset-1"),
            file("/home/me/Immich/Photos/2024/03 March/gone.jpg", "missing"),
        ];
        seeder.seed(files.clone()).await.unwrap();

        let seeded = |file: &SeedFile| thumbnail_path(&thumbnail_root, Bucket::XLarge, &file_uri(&file.path)).exists();
        assert!(seeded(&files[0]) && seeded(&files[1]));
        assert!(!seeded(&files[2]));
        // Two Appearances of one Asset: one server fetch, plus the failed one.
        assert_eq!(source.requests.load(Ordering::SeqCst), 2);

        // A second listing finds the seeds and fetches nothing.
        seeder.seed(files.clone()).await.unwrap();
        assert_eq!(source.requests.load(Ordering::SeqCst), 3, "only the missing Asset is asked for again");

        assert_eq!(remove_seeds(&account_cache, &thumbnail_root).unwrap(), 2);
        assert!(!seeded(&files[0]) && !seeded(&files[1]));
        assert_eq!(remove_seeds(&account_cache, &thumbnail_root).unwrap(), 0);
    }
}
