use std::collections::HashSet;

use freedesktop_desktop_entry::{
    DesktopEntry, Iter, current_desktop, default_paths, get_languages_from_env,
};

use crate::launcher::Application;

pub fn load_applications() -> Vec<Application> {
    let locales = get_languages_from_env();
    let desktops = current_desktop().unwrap_or_default();
    let mut seen = HashSet::new();
    let mut applications = Vec::new();

    for entry in Iter::new(default_paths()).entries(Some(&locales)) {
        // Higher-priority XDG directories appear first. Hidden entries must also
        // shadow a lower-priority entry with the same application ID.
        if !seen.insert(entry.id().to_owned()) {
            continue;
        }
        if let Some(application) = application_from_entry(&entry, &locales, &desktops) {
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

    Some(Application::new(
        entry.id().to_owned(),
        name,
        entry.icon().map(str::to_owned),
        entry.exec().map(str::to_owned),
    ))
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

    use super::application_from_entry;

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
            "[Desktop Entry]\nType=Application\nName=Test App\nIcon=test-icon\nExec=test-app %U\n",
        );
        let application = application_from_entry(&entry, &["en_US".into()], &["kde".into()])
            .expect("visible application");
        assert_eq!(application.name(), "Test App");
        assert_eq!(application.icon(), Some("test-icon"));
        assert_eq!(application.exec(), Some("test-app %U"));
    }

    #[test]
    fn filters_hidden_and_desktop_specific_entries() {
        let hidden =
            parse("[Desktop Entry]\nType=Application\nName=Hidden\nExec=hidden\nNoDisplay=true\n");
        assert!(application_from_entry(&hidden, &[], &["kde".into()]).is_none());

        let gnome_only = parse(
            "[Desktop Entry]\nType=Application\nName=GNOME Tool\nExec=tool\nOnlyShowIn=GNOME;\n",
        );
        assert!(application_from_entry(&gnome_only, &[], &["kde".into()]).is_none());
    }
}
