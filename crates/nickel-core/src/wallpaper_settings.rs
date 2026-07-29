use std::{
    fs, io,
    path::{Path, PathBuf},
};

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
        let mut settings = Self::default();
        for line in fs::read_to_string(path)?.lines() {
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
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let position = match self.position {
            WallpaperPosition::Center => "center",
            WallpaperPosition::Tile => "tile",
            WallpaperPosition::Stretch => "stretch",
            WallpaperPosition::Fit => "fit",
            WallpaperPosition::Span => "span",
            WallpaperPosition::Fill => "fill",
        };
        fs::write(
            path,
            format!(
                "image={}\nposition={position}\n",
                self.image
                    .as_deref()
                    .map(Path::to_string_lossy)
                    .unwrap_or_default()
            ),
        )
    }
}

fn settings_path() -> io::Result<PathBuf> {
    #[cfg(target_os = "windows")]
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        return Ok(PathBuf::from(local)
            .join("Nickel")
            .join("wallpaper-settings"));
    }
    #[cfg(target_os = "windows")]
    return Err(io::Error::new(
        io::ErrorKind::NotFound,
        "LOCALAPPDATA is not set",
    ));
    #[cfg(not(target_os = "windows"))]
    {
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "XDG_CONFIG_HOME and HOME are not set",
                )
            })?;
        Ok(config.join("nickel").join("wallpaper-settings"))
    }
}

#[cfg(test)]
mod tests {
    use super::{WallpaperPosition, WallpaperSettings};

    #[test]
    fn round_trips_wallpaper_preferences() {
        let path = std::env::temp_dir().join(format!("nickel-wallpaper-{}", std::process::id()));
        let expected = WallpaperSettings {
            image: Some("/tmp/fantasy.png".into()),
            position: WallpaperPosition::Fit,
        };
        expected.save(&path).unwrap();
        assert_eq!(WallpaperSettings::load(&path).unwrap(), expected);
        let _ = std::fs::remove_file(path);
    }
}
