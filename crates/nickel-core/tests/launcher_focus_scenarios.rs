//! Behavioral launcher/focus contracts driven through the semantic scenario API.
//!
//! These cases keep the observable effect protocol in the oracle: visibility, focus requests,
//! surface identity, and focus restoration are all checked together.  They intentionally do not
//! inspect or mutate the scenario's internal state.

use nickel_core::focus::{FocusRequest, FocusTransaction};
use nickel_core::launcher::LauncherActivationSource;
use nickel_core::scenario::{
    ClickTarget, LauncherEffect, RecordedEffect, Surface, SurfaceIdentity, scenario,
};

fn first_focus_request() -> FocusRequest<SurfaceIdentity> {
    FocusRequest {
        transaction: FocusTransaction(1),
        surface: SurfaceIdentity(1),
    }
}

#[test]
fn opening_launcher_requests_focus_once_and_inside_click_is_a_surface_noop() {
    let opened = scenario("open launcher and inspect its surface")
        .click(ClickTarget::PanelLauncher)
        .capture_surface("launcher", Surface::Launcher);

    opened
        .click(ClickTarget::LauncherBackground)
        .expect_visible(Surface::Launcher)
        .expect_same_surface("launcher", Surface::Launcher)
        .expect_launcher_effects(&[
            LauncherEffect::ShowSurface(SurfaceIdentity(1)),
            LauncherEffect::RequestFocus(first_focus_request()),
        ]);
}

#[test]
fn closing_and_reopening_launcher_reuses_surface_but_allocates_a_new_focus_transaction() {
    let first = scenario("launcher reopen has fresh focus authority")
        .click(ClickTarget::PanelLauncher)
        .capture_surface("launcher", Surface::Launcher);

    let reopened = first
        .click(ClickTarget::PanelLauncher)
        .click(ClickTarget::PanelLauncher)
        .expect_visible(Surface::Launcher)
        .expect_same_surface("launcher", Surface::Launcher);

    reopened.expect_launcher_effects(&[
        LauncherEffect::ShowSurface(SurfaceIdentity(1)),
        LauncherEffect::RequestFocus(first_focus_request()),
        LauncherEffect::HideSurface(SurfaceIdentity(1)),
        LauncherEffect::ShowSurface(SurfaceIdentity(1)),
        LauncherEffect::RequestFocus(FocusRequest {
            transaction: FocusTransaction(2),
            surface: SurfaceIdentity(1),
        }),
    ]);
}

#[test]
fn acknowledged_launcher_focus_loss_hides_once_and_restores_previous_window_focus() {
    let opened = scenario("acknowledged launcher focus restoration")
        .window("editor")
        .app("editor")
        .active()
        .click(ClickTarget::PanelLauncher)
        .acknowledge_current_focus()
        .capture_focus("current");

    opened
        .lose_captured_focus("current")
        .expect_hidden(Surface::Launcher)
        .expect_active("editor")
        .expect_last_event_authority()
        .expect_launcher_effects(&[
            LauncherEffect::ShowSurface(SurfaceIdentity(1)),
            LauncherEffect::RequestFocus(first_focus_request()),
            LauncherEffect::HideSurface(SurfaceIdentity(1)),
            LauncherEffect::RestoreWindowFocus("editor".into()),
        ])
        .expect_ordered_effects(&[
            RecordedEffect::Launcher(LauncherEffect::ShowSurface(SurfaceIdentity(1))),
            RecordedEffect::Launcher(LauncherEffect::RequestFocus(first_focus_request())),
            RecordedEffect::Launcher(LauncherEffect::HideSurface(SurfaceIdentity(1))),
            RecordedEffect::Launcher(LauncherEffect::RestoreWindowFocus("editor".into())),
        ]);
}

#[test]
fn desktop_dismissal_is_idempotent_and_restores_focus_only_on_the_transition() {
    let opened = scenario("desktop dismissal is one transition")
        .window("terminal")
        .app("terminal")
        .active()
        .click(ClickTarget::PanelLauncher);

    opened
        .click(ClickTarget::Desktop)
        .click(ClickTarget::Desktop)
        .expect_hidden(Surface::Launcher)
        .expect_launcher_effects(&[
            LauncherEffect::ShowSurface(SurfaceIdentity(1)),
            LauncherEffect::RequestFocus(first_focus_request()),
            LauncherEffect::HideSurface(SurfaceIdentity(1)),
            LauncherEffect::RestoreWindowFocus("terminal".into()),
        ])
        .expect_ordered_effects(&[
            RecordedEffect::Launcher(LauncherEffect::ShowSurface(SurfaceIdentity(1))),
            RecordedEffect::Launcher(LauncherEffect::RequestFocus(first_focus_request())),
            RecordedEffect::Launcher(LauncherEffect::HideSurface(SurfaceIdentity(1))),
            RecordedEffect::Launcher(LauncherEffect::RestoreWindowFocus("terminal".into())),
        ]);
}

#[test]
fn controller_and_accessibility_panel_activation_share_the_focus_effect_contract() {
    scenario("controller opens launcher")
        .activate(
            LauncherActivationSource::Controller,
            ClickTarget::PanelLauncher,
        )
        .expect_visible(Surface::Launcher)
        .expect_launcher_effects(&[
            LauncherEffect::ShowSurface(SurfaceIdentity(1)),
            LauncherEffect::RequestFocus(first_focus_request()),
        ]);

    scenario("accessibility opens launcher")
        .activate(
            LauncherActivationSource::Accessibility,
            ClickTarget::PanelLauncher,
        )
        .expect_visible(Surface::Launcher)
        .expect_launcher_effects(&[
            LauncherEffect::ShowSurface(SurfaceIdentity(1)),
            LauncherEffect::RequestFocus(first_focus_request()),
        ]);
}

#[test]
fn invoking_taskbar_output_owns_launcher_without_replacing_its_surface() {
    scenario("launcher follows the invoking taskbar")
        .output("DP-1", 1920, 1080, 0)
        .output("HDMI-A-1", 2560, 1440, 1)
        .click_panel_launcher_on("HDMI-A-1")
        .capture_surface("launcher", Surface::Launcher)
        .expect_launcher_output("HDMI-A-1")
        .expect_same_surface("launcher", Surface::Launcher)
        .expect_launcher_effects(&[
            LauncherEffect::ShowSurface(SurfaceIdentity(1)),
            LauncherEffect::RequestFocus(first_focus_request()),
        ]);
}

#[test]
fn alternating_pointer_invocations_move_one_logical_launcher() {
    scenario("launcher follows alternating taskbars")
        .output("DP-1", 1920, 1080, 0)
        .output("portrait", 1440, 2560, 1)
        .click_panel_launcher_on("DP-1")
        .capture_surface("launcher", Surface::Launcher)
        .click_panel_launcher_on("DP-1")
        .click_panel_launcher_on("portrait")
        .expect_launcher_output("portrait")
        .expect_same_surface("launcher", Surface::Launcher);
}

#[test]
fn keyboard_and_controller_follow_the_focused_window_output() {
    for source in [
        LauncherActivationSource::Keyboard,
        LauncherActivationSource::Controller,
    ] {
        scenario(format!("{source:?} launcher follows focus"))
            .output("DP-1", 1920, 1080, 0)
            .output("HDMI-A-1", 2560, 1440, 1)
            .window("editor")
            .bounds(2100.0, 100.0, 800.0, 600.0)
            .active()
            .activate(source, ClickTarget::PanelLauncher)
            .expect_launcher_output("HDMI-A-1");
    }
}

#[test]
fn no_focus_uses_recent_interaction_then_primary() {
    scenario("keyboard launcher uses interaction history")
        .output("DP-1", 1920, 1080, 0)
        .output("HDMI-A-1", 2560, 1440, 1)
        .interact_on("HDMI-A-1")
        .activate(
            LauncherActivationSource::Keyboard,
            ClickTarget::PanelLauncher,
        )
        .expect_launcher_output("HDMI-A-1");

    scenario("keyboard launcher falls back to primary")
        .output("DP-1", 1920, 1080, 0)
        .output("HDMI-A-1", 2560, 1440, 1)
        .activate(
            LauncherActivationSource::Keyboard,
            ClickTarget::PanelLauncher,
        )
        .expect_launcher_output("DP-1");
}

#[test]
fn removing_the_launcher_output_relocates_the_existing_surface() {
    scenario("open launcher output disappears")
        .output("DP-1", 1920, 1080, 0)
        .output("HDMI-A-1", 2560, 1440, 1)
        .click_panel_launcher_on("HDMI-A-1")
        .capture_surface("launcher", Surface::Launcher)
        .disconnect_output("HDMI-A-1")
        .expect_launcher_output("DP-1")
        .expect_same_surface("launcher", Surface::Launcher);
}
