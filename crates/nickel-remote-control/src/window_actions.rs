use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WindowAction {
    Activate,
    Minimize,
    Maximize,
    Restore,
    Fullscreen,
    ExitFullscreen,
    Close,
    /// Move this authorized window to a live session-owned workspace.
    MoveToWorkspace {
        workspace: u64,
    },
    /// Global logical client bounds. Keep the existing position or size to
    /// perform only a resize or move. Native clients may negotiate the size.
    SetBounds {
        x: i32,
        y: i32,
        width: u32,
        height: u32,
    },
}

impl WindowAction {
    pub fn validate(self) -> Result<(), String> {
        if let Self::SetBounds {
            x,
            y,
            width,
            height,
        } = self
            && (!(1..=16_384).contains(&width)
                || !(1..=16_384).contains(&height)
                || x.checked_add(width as i32).is_none()
                || y.checked_add(height as i32).is_none())
        {
            return Err("window bounds exceed operation limits".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct WindowOutcome {
    pub requested: WindowAction,
    /// Confirmation reflects current production window state, not receipt of a command or pixels.
    pub confirmed: bool,
    pub window: Option<crate::WindowSummary>,
}

impl WindowOutcome {
    pub fn observed(requested: WindowAction, window: Option<crate::WindowSummary>) -> Self {
        let confirmed = match (&requested, &window) {
            (WindowAction::Close, None) => true,
            (WindowAction::Activate, Some(window)) => window.active && !window.minimized,
            (WindowAction::Minimize, Some(window)) => window.minimized,
            (WindowAction::Maximize, Some(window)) => {
                window.maximized && !window.fullscreen && !window.minimized
            }
            (WindowAction::Restore, Some(window)) => {
                !window.minimized && !window.maximized && !window.fullscreen
            }
            (WindowAction::Fullscreen, Some(window)) => window.fullscreen && !window.minimized,
            (WindowAction::ExitFullscreen, Some(window)) => !window.fullscreen,
            (WindowAction::MoveToWorkspace { workspace }, Some(window)) => {
                window.workspace == *workspace
            }
            (
                WindowAction::SetBounds {
                    x,
                    y,
                    width,
                    height,
                },
                Some(window),
            ) => {
                window.x == *x
                    && window.y == *y
                    && window.width == *width
                    && window.height == *height
            }
            _ => false,
        };
        Self {
            requested,
            confirmed,
            window,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_dialog_and_unsettled_fullscreen_do_not_count_as_confirmed_actions() {
        let window = crate::WindowSummary {
            id: "1".into(),
            application_id: "app".into(),
            title: "Document".into(),
            active: true,
            minimized: false,
            maximized: true,
            fullscreen: false,
            x: 5,
            y: 45,
            width: 1000,
            height: 600,
            generation: 1,
            workspace: 1,
            verified_application: None,
        };
        assert!(!WindowOutcome::observed(WindowAction::Close, Some(window.clone())).confirmed);
        assert!(!WindowOutcome::observed(WindowAction::Fullscreen, Some(window.clone())).confirmed);
        assert!(!WindowOutcome::observed(WindowAction::Restore, Some(window.clone())).confirmed);
        assert!(
            WindowOutcome::observed(
                WindowAction::MoveToWorkspace { workspace: 1 },
                Some(window.clone())
            )
            .confirmed
        );
        assert!(
            !WindowOutcome::observed(
                WindowAction::MoveToWorkspace { workspace: 2 },
                Some(window.clone())
            )
            .confirmed
        );
        assert!(
            !WindowOutcome::observed(
                WindowAction::SetBounds {
                    x: 5,
                    y: 45,
                    width: 1001,
                    height: 600
                },
                Some(window.clone()),
            )
            .confirmed,
            "receiving a configure request is not confirmation of the requested size"
        );
        assert!(WindowOutcome::observed(WindowAction::Maximize, Some(window)).confirmed);
        assert!(WindowOutcome::observed(WindowAction::Close, None).confirmed);
        assert!(!WindowOutcome::observed(WindowAction::Activate, None).confirmed);
    }

    #[test]
    fn bounds_reject_overflow_and_unbounded_sizes_but_allow_negative_monitor_coordinates() {
        for (x, y, width, height) in [
            (0, 0, 0, 200),
            (0, 0, 200, 0),
            (0, 0, u32::MAX, 200),
            (i32::MAX, 0, 200, 200),
            (0, i32::MAX, 200, 200),
        ] {
            assert!(
                WindowAction::SetBounds {
                    x,
                    y,
                    width,
                    height
                }
                .validate()
                .is_err()
            );
        }
        let action = WindowAction::SetBounds {
            x: -1600,
            y: -800,
            width: 640,
            height: 480,
        };
        assert!(action.validate().is_ok());
        let wire = serde_json::to_string(&action).unwrap();
        assert_eq!(serde_json::from_str::<WindowAction>(&wire).unwrap(), action);
    }
}
