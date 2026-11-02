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

use crate::thumbnails::{file_uri, remove_thumbnails, render_png, thumbnail_path, write_thumbnail, Bucket};

/// How many small thumbnails are fetched at once. A listing waits for these.
const PARALLEL_SMALL_FETCHES: usize = 12;
/// How many previews are fetched at once, in the background. Their own budget, so they never hold up a listing.
const PARALLEL_PREVIEW_FETCHES: usize = 4;

/// The ledger of seeded URIs, in the account's cache directory.
const LEDGER_FILE: &str = "seeded-thumbnails";
/// Server thumbnails by Asset id, in the account's cache directory.
const SERVER_THUMBNAILS_DIRECTORY: &str = "server-thumbnails";

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

/// Writes seeds for one account.
pub struct Seeder<Thumbs> {
    source: Arc<Thumbs>,
    thumbnail_root: PathBuf,
    server_thumbnails: PathBuf,
    ledger_path: PathBuf,
    ledger_lock: Mutex<()>,
    sharp: bool,
    small_fetches: Semaphore,
    preview_fetches: Semaphore,
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
            small_fetches: Semaphore::new(PARALLEL_SMALL_FETCHES),
            preview_fetches: Semaphore::new(PARALLEL_PREVIEW_FETCHES),
        })
    }

    /// Seed `files`. The returned task finishes once every file has an `x-large` seed or has failed; dropping it
    /// does not stop the work. Sharp seeds are written afterwards, in a task of their own.
    pub fn seed(self: &Arc<Self>, files: Vec<SeedFile>) -> tokio::task::JoinHandle<()> {
        let seeder = self.clone();
        tokio::spawn(async move {
            let wanted = small_buckets().await;
            let missing = seeder.lacking(&files, &wanted);
            seeder.seed_all(&missing, ThumbnailKind::Small).await;
            if seeder.sharp {
                let seeder = seeder.clone();
                tokio::spawn(async move {
                    // Every file without a sharp seed, also those that had a small one already (sharp thumbnails
                    // switched on later, or a pass cut short by a restart).
                    let missing = seeder.lacking(&files, &[Bucket::XxLarge]);
                    seeder.seed_all(&missing, ThumbnailKind::Preview).await;
                });
            }
        })
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

    /// Each file with the buckets of `wanted` it has no current seed in; files with all of them are left out.
    fn lacking(&self, files: &[SeedFile], wanted: &[Bucket]) -> Vec<(SeedFile, Vec<Bucket>)> {
        files
            .iter()
            .filter_map(|file| {
                let buckets: Vec<Bucket> = wanted.iter().copied().filter(|bucket| !self.has_seed(file, *bucket)).collect();
                (!buckets.is_empty()).then(|| (file.clone(), buckets))
            })
            .collect()
    }

    /// Write each file's seed into the buckets it lacks, from one server thumbnail per Asset.
    async fn seed_all(self: &Arc<Self>, files: &[(SeedFile, Vec<Bucket>)], kind: ThumbnailKind) {
        // One fetch per Asset, however many of its Appearances the folder holds.
        let mut by_asset: HashMap<&str, Vec<(SeedFile, Vec<Bucket>)>> = HashMap::new();
        for (file, buckets) in files {
            by_asset.entry(file.asset_id.as_str()).or_default().push((file.clone(), buckets.clone()));
        }
        let mut tasks = tokio::task::JoinSet::new();
        for (asset_id, appearances) in by_asset {
            let seeder = self.clone();
            let asset_id = asset_id.to_owned();
            tasks.spawn(async move {
                let image = match seeder.server_thumbnail(&asset_id, kind).await {
                    Ok(image) => Arc::new(image),
                    Err(error) => return log::debug!("no seed for Asset {asset_id}: {error}"),
                };
                for (file, buckets) in appearances {
                    for bucket in &buckets {
                        if let Err(message) = seeder.write_seed(&file, image.clone(), *bucket).await {
                            log::debug!("no {} seed for {}: {message}", bucket.directory_name(), file.path.display());
                        }
                    }
                }
            });
        }
        while tasks.join_next().await.is_some() {}
    }

    async fn write_seed(&self, file: &SeedFile, image: Arc<Vec<u8>>, bucket: Bucket) -> Result<(), String> {
        let uri = file_uri(&file.path);
        let thumbnail_root = self.thumbnail_root.clone();
        let mtime = file.modified.timestamp();
        let rendered_uri = uri.clone();
        tokio::task::spawn_blocking(move || {
            let png = render_png(&image, bucket, &rendered_uri, mtime)?;
            write_thumbnail(&thumbnail_root, bucket, &rendered_uri, &png)
        })
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())?;
        self.record(&uri);
        Ok(())
    }

    /// The server's thumbnail of an Asset, from the account's cache when it is there.
    async fn server_thumbnail(&self, asset_id: &str, kind: ThumbnailKind) -> Result<Vec<u8>, SourceError> {
        let suffix = match kind {
            ThumbnailKind::Small => "small",
            ThumbnailKind::Preview => "preview",
        };
        let cached = self.server_thumbnails.join(format!("{}.{suffix}", sanitised_id(asset_id)));
        if let Ok(bytes) = tokio::fs::read(&cached).await {
            return Ok(bytes);
        }
        let budget = match kind {
            ThumbnailKind::Small => &self.small_fetches,
            ThumbnailKind::Preview => &self.preview_fetches,
        };
        let _permit = budget.acquire().await.map_err(|_| SourceError::Busy)?;
        let bytes = self.source.thumbnail(asset_id, kind).await?;
        let temporary = cached.with_extension(format!("{suffix}.tmp"));
        if tokio::fs::write(&temporary, &bytes).await.is_ok() {
            let _ = tokio::fs::rename(&temporary, &cached).await;
        }
        Ok(bytes)
    }

    fn record(&self, uri: &str) {
        let _guard = self.ledger_lock.lock().expect("ledger lock");
        let appended = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(&self.ledger_path)
            .and_then(|mut ledger| writeln!(ledger, "{uri}"));
        if let Err(error) = appended {
            log::warn!("could not record a seeded thumbnail in {}: {error}", self.ledger_path.display());
        }
    }
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

    /// Serves one generated JPEG for every Asset, and counts the requests.
    struct FakeThumbnails {
        jpeg: Vec<u8>,
        requests: AtomicUsize,
    }

    impl FakeThumbnails {
        fn new() -> FakeThumbnails {
            let image = image::RgbImage::from_pixel(375, 250, image::Rgb([200, 120, 40]));
            let mut jpeg = Vec::new();
            image::DynamicImage::ImageRgb8(image)
                .write_to(&mut std::io::Cursor::new(&mut jpeg), image::ImageFormat::Jpeg)
                .expect("encode a test JPEG");
            FakeThumbnails { jpeg, requests: AtomicUsize::new(0) }
        }
    }

    impl ThumbnailSource for FakeThumbnails {
        async fn thumbnail(&self, asset_id: &str, _kind: ThumbnailKind) -> Result<Vec<u8>, SourceError> {
            self.requests.fetch_add(1, Ordering::SeqCst);
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
