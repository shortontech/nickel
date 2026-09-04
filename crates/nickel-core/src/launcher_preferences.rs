use crate::persistence::{atomic_write, config_path};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

const MAX_ENTRIES: usize = 64;

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
        let contents = fs::read_to_string(path)?;
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
        let path = path.as_ref();
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
        atomic_write(path, contents)
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

    pub fn record_launch(&mut self, application_id: &str) {
        self.recents.retain(|id| id != application_id);
        self.recents.insert(0, application_id.to_owned());
        self.recents.truncate(MAX_ENTRIES);
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
    if !value.len().is_multiple_of(2) || value.len() > 8_192 {
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

fn preferences_path() -> io::Result<PathBuf> {
    config_path("launcher-preferences")
}

#[cfg(test)]
mod tests {
    use super::LauncherPreferences;

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
}
