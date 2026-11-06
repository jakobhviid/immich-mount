//! The metadata store: state that must survive restarts, in one redb database per account.
//!
//! It holds the names handed out per folder, so a path keeps meaning the same Asset across restarts (principle 8),
//! and each folder's latest listing. With the journal on (the macOS edition), it also records every change between a
//! folder's kept listing and its next one (ARCHITECTURE.md, "The macOS edition", "Changes"). redb is pure Rust and
//! crash-safe, so the core stays portable and statically buildable (ARCHITECTURE.md, "The metadata store").

use std::collections::{HashMap, HashSet};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::Path;

use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};

use crate::tree::{Entry, NameMemory, NodeKey};

/// Folder storage key to its assigned names, encoded as `entry-key<TAB>name` lines.
const NAMES: TableDefinition<&str, &str> = TableDefinition::new("names");
/// Each folder's latest listing, as JSON, for showing folders while Immich cannot be reached.
const LISTINGS: TableDefinition<&str, &[u8]> = TableDefinition::new("listings");
/// Format version of the store, for future migrations, and the journal's bookkeeping.
const META: TableDefinition<&str, u64> = TableDefinition::new("meta");
/// The change journal: sequence number to one `Change`, as JSON.
const JOURNAL: TableDefinition<u64, &[u8]> = TableDefinition::new("journal");
/// Small state a platform keeps between runs, by name: the change poll's ETags and counts.
const STATE: TableDefinition<&str, &str> = TableDefinition::new("state");
/// The store's identity in anchors, random, made when the journal is first used: a new store means new anchors.
const JOURNAL_EPOCH: &str = "journal-epoch";
/// The last sequence number handed out.
const JOURNAL_HEAD: &str = "journal-head";
/// The oldest sequence number still kept; anchors before it are expired.
const JOURNAL_FLOOR: &str = "journal-floor";
/// Nodes gone from the folder that listed them, whose deletion waits for `settle_departures`: a node that only moved
/// (an album whose year folder changed) shows in another folder's listing first, and is not deleted.
const DEPARTED: &str = "departed";
/// Folders to list again: touched by a change, or shown while one happened, until a listing of them is kept.
const RELIST: &str = "relist";
const STORE_FORMAT_VERSION: u64 = 1;

/// Why the store could not be opened.
#[derive(Debug, thiserror::Error)]
#[error("the metadata store at {path}: {message}")]
pub struct StoreError {
    pub path: String,
    pub message: String,
}

pub struct MetadataStore {
    database: Database,
    /// How many journal records are kept, when the journal is on.
    journal_limit: Option<u64>,
}

/// One record of the change journal.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Change {
    /// An entry appeared in `folder`, or what it shows (its name, size or time) changed.
    Updated { folder: NodeKey, entry: Entry },
    /// A node went away, from the folder that listed it; a folder's kept contents go with it.
    Deleted(NodeKey),
}

/// A point in the journal: the store's identity and a sequence number. Its text form is what the macOS edition hands
/// the system as a sync anchor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Anchor {
    pub epoch: u64,
    pub sequence: u64,
}

impl Anchor {
    pub fn to_text(self) -> String {
        format!("{:016x}:{}", self.epoch, self.sequence)
    }

    pub fn from_text(text: &str) -> Option<Anchor> {
        let (epoch, sequence) = text.split_once(':')?;
        Some(Anchor { epoch: u64::from_str_radix(epoch, 16).ok()?, sequence: sequence.parse().ok()? })
    }
}

/// What the journal holds after an anchor.
#[derive(Debug, Clone, PartialEq)]
pub enum JournalRead {
    /// The changes after the anchor, oldest first, up to the asked number; `anchor` is where they end, and `more`
    /// says whether others follow.
    Changes { changes: Vec<Change>, anchor: Anchor, more: bool },
    /// The anchor is from another store, or older than the oldest record kept: the reader starts again.
    Expired,
}

impl MetadataStore {
    /// Open the store at `path`, creating it if needed.
    pub fn open(path: &Path) -> Result<MetadataStore, StoreError> {
        let fail = |message: String| StoreError { path: path.display().to_string(), message };
        if let Some(parent) = path.parent() {
            std::fs::DirBuilder::new().recursive(true).mode(0o700).create(parent).map_err(|error| fail(error.to_string()))?;
        }
        let database = Database::create(path).map_err(|error| fail(error.to_string()))?;
        // The store holds names and ids from a private library: readable by its owner only.
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(|error| fail(error.to_string()))?;
        let store = MetadataStore { database, journal_limit: None };
        store.write_format_version().map_err(fail)?;
        Ok(store)
    }

    /// Turn the change journal on, keeping the latest `limit` records (the macOS edition; Linux does not read it).
    pub fn with_journal(mut self, limit: u64) -> Result<MetadataStore, StoreError> {
        let fail = |message: String| StoreError { path: "the metadata store".to_owned(), message };
        let transaction = self.database.begin_write().map_err(|error| fail(error.to_string()))?;
        {
            let mut meta = transaction.open_table(META).map_err(|error| fail(error.to_string()))?;
            if meta.get(JOURNAL_EPOCH).map_err(|error| fail(error.to_string()))?.is_none() {
                let mut bytes = [0u8; 8];
                getrandom::fill(&mut bytes).map_err(|error| fail(error.to_string()))?;
                meta.insert(JOURNAL_EPOCH, u64::from_le_bytes(bytes)).map_err(|error| fail(error.to_string()))?;
                meta.insert(JOURNAL_HEAD, 0).map_err(|error| fail(error.to_string()))?;
                meta.insert(JOURNAL_FLOOR, 1).map_err(|error| fail(error.to_string()))?;
            }
            transaction.open_table(JOURNAL).map_err(|error| fail(error.to_string()))?;
        }
        transaction.commit().map_err(|error| fail(error.to_string()))?;
        self.journal_limit = Some(limit.max(1));
        Ok(self)
    }

    /// Where the journal ends now.
    pub fn journal_anchor(&self) -> Result<Anchor, StoreError> {
        let read = || -> Result<Anchor, String> {
            let transaction = self.database.begin_read().map_err(|error| error.to_string())?;
            let meta = transaction.open_table(META).map_err(|error| error.to_string())?;
            let value = |key: &str| -> Result<u64, String> {
                Ok(meta.get(key).map_err(|error| error.to_string())?.map(|value| value.value()).unwrap_or_default())
            };
            Ok(Anchor { epoch: value(JOURNAL_EPOCH)?, sequence: value(JOURNAL_HEAD)? })
        };
        read().map_err(|message| StoreError { path: "the metadata store's journal".to_owned(), message })
    }

    /// The changes after `anchor`, at most `limit` of them.
    pub fn journal_since(&self, anchor: Anchor, limit: usize) -> Result<JournalRead, StoreError> {
        let read = || -> Result<JournalRead, String> {
            let transaction = self.database.begin_read().map_err(|error| error.to_string())?;
            let meta = transaction.open_table(META).map_err(|error| error.to_string())?;
            let value = |key: &str| -> Result<u64, String> {
                Ok(meta.get(key).map_err(|error| error.to_string())?.map(|value| value.value()).unwrap_or_default())
            };
            let (epoch, head, floor) = (value(JOURNAL_EPOCH)?, value(JOURNAL_HEAD)?, value(JOURNAL_FLOOR)?);
            if anchor.epoch != epoch || anchor.sequence > head || anchor.sequence + 1 < floor {
                return Ok(JournalRead::Expired);
            }
            let journal = transaction.open_table(JOURNAL).map_err(|error| error.to_string())?;
            let mut changes = Vec::new();
            let mut last = anchor.sequence;
            for record in journal.range(anchor.sequence + 1..).map_err(|error| error.to_string())?.take(limit) {
                let (sequence, encoded) = record.map_err(|error| error.to_string())?;
                changes.push(serde_json::from_slice(encoded.value()).map_err(|error| error.to_string())?);
                last = sequence.value();
            }
            Ok(JournalRead::Changes { changes, anchor: Anchor { epoch, sequence: last }, more: last < head })
        };
        read().map_err(|message| StoreError { path: "the metadata store's journal".to_owned(), message })
    }

    /// Change one piece of kept state in one write transaction, with the journal records the change brings (none
    /// with the journal off): nothing else changes the state or the journal between the read and the write. `change`
    /// gets the value kept, and gives the value to keep (`None` removes it), the records, and an answer.
    pub fn update_state<Answer>(
        &self,
        name: &str,
        change: impl FnOnce(Option<&str>) -> (Option<String>, Vec<Change>, Answer),
    ) -> Result<Answer, StoreError> {
        let write = || -> Result<Answer, String> {
            let transaction = self.database.begin_write().map_err(|error| error.to_string())?;
            let answer = {
                let mut table = transaction.open_table(STATE).map_err(|error| error.to_string())?;
                let kept = table.get(name).map_err(|error| error.to_string())?.map(|value| value.value().to_owned());
                let (value, changes, answer) = change(kept.as_deref());
                match value {
                    Some(value) => table.insert(name, value.as_str()).map(|_| ()),
                    None => table.remove(name).map(|_| ()),
                }
                .map_err(|error| error.to_string())?;
                if let Some(limit) = self.journal_limit {
                    append_changes(&transaction, &changes, limit)?;
                }
                answer
            };
            transaction.commit().map_err(|error| error.to_string())?;
            Ok(answer)
        };
        write().map_err(|message| StoreError { path: format!("the metadata store's {name}"), message })
    }

    /// Add folders to list again: a change touched them, or one happened while they were shown.
    pub fn mark_for_relisting(&self, folders: &[NodeKey]) -> Result<(), StoreError> {
        self.update_state(RELIST, |kept| {
            let mut keys = key_set(kept);
            keys.extend(folders.iter().map(NodeKey::storage_key));
            (Some(serde_json::to_string(&keys).unwrap_or_default()), Vec::new(), ())
        })
    }

    /// The folders waiting to be listed again.
    pub fn folders_to_relist(&self) -> Vec<NodeKey> {
        key_set(self.kept_state(RELIST).as_deref()).iter().filter_map(|key| NodeKey::from_storage_key(key)).collect()
    }

    /// A folder was listed again, or needs it no more.
    pub fn relisted(&self, folder: &NodeKey) -> Result<(), StoreError> {
        self.update_state(RELIST, |kept| {
            let mut keys = key_set(kept);
            keys.remove(&folder.storage_key());
            ((!keys.is_empty()).then(|| serde_json::to_string(&keys).unwrap_or_default()), Vec::new(), ())
        })
    }

    /// Record the deletion of every departed node no kept listing holds any more, and of everything kept below it,
    /// whose kept listings go. A node another folder lists now only moved: it keeps what was kept below it. Returns
    /// how many deletions were recorded.
    pub fn settle_departures(&self) -> Result<usize, StoreError> {
        let Some(limit) = self.journal_limit else { return Ok(0) };
        let settle = || -> Result<usize, String> {
            let transaction = self.database.begin_write().map_err(|error| error.to_string())?;
            let recorded = {
                let mut state = transaction.open_table(STATE).map_err(|error| error.to_string())?;
                let departed = key_set(state.get(DEPARTED).map_err(|error| error.to_string())?.as_ref().map(|value| value.value()));
                if departed.is_empty() {
                    return Ok(0);
                }
                state.remove(DEPARTED).map_err(|error| error.to_string())?;
                let mut listings = transaction.open_table(LISTINGS).map_err(|error| error.to_string())?;
                let mut listed = HashSet::new();
                for record in listings.iter().map_err(|error| error.to_string())? {
                    let (_, encoded) = record.map_err(|error| error.to_string())?;
                    let entries: Vec<Entry> = serde_json::from_slice(encoded.value()).unwrap_or_default();
                    listed.extend(entries.into_iter().map(|entry| entry.node));
                }
                let mut changes = Vec::new();
                for node in departed.iter().filter_map(|key| NodeKey::from_storage_key(key)) {
                    if !listed.contains(&node) {
                        forget_subtree(&mut listings, node, &mut changes)?;
                    }
                }
                append_changes(&transaction, &changes, limit)?;
                changes.len()
            };
            transaction.commit().map_err(|error| error.to_string())?;
            Ok(recorded)
        };
        settle().map_err(|message| StoreError { path: "the metadata store's journal".to_owned(), message })
    }

    /// Every folder with a kept listing: the folders a platform has been shown.
    pub fn kept_folders(&self) -> Vec<NodeKey> {
        let read = || -> Result<Vec<NodeKey>, String> {
            let transaction = self.database.begin_read().map_err(|error| error.to_string())?;
            let table = match transaction.open_table(LISTINGS) {
                Ok(table) => table,
                Err(redb::TableError::TableDoesNotExist(_)) => return Ok(Vec::new()),
                Err(error) => return Err(error.to_string()),
            };
            let mut folders = Vec::new();
            for record in table.iter().map_err(|error| error.to_string())? {
                let (key, _) = record.map_err(|error| error.to_string())?;
                folders.extend(NodeKey::from_storage_key(key.value()));
            }
            Ok(folders)
        };
        read().unwrap_or_else(|error| {
            log::warn!("could not read the kept folders: {error}");
            Vec::new()
        })
    }

    /// Small state kept between runs, by name.
    pub fn kept_state(&self, name: &str) -> Option<String> {
        let transaction = self.database.begin_read().ok()?;
        let table = transaction.open_table(STATE).ok()?;
        let stored = table.get(name).ok()??;
        Some(stored.value().to_owned())
    }

    /// Keep a piece of state, logging a failure: for state that a later run can do without.
    pub fn keep_state(&self, name: &str, value: &str) {
        let write = || -> Result<(), String> {
            let transaction = self.database.begin_write().map_err(|error| error.to_string())?;
            {
                let mut table = transaction.open_table(STATE).map_err(|error| error.to_string())?;
                table.insert(name, value).map_err(|error| error.to_string())?;
            }
            transaction.commit().map_err(|error| error.to_string())
        };
        if let Err(error) = write() {
            log::warn!("could not keep {name}: {error}");
        }
    }

    fn write_format_version(&self) -> Result<(), String> {
        let transaction = self.database.begin_write().map_err(|error| error.to_string())?;
        {
            let mut meta = transaction.open_table(META).map_err(|error| error.to_string())?;
            meta.insert("format", STORE_FORMAT_VERSION).map_err(|error| error.to_string())?;
        }
        transaction.commit().map_err(|error| error.to_string())
    }

    fn read_names(&self, folder: &NodeKey) -> Result<HashMap<String, String>, String> {
        let transaction = self.database.begin_read().map_err(|error| error.to_string())?;
        let table = match transaction.open_table(NAMES) {
            Ok(table) => table,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(HashMap::new()),
            Err(error) => return Err(error.to_string()),
        };
        let stored = table.get(folder.storage_key().as_str()).map_err(|error| error.to_string())?;
        Ok(stored.map(|value| decode_names(value.value())).unwrap_or_default())
    }

    fn write_names(&self, folder: &NodeKey, names: &HashMap<String, String>) -> Result<(), String> {
        let transaction = self.database.begin_write().map_err(|error| error.to_string())?;
        {
            let mut table = transaction.open_table(NAMES).map_err(|error| error.to_string())?;
            table.insert(folder.storage_key().as_str(), encode_names(names).as_str()).map_err(|error| error.to_string())?;
        }
        transaction.commit().map_err(|error| error.to_string())
    }
}

impl MetadataStore {
    fn read_listing(&self, folder: &NodeKey) -> Result<Option<Vec<Entry>>, String> {
        let transaction = self.database.begin_read().map_err(|error| error.to_string())?;
        let table = match transaction.open_table(LISTINGS) {
            Ok(table) => table,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(None),
            Err(error) => return Err(error.to_string()),
        };
        let stored = table.get(folder.storage_key().as_str()).map_err(|error| error.to_string())?;
        match stored {
            Some(value) => serde_json::from_slice(value.value()).map(Some).map_err(|error| error.to_string()),
            None => Ok(None),
        }
    }

    /// Keep a folder's listing and, with the journal on, record how it differs from the one kept before, in one
    /// transaction: a crash never keeps a listing whose changes were not recorded. What appeared or changed is
    /// recorded at once; what went away departs, to be settled once the change's folders were all listed again.
    fn write_listing(&self, folder: &NodeKey, entries: &[Entry]) -> Result<(), String> {
        let encoded = serde_json::to_vec(entries).map_err(|error| error.to_string())?;
        let transaction = self.database.begin_write().map_err(|error| error.to_string())?;
        {
            let mut listings = transaction.open_table(LISTINGS).map_err(|error| error.to_string())?;
            let previous = listings
                .insert(folder.storage_key().as_str(), encoded.as_slice())
                .map_err(|error| error.to_string())?
                .filter(|_| self.journal_limit.is_some())
                .map(|value| value.value().to_vec());
            if let (Some(limit), Some(previous)) = (self.journal_limit, previous) {
                let previous: Vec<Entry> = serde_json::from_slice(&previous).unwrap_or_default();
                let now: HashMap<&NodeKey, &Entry> = entries.iter().map(|entry| (&entry.node, entry)).collect();
                let gone: Vec<String> =
                    previous.iter().filter(|entry| !now.contains_key(&entry.node)).map(|entry| entry.node.storage_key()).collect();
                if !gone.is_empty() {
                    let mut state = transaction.open_table(STATE).map_err(|error| error.to_string())?;
                    let mut departed = key_set(state.get(DEPARTED).map_err(|error| error.to_string())?.as_ref().map(|value| value.value()));
                    departed.extend(gone);
                    let encoded = serde_json::to_string(&departed).map_err(|error| error.to_string())?;
                    state.insert(DEPARTED, encoded.as_str()).map_err(|error| error.to_string())?;
                }
                let before: HashMap<&NodeKey, &Entry> = previous.iter().map(|entry| (&entry.node, entry)).collect();
                let changes: Vec<Change> = entries
                    .iter()
                    .filter(|entry| before.get(&entry.node) != Some(entry))
                    .map(|entry| Change::Updated { folder: folder.clone(), entry: entry.clone() })
                    .collect();
                append_changes(&transaction, &changes, limit)?;
            }
        }
        transaction.commit().map_err(|error| error.to_string())
    }
}

/// A set of storage keys, kept as JSON.
fn key_set(kept: Option<&str>) -> std::collections::BTreeSet<String> {
    kept.and_then(|text| serde_json::from_str(text).ok()).unwrap_or_default()
}

/// A node gone from its folder: its deletion, and the deletion of everything kept below it, whose kept listings go.
fn forget_subtree(listings: &mut redb::Table<&str, &[u8]>, gone: NodeKey, changes: &mut Vec<Change>) -> Result<(), String> {
    let mut pending = vec![gone];
    let mut seen = HashSet::new();
    while let Some(node) = pending.pop() {
        if !seen.insert(node.clone()) {
            continue;
        }
        let kept = listings.remove(node.storage_key().as_str()).map_err(|error| error.to_string())?;
        if let Some(Ok(children)) = kept.map(|value| serde_json::from_slice::<Vec<Entry>>(value.value())) {
            pending.extend(children.into_iter().map(|child| child.node));
        }
        changes.push(Change::Deleted(node));
    }
    Ok(())
}

/// Number the changes after the journal's head, and trim the journal to its last `limit` records.
fn append_changes(transaction: &redb::WriteTransaction, changes: &[Change], limit: u64) -> Result<(), String> {
    if changes.is_empty() {
        return Ok(());
    }
    let mut meta = transaction.open_table(META).map_err(|error| error.to_string())?;
    let mut journal = transaction.open_table(JOURNAL).map_err(|error| error.to_string())?;
    let value = |meta: &redb::Table<&str, u64>, key: &str| -> Result<u64, String> {
        Ok(meta.get(key).map_err(|error| error.to_string())?.map(|value| value.value()).unwrap_or_default())
    };
    let mut head = value(&meta, JOURNAL_HEAD)?;
    for change in changes {
        head += 1;
        let encoded = serde_json::to_vec(change).map_err(|error| error.to_string())?;
        journal.insert(head, encoded.as_slice()).map_err(|error| error.to_string())?;
    }
    meta.insert(JOURNAL_HEAD, head).map_err(|error| error.to_string())?;
    let floor = value(&meta, JOURNAL_FLOOR)?.max(1);
    if head + 1 - floor > limit {
        let new_floor = head + 1 - limit;
        journal.retain_in(floor..new_floor, |_, _| false).map_err(|error| error.to_string())?;
        meta.insert(JOURNAL_FLOOR, new_floor).map_err(|error| error.to_string())?;
    }
    Ok(())
}

impl NameMemory for MetadataStore {
    fn keep_listing(&self, folder: &NodeKey, entries: &[Entry]) {
        if let Err(error) = self.write_listing(folder, entries) {
            log::warn!("could not keep the listing of {}: {error}", folder.storage_key());
        }
    }

    /// With the journal on, a listing shown is what the system holds, so it is kept even when a change came during
    /// its fetch, and the folder is listed again at the next poll. Without the journal it is not kept: a kept listing
    /// answers offline, and one a change may have outdated must not.
    fn shown_during_change(&self, folder: &NodeKey, entries: &[Entry]) {
        if self.journal_limit.is_none() {
            return;
        }
        self.keep_listing(folder, entries);
        if let Err(error) = self.mark_for_relisting(std::slice::from_ref(folder)) {
            log::warn!("could not mark {} to be listed again: {error}", folder.storage_key());
        }
    }

    fn kept_listing(&self, folder: &NodeKey) -> Option<Vec<Entry>> {
        self.read_listing(folder).unwrap_or_else(|error| {
            log::warn!("could not read the kept listing of {}: {error}", folder.storage_key());
            None
        })
    }

    fn remembered(&self, folder: &NodeKey) -> HashMap<String, String> {
        self.read_names(folder).unwrap_or_else(|error| {
            log::error!("could not read remembered names for {}: {error}", folder.storage_key());
            HashMap::new()
        })
    }

    fn remember(&self, folder: &NodeKey, names: &HashMap<String, String>) {
        if let Err(error) = self.write_names(folder, names) {
            log::error!("could not store names for {}: {error}", folder.storage_key());
        }
    }
}

fn encode_names(names: &HashMap<String, String>) -> String {
    let mut lines: Vec<String> = names.iter().map(|(key, name)| format!("{key}\t{name}")).collect();
    lines.sort();
    lines.join("\n")
}

fn decode_names(encoded: &str) -> HashMap<String, String> {
    encoded
        .lines()
        .filter_map(|line| line.split_once('\t'))
        .map(|(key, name)| (key.to_string(), name.to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::YearMonth;

    #[test]
    fn names_survive_reopening_the_store() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("metadata.redb");
        let folder = NodeKey::TimelineMonth(YearMonth { year: 2024, month: 3 });
        let names: HashMap<String, String> = [("a", "2024-03-15 14.32.07.jpg"), ("b", "2024-03-15 14.32.07-1.jpg")]
            .into_iter()
            .map(|(key, name)| (key.to_string(), name.to_string()))
            .collect();
        {
            let store = MetadataStore::open(&path).unwrap();
            assert!(store.remembered(&folder).is_empty());
            store.remember(&folder, &names);
        }
        let reopened = MetadataStore::open(&path).unwrap();
        assert_eq!(reopened.remembered(&folder), names);
    }

    #[test]
    fn storage_keys_escape_separators_in_free_text() {
        let city = NodeKey::City { country: "A|B".into(), city: "50%".into() };
        assert_eq!(city.storage_key(), "city|A%7CB|50%25");
        let nested = NodeKey::GroupYear { base: Box::new(NodeKey::Album("x".into())), year: 2024 };
        assert_eq!(nested.storage_key(), "group-year|album%7Cx|2024");
    }

    #[test]
    fn listings_survive_reopening_the_store() {
        use crate::tree::EntryKind;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("metadata.redb");
        let folder = NodeKey::TimelineMonth(YearMonth { year: 2024, month: 3 });
        let entries = vec![crate::tree::Entry {
            name: "2024-03-15 14.32.07.jpg".into(),
            node: NodeKey::Asset { parent: Box::new(folder.clone()), asset_id: "a".into() },
            kind: EntryKind::File { asset_id: "a".into(), size: Some(1000), modified: chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap() },
        }];
        MetadataStore::open(&path).unwrap().keep_listing(&folder, &entries);
        assert_eq!(MetadataStore::open(&path).unwrap().kept_listing(&folder), Some(entries));
        assert_eq!(MetadataStore::open(&path).unwrap().kept_listing(&NodeKey::Root), None);
    }

    fn photo(folder: &NodeKey, id: &str, name: &str) -> Entry {
        Entry {
            name: name.into(),
            node: NodeKey::Asset { parent: Box::new(folder.clone()), asset_id: id.into() },
            kind: crate::tree::EntryKind::File { asset_id: id.into(), size: Some(1000), modified: chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap() },
        }
    }

    fn folder_entry(name: &str, node: NodeKey) -> Entry {
        Entry { name: name.into(), node, kind: crate::tree::EntryKind::Folder }
    }

    fn changes_since(store: &MetadataStore, anchor: Anchor) -> Vec<Change> {
        match store.journal_since(anchor, 100).unwrap() {
            JournalRead::Changes { changes, .. } => changes,
            JournalRead::Expired => panic!("the anchor expired"),
        }
    }

    #[test]
    fn the_journal_records_what_appeared_changed_and_went_away() {
        let directory = tempfile::tempdir().unwrap();
        let store = MetadataStore::open(&directory.path().join("metadata.redb")).unwrap().with_journal(100).unwrap();
        let month = NodeKey::TimelineMonth(YearMonth { year: 2024, month: 3 });
        let start = store.journal_anchor().unwrap();
        store.keep_listing(&month, &[photo(&month, "a", "a.jpg"), photo(&month, "b", "b.jpg")]);
        assert_eq!(store.journal_anchor().unwrap(), start, "a first listing is what the system was shown: no changes");

        store.keep_listing(&month, &[photo(&month, "a", "a-1.jpg"), photo(&month, "c", "c.jpg")]);
        let changes = changes_since(&store, start);
        assert_eq!(changes.len(), 2, "what went away waits until the departures are settled");
        assert!(changes.contains(&Change::Updated { folder: month.clone(), entry: photo(&month, "a", "a-1.jpg") }), "renamed");
        assert!(changes.contains(&Change::Updated { folder: month.clone(), entry: photo(&month, "c", "c.jpg") }), "new");
        assert_eq!(store.settle_departures().unwrap(), 1);
        assert_eq!(changes_since(&store, start)[2], Change::Deleted(photo(&month, "b", "").node));
        let now = store.journal_anchor().unwrap();
        assert_eq!(now.sequence, start.sequence + 3);
        assert_eq!(changes_since(&store, now), vec![]);

        store.keep_listing(&month, &[photo(&month, "a", "a-1.jpg"), photo(&month, "c", "c.jpg")]);
        assert_eq!(store.journal_anchor().unwrap(), now, "the same listing again changes nothing");
    }

    #[test]
    fn a_folder_that_went_away_takes_its_kept_contents_with_it() {
        let directory = tempfile::tempdir().unwrap();
        let store = MetadataStore::open(&directory.path().join("metadata.redb")).unwrap().with_journal(100).unwrap();
        let year = NodeKey::TimelineYear(2024);
        let month = NodeKey::TimelineMonth(YearMonth { year: 2024, month: 3 });
        store.keep_listing(&year, &[folder_entry("03 March", month.clone())]);
        store.keep_listing(&month, &[photo(&month, "a", "a.jpg")]);
        let start = store.journal_anchor().unwrap();
        store.keep_listing(&year, &[]);
        store.settle_departures().unwrap();
        let changes = changes_since(&store, start);
        assert!(changes.contains(&Change::Deleted(month.clone())));
        assert!(changes.contains(&Change::Deleted(photo(&month, "a", "").node)), "and the photo the system held in it");
        assert_eq!(store.kept_listing(&month), None, "its listing is forgotten, so polls stop asking for it");
        assert_eq!(store.kept_folders(), vec![year]);
    }

    #[test]
    fn an_anchor_expires_once_trimmed_or_from_another_store() {
        let directory = tempfile::tempdir().unwrap();
        let store = MetadataStore::open(&directory.path().join("metadata.redb")).unwrap().with_journal(2).unwrap();
        let month = NodeKey::TimelineMonth(YearMonth { year: 2024, month: 3 });
        let start = store.journal_anchor().unwrap();
        store.keep_listing(&month, &[]);
        store.keep_listing(&month, &[photo(&month, "a", "a.jpg"), photo(&month, "b", "b.jpg"), photo(&month, "c", "c.jpg")]);
        assert_eq!(store.journal_since(start, 100).unwrap(), JournalRead::Expired, "only the last two records are kept");
        let latest = store.journal_anchor().unwrap();
        assert!(matches!(store.journal_since(Anchor { sequence: latest.sequence - 2, ..latest }, 100).unwrap(), JournalRead::Changes { .. }));
        assert_eq!(store.journal_since(Anchor { epoch: latest.epoch ^ 1, ..latest }, 100).unwrap(), JournalRead::Expired);
        assert_eq!(store.journal_since(Anchor { sequence: latest.sequence + 1, ..latest }, 100).unwrap(), JournalRead::Expired);
        match store.journal_since(Anchor { sequence: latest.sequence - 2, ..latest }, 1).unwrap() {
            JournalRead::Changes { changes, anchor, more } => {
                assert_eq!((changes.len(), anchor.sequence, more), (1, latest.sequence - 1, true), "read in parts");
            }
            JournalRead::Expired => panic!("still kept"),
        }
    }

    #[test]
    fn the_journal_and_its_anchors_survive_reopening_the_store() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("metadata.redb");
        let month = NodeKey::TimelineMonth(YearMonth { year: 2024, month: 3 });
        let start = {
            let store = MetadataStore::open(&path).unwrap().with_journal(100).unwrap();
            store.keep_listing(&month, &[]);
            let start = store.journal_anchor().unwrap();
            store.keep_listing(&month, &[photo(&month, "a", "a.jpg")]);
            store.keep_state("poll", "etags");
            start
        };
        let store = MetadataStore::open(&path).unwrap().with_journal(100).unwrap();
        assert_eq!(changes_since(&store, start).len(), 1);
        assert_eq!(Anchor::from_text(&start.to_text()), Some(start));
        assert_eq!(Anchor::from_text("not an anchor"), None);
        assert_eq!(store.kept_state("poll").as_deref(), Some("etags"));
        assert_eq!(store.kept_state("other"), None);
    }

    #[test]
    fn an_album_that_moves_to_another_year_folder_is_moved_not_deleted() {
        for new_year_listed_first in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let store = MetadataStore::open(&directory.path().join("metadata.redb")).unwrap().with_journal(100).unwrap();
            let (old_year, new_year) = (NodeKey::AlbumYear(Some(2023)), NodeKey::AlbumYear(Some(2025)));
            let album = NodeKey::Album("x".into());
            store.keep_listing(&old_year, &[folder_entry("Trip", album.clone())]);
            store.keep_listing(&new_year, &[]);
            store.keep_listing(&album, &[photo(&album, "a", "a.jpg")]);
            let start = store.journal_anchor().unwrap();
            if new_year_listed_first {
                store.keep_listing(&new_year, &[folder_entry("Trip", album.clone())]);
                store.keep_listing(&old_year, &[]);
            } else {
                store.keep_listing(&old_year, &[]);
                store.keep_listing(&new_year, &[folder_entry("Trip", album.clone())]);
            }
            assert_eq!(store.settle_departures().unwrap(), 0, "nothing went away");
            let changes = changes_since(&store, start);
            assert_eq!(changes, [Change::Updated { folder: new_year.clone(), entry: folder_entry("Trip", album.clone()) }]);
            assert_eq!(store.kept_listing(&album).map(|listing| listing.len()), Some(1), "it keeps what was kept below it");
        }
    }

    #[test]
    fn folders_to_list_again_and_state_change_in_one_step() {
        let directory = tempfile::tempdir().unwrap();
        let store = MetadataStore::open(&directory.path().join("metadata.redb")).unwrap().with_journal(100).unwrap();
        let month = NodeKey::TimelineMonth(YearMonth { year: 2024, month: 3 });
        store.mark_for_relisting(&[month.clone(), NodeKey::Root]).unwrap();
        store.mark_for_relisting(std::slice::from_ref(&month)).unwrap();
        assert_eq!(store.folders_to_relist().len(), 2);
        store.relisted(&month).unwrap();
        assert_eq!(store.folders_to_relist(), [NodeKey::Root]);

        let start = store.journal_anchor().unwrap();
        let answer = store
            .update_state("queue", |kept| {
                assert_eq!(kept, None);
                (Some("one".to_owned()), vec![Change::Deleted(month.clone())], 7)
            })
            .unwrap();
        assert_eq!((answer, store.kept_state("queue").as_deref()), (7, Some("one")));
        assert_eq!(changes_since(&store, start), [Change::Deleted(month.clone())], "recorded with the state");
        store.update_state("queue", |_| (None, Vec::new(), ())).unwrap();
        assert_eq!(store.kept_state("queue"), None);
    }

    #[test]
    fn a_listing_shown_during_a_change_is_kept_and_listed_again_only_with_the_journal() {
        let directory = tempfile::tempdir().unwrap();
        let month = NodeKey::TimelineMonth(YearMonth { year: 2024, month: 3 });
        let plain = MetadataStore::open(&directory.path().join("plain.redb")).unwrap();
        plain.shown_during_change(&month, &[photo(&month, "a", "a.jpg")]);
        assert_eq!(plain.kept_listing(&month), None, "Linux keeps no listing a change may have outdated");
        let journaled = MetadataStore::open(&directory.path().join("journaled.redb")).unwrap().with_journal(100).unwrap();
        journaled.shown_during_change(&month, &[photo(&month, "a", "a.jpg")]);
        assert_eq!(journaled.kept_listing(&month).map(|listing| listing.len()), Some(1));
        assert_eq!(journaled.folders_to_relist(), [month]);
    }

    #[test]
    fn without_the_journal_nothing_is_recorded() {
        let directory = tempfile::tempdir().unwrap();
        let store = MetadataStore::open(&directory.path().join("metadata.redb")).unwrap();
        let month = NodeKey::TimelineMonth(YearMonth { year: 2024, month: 3 });
        store.keep_listing(&month, &[]);
        store.keep_listing(&month, &[photo(&month, "a", "a.jpg")]);
        assert_eq!(store.journal_anchor().unwrap().sequence, 0);
    }
}

