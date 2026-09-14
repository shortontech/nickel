//! Small production-path fixtures for acceptance-oracle composition.

use nickel_core::{
    acceptance::{BoundedTrace, TraceAssertions},
    launcher::LauncherActivationSource,
    scenario::{ClickTarget, LauncherEffect, Surface, scenario},
};

#[test]
fn stale_focus_tail_cannot_hide_a_reopened_surface() {
    let opened = scenario("stale focus tail after reopen")
        .activate(
            LauncherActivationSource::Accessibility,
            ClickTarget::PanelLauncher,
        )
        .capture_focus("old")
        .click(ClickTarget::Desktop)
        .activate(
            LauncherActivationSource::Controller,
            ClickTarget::PanelLauncher,
        )
        .lose_captured_focus("old")
        .expect_visible(Surface::Launcher);

    let mut observed = BoundedTrace::new(8);
    for effect in opened.platform().launcher_effects() {
        observed.record(effect.clone());
    }
    observed.assert_count(
        &LauncherEffect::HideSurface(nickel_core::scenario::SurfaceIdentity(1)),
        1,
    );
    assert_eq!(observed.dropped(), 0);
}

#[test]
fn handled_launcher_background_has_no_fallback_activation() {
    let state = scenario("handled background is not fallback activation")
        .activate(
            LauncherActivationSource::Keyboard,
            ClickTarget::PanelLauncher,
        )
        .click(ClickTarget::LauncherBackground)
        .expect_visible(Surface::Launcher);

    let mut observed = BoundedTrace::new(4);
    for effect in state.platform().launcher_effects() {
        observed.record(effect.clone());
    }
    observed.assert_count(
        &LauncherEffect::ShowSurface(nickel_core::scenario::SurfaceIdentity(1)),
        1,
    );
    observed.assert_absent(&LauncherEffect::HideSurface(
        nickel_core::scenario::SurfaceIdentity(1),
    ));
}
