use std::{
    io,
    path::{Path, PathBuf},
    process::{Child, Command},
};

#[derive(Clone, Debug, PartialEq)]
pub struct TrayItem {
    pub id: String,
    pub title: String,
    pub icon: image::RgbaImage,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ApplicationId(String);

impl ApplicationId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Application {
    id: ApplicationId,
    identity_aliases: Vec<String>,
    name: String,
    icon: Option<String>,
    icon_path: Option<PathBuf>,
    launch_command: Option<Vec<String>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApplicationDiscoveryStatus {
    ReadyEmpty,
    Ready,
    PartialFailure,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ApplicationSkipReason {
    ParseFailure,
    UnsupportedType,
    Hidden,
    NoDisplay,
    WrongDesktop,
    MissingName,
    EmptyName,
    MissingExec,
    InvalidExec,
}

impl ApplicationSkipReason {
    const ALL: [Self; 9] = [
        Self::ParseFailure,
        Self::UnsupportedType,
        Self::Hidden,
        Self::NoDisplay,
        Self::WrongDesktop,
        Self::MissingName,
        Self::EmptyName,
        Self::MissingExec,
        Self::InvalidExec,
    ];

    const fn index(self) -> usize {
        match self {
            Self::ParseFailure => 0,
            Self::UnsupportedType => 1,
            Self::Hidden => 2,
            Self::NoDisplay => 3,
            Self::WrongDesktop => 4,
            Self::MissingName => 5,
            Self::EmptyName => 6,
            Self::MissingExec => 7,
            Self::InvalidExec => 8,
        }
    }

    const fn is_failure(self) -> bool {
        matches!(
            self,
            Self::ParseFailure | Self::MissingName | Self::InvalidExec
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApplicationDiscoveryReport {
    scanned: usize,
    accepted: usize,
    skipped: [usize; ApplicationSkipReason::ALL.len()],
}

impl ApplicationDiscoveryReport {
    pub(crate) fn new() -> Self {
        Self {
            scanned: 0,
            accepted: 0,
            skipped: [0; ApplicationSkipReason::ALL.len()],
        }
    }

    pub fn scanned(&self) -> usize {
        self.scanned
    }

    pub fn accepted(&self) -> usize {
        self.accepted
    }

    pub fn skipped(&self, reason: ApplicationSkipReason) -> usize {
        self.skipped[reason.index()]
    }

    pub fn has_failures(&self) -> bool {
        ApplicationSkipReason::ALL
            .into_iter()
            .any(|reason| reason.is_failure() && self.skipped(reason) > 0)
    }

    pub(crate) fn record_scanned(&mut self) {
        self.scanned += 1;
    }

    pub(crate) fn record(&mut self, reason: ApplicationSkipReason) {
        self.skipped[reason.index()] += 1;
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApplicationDiscovery {
    applications: Vec<Application>,
    status: ApplicationDiscoveryStatus,
    report: ApplicationDiscoveryReport,
}

impl ApplicationDiscovery {
    pub fn ready(applications: Vec<Application>) -> Self {
        let status = if applications.is_empty() {
            ApplicationDiscoveryStatus::ReadyEmpty
        } else {
            ApplicationDiscoveryStatus::Ready
        };
        let report = ApplicationDiscoveryReport {
            scanned: applications.len(),
            accepted: applications.len(),
            skipped: [0; ApplicationSkipReason::ALL.len()],
        };
        Self {
            applications,
            status,
            report,
        }
    }

    pub fn status(&self) -> ApplicationDiscoveryStatus {
        self.status
    }

    pub fn report(&self) -> &ApplicationDiscoveryReport {
        &self.report
    }

    pub fn applications(&self) -> &[Application] {
        &self.applications
    }

    pub fn into_applications(self) -> Vec<Application> {
        self.applications
    }

    pub(crate) fn from_report(
        applications: Vec<Application>,
        mut report: ApplicationDiscoveryReport,
    ) -> Self {
        report.accepted = applications.len();
        let status = if report.has_failures() {
            ApplicationDiscoveryStatus::PartialFailure
        } else if applications.is_empty() {
            ApplicationDiscoveryStatus::ReadyEmpty
        } else {
            ApplicationDiscoveryStatus::Ready
        };
        Self {
            applications,
            status,
            report,
        }
    }
}

impl Application {
    pub fn new(
        id: String,
        name: String,
        icon: Option<String>,
        icon_path: Option<PathBuf>,
        launch_command: Option<Vec<String>>,
    ) -> Self {
        Self {
            id: ApplicationId::new(id),
            identity_aliases: Vec::new(),
            name,
            icon,
            icon_path,
            launch_command,
        }
    }

    pub fn id(&self) -> &str {
        self.id.as_str()
    }

    pub fn application_id(&self) -> &ApplicationId {
        &self.id
    }

    pub fn with_identity_alias(mut self, alias: impl Into<String>) -> Self {
        let alias = alias.into();
        if !alias.is_empty() && !self.identity_aliases.iter().any(|known| known == &alias) {
            self.identity_aliases.push(alias);
        }
        self
    }

    pub fn matches_native_id(&self, native_id: &str) -> bool {
        let native_id = native_id.trim_end_matches(".desktop");
        std::iter::once(self.id())
            .chain(self.identity_aliases.iter().map(String::as_str))
            .any(|candidate| {
                candidate
                    .trim_end_matches(".desktop")
                    .eq_ignore_ascii_case(native_id)
            })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn icon(&self) -> Option<&str> {
        self.icon.as_deref()
    }

    pub fn icon_path(&self) -> Option<&Path> {
        self.icon_path.as_deref()
    }

    pub fn launch_command(&self) -> Option<&[String]> {
        self.launch_command.as_deref()
    }

    pub fn launch(&self) -> io::Result<Child> {
        self.process()?.spawn()
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn launch_as_session_client(&self) -> io::Result<Child> {
        self.session_process()?.spawn()
    }

    fn process(&self) -> io::Result<Command> {
        let mut command = self.process_with_capabilities()?;
        // These capabilities authenticate trusted Nickel session clients.
        // They must never cross into an ordinary application.
        command
            .env_remove("NICKEL_SESSION_CONTROL")
            .env_remove("NICKEL_SESSION_TOKEN")
            .env_remove("NICKEL_SHELL_TEST_CONTROL");
        Ok(command)
    }

    fn process_with_capabilities(&self) -> io::Result<Command> {
        let (program, arguments) = self
            .launch_command
            .as_deref()
            .and_then(|command| command.split_first())
            .ok_or_else(|| io::Error::other("application has no launch command"))?;
        let mut command = Command::new(program);
        command.args(arguments);
        if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
            command.current_dir(home);
        }
        Ok(command)
    }

    #[cfg(target_os = "linux")]
    fn session_process(&self) -> io::Result<Command> {
        let mut command = self.process_with_capabilities()?;
        command.env_remove("NICKEL_SHELL_TEST_CONTROL");
        Ok(command)
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct WindowId(pub u64);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenWindow {
    pub id: WindowId,
    pub application_id: Option<ApplicationId>,
    pub active: bool,
    pub title: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WindowGroup {
    pub application_id: Option<ApplicationId>,
    pub application_name: String,
    pub windows: Vec<OpenWindow>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WindowPreview {
    pub window: WindowId,
    pub image: image::RgbaImage,
}

impl WindowGroup {
    pub fn active(&self) -> bool {
        self.windows.iter().any(|window| window.active)
    }
}

#[cfg(test)]
mod tests {
    use super::{Application, ApplicationId};

    #[test]
    fn application_ids_are_opaque() {
        let id = ApplicationId::new("org.example.App.desktop");
        assert_eq!(id.as_str(), "org.example.App.desktop");
    }

    #[test]
    fn launched_applications_do_not_inherit_session_capabilities() {
        let application = Application::new(
            "test".into(),
            "Test".into(),
            None,
            None,
            Some(vec!["true".into()]),
        );
        let command = application.process().unwrap();
        let removals = command
            .get_envs()
            .filter_map(|(name, value)| value.is_none().then_some(name))
            .collect::<Vec<_>>();
        assert!(removals.contains(&std::ffi::OsStr::new("NICKEL_SESSION_CONTROL")));
        assert!(removals.contains(&std::ffi::OsStr::new("NICKEL_SESSION_TOKEN")));
        assert!(removals.contains(&std::ffi::OsStr::new("NICKEL_SHELL_TEST_CONTROL")));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn trusted_session_client_retains_only_production_session_capabilities() {
        let application = Application::new(
            "nickel-settings".into(),
            "Nickel Settings".into(),
            None,
            None,
            Some(vec!["true".into()]),
        );
        let command = application.session_process().unwrap();
        let removals = command
            .get_envs()
            .filter_map(|(name, value)| value.is_none().then_some(name))
            .collect::<Vec<_>>();
        assert!(!removals.contains(&std::ffi::OsStr::new("NICKEL_SESSION_CONTROL")));
        assert!(!removals.contains(&std::ffi::OsStr::new("NICKEL_SESSION_TOKEN")));
        assert!(removals.contains(&std::ffi::OsStr::new("NICKEL_SHELL_TEST_CONTROL")));
    }
}
