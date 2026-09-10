use nickel_storage::{atomic_write, config_path, read_regular_file, stage_write};
use std::{
    io,
    path::{Path, PathBuf},
};

const MAX_SETTINGS_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum WallpaperPosition {
    Center,
    Tile,
    Stretch,
    Fit,
    Span,
    #[default]
    Fill,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct WallpaperSettings {
    pub image: Option<PathBuf>,
    pub position: WallpaperPosition,
}

impl WallpaperSettings {
    pub fn load_default() -> Self {
        settings_path().and_then(Self::load).unwrap_or_default()
    }

    pub fn save_default(&self) -> io::Result<()> {
        self.save(settings_path()?)
    }

    pub fn load(path: impl AsRef<Path>) -> io::Result<Self> {
        let bytes = read_regular_file(path.as_ref(), MAX_SETTINGS_BYTES)?
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
        let contents = std::str::from_utf8(&bytes)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let mut settings = Self::default();
        for line in contents.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            match key.trim() {
                "image" if !value.trim().is_empty() => settings.image = Some(value.trim().into()),
                "position" => {
                    settings.position = match value.trim() {
                        "center" => WallpaperPosition::Center,
                        "tile" => WallpaperPosition::Tile,
                        "stretch" => WallpaperPosition::Stretch,
                        "fit" => WallpaperPosition::Fit,
                        "span" => WallpaperPosition::Span,
                        _ => WallpaperPosition::Fill,
                    }
                }
                _ => {}
            }
        }
        Ok(settings)
    }

    pub fn save(&self, path: impl AsRef<Path>) -> io::Result<()> {
        let path = path.as_ref();
        let _lock = nickel_storage::TransactionLock::try_acquire(path)?;
        atomic_write(path, self.encode())
    }

    fn encode(&self) -> String {
        let position = match self.position {
            WallpaperPosition::Center => "center",
            WallpaperPosition::Tile => "tile",
            WallpaperPosition::Stretch => "stretch",
            WallpaperPosition::Fit => "fit",
            WallpaperPosition::Span => "span",
            WallpaperPosition::Fill => "fill",
        };
        format!(
            "image={}\nposition={position}\n",
            self.image
                .as_deref()
                .map(Path::to_string_lossy)
                .unwrap_or_default()
        )
    }
}

/// A wallpaper preference replacement prepared without changing the destination.
/// The transaction lock is retained through the checked atomic rename.
pub struct PreparedWallpaperSettings {
    path: PathBuf,
    revision: Option<nickel_storage::RegularFileRevision>,
    requested: WallpaperSettings,
    staged: nickel_storage::StagedWrite,
    _lock: nickel_storage::TransactionLock,
}

impl PreparedWallpaperSettings {
    /// Missing settings represent the initial default. Unreadable settings and
    /// an observed local replacement are never converted into a writable default.
    pub fn prepare(
        path: PathBuf,
        prior: &WallpaperSettings,
        requested: WallpaperSettings,
    ) -> io::Result<Self> {
        let lock = nickel_storage::TransactionLock::try_acquire(&path)?;
        let revision = nickel_storage::regular_file_revision(&path)?;
        let current = match WallpaperSettings::load(&path) {
            Ok(value) => value,
            Err(error) if error.kind() == io::ErrorKind::NotFound && revision.is_none() => {
                WallpaperSettings::default()
            }
            Err(error) => return Err(error),
        };
        if nickel_storage::regular_file_revision(&path)? != revision || &current != prior {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "wallpaper settings changed",
            ));
        }
        let staged = stage_write(&path, requested.encode())?;
        Ok(Self {
            path,
            revision,
            requested,
            staged,
            _lock: lock,
        })
    }

    /// Recheck the file revision and caller authority immediately before rename.
    pub fn commit(self, check: impl FnOnce() -> io::Result<()>) -> io::Result<WallpaperSettings> {
        self.staged.commit(|| {
            if nickel_storage::regular_file_revision(&self.path)? != self.revision {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "wallpaper settings changed",
                ));
            }
            check()
        })?;
        Ok(self.requested)
    }
}

pub fn settings_path() -> io::Result<PathBuf> {
    config_path("wallpaper-settings")
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_SETTINGS_BYTES, PreparedWallpaperSettings, WallpaperPosition, WallpaperSettings,
    };

    fn fixture_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "nickel-wallpaper-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ))
    }

    #[test]
    fn round_trips_wallpaper_preferences() {
        let path = fixture_path("round-trip");
        let expected = WallpaperSettings {
            image: Some("/tmp/fantasy.png".into()),
            position: WallpaperPosition::Fit,
        };
        expected.save(&path).expect("save wallpaper settings");
        assert_eq!(
            WallpaperSettings::load(&path).expect("load wallpaper settings"),
            expected
        );
        std::fs::remove_file(path).expect("remove wallpaper settings fixture");
    }

    #[test]
    fn rejects_oversized_settings_before_parsing() {
        let path = fixture_path("oversized");
        std::fs::write(&path, vec![b' '; MAX_SETTINGS_BYTES + 1]).expect("write oversized fixture");
        let error = WallpaperSettings::load(&path).expect_err("oversized settings must fail");
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        std::fs::remove_file(path).expect("remove oversized fixture");
    }

    #[test]
    fn rejects_non_utf8_settings_instead_of_returning_partial_defaults() {
        let path = fixture_path("non-utf8");
        std::fs::write(&path, b"position=fit\nimage=\xff\n").expect("write non-UTF-8 fixture");
        let error = WallpaperSettings::load(&path).expect_err("non-UTF-8 settings must fail");
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        std::fs::remove_file(path).expect("remove non-UTF-8 fixture");
    }

    #[test]
    fn staged_change_rejects_local_replacement_and_cancelled_commit() {
        let directory = tempfile::tempdir().expect("wallpaper fixture directory");
        let path = directory.path().join("wallpaper-settings");
        let prior = WallpaperSettings {
            image: Some("/tmp/prior.png".into()),
            position: WallpaperPosition::Fit,
        };
        prior.save(&path).expect("save prior settings");
        let requested = WallpaperSettings {
            image: None,
            position: WallpaperPosition::Center,
        };

        let cancelled = PreparedWallpaperSettings::prepare(path.clone(), &prior, requested.clone())
            .expect("prepare cancelled transaction");
        assert!(
            cancelled
                .commit(|| Err(std::io::Error::other("cancelled")))
                .is_err()
        );
        assert_eq!(WallpaperSettings::load(&path).unwrap(), prior);

        let stale = PreparedWallpaperSettings::prepare(path.clone(), &prior, requested.clone())
            .expect("prepare stale transaction");
        // The transaction lock blocks cooperative writers, but an external
        // replacement must still lose the revision comparison at commit.
        std::fs::write(&path, "image=/tmp/prior.png\nposition=tile\n")
            .expect("replace settings externally");
        assert!(stale.commit(|| Ok(())).is_err());
        assert_eq!(
            WallpaperSettings::load(&path).unwrap().position,
            WallpaperPosition::Tile
        );

        let external = WallpaperSettings::load(&path).unwrap();
        let accepted =
            PreparedWallpaperSettings::prepare(path.clone(), &external, requested.clone())
                .expect("prepare accepted transaction")
                .commit(|| Ok(()))
                .expect("commit accepted transaction");
        assert_eq!(accepted, requested);
        assert_eq!(WallpaperSettings::load(&path).unwrap(), requested);
        let names = std::fs::read_dir(directory.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(names.len(), 2, "settings and its transaction lock remain");
        assert!(!names.iter().any(|name| name.contains(".tmp-")));
    }
}
