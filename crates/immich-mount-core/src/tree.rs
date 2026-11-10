//! The tree engine: what each folder of the mount contains, with final names and attributes.
//!
//! Membership comes from the `LibrarySource` (Immich decides). This module applies the layout (sections, date and day
//! folders, album layouts), names every entry (`naming`), and remembers the names it handed out (`NameMemory`), so a
//! path keeps meaning the same Asset. Listings are cached for a freshness window (ARCHITECTURE.md, "Freshness").

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};

use crate::links::ImmichItem;
use crate::layout::{
    album_folder_base, album_year_folder, day_folder_name, month_folder_name, year_folder_name, AlbumLayout,
    GroupingMode, Section,
};
use crate::model::{Album, Asset, YearMonth};
use crate::naming::{assign_names, file_base_name, sanitise, NameRequest};
use crate::settings::Config;
use crate::source::{LibraryChanges, LibrarySource, SourceError, Watched};

/// The identity of every node in the mount. The parent is part of an Asset's identity, so one Asset can appear in
/// several folders (one Appearance each).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub enum NodeKey {
    Root,
    Section(SectionKey),
    /// The Upload folder: it holds only files being uploaded, which the platform's filesystem lists.
    Upload,
    /// A folder copied into Upload, by its path below Upload, the copied folder first. Like Upload it holds only
    /// what is being uploaded, which the platform lists (SPEC.md, "Uploads").
    UploadFolder(Vec<String>),
    TimelineYear(i32),
    TimelineMonth(YearMonth),
    TimelineDay(YearMonth, u32),
    /// A year folder above albums, with the `year-folders` layout. `None` holds albums without photos.
    AlbumYear(Option<i32>),
    Album(String),
    Person(String),
    Country(String),
    City { country: String, city: String },
    GroupYear { base: Box<NodeKey>, year: i32 },
    GroupMonth { base: Box<NodeKey>, month: YearMonth },
    Asset { parent: Box<NodeKey>, asset_id: String },
}

impl NodeKey {
    /// A stable text form, for storing per-folder state (assigned names, listings). Free text is escaped so the
    /// separator never appears inside a part.
    pub fn storage_key(&self) -> String {
        fn escape(text: &str) -> String {
            text.replace('%', "%25").replace('|', "%7C")
        }
        match self {
            NodeKey::Root => "root".to_string(),
            NodeKey::Upload => "upload".to_string(),
            NodeKey::UploadFolder(path) => {
                std::iter::once("upload-folder".to_owned()).chain(path.iter().map(|name| escape(name))).collect::<Vec<_>>().join("|")
            }
            NodeKey::Section(section) => format!("section|{section:?}"),
            NodeKey::TimelineYear(year) => format!("year|{year}"),
            NodeKey::TimelineMonth(month) => format!("month|{}-{:02}", month.year, month.month),
            NodeKey::TimelineDay(month, day) => format!("day|{}-{:02}-{day:02}", month.year, month.month),
            NodeKey::AlbumYear(Some(year)) => format!("album-year|{year}"),
            NodeKey::AlbumYear(None) => "album-year|unknown".to_string(),
            NodeKey::Album(id) => format!("album|{}", escape(id)),
            NodeKey::Person(id) => format!("person|{}", escape(id)),
            NodeKey::Country(country) => format!("country|{}", escape(country)),
            NodeKey::City { country, city } => format!("city|{}|{}", escape(country), escape(city)),
            NodeKey::GroupYear { base, year } => format!("group-year|{}|{year}", escape(&base.storage_key())),
            NodeKey::GroupMonth { base, month } => {
                format!("group-month|{}|{}-{:02}", escape(&base.storage_key()), month.year, month.month)
            }
            NodeKey::Asset { parent, asset_id } => format!("asset|{}|{}", escape(&parent.storage_key()), escape(asset_id)),
        }
    }

    /// The key back from its storage form, as the macOS edition's item identifiers carry it (ARCHITECTURE.md, "The
    /// macOS edition"). `None` for any text `storage_key` does not write, so an identifier has one meaning only.
    pub fn from_storage_key(text: &str) -> Option<NodeKey> {
        // Escaped text holds `%` only as `%25` or `%7C`, so the pipe goes back first.
        fn unescape(text: &str) -> String {
            text.replace("%7C", "|").replace("%25", "%")
        }
        fn year_month(text: &str) -> Option<YearMonth> {
            let (year, month) = text.rsplit_once('-')?;
            Some(YearMonth { year: year.parse().ok()?, month: month.parse().ok()? })
        }
        let parts: Vec<&str> = text.split('|').collect();
        let node = match parts.as_slice() {
            ["root"] => NodeKey::Root,
            ["upload"] => NodeKey::Upload,
            ["upload-folder", path @ ..] if !path.is_empty() => NodeKey::UploadFolder(path.iter().map(|name| unescape(name)).collect()),
            ["section", "Photos"] => NodeKey::Section(SectionKey::Photos),
            ["section", "Albums"] => NodeKey::Section(SectionKey::Albums),
            ["section", "People"] => NodeKey::Section(SectionKey::People),
            ["section", "Places"] => NodeKey::Section(SectionKey::Places),
            ["section", "Favorites"] => NodeKey::Section(SectionKey::Favorites),
            ["section", "Archive"] => NodeKey::Section(SectionKey::Archive),
            ["year", year] => NodeKey::TimelineYear(year.parse().ok()?),
            ["month", month] => NodeKey::TimelineMonth(year_month(month)?),
            ["day", date] => {
                let (month, day) = date.rsplit_once('-')?;
                NodeKey::TimelineDay(year_month(month)?, day.parse().ok()?)
            }
            ["album-year", "unknown"] => NodeKey::AlbumYear(None),
            ["album-year", year] => NodeKey::AlbumYear(Some(year.parse().ok()?)),
            ["album", album_id] => NodeKey::Album(unescape(album_id)),
            ["person", person_id] => NodeKey::Person(unescape(person_id)),
            ["country", country] => NodeKey::Country(unescape(country)),
            ["city", country, city] => NodeKey::City { country: unescape(country), city: unescape(city) },
            ["group-year", base, year] => {
                NodeKey::GroupYear { base: Box::new(NodeKey::from_storage_key(&unescape(base))?), year: year.parse().ok()? }
            }
            ["group-month", base, month] => {
                NodeKey::GroupMonth { base: Box::new(NodeKey::from_storage_key(&unescape(base))?), month: year_month(month)? }
            }
            ["asset", parent, asset_id] => {
                NodeKey::Asset { parent: Box::new(NodeKey::from_storage_key(&unescape(parent))?), asset_id: unescape(asset_id) }
            }
            _ => return None,
        };
        // Numbers parse from more than one spelling ("3", "03", "+3"); only the one `storage_key` writes is accepted.
        (node.storage_key() == text).then_some(node)
    }

    /// Where a folder that takes new files is below Upload: empty for Upload itself, the path for a folder copied
    /// into it. `None` for every other node, which is a view of the library.
    pub fn upload_path(&self) -> Option<&[String]> {
        match self {
            NodeKey::Upload => Some(&[]),
            NodeKey::UploadFolder(path) => Some(path),
            _ => None,
        }
    }

    /// The folder named `name` inside a folder that takes new files; `None` inside any other.
    pub fn upload_child(&self, name: &str) -> Option<NodeKey> {
        let mut path = self.upload_path()?.to_vec();
        path.push(name.to_owned());
        Some(NodeKey::UploadFolder(path))
    }
}

/// `Section` as part of a hashable, ordered key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub enum SectionKey {
    Photos,
    Albums,
    People,
    Places,
    Favorites,
    Archive,
}

impl From<Section> for SectionKey {
    fn from(section: Section) -> SectionKey {
        match section {
            Section::Photos => SectionKey::Photos,
            Section::Albums => SectionKey::Albums,
            Section::People => SectionKey::People,
            Section::Places => SectionKey::Places,
            Section::Favorites => SectionKey::Favorites,
            Section::Archive => SectionKey::Archive,
        }
    }
}

/// One entry of a folder listing.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Entry {
    pub name: String,
    pub node: NodeKey,
    pub kind: EntryKind,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum EntryKind {
    Folder,
    File { asset_id: String, size: Option<u64>, modified: DateTime<Utc> },
}

/// Remembers the names handed out per folder, so they survive refreshes and restarts.
pub trait NameMemory: Send + Sync {
    fn remembered(&self, folder: &NodeKey) -> HashMap<String, String>;
    fn remember(&self, folder: &NodeKey, names: &HashMap<String, String>);

    /// Keep a folder's latest listing, so it can still be shown when Immich cannot be reached, after a restart too.
    fn keep_listing(&self, _folder: &NodeKey, _entries: &[Entry]) {}

    /// A kept listing, however old.
    fn kept_listing(&self, _folder: &NodeKey) -> Option<Vec<Entry>> {
        None
    }

    /// A listing that was answered, but fetched while a change came, so it is not cached as fresh. Nothing by
    /// default; the macOS edition's store keeps it, since the system holds what it was shown (`MetadataStore`).
    fn shown_during_change(&self, _folder: &NodeKey, _entries: &[Entry]) {}
}

/// One store can serve several trees through an `Arc`: the macOS edition builds a new tree when the settings change.
impl<Shared: NameMemory> NameMemory for Arc<Shared> {
    fn remembered(&self, folder: &NodeKey) -> HashMap<String, String> {
        self.as_ref().remembered(folder)
    }
    fn remember(&self, folder: &NodeKey, names: &HashMap<String, String>) {
        self.as_ref().remember(folder, names)
    }
    fn keep_listing(&self, folder: &NodeKey, entries: &[Entry]) {
        self.as_ref().keep_listing(folder, entries)
    }
    fn kept_listing(&self, folder: &NodeKey) -> Option<Vec<Entry>> {
        self.as_ref().kept_listing(folder)
    }
    fn shown_during_change(&self, folder: &NodeKey, entries: &[Entry]) {
        self.as_ref().shown_during_change(folder, entries)
    }
}

/// A `NameMemory` that lives only as long as the process; the metadata store replaces it in the daemon.
#[derive(Default)]
pub struct InMemoryNames {
    folders: Mutex<HashMap<NodeKey, HashMap<String, String>>>,
}

impl NameMemory for InMemoryNames {
    fn remembered(&self, folder: &NodeKey) -> HashMap<String, String> {
        self.folders.lock().expect("name memory lock").get(folder).cloned().unwrap_or_default()
    }
    fn remember(&self, folder: &NodeKey, names: &HashMap<String, String>) {
        self.folders.lock().expect("name memory lock").insert(folder.clone(), names.clone());
    }
}

/// How long a listing is served from cache before the source is asked again.
pub fn freshness_window(node: &NodeKey, now: DateTime<Utc>) -> Duration {
    use chrono::Datelike;
    let minutes = |count: u64| Duration::from_secs(count * 60);
    match node {
        NodeKey::Root | NodeKey::Section(SectionKey::Photos) | NodeKey::TimelineYear(_) => minutes(5),
        NodeKey::TimelineMonth(month) | NodeKey::TimelineDay(month, _) => {
            if month.year == now.year() && month.month == now.month() { minutes(1) } else { minutes(60) }
        }
        _ => minutes(10),
    }
}

/// Whether a folder's listing depends on something in `changes`.
pub fn is_touched(node: &NodeKey, changes: &LibraryChanges) -> bool {
    if changes.everything {
        return true;
    }
    let month_changed = |month: &YearMonth| changes.months.contains(month);
    match node {
        NodeKey::Section(SectionKey::Photos) => changes.timeline_months_changed,
        NodeKey::TimelineYear(year) => changes.timeline_months_changed || changes.months.iter().any(|month| month.year == *year),
        NodeKey::TimelineMonth(month) | NodeKey::TimelineDay(month, _) => month_changed(month),
        NodeKey::Section(SectionKey::Albums) | NodeKey::AlbumYear(_) => changes.album_list_changed || !changes.albums.is_empty(),
        NodeKey::Album(album_id) => changes.albums.contains(album_id),
        NodeKey::Section(SectionKey::Archive) => changes.archive_changed,
        NodeKey::Section(SectionKey::Favorites) => changes.favorites_changed,
        NodeKey::GroupYear { base, .. } | NodeKey::GroupMonth { base, .. } => is_touched(base, changes),
        _ => false,
    }
}

/// An Asset list and when it was fetched.
type FetchedAssets = (Instant, Arc<Vec<Asset>>);

/// The tree engine.
pub struct Tree<Source, Names> {
    source: Source,
    names: Names,
    config: Config,
    listings: Mutex<HashMap<NodeKey, (Instant, Vec<Entry>)>>,
    /// The Asset lists of albums, people, cities, favorites and the archive, which their date-group folders reuse.
    base_assets: Mutex<HashMap<NodeKey, FetchedAssets>>,
    /// One gate per folder being fetched, so concurrent visits wait for a single fetch.
    in_flight: Mutex<HashMap<NodeKey, Arc<tokio::sync::Mutex<()>>>>,
    /// Counts invalidations. A fetch that saw the server before an invalidation must not be cached as fresh after it.
    generation: std::sync::atomic::AtomicU64,
}

impl<Source: LibrarySource, Names: NameMemory> Tree<Source, Names> {
    pub fn new(source: Source, names: Names, config: Config) -> Self {
        Tree {
            source,
            names,
            config,
            listings: Mutex::new(HashMap::new()),
            base_assets: Mutex::new(HashMap::new()),
            in_flight: Mutex::new(HashMap::new()),
            generation: std::sync::atomic::AtomicU64::new(0),
        }
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    /// The sections the change poll must watch, because this tree shows them.
    pub fn watched(&self) -> Watched {
        Watched { archive: self.config.show_archive, favorites: self.config.show_favorites }
    }

    /// Where names and listings are kept: the metadata store, which also holds the macOS edition's journal.
    pub fn names(&self) -> &Names {
        &self.names
    }

    /// Forget a cached listing, so the next call asks the source again.
    pub fn invalidate(&self, node: &NodeKey) {
        self.generation.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.listings.lock().expect("listing cache lock").remove(node);
        self.base_assets.lock().expect("base asset lock").remove(node);
    }

    /// Forget every listing, so every folder is read from Immich again on its next visit.
    pub fn forget_all(&self) {
        self.generation.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.listings.lock().expect("listing cache lock").clear();
        self.base_assets.lock().expect("base asset lock").clear();
    }

    /// Forget every listing of the timeline (Photos, its years, months and days), after an upload whose month is not
    /// known yet.
    pub fn forget_timeline(&self) {
        self.generation.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let is_timeline = |node: &NodeKey| {
            matches!(node, NodeKey::Section(SectionKey::Photos) | NodeKey::TimelineYear(_) | NodeKey::TimelineMonth(_) | NodeKey::TimelineDay(..))
        };
        self.listings.lock().expect("listing cache lock").retain(|node, _| !is_timeline(node));
    }

    /// Forget every listing the changes touch, so the next visit reads them again. Returns how many were forgotten.
    pub fn apply_changes(&self, changes: &LibraryChanges) -> usize {
        if changes.is_empty() {
            return 0;
        }
        self.generation.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.base_assets.lock().expect("base asset lock").retain(|node, _| !is_touched(node, changes));
        let mut listings = self.listings.lock().expect("listing cache lock");
        let before = listings.len();
        listings.retain(|node, _| !is_touched(node, changes));
        before - listings.len()
    }

    fn fresh_listing(&self, node: &NodeKey) -> Option<Vec<Entry>> {
        let listings = self.listings.lock().expect("listing cache lock");
        let (fetched_at, entries) = listings.get(node)?;
        (fetched_at.elapsed() < freshness_window(node, Utc::now())).then(|| entries.clone())
    }

    /// The entries of a folder, from cache while fresh. Concurrent calls for one folder share a single fetch.
    pub async fn list(&self, node: &NodeKey) -> Result<Vec<Entry>, SourceError> {
        self.list_or_not(node, true).await
    }

    /// The entries of a folder as the source has them now, for a platform that must know a change reached it (the
    /// macOS edition's poll): offline or busy is an error, never an older listing.
    pub async fn relist(&self, node: &NodeKey) -> Result<Vec<Entry>, SourceError> {
        self.list_or_not(node, false).await
    }

    async fn list_or_not(&self, node: &NodeKey, older_will_do: bool) -> Result<Vec<Entry>, SourceError> {
        if let Some(entries) = self.fresh_listing(node) {
            return Ok(entries);
        }
        let gate = self.in_flight.lock().expect("in-flight lock").entry(node.clone()).or_default().clone();
        let _turn = gate.lock().await;
        if let Some(entries) = self.fresh_listing(node) {
            return Ok(entries);
        }
        let generation = self.generation.load(std::sync::atomic::Ordering::SeqCst);
        let entries = match self.list_uncached(node).await {
            Ok(entries) => entries,
            // Offline or busy, an older listing beats an error: the folder still opens, and the next visit tries
            // again (SPEC.md, "Errors seen by programs using the mount").
            Err(error @ (SourceError::Offline(_) | SourceError::Busy)) if older_will_do => return self.stale_listing(node).ok_or(error),
            Err(error) => return Err(error),
        };
        // An invalidation during the fetch means the server changed under it: answer, but do not keep it as fresh.
        if self.generation.load(std::sync::atomic::Ordering::SeqCst) == generation {
            let previous = self.listings.lock().expect("listing cache lock").insert(node.clone(), (Instant::now(), entries.clone()));
            // The database write is skipped when nothing changed, which is most refreshes.
            if previous.is_none_or(|(_, previous)| previous != entries) {
                self.names.keep_listing(node, &entries);
            }
        } else {
            self.names.shown_during_change(node, &entries);
        }
        self.in_flight.lock().expect("in-flight lock").remove(node);
        Ok(entries)
    }

    /// A listing still held but past its freshness window: answered at once while a fresh one is fetched
    /// (stale-while-revalidate). A listing forgotten because Immich changed is not held, so a known change is always
    /// fetched before the folder answers.
    pub fn expired_listing(&self, node: &NodeKey) -> Option<Vec<Entry>> {
        let listings = self.listings.lock().expect("listing cache lock");
        let (fetched_at, entries) = listings.get(node)?;
        (fetched_at.elapsed() >= freshness_window(node, Utc::now())).then(|| entries.clone())
    }

    /// A listing however old, for when the source cannot be reached: from memory, or kept from an earlier run.
    pub fn stale_listing(&self, node: &NodeKey) -> Option<Vec<Entry>> {
        let in_memory = self.listings.lock().expect("listing cache lock").get(node).map(|(_, entries)| entries.clone());
        in_memory.or_else(|| self.names.kept_listing(node))
    }

    /// The folder that lists `node`; `None` for the root. The key says, except for an album under year folders,
    /// whose year comes from the album list.
    pub async fn parent_of(&self, node: &NodeKey) -> Result<Option<NodeKey>, SourceError> {
        let parent = match node {
            NodeKey::Root => return Ok(None),
            NodeKey::Section(_) | NodeKey::Upload => NodeKey::Root,
            NodeKey::UploadFolder(path) if path.len() > 1 => NodeKey::UploadFolder(path[..path.len() - 1].to_vec()),
            NodeKey::UploadFolder(_) => NodeKey::Upload,
            NodeKey::TimelineYear(_) => NodeKey::Section(SectionKey::Photos),
            NodeKey::TimelineMonth(month) => NodeKey::TimelineYear(month.year),
            NodeKey::TimelineDay(month, _) => NodeKey::TimelineMonth(*month),
            NodeKey::AlbumYear(_) => NodeKey::Section(SectionKey::Albums),
            NodeKey::Album(album_id) if self.config.album_layout == AlbumLayout::YearFolders => {
                let albums = self.source.albums().await?;
                let album = albums.iter().find(|album| album.id == *album_id).ok_or(SourceError::NotFound)?;
                NodeKey::AlbumYear(album.year)
            }
            NodeKey::Album(_) => NodeKey::Section(SectionKey::Albums),
            NodeKey::Person(_) => NodeKey::Section(SectionKey::People),
            NodeKey::Country(_) => NodeKey::Section(SectionKey::Places),
            NodeKey::City { country, .. } => NodeKey::Country(country.clone()),
            NodeKey::GroupYear { base, .. } => (**base).clone(),
            NodeKey::GroupMonth { base, month } => NodeKey::GroupYear { base: base.clone(), year: month.year },
            NodeKey::Asset { parent, .. } => (**parent).clone(),
        };
        Ok(Some(parent))
    }

    /// The entry that lists `node`, and the folder it is listed in, for a platform that asks about one item by its
    /// key (the macOS edition). `None` for the root, which no folder lists, and for a node its folder does not hold
    /// (any more).
    pub async fn entry_of(&self, node: &NodeKey) -> Result<Option<(NodeKey, Entry)>, SourceError> {
        let parent = match self.parent_of(node).await {
            Ok(Some(parent)) => parent,
            Ok(None) | Err(SourceError::NotFound) => return Ok(None),
            Err(error) => return Err(error),
        };
        let entry = self.list(&parent).await?.into_iter().find(|entry| entry.node == *node);
        Ok(entry.map(|entry| (parent, entry)))
    }

    /// Find one entry of a folder by name.
    pub async fn lookup(&self, parent: &NodeKey, name: &str) -> Result<Option<Entry>, SourceError> {
        Ok(self.list(parent).await?.into_iter().find(|entry| entry.name == name))
    }

    /// The key of the item a link names, where this mount shows it (ROADMAP.md, "Show in Immich Mount"); `None` where
    /// it shows it nowhere: a section that is off, a person the People section leaves out, an id the account cannot
    /// see. A photo shows in the folder the link names, or in its own place where that folder is not shown or no longer
    /// holds it: Photos, or Archive when it is archived and that section is on. Its date folders come from the Asset.
    pub async fn locate(&self, item: &ImmichItem) -> Result<Option<NodeKey>, SourceError> {
        let ImmichItem::Asset { within, asset_id } = item else {
            return self.locate_folder(item).await;
        };
        let asset = match self.source.asset(asset_id).await {
            Ok(asset) => asset,
            Err(SourceError::NotFound) => return Ok(None),
            Err(error) => return Err(error),
        };
        // Where the link saw it, when the drive shows that folder and it still holds the photo; else the photo alone,
        // in its own place, as the protocol asks of a receiver without that view.
        if let Some(folder) = within.as_deref() {
            if let Some(folder) = self.locate_folder(folder).await? {
                if let Some(key) = self.find_asset(folder, &asset).await? {
                    return Ok(Some(key));
                }
            }
        }
        match self.find_asset(NodeKey::Section(SectionKey::Photos), &asset).await? {
            Some(key) => Ok(Some(key)),
            None => self.find_asset(NodeKey::Section(SectionKey::Archive), &asset).await,
        }
    }

    /// The key of a folder a link names, when this mount lists it.
    async fn locate_folder(&self, item: &ImmichItem) -> Result<Option<NodeKey>, SourceError> {
        let key = match (item.folder_key(), item) {
            (Some(key), _) => key,
            // Immich's Places page names a city alone; the first country listing it is taken.
            (None, ImmichItem::City { country: None, city }) => {
                let mut found = None;
                for country in self.source.countries().await? {
                    if self.source.cities(&country).await?.iter().any(|listed| listed == city) {
                        found = Some(NodeKey::City { country, city: city.clone() });
                        break;
                    }
                }
                match found {
                    Some(key) => key,
                    None => return Ok(None),
                }
            }
            (None, _) => return Ok(None),
        };
        Ok(self.entry_of(&key).await?.map(|_| key))
    }

    /// The Asset's key inside `folder` (a section, an album, a person, a city), going down the date folders it falls
    /// in; `None` when the folder is not shown, or does not hold it.
    async fn find_asset(&self, folder: NodeKey, asset: &Asset) -> Result<Option<NodeKey>, SourceError> {
        if self.entry_of(&folder).await?.is_none() {
            return Ok(None);
        }
        let mut folder = folder;
        loop {
            let entries = match self.list(&folder).await {
                Ok(entries) => entries,
                Err(SourceError::NotFound) => return Ok(None),
                Err(error) => return Err(error),
            };
            let target = NodeKey::Asset { parent: Box::new(folder.clone()), asset_id: asset.id.clone() };
            if entries.iter().any(|entry| entry.node == target) {
                return Ok(Some(target));
            }
            match entries.into_iter().find(|entry| holds_date_of(&entry.node, asset)) {
                Some(entry) => folder = entry.node,
                None => return Ok(None),
            }
        }
    }

    /// The names from the root down to `node`, as the file manager shows them; `None` when a folder on the way no
    /// longer lists the next.
    pub async fn path_of(&self, node: &NodeKey) -> Result<Option<Vec<String>>, SourceError> {
        let mut names = Vec::new();
        let mut current = node.clone();
        while current != NodeKey::Root {
            match self.entry_of(&current).await? {
                Some((parent, entry)) => {
                    names.push(entry.name);
                    current = parent;
                }
                None => return Ok(None),
            }
        }
        names.reverse();
        Ok(Some(names))
    }

    /// The key of the entry at `names` below the root, one folder or file name each; `None` when a name is not
    /// there. Each folder is looked up in the listing at hand however old, as the file manager that asks has just
    /// shown it, and read from the source only when there is none. Names are kept per folder, so an old listing still
    /// gives the item the name means.
    pub async fn key_at(&self, names: &[&str]) -> Result<Option<NodeKey>, SourceError> {
        let mut node = NodeKey::Root;
        for name in names {
            let entries = match self.stale_listing(&node) {
                Some(entries) => entries,
                None => self.list(&node).await?,
            };
            match entries.into_iter().find(|entry| entry.name == *name) {
                Some(entry) => node = entry.node,
                None => return Ok(None),
            }
        }
        Ok(Some(node))
    }

    async fn list_uncached(&self, node: &NodeKey) -> Result<Vec<Entry>, SourceError> {
        match node {
            NodeKey::Root => Ok(self.root_entries()),
            NodeKey::Upload | NodeKey::UploadFolder(_) => Ok(Vec::new()),
            NodeKey::Section(SectionKey::Photos) => {
                let years: BTreeSet<i32> = self.source.timeline_months().await?.iter().map(|(month, _)| month.year).collect();
                Ok(years.into_iter().map(|year| folder(year_folder_name(year), NodeKey::TimelineYear(year))).collect())
            }
            NodeKey::TimelineYear(year) => {
                let months = self.source.timeline_months().await?;
                Ok(months
                    .iter()
                    .filter(|(month, _)| month.year == *year)
                    .filter_map(|(month, _)| Some(folder(month_folder_name(month.month)?, NodeKey::TimelineMonth(*month))))
                    .collect())
            }
            NodeKey::TimelineMonth(month) => {
                let assets = self.source.month_assets(*month).await?;
                if self.config.grouping_photos.splits(assets.len()) {
                    let days: BTreeSet<u32> = assets.iter().map(Asset::day).collect();
                    Ok(days.into_iter().map(|day| folder(day_folder_name(day), NodeKey::TimelineDay(*month, day))).collect())
                } else {
                    Ok(self.file_entries(node, &assets))
                }
            }
            NodeKey::TimelineDay(month, day) => {
                let assets: Vec<Asset> = self.source.month_assets(*month).await?.into_iter().filter(|asset| asset.day() == *day).collect();
                Ok(self.file_entries(node, &assets))
            }
            NodeKey::Section(SectionKey::Albums) => {
                let albums = self.source.albums().await?;
                match self.config.album_layout {
                    AlbumLayout::YearFolders => {
                        let years: BTreeSet<Option<i32>> = albums.iter().map(|album| album.year).collect();
                        Ok(years.into_iter().map(|year| folder(album_year_folder(year), NodeKey::AlbumYear(year))).collect())
                    }
                    _ => Ok(self.album_entries(node, &albums)),
                }
            }
            NodeKey::AlbumYear(year) => {
                let albums: Vec<Album> = self.source.albums().await?.into_iter().filter(|album| album.year == *year).collect();
                Ok(self.album_entries(node, &albums))
            }
            NodeKey::Section(SectionKey::People) => {
                let people = self.source.people().await?;
                let requests: Vec<NameRequest> = people
                    .iter()
                    .filter(|person| !person.is_hidden && !person.name.trim().is_empty())
                    .map(|person| folder_request(&person.id, &person.name, person.created_at))
                    .collect();
                Ok(self.named_folders(node, &requests, |id| NodeKey::Person(id.to_string())))
            }
            NodeKey::Section(SectionKey::Places) => {
                let countries = self.source.countries().await?;
                let requests: Vec<NameRequest> = countries.iter().map(|country| folder_request(country, country, DateTime::<Utc>::UNIX_EPOCH)).collect();
                Ok(self.named_folders(node, &requests, |country| NodeKey::Country(country.to_string())))
            }
            NodeKey::Country(country) => {
                let cities = self.source.cities(country).await?;
                let requests: Vec<NameRequest> = cities.iter().map(|city| folder_request(city, city, DateTime::<Utc>::UNIX_EPOCH)).collect();
                Ok(self.named_folders(node, &requests, |city| NodeKey::City { country: country.clone(), city: city.to_string() }))
            }
            NodeKey::Album(_)
            | NodeKey::Person(_)
            | NodeKey::City { .. }
            | NodeKey::Section(SectionKey::Favorites | SectionKey::Archive) => {
                let assets = self.assets_of(node).await?;
                if self.grouping_for(node).splits(assets.len()) {
                    let years: BTreeSet<i32> = assets.iter().map(Asset::year).collect();
                    Ok(years
                        .into_iter()
                        .map(|year| folder(year_folder_name(year), NodeKey::GroupYear { base: Box::new(node.clone()), year }))
                        .collect())
                } else {
                    Ok(self.file_entries(node, &assets))
                }
            }
            NodeKey::GroupYear { base, year } => {
                let months: BTreeSet<u32> = self.assets_of(base).await?.iter().filter(|asset| asset.year() == *year).map(Asset::month).collect();
                Ok(months
                    .into_iter()
                    .filter_map(|month| {
                        let key = NodeKey::GroupMonth { base: base.clone(), month: YearMonth { year: *year, month } };
                        Some(folder(month_folder_name(month)?, key))
                    })
                    .collect())
            }
            NodeKey::GroupMonth { base, month } => {
                let assets: Vec<Asset> = self
                    .assets_of(base)
                    .await?
                    .iter()
                    .filter(|asset| asset.year() == month.year && asset.month() == month.month)
                    .cloned()
                    .collect();
                Ok(self.file_entries(node, &assets))
            }
            NodeKey::Asset { .. } => Err(SourceError::NotFound),
        }
    }

    fn root_entries(&self) -> Vec<Entry> {
        let shown = [
            (Section::Photos, self.config.show_photos),
            (Section::Albums, self.config.show_albums),
            (Section::People, self.config.show_people),
            (Section::Places, self.config.show_places),
            (Section::Favorites, self.config.show_favorites),
            (Section::Archive, self.config.show_archive),
        ];
        let sections = shown
            .into_iter()
            .filter(|(_, is_shown)| *is_shown)
            .map(|(section, _)| folder(section.folder_name().to_string(), NodeKey::Section(section.into())));
        let upload = self.config.uploads_enabled.then(|| folder(crate::layout::UPLOAD_FOLDER.to_string(), NodeKey::Upload));
        sections.chain(upload).collect()
    }

    fn grouping_for(&self, node: &NodeKey) -> GroupingMode {
        match node {
            NodeKey::Album(_) => self.config.grouping_albums,
            NodeKey::Person(_) => self.config.grouping_people,
            NodeKey::Section(SectionKey::Favorites) => self.config.grouping_favorites,
            NodeKey::Section(SectionKey::Archive) => self.config.grouping_archive,
            _ => self.config.grouping_places,
        }
    }

    /// The Assets of an album, person, city, the favorites or the archive. Kept while fresh, so its year and month
    /// folders do not fetch the whole list again.
    async fn assets_of(&self, base: &NodeKey) -> Result<Arc<Vec<Asset>>, SourceError> {
        if let Some((fetched_at, assets)) = self.base_assets.lock().expect("base asset lock").get(base) {
            if fetched_at.elapsed() < freshness_window(base, Utc::now()) {
                return Ok(assets.clone());
            }
        }
        let assets = Arc::new(match base {
            NodeKey::Album(album_id) => self.source.album_assets(album_id).await?,
            NodeKey::Person(person_id) => self.source.person_assets(person_id).await?,
            NodeKey::City { country, city } => self.source.city_assets(country, city).await?,
            NodeKey::Section(SectionKey::Favorites) => self.source.favorite_assets().await?,
            NodeKey::Section(SectionKey::Archive) => self.source.archive_assets().await?,
            _ => return Err(SourceError::NotFound),
        });
        self.base_assets.lock().expect("base asset lock").insert(base.clone(), (Instant::now(), assets.clone()));
        Ok(assets)
    }

    fn album_entries(&self, folder_node: &NodeKey, albums: &[Album]) -> Vec<Entry> {
        let layout = self.config.album_layout;
        let requests: Vec<NameRequest> = albums
            .iter()
            .map(|album| NameRequest {
                key: album.id.clone(),
                base: album_folder_base(layout, &album.name, album.year),
                created_at: album.created_at,
                is_file: false,
            })
            .collect();
        self.named_folders(folder_node, &requests, |album_id| NodeKey::Album(album_id.to_string()))
    }

    fn named_folders(&self, folder_node: &NodeKey, requests: &[NameRequest], node_for: impl Fn(&str) -> NodeKey) -> Vec<Entry> {
        let names = self.assign_and_remember(folder_node, requests);
        requests
            .iter()
            .map(|request| folder(names[&request.key].clone(), node_for(&request.key)))
            .collect()
    }

    fn file_entries(&self, folder_node: &NodeKey, assets: &[Asset]) -> Vec<Entry> {
        let requests: Vec<NameRequest> = assets
            .iter()
            .map(|asset| NameRequest {
                key: asset.id.clone(),
                base: file_base_name(self.config.file_naming, &asset.original_file_name, asset.local_capture_time),
                created_at: asset.uploaded_at,
                is_file: true,
            })
            .collect();
        let names = self.assign_and_remember(folder_node, &requests);
        assets
            .iter()
            .map(|asset| Entry {
                name: names[&asset.id].clone(),
                node: NodeKey::Asset { parent: Box::new(folder_node.clone()), asset_id: asset.id.clone() },
                kind: EntryKind::File { asset_id: asset.id.clone(), size: asset.size, modified: asset.captured_at },
            })
            .collect()
    }

    fn assign_and_remember(&self, folder_node: &NodeKey, requests: &[NameRequest]) -> HashMap<String, String> {
        let remembered = self.names.remembered(folder_node);
        let names = assign_names(requests, &remembered);
        if names != remembered {
            self.names.remember(folder_node, &names);
        }
        names
    }
}

/// Whether a date folder holds the Asset's date: its year, month or day, in the timeline or in an album, person,
/// place or section.
fn holds_date_of(node: &NodeKey, asset: &Asset) -> bool {
    let month = YearMonth { year: asset.year(), month: asset.month() };
    match node {
        NodeKey::TimelineYear(year) | NodeKey::GroupYear { year, .. } => *year == asset.year(),
        NodeKey::TimelineMonth(folder_month) | NodeKey::GroupMonth { month: folder_month, .. } => *folder_month == month,
        NodeKey::TimelineDay(folder_month, day) => *folder_month == month && *day == asset.day(),
        _ => false,
    }
}

fn folder(name: String, node: NodeKey) -> Entry {
    Entry { name, node, kind: EntryKind::Folder }
}

fn folder_request(key: &str, name: &str, created_at: DateTime<Utc>) -> NameRequest {
    NameRequest { key: key.to_string(), base: sanitise(name, false), created_at, is_file: false }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Person;
    use chrono::{NaiveDate, TimeZone};

    /// A small library held in memory.
    #[derive(Default)]
    struct FakeLibrary {
        assets: Vec<Asset>,
        albums: Vec<(Album, Vec<String>)>,
        people: Vec<(Person, Vec<String>)>,
        archived: Vec<String>,
        favorites: Vec<String>,
        album_fetches: std::sync::atomic::AtomicUsize,
    }

    impl FakeLibrary {
        fn find(&self, ids: &[String]) -> Vec<Asset> {
            self.assets.iter().filter(|asset| ids.contains(&asset.id)).cloned().collect()
        }
    }

    impl LibrarySource for FakeLibrary {
        async fn timeline_months(&self) -> Result<Vec<(YearMonth, u32)>, SourceError> {
            let mut counts: HashMap<YearMonth, u32> = HashMap::new();
            for asset in &self.assets {
                *counts.entry(YearMonth { year: asset.year(), month: asset.month() }).or_default() += 1;
            }
            Ok(counts.into_iter().collect())
        }
        async fn month_assets(&self, month: YearMonth) -> Result<Vec<Asset>, SourceError> {
            // The timeline leaves archived Assets out, as Immich's does.
            Ok(self
                .assets
                .iter()
                .filter(|asset| asset.year() == month.year && asset.month() == month.month && !self.archived.contains(&asset.id))
                .cloned()
                .collect())
        }
        async fn albums(&self) -> Result<Vec<Album>, SourceError> {
            Ok(self.albums.iter().map(|(album, _)| album.clone()).collect())
        }
        async fn album_assets(&self, album_id: &str) -> Result<Vec<Asset>, SourceError> {
            self.album_fetches.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let (_, ids) = self.albums.iter().find(|(album, _)| album.id == album_id).ok_or(SourceError::NotFound)?;
            Ok(self.find(ids))
        }
        async fn people(&self) -> Result<Vec<Person>, SourceError> {
            Ok(self.people.iter().map(|(person, _)| person.clone()).collect())
        }
        async fn person_assets(&self, person_id: &str) -> Result<Vec<Asset>, SourceError> {
            let (_, ids) = self.people.iter().find(|(person, _)| person.id == person_id).ok_or(SourceError::NotFound)?;
            Ok(self.find(ids))
        }
        async fn countries(&self) -> Result<Vec<String>, SourceError> {
            Ok(vec!["Denmark".into()])
        }
        async fn cities(&self, _country: &str) -> Result<Vec<String>, SourceError> {
            Ok(vec!["Odense".into()])
        }
        async fn city_assets(&self, _country: &str, city: &str) -> Result<Vec<Asset>, SourceError> {
            Ok(self.assets.iter().filter(|asset| asset.city.as_deref() == Some(city)).cloned().collect())
        }
        async fn archive_assets(&self) -> Result<Vec<Asset>, SourceError> {
            Ok(self.find(&self.archived))
        }
        async fn favorite_assets(&self) -> Result<Vec<Asset>, SourceError> {
            Ok(self.find(&self.favorites))
        }
        async fn asset(&self, asset_id: &str) -> Result<Asset, SourceError> {
            self.assets.iter().find(|asset| asset.id == asset_id).cloned().ok_or(SourceError::NotFound)
        }
    }

    fn asset(id: &str, original: &str, day: u32, second: u32, uploaded_minute: u32) -> Asset {
        let local = NaiveDate::from_ymd_opt(2024, 3, day).unwrap().and_hms_opt(14, 32, second).unwrap();
        Asset {
            id: id.into(),
            original_file_name: original.into(),
            local_capture_time: local,
            captured_at: Utc.from_utc_datetime(&local),
            uploaded_at: Utc.with_ymd_and_hms(2024, 4, 1, 12, uploaded_minute, 0).unwrap(),
            size: Some(1000),
            city: Some("Odense".into()),
            country: Some("Denmark".into()),
        }
    }

    fn tree(library: FakeLibrary, config: Config) -> Tree<FakeLibrary, InMemoryNames> {
        Tree::new(library, InMemoryNames::default(), config)
    }

    fn names(entries: &[Entry]) -> Vec<String> {
        let mut names: Vec<String> = entries.iter().map(|entry| entry.name.clone()).collect();
        names.sort();
        names
    }

    #[test]
    fn changes_touch_only_the_folders_that_depend_on_them() {
        let march = YearMonth { year: 2024, month: 3 };
        let april = YearMonth { year: 2024, month: 4 };
        let changes = LibraryChanges { months: vec![march], albums: vec!["trip".into()], ..Default::default() };
        let touched = |node: NodeKey| is_touched(&node, &changes);
        assert!(touched(NodeKey::TimelineMonth(march)) && touched(NodeKey::TimelineDay(march, 15)));
        assert!(touched(NodeKey::TimelineYear(2024)), "the year shows the month's file count");
        assert!(!touched(NodeKey::TimelineMonth(april)) && !touched(NodeKey::TimelineYear(2023)));
        assert!(!touched(NodeKey::Section(SectionKey::Photos)), "no month came or went");
        assert!(touched(NodeKey::Album("trip".into())));
        assert!(touched(NodeKey::GroupYear { base: Box::new(NodeKey::Album("trip".into())), year: 2024 }));
        assert!(touched(NodeKey::Section(SectionKey::Albums)), "an album's year can move with its newest photo");
        assert!(!touched(NodeKey::Album("other".into())) && !touched(NodeKey::Person("p".into())));
        assert!(!touched(NodeKey::Section(SectionKey::Archive)), "a month's count says nothing about the archive");
        let archive_month = NodeKey::GroupMonth { base: Box::new(NodeKey::Section(SectionKey::Archive)), month: march };
        let archived = LibraryChanges { archive_changed: true, ..Default::default() };
        assert!(is_touched(&archive_month, &archived) && !is_touched(&NodeKey::TimelineMonth(march), &archived));
        let favorited = LibraryChanges { favorites_changed: true, ..Default::default() };
        assert!(is_touched(&NodeKey::Section(SectionKey::Favorites), &favorited) && !is_touched(&archive_month, &favorited));
    }

    #[tokio::test]
    async fn a_path_of_names_gives_the_key_it_shows() {
        let library = FakeLibrary { assets: vec![asset("a", "IMG_1.JPG", 15, 7, 1)], ..Default::default() };
        let tree = tree(library, Config::from_toml("").unwrap());
        assert_eq!(tree.key_at(&[]).await.unwrap(), Some(NodeKey::Root));
        let year = tree.list(&NodeKey::Section(SectionKey::Photos)).await.unwrap().remove(0);
        let month = tree.list(&year.node).await.unwrap().remove(0);
        let file = tree.list(&month.node).await.unwrap().remove(0);
        let names = ["Photos", year.name.as_str(), month.name.as_str(), file.name.as_str()];
        assert_eq!(tree.key_at(&names).await.unwrap(), Some(file.node));
        assert_eq!(tree.key_at(&["Photos", year.name.as_str(), "nothing here"]).await.unwrap(), None);
        assert_eq!(tree.key_at(&["Places", "Denmark", "Odense"]).await.unwrap(), Some(NodeKey::City { country: "Denmark".into(), city: "Odense".into() }));
    }

    fn linked_library() -> FakeLibrary {
        let created = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
        FakeLibrary {
            assets: vec![asset("a", "IMG_1.JPG", 15, 7, 1), asset("b", "IMG_2.JPG", 16, 8, 2)],
            albums: vec![(Album { id: "trip".into(), name: "Trip".into(), year: Some(2024), created_at: created }, vec!["a".into()])],
            people: vec![
                (Person { id: "ann".into(), name: "Ann".into(), is_hidden: false, created_at: created }, vec!["b".into()]),
                (Person { id: "hid".into(), name: "Hidden".into(), is_hidden: true, created_at: created }, vec!["b".into()]),
            ],
            archived: vec!["b".into()],
            ..Default::default()
        }
    }

    fn photo(asset_id: &str, within: Option<ImmichItem>) -> ImmichItem {
        ImmichItem::Asset { within: within.map(Box::new), asset_id: asset_id.into() }
    }

    #[tokio::test]
    async fn a_link_finds_its_item_where_the_mount_shows_it() {
        let mut config = Config::from_toml("").unwrap();
        config.grouping_albums = GroupingMode::Always;
        let tree = tree(linked_library(), config);
        let march = YearMonth { year: 2024, month: 3 };
        let in_album = tree.locate(&photo("a", Some(ImmichItem::Album("trip".into())))).await.unwrap().unwrap();
        let album_month = NodeKey::GroupMonth { base: Box::new(NodeKey::Album("trip".into())), month: march };
        assert_eq!(in_album, NodeKey::Asset { parent: Box::new(album_month), asset_id: "a".into() }, "down the album's date folders");
        let names = tree.path_of(&in_album).await.unwrap().unwrap();
        assert_eq!(&names[..3], ["Albums", "2024 - Trip", "2024"]);
        assert_eq!(tree.key_at(&names.iter().map(String::as_str).collect::<Vec<_>>()).await.unwrap(), Some(in_album), "the path leads back");
        let in_timeline = tree.locate(&photo("a", None)).await.unwrap().unwrap();
        assert_eq!(in_timeline, NodeKey::Asset { parent: Box::new(NodeKey::TimelineMonth(march)), asset_id: "a".into() });
        assert_eq!(
            tree.locate(&photo("b", Some(ImmichItem::Person("ann".into())))).await.unwrap(),
            Some(NodeKey::Asset { parent: Box::new(NodeKey::Person("ann".into())), asset_id: "b".into() })
        );
        let odense = ImmichItem::City { country: None, city: "Odense".into() };
        assert_eq!(tree.locate(&odense).await.unwrap(), Some(NodeKey::City { country: "Denmark".into(), city: "Odense".into() }), "a city by its name alone");
        assert_eq!(tree.path_of(&NodeKey::Section(SectionKey::Places)).await.unwrap(), Some(vec!["Places".to_owned()]));
    }

    #[tokio::test]
    async fn a_photo_of_the_timeline_lands_in_its_day_folder_where_months_are_split() {
        let mut config = Config::from_toml("").unwrap();
        config.grouping_photos = GroupingMode::Always;
        let tree = tree(linked_library(), config);
        let found = tree.locate(&photo("a", None)).await.unwrap().unwrap();
        let day = NodeKey::TimelineDay(YearMonth { year: 2024, month: 3 }, 15);
        assert_eq!(found, NodeKey::Asset { parent: Box::new(day), asset_id: "a".into() });
        let names = tree.path_of(&found).await.unwrap().unwrap();
        assert_eq!(names.len(), 5, "Photos, the year, the month, the day, the file: {names:?}");
    }

    #[tokio::test]
    async fn a_link_to_what_the_mount_does_not_show_finds_nothing() {
        let tree = tree(linked_library(), Config::from_toml("").unwrap());
        let march = YearMonth { year: 2024, month: 3 };
        assert_eq!(
            tree.locate(&photo("a", Some(ImmichItem::Person("ann".into())))).await.unwrap(),
            Some(NodeKey::Asset { parent: Box::new(NodeKey::TimelineMonth(march)), asset_id: "a".into() }),
            "a person without the photo: the photo in its own place"
        );
        assert_eq!(
            tree.locate(&photo("a", Some(ImmichItem::Album("gone".into())))).await.unwrap(),
            Some(NodeKey::Asset { parent: Box::new(NodeKey::TimelineMonth(march)), asset_id: "a".into() }),
            "an album the drive does not show: likewise"
        );
        let nothing = [
            photo("missing", None),
            ImmichItem::Person("hid".into()),
            ImmichItem::Album("gone".into()),
            ImmichItem::Section(SectionKey::Archive),
            photo("b", Some(ImmichItem::Section(SectionKey::Archive))),
            ImmichItem::City { country: Some("Denmark".into()), city: "Aarhus".into() },
        ];
        for item in nothing {
            assert_eq!(tree.locate(&item).await.unwrap(), None, "{item:?}");
        }
        let mut config = Config::from_toml("").unwrap();
        config.show_archive = true;
        let with_archive = super::tests::tree(linked_library(), config);
        assert_eq!(
            with_archive.locate(&photo("b", Some(ImmichItem::Section(SectionKey::Archive)))).await.unwrap(),
            Some(NodeKey::Asset {
                parent: Box::new(NodeKey::GroupMonth { base: Box::new(NodeKey::Section(SectionKey::Archive)), month: YearMonth { year: 2024, month: 3 } }),
                asset_id: "b".into()
            }),
            "shown once the section is on, in its date folders"
        );
    }

    #[tokio::test]
    async fn applying_changes_forgets_the_touched_listings() {
        let library = FakeLibrary { assets: vec![asset("a", "IMG_1.JPG", 15, 7, 1)], ..Default::default() };
        let tree = tree(library, Config::from_toml("").unwrap());
        let month = NodeKey::TimelineMonth(YearMonth { year: 2024, month: 3 });
        tree.list(&NodeKey::Root).await.unwrap();
        tree.list(&month).await.unwrap();
        assert_eq!(tree.apply_changes(&LibraryChanges::default()), 0);
        let changes = LibraryChanges { months: vec![YearMonth { year: 2024, month: 3 }], ..Default::default() };
        assert_eq!(tree.apply_changes(&changes), 1, "the month goes; the root stays");
    }

    #[tokio::test]
    async fn an_expired_listing_is_offered_but_a_changed_one_is_not() {
        let library = FakeLibrary { assets: vec![asset("a", "IMG_1.JPG", 15, 7, 1)], ..Default::default() };
        let tree = tree(library, Config::from_toml("").unwrap());
        let month = NodeKey::TimelineMonth(YearMonth { year: 2024, month: 3 });
        tree.list(&month).await.unwrap();
        assert_eq!(tree.expired_listing(&month), None, "still fresh");
        let age = |tree: &Tree<FakeLibrary, InMemoryNames>| {
            if let Some((fetched_at, _)) = tree.listings.lock().unwrap().get_mut(&month) {
                *fetched_at = Instant::now().checked_sub(Duration::from_secs(2 * 3600)).unwrap();
            }
        };
        age(&tree);
        assert_eq!(tree.expired_listing(&month).map(|entries| entries.len()), Some(1));
        tree.apply_changes(&LibraryChanges { months: vec![YearMonth { year: 2024, month: 3 }], ..Default::default() });
        assert_eq!(tree.expired_listing(&month), None, "a known change is fetched before answering");
    }

    #[tokio::test]
    async fn root_lists_enabled_sections_with_immichs_labels() {
        let mut config = Config::from_toml("").unwrap();
        config.show_places = false;
        let entries = tree(FakeLibrary::default(), config.clone()).list(&NodeKey::Root).await.unwrap();
        assert_eq!(names(&entries), ["Albums", "People", "Photos", "Upload"]);
        config.uploads_enabled = false;
        let entries = tree(FakeLibrary::default(), config.clone()).list(&NodeKey::Root).await.unwrap();
        assert_eq!(names(&entries), ["Albums", "People", "Photos"], "without uploads, no Upload folder");
        config.show_archive = true;
        let entries = tree(FakeLibrary::default(), config.clone()).list(&NodeKey::Root).await.unwrap();
        assert_eq!(names(&entries), ["Albums", "Archive", "People", "Photos"], "the archive when asked for");
        assert!(tree(FakeLibrary::default(), config.clone()).watched().archive, "a shown archive is watched");
        config.show_favorites = true;
        let entries = tree(FakeLibrary::default(), config.clone()).list(&NodeKey::Root).await.unwrap();
        assert_eq!(names(&entries), ["Albums", "Archive", "Favorites", "People", "Photos"]);
        assert_eq!(tree(FakeLibrary::default(), config).watched(), Watched { archive: true, favorites: true });
    }

    #[tokio::test]
    async fn the_archive_goes_year_month_files_by_default() {
        let library = FakeLibrary {
            assets: vec![asset("a", "a.jpg", 15, 1, 1), asset("b", "b.jpg", 16, 1, 2)],
            archived: vec!["b".into()],
            ..Default::default()
        };
        let tree = tree(library, Config::from_toml("[sections]\narchive = true\n").unwrap());
        let years = tree.list(&NodeKey::Section(SectionKey::Archive)).await.unwrap();
        assert_eq!(names(&years), ["2024"], "split from the first archived photo");
        let months = tree.list(&years[0].node).await.unwrap();
        assert_eq!(names(&months), ["03 March"]);
        assert_eq!(names(&tree.list(&months[0].node).await.unwrap()), ["2024-03-16 14.32.01.jpg"], "only what is archived");
        assert_eq!(tree.parent_of(&years[0].node).await.unwrap(), Some(NodeKey::Section(SectionKey::Archive)));
    }

    #[tokio::test]
    async fn favorites_go_year_month_files_by_default() {
        let library = FakeLibrary {
            assets: vec![asset("a", "a.jpg", 15, 1, 1), asset("b", "b.jpg", 16, 1, 2)],
            favorites: vec!["a".into()],
            ..Default::default()
        };
        let tree = tree(library, Config::from_toml("[sections]\nfavorites = true\n").unwrap());
        let years = tree.list(&NodeKey::Section(SectionKey::Favorites)).await.unwrap();
        assert_eq!(names(&years), ["2024"]);
        let months = tree.list(&years[0].node).await.unwrap();
        assert_eq!(names(&tree.list(&months[0].node).await.unwrap()), ["2024-03-15 14.32.01.jpg"], "only the favorites");
    }

    #[tokio::test]
    async fn the_timeline_goes_year_month_files_with_date_time_names() {
        let library = FakeLibrary { assets: vec![asset("a", "IMG_1.JPG", 15, 7, 1), asset("b", "IMG_2.JPG", 15, 7, 2)], ..Default::default() };
        let tree = tree(library, Config::from_toml("").unwrap());
        let years = tree.list(&NodeKey::Section(SectionKey::Photos)).await.unwrap();
        assert_eq!(names(&years), ["2024"]);
        let months = tree.list(&years[0].node).await.unwrap();
        assert_eq!(names(&months), ["03 March"]);
        let files = tree.list(&months[0].node).await.unwrap();
        assert_eq!(names(&files), ["2024-03-15 14.32.07-1.JPG", "2024-03-15 14.32.07.JPG"]);
    }

    #[tokio::test]
    async fn busy_months_split_into_days() {
        let library = FakeLibrary { assets: vec![asset("a", "a.jpg", 15, 1, 1), asset("b", "b.jpg", 16, 1, 2)], ..Default::default() };
        let mut config = Config::from_toml("").unwrap();
        config.grouping_photos = GroupingMode::Always;
        let tree = tree(library, config);
        let days = tree.list(&NodeKey::TimelineMonth(YearMonth { year: 2024, month: 3 })).await.unwrap();
        assert_eq!(names(&days), ["15", "16"]);
        let files = tree.list(&days[1].node).await.unwrap();
        assert_eq!(names(&files), ["2024-03-16 14.32.01.jpg"]);
    }

    #[tokio::test]
    async fn a_grouped_albums_folders_share_one_fetch_of_its_assets() {
        let created = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
        let library = FakeLibrary {
            assets: vec![asset("a", "IMG_1.JPG", 15, 7, 1)],
            albums: vec![(Album { id: "trip".into(), name: "Trip".into(), year: Some(2024), created_at: created }, vec!["a".into()])],
            ..Default::default()
        };
        let mut config = Config::from_toml("").unwrap();
        config.grouping_albums = crate::layout::GroupingMode::Always;
        let tree = Arc::new(tree(library, config));
        let album = NodeKey::Album("trip".into());
        // Two visits at once share one fetch.
        let (first, second) = tokio::join!(tree.list(&album), tree.list(&album));
        assert_eq!(names(&first.unwrap()), ["2024"]);
        assert_eq!(names(&second.unwrap()), ["2024"]);
        let year = NodeKey::GroupYear { base: Box::new(album.clone()), year: 2024 };
        let months = tree.list(&year).await.unwrap();
        tree.list(&months[0].node).await.unwrap();
        assert_eq!(tree.source.album_fetches.load(std::sync::atomic::Ordering::SeqCst), 1, "the year and month reuse the album's list");
    }

    #[tokio::test]
    async fn albums_use_the_year_prefix_and_resolve_clashes() {
        let created = |minute| Utc.with_ymd_and_hms(2024, 1, 1, 0, minute, 0).unwrap();
        let library = FakeLibrary {
            albums: vec![
                (Album { id: "x".into(), name: "Trip".into(), year: Some(2023), created_at: created(1) }, vec![]),
                (Album { id: "y".into(), name: "Trip".into(), year: Some(2023), created_at: created(2) }, vec![]),
                (Album { id: "z".into(), name: "Empty".into(), year: None, created_at: created(3) }, vec![]),
            ],
            ..Default::default()
        };
        let entries = tree(library, Config::from_toml("").unwrap()).list(&NodeKey::Section(SectionKey::Albums)).await.unwrap();
        assert_eq!(names(&entries), ["2023 - Trip", "2023 - Trip-1", "Empty"]);
    }

    #[tokio::test]
    async fn the_year_folders_layout_groups_albums_by_year() {
        let created = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
        let library = FakeLibrary {
            albums: vec![
                (Album { id: "x".into(), name: "Trip".into(), year: Some(2023), created_at: created }, vec![]),
                (Album { id: "z".into(), name: "Empty".into(), year: None, created_at: created }, vec![]),
            ],
            ..Default::default()
        };
        let mut config = Config::from_toml("").unwrap();
        config.album_layout = AlbumLayout::YearFolders;
        let tree = tree(library, config);
        let years = tree.list(&NodeKey::Section(SectionKey::Albums)).await.unwrap();
        assert_eq!(names(&years), ["2023", "Unknown Year"]);
        let inside = tree.list(&NodeKey::AlbumYear(Some(2023))).await.unwrap();
        assert_eq!(names(&inside), ["Trip"]);
    }

    #[tokio::test]
    async fn people_hide_hidden_and_unnamed_and_large_folders_get_date_groups() {
        let created = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
        let person = |id: &str, name: &str, hidden| Person { id: id.into(), name: name.into(), is_hidden: hidden, created_at: created };
        let library = FakeLibrary {
            assets: vec![asset("a", "a.jpg", 15, 1, 1)],
            people: vec![(person("p", "Anna", false), vec!["a".into()]), (person("h", "Hidden", true), vec![]), (person("u", " ", false), vec![])],
            ..Default::default()
        };
        let mut config = Config::from_toml("").unwrap();
        config.grouping_people = GroupingMode::Always;
        let tree = tree(library, config);
        let people = tree.list(&NodeKey::Section(SectionKey::People)).await.unwrap();
        assert_eq!(names(&people), ["Anna"]);
        let years = tree.list(&people[0].node).await.unwrap();
        assert_eq!(names(&years), ["2024"]);
        let months = tree.list(&years[0].node).await.unwrap();
        assert_eq!(names(&months), ["03 March"]);
        assert_eq!(names(&tree.list(&months[0].node).await.unwrap()), ["2024-03-15 14.32.01.jpg"]);
    }

    #[test]
    fn every_key_comes_back_from_its_storage_form() {
        let march = YearMonth { year: 2024, month: 3 };
        let album = NodeKey::Album("a|b%7C".into());
        let keys = [
            NodeKey::Root,
            NodeKey::Upload,
            NodeKey::UploadFolder(vec!["Trip".into()]),
            NodeKey::UploadFolder(vec!["Trip|2026".into(), "Day 1 %7C".into()]),
            NodeKey::Section(SectionKey::Photos),
            NodeKey::Section(SectionKey::Places),
            NodeKey::Section(SectionKey::Archive),
            NodeKey::Section(SectionKey::Favorites),
            NodeKey::GroupMonth { base: Box::new(NodeKey::Section(SectionKey::Archive)), month: march },
            NodeKey::TimelineYear(2024),
            NodeKey::TimelineMonth(march),
            NodeKey::TimelineDay(march, 5),
            NodeKey::AlbumYear(Some(2023)),
            NodeKey::AlbumYear(None),
            album.clone(),
            NodeKey::Person("p-1".into()),
            NodeKey::Country("Côte d'Ivoire".into()),
            NodeKey::City { country: "A|B".into(), city: "50%".into() },
            NodeKey::GroupYear { base: Box::new(album.clone()), year: 2024 },
            NodeKey::GroupMonth { base: Box::new(NodeKey::City { country: "x|y".into(), city: "%".into() }), month: march },
            NodeKey::Asset {
                parent: Box::new(NodeKey::GroupMonth { base: Box::new(album), month: march }),
                asset_id: "id|%".into(),
            },
        ];
        for key in keys {
            assert_eq!(NodeKey::from_storage_key(&key.storage_key()), Some(key.clone()), "{}", key.storage_key());
        }
    }

    #[test]
    fn text_no_key_writes_is_no_key() {
        for text in ["", "root|x", "month|2024-3", "month|2024", "day|2024-03", "year|+2024", "section|Trash", "album", "asset|root"] {
            assert_eq!(NodeKey::from_storage_key(text), None, "{text}");
        }
    }

    #[tokio::test]
    async fn an_entry_is_found_by_its_key_through_its_parent() {
        let created = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
        let library = FakeLibrary {
            assets: vec![asset("a", "IMG_1.JPG", 15, 7, 1)],
            albums: vec![(Album { id: "x".into(), name: "Trip".into(), year: Some(2023), created_at: created }, vec!["a".into()])],
            ..Default::default()
        };
        let mut config = Config::from_toml("").unwrap();
        config.album_layout = AlbumLayout::YearFolders;
        let tree = tree(library, config);
        let album = NodeKey::Album("x".into());
        assert_eq!(tree.parent_of(&album).await.unwrap(), Some(NodeKey::AlbumYear(Some(2023))));
        assert_eq!(tree.entry_of(&album).await.unwrap().map(|(_, entry)| entry.name), Some("Trip".into()));
        let photo = NodeKey::Asset { parent: Box::new(NodeKey::TimelineMonth(YearMonth { year: 2024, month: 3 })), asset_id: "a".into() };
        let (folder, entry) = tree.entry_of(&photo).await.unwrap().expect("the photo is in its month");
        assert_eq!(folder, NodeKey::TimelineMonth(YearMonth { year: 2024, month: 3 }));
        assert!(matches!(entry.kind, EntryKind::File { .. }));
        assert_eq!(tree.entry_of(&NodeKey::Root).await.unwrap(), None);
        assert_eq!(tree.entry_of(&NodeKey::Album("gone".into())).await.unwrap(), None);
        let elsewhere = NodeKey::Asset { parent: Box::new(NodeKey::TimelineMonth(YearMonth { year: 2023, month: 1 })), asset_id: "a".into() };
        assert_eq!(tree.entry_of(&elsewhere).await.unwrap(), None, "an Asset is found only where it appears");
    }

    #[tokio::test]
    async fn files_carry_size_and_capture_time() {
        let library = FakeLibrary { assets: vec![asset("a", "a.jpg", 15, 1, 1)], ..Default::default() };
        let tree = tree(library, Config::from_toml("").unwrap());
        let files = tree.list(&NodeKey::TimelineMonth(YearMonth { year: 2024, month: 3 })).await.unwrap();
        match &files[0].kind {
            EntryKind::File { size, modified, .. } => {
                assert_eq!(*size, Some(1000));
                assert_eq!(modified.to_rfc3339(), "2024-03-15T14:32:01+00:00");
            }
            EntryKind::Folder => panic!("expected a file"),
        }
    }
}
