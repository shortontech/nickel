//! Cross-policy semantic shell scenarios found during the test-authority audit.

use nickel_core::focus::{FocusRequest, FocusTransaction};
use nickel_core::hotkeys::{Hotkey, HotkeyAction, KeyEdge};
use nickel_core::scenario::{
    ClickTarget, Key, LauncherEffect, RecordedEffect, Surface, SurfaceIdentity, scenario,
};
use nickel_core::task_switcher::TaskSwitchEffect;

#[test]
fn repeated_alt_tab_edges_cycle_once_per_press() {
    scenario("repeated held-alt tab presses")
        .window("current")
        .app("editor")
        .active()
        .window("browser")
        .app("browser")
        .window("terminal")
        .app("terminal")
        .key_edge(Hotkey::Alt, KeyEdge::Pressed)
        .key_edge(Hotkey::Tab, KeyEdge::Pressed)
        .key_edge(Hotkey::Tab, KeyEdge::Released)
        .key_edge(Hotkey::Tab, KeyEdge::Pressed)
        .key_edge(Hotkey::Tab, KeyEdge::Released)
        .key_edge(Hotkey::Alt, KeyEdge::Released)
        .expect_active("terminal")
        .expect_actions(&[
            HotkeyAction::SwitchNext,
            HotkeyAction::SwitchNext,
            HotkeyAction::CommitSwitch,
        ])
        .expect_effects(&[
            TaskSwitchEffect::ShowFlip { session: 1 },
            TaskSwitchEffect::RequestPreviews(vec![
                "current".into(),
                "browser".into(),
                "terminal".into(),
            ]),
            TaskSwitchEffect::SelectPreview("browser".into()),
            TaskSwitchEffect::SelectPreview("terminal".into()),
            TaskSwitchEffect::HideFlip { session: 1 },
            TaskSwitchEffect::ActivateWindow("terminal".into()),
        ]);
}

#[test]
fn alt_backtick_scopes_candidates_to_the_active_application() {
    scenario("group switcher only shows the active application")
        .window("chrome-current")
        .app("chrome")
        .active()
        .window("editor")
        .app("editor")
        .window("chrome-old")
        .app("chrome")
        .press(Key::AltBacktick)
        .expect_active("chrome-old")
        .expect_last_event_authority()
        .expect_actions(&[HotkeyAction::SwitchGroupNext, HotkeyAction::CommitSwitch])
        .expect_effects(&[
            TaskSwitchEffect::ShowFlip { session: 1 },
            TaskSwitchEffect::RequestPreviews(vec!["chrome-current".into(), "chrome-old".into()]),
            TaskSwitchEffect::SelectPreview("chrome-old".into()),
            TaskSwitchEffect::HideFlip { session: 1 },
            TaskSwitchEffect::ActivateWindow("chrome-old".into()),
        ])
        .expect_flip_hidden();
}

#[test]
fn reverse_alt_backtick_selects_the_oldest_window_in_the_active_application() {
    scenario("reverse group switch stays inside active application")
        .window("chrome-current")
        .app("chrome")
        .active()
        .window("editor")
        .app("editor")
        .window("chrome-oldest")
        .app("chrome")
        .press(Key::AltShiftBacktick)
        .expect_active("chrome-oldest")
        .expect_actions(&[
            HotkeyAction::SwitchGroupPrevious,
            HotkeyAction::CommitSwitch,
        ])
        .expect_effects(&[
            TaskSwitchEffect::ShowFlip { session: 1 },
            TaskSwitchEffect::RequestPreviews(vec![
                "chrome-current".into(),
                "chrome-oldest".into(),
            ]),
            TaskSwitchEffect::SelectPreview("chrome-oldest".into()),
            TaskSwitchEffect::HideFlip { session: 1 },
            TaskSwitchEffect::ActivateWindow("chrome-oldest".into()),
        ])
        .expect_flip_hidden();
}

#[test]
fn reverse_alt_tab_commits_the_oldest_candidate_with_exact_effects() {
    scenario("reverse switch commits oldest candidate")
        .window("current")
        .app("editor")
        .active()
        .window("middle")
        .app("browser")
        .window("oldest")
        .app("terminal")
        .press(Key::AltShiftTab)
        .expect_active("oldest")
        .expect_actions(&[HotkeyAction::SwitchPrevious, HotkeyAction::CommitSwitch])
        .expect_effects(&[
            TaskSwitchEffect::ShowFlip { session: 1 },
            TaskSwitchEffect::RequestPreviews(vec![
                "current".into(),
                "middle".into(),
                "oldest".into(),
            ]),
            TaskSwitchEffect::SelectPreview("oldest".into()),
            TaskSwitchEffect::HideFlip { session: 1 },
            TaskSwitchEffect::ActivateWindow("oldest".into()),
        ])
        .expect_flip_hidden();
}

#[test]
fn alt_without_a_switch_is_a_noop() {
    scenario("bare alt does not open or commit a switcher")
        .window("current")
        .app("editor")
        .active()
        .window("other")
        .app("browser")
        .key_edge(Hotkey::Alt, KeyEdge::Pressed)
        .key_edge(Hotkey::Alt, KeyEdge::Released)
        .expect_active("current")
        .expect_actions(&[])
        .expect_effects(&[])
        .expect_flip_hidden();
}

#[test]
fn switching_with_no_windows_records_input_but_emits_no_platform_effects() {
    scenario("empty window inventory is a switcher no-op")
        .key_edge(Hotkey::Alt, KeyEdge::Pressed)
        .key_edge(Hotkey::Tab, KeyEdge::Pressed)
        .key_edge(Hotkey::Tab, KeyEdge::Released)
        .key_edge(Hotkey::Alt, KeyEdge::Released)
        .expect_actions(&[HotkeyAction::SwitchNext, HotkeyAction::CommitSwitch])
        .expect_effects(&[])
        .expect_flip_hidden();
}

#[test]
fn switching_one_window_selects_and_activates_it_once() {
    scenario("single window switch has one complete transaction")
        .window("only")
        .app("editor")
        .active()
        .press(Key::AltTab)
        .expect_active("only")
        .expect_actions(&[HotkeyAction::SwitchNext, HotkeyAction::CommitSwitch])
        .expect_effects(&[
            TaskSwitchEffect::ShowFlip { session: 1 },
            TaskSwitchEffect::RequestPreviews(vec!["only".into()]),
            TaskSwitchEffect::SelectPreview("only".into()),
            TaskSwitchEffect::HideFlip { session: 1 },
            TaskSwitchEffect::ActivateWindow("only".into()),
        ])
        .expect_flip_hidden();
}

#[test]
fn output_disconnect_and_reconnect_reflows_deterministically() {
    scenario("output disconnect and reconnect")
        .output("DP-1", 1920, 1080, 1)
        .output("HDMI-A-1", 2560, 1440, 0)
        .expect_output_position("HDMI-A-1", 0, 0)
        .expect_output_position("DP-1", 2560, 0)
        .disconnect_output("HDMI-A-1")
        .expect_output_position("DP-1", 0, 0)
        .output("HDMI-A-1", 2560, 1440, 0)
        .expect_output_position("HDMI-A-1", 0, 0)
        .expect_output_position("DP-1", 2560, 0)
        .expect_within_budget();
}

#[test]
fn alt_shift_print_screen_routes_to_file_capture() {
    scenario("alt-shift-print-screen file capture")
        .key_edge(Hotkey::Alt, KeyEdge::Pressed)
        .key_edge(Hotkey::Shift, KeyEdge::Pressed)
        .key_edge(Hotkey::PrintScreen, KeyEdge::Pressed)
        .key_edge(Hotkey::PrintScreen, KeyEdge::Pressed)
        .key_edge(Hotkey::PrintScreen, KeyEdge::Released)
        .key_edge(Hotkey::Shift, KeyEdge::Released)
        .key_edge(Hotkey::PrintScreen, KeyEdge::Pressed)
        .key_edge(Hotkey::PrintScreen, KeyEdge::Released)
        .key_edge(Hotkey::Alt, KeyEdge::Released)
        .expect_actions(&[
            HotkeyAction::CaptureActiveWindowToFile,
            HotkeyAction::CaptureActiveWindow,
        ]);
}

#[test]
fn stale_focus_loss_cannot_dismiss_a_reopened_launcher() {
    scenario("stale focus loss during launcher reopen")
        .click(ClickTarget::PanelLauncher)
        .acknowledge_current_focus()
        .capture_focus("old")
        .click(ClickTarget::PanelLauncher)
        .click(ClickTarget::PanelLauncher)
        .lose_captured_focus("old")
        .expect_visible(Surface::Launcher)
        .expect_launcher_effects(&[
            LauncherEffect::ShowSurface(SurfaceIdentity(1)),
            LauncherEffect::RequestFocus(FocusRequest {
                transaction: FocusTransaction(1),
                surface: SurfaceIdentity(1),
            }),
            LauncherEffect::HideSurface(SurfaceIdentity(1)),
            LauncherEffect::ShowSurface(SurfaceIdentity(1)),
            LauncherEffect::RequestFocus(FocusRequest {
                transaction: FocusTransaction(2),
                surface: SurfaceIdentity(1),
            }),
        ]);
}

#[test]
fn removing_selected_window_during_flip_does_not_activate_it() {
    scenario("removed selected window during flip")
        .window("current")
        .app("editor")
        .active()
        .window("closing")
        .app("browser")
        .key_edge(Hotkey::Alt, KeyEdge::Pressed)
        .key_edge(Hotkey::Tab, KeyEdge::Pressed)
        .remove_window("closing")
        .expect_last_event_authority()
        .key_edge(Hotkey::Tab, KeyEdge::Released)
        .key_edge(Hotkey::Alt, KeyEdge::Released)
        .expect_effects(&[
            TaskSwitchEffect::ShowFlip { session: 1 },
            TaskSwitchEffect::RequestPreviews(vec!["current".into(), "closing".into()]),
            TaskSwitchEffect::SelectPreview("closing".into()),
            TaskSwitchEffect::RequestPreviews(vec!["current".into()]),
            TaskSwitchEffect::SelectPreview("current".into()),
            TaskSwitchEffect::HideFlip { session: 1 },
            TaskSwitchEffect::ActivateWindow("current".into()),
        ])
        .expect_active("current")
        .expect_flip_hidden();
}

#[test]
fn removing_unselected_window_during_flip_updates_preview_candidates() {
    scenario("removed unselected window leaves only live previews")
        .window("current")
        .app("editor")
        .active()
        .window("selected")
        .app("browser")
        .window("closing")
        .app("terminal")
        .key_edge(Hotkey::Alt, KeyEdge::Pressed)
        .key_edge(Hotkey::Tab, KeyEdge::Pressed)
        .remove_window("closing")
        .key_edge(Hotkey::Tab, KeyEdge::Released)
        .key_edge(Hotkey::Alt, KeyEdge::Released)
        .expect_active("selected")
        .expect_effects(&[
            TaskSwitchEffect::ShowFlip { session: 1 },
            TaskSwitchEffect::RequestPreviews(vec![
                "current".into(),
                "selected".into(),
                "closing".into(),
            ]),
            TaskSwitchEffect::SelectPreview("selected".into()),
            TaskSwitchEffect::RequestPreviews(vec!["current".into(), "selected".into()]),
            TaskSwitchEffect::HideFlip { session: 1 },
            TaskSwitchEffect::ActivateWindow("selected".into()),
        ])
        .expect_flip_hidden();
}

#[test]
fn removing_current_window_during_flip_never_reactivates_it() {
    scenario("removed current window cannot remain a flip candidate")
        .window("current")
        .app("editor")
        .active()
        .window("selected")
        .app("browser")
        .window("other")
        .app("terminal")
        .key_edge(Hotkey::Alt, KeyEdge::Pressed)
        .key_edge(Hotkey::Tab, KeyEdge::Pressed)
        .remove_window("current")
        .key_edge(Hotkey::Tab, KeyEdge::Released)
        .key_edge(Hotkey::Alt, KeyEdge::Released)
        .expect_active("selected")
        .expect_effects(&[
            TaskSwitchEffect::ShowFlip { session: 1 },
            TaskSwitchEffect::RequestPreviews(vec![
                "current".into(),
                "selected".into(),
                "other".into(),
            ]),
            TaskSwitchEffect::SelectPreview("selected".into()),
            TaskSwitchEffect::RequestPreviews(vec!["selected".into(), "other".into()]),
            TaskSwitchEffect::HideFlip { session: 1 },
            TaskSwitchEffect::ActivateWindow("selected".into()),
        ])
        .expect_flip_hidden();
}

#[test]
fn removing_all_flip_candidates_closes_without_activation() {
    scenario("empty flip after final window removal")
        .window("current")
        .app("editor")
        .active()
        .window("closing")
        .app("browser")
        .key_edge(Hotkey::Alt, KeyEdge::Pressed)
        .key_edge(Hotkey::Tab, KeyEdge::Pressed)
        .remove_window("closing")
        .remove_window("current")
        .key_edge(Hotkey::Tab, KeyEdge::Released)
        .key_edge(Hotkey::Alt, KeyEdge::Released)
        .expect_effects(&[
            TaskSwitchEffect::ShowFlip { session: 1 },
            TaskSwitchEffect::RequestPreviews(vec!["current".into(), "closing".into()]),
            TaskSwitchEffect::SelectPreview("closing".into()),
            TaskSwitchEffect::RequestPreviews(vec!["current".into()]),
            TaskSwitchEffect::SelectPreview("current".into()),
            TaskSwitchEffect::HideFlip { session: 1 },
        ])
        .expect_flip_hidden();
}

#[test]
fn semantic_window_click_is_observed_through_production_effects() {
    scenario("window click authority")
        .window("editor")
        .app("editor")
        .bounds(40.0, 70.0, 500.0, 400.0)
        .window("terminal")
        .app("terminal")
        .bounds(600.0, 100.0, 500.0, 400.0)
        .active()
        .click(ClickTarget::PanelLauncher)
        .click_window("editor")
        .expect_active("editor")
        .expect_last_event_authority()
        .expect_effects(&[TaskSwitchEffect::ActivateWindow("editor".into())])
        .expect_ordered_effects(&[
            RecordedEffect::Launcher(LauncherEffect::ShowSurface(SurfaceIdentity(1))),
            RecordedEffect::Launcher(LauncherEffect::RequestFocus(FocusRequest {
                transaction: FocusTransaction(1),
                surface: SurfaceIdentity(1),
            })),
            RecordedEffect::Launcher(LauncherEffect::HideSurface(SurfaceIdentity(1))),
            RecordedEffect::Task(TaskSwitchEffect::ActivateWindow("editor".into())),
        ])
        .expect_authority_path(
            "window.active",
            &["WindowSurface geometry", "hit_test", "reduce_pointer_press"],
        );
}

#[test]
fn semantic_window_target_tracks_production_geometry_changes() {
    for (x, y, width, height) in [
        (0.0, 0.0, 80.0, 60.0),
        (973.0, 411.0, 377.0, 289.0),
    ] {
        scenario("window target follows fixture geometry")
            .window("target")
            .app("editor")
            .bounds(x, y, width, height)
            .window("other")
            .app("terminal")
            .bounds(x + width + 10.0, y, width, height)
            .active()
            .click_window("target")
            .expect_active("target")
            .expect_last_event_authority();
    }
}
