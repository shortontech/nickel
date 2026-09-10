use nickel_storage::{atomic_write, config_path, read_regular_file};
use std::{
    io,
    path::{Path, PathBuf},
};

const MAX_ENTRIES: usize = 64;
const MAX_ENCODED_ID_BYTES: usize = 8_192;
// Both complete lists, with the largest supported IDs and line framing.
const MAX_FILE_BYTES: usize = 2 * MAX_ENTRIES * (MAX_ENCODED_ID_BYTES + 10);

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LauncherPreferences {
    favorites: Vec<String>,
    recents: Vec<String>,
}

impl LauncherPreferences {
    pub fn load_default() -> io::Result<Self> {
        Self::load(preferences_path()?)
    }

    pub fn save_default(&self) -> io::Result<()> {
        self.save(preferences_path()?)
    }

    pub fn load(path: impl AsRef<Path>) -> io::Result<Self> {
        let bytes = read_regular_file(path.as_ref(), MAX_FILE_BYTES)?
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
        let contents = std::str::from_utf8(&bytes)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let mut preferences = Self::default();
        for line in contents.lines() {
            let Some((kind, encoded)) = line.split_once('=') else {
                continue;
            };
            let Some(value) = decode(encoded) else {
                continue;
            };
            let entries = match kind {
                "favorite" => &mut preferences.favorites,
                "recent" => &mut preferences.recents,
                _ => continue,
            };
            if entries.len() < MAX_ENTRIES && !entries.contains(&value) {
                entries.push(value);
            }
        }
        Ok(preferences)
    }

    pub fn save(&self, path: impl AsRef<Path>) -> io::Result<()> {
        let _lock = nickel_storage::TransactionLock::try_acquire(path.as_ref())?;
        atomic_write(path.as_ref(), self.encode()?)
    }

    fn encode(&self) -> io::Result<String> {
        if self.favorites.len() > MAX_ENTRIES
            || self.recents.len() > MAX_ENTRIES
            || self
                .favorites
                .iter()
                .chain(&self.recents)
                .any(|id| id.is_empty() || id.len() > MAX_ENCODED_ID_BYTES / 2)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "launcher preference bounds exceeded",
            ));
        }
        let mut contents = String::new();
        for favorite in self.favorites.iter().take(MAX_ENTRIES) {
            contents.push_str("favorite=");
            contents.push_str(&encode(favorite));
            contents.push('\n');
        }
        for recent in self.recents.iter().take(MAX_ENTRIES) {
            contents.push_str("recent=");
            contents.push_str(&encode(recent));
            contents.push('\n');
        }
        Ok(contents)
    }

    pub fn favorites(&self) -> &[String] {
        &self.favorites
    }

    pub fn recents(&self) -> &[String] {
        &self.recents
    }

    pub fn replace_favorites(&mut self, favorites: impl IntoIterator<Item = String>) {
        self.favorites.clear();
        for favorite in favorites {
            if self.favorites.len() >= MAX_ENTRIES {
                break;
            }
            if !favorite.is_empty() && !self.favorites.contains(&favorite) {
                self.favorites.push(favorite);
            }
        }
    }

    pub fn is_favorite(&self, application_id: &str) -> bool {
        self.favorites.iter().any(|id| id == application_id)
    }

    pub fn toggle_favorite(&mut self, application_id: &str) -> bool {
        if let Some(index) = self.favorites.iter().position(|id| id == application_id) {
            self.favorites.remove(index);
            false
        } else {
            self.favorites.push(application_id.to_owned());
            if self.favorites.len() > MAX_ENTRIES {
                self.favorites.remove(0);
            }
            true
        }
    }

    pub fn move_favorite(&mut self, application_id: &str, direction: isize) -> bool {
        let Some(index) = self.favorites.iter().position(|id| id == application_id) else {
            return false;
        };
        let next = if direction.is_negative() {
            index.saturating_sub(1)
        } else {
            (index + 1).min(self.favorites.len().saturating_sub(1))
        };
        if next == index {
            return false;
        }
        self.favorites.swap(index, next);
        true
    }

    pub fn record_launch(&mut self, application_id: &str) {
        self.recents.retain(|id| id != application_id);
        self.recents.insert(0, application_id.to_owned());
        self.recents.truncate(MAX_ENTRIES);
    }
}

/// One locked, checked production preference replacement. Both local history
/// updates and remote favorite transactions preserve the same prior snapshot.
pub struct PreparedLauncherPreferences {
    path: PathBuf,
    revision: Option<nickel_storage::RegularFileRevision>,
    requested: LauncherPreferences,
    staged: nickel_storage::StagedWrite,
    _lock: nickel_storage::TransactionLock,
}
impl PreparedLauncherPreferences {
    /// Stage bounded content off the event owner. Missing files represent the
    /// initial default; unreadable files never become a writable default.
    pub fn prepare(
        path: PathBuf,
        prior: &LauncherPreferences,
        requested: LauncherPreferences,
    ) -> io::Result<Self> {
        Self::prepare_with(path, |current| {
            if &current != prior {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "launcher preferences changed",
                ));
            }
            Ok(requested)
        })
    }
    /// Local favorites updates preserve freshly read history rather than writing
    /// a possibly stale whole-UI snapshot over it.
    pub fn prepare_favorites(
        path: PathBuf,
        prior: &[String],
        requested: Vec<String>,
    ) -> io::Result<Self> {
        Self::prepare_with(path, |mut current| {
            if current.favorites() != prior {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "launcher favorites changed",
                ));
            }
            current.replace_favorites(requested);
            Ok(current)
        })
    }
    /// Launch recording never replaces favorites from a stale UI snapshot.
    pub fn prepare_recent(path: PathBuf, application_id: &str) -> io::Result<Self> {
        Self::prepare_with(path, |mut current| {
            current.record_launch(application_id);
            Ok(current)
        })
    }
    fn prepare_with(
        path: PathBuf,
        change: impl FnOnce(LauncherPreferences) -> io::Result<LauncherPreferences>,
    ) -> io::Result<Self> {
        let lock = nickel_storage::TransactionLock::try_acquire(&path)?;
        let revision = nickel_storage::regular_file_revision(&path)?;
        let current = match LauncherPreferences::load(&path) {
            Ok(value) => value,
            Err(error) if error.kind() == io::ErrorKind::NotFound && revision.is_none() => {
                LauncherPreferences::default()
            }
            Err(error) => return Err(error),
        };
        if nickel_storage::regular_file_revision(&path)? != revision {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "launcher preferences changed",
            ));
        }
        let requested = change(current)?;
        let staged = nickel_storage::stage_write(&path, requested.encode()?)?;
        Ok(Self {
            path,
            revision,
            requested,
            staged,
            _lock: lock,
        })
    }
    /// Metadata-only prior validation, then caller authority immediately before
    /// atomic rename. The returned snapshot is accepted state even if a caller's
    /// subsequent result authorization fails; reconcile it before discarding reply.
    pub fn commit(self, check: impl FnOnce() -> io::Result<()>) -> io::Result<LauncherPreferences> {
        self.staged.commit(|| {
            if nickel_storage::regular_file_revision(&self.path)? != self.revision {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "launcher preferences changed",
                ));
            }
            check()
        })?;
        Ok(self.requested)
    }
}

fn encode(value: &str) -> String {
    use std::fmt::Write;

    value.as_bytes().iter().fold(
        String::with_capacity(value.len().saturating_mul(2)),
        |mut encoded, byte| {
            let _ = write!(encoded, "{byte:02x}");
            encoded
        },
    )
}

fn decode(value: &str) -> Option<String> {
    if !value.len().is_multiple_of(2) || value.len() > MAX_ENCODED_ID_BYTES {
        return None;
    }
    let bytes = value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let pair = std::str::from_utf8(pair).ok()?;
            u8::from_str_radix(pair, 16).ok()
        })
        .collect::<Option<Vec<_>>>()?;
    String::from_utf8(bytes)
        .ok()
        .filter(|value| !value.is_empty())
}

pub fn preferences_path() -> io::Result<PathBuf> {
    config_path("launcher-preferences")
}

#[cfg(test)]
mod tests {
    use super::LauncherPreferences;

    #[test]
    fn local_favorites_preserve_new_history_and_history_preserves_new_favorites() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("preferences");
        let mut external = LauncherPreferences::default();
        external.record_launch("new-private-recent");
        external.save(&path).unwrap();
        let favorite = super::PreparedLauncherPreferences::prepare_favorites(
            path.clone(),
            &[],
            vec!["installed.desktop".into()],
        )
        .unwrap();
        let actual = favorite.commit(|| Ok(())).unwrap();
        assert_eq!(actual.recents(), ["new-private-recent"]);
        let recent = super::PreparedLauncherPreferences::prepare_recent(
            path.clone(),
            "other-private-recent",
        )
        .unwrap();
        let actual = recent.commit(|| Ok(())).unwrap();
        assert_eq!(actual.favorites(), ["installed.desktop"]);
        assert_eq!(
            actual.recents(),
            ["other-private-recent", "new-private-recent"]
        );
        assert!(
            super::PreparedLauncherPreferences::prepare_favorites(
                path,
                &[],
                vec!["stale.desktop".into()]
            )
            .is_err()
        );
    }

    #[test]
    fn checked_preferences_preserve_recents_and_reject_local_races_and_revocation() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("preferences");
        let mut prior = LauncherPreferences::default();
        prior.record_launch("private-recent");
        prior.save(&path).unwrap();
        let mut requested = prior.clone();
        requested.toggle_favorite("installed.desktop");
        let staged =
            super::PreparedLauncherPreferences::prepare(path.clone(), &prior, requested.clone())
                .unwrap();
        assert_eq!(
            prior.save(&path).unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        assert!(
            staged
                .commit(|| Err(std::io::Error::other("revoked")))
                .is_err()
        );
        assert_eq!(LauncherPreferences::load(&path).unwrap(), prior);
        let staged =
            super::PreparedLauncherPreferences::prepare(path.clone(), &prior, requested.clone())
                .unwrap();
        std::fs::write(&path, "favorite=666f726569676e\n").unwrap();
        assert!(staged.commit(|| Ok(())).is_err());
        assert_eq!(
            LauncherPreferences::load(&path).unwrap().favorites(),
            ["foreign"]
        );
        prior.save(&path).unwrap();
        let staged =
            super::PreparedLauncherPreferences::prepare(path.clone(), &prior, requested.clone())
                .unwrap();
        let actual = staged.commit(|| Ok(())).unwrap();
        assert_eq!(actual, requested);
        assert_eq!(actual.recents(), ["private-recent"]);
        assert_eq!(LauncherPreferences::load(&path).unwrap(), requested);
    }

    #[test]
    fn preference_read_bounds_preserve_missing_empty_and_maximum_valid_lists() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("preferences");
        assert_eq!(
            LauncherPreferences::load(&path).unwrap_err().kind(),
            std::io::ErrorKind::NotFound
        );
        std::fs::write(&path, b"").unwrap();
        assert_eq!(
            LauncherPreferences::load(&path).unwrap(),
            LauncherPreferences::default()
        );
        let mut preferences = LauncherPreferences::default();
        for index in 0..super::MAX_ENTRIES {
            let id = format!(
                "{index:04}{}",
                "x".repeat(super::MAX_ENCODED_ID_BYTES / 2 - 4)
            );
            preferences.toggle_favorite(&id);
            preferences.record_launch(&id);
        }
        preferences.save(&path).unwrap();
        assert_eq!(LauncherPreferences::load(&path).unwrap(), preferences);
        std::fs::write(&path, [0xff]).unwrap();
        assert_eq!(
            LauncherPreferences::load(&path).unwrap_err().kind(),
            std::io::ErrorKind::InvalidData
        );
        std::fs::write(&path, vec![b' '; super::MAX_FILE_BYTES + 1]).unwrap();
        assert_eq!(
            LauncherPreferences::load(&path).unwrap_err().kind(),
            std::io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn favorites_and_recents_round_trip_canonical_unicode_ids() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("launcher-preferences");
        let mut preferences = LauncherPreferences::default();
        assert!(preferences.toggle_favorite("org.nickel.Files"));
        assert!(preferences.toggle_favorite("例.desktop"));
        preferences.record_launch("org.nickel.Files");
        preferences.record_launch("例.desktop");
        preferences.record_launch("org.nickel.Files");
        preferences.save(&path).expect("save preferences");

        let loaded = LauncherPreferences::load(&path).expect("load preferences");
        assert_eq!(loaded.favorites(), ["org.nickel.Files", "例.desktop"]);
        assert_eq!(loaded.recents(), ["org.nickel.Files", "例.desktop"]);
    }

    #[test]
    fn toggle_and_launch_delivery_are_idempotent_or_deduplicated() {
        let mut preferences = LauncherPreferences::default();
        assert!(preferences.toggle_favorite("files"));
        assert!(!preferences.toggle_favorite("files"));
        assert!(preferences.favorites().is_empty());
        preferences.record_launch("files");
        preferences.record_launch("files");
        assert_eq!(preferences.recents(), ["files"]);
    }

    #[test]
    fn malformed_and_duplicate_lines_are_bounded_and_ignored() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("launcher-preferences");
        std::fs::write(
            &path,
            "favorite=66696c6573\nfavorite=66696c6573\nrecent=zz\nunknown=74657374\n",
        )
        .expect("fixture");
        let loaded = LauncherPreferences::load(path).expect("load preferences");
        assert_eq!(loaded.favorites(), ["files"]);
        assert!(loaded.recents().is_empty());
    }

    #[test]
    fn favorite_reordering_is_stable_bounded_and_ignores_stale_requests() {
        let mut preferences = LauncherPreferences::default();
        preferences.replace_favorites(["a".into(), "b".into(), "c".into()]);
        assert!(preferences.move_favorite("b", -1));
        assert_eq!(preferences.favorites(), ["b", "a", "c"]);
        assert!(!preferences.move_favorite("b", -1));
        assert!(preferences.move_favorite("b", 1));
        assert_eq!(preferences.favorites(), ["a", "b", "c"]);
        assert!(!preferences.move_favorite("missing", 1));
    }
}
