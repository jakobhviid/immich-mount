//! The production library and content source: the Immich client, mapped onto the core's model.
//!
//! Membership comes from Immich (ARCHITECTURE.md, "Membership comes from Immich"):
//! - a timeline month is the ids in its timeline bucket, which groups by local time. Search filters in UTC, so it
//!   only supplies the details (names, sizes, places), over the month widened by 14 hours on each side
//! - albums, people, places, favorites and the archive are searches on the server's own filters
//!
//! Hidden Assets (the video half of a Live Photo) and the locked folder never appear; archived Assets appear only in
//! albums, favorites and the archive, as in Immich.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Datelike, Duration, Utc};
use futures_util::StreamExt;
use immich_api::{ApiError, AssetQuery, Client, Fetched, ThumbnailSize, TimelineScope, Visibility};

use crate::content::{ContentSource, RangeBytes};
use crate::folder_uploads::AlbumTarget;
use crate::model::{Album, Asset, Person, YearMonth};
use crate::source::{LibraryChanges, LibrarySource, SourceError, ThumbnailKind, ThumbnailSource, Watched};
use crate::uploads::{MediaTypes, UploadError, UploadTarget, Uploaded};

/// How far a capture in a month can lie outside it in UTC: time zone offsets run from -12 to +14 hours.
const TIME_ZONE_MARGIN_HOURS: i64 = 14;

/// Every search asks for the server's largest page, so a month of a few thousand photos is a handful of requests.
const PAGE_SIZE: u32 = immich_api::search::MAX_PAGE_SIZE;

/// How many sizes are probed at once when the search did not report them.
const SIZE_PROBES: usize = 8;

/// How many members of a month the search may miss before the listing fails rather than fetch them one by one.
/// A miss is a race (an Asset changed between the two requests); many misses mean the search itself is wrong.
const MISSED_MEMBER_LIMIT: usize = 64;

/// Reads the library and originals from an Immich server.
pub struct ImmichSource {
    client: Client,
    poll: tokio::sync::Mutex<PollState>,
    /// Until when requests fail at once, because the server was just found unreachable.
    offline_until: std::sync::Mutex<Option<std::time::Instant>>,
}

/// After the server is found unreachable, requests fail at once for this long instead of each waiting on the network
/// (SPEC.md, "Content": an uncached read fails fast while offline). It outlasts a poll interval, so the change poll,
/// which ignores it, is what ends it: by succeeding.
const OFFLINE_GRACE: std::time::Duration = std::time::Duration::from_secs(45);
/// How long one change poll may take, sync stream included; past it the poll counts as offline. A half-open
/// connection after a suspend must not stop change tracking for good.
const POLL_LIMIT: std::time::Duration = std::time::Duration::from_secs(25);
/// How long one poll reads the sync stream, inside `POLL_LIMIT`. A session's first stream carries the whole library;
/// what is not read by then is read at the next poll, from the last ack, so a large library on a slow link is taken in
/// over several polls instead of counting as offline.
const SYNC_LIMIT: std::time::Duration = std::time::Duration::from_secs(15);
/// While the first stream is read (the baseline, whose records change nothing), it is acked every so many records, so
/// a stream cut short by `SYNC_LIMIT` goes on where it stopped.
const BASELINE_ACK_EVERY: usize = 5_000;

/// What the last change poll saw (ARCHITECTURE.md, "Freshness"): ETags, so an unchanged library costs two
/// 304s, and the values to compare when something did change.
#[derive(Default)]
struct PollState {
    started: bool,
    /// Whether a sync stream was read to its end in this run; until then, its records only set the baseline.
    sync_baseline_done: bool,
    buckets_etag: Option<String>,
    month_counts: HashMap<YearMonth, u32>,
    albums_etag: Option<String>,
    /// Per album: its name, Asset count and last update.
    albums: HashMap<String, (String, u64, DateTime<Utc>)>,
    /// The archive's buckets, while it is watched.
    archive: Option<WatchedBuckets>,
    /// The favorites' buckets, while they are watched.
    favorites: Option<WatchedBuckets>,
}

/// What the poll saw of a watched section's buckets: their ETag, and the months that hold Assets, with counts, in
/// order.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
struct WatchedBuckets {
    etag: Option<String>,
    month_counts: Vec<(YearMonth, u32)>,
}

/// What the change poll saw, in a form kept between runs (`ImmichSource::poll_memory`).
#[derive(serde::Serialize, serde::Deserialize)]
struct PollMemory {
    buckets_etag: Option<String>,
    month_counts: Vec<(YearMonth, u32)>,
    albums_etag: Option<String>,
    albums: Vec<(String, String, u64, DateTime<Utc>)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    archive: Option<WatchedBuckets>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    favorites: Option<WatchedBuckets>,
}

impl ImmichSource {
    pub fn new(client: Client) -> ImmichSource {
        ImmichSource { client, poll: tokio::sync::Mutex::new(PollState::default()), offline_until: std::sync::Mutex::new(None) }
    }

    pub fn client(&self) -> &Client {
        &self.client
    }

    fn check_online(&self) -> Result<(), SourceError> {
        match *self.offline_until.lock().expect("offline lock") {
            Some(until) if std::time::Instant::now() < until => Err(SourceError::Offline("the server was unreachable moments ago".to_owned())),
            _ => Ok(()),
        }
    }

    /// What the change poll has seen, as text to keep between runs, so a process started again (the macOS edition's
    /// extension, which the system stops when idle) sees what changed while it was stopped. `None` before the first
    /// poll. The sync stream's place is the server's (its acks); what a recalled poll adds is that the stream's next
    /// records are changes to report, not a baseline to take note of.
    pub async fn poll_memory(&self) -> Option<String> {
        let poll = self.poll.lock().await;
        if !poll.started {
            return None;
        }
        let memory = PollMemory {
            buckets_etag: poll.buckets_etag.clone(),
            month_counts: poll.month_counts.iter().map(|(month, count)| (*month, *count)).collect(),
            albums_etag: poll.albums_etag.clone(),
            albums: poll.albums.iter().map(|(id, (name, count, updated_at))| (id.clone(), name.clone(), *count, *updated_at)).collect(),
            archive: poll.archive.clone(),
            favorites: poll.favorites.clone(),
        };
        serde_json::to_string(&memory).ok()
    }

    /// Start from what an earlier run's poll saw (`poll_memory`): the next poll reports what changed since. Text that
    /// does not parse is ignored, and the next poll only takes note, as a first one does.
    pub async fn recall_poll(&self, memory: &str) {
        let Ok(memory) = serde_json::from_str::<PollMemory>(memory) else { return };
        let mut poll = self.poll.lock().await;
        poll.buckets_etag = memory.buckets_etag;
        poll.month_counts = memory.month_counts.into_iter().collect();
        poll.albums_etag = memory.albums_etag;
        poll.albums = memory.albums.into_iter().map(|(id, name, count, updated_at)| (id, (name, count, updated_at))).collect();
        poll.archive = memory.archive;
        poll.favorites = memory.favorites;
        poll.started = true;
        poll.sync_baseline_done = true;
    }

    /// What changed since the last poll (ARCHITECTURE.md, "Freshness").
    async fn poll_changes(&self, watched: Watched) -> Result<LibraryChanges, SourceError> {
        let mut poll = self.poll.lock().await;
        let mut changes = LibraryChanges::default();

        // Nothing is remembered until both requests succeeded, so a failed poll is simply repeated.
        let mut new_buckets = None;
        let buckets = self.client.time_buckets_if_changed(&TimelineScope::Library, poll.buckets_etag.as_deref()).await;
        if let Fetched::Changed { value, etag } = buckets.map_err(source_error)? {
            let counts: HashMap<YearMonth, u32> = value
                .iter()
                .map(|bucket| (YearMonth { year: bucket.month.year, month: bucket.month.month }, bucket.count))
                .collect();
            if poll.started {
                changes.timeline_months_changed = counts.keys().collect::<HashSet<_>>() != poll.month_counts.keys().collect();
                let mut months: HashSet<YearMonth> =
                    counts.iter().filter(|(month, count)| poll.month_counts.get(month) != Some(count)).map(|(month, _)| *month).collect();
                months.extend(poll.month_counts.keys().filter(|month| !counts.contains_key(month)));
                changes.months = months.into_iter().collect();
            }
            new_buckets = Some((counts, etag));
        }
        // A section not watched is forgotten, so watching it again starts by taking note.
        let archive = match watched.archive {
            true => Some(self.watched_buckets(&TimelineScope::Archive, poll.archive.as_ref()).await?),
            false => None,
        };
        changes.archive_changed = section_changed(poll.archive.as_ref(), archive.as_ref());
        let favorites = match watched.favorites {
            true => Some(self.watched_buckets(&TimelineScope::Favorites, poll.favorites.as_ref()).await?),
            false => None,
        };
        changes.favorites_changed = section_changed(poll.favorites.as_ref(), favorites.as_ref());

        // A credential that may not read albums still gets the timeline's changes.
        let albums_answer = match self.client.albums_if_changed(poll.albums_etag.as_deref()).await {
            Err(ApiError::Forbidden { .. }) => Ok(Fetched::NotModified),
            other => other,
        };
        if let Fetched::Changed { value, etag } = albums_answer.map_err(source_error)? {
            let albums: HashMap<String, (String, u64, DateTime<Utc>)> =
                value.into_iter().map(|album| (album.id, (album.name, album.asset_count, album.updated_at))).collect();
            if poll.started {
                let same_names = albums.len() == poll.albums.len()
                    && albums.iter().all(|(id, (name, _, _))| poll.albums.get(id).is_some_and(|(old_name, _, _)| old_name == name));
                changes.album_list_changed = !same_names;
                changes.albums = albums
                    .iter()
                    .filter(|(id, (_, count, updated_at))| {
                        poll.albums.get(*id).is_none_or(|(_, old_count, old_updated_at)| old_count != count || old_updated_at != updated_at)
                    })
                    .map(|(id, _)| id.clone())
                    .collect();
            }
            poll.albums = albums;
            poll.albums_etag = etag;
        }
        if let Some((counts, etag)) = new_buckets {
            poll.month_counts = counts;
            poll.buckets_etag = etag;
        }
        poll.archive = archive;
        poll.favorites = favorites;

        if self.client.credential().is_some_and(|credential| matches!(credential, immich_api::Credential::Session(_))) {
            // Sessions also follow the sync stream, which is exact: deletions and album membership included. Running
            // out of time is not being offline: the server answered the requests above.
            match tokio::time::timeout(SYNC_LIMIT, self.follow_sync_stream(&mut changes, poll.sync_baseline_done)).await {
                Ok(Ok(())) => poll.sync_baseline_done = true,
                Ok(Err(error)) => log::debug!("sync stream: {error}"),
                Err(_) => log::debug!("sync stream: still reading; the next poll goes on from the last ack"),
            }
        }
        poll.started = true;
        Ok(changes)
    }

    /// A watched section's buckets as they are now: those seen before when the server answers that nothing changed.
    async fn watched_buckets(&self, scope: &TimelineScope, seen: Option<&WatchedBuckets>) -> Result<WatchedBuckets, SourceError> {
        let answer = self.client.time_buckets_if_changed(scope, seen.and_then(|seen| seen.etag.as_deref())).await.map_err(source_error)?;
        Ok(match answer {
            Fetched::NotModified => seen.cloned().unwrap_or_default(),
            Fetched::Changed { value, etag } => {
                let mut month_counts: Vec<(YearMonth, u32)> = value
                    .iter()
                    .filter(|bucket| bucket.count > 0)
                    .map(|bucket| (YearMonth { year: bucket.month.year, month: bucket.month.month }, bucket.count))
                    .collect();
                month_counts.sort();
                WatchedBuckets { etag, month_counts }
            }
        })
    }

    /// Read the sync stream's records since the last ack into `changes`, then ack them (ARCHITECTURE.md, "Freshness").
    /// The first stream of a run only sets the baseline: it is acked without changes, since nothing is cached yet.
    async fn follow_sync_stream(&self, changes: &mut LibraryChanges, started: bool) -> Result<(), ApiError> {
        let mut stream = self.client.sync_stream(&["AssetsV2", "AlbumToAssetsV1"], false).await?;
        let mut last_acks: HashMap<String, String> = HashMap::new();
        let mut months: HashSet<YearMonth> = changes.months.iter().copied().collect();
        let mut albums: HashSet<String> = changes.albums.iter().cloned().collect();
        let mut since_ack = 0;
        while let Some(line) = stream.next_line().await? {
            let kind = match &line.record {
                immich_api::SyncRecord::AssetV2(asset) => {
                    if let Some(local) = asset.local_date_time {
                        months.insert(YearMonth { year: local.year(), month: local.month() });
                    }
                    "AssetV2".to_owned()
                }
                immich_api::SyncRecord::AssetDeleteV1 { .. } => {
                    changes.everything = true;
                    "AssetDeleteV1".to_owned()
                }
                immich_api::SyncRecord::AssetExifV1(_) => "AssetExifV1".to_owned(),
                immich_api::SyncRecord::SyncResetV1 => {
                    changes.everything = true;
                    "SyncResetV1".to_owned()
                }
                immich_api::SyncRecord::SyncCompleteV1 => "SyncCompleteV1".to_owned(),
                immich_api::SyncRecord::Other { kind, data } => {
                    if let Some(album_id) = data.get("albumId").and_then(|value| value.as_str()) {
                        albums.insert(album_id.to_owned());
                    }
                    kind.clone()
                }
            };
            last_acks.insert(kind, line.ack);
            since_ack += 1;
            if !started && since_ack >= BASELINE_ACK_EVERY {
                self.client.sync_ack(&last_acks.values().cloned().collect::<Vec<_>>()).await?;
                since_ack = 0;
            }
        }
        // The changes are taken before the ack: if the ack's answer is lost, they are not.
        if started {
            changes.months = months.into_iter().collect();
            changes.albums = albums.into_iter().collect();
        } else {
            changes.everything = false;
        }
        let record_kinds = last_acks.keys().cloned().collect::<Vec<_>>().join(", ");
        if !last_acks.is_empty() {
            self.client.sync_ack(&last_acks.into_values().collect::<Vec<_>>()).await?;
        }
        log::debug!("sync stream: {}", if record_kinds.is_empty() { "nothing new".to_owned() } else { record_kinds });
        Ok(())
    }

    /// Remember an unreachable server for a moment; any answer from it ends that.
    fn note_outcome<T>(&self, outcome: &Result<T, SourceError>) {
        let mut offline_until = self.offline_until.lock().expect("offline lock");
        match outcome {
            Err(SourceError::Offline(_)) => *offline_until = Some(std::time::Instant::now() + OFFLINE_GRACE),
            _ => *offline_until = None,
        }
    }

    /// Every shown Asset matching `query`, in the core's model.
    async fn search(&self, query: AssetQuery) -> Result<Vec<Asset>, SourceError> {
        let found = self.client.search_all_assets(&query, PAGE_SIZE).await.map_err(source_error)?;
        Ok(self.with_sizes(found.into_iter().filter(is_shown).map(to_asset).collect()).await)
    }

    /// Fill in the sizes the search did not report (an Asset whose metadata Immich has not extracted yet) with a
    /// `HEAD` of the original, which downloads nothing (SPEC.md, "Attributes"). A size that stays unknown makes reads
    /// fail rather than look empty.
    async fn with_sizes(&self, mut assets: Vec<Asset>) -> Vec<Asset> {
        let missing: Vec<(usize, String)> =
            assets.iter().enumerate().filter(|(_, asset)| asset.size.is_none()).map(|(index, asset)| (index, asset.id.clone())).collect();
        if missing.is_empty() {
            return assets;
        }
        let sizes: Vec<(usize, Option<u64>)> = futures_util::stream::iter(missing)
            .map(|(index, asset_id)| async move { (index, self.client.original_head(&asset_id).await.ok().map(|(size, _)| size)) })
            .buffer_unordered(SIZE_PROBES)
            .collect()
            .await;
        for (index, size) in sizes {
            assets[index].size = size;
        }
        assets
    }
}

impl LibrarySource for ImmichSource {
    async fn timeline_months(&self) -> Result<Vec<(YearMonth, u32)>, SourceError> {
        self.check_online()?;
        let outcome: Result<Vec<(YearMonth, u32)>, SourceError> = async {
            let buckets = self.client.time_buckets(&TimelineScope::Library).await.map_err(source_error)?;
            Ok(buckets
                .into_iter()
                .filter(|bucket| bucket.count > 0)
                .map(|bucket| (YearMonth { year: bucket.month.year, month: bucket.month.month }, bucket.count))
                .collect())
        }
        .await;
        self.note_outcome(&outcome);
        outcome
    }

    async fn month_assets(&self, month: YearMonth) -> Result<Vec<Asset>, SourceError> {
        self.check_online()?;
        let outcome: Result<Vec<Asset>, SourceError> = async {
            let bucket_month = immich_api::YearMonth::new(month.year, month.month).ok_or(SourceError::NotFound)?;
            let query = AssetQuery {
                taken: Some(bucket_month.utc_range_widened(Duration::hours(TIME_ZONE_MARGIN_HOURS))),
                visibility: Some(Visibility::Timeline),
                ..AssetQuery::default()
            };
            // Membership and details are asked for together: neither needs the other's answer.
            let (bucket, found) =
                futures_util::future::join(self.client.time_bucket(&TimelineScope::Library, bucket_month), self.search(query)).await;
            let members: HashSet<String> =
                bucket.map_err(source_error)?.into_iter().filter(|member| !member.is_trashed).map(|member| member.id).collect();
            if members.is_empty() {
                return Ok(Vec::new());
            }
            let mut assets: Vec<Asset> = found?.into_iter().filter(|asset| members.contains(&asset.id)).collect();

            let found: HashSet<&str> = assets.iter().map(|asset| asset.id.as_str()).collect();
            let missed: Vec<String> = members.iter().filter(|id| !found.contains(id.as_str())).cloned().collect();
            if missed.len() > MISSED_MEMBER_LIMIT {
                return Err(SourceError::Protocol(format!(
                    "the search for {}-{:02} missed {} of its {} Assets",
                    month.year,
                    month.month,
                    missed.len(),
                    members.len()
                )));
            }
            for id in missed {
                match self.client.asset(&id).await {
                    Ok(asset) if is_shown(&asset) => assets.push(to_asset(asset)),
                    Ok(_) | Err(ApiError::NotFound) => {}
                    Err(error) => return Err(source_error(error)),
                }
            }
            Ok(self.with_sizes(assets).await)
        }
        .await;
        self.note_outcome(&outcome);
        outcome
    }

    async fn albums(&self) -> Result<Vec<Album>, SourceError> {
        self.check_online()?;
        let outcome: Result<Vec<Album>, SourceError> = async {
            let albums = self.client.albums().await.map_err(source_error)?;
            Ok(albums
                .into_iter()
                .map(|album| Album {
                    id: album.id,
                    name: album.name,
                    year: album.end_date.map(|end| end.year()),
                    created_at: album.created_at,
                })
                .collect())
        }
        .await;
        self.note_outcome(&outcome);
        outcome
    }

    async fn album_assets(&self, album_id: &str) -> Result<Vec<Asset>, SourceError> {
        self.check_online()?;
        let outcome: Result<Vec<Asset>, SourceError> = async {
            self.search(AssetQuery { album_ids: Some(vec![album_id.to_owned()]), ..AssetQuery::default() }).await
        }
        .await;
        self.note_outcome(&outcome);
        outcome
    }

    async fn people(&self) -> Result<Vec<Person>, SourceError> {
        self.check_online()?;
        let outcome: Result<Vec<Person>, SourceError> = async {
            let people = self.client.people().await.map_err(source_error)?;
            Ok(people
                .into_iter()
                .map(|person| Person {
                    id: person.id,
                    name: person.name,
                    is_hidden: person.is_hidden,
                    created_at: person.created_at.unwrap_or(DateTime::<Utc>::UNIX_EPOCH),
                })
                .collect())
        }
        .await;
        self.note_outcome(&outcome);
        outcome
    }

    async fn person_assets(&self, person_id: &str) -> Result<Vec<Asset>, SourceError> {
        self.check_online()?;
        let outcome: Result<Vec<Asset>, SourceError> = async {
            self.search(AssetQuery {
                person_ids: Some(vec![person_id.to_owned()]),
                visibility: Some(Visibility::Timeline),
                ..AssetQuery::default()
            })
            .await
        }
        .await;
        self.note_outcome(&outcome);
        outcome
    }

    async fn countries(&self) -> Result<Vec<String>, SourceError> {
        self.check_online()?;
        let outcome: Result<Vec<String>, SourceError> = async {
            self.client.countries().await.map_err(source_error)
        }
        .await;
        self.note_outcome(&outcome);
        outcome
    }

    async fn cities(&self, country: &str) -> Result<Vec<String>, SourceError> {
        self.check_online()?;
        let outcome: Result<Vec<String>, SourceError> = async {
            self.client.cities(country).await.map_err(source_error)
        }
        .await;
        self.note_outcome(&outcome);
        outcome
    }

    async fn changes(&self, watched: Watched) -> Result<LibraryChanges, SourceError> {
        // The poll is also the probe that notices the server is back, so it never waits out the offline grace.
        let outcome = match tokio::time::timeout(POLL_LIMIT, self.poll_changes(watched)).await {
            Ok(outcome) => outcome,
            Err(_) => Err(SourceError::Offline("the change poll took too long".to_owned())),
        };
        self.note_outcome(&outcome);
        outcome
    }

    async fn city_assets(&self, country: &str, city: &str) -> Result<Vec<Asset>, SourceError> {
        self.check_online()?;
        let outcome: Result<Vec<Asset>, SourceError> = async {
            self.search(AssetQuery {
                country: Some(country.to_owned()),
                city: Some(city.to_owned()),
                visibility: Some(Visibility::Timeline),
                ..AssetQuery::default()
            })
            .await
        }
        .await;
        self.note_outcome(&outcome);
        outcome
    }

    async fn archive_assets(&self) -> Result<Vec<Asset>, SourceError> {
        self.check_online()?;
        let outcome: Result<Vec<Asset>, SourceError> = async {
            self.search(AssetQuery { visibility: Some(Visibility::Archive), ..AssetQuery::default() }).await
        }
        .await;
        self.note_outcome(&outcome);
        outcome
    }

    async fn favorite_assets(&self) -> Result<Vec<Asset>, SourceError> {
        self.check_online()?;
        let outcome: Result<Vec<Asset>, SourceError> = async {
            self.search(AssetQuery { favorites_only: true, ..AssetQuery::default() }).await
        }
        .await;
        self.note_outcome(&outcome);
        outcome
    }
}

impl ContentSource for ImmichSource {
    async fn read_range(&self, asset_id: &str, start: u64, end_inclusive: u64) -> Result<RangeBytes, SourceError> {
        self.check_online()?;
        let outcome: Result<RangeBytes, SourceError> = async {
            let response = self.client.original_range(asset_id, start, end_inclusive).await.map_err(source_error)?;
            Ok(RangeBytes { bytes: response.bytes.to_vec(), version: response.etag })
        }
        .await;
        self.note_outcome(&outcome);
        outcome
    }
}

impl ImmichSource {
    /// An Asset's whole original, written to `destination`, for a platform that asks for whole files (the macOS
    /// edition). Fails at once while the server was just found unreachable, as reads do.
    pub async fn download_original(&self, asset_id: &str, destination: &std::path::Path) -> Result<u64, SourceError> {
        self.check_online()?;
        let outcome = self.client.download_original(asset_id, destination).await.map_err(source_error);
        self.note_outcome(&outcome);
        outcome
    }
}

impl ThumbnailSource for ImmichSource {
    async fn thumbnail(&self, asset_id: &str, kind: ThumbnailKind) -> Result<Vec<u8>, SourceError> {
        self.check_online()?;
        let outcome: Result<Vec<u8>, SourceError> = async {
            let size = match kind {
                ThumbnailKind::Small => ThumbnailSize::Thumbnail,
                ThumbnailKind::Preview => ThumbnailSize::Preview,
            };
            let (bytes, _content_type) = self.client.thumbnail(asset_id, size).await.map_err(source_error)?;
            Ok(bytes.to_vec())
        }
        .await;
        self.note_outcome(&outcome);
        outcome
    }
}

impl UploadTarget for ImmichSource {
    async fn media_types(&self) -> Result<MediaTypes, SourceError> {
        let extensions = self.client.server_media_types().await.map_err(source_error)?;
        Ok(MediaTypes::new(extensions))
    }

    /// The upload sequence (ARCHITECTURE.md, "Uploads"): the checksum, the bulk check, which settles duplicates
    /// without sending a byte, then the streamed upload. A lost answer is safe to retry: the checksum turns a second
    /// upload into a duplicate of the first, not a second Asset.
    async fn upload(&self, path: &std::path::Path, file_name: &str, modified: DateTime<Utc>) -> Result<Uploaded, UploadError> {
        self.check_online()?;
        let checksum = immich_api::Sha1Checksum::of_file(path).await.map_err(|error| UploadError::LocalFile(error.to_string()))?;
        let checks = self.client.bulk_upload_check(&[checksum.to_base64()]).await.map_err(source_error)?;
        let outcome = match checks.into_iter().next() {
            Some(immich_api::UploadCheck::Duplicate { trashed: true, .. }) => return Err(UploadError::TrashedDuplicate),
            Some(immich_api::UploadCheck::Duplicate { asset_id, trashed: false }) => return Ok(Uploaded::AlreadyInLibrary(asset_id)),
            Some(immich_api::UploadCheck::Reject { reason }) => return Err(UploadError::Rejected(reason)),
            _ => self.client.upload_asset(path, file_name, modified, modified).await,
        };
        let result = match outcome {
            Ok(immich_api::UploadOutcome::Created(asset_id)) => Ok(Uploaded::Created(asset_id)),
            // After an `Accept`, a duplicate is this upload's own earlier attempt, whose answer was lost.
            Ok(immich_api::UploadOutcome::Duplicate { id, trashed: false }) => Ok(Uploaded::AlreadyInLibrary(id)),
            Ok(immich_api::UploadOutcome::Duplicate { trashed: true, .. }) => Err(UploadError::TrashedDuplicate),
            Err(ApiError::LocalFile(message)) => Err(UploadError::LocalFile(message)),
            Err(error) => Err(UploadError::Source(source_error(error))),
        };
        if let Err(UploadError::Source(error)) = &result {
            self.note_outcome::<()>(&Err(error.clone()));
        }
        result
    }
}

/// Albums of the folders copied into Upload (ARCHITECTURE.md, "Folders copied into Upload"). Making one needs
/// `album.read` (for the names taken), `album.create` and `albumAsset.create`.
impl AlbumTarget for ImmichSource {
    async fn album_names(&self) -> Result<Vec<String>, SourceError> {
        self.check_online()?;
        let albums = self.client.albums().await.map_err(source_error)?;
        Ok(albums.into_iter().map(|album| album.name).collect())
    }

    async fn create_album(&self, name: &str, asset_ids: &[String]) -> Result<String, SourceError> {
        self.check_online()?;
        let album = immich_api::NewAlbum { name: name.to_owned(), asset_ids: asset_ids.to_vec(), ..immich_api::NewAlbum::default() };
        Ok(self.client.create_album(&album).await.map_err(source_error)?.id)
    }

    async fn add_to_album(&self, album_id: &str, asset_ids: &[String]) -> Result<(), SourceError> {
        self.check_online()?;
        self.client.add_assets_to_album(album_id, asset_ids).await.map_err(source_error)?;
        Ok(())
    }

    async fn album_with(&self, name: &str, asset_id: &str) -> Result<Option<String>, SourceError> {
        self.check_online()?;
        let filter = immich_api::AlbumFilter { containing_asset: Some(asset_id.to_owned()), ..immich_api::AlbumFilter::default() };
        let albums = self.client.albums_where(&filter).await.map_err(source_error)?;
        Ok(albums.into_iter().find(|album| album.name == name).map(|album| album.id))
    }
}

/// Whether a watched section's months changed between two polls. A section seen for the first time only takes note.
fn section_changed(seen: Option<&WatchedBuckets>, now: Option<&WatchedBuckets>) -> bool {
    matches!((seen, now), (Some(seen), Some(now)) if seen.month_counts != now.month_counts)
}

/// Whether an Asset belongs in the mount at all.
fn is_shown(asset: &immich_api::Asset) -> bool {
    !asset.is_trashed && matches!(asset.visibility, Visibility::Timeline | Visibility::Archive)
}

fn to_asset(asset: immich_api::Asset) -> Asset {
    Asset {
        id: asset.id,
        original_file_name: asset.original_file_name,
        local_capture_time: asset.local_date_time,
        captured_at: asset.file_created_at,
        uploaded_at: asset.created_at,
        size: asset.file_size,
        city: asset.city,
        country: asset.country,
    }
}

/// The kind of failure the mount branches on, for a client error.
pub fn source_error(error: ApiError) -> SourceError {
    match error {
        ApiError::Offline(reason) => SourceError::Offline(reason),
        ApiError::Unauthorized => SourceError::SignedOut,
        ApiError::Forbidden { missing_permission } => SourceError::MissingPermission(missing_permission),
        ApiError::NotFound => SourceError::NotFound,
        ApiError::RangeNotSatisfiable => SourceError::Changed,
        ApiError::Transient { .. } => SourceError::Busy,
        ApiError::Protocol(reason) | ApiError::LocalFile(reason) => SourceError::Protocol(reason),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unreachable_server_fails_requests_at_once_until_it_answers_again() {
        let source = ImmichSource::new(Client::new("http://127.0.0.1:9", None, "test").unwrap());
        assert!(source.check_online().is_ok());
        source.note_outcome::<()>(&Err(SourceError::Offline("refused".into())));
        assert!(matches!(source.check_online(), Err(SourceError::Offline(_))));
        source.note_outcome(&Ok(()));
        assert!(source.check_online().is_ok(), "any answer from the server ends the grace");
    }

    #[tokio::test]
    async fn what_the_poll_saw_comes_back_as_it_was_kept() {
        let source = ImmichSource::new(Client::new("http://127.0.0.1:9", None, "test").unwrap());
        assert_eq!(source.poll_memory().await, None, "nothing seen yet");
        let kept = r#"{"buckets_etag":"W/\"1\"","month_counts":[[{"year":2024,"month":3},5]],"albums_etag":null,"albums":[["a","Trip",2,"2024-03-01T00:00:00Z"]]}"#;
        source.recall_poll(kept).await;
        let again: serde_json::Value = serde_json::from_str(&source.poll_memory().await.unwrap()).unwrap();
        assert_eq!(again, serde_json::from_str::<serde_json::Value>(kept).unwrap());
        let fresh = ImmichSource::new(Client::new("http://127.0.0.1:9", None, "test").unwrap());
        fresh.recall_poll("not what a poll keeps").await;
        assert_eq!(fresh.poll_memory().await, None, "ignored: the next poll only takes note");
        let with_archive = r#"{"buckets_etag":null,"month_counts":[],"albums_etag":null,"albums":[],"archive":{"etag":"W/\"2\"","month_counts":[[{"year":2023,"month":1},4]]}}"#;
        source.recall_poll(with_archive).await;
        let again: serde_json::Value = serde_json::from_str(&source.poll_memory().await.unwrap()).unwrap();
        assert_eq!(again, serde_json::from_str::<serde_json::Value>(with_archive).unwrap(), "the archive's buckets are kept too");
    }

    #[test]
    fn a_watched_section_changes_when_its_months_do() {
        let buckets = |count| WatchedBuckets { etag: None, month_counts: vec![(YearMonth { year: 2023, month: 1 }, count)] };
        assert!(section_changed(Some(&buckets(4)), Some(&buckets(5))));
        assert!(!section_changed(Some(&buckets(4)), Some(&buckets(4))));
        assert!(!section_changed(None, Some(&buckets(4))), "first seen: only noted");
        assert!(!section_changed(Some(&buckets(4)), None), "no longer watched");
    }

    #[test]
    fn client_errors_map_onto_the_kinds_the_mount_branches_on() {
        assert_eq!(source_error(ApiError::Unauthorized), SourceError::SignedOut);
        assert_eq!(source_error(ApiError::Transient { retry_after: None }), SourceError::Busy);
        assert_eq!(
            source_error(ApiError::Forbidden { missing_permission: Some("asset.download".into()) }),
            SourceError::MissingPermission(Some("asset.download".into()))
        );
        assert!(matches!(source_error(ApiError::RangeNotSatisfiable), SourceError::Changed));
    }
}
