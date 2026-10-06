//! Startup-only inheritance. Nickel's persisted choices remain authoritative.
use nickel_core::dpi::Scale120;
use std::{path::Path, sync::OnceLock};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct InheritedDesktopPreferences {
    pub icon_theme: Option<String>,
    pub cursor_theme: Option<String>,
    pub cursor_size: Option<u32>,
    pub scale: Option<Scale120>,
}

impl InheritedDesktopPreferences {
    fn fallback(self, other: Self) -> Self {
        Self {
            icon_theme: self.icon_theme.or(other.icon_theme),
            cursor_theme: self.cursor_theme.or(other.cursor_theme),
            cursor_size: self.cursor_size.or(other.cursor_size),
            scale: self.scale.or(other.scale),
        }
    }
}

/// Cache the bounded startup observation rather than invoking desktop helpers
/// during icon resolution, rendering, or input. Explicit Nickel choices are
/// resolved by the caller before consulting these inherited values.
pub fn inherited_desktop_preferences() -> &'static InheritedDesktopPreferences {
    static PREFERENCES: OnceLock<InheritedDesktopPreferences> = OnceLock::new();
    PREFERENCES.get_or_init(|| {
        let gtk = gtk_settings(crate::toolkit_scale::read_gtk_interface_setting);
        let root = std::env::var_os("XDG_CONFIG_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(".config")));
        root.map_or(gtk.clone(), |root| {
            gtk.fallback(gtk_ini(&read_config(&root.join("gtk-4.0/settings.ini"))))
                .fallback(gtk_ini(&read_config(&root.join("gtk-3.0/settings.ini"))))
                .fallback(kde_settings(
                    &read_config(&root.join("kdeglobals")),
                    &read_config(&root.join("kcminputrc")),
                ))
        })
    })
}

fn read_config(path: &Path) -> String {
    nickel_storage::read_regular_file(path, 64 * 1024)
        .ok()
        .flatten()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .unwrap_or_default()
}

fn gtk_settings(mut read: impl FnMut(&str) -> Option<String>) -> InheritedDesktopPreferences {
    InheritedDesktopPreferences {
        icon_theme: read("icon-theme").as_deref().and_then(theme),
        cursor_theme: read("cursor-theme").as_deref().and_then(theme),
        cursor_size: read("cursor-size").as_deref().and_then(cursor_size),
        // Zero means automatic, so it does not supply an inherited scale.
        scale: read("scaling-factor").as_deref().and_then(scale),
    }
}

fn gtk_ini(contents: &str) -> InheritedDesktopPreferences {
    InheritedDesktopPreferences {
        icon_theme: ini_value(contents, "Settings", "gtk-icon-theme-name").and_then(theme),
        cursor_theme: ini_value(contents, "Settings", "gtk-cursor-theme-name").and_then(theme),
        cursor_size: ini_value(contents, "Settings", "gtk-cursor-theme-size").and_then(cursor_size),
        // gtk-xft-dpi is font DPI, not compositor scale.
        scale: None,
    }
}

fn kde_settings(icons: &str, cursor: &str) -> InheritedDesktopPreferences {
    InheritedDesktopPreferences {
        icon_theme: ini_value(icons, "Icons", "Theme").and_then(theme),
        cursor_theme: ini_value(cursor, "Mouse", "cursorTheme").and_then(theme),
        cursor_size: ini_value(cursor, "Mouse", "cursorSize").and_then(cursor_size),
        scale: ini_value(icons, "KScreen", "ScaleFactor").and_then(scale),
    }
}

fn ini_value<'a>(contents: &'a str, group: &str, key: &str) -> Option<&'a str> {
    let mut inside = false;
    let mut seen_group = false;
    let mut value = None;
    for line in contents.lines().map(str::trim) {
        if line.starts_with('[') {
            inside = line
                .strip_prefix('[')
                .and_then(|line| line.strip_suffix(']'))
                == Some(group);
            if inside && std::mem::replace(&mut seen_group, true) {
                return None;
            }
        } else if inside
            && let Some((name, contents)) = line.split_once('=')
            && name.trim() == key
        {
            if value.is_some() {
                return None;
            }
            value = Some(contents.trim());
        }
    }
    value
}

fn theme(value: &str) -> Option<String> {
    let value = value.trim();
    let value = value
        .strip_prefix('\'')
        .and_then(|value| value.strip_suffix('\''))
        .or_else(|| {
            value
                .strip_prefix('"')
                .and_then(|value| value.strip_suffix('"'))
        })
        .unwrap_or(value);
    (!value.is_empty()
        && value.len() <= 256
        && !matches!(value, "." | "..")
        && !value.chars().any(|character| {
            character.is_control() || matches!(character, '/' | '\\' | '\'' | '"')
        }))
    .then(|| value.to_owned())
}

fn cursor_size(value: &str) -> Option<u32> {
    value
        .trim()
        .strip_prefix("uint32 ")
        .unwrap_or(value.trim())
        .parse()
        .ok()
        .filter(|size| (1..=512).contains(size))
}

fn scale(value: &str) -> Option<Scale120> {
    let factor: f64 = value
        .trim()
        .strip_prefix("uint32 ")
        .unwrap_or(value.trim())
        .parse()
        .ok()?;
    if !factor.is_finite() || !(0.5..=4.0).contains(&factor) {
        return None;
    }
    Scale120::new((factor * 120.0).round() as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gtk_wins_per_field_and_missing_fields_fall_back_to_kde() {
        let gtk = gtk_settings(|key| match key {
            "icon-theme" => Some("'Adwaita'".into()),
            "cursor-size" => Some("32".into()),
            "scaling-factor" => Some("uint32 2".into()),
            _ => None,
        });
        let resolved = gtk.fallback(kde_settings(
            "[Icons]\nTheme=Breeze\n[KScreen]\nScaleFactor=1.25\n",
            "[Mouse]\ncursorTheme=Breeze_Cursors\ncursorSize=24\n",
        ));
        assert_eq!(resolved.icon_theme.as_deref(), Some("Adwaita"));
        assert_eq!(resolved.cursor_theme.as_deref(), Some("Breeze_Cursors"));
        assert_eq!(resolved.cursor_size, Some(32));
        assert_eq!(resolved.scale, Scale120::new(240));
    }

    #[test]
    fn automatic_or_invalid_gtk_scale_allows_kde_fractional_fallback() {
        for value in ["uint32 0", "NaN", "inf", "-1", "999", "invalid"] {
            let gtk = gtk_settings(|key| (key == "scaling-factor").then(|| value.into()));
            assert_eq!(
                gtk.fallback(kde_settings("[KScreen]\nScaleFactor=1.25\n", ""))
                    .scale,
                Scale120::new(150)
            );
        }
        assert_eq!(kde_settings("[KScreen]\nScaleFactor=NaN\n", "").scale, None);
    }

    #[test]
    fn gtk_files_precede_kde_without_treating_font_dpi_as_display_scale() {
        let gtk = gtk_ini(
            "[Settings]\ngtk-icon-theme-name=Papirus\ngtk-cursor-theme-name=Adwaita\ngtk-cursor-theme-size=36\ngtk-xft-dpi=196608\n",
        );
        let resolved = gtk.fallback(kde_settings(
            "[Icons]\nTheme=Breeze\n[KScreen]\nScaleFactor=1.5\n",
            "[Mouse]\ncursorTheme=Oxygen_Black\ncursorSize=24\n",
        ));
        assert_eq!(resolved.icon_theme.as_deref(), Some("Papirus"));
        assert_eq!(resolved.cursor_theme.as_deref(), Some("Adwaita"));
        assert_eq!(resolved.cursor_size, Some(36));
        assert_eq!(resolved.scale, Scale120::new(180));
    }

    #[test]
    fn inheritance_ignores_wrong_sections_ambiguous_keys_and_invalid_theme_paths() {
        assert_eq!(
            kde_settings(
                "[Other]\nTheme=Wrong\nScaleFactor=2\n",
                "[Other]\ncursorTheme=Wrong\ncursorSize=36\n"
            ),
            InheritedDesktopPreferences::default()
        );
        assert_eq!(
            ini_value("[Icons]\nTheme=A\nTheme=B\n", "Icons", "Theme"),
            None
        );
        assert_eq!(
            ini_value("[Icons]\nTheme=A\n[Icons]\nTheme=B\n", "Icons", "Theme"),
            None
        );
        for value in [
            "'',",
            "../Breeze",
            "/tmp/theme",
            "..",
            "A\nB",
            "'unfinished",
        ] {
            assert!(theme(value).is_none());
        }
    }
}
