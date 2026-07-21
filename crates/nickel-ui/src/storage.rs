use std::{env, fs, path::PathBuf};

use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};

const METADATA: TableDefinition<&str, u64> = TableDefinition::new("metadata");
const PINS: TableDefinition<&str, u64> = TableDefinition::new("pins");
const SCHEMA_VERSION: u64 = 1;

pub struct PinStore {
    database: Database,
}

impl PinStore {
    pub fn open_default() -> Result<Self, String> {
        Self::open(state_path()?)
    }

    pub fn open(path: PathBuf) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
        }
        let database = Database::create(&path)
            .map_err(|error| format!("failed to open {}: {error}", path.display()))?;
        let store = Self { database };
        store.initialize()?;
        Ok(store)
    }

    pub fn pins(&self) -> Result<Vec<(String, u64)>, String> {
        let transaction = self.database.begin_read().map_err(display_error)?;
        let table = transaction.open_table(PINS).map_err(display_error)?;
        let mut pins = table
            .iter()
            .map_err(display_error)?
            .map(|entry| {
                let (id, order) = entry.map_err(display_error)?;
                Ok((id.value().to_owned(), order.value()))
            })
            .collect::<Result<Vec<_>, String>>()?;
        pins.sort_by_key(|(_, order)| *order);
        Ok(pins)
    }

    pub fn toggle(&self, application_id: &str) -> Result<bool, String> {
        let transaction = self.database.begin_write().map_err(display_error)?;
        let pinned = {
            let mut table = transaction.open_table(PINS).map_err(display_error)?;
            if table.get(application_id).map_err(display_error)?.is_some() {
                table.remove(application_id).map_err(display_error)?;
                false
            } else {
                let next_order = table
                    .iter()
                    .map_err(display_error)?
                    .map(|entry| entry.map(|(_, order)| order.value()))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(display_error)?
                    .into_iter()
                    .max()
                    .unwrap_or(0)
                    .saturating_add(1);
                table
                    .insert(application_id, next_order)
                    .map_err(display_error)?;
                true
            }
        };
        transaction.commit().map_err(display_error)?;
        Ok(pinned)
    }

    fn initialize(&self) -> Result<(), String> {
        let transaction = self.database.begin_write().map_err(display_error)?;
        {
            let mut metadata = transaction.open_table(METADATA).map_err(display_error)?;
            metadata
                .insert("schema_version", SCHEMA_VERSION)
                .map_err(display_error)?;
            transaction.open_table(PINS).map_err(display_error)?;
        }
        transaction.commit().map_err(display_error)
    }
}

fn state_path() -> Result<PathBuf, String> {
    if let Some(path) = env::var_os("NICKEL_STATE_PATH") {
        return Ok(path.into());
    }

    #[cfg(target_os = "windows")]
    let path = env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .map(|directory| directory.join("Nickel").join("state.redb"));
    #[cfg(target_os = "macos")]
    let path = env::var_os("HOME").map(PathBuf::from).map(|directory| {
        directory
            .join("Library")
            .join("Application Support")
            .join("Nickel")
            .join("state.redb")
    });
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    let path = env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            env::var_os("HOME").map(|home| PathBuf::from(home).join(".local").join("share"))
        })
        .map(|directory| directory.join("nickel").join("state.redb"));

    path.ok_or_else(|| "could not determine Nickel's persistent data directory".to_owned())
}

fn display_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::PinStore;

    #[test]
    fn pins_survive_reopen_and_keep_insertion_order() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("state.redb");
        {
            let store = PinStore::open(path.clone()).expect("open store");
            assert!(store.toggle("org.example.Second").expect("pin second"));
            assert!(store.toggle("org.example.First").expect("pin first"));
            assert_eq!(
                store.pins().expect("read pins"),
                vec![
                    ("org.example.Second".into(), 1),
                    ("org.example.First".into(), 2),
                ]
            );
        }
        let reopened = PinStore::open(path).expect("reopen store");
        assert!(!reopened.toggle("org.example.Second").expect("unpin"));
        assert_eq!(
            reopened.pins().expect("read remaining pins"),
            vec![("org.example.First".into(), 2)]
        );
    }
}
