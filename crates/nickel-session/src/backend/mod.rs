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

pub struct SessionArguments {
    pub backend: BackendKind,
    pub command: Option<(OsString, Vec<OsString>)>,
}

impl SessionArguments {
    pub fn parse(
        args: impl IntoIterator<Item = OsString>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let mut args = args.into_iter().peekable();
        let mut backend = if cfg!(feature = "backend-winit") {
            BackendKind::Winit
        } else {
            BackendKind::Udev
        };
        let mut command = None;

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
                _ => {
                    return Err(format!(
                        "unexpected argument {}; usage: nickel-session [--backend winit|udev] [--command PROGRAM [ARG ...]]",
                        argument.to_string_lossy()
                    )
                    .into());
                }
            }
        }

        if !backend.available() {
            let name = match backend {
                BackendKind::Winit => "winit",
                BackendKind::Udev => "udev",
            };
            return Err(BackendSelectionError::Unavailable(name).into());
        }

        Ok(Self { backend, command })
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::{BackendKind, SessionArguments};

    #[test]
    fn selects_nested_backend_explicitly() {
        let arguments =
            SessionArguments::parse([OsString::from("--backend"), OsString::from("nested")])
                .expect("nested backend should be available in default tests");
        assert_eq!(arguments.backend, BackendKind::Winit);
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
    }
}
