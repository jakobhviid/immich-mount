//! Mounts a small in-memory library for real and reads it through the kernel.
//!
//! Needs `/dev/fuse` and `fusermount3`, so it is ignored by default: run it with
//! `cargo test -p immich-mount-linux --test mount -- --ignored`, as CI does on every push.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use chrono::{NaiveDate, TimeZone, Utc};
use immich_mount_core::content::{BlockCache, ContentSource, RangeBytes};
use immich_mount_core::folder_uploads::{FolderChoice, FolderEvent, FolderUploads};
use immich_mount_core::model::{Album, Asset, Person, YearMonth};
use immich_mount_core::settings::Config;
use immich_mount_core::source::{LibrarySource, SourceError, ThumbnailKind, ThumbnailSource};
use immich_mount_core::uploads::{MediaTypes, UploadError, UploadTarget, Uploaded};
use immich_mount_core::tree::{InMemoryNames, Tree};
use immich_mount_linux::fuse::{mount, ImmichFilesystem};
use immich_mount_linux::staging::Staging;

struct Library {
    assets: Vec<Asset>,
    originals: HashMap<String, Vec<u8>>,
    /// Bytes of originals fetched, as a server would count them.
    fetched: std::sync::atomic::AtomicU64,
    /// A server that takes requests and never answers them (it drops packets): month and city listings hang.
    silent: std::sync::atomic::AtomicBool,
}

impl Library {
    async fn answer_or_hang(&self) {
        if self.silent.load(std::sync::atomic::Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_secs(3600)).await;
        }
    }
}

impl LibrarySource for Library {
    async fn timeline_months(&self) -> Result<Vec<(YearMonth, u32)>, SourceError> {
        Ok(vec![(YearMonth { year: 2024, month: 3 }, self.assets.len() as u32)])
    }
    async fn month_assets(&self, _month: YearMonth) -> Result<Vec<Asset>, SourceError> {
        self.answer_or_hang().await;
        Ok(self.assets.clone())
    }
    async fn albums(&self) -> Result<Vec<Album>, SourceError> {
        Ok(vec![Album { id: "album-1".into(), name: "Summer trip".into(), year: Some(2024), created_at: Utc::now() }])
    }
    async fn album_assets(&self, _album_id: &str) -> Result<Vec<Asset>, SourceError> {
        Ok(self.assets[..1].to_vec())
    }
    async fn people(&self) -> Result<Vec<Person>, SourceError> {
        Ok(vec![])
    }
    async fn person_assets(&self, _person_id: &str) -> Result<Vec<Asset>, SourceError> {
        Ok(vec![])
    }
    async fn countries(&self) -> Result<Vec<String>, SourceError> {
        Ok(vec!["Denmark".into()])
    }
    async fn cities(&self, _country: &str) -> Result<Vec<String>, SourceError> {
        Ok(vec!["Odense".into()])
    }
    async fn city_assets(&self, _country: &str, _city: &str) -> Result<Vec<Asset>, SourceError> {
        self.answer_or_hang().await;
        Ok(self.assets.clone())
    }
    async fn archive_assets(&self) -> Result<Vec<Asset>, SourceError> {
        Ok(vec![])
    }
    async fn favorite_assets(&self) -> Result<Vec<Asset>, SourceError> {
        Ok(vec![])
    }
}

impl ContentSource for Library {
    async fn read_range(&self, asset_id: &str, start: u64, end_inclusive: u64) -> Result<RangeBytes, SourceError> {
        let bytes = self.originals.get(asset_id).ok_or(SourceError::NotFound)?;
        self.fetched.fetch_add(end_inclusive - start + 1, std::sync::atomic::Ordering::SeqCst);
        Ok(RangeBytes { bytes: bytes[start as usize..=end_inclusive as usize].to_vec(), version: None })
    }
}

/// No server thumbnails: this test is about the filesystem, and seeding is tested in `seeding.rs`.
impl ThumbnailSource for Library {
    async fn thumbnail(&self, _asset_id: &str, _kind: ThumbnailKind) -> Result<Vec<u8>, SourceError> {
        Err(SourceError::NotFound)
    }
}

/// Every upload is a new Asset; most tests mount without uploads.
impl UploadTarget for Library {
    async fn media_types(&self) -> Result<MediaTypes, SourceError> {
        Ok(MediaTypes::default())
    }
    async fn upload(&self, _path: &std::path::Path, _file_name: &str, _modified: chrono::DateTime<Utc>) -> Result<Uploaded, UploadError> {
        Ok(Uploaded::Created("uploaded".into()))
    }
}

fn asset(id: &str, second: u32, length: usize) -> (Asset, Vec<u8>) {
    let local = NaiveDate::from_ymd_opt(2024, 3, 15).unwrap().and_hms_opt(14, 32, second).unwrap();
    let asset = Asset {
        id: id.into(),
        original_file_name: format!("IMG_{second}.jpg"),
        local_capture_time: local,
        captured_at: Utc.from_utc_datetime(&local),
        uploaded_at: Utc::now(),
        size: Some(length as u64),
        city: Some("Odense".into()),
        country: Some("Denmark".into()),
    };
    let bytes = (0..length).map(|index| ((index * 7 + second as usize) % 256) as u8).collect();
    (asset, bytes)
}

fn sorted_names(path: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(path).unwrap().map(|entry| entry.unwrap().file_name().into_string().unwrap()).collect();
    names.sort();
    names
}

#[test]
#[ignore = "needs /dev/fuse and fusermount3"]
fn the_mount_lists_and_reads_like_a_normal_folder() {
    let (small, small_bytes) = asset("asset-small", 7, 1000);
    let (large, large_bytes) = asset("asset-large", 9, 5 * 1024 * 1024 + 123);
    let library = Arc::new(Library {
        assets: vec![small, large],
        originals: HashMap::from([("asset-small".to_string(), small_bytes.clone()), ("asset-large".to_string(), large_bytes.clone())]),
        fetched: Default::default(),
        silent: Default::default(),
    });

    let runtime = tokio::runtime::Runtime::new().unwrap();
    let cache_directory = tempfile::tempdir().unwrap();
    let mountpoint = tempfile::tempdir().unwrap();
    let tree = Arc::new(Tree::new(library.clone(), InMemoryNames::default(), Config::from_toml("").unwrap()));
    let content = Arc::new(BlockCache::new(library.clone(), cache_directory.path(), u64::MAX).unwrap());
    let filesystem = ImmichFilesystem::new(tree, content, runtime.handle().clone());
    let session = mount(filesystem, mountpoint.path()).expect("mounts");
    std::thread::sleep(Duration::from_millis(200));

    let root = mountpoint.path();
    assert_eq!(sorted_names(root), [".directory", ".nomedia", ".trackerignore", ".xdg-volume-info", "Albums", "People", "Photos", "Places", "Upload"]);
    assert_eq!(std::fs::read_to_string(root.join(".xdg-volume-info")).unwrap(), "[Volume Info]\nName=Immich Mount\nIcon=dev.hviid.ImmichMount\n");
    assert_eq!(std::fs::read_to_string(root.join(".directory")).unwrap(), "[Desktop Entry]\nIcon=dev.hviid.ImmichMount\n", "Dolphin's folder icon");

    let month = root.join("Photos").join("2024").join("03 March");
    assert_eq!(sorted_names(&month), ["2024-03-15 14.32.07.jpg", "2024-03-15 14.32.09.jpg"]);

    let small_path = month.join("2024-03-15 14.32.07.jpg");
    let metadata = std::fs::metadata(&small_path).unwrap();
    assert_eq!(metadata.len(), 1000);
    let expected_mtime = SystemTime::from(Utc.with_ymd_and_hms(2024, 3, 15, 14, 32, 7).unwrap());
    assert_eq!(metadata.modified().unwrap(), expected_mtime);
    assert_eq!(std::fs::read(&small_path).unwrap(), small_bytes);
    assert_eq!(std::fs::read(month.join("2024-03-15 14.32.09.jpg")).unwrap(), large_bytes);

    assert_eq!(sorted_names(&root.join("Albums")), ["2024 - Summer trip"]);
    assert_eq!(sorted_names(&root.join("Albums").join("2024 - Summer trip")), ["2024-03-15 14.32.07.jpg"]);
    assert_eq!(sorted_names(&root.join("Places").join("Denmark").join("Odense")).len(), 2);

    assert!(std::fs::write(month.join("new.jpg"), b"x").is_err(), "the mount is read-only");
    drop(session);
}

/// A library of a small, a medium and a large photo, mounted: the mount point, and what must live as long as the mount.
struct Mounted {
    library: Arc<Library>,
    /// The folders copied into Upload, as the filesystem reports them.
    folders: Arc<std::sync::Mutex<FolderUploads>>,
    mountpoint: tempfile::TempDir,
    _cache: tempfile::TempDir,
    _runtime: tokio::runtime::Runtime,
    session: Option<fuser::BackgroundSession>,
}

impl Mounted {
    fn new() -> Mounted {
        Mounted::build(false)
    }

    /// Mounted with uploads, into a staging area of its own.
    fn with_uploads() -> Mounted {
        Mounted::build(true)
    }

    fn build(uploads: bool) -> Mounted {
        let (small, small_bytes) = asset("asset-small", 7, 1000);
        let (medium, medium_bytes) = asset("asset-medium", 8, 900_000);
        let (large, large_bytes) = asset("asset-large", 9, 5 * 1024 * 1024 + 123);
        let library = Arc::new(Library {
            assets: vec![small, medium, large],
            originals: HashMap::from([
                ("asset-small".to_string(), small_bytes),
                ("asset-medium".to_string(), medium_bytes),
                ("asset-large".to_string(), large_bytes),
            ]),
            fetched: Default::default(),
            silent: Default::default(),
        });
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let cache = tempfile::tempdir().unwrap();
        let mountpoint = tempfile::tempdir().unwrap();
        let tree = Arc::new(Tree::new(library.clone(), InMemoryNames::default(), Config::from_toml("").unwrap()));
        let content = Arc::new(BlockCache::new(library.clone(), cache.path(), u64::MAX).unwrap());
        let mut filesystem = ImmichFilesystem::new(tree, content, runtime.handle().clone());
        let folders = Arc::new(std::sync::Mutex::new(FolderUploads::default()));
        if uploads {
            let staging = Staging::new(&cache.path().join("staging"), &cache.path().join("not-uploaded")).unwrap();
            filesystem = filesystem.with_uploads(Arc::new(staging), MediaTypes::default(), Arc::new(library.clone()), folders.clone());
        }
        let session = mount(filesystem, mountpoint.path()).expect("mounts");
        std::thread::sleep(Duration::from_millis(200));
        Mounted { library, folders, mountpoint, _cache: cache, _runtime: runtime, session: Some(session) }
    }

    fn month(&self) -> std::path::PathBuf {
        self.mountpoint.path().join("Photos").join("2024").join("03 March")
    }
}

fn errno(outcome: std::io::Result<impl Sized>) -> Option<i32> {
    outcome.err().and_then(|error| error.raw_os_error())
}

fn access(path: &std::path::Path, mode: i32) -> bool {
    let path = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
    unsafe { libc::access(path.as_ptr(), mode) == 0 }
}

#[test]
#[ignore = "needs /dev/fuse and fusermount3"]
fn copies_are_ordinary_files_and_access_says_what_the_mount_allows() {
    use std::os::unix::fs::PermissionsExt;
    let mounted = Mounted::new();
    let root = mounted.mountpoint.path();
    let photo = mounted.month().join("2024-03-15 14.32.07.jpg");
    // The modes are what copies get; cp and GIO carry them over.
    assert_eq!(std::fs::metadata(&photo).unwrap().permissions().mode() & 0o777, 0o644);
    assert_eq!(std::fs::metadata(root.join("Albums")).unwrap().permissions().mode() & 0o777, 0o755);
    let elsewhere = tempfile::tempdir().unwrap();
    let copy = elsewhere.path().join("copy.jpg");
    std::fs::copy(&photo, &copy).unwrap();
    assert_eq!(std::fs::metadata(&copy).unwrap().permissions().mode() & 0o200, 0o200, "a copy is writable");
    // access() says what the mount allows, which file managers draw their read-only emblem from.
    assert!(access(&photo, libc::R_OK) && !access(&photo, libc::W_OK));
    assert!(access(&root.join("Albums"), libc::R_OK | libc::X_OK) && !access(&root.join("Albums"), libc::W_OK));
    assert!(!access(&mounted.month(), libc::W_OK), "without uploads, Photos takes nothing either");
}

#[test]
#[ignore = "needs /dev/fuse and fusermount3"]
fn a_peek_fetches_the_edges_once() {
    use std::os::unix::fs::FileExt;
    let mounted = Mounted::new();
    let photo = std::fs::File::open(mounted.month().join("2024-03-15 14.32.09.jpg")).unwrap();
    let size = photo.metadata().unwrap().len();
    // rom-properties' pattern: the first pages and the last one, which the kernel widens with its own read-ahead.
    let mut page = [0u8; 4096];
    photo.read_at(&mut page, 0).unwrap();
    photo.read_at(&mut page, 4096).unwrap();
    photo.read_at(&mut page, size - 4096).unwrap();
    let fetched = mounted.library.fetched.load(std::sync::atomic::Ordering::SeqCst);
    assert!(fetched < 256 * 1024, "a peek fetched {fetched} bytes of a {size}-byte original");
    // A photo under one block, looked at three times with its pages gone from memory in between (it is in a month
    // and an album, or the folder is opened again later): its start and end fetched once, never the original.
    drop(photo);
    let medium = mounted.month().join("2024-03-15 14.32.08.jpg");
    for _ in 0..3 {
        let photo = std::fs::File::open(&medium).unwrap();
        unsafe { libc::posix_fadvise(std::os::fd::AsRawFd::as_raw_fd(&photo), 0, 0, libc::POSIX_FADV_DONTNEED) };
        let size = photo.metadata().unwrap().len();
        photo.read_at(&mut page, 0).unwrap();
        photo.read_at(&mut page, size - 4096).unwrap();
    }
    let again = mounted.library.fetched.load(std::sync::atomic::Ordering::SeqCst) - fetched;
    assert_eq!(again, (64 + 16) * 1024, "three peeks at a 900 KB photo fetch its edges once");
}

#[test]
#[ignore = "needs /dev/fuse and fusermount3"]
fn a_mount_in_use_is_busy_until_detached_lazily() {
    let mut mounted = Mounted::new();
    let in_use = std::fs::File::open(mounted.month()).unwrap();
    let refused = immich_mount_linux::mounts::unmount(mounted.mountpoint.path(), false).unwrap_err();
    assert!(immich_mount_linux::mounts::is_busy(&refused), "{refused}");
    immich_mount_linux::mounts::unmount(mounted.mountpoint.path(), true).expect("a lazy unmount detaches it");
    drop(in_use);
    drop(mounted.session.take());
}

#[test]
#[ignore = "needs /dev/fuse and fusermount3"]
fn links_device_nodes_and_ioctls_are_refused_cleanly() {
    use std::os::fd::AsRawFd;
    let mounted = Mounted::new();
    let month = mounted.month();
    assert_eq!(errno(std::os::unix::fs::symlink("/", month.join("link"))), Some(libc::EROFS));
    assert_eq!(errno(std::fs::hard_link(month.join("2024-03-15 14.32.07.jpg"), month.join("hard"))), Some(libc::EROFS));
    assert_eq!(errno(std::fs::create_dir(month.join("folder"))), Some(libc::EROFS));
    // KIO asks for the filesystem's label; the answer is a plain "not supported", not a missing operation.
    let photo = std::fs::File::open(month.join("2024-03-15 14.32.07.jpg")).unwrap();
    let mut label = [0u8; 256];
    const FS_IOC_GETFSLABEL: libc::c_ulong = 0x8100_9431;
    let outcome = unsafe { libc::ioctl(photo.as_raw_fd(), FS_IOC_GETFSLABEL, label.as_mut_ptr()) };
    assert_eq!((outcome, std::io::Error::last_os_error().raw_os_error()), (-1, Some(libc::ENOTTY)));
}

#[test]
#[ignore = "needs /dev/fuse and fusermount3"]
fn a_silent_server_costs_a_probe_nothing_and_a_folder_one_wait() {
    let mounted = Mounted::new();
    let month = mounted.month();
    assert!(month.is_dir());
    mounted.library.silent.store(true, std::sync::atomic::Ordering::SeqCst);
    // rom-properties' probe for an extracted Wii U disc, in a folder whose listing will not come: absent at once.
    let started = std::time::Instant::now();
    assert_eq!(errno(std::fs::metadata(month.join("code").join("app.xml"))), Some(libc::ENOENT));
    assert!(started.elapsed() < Duration::from_secs(2), "the probe took {:?}", started.elapsed());
    // A real name waits the listing's deadline once, and fails.
    let started = std::time::Instant::now();
    assert!(std::fs::metadata(month.join("2024-03-15 14.32.07.jpg")).is_err());
    assert!(started.elapsed() >= Duration::from_secs(5), "the first wait was {:?}", started.elapsed());
    // Then the server counts as silent: another folder that is not at hand fails in a moment, not another 10 s.
    let started = std::time::Instant::now();
    let city = mounted.mountpoint.path().join("Places").join("Denmark").join("Odense");
    assert!(std::fs::read_dir(city).and_then(|entries| entries.collect::<Result<Vec<_>, _>>()).is_err());
    assert!(started.elapsed() < Duration::from_secs(3), "the second folder took {:?}", started.elapsed());
}

#[test]
#[ignore = "needs /dev/fuse and fusermount3"]
fn a_dead_mount_is_known_dead_though_its_attributes_are_cached() {
    let mut mounted = Mounted::new();
    let root = mounted.mountpoint.path().to_path_buf();
    assert!(std::fs::metadata(&root).is_ok(), "the root's attributes, now in the kernel's cache");
    assert_eq!(immich_mount_linux::mounts::mount_state(&root), immich_mount_linux::mounts::MountState::Mounted);
    // Abort the connection, as a crashed daemon leaves it; the mounting user owns its control files.
    let device = std::fs::metadata(&root).unwrap();
    let minor = std::os::unix::fs::MetadataExt::dev(&device) & 0xff;
    std::fs::write(format!("/sys/fs/fuse/connections/{minor}/abort"), "1").expect("aborting the connection");
    assert!(std::fs::metadata(&root).is_ok(), "stat still answers from the cache");
    assert_eq!(immich_mount_linux::mounts::mount_state(&root), immich_mount_linux::mounts::MountState::Stale);
    immich_mount_linux::mounts::unmount(&root, true).unwrap();
    drop(mounted.session.take());
}

/// A whole 1x1 PNG, so the upload's completeness check passes.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01,
    0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41,
    0x54, 0x78, 0x9C, 0x63, 0x00, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49,
    0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
];

#[test]
#[ignore = "needs /dev/fuse and fusermount3"]
fn only_upload_takes_new_files() {
    let mounted = Mounted::with_uploads();
    let upload = mounted.mountpoint.path().join("Upload");
    // The library's folders are views built from its metadata: nothing is copied into them.
    assert_eq!(errno(std::fs::write(mounted.month().join("new.png"), PNG)), Some(libc::EROFS));
    assert!(!access(&mounted.month(), libc::W_OK) && access(&upload, libc::W_OK));
    // Upload takes the file; its close returns once Immich has it, and what is left may go at once.
    std::fs::write(upload.join("new.png"), PNG).unwrap();
    assert_eq!(sorted_names(&upload), ["new.png"]);
    std::fs::remove_file(upload.join("new.png")).unwrap();
    assert!(sorted_names(&upload).is_empty());
    // Folders go into Upload only, and never hidden ones; links not at all.
    assert_eq!(errno(std::fs::create_dir(mounted.month().join("folder"))), Some(libc::EROFS));
    assert_eq!(errno(std::fs::create_dir(upload.join(".thumbnails"))), Some(libc::EPERM));
    assert_eq!(errno(std::os::unix::fs::symlink("new.png", upload.join("link"))), Some(libc::EPERM));
}

#[test]
#[ignore = "needs /dev/fuse and fusermount3"]
fn a_folder_copied_into_upload_uploads_everything_in_it() {
    let mounted = Mounted::with_uploads();
    let upload = mounted.mountpoint.path().join("Upload");
    let (trip, day) = (upload.join("Trip"), upload.join("Trip").join("Day 1"));
    std::fs::create_dir_all(&day).unwrap();
    assert_eq!(errno(std::fs::create_dir(&trip)), Some(libc::EEXIST));
    assert!(access(&day, libc::W_OK));
    std::fs::write(trip.join("one.png"), PNG).unwrap();
    std::fs::write(day.join("two.png"), PNG).unwrap();
    assert_eq!(errno(std::fs::write(day.join("notes.txt"), "no")), Some(libc::ENOTSUP));
    assert_eq!(sorted_names(&upload), ["Trip"]);
    assert_eq!(sorted_names(&trip), ["Day 1", "one.png"]);
    assert_eq!(sorted_names(&day), ["two.png"]);
    // A folder's times, as cp -a and GIO set them after a copy, are taken.
    let folder = std::fs::File::open(&day).unwrap();
    folder.set_modified(SystemTime::now()).unwrap();
    // Not empty: only an empty folder may go (a cancelled copy cleaning up).
    assert_eq!(errno(std::fs::remove_dir(&day)), Some(libc::ENOTEMPTY));
    std::fs::create_dir(trip.join("Empty")).unwrap();
    std::fs::remove_dir(trip.join("Empty")).unwrap();
    // Every upload was noted under its folder, and once the copy settles it is asked about.
    let later = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs() + 60;
    let events = mounted.folders.lock().unwrap().tick(later, FolderChoice::Ask, |_| false);
    let [FolderEvent::Ask(question)] = events.as_slice() else { panic!("one question: {events:?}") };
    assert_eq!((question.folder.as_str(), question.uploaded, question.not_uploaded), ("Trip", 2, 1));
    assert_eq!(question.albums, ["Trip", "Trip › Day 1"]);
}
