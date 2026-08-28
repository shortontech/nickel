//! Behavioral launcher/focus contracts driven through the semantic scenario API.
//!
//! These cases keep the observable effect protocol in the oracle: visibility, focus requests,
//! surface identity, and focus restoration are all checked together.  They intentionally do not
//! inspect or mutate the scenario's internal state.

use nickel_core::focus::{FocusRequest, FocusTransaction};
use nickel_core::launcher::LauncherActivationSource;
use nickel_core::scenario::{ClickTarget, LauncherEffect, Surface, SurfaceIdentity, scenario};

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
#[ignore = "harness finding: acknowledged focus loss hides without restoring prior window focus; proposed private spec: launcher focus restoration effects"]
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
        .expect_launcher_effects(&[
            LauncherEffect::ShowSurface(SurfaceIdentity(1)),
            LauncherEffect::RequestFocus(first_focus_request()),
            LauncherEffect::HideSurface(SurfaceIdentity(1)),
            LauncherEffect::RestoreWindowFocus("editor".into()),
        ]);
}

#[test]
#[ignore = "harness finding: pointer dismissal hides without restoring prior window focus; proposed private spec: launcher focus restoration effects"]
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
