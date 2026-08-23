use std::{
    collections::HashSet,
    env, fs,
    path::{Path, PathBuf},
};

use freedesktop_desktop_entry::{
    DesktopEntry, Iter, current_desktop, default_paths, get_languages_from_env,
};

use crate::launcher::Application;

pub fn load_applications() -> Vec<Application> {
    let locales = get_languages_from_env();
    let desktops = current_desktop().unwrap_or_default();
    let mut seen = HashSet::new();
    let mut applications = Vec::new();
    let icon_theme = icon_theme();

    for entry in Iter::new(default_paths()).entries(Some(&locales)) {
        // Higher-priority XDG directories appear first. Hidden entries must also
        // shadow a lower-priority entry with the same application ID.
        if !seen.insert(entry.id().to_owned()) {
            continue;
        }
        if let Some(application) = application_from_entry(&entry, &locales, &desktops, &icon_theme)
        {
            applications.push(application);
        }
    }

    applications.sort_by(|left, right| {
        left.name()
            .to_lowercase()
            .cmp(&right.name().to_lowercase())
            .then_with(|| left.id().cmp(right.id()))
    });
    applications
}

fn application_from_entry(
    entry: &DesktopEntry,
    locales: &[String],
    desktops: &[String],
    icon_theme: &str,
) -> Option<Application> {
    if entry.type_() != Some("Application")
        || entry.hidden()
        || entry.no_display()
        || !visible_on_desktop(entry, desktops)
    {
        return None;
    }

    let name = entry.name(locales)?.trim().to_owned();
    if name.is_empty() || (entry.exec().is_none() && !entry.dbus_activatable()) {
        return None;
    }

    let icon = entry.icon().map(str::to_owned);
    let icon_path = icon
        .as_deref()
        .and_then(|name| resolve_icon(name, icon_theme));
    Some(Application::new(
        entry.id().to_owned(),
        name,
        icon,
        icon_path,
        entry.exec().and_then(parse_exec),
    ))
}

fn parse_exec(exec: &str) -> Option<Vec<String>> {
    let arguments = shlex::split(exec)?;
    let arguments: Vec<_> = arguments
        .into_iter()
        .filter(|argument| {
            !matches!(
                argument.as_str(),
                "%f" | "%F" | "%u" | "%U" | "%i" | "%c" | "%k"
            )
        })
        .map(|argument| argument.replace("%%", "%"))
        .collect();
    (!arguments.is_empty()).then_some(arguments)
}

fn resolve_icon(name: &str, theme: &str) -> Option<PathBuf> {
    let path = Path::new(name);
    if path.is_absolute() && path.is_file() {
        return Some(path.to_owned());
    }
    [theme, "breeze-dark", "breeze", "hicolor", "Adwaita"]
        .into_iter()
        .find_map(|candidate| {
            freedesktop_icons::lookup(name)
                .with_size(48)
                .with_theme(candidate)
                .with_cache()
                .find()
        })
}

fn icon_theme() -> String {
    if let Ok(theme) = env::var("NICKEL_ICON_THEME")
        && !theme.trim().is_empty()
    {
        return theme;
    }

    let config_home = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")));
    config_home
        .and_then(|directory| fs::read_to_string(directory.join("kdeglobals")).ok())
        .and_then(|contents| value_in_section(&contents, "Icons", "Theme"))
        .unwrap_or_else(|| "hicolor".to_owned())
}

fn value_in_section(contents: &str, section: &str, key: &str) -> Option<String> {
    let mut in_section = false;
    for line in contents.lines().map(str::trim) {
        if line.starts_with('[') && line.ends_with(']') {
            in_section = &line[1..line.len() - 1] == section;
        } else if in_section
            && let Some(value) = line
                .strip_prefix(key)
                .and_then(|line| line.strip_prefix('='))
        {
            let value = value.trim();
            if !value.is_empty() {
                return Some(value.to_owned());
            }
        }
    }
    None
}

fn visible_on_desktop(entry: &DesktopEntry, desktops: &[String]) -> bool {
    let matches_current = |candidate: &str| {
        desktops
            .iter()
            .any(|desktop| candidate.eq_ignore_ascii_case(desktop))
    };

    if entry
        .only_show_in()
        .is_some_and(|values| !values.into_iter().any(matches_current))
    {
        return false;
    }
    !entry
        .not_show_in()
        .is_some_and(|values| values.into_iter().any(matches_current))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use freedesktop_desktop_entry::DesktopEntry;

    use super::{application_from_entry, value_in_section};

    fn parse(contents: &str) -> DesktopEntry {
        DesktopEntry::from_str(
            PathBuf::from("org.example.Test.desktop"),
            contents,
            Some(&["en_US"]),
        )
        .expect("valid desktop entry")
    }

    #[test]
    fn extracts_application_and_icon_metadata() {
        let entry = parse(
            "[Desktop Entry]\nType=Application\nName=Test App\nIcon=test-icon\nExec=test-app --label \"two words\" %U\n",
        );
        let application =
            application_from_entry(&entry, &["en_US".into()], &["kde".into()], "hicolor")
                .expect("visible application");
        assert_eq!(application.name(), "Test App");
        assert_eq!(application.icon(), Some("test-icon"));
        assert_eq!(
            application.launch_command(),
            Some(
                [
                    "test-app".to_owned(),
                    "--label".to_owned(),
                    "two words".to_owned()
                ]
                .as_slice()
            )
        );
    }

    #[test]
    fn filters_hidden_and_desktop_specific_entries() {
        let hidden =
            parse("[Desktop Entry]\nType=Application\nName=Hidden\nExec=hidden\nNoDisplay=true\n");
        assert!(application_from_entry(&hidden, &[], &["kde".into()], "hicolor").is_none());

        let gnome_only = parse(
            "[Desktop Entry]\nType=Application\nName=GNOME Tool\nExec=tool\nOnlyShowIn=GNOME;\n",
        );
        assert!(application_from_entry(&gnome_only, &[], &["kde".into()], "hicolor").is_none());
    }

    #[test]
    fn reads_kde_icon_theme_without_a_configuration_dependency() {
        let config = "[General]\nColorScheme=BreezeDark\n\n[Icons]\nTheme=breeze-dark\n";
        assert_eq!(
            value_in_section(config, "Icons", "Theme").as_deref(),
            Some("breeze-dark")
        );
    }
}
