//! Workspace behavior through the production model and scenario effect recorder.

use nickel_core::{
    hotkeys::{HotkeyAction, KeyCode, KeyEdge},
    scenario::{Key, RecordedEffect, WorkspaceEffect, scenario},
    task_switcher::TaskSwitchEffect,
};

#[test]
fn workspace_keyboard_chords_reach_the_workspace_reducer() {
    scenario("workspace keyboard chords use production reducers")
        .output("DP-1", 1920, 1080, 0)
        .window("editor")
        .app("editor")
        .active()
        .create_workspace("second")
        .key_edge(KeyCode::ControlLeft, KeyEdge::Pressed)
        .key_edge(KeyCode::SuperLeft, KeyEdge::Pressed)
        .key_edge(KeyCode::ArrowRight, KeyEdge::Pressed)
        .key_edge(KeyCode::ArrowRight, KeyEdge::Released)
        .key_edge(KeyCode::SuperLeft, KeyEdge::Released)
        .key_edge(KeyCode::ControlLeft, KeyEdge::Released)
        .expect_actions(&[HotkeyAction::SwitchWorkspaceNext])
        .expect_workspace("second")
        .expect_authority_path(
            "window.workspace.visible",
            &["ShortcutEngine", "Workspaces directional reducer"],
        );
}

#[test]
fn workspace_switch_hides_shows_and_restores_focus_in_order() {
    scenario("workspace switch owns visibility and focus ordering")
        .output("DP-1", 1920, 1080, 0)
        .window("editor")
        .app("code")
        .active()
        .window("terminal")
        .app("terminal")
        .create_workspace("research")
        .switch_workspace("research", "DP-1")
        .window("browser")
        .app("browser")
        .active()
        .switch_workspace("main", "DP-1")
        .expect_workspace("main")
        .expect_last_event_authority()
        .expect_window_visible("browser", false)
        .expect_window_visible("editor", true)
        .expect_active("editor")
        .expect_ordered_effects(&[
            RecordedEffect::Workspace(WorkspaceEffect::HideWindow("editor".into())),
            RecordedEffect::Workspace(WorkspaceEffect::HideWindow("terminal".into())),
            RecordedEffect::Workspace(WorkspaceEffect::HideWindow("browser".into())),
            RecordedEffect::Workspace(WorkspaceEffect::ShowWindow("editor".into())),
            RecordedEffect::Workspace(WorkspaceEffect::ShowWindow("terminal".into())),
            RecordedEffect::Workspace(WorkspaceEffect::ActivateWindow("editor".into())),
        ]);
}

#[test]
fn alt_tab_candidates_are_filtered_by_the_active_workspace() {
    scenario("task switching cannot cross workspaces")
        .output("DP-1", 1920, 1080, 0)
        .window("editor")
        .app("code")
        .active()
        .create_workspace("research")
        .switch_workspace("research", "DP-1")
        .window("browser-a")
        .app("browser")
        .active()
        .window("browser-b")
        .app("browser")
        .press(Key::AltTab)
        .expect_effects(&[
            TaskSwitchEffect::ShowFlip { session: 1 },
            TaskSwitchEffect::RequestPreviews(vec!["browser-a".into(), "browser-b".into()]),
            TaskSwitchEffect::SelectPreview("browser-b".into()),
            TaskSwitchEffect::HideFlip { session: 1 },
            TaskSwitchEffect::ActivateWindow("browser-b".into()),
        ]);
}

#[test]
fn moving_the_focused_window_restores_focus_without_exposing_other_workspace() {
    scenario("moving focused window is one authoritative transition")
        .output("DP-1", 1920, 1080, 0)
        .window("editor")
        .app("code")
        .window("terminal")
        .app("terminal")
        .active()
        .create_workspace("research")
        .move_window_to_workspace("terminal", "research")
        .expect_window_visible("terminal", false)
        .expect_active("editor")
        .expect_ordered_effects(&[
            RecordedEffect::Workspace(WorkspaceEffect::HideWindow("terminal".into())),
            RecordedEffect::Workspace(WorkspaceEffect::ActivateWindow("editor".into())),
        ]);
}

#[test]
fn grouped_application_members_do_not_leak_across_workspace_window_feeds() {
    scenario("same application groups remain workspace local")
        .output("DP-1", 1920, 1080, 0)
        .window("browser-main")
        .app("browser")
        .active()
        .create_workspace("research")
        .switch_workspace("research", "DP-1")
        .window("browser-research")
        .app("browser")
        .active()
        .expect_visible_windows(&["browser-research"])
        .switch_workspace("main", "DP-1")
        .expect_visible_windows(&["browser-main"]);
}
