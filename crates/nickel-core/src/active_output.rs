//! Platform-neutral policy for choosing the output that owns a transient surface.

/// The input context that caused a transient surface to become visible.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvocationSource {
    Pointer,
    Touch,
    Keyboard,
    Controller,
    Accessibility,
    /// The native input event was handled before an asynchronous shell request.
    RecentInteraction,
}

/// Inputs to active-output selection, ordered independently of platform APIs.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ActiveOutputContext<'a> {
    pub pointer: Option<&'a str>,
    pub focused_surface: Option<&'a str>,
    pub recent_interaction: Option<&'a str>,
    pub primary: Option<&'a str>,
    /// Enabled outputs in deterministic platform order.
    pub enabled: &'a [&'a str],
}

impl ActiveOutputContext<'_> {
    fn enabled(&self, candidate: Option<&str>) -> Option<String> {
        candidate
            .filter(|candidate| self.enabled.contains(candidate))
            .map(str::to_owned)
    }
}

/// Selects the output for a new hidden-to-visible transition.
pub fn resolve_active_output(
    source: InvocationSource,
    context: ActiveOutputContext<'_>,
) -> Option<String> {
    let direct = match source {
        InvocationSource::Pointer | InvocationSource::Touch => context.pointer,
        InvocationSource::Keyboard
        | InvocationSource::Controller
        | InvocationSource::Accessibility => context.focused_surface,
        InvocationSource::RecentInteraction => None,
    };
    context
        .enabled(direct)
        .or_else(|| context.enabled(context.recent_interaction))
        .or_else(|| context.enabled(context.primary))
        .or_else(|| context.enabled.first().map(|output| (*output).to_owned()))
}

#[cfg(test)]
mod tests {
    use super::{ActiveOutputContext, InvocationSource, resolve_active_output};

    const ENABLED: &[&str] = &["DP-1", "HDMI-A-1"];

    fn context() -> ActiveOutputContext<'static> {
        ActiveOutputContext {
            pointer: Some("DP-1"),
            focused_surface: Some("HDMI-A-1"),
            recent_interaction: Some("HDMI-A-1"),
            primary: Some("DP-1"),
            enabled: ENABLED,
        }
    }

    #[test]
    fn pointer_and_touch_use_the_pointer_output() {
        assert_eq!(
            resolve_active_output(InvocationSource::Pointer, context()).as_deref(),
            Some("DP-1")
        );
        assert_eq!(
            resolve_active_output(InvocationSource::Touch, context()).as_deref(),
            Some("DP-1")
        );
    }

    #[test]
    fn keyboard_and_controller_use_the_focused_surface_output() {
        assert_eq!(
            resolve_active_output(InvocationSource::Keyboard, context()).as_deref(),
            Some("HDMI-A-1")
        );
        assert_eq!(
            resolve_active_output(InvocationSource::Controller, context()).as_deref(),
            Some("HDMI-A-1")
        );
    }

    #[test]
    fn asynchronous_shell_requests_use_captured_interaction_history() {
        assert_eq!(
            resolve_active_output(InvocationSource::RecentInteraction, context()).as_deref(),
            Some("HDMI-A-1")
        );
    }

    #[test]
    fn stale_direct_references_fall_through_the_policy() {
        let context = ActiveOutputContext {
            focused_surface: Some("disconnected"),
            recent_interaction: Some("HDMI-A-1"),
            primary: Some("DP-1"),
            enabled: ENABLED,
            ..Default::default()
        };
        assert_eq!(
            resolve_active_output(InvocationSource::Keyboard, context).as_deref(),
            Some("HDMI-A-1")
        );
    }

    #[test]
    fn primary_then_deterministic_output_are_final_fallbacks() {
        let primary = ActiveOutputContext {
            primary: Some("HDMI-A-1"),
            enabled: ENABLED,
            ..Default::default()
        };
        assert_eq!(
            resolve_active_output(InvocationSource::Keyboard, primary).as_deref(),
            Some("HDMI-A-1")
        );
        let deterministic = ActiveOutputContext {
            primary: Some("disconnected"),
            enabled: ENABLED,
            ..Default::default()
        };
        assert_eq!(
            resolve_active_output(InvocationSource::Keyboard, deterministic).as_deref(),
            Some("DP-1")
        );
    }
}
