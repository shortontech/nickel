use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{
        Arc, OnceLock, RwLock,
        atomic::{AtomicU64, Ordering},
    },
};

use freedesktop_desktop_entry::{
    DesktopEntry, Iter, current_desktop, default_paths, get_languages_from_env,
};

use crate::{
    launcher::Application,
    model::{
        ApplicationDiscovery, ApplicationDiscoveryReport, ApplicationLaunchClass,
        ApplicationSkipReason,
    },
};

const RUN_SIGNATURE_ENTRY_LIMIT: usize = 4_096;
const RUN_SIGNATURE_BYTE_LIMIT: usize = 4 * 1024 * 1024;
static RUN_SIGNATURE_GENERATION: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RunSignatureDiagnostics {
    pub generation: u64,
    pub entries: usize,
    pub retained_bytes: usize,
    pub skipped: usize,
}

#[derive(Clone, Debug, Default)]
struct RunSignatureIndex {
    applications: Vec<Application>,
    diagnostics: RunSignatureDiagnostics,
}

impl RunSignatureIndex {
    fn build(applications: &[Application], generation: u64) -> Self {
        let mut indexed = Vec::new();
        let mut retained_bytes = 0_usize;
        let mut skipped = 0_usize;
        for application in applications {
            let Some(command) = application.launch_command() else {
                continue;
            };
            let bytes = application.id().len()
                + application.name().len()
                + command.iter().map(String::len).sum::<usize>()
                + application
                    .working_directory()
                    .map_or(0, |path| path.as_os_str().len());
            if indexed.len() >= RUN_SIGNATURE_ENTRY_LIMIT
                || retained_bytes.saturating_add(bytes) > RUN_SIGNATURE_BYTE_LIMIT
            {
                skipped += 1;
                continue;
            }
            retained_bytes += bytes;
            indexed.push(application.clone());
        }
        Self {
            diagnostics: RunSignatureDiagnostics {
                generation,
                entries: indexed.len(),
                retained_bytes,
                skipped,
            },
            applications: indexed,
        }
    }

    fn exact(&self, arguments: &[String]) -> Option<Application> {
        let mut matches = self
            .applications
            .iter()
            .filter(|application| application.launch_command() == Some(arguments));
        let application = matches.next()?.clone();
        // A shared signature does not identify which desktop entry's cwd, startup identity, or
        // terminal policy was intended. Ambiguity therefore retains deferred observation.
        matches.next().is_none().then_some(application)
    }
}

fn run_signature_authority() -> &'static RwLock<Arc<RunSignatureIndex>> {
    static AUTHORITY: OnceLock<RwLock<Arc<RunSignatureIndex>>> = OnceLock::new();
    AUTHORITY.get_or_init(|| RwLock::new(Arc::new(RunSignatureIndex::default())))
}

fn publish_run_signatures(applications: &[Application]) {
    let generation = RUN_SIGNATURE_GENERATION.fetch_add(1, Ordering::AcqRel) + 1;
    let snapshot = Arc::new(RunSignatureIndex::build(applications, generation));
    *run_signature_authority()
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = snapshot;
}

pub fn classify_run_application(arguments: &[String]) -> Option<Application> {
    run_signature_authority()
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .exact(arguments)
}

pub fn run_signature_diagnostics() -> RunSignatureDiagnostics {
    run_signature_authority()
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .diagnostics
}

pub fn load_applications() -> ApplicationDiscovery {
    let locales = get_languages_from_env();
    let desktops = current_desktop().unwrap_or_default();
    let icon_theme = icon_theme();
    let discovery = discover_entries(
        Iter::new(default_paths())
            .map(|path| nickel_platform::desktop_entry_from_path(&path, Some(&locales)).ok_or(())),
        &locales,
        &desktops,
        &icon_theme,
    );
    publish_run_signatures(discovery.applications());
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
        invalid_terminal = discovery.report().skipped(ApplicationSkipReason::InvalidTerminal),
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
    if !nickel_platform::desktop_entry_is_application(entry) {
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
    let launch_class = match entry.desktop_entry("Terminal") {
        None | Some("false") => ApplicationLaunchClass::Graphical,
        Some("true") => ApplicationLaunchClass::Terminal,
        Some(_) => return Err(ApplicationSkipReason::InvalidTerminal),
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
    )
    .with_launch_policy(launch_class, entry.path().map(PathBuf::from));
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
    nickel_platform::system_icon_theme()
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
        RUN_SIGNATURE_ENTRY_LIMIT, RunSignatureIndex, application_from_entry,
        application_from_entry_result, discover_entries,
    };
    use crate::model::{
        Application, ApplicationDiscoveryStatus, ApplicationLaunchClass, ApplicationSkipReason,
    };

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
    fn terminal_and_working_directory_are_canonical_launch_metadata() {
        let entry = parse(
            "[Desktop Entry]\nType=Application\nName=Console Tool\nExec=tool --label \"two words\"\nTerminal=true\nPath=/tmp\n",
        );
        let application = application_from_entry(&entry, &[], &[], "hicolor").unwrap();
        assert_eq!(
            application.launch_class(),
            crate::model::ApplicationLaunchClass::Terminal
        );
        assert_eq!(
            application.working_directory(),
            Some(std::path::Path::new("/tmp"))
        );
        assert_eq!(
            application.launch_command(),
            Some(
                [
                    "tool".to_owned(),
                    "--label".to_owned(),
                    "two words".to_owned()
                ]
                .as_slice()
            )
        );
    }

    #[test]
    fn malformed_terminal_value_rejects_the_entry_instead_of_guessing() {
        let entry = parse(
            "[Desktop Entry]\nType=Application\nName=Ambiguous\nExec=tool\nTerminal=perhaps\n",
        );
        assert_eq!(
            application_from_entry_result(&entry, &[], &[], "hicolor"),
            Err(ApplicationSkipReason::InvalidTerminal)
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

    fn indexed_application(
        id: &str,
        command: &[&str],
        class: ApplicationLaunchClass,
    ) -> Application {
        Application::new(
            id.to_owned(),
            id.to_owned(),
            None,
            None,
            Some(
                command
                    .iter()
                    .map(|argument| (*argument).to_owned())
                    .collect(),
            ),
        )
        .with_launch_policy(class, None)
    }

    #[test]
    fn run_signature_index_matches_only_one_exact_canonical_launch() {
        let graphical = indexed_application(
            "org.example.Editor.desktop",
            &["editor", "--new-window"],
            ApplicationLaunchClass::Graphical,
        );
        let terminal = indexed_application(
            "org.example.Tool.desktop",
            &["tool", "--interactive"],
            ApplicationLaunchClass::Terminal,
        );
        let index = RunSignatureIndex::build(&[graphical, terminal], 7);

        let matched = index
            .exact(&["editor".into(), "--new-window".into()])
            .expect("exact desktop launch signature");
        assert_eq!(matched.launch_class(), ApplicationLaunchClass::Graphical);
        assert!(
            index
                .exact(&["editor".into(), "--version".into()])
                .is_none()
        );
        assert_eq!(index.diagnostics.generation, 7);
    }

    #[test]
    fn ambiguous_and_over_budget_signatures_remain_conservative() {
        let one = indexed_application(
            "one.desktop",
            &["shared"],
            ApplicationLaunchClass::Graphical,
        );
        let two = indexed_application("two.desktop", &["shared"], ApplicationLaunchClass::Terminal);
        let ambiguous = RunSignatureIndex::build(&[one, two], 1);
        assert!(ambiguous.exact(&["shared".into()]).is_none());

        let applications = (0..RUN_SIGNATURE_ENTRY_LIMIT + 3)
            .map(|index| {
                indexed_application(
                    &format!("app-{index}.desktop"),
                    &[&format!("app-{index}")],
                    ApplicationLaunchClass::Graphical,
                )
            })
            .collect::<Vec<_>>();
        let bounded = RunSignatureIndex::build(&applications, 2);
        assert_eq!(bounded.diagnostics.entries, RUN_SIGNATURE_ENTRY_LIMIT);
        assert_eq!(bounded.diagnostics.skipped, 3);
    }
}
