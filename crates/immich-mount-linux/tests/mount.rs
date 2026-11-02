//! Mounts a small in-memory library for real and reads it through the kernel.
//!
//! Needs `/dev/fuse` and `fusermount3`, so it is ignored by default: run it with
//! `cargo test -p immich-mount-linux --test mount -- --ignored`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use chrono::{NaiveDate, TimeZone, Utc};
use immich_mount_core::content::{BlockCache, ContentSource, RangeBytes};
use immich_mount_core::model::{Album, Asset, Person, YearMonth};
use immich_mount_core::settings::Config;
use immich_mount_core::source::{LibrarySource, SourceError, ThumbnailKind, ThumbnailSource};
use immich_mount_core::uploads::{MediaTypes, UploadError, UploadTarget, Uploaded};
use immich_mount_core::tree::{InMemoryNames, Tree};
use immich_mount_linux::fuse::{mount, ImmichFilesystem};

struct Library {
    assets: Vec<Asset>,
    originals: HashMap<String, Vec<u8>>,
}

impl LibrarySource for Library {
    async fn timeline_months(&self) -> Result<Vec<(YearMonth, u32)>, SourceError> {
        Ok(vec![(YearMonth { year: 2024, month: 3 }, self.assets.len() as u32)])
    }
    async fn month_assets(&self, _month: YearMonth) -> Result<Vec<Asset>, SourceError> {
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
        Ok(self.assets.clone())
    }
}

impl ContentSource for Library {
    async fn read_range(&self, asset_id: &str, start: u64, end_inclusive: u64) -> Result<RangeBytes, SourceError> {
        let bytes = self.originals.get(asset_id).ok_or(SourceError::NotFound)?;
        Ok(RangeBytes { bytes: bytes[start as usize..=end_inclusive as usize].to_vec(), version: None })
    }
}

/// No server thumbnails: this test is about the filesystem, and seeding is tested in `seeding.rs`.
impl ThumbnailSource for Library {
    async fn thumbnail(&self, _asset_id: &str, _kind: ThumbnailKind) -> Result<Vec<u8>, SourceError> {
        Err(SourceError::NotFound)
    }
}

/// Uploads are refused: this test mounts without them.
impl UploadTarget for Library {
    async fn media_types(&self) -> Result<MediaTypes, SourceError> {
        Ok(MediaTypes::default())
    }
    async fn upload(&self, _path: &std::path::Path, _file_name: &str, _modified: chrono::DateTime<Utc>) -> Result<Uploaded, UploadError> {
        Err(UploadError::Source(SourceError::Busy))
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
    assert_eq!(sorted_names(root), [".nomedia", ".trackerignore", ".xdg-volume-info", "Albums", "People", "Photos", "Places"]);
    assert_eq!(std::fs::read_to_string(root.join(".xdg-volume-info")).unwrap(), "[Volume Info]\nName=Immich\n");

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
