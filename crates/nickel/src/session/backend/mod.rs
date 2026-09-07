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

/// Selects how compositor-owned Nickel UI is presented.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum InternalUiRendererMode {
    #[default]
    Gpu,
    Software,
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
    pub ui_renderer: InternalUiRendererMode,
}

impl SessionArguments {
    pub fn parse(
        args: impl IntoIterator<Item = std::ffi::OsString>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let mut args = args.into_iter().peekable();
        let mut backend = if cfg!(feature = "backend-udev") {
            BackendKind::Udev
        } else {
            BackendKind::Winit
        };
        let mut test_control = false;
        let mut ui_renderer = InternalUiRendererMode::Gpu;
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
                Some("--test-control") => test_control = true,
                Some("--ui-renderer") => {
                    let value = args
                        .next()
                        .ok_or_else(|| "--ui-renderer requires gpu or software".to_owned())?;
                    ui_renderer = match value.to_str() {
                        Some("gpu") => InternalUiRendererMode::Gpu,
                        Some("software") => InternalUiRendererMode::Software,
                        _ => return Err("--ui-renderer expects gpu or software".into()),
                    };
                }
                _ => {
                    return Err(format!(
                        "unexpected argument {}; usage: nickel [--backend winit|udev] [--test-control] [--ui-renderer gpu|software]",
                        argument.to_string_lossy()
                    )
                    .into());
                }
            }
        }

        if test_control && backend != BackendKind::Winit {
            return Err("--test-control is available only with the nested backend".into());
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
            ui_renderer,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::{BackendKind, InternalUiRendererMode, SessionArguments};

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

    #[cfg(feature = "backend-winit")]
    #[test]
    fn internal_shell_is_default_and_keeps_test_control_available() {
        let arguments = SessionArguments::parse([
            OsString::from("--backend"),
            OsString::from("nested"),
            OsString::from("--test-control"),
        ])
        .expect("external test control is independent of shell supervision");
        assert!(arguments.test_control);
        assert_eq!(arguments.ui_renderer, InternalUiRendererMode::Gpu);
    }

    #[cfg(feature = "backend-winit")]
    #[test]
    fn software_ui_renderer_is_an_explicit_backend_independent_override() {
        let arguments = SessionArguments::parse([
            OsString::from("--backend"),
            OsString::from("nested"),
            OsString::from("--ui-renderer"),
            OsString::from("software"),
        ])
        .expect("software fallback should be accepted by the nested backend");
        assert_eq!(arguments.ui_renderer, InternalUiRendererMode::Software);
    }

    #[cfg(feature = "backend-udev")]
    #[test]
    fn software_ui_renderer_is_accepted_by_the_native_backend() {
        let arguments = SessionArguments::parse([
            OsString::from("--backend"),
            OsString::from("udev"),
            OsString::from("--ui-renderer"),
            OsString::from("software"),
        ])
        .expect("software fallback should be accepted by the native backend");
        assert_eq!(arguments.backend, BackendKind::Udev);
        assert_eq!(arguments.ui_renderer, InternalUiRendererMode::Software);
    }

    #[test]
    fn legacy_shell_process_options_are_rejected() {
        let error = SessionArguments::parse([
            OsString::from("--shell-process"),
            OsString::from("supervised"),
        ])
        .expect_err("the out-of-process shell is retired");
        assert!(error.to_string().contains("unexpected argument"));
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
