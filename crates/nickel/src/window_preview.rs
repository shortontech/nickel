use nickel_core::theme::ThemePalette;
use nickel_ui::{Rect, SemanticTheme};

use crate::{
    model::{ApplicationId, OpenWindow, WindowGroup, WindowId},
    platform::WorkspaceSummary,
};

pub const CARD_WIDTH: f32 = 276.0;
pub const TASK_SWITCHER_CARD_WIDTH: f32 = 220.0;
pub const PREVIEW_HEIGHT: f32 = 214.0;
const GAP: f32 = 10.0;
const PADDING: f32 = 12.0;
pub const MENU_WIDTH: f32 = 220.0;
const MENU_ROW_HEIGHT: f32 = 32.0;
const MENU_ROW_GAP: f32 = 2.0;
const MENU_PADDING: f32 = 6.0;

pub(crate) fn semantic_theme_from_palette(palette: ThemePalette) -> SemanticTheme {
    SemanticTheme::from_tokens(nickel_ui::SemanticTokenSet::standard(
        palette.background,
        palette.panel,
        palette.surface,
        palette.surface_hover,
        palette.surface_hover,
        palette.text,
        palette.muted,
        palette.accent,
        palette.accent_soft,
        palette.complement,
        palette.complement,
    ))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PreviewAction {
    Activate(WindowId),
    Close(WindowId),
    OpenMenu(WindowId),
    Dismiss,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MenuAction {
    Dismiss,
    ShowWorkspaces,
    ShowDisplays,
    Back,
    Activate(WindowId),
    Close(WindowId),
    MaximizeRestore(WindowId),
    Minimize(WindowId),
    FullscreenRestore(WindowId),
    SnapLeading(WindowId),
    SnapTrailing(WindowId),
    MoveToWorkspace(WindowId, u64),
    MoveToDisplay(WindowId, String),
}

pub(crate) fn same_window_identity(captured: &OpenWindow, current: &OpenWindow) -> bool {
    captured.id == current.id && captured.application_id == current.application_id
}

pub(crate) fn window_menu_action_is_current(
    captured: &OpenWindow,
    current: &OpenWindow,
    action: &MenuAction,
) -> bool {
    if !same_window_identity(captured, current) {
        return false;
    }
    let id = current.id;
    let capabilities = current.state.capabilities;
    match action {
        MenuAction::Dismiss
        | MenuAction::ShowWorkspaces
        | MenuAction::ShowDisplays
        | MenuAction::Back => true,
        MenuAction::Activate(target) => *target == id && capabilities.activate,
        MenuAction::Close(target) => *target == id && capabilities.close,
        MenuAction::MaximizeRestore(target) => *target == id && capabilities.maximize,
        MenuAction::Minimize(target) => *target == id && capabilities.minimize,
        MenuAction::FullscreenRestore(target) => *target == id && capabilities.fullscreen,
        MenuAction::SnapLeading(target) | MenuAction::SnapTrailing(target) => {
            *target == id && capabilities.maximize && !current.state.fullscreen
        }
        MenuAction::MoveToWorkspace(target, _) => *target == id && capabilities.move_workspace,
        MenuAction::MoveToDisplay(target, _) => *target == id && capabilities.move_display,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApplicationMenuTarget {
    pub application_id: Option<ApplicationId>,
    pub application_name: String,
    pub windows: Vec<WindowId>,
    pub all_closeable: bool,
}

impl ApplicationMenuTarget {
    pub fn capture(group: &WindowGroup) -> Self {
        let mut seen = std::collections::HashSet::new();
        let windows = group
            .windows
            .iter()
            .filter_map(|window| seen.insert(window.id).then_some(window.id))
            .collect::<Vec<_>>();
        let mut windows = windows;
        windows.sort_unstable_by_key(|window| window.0);
        let all_closeable = !windows.is_empty()
            && group
                .windows
                .iter()
                .filter(|window| seen.contains(&window.id))
                .all(|window| window.state.capabilities.close);
        Self {
            application_id: group.application_id.clone(),
            application_name: group.application_name.clone(),
            windows,
            all_closeable,
        }
    }

    pub fn contains_current_window(&self, window: &OpenWindow) -> bool {
        self.windows.contains(&window.id) && self.application_id == window.application_id
    }

    pub fn survives(&self, windows: &[OpenWindow], canonical_item_available: bool) -> bool {
        canonical_item_available
            || windows
                .iter()
                .any(|window| self.contains_current_window(window))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ApplicationMenuAction {
    Dismiss,
    TogglePin(ApplicationId),
    CloseAll,
}

pub(crate) fn application_menu_entries(
    target: &ApplicationMenuTarget,
    pinned: bool,
) -> Vec<(String, ApplicationMenuAction)> {
    let mut entries = Vec::new();
    if let Some(application_id) = &target.application_id {
        entries.push((
            (if pinned {
                "Unpin from Nickel Bar"
            } else {
                "Pin to Nickel Bar"
            })
            .into(),
            ApplicationMenuAction::TogglePin(application_id.clone()),
        ));
    }
    if target.all_closeable {
        entries.push(("Close all windows".into(), ApplicationMenuAction::CloseAll));
    }
    entries
}

pub(crate) fn validated_application_close_targets(
    target: &ApplicationMenuTarget,
    current_windows: &[OpenWindow],
) -> Vec<WindowId> {
    target
        .windows
        .iter()
        .copied()
        .filter(|id| {
            current_windows.iter().any(|window| {
                window.id == *id
                    && window.state.capabilities.close
                    && target.contains_current_window(window)
            })
        })
        .collect()
}

pub(crate) fn window_menu_entries(
    window: &OpenWindow,
    workspaces: &[WorkspaceSummary],
    outputs: &[String],
) -> Vec<(String, MenuAction)> {
    let id = window.id;
    let capabilities = window.state.capabilities;
    let mut entries = Vec::new();
    if capabilities.activate {
        entries.push((
            (if window.state.minimized {
                "Restore"
            } else {
                "Activate"
            })
            .into(),
            MenuAction::Activate(id),
        ));
    }
    if capabilities.minimize {
        entries.push((
            (if window.state.minimized {
                "Unminimize"
            } else {
                "Minimize"
            })
            .into(),
            if window.state.minimized {
                MenuAction::Activate(id)
            } else {
                MenuAction::Minimize(id)
            },
        ));
    }
    if capabilities.maximize {
        entries.push((
            (if window.state.maximized {
                "Restore from Maximized"
            } else {
                "Maximize"
            })
            .into(),
            MenuAction::MaximizeRestore(id),
        ));
    }
    if capabilities.maximize && !window.state.fullscreen {
        entries.push(("Snap left".into(), MenuAction::SnapLeading(id)));
        entries.push(("Snap right".into(), MenuAction::SnapTrailing(id)));
    }
    if capabilities.fullscreen {
        entries.push((
            (if window.state.fullscreen {
                "Leave Fullscreen"
            } else {
                "Fullscreen"
            })
            .into(),
            MenuAction::FullscreenRestore(id),
        ));
    }
    if capabilities.move_workspace && !workspaces.is_empty() {
        entries.push(("Move to Workspace ›".into(), MenuAction::ShowWorkspaces));
    }
    if capabilities.move_display && outputs.len() > 1 {
        entries.push(("Move to Display ›".into(), MenuAction::ShowDisplays));
    }
    if capabilities.close {
        entries.push(("Close Window".to_owned(), MenuAction::Close(id)));
    }
    entries
}

pub(crate) fn workspace_menu_entries(
    window: &OpenWindow,
    workspaces: &[WorkspaceSummary],
) -> Vec<(String, MenuAction)> {
    let mut entries = vec![("‹ Window Actions".into(), MenuAction::Back)];
    entries.extend(workspace_move_destinations(workspaces).into_iter().map(
        |(label, workspace)| {
            let checked = window.state.workspace == Some(workspace);
            (
                format!("{}Workspace {label}", if checked { "✓ " } else { "" }),
                MenuAction::MoveToWorkspace(window.id, workspace),
            )
        },
    ));
    entries
}

pub(crate) fn display_menu_entries(
    window: &OpenWindow,
    outputs: &[String],
) -> Vec<(String, MenuAction)> {
    let mut entries = vec![("‹ Window Actions".into(), MenuAction::Back)];
    entries.extend(outputs.iter().map(|output| {
        let checked = window.state.output.as_ref() == Some(output);
        (
            format!("{}{output}", if checked { "✓ " } else { "" }),
            MenuAction::MoveToDisplay(window.id, output.clone()),
        )
    }));
    entries
}

pub(crate) fn window_menu_max_rows(
    window: &OpenWindow,
    workspaces: &[WorkspaceSummary],
    outputs: &[String],
) -> usize {
    window_menu_entries(window, workspaces, outputs)
        .len()
        .max(workspaces.len().saturating_add(1))
        .max(outputs.len().saturating_add(1))
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TaskbarPreviewAnchor {
    output_origin_x: i32,
    control_bounds: Rect,
}

impl TaskbarPreviewAnchor {
    pub fn new(output_origin_x: i32, control_bounds: Rect) -> Self {
        Self {
            output_origin_x,
            control_bounds,
        }
    }

    pub fn preview_origin_x(self, preview_width: u32) -> i32 {
        let control_center = self.control_bounds.origin.x + self.control_bounds.size.width / 2.0;
        (self.output_origin_x + (control_center - preview_width as f32 / 2.0).round() as i32)
            .max(self.output_origin_x)
    }
}

pub fn preview_dimensions(window_count: usize) -> (u32, u32) {
    let count = window_count.max(1) as f32;
    (
        (PADDING * 2.0 + count * CARD_WIDTH + (count - 1.0) * GAP).round() as u32,
        PREVIEW_HEIGHT.round() as u32,
    )
}

pub fn task_switcher_dimensions(window_count: usize) -> (u32, u32) {
    let count = window_count.clamp(1, 5) as f32;
    (
        (PADDING * 2.0 + count * TASK_SWITCHER_CARD_WIDTH + (count - 1.0) * GAP).round() as u32,
        PREVIEW_HEIGHT.round() as u32,
    )
}

pub fn menu_height(workspaces: &[WorkspaceSummary]) -> f32 {
    let destination_count = workspace_move_destinations(workspaces).len();
    let row_count = 4 + destination_count;
    MENU_PADDING * 2.0
        + row_count as f32 * MENU_ROW_HEIGHT
        + row_count.saturating_sub(1) as f32 * MENU_ROW_GAP
}

pub fn menu_height_for_rows(row_count: usize) -> f32 {
    MENU_PADDING * 2.0
        + row_count as f32 * MENU_ROW_HEIGHT
        + row_count.saturating_sub(1) as f32 * MENU_ROW_GAP
}

fn workspace_move_destinations(workspaces: &[WorkspaceSummary]) -> Vec<(String, u64)> {
    workspaces
        .iter()
        .enumerate()
        .map(|(index, workspace)| ((index + 1).to_string(), workspace.id))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::OpenWindow;

    #[test]
    fn five_task_switcher_cards_fit_a_1280_pixel_output() {
        let (width, _) = task_switcher_dimensions(5);
        assert!(width <= 1_200);
    }

    #[test]
    fn taskbar_preview_anchor_keeps_a_wide_preview_on_the_invoking_output() {
        let anchor = TaskbarPreviewAnchor::new(1_920, Rect::new(48.0, 0.0, 48.0, 56.0));

        assert_eq!(anchor.preview_origin_x(1_160), 1_920);
        assert!(anchor.preview_origin_x(80) >= 1_920);
    }

    #[test]
    fn menu_model_targets_the_selected_window_and_exposes_submenus() {
        let workspaces = [
            WorkspaceSummary {
                id: 1,
                active: false,
            },
            WorkspaceSummary {
                id: 7,
                active: true,
            },
            WorkspaceSummary {
                id: 8,
                active: false,
            },
        ];
        let window = OpenWindow {
            id: WindowId(9),
            application_id: Some(ApplicationId::new("editor")),
            active: false,
            title: "Document".into(),
            state: crate::model::WindowState {
                workspace: Some(7),
                output: Some("left".into()),
                capabilities: crate::model::WindowCapabilities {
                    fullscreen: true,
                    move_workspace: true,
                    move_display: true,
                    ..Default::default()
                },
                ..Default::default()
            },
        };
        let outputs = vec!["left".into(), "right".into()];
        let root = window_menu_entries(&window, &workspaces, &outputs);
        for action in [
            MenuAction::Activate(WindowId(9)),
            MenuAction::Minimize(WindowId(9)),
            MenuAction::MaximizeRestore(WindowId(9)),
            MenuAction::FullscreenRestore(WindowId(9)),
            MenuAction::Close(WindowId(9)),
        ] {
            assert!(root.iter().any(|(_, candidate)| *candidate == action));
        }
        assert!(
            root.iter()
                .any(|(_, action)| *action == MenuAction::ShowWorkspaces)
        );
        assert!(
            root.iter()
                .any(|(_, action)| *action == MenuAction::ShowDisplays)
        );
        let workspace_entries = workspace_menu_entries(&window, &workspaces);
        assert!(
            workspace_entries
                .iter()
                .any(|(label, _)| label == "✓ Workspace 2")
        );
        let display_entries = display_menu_entries(&window, &outputs);
        assert!(display_entries.iter().any(|(label, _)| label == "✓ left"));
    }

    #[test]
    fn menu_model_uses_current_window_state_and_topology() {
        let mut captured = OpenWindow {
            id: WindowId(9),
            application_id: Some(ApplicationId::new("editor")),
            active: false,
            title: "Old title".into(),
            state: crate::model::WindowState::default(),
        };
        captured.active = true;
        captured.title = "New title".into();
        captured.state.maximized = true;
        captured.state.workspace = Some(12);
        captured.state.output = Some("right".into());
        captured.state.capabilities.move_workspace = true;
        captured.state.capabilities.move_display = true;
        let workspaces = vec![WorkspaceSummary {
            id: 12,
            active: true,
        }];
        let outputs = vec!["left".into(), "right".into()];

        assert!(
            window_menu_entries(&captured, &workspaces, &outputs,)
                .iter()
                .any(|(label, action)| label == "Restore from Maximized"
                    && *action == MenuAction::MaximizeRestore(WindowId(9)))
        );
        assert!(
            workspace_menu_entries(&captured, &workspaces)
                .iter()
                .any(|(label, _)| label == "✓ Workspace 1")
        );
        assert!(
            display_menu_entries(&captured, &outputs)
                .iter()
                .any(|(label, _)| label == "✓ right")
        );
    }

    #[test]
    fn menu_model_uses_inverse_labels_and_omits_unsupported_commands() {
        let window = OpenWindow {
            id: WindowId(44),
            application_id: None,
            active: false,
            title: "Player".into(),
            state: crate::model::WindowState {
                minimized: true,
                maximized: true,
                fullscreen: true,
                workspace: Some(2),
                output: Some("right".into()),
                capabilities: crate::model::WindowCapabilities {
                    activate: true,
                    close: true,
                    minimize: true,
                    maximize: true,
                    fullscreen: true,
                    move_workspace: false,
                    move_display: false,
                },
            },
        };
        let entries = window_menu_entries(
            &window,
            &[WorkspaceSummary {
                id: 2,
                active: true,
            }],
            &["left".into(), "right".into()],
        );
        let labels = entries
            .iter()
            .map(|(label, _)| label.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            labels,
            [
                "Restore",
                "Unminimize",
                "Restore from Maximized",
                "Leave Fullscreen",
                "Close Window"
            ]
        );
        assert!(!entries.iter().any(|(_, action)| matches!(
            action,
            MenuAction::MoveToWorkspace(..) | MenuAction::MoveToDisplay(..)
        )));
    }

    #[test]
    fn unavailable_pinned_application_uses_an_explicit_application_target() {
        let application = ApplicationId::new("org.example.missing");
        let target = ApplicationMenuTarget {
            application_id: Some(application.clone()),
            application_name: "Missing".into(),
            windows: Vec::new(),
            all_closeable: false,
        };
        let entries = application_menu_entries(&target, true);

        assert!(entries.iter().any(|(label, action)| {
            label == "Unpin from Nickel Bar"
                && *action == ApplicationMenuAction::TogglePin(application.clone())
        }));
        assert!(
            entries
                .iter()
                .all(|(_, action)| !matches!(action, ApplicationMenuAction::CloseAll))
        );
    }

    #[test]
    fn application_menu_captures_unique_membership_and_never_exposes_window_actions() {
        let application = ApplicationId::new("org.example.Editor");
        let make_window = |id, close| OpenWindow {
            id: WindowId(id),
            application_id: Some(application.clone()),
            active: false,
            title: format!("Window {id}"),
            state: crate::model::WindowState {
                capabilities: crate::model::WindowCapabilities {
                    close,
                    ..Default::default()
                },
                ..Default::default()
            },
        };
        let group = WindowGroup {
            application_id: Some(application.clone()),
            application_name: "Editor".into(),
            windows: vec![
                make_window(2, true),
                make_window(1, true),
                make_window(2, true),
            ],
        };
        let target = ApplicationMenuTarget::capture(&group);
        assert_eq!(target.windows, vec![WindowId(1), WindowId(2)]);
        let mut reordered = group.clone();
        reordered.windows.reverse();
        assert_eq!(
            ApplicationMenuTarget::capture(&reordered).windows,
            target.windows,
            "captured close order must not depend on transient group ordering"
        );
        let entries = application_menu_entries(&target, false);
        assert_eq!(
            entries,
            vec![
                (
                    "Pin to Nickel Bar".into(),
                    ApplicationMenuAction::TogglePin(application)
                ),
                ("Close all windows".into(), ApplicationMenuAction::CloseAll),
            ]
        );
        assert!(entries.iter().all(|(label, _)| {
            !label.contains("Maximize")
                && !label.contains("Minimize")
                && !label.contains("Workspace")
                && label != "Close Window"
        }));
    }

    #[test]
    fn application_menu_omits_bulk_close_for_mixed_capabilities() {
        let group = WindowGroup {
            application_id: None,
            application_name: "Unresolved".into(),
            windows: vec![
                OpenWindow {
                    id: WindowId(1),
                    application_id: None,
                    active: false,
                    title: "Closeable".into(),
                    state: crate::model::WindowState::default(),
                },
                OpenWindow {
                    id: WindowId(2),
                    application_id: None,
                    active: false,
                    title: "Protected".into(),
                    state: crate::model::WindowState {
                        capabilities: crate::model::WindowCapabilities {
                            close: false,
                            ..Default::default()
                        },
                        ..Default::default()
                    },
                },
            ],
        };
        let target = ApplicationMenuTarget::capture(&group);
        assert!(!target.all_closeable);
        assert!(application_menu_entries(&target, false).is_empty());
    }

    #[test]
    fn application_target_rejects_reused_ids_from_another_application() {
        let application = ApplicationId::new("org.nickel.One");
        let target = ApplicationMenuTarget::capture(&WindowGroup {
            application_id: Some(application.clone()),
            application_name: "One".into(),
            windows: vec![OpenWindow {
                id: WindowId(7),
                application_id: Some(application),
                active: true,
                title: "Original".into(),
                state: crate::model::WindowState::default(),
            }],
        });
        let reused = OpenWindow {
            id: WindowId(7),
            application_id: Some(ApplicationId::new("org.nickel.Two")),
            active: true,
            title: "Reused".into(),
            state: crate::model::WindowState::default(),
        };

        assert!(!target.contains_current_window(&reused));
        assert!(!target.survives(&[reused], false));
        assert!(
            target.survives(&[], true),
            "a closed canonical pin still exists"
        );
    }

    #[test]
    fn window_menu_actions_reject_reuse_and_revoked_capabilities() {
        let captured = OpenWindow {
            id: WindowId(9),
            application_id: Some(ApplicationId::new("org.nickel.Editor")),
            active: true,
            title: "Captured".into(),
            state: crate::model::WindowState::default(),
        };
        let mut current = captured.clone();
        assert!(window_menu_action_is_current(
            &captured,
            &current,
            &MenuAction::Close(WindowId(9))
        ));

        current.state.capabilities.close = false;
        assert!(!window_menu_action_is_current(
            &captured,
            &current,
            &MenuAction::Close(WindowId(9))
        ));
        current.state.capabilities.close = true;
        current.application_id = Some(ApplicationId::new("org.nickel.Other"));
        assert!(!same_window_identity(&captured, &current));
        assert!(!window_menu_action_is_current(
            &captured,
            &current,
            &MenuAction::Activate(WindowId(9))
        ));
        assert!(!window_menu_action_is_current(
            &captured,
            &captured,
            &MenuAction::Close(WindowId(10))
        ));
    }

    #[test]
    fn bulk_close_validation_never_retargets_or_expands_the_capture() {
        let application = ApplicationId::new("org.nickel.One");
        let window = |id, application_id: Option<ApplicationId>, close| OpenWindow {
            id: WindowId(id),
            application_id,
            active: false,
            title: format!("Window {id}"),
            state: crate::model::WindowState {
                capabilities: crate::model::WindowCapabilities {
                    close,
                    ..Default::default()
                },
                ..Default::default()
            },
        };
        let target = ApplicationMenuTarget::capture(&WindowGroup {
            application_id: Some(application.clone()),
            application_name: "One".into(),
            windows: vec![
                window(3, Some(application.clone()), true),
                window(1, Some(application.clone()), true),
            ],
        });
        let current = vec![
            window(1, Some(application.clone()), true),
            window(2, Some(application.clone()), true), // joined after invocation
            window(3, Some(ApplicationId::new("org.nickel.Other")), true), // reused
            window(1, Some(application), true),         // duplicate feed observation
        ];

        assert_eq!(
            validated_application_close_targets(&target, &current),
            [WindowId(1)]
        );
        assert!(validated_application_close_targets(&target, &[]).is_empty());
    }
}
