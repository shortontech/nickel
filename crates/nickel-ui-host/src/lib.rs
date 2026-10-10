//! Nickel-owned adapters for the reusable Twinkle engine.

use nickel_core::theme::ThemePalette;
use twinkle::{SemanticTheme, SemanticTokenSet};

/// Resolves Nickel's effective palette into ordinary Twinkle appearance tokens.
/// This conveys appearance only; it does not grant settings mutation authority.
#[must_use]
pub fn semantic_theme(palette: ThemePalette) -> SemanticTheme {
    SemanticTheme::from_tokens(semantic_tokens(palette))
}

#[must_use]
pub fn semantic_tokens(palette: ThemePalette) -> SemanticTokenSet {
    SemanticTokenSet::standard(
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
    )
}

/// Builds a keyboard with Nickel's effective appearance and localized host labels.
#[must_use]
pub fn keyboard_app(palette: ThemePalette) -> twinkle::on_screen_keyboard::KeyboardApp {
    let mut app = twinkle::on_screen_keyboard::KeyboardApp::new(semantic_tokens(palette));
    app.set_labels(twinkle::on_screen_keyboard::KeyboardLabels {
        insufficient_space: nickel_i18n::system_text("ui-on-screen-keyboard-not-enough-space-for-usable-keys-increase-the-available-window-area-or-reduce-display-scaling"),
        layout_name: nickel_i18n::system_text("ui-on-screen-keyboard-english-us"),
    });
    app
}

/// Borrows Nickel's catalog to resolve action legends without exposing it to Twinkle.
pub struct ActionLabels<'a>(pub &'a nickel_i18n::Localizer);

impl twinkle::ActionLegendLocalizer for ActionLabels<'_> {
    type Label = nickel_i18n::ActionLabel;

    fn action_label(&self, label: Self::Label) -> String {
        self.0.action_label(label)
    }

    fn is_right_to_left(&self) -> bool {
        self.0.is_right_to_left()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use twinkle::ActionLegendLocalizer;

    #[test]
    fn nickel_catalog_supplies_expanded_and_rtl_action_labels() {
        let german = nickel_i18n::Localizer::for_locale(Some("de-DE"));
        let arabic = nickel_i18n::Localizer::for_locale(Some("ar"));
        assert_eq!(
            ActionLabels(&german).action_label(nickel_i18n::ActionLabel::Open),
            "Öffnen"
        );
        assert_eq!(
            ActionLabels(&arabic).action_label(nickel_i18n::ActionLabel::Open),
            "فتح"
        );
        assert!(!ActionLabels(&german).is_right_to_left());
        assert!(ActionLabels(&arabic).is_right_to_left());
    }
}

#[cfg(any(unix, windows))]
mod controller;

pub fn controller_source() -> twinkle::ControllerSource {
    #[cfg(any(unix, windows))]
    {
        controller::controller_source()
    }
    #[cfg(not(any(unix, windows)))]
    {
        twinkle::ControllerSource::Standalone
    }
}

pub fn run<A: twinkle::Application>(application: A) -> Result<(), Box<dyn std::error::Error>> {
    run_with_adapter(application, twinkle::DefaultHostAdapter)
}

pub fn run_with_adapter<A: twinkle::Application>(
    application: A,
    adapter: impl twinkle::HostAdapter<A>,
) -> Result<(), Box<dyn std::error::Error>> {
    twinkle::run_with_controller_source(application, adapter, controller_source())
}

#[cfg(target_os = "windows")]
pub fn run_with_adapter_on_any_thread<A: twinkle::Application>(
    application: A,
    adapter: impl twinkle::HostAdapter<A>,
) -> Result<(), Box<dyn std::error::Error>> {
    twinkle::run_with_controller_source_on_any_thread(application, adapter, controller_source())
}
