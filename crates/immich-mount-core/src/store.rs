//! The metadata store: state that must survive restarts, in one redb database per account.
//!
//! It holds the names handed out per folder, so a path keeps meaning the same Asset across restarts (principle 8).
//! Listings, the content block map, the thumbnail ledger and the sync cursor follow. redb is pure Rust and
//! crash-safe, so the core stays portable and statically buildable (ARCHITECTURE.md, "The metadata store").

use std::collections::HashMap;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::Path;

use redb::{Database, ReadableDatabase, TableDefinition};

use crate::tree::{Entry, NameMemory, NodeKey};

/// Folder storage key to its assigned names, encoded as `entry-key<TAB>name` lines.
const NAMES: TableDefinition<&str, &str> = TableDefinition::new("names");
/// Each folder's latest listing, as JSON, for showing folders while Immich cannot be reached.
const LISTINGS: TableDefinition<&str, &[u8]> = TableDefinition::new("listings");
/// Format version of the store, for future migrations.
const META: TableDefinition<&str, u64> = TableDefinition::new("meta");
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
        let store = MetadataStore { database };
        store.write_format_version().map_err(fail)?;
        Ok(store)
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

    fn write_listing(&self, folder: &NodeKey, entries: &[Entry]) -> Result<(), String> {
        let encoded = serde_json::to_vec(entries).map_err(|error| error.to_string())?;
        let transaction = self.database.begin_write().map_err(|error| error.to_string())?;
        {
            let mut table = transaction.open_table(LISTINGS).map_err(|error| error.to_string())?;
            table.insert(folder.storage_key().as_str(), encoded.as_slice()).map_err(|error| error.to_string())?;
        }
        transaction.commit().map_err(|error| error.to_string())
    }
}

impl NameMemory for MetadataStore {
    fn keep_listing(&self, folder: &NodeKey, entries: &[Entry]) {
        if let Err(error) = self.write_listing(folder, entries) {
            log::warn!("could not keep the listing of {}: {error}", folder.storage_key());
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
}

