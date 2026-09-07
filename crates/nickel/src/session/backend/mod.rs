use std::ffi::OsString;

use thiserror::Error;

#[cfg(feature = "backend-udev")]
mod drm_scanner;
#[cfg(feature = "backend-udev")]
mod session_activity;
#[cfg(feature = "backend-udev")]
pub use nickel_core::output_layout::OutputLayout;
#[cfg(feature = "backend-udev")]
pub use session_activity::SessionActivity;

#[cfg(feature = "backend-udev")]
pub mod udev;
#[cfg(feature = "backend-winit")]
pub mod winit;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackendKind {
    Winit,
    Udev,
}

/// Selects whether the compositor starts the transitional out-of-process shell.
///
/// `Disabled` is the migration seam used by the compositor-owned UI host. It
/// deliberately leaves the external control protocol available for probes and
/// acceptance tests; it only removes shell child lifecycle ownership.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ShellProcessMode {
    #[default]
    Supervised,
    Disabled,
}

impl BackendKind {
    pub fn parse(value: &str) -> Result<Self, BackendSelectionError> {
        match value {
            "winit" | "nested" => Ok(Self::Winit),
            "udev" | "drm" | "native" => Ok(Self::Udev),
            _ => Err(BackendSelectionError::Unknown(value.to_owned())),
        }
    }

    pub fn available(self) -> bool {
        match self {
            Self::Winit => cfg!(feature = "backend-winit"),
            Self::Udev => cfg!(feature = "backend-udev"),
        }
    }
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum BackendSelectionError {
    #[error("unknown backend '{0}'; expected 'winit' or 'udev'")]
    Unknown(String),
    #[error("backend '{0}' is not compiled in")]
    Unavailable(&'static str),
    #[error("--backend requires a value")]
    MissingValue,
}

#[derive(Debug)]
pub struct SessionArguments {
    pub backend: BackendKind,
    pub test_control: bool,
    pub command: Option<(OsString, Vec<OsString>)>,
    pub shell_process: ShellProcessMode,
}

impl SessionArguments {
    pub fn parse(
        args: impl IntoIterator<Item = OsString>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let mut args = args.into_iter().peekable();
        let mut backend = if cfg!(feature = "backend-udev") {
            BackendKind::Udev
        } else {
            BackendKind::Winit
        };
        let mut test_control = false;
        let mut command = None;
        let mut shell_process = ShellProcessMode::Supervised;

        while let Some(argument) = args.next() {
            match argument.to_str() {
                Some("--backend") => {
                    let value = args.next().ok_or(BackendSelectionError::MissingValue)?;
                    backend = BackendKind::parse(
                        value
                            .to_str()
                            .ok_or_else(|| "backend name is not valid UTF-8".to_owned())?,
                    )?;
                }
                Some("-c" | "--command") => {
                    let program = args
                        .next()
                        .ok_or_else(|| "--command requires a program".to_owned())?;
                    command = Some((program, args.collect()));
                    break;
                }
                Some("--shell-process") => {
                    let value = args.next().ok_or_else(|| {
                        "--shell-process requires supervised or disabled".to_owned()
                    })?;
                    shell_process = match value.to_str() {
                        Some("supervised") => ShellProcessMode::Supervised,
                        Some("disabled") => ShellProcessMode::Disabled,
                        _ => return Err("--shell-process expects supervised or disabled".into()),
                    };
                }
                Some("--test-control") => test_control = true,
                _ => {
                    return Err(format!(
                        "unexpected argument {}; usage: nickel [--backend winit|udev] [--test-control] [--shell-process supervised|disabled] [--command PROGRAM [ARG ...]]",
                        argument.to_string_lossy()
                    )
                    .into());
                }
            }
        }

        if test_control && backend != BackendKind::Winit {
            return Err("--test-control is available only with the nested backend".into());
        }

        if shell_process == ShellProcessMode::Disabled && command.is_some() {
            return Err("--command cannot be combined with --shell-process disabled".into());
        }

        if !backend.available() {
            let name = match backend {
                BackendKind::Winit => "winit",
                BackendKind::Udev => "udev",
            };
            return Err(BackendSelectionError::Unavailable(name).into());
        }

        Ok(Self {
            backend,
            test_control,
            command,
            shell_process,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::{BackendKind, SessionArguments, ShellProcessMode};

    #[cfg(feature = "backend-winit")]
    #[test]
    fn selects_nested_backend_explicitly() {
        let arguments =
            SessionArguments::parse([OsString::from("--backend"), OsString::from("nested")])
                .expect("nested backend should be available in default tests");
        assert_eq!(arguments.backend, BackendKind::Winit);
        assert!(!arguments.test_control);
    }

    #[cfg(feature = "backend-udev")]
    #[test]
    fn defaults_to_native_backend_when_udev_is_available() {
        let arguments = SessionArguments::parse([]).expect("default backend should be available");
        assert_eq!(arguments.backend, BackendKind::Udev);
    }

    #[test]
    fn command_consumes_remaining_arguments() {
        let arguments = SessionArguments::parse([
            OsString::from("--command"),
            OsString::from("nickel"),
            OsString::from("--example"),
        ])
        .expect("command should parse");
        let (program, arguments) = arguments.command.expect("command should be present");
        assert_eq!(program, "nickel");
        assert_eq!(arguments, [OsString::from("--example")]);
        assert_eq!(
            SessionArguments::parse([])
                .expect("default arguments should parse")
                .shell_process,
            ShellProcessMode::Supervised
        );
    }

    #[cfg(feature = "backend-winit")]
    #[test]
    fn shell_process_can_be_disabled_without_disabling_test_control() {
        let arguments = SessionArguments::parse([
            OsString::from("--backend"),
            OsString::from("nested"),
            OsString::from("--test-control"),
            OsString::from("--shell-process"),
            OsString::from("disabled"),
        ])
        .expect("external test control is independent of shell supervision");
        assert!(arguments.test_control);
        assert_eq!(arguments.shell_process, ShellProcessMode::Disabled);
        assert!(arguments.command.is_none());
    }

    #[test]
    fn disabled_shell_process_rejects_a_child_command() {
        let error = SessionArguments::parse([
            OsString::from("--shell-process"),
            OsString::from("disabled"),
            OsString::from("--command"),
            OsString::from("nickel"),
        ])
        .expect_err("disabled supervision cannot accept a child command");
        assert!(error.to_string().contains("cannot be combined"));
    }

    #[cfg(feature = "backend-winit")]
    #[test]
    fn test_control_is_explicit_and_nested_only() {
        let arguments = SessionArguments::parse([
            OsString::from("--backend"),
            OsString::from("nested"),
            OsString::from("--test-control"),
        ])
        .expect("nested test control should be accepted");
        assert!(arguments.test_control);

        let error = SessionArguments::parse([
            OsString::from("--backend"),
            OsString::from("udev"),
            OsString::from("--test-control"),
        ])
        .expect_err("native test control must be rejected");
        assert!(error.to_string().contains("only with the nested backend"));
    }
}
