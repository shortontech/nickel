use std::{
    collections::HashSet,
    env, fs,
    path::{Path, PathBuf},
};

use freedesktop_desktop_entry::{
    DesktopEntry, Iter, current_desktop, default_paths, get_languages_from_env,
};

use crate::{
    launcher::Application,
    model::{ApplicationDiscovery, ApplicationDiscoveryReport, ApplicationSkipReason},
};

pub fn load_applications() -> ApplicationDiscovery {
    let locales = get_languages_from_env();
    let desktops = current_desktop().unwrap_or_default();
    let icon_theme = icon_theme();
    let discovery = discover_entries(
        Iter::new(default_paths())
            .map(|path| DesktopEntry::from_path(path, Some(&locales)).map_err(|_| ())),
        &locales,
        &desktops,
        &icon_theme,
    );
    tracing::info!(
        scanned = discovery.report().scanned(),
        accepted = discovery.report().accepted(),
        parse_failures = discovery.report().skipped(ApplicationSkipReason::ParseFailure),
        unsupported_type = discovery.report().skipped(ApplicationSkipReason::UnsupportedType),
        hidden = discovery.report().skipped(ApplicationSkipReason::Hidden),
        no_display = discovery.report().skipped(ApplicationSkipReason::NoDisplay),
        wrong_desktop = discovery.report().skipped(ApplicationSkipReason::WrongDesktop),
        missing_name = discovery.report().skipped(ApplicationSkipReason::MissingName),
        empty_name = discovery.report().skipped(ApplicationSkipReason::EmptyName),
        missing_exec = discovery.report().skipped(ApplicationSkipReason::MissingExec),
        invalid_exec = discovery.report().skipped(ApplicationSkipReason::InvalidExec),
        status = ?discovery.status(),
        "desktop-entry discovery complete"
    );
    discovery
}

fn discover_entries<I>(
    entries: I,
    locales: &[String],
    desktops: &[String],
    icon_theme: &str,
) -> ApplicationDiscovery
where
    I: IntoIterator<Item = Result<DesktopEntry, ()>>,
{
    let mut seen = HashSet::new();
    let mut applications = Vec::new();
    let mut report = ApplicationDiscoveryReport::new();
    for parsed in entries {
        report.record_scanned();
        let Ok(entry) = parsed else {
            report.record(ApplicationSkipReason::ParseFailure);
            continue;
        };
        // Higher-priority XDG directories appear first. Hidden entries must also
        // shadow a lower-priority entry with the same application ID.
        if !seen.insert(entry.id().to_owned()) {
            continue;
        }
        match application_from_entry_result(&entry, locales, desktops, icon_theme) {
            Ok(application) => applications.push(application),
            Err(reason) => report.record(reason),
        }
    }
    applications.sort_by(|left, right| {
        left.name()
            .to_lowercase()
            .cmp(&right.name().to_lowercase())
            .then_with(|| left.id().cmp(right.id()))
    });
    ApplicationDiscovery::from_report(applications, report)
}

fn application_from_entry(
    entry: &DesktopEntry,
    locales: &[String],
    desktops: &[String],
    icon_theme: &str,
) -> Option<Application> {
    application_from_entry_result(entry, locales, desktops, icon_theme).ok()
}

fn application_from_entry_result(
    entry: &DesktopEntry,
    locales: &[String],
    desktops: &[String],
    icon_theme: &str,
) -> Result<Application, ApplicationSkipReason> {
    if entry.type_() != Some("Application") {
        return Err(ApplicationSkipReason::UnsupportedType);
    }
    if entry.hidden() {
        return Err(ApplicationSkipReason::Hidden);
    }
    if entry.no_display() {
        return Err(ApplicationSkipReason::NoDisplay);
    }
    if !visible_on_desktop(entry, desktops) {
        return Err(ApplicationSkipReason::WrongDesktop);
    }

    let Some(raw_name) = entry.name(locales) else {
        return Err(ApplicationSkipReason::MissingName);
    };
    let name = raw_name.trim().to_owned();
    if name.is_empty() {
        return Err(ApplicationSkipReason::EmptyName);
    }
    let launch_command = match entry.exec() {
        Some(exec) => parse_exec(exec).ok_or(ApplicationSkipReason::InvalidExec)?,
        None if !entry.dbus_activatable() => return Err(ApplicationSkipReason::MissingExec),
        None => Vec::new(),
    };

    let icon = entry.icon().map(str::to_owned);
    let icon_path = icon
        .as_deref()
        .and_then(|name| resolve_icon(name, icon_theme));
    let mut application = Application::new(
        entry.id().to_owned(),
        name,
        icon,
        icon_path,
        (!launch_command.is_empty()).then_some(launch_command),
    );
    if let Some(startup_wm_class) = entry.startup_wm_class() {
        application = application.with_identity_alias(startup_wm_class);
    }
    Ok(application)
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

    use super::{
        application_from_entry, application_from_entry_result, discover_entries, value_in_section,
    };
    use crate::model::{ApplicationDiscoveryStatus, ApplicationSkipReason};

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
            "[Desktop Entry]\nType=Application\nName=Test App\nIcon=test-icon\nExec=test-app --label \"two words\" %U\nStartupWMClass=TestAppWindow\n",
        );
        let application =
            application_from_entry(&entry, &["en_US".into()], &["kde".into()], "hicolor")
                .expect("visible application");
        assert_eq!(application.name(), "Test App");
        assert_eq!(application.icon(), Some("test-icon"));
        assert!(application.matches_native_id("TestAppWindow"));
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

    #[test]
    fn classifies_parse_and_entry_failures_without_exposing_exec_text() {
        let missing_name = parse("[Desktop Entry]\nType=Application\nExec=missing-name\n");
        assert_eq!(
            application_from_entry_result(&missing_name, &[], &[], "hicolor"),
            Err(ApplicationSkipReason::MissingName)
        );
        let invalid_exec =
            parse("[Desktop Entry]\nType=Application\nName=Broken\nExec=broken \"unterminated\n");
        assert_eq!(
            application_from_entry_result(&invalid_exec, &[], &[], "hicolor"),
            Err(ApplicationSkipReason::InvalidExec)
        );
    }

    #[test]
    fn discovery_reports_partial_failure_separately_from_ready_empty() {
        let valid = parse("[Desktop Entry]\nType=Application\nName=Valid\nExec=valid\n");
        let discovery = discover_entries([Ok(valid), Err(())], &[], &[], "hicolor");
        assert_eq!(
            discovery.status(),
            ApplicationDiscoveryStatus::PartialFailure
        );
        assert_eq!(discovery.applications().len(), 1);
        assert_eq!(
            discovery
                .report()
                .skipped(ApplicationSkipReason::ParseFailure),
            1
        );

        let empty = discover_entries(Vec::<Result<DesktopEntry, ()>>::new(), &[], &[], "hicolor");
        assert_eq!(empty.status(), ApplicationDiscoveryStatus::ReadyEmpty);
    }
}
