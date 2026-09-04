//! Semantic input and output contracts driven through production reducers.

use nickel_core::hotkeys::{HotkeyAction, KeyCode, KeyEdge};
use nickel_core::scenario::ScreenshotEffect;
use nickel_core::scenario::scenario;
use nickel_core::task_switcher::TaskSwitchEffect;

#[test]
fn alt_release_commits_pending_switch_before_tab_release() {
    scenario("Alt release commits the pending switch before Tab release")
        .window("current")
        .app("editor")
        .active()
        .window("next")
        .app("browser")
        .key_edge(KeyCode::AltLeft, KeyEdge::Pressed)
        .key_edge(KeyCode::Tab, KeyEdge::Pressed)
        .key_edge(KeyCode::AltLeft, KeyEdge::Released)
        .key_edge(KeyCode::Tab, KeyEdge::Released)
        .expect_actions(&[HotkeyAction::SwitchNext, HotkeyAction::CommitSwitch])
        .expect_effects(&[
            TaskSwitchEffect::ShowFlip { session: 1 },
            TaskSwitchEffect::RequestPreviews(vec!["current".into(), "next".into()]),
            TaskSwitchEffect::SelectPreview("next".into()),
            TaskSwitchEffect::HideFlip { session: 1 },
            TaskSwitchEffect::ActivateWindow("next".into()),
        ])
        .expect_active("next")
        .expect_flip_hidden();
}

#[test]
fn tab_repeat_while_held_emits_one_switch_action() {
    scenario("repeated tab press while held is suppressed")
        .window("current")
        .app("editor")
        .active()
        .window("next")
        .app("browser")
        .window("last")
        .app("terminal")
        .key_edge(KeyCode::AltLeft, KeyEdge::Pressed)
        .key_edge(KeyCode::Tab, KeyEdge::Pressed)
        .key_edge(KeyCode::Tab, KeyEdge::Pressed)
        .key_edge(KeyCode::Tab, KeyEdge::Released)
        .key_edge(KeyCode::AltLeft, KeyEdge::Released)
        .expect_actions(&[HotkeyAction::SwitchNext, HotkeyAction::CommitSwitch])
        .expect_effects(&[
            TaskSwitchEffect::ShowFlip { session: 1 },
            TaskSwitchEffect::RequestPreviews(vec!["current".into(), "next".into(), "last".into()]),
            TaskSwitchEffect::SelectPreview("next".into()),
            TaskSwitchEffect::HideFlip { session: 1 },
            TaskSwitchEffect::ActivateWindow("next".into()),
        ])
        .expect_active("next")
        .expect_flip_hidden();
}

#[test]
fn reverse_tab_repeat_is_suppressed_and_shift_release_changes_later_direction() {
    scenario("reverse repeat is suppressed and direction follows current modifiers")
        .window("current")
        .app("editor")
        .active()
        .window("middle")
        .app("browser")
        .window("oldest")
        .app("terminal")
        .key_edge(KeyCode::ShiftRight, KeyEdge::Pressed)
        .key_edge(KeyCode::AltRight, KeyEdge::Pressed)
        .key_edge(KeyCode::Tab, KeyEdge::Pressed)
        .key_edge(KeyCode::Tab, KeyEdge::Pressed)
        .key_edge(KeyCode::Tab, KeyEdge::Released)
        .key_edge(KeyCode::ShiftRight, KeyEdge::Released)
        .key_edge(KeyCode::Tab, KeyEdge::Pressed)
        .key_edge(KeyCode::Tab, KeyEdge::Released)
        .key_edge(KeyCode::AltRight, KeyEdge::Released)
        .expect_actions(&[
            HotkeyAction::SwitchPrevious,
            HotkeyAction::SwitchNext,
            HotkeyAction::CommitSwitch,
        ])
        .expect_effects(&[
            TaskSwitchEffect::ShowFlip { session: 1 },
            TaskSwitchEffect::RequestPreviews(vec![
                "current".into(),
                "middle".into(),
                "oldest".into(),
            ]),
            TaskSwitchEffect::SelectPreview("oldest".into()),
            TaskSwitchEffect::SelectPreview("current".into()),
            TaskSwitchEffect::HideFlip { session: 1 },
            TaskSwitchEffect::ActivateWindow("current".into()),
        ])
        .expect_active("current")
        .expect_flip_hidden();
}

#[test]
fn print_screen_repeats_are_one_action_per_physical_press() {
    scenario("screenshot shortcut repeats only after release")
        .key_edge(KeyCode::PrintScreen, KeyEdge::Pressed)
        .key_edge(KeyCode::PrintScreen, KeyEdge::Pressed)
        .key_edge(KeyCode::PrintScreen, KeyEdge::Released)
        .key_edge(KeyCode::PrintScreen, KeyEdge::Pressed)
        .key_edge(KeyCode::PrintScreen, KeyEdge::Released)
        .expect_actions(&[
            HotkeyAction::ShowScreenshotTool,
            HotkeyAction::ShowScreenshotTool,
        ])
        .expect_screenshot_effects(&[
            ScreenshotEffect::RequestInteractiveRegionCapture,
            ScreenshotEffect::RequestInteractiveRegionCapture,
        ])
        .expect_screenshot_overlay_requested()
        .expect_effects(&[]);
}

#[test]
fn alt_print_screen_captures_the_active_window_without_switch_effects() {
    scenario("alt screenshot captures the active window")
        .window("editor")
        .app("editor")
        .active()
        .key_edge(KeyCode::AltLeft, KeyEdge::Pressed)
        .key_edge(KeyCode::PrintScreen, KeyEdge::Pressed)
        .key_edge(KeyCode::PrintScreen, KeyEdge::Released)
        .key_edge(KeyCode::AltLeft, KeyEdge::Released)
        .expect_actions(&[HotkeyAction::CaptureActiveWindow])
        .expect_screenshot_effects(&[ScreenshotEffect::CaptureActiveWindowToClipboard])
        .expect_effects(&[])
        .expect_active("editor");
}

#[test]
fn output_reconnect_updates_dimensions_and_reflows_following_outputs() {
    scenario("output geometry update reflows the topology")
        .output("DP-1", 1920, 1080, 0)
        .output("HDMI-A-1", 2560, 1440, 1)
        .expect_output_position("DP-1", 0, 0)
        .expect_output_position("HDMI-A-1", 1920, 0)
        .output("DP-1", 1280, 720, 1)
        .expect_output_position("DP-1", 0, 0)
        .expect_output_position("HDMI-A-1", 1280, 0)
        .expect_within_budget();
}

#[test]
fn equal_priority_outputs_use_name_order_after_reconnect() {
    scenario("equal priority output order is stable")
        .output("DP-2", 1920, 1080, 4)
        .output("DP-1", 1920, 1080, 4)
        .expect_output_position("DP-1", 0, 0)
        .expect_output_position("DP-2", 1920, 0)
        .output("DP-2", 1280, 720, 4)
        .expect_output_position("DP-1", 0, 0)
        .expect_output_position("DP-2", 1920, 0)
        .expect_within_budget();
}

#[test]
fn disconnecting_an_unknown_output_preserves_connected_positions() {
    scenario("unknown output removal is a topology no-op")
        .output("DP-1", 1920, 1080, 0)
        .output("HDMI-A-1", 2560, 1440, 1)
        .disconnect_output("missing")
        .expect_last_event_authority()
        .expect_output_position("DP-1", 0, 0)
        .expect_output_position("HDMI-A-1", 1920, 0)
        .expect_within_budget();
}

#[test]
fn switch_key_without_alt_does_not_create_a_task_switch() {
    scenario("unmodified tab is ignored by switcher")
        .window("current")
        .app("editor")
        .active()
        .window("other")
        .app("browser")
        .key_edge(KeyCode::Tab, KeyEdge::Pressed)
        .key_edge(KeyCode::Tab, KeyEdge::Released)
        .expect_actions(&[])
        .expect_effects(&[])
        .expect_active("current")
        .expect_flip_hidden();
}
