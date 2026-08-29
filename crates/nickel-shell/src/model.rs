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
    name: String,
    icon: Option<String>,
    icon_path: Option<PathBuf>,
    launch_command: Option<Vec<String>>,
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

    fn process(&self) -> io::Result<Command> {
        let (program, arguments) = self
            .launch_command
            .as_deref()
            .and_then(|command| command.split_first())
            .ok_or_else(|| io::Error::other("application has no launch command"))?;
        let mut command = Command::new(program);
        command.args(arguments);
        // These capabilities authenticate the trusted Nickel shell to its
        // compositor. They must never cross into an ordinary application.
        command
            .env_remove("NICKEL_SESSION_CONTROL")
            .env_remove("NICKEL_SESSION_TOKEN");
        if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
            command.current_dir(home);
        }
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
    }
}
