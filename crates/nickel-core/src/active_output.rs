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

/// Selects the default output captured for a new top-level window transaction.
///
/// Unlike a launcher invocation, creating a window has no direct input source
/// attached to it. The focused application therefore wins, followed by the
/// compositor's last genuine interaction. A current pointer location is useful
/// only when no interaction has been recorded (for example, immediately after
/// login). Every candidate is checked against the enabled topology so a stale
/// focus or hot-unplugged output cannot strand the new window.
pub fn resolve_new_window_output(context: ActiveOutputContext<'_>) -> Option<String> {
    context
        .enabled(context.focused_surface)
        .or_else(|| context.enabled(context.recent_interaction))
        .or_else(|| context.enabled(context.pointer))
        .or_else(|| context.enabled(context.primary))
        .or_else(|| {
            context
                .enabled
                .iter()
                .min()
                .map(|output| (*output).to_owned())
        })
}

#[cfg(test)]
mod tests {
    use super::{
        ActiveOutputContext, InvocationSource, resolve_active_output, resolve_new_window_output,
    };

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

    #[test]
    fn new_windows_prefer_focused_application_then_recent_interaction() {
        assert_eq!(
            resolve_new_window_output(context()).as_deref(),
            Some("HDMI-A-1")
        );
        let without_focus = ActiveOutputContext {
            focused_surface: None,
            ..context()
        };
        assert_eq!(
            resolve_new_window_output(without_focus).as_deref(),
            Some("HDMI-A-1")
        );
    }

    #[test]
    fn new_windows_do_not_let_pointer_position_override_interaction_history() {
        let context = ActiveOutputContext {
            pointer: Some("DP-1"),
            focused_surface: None,
            recent_interaction: Some("HDMI-A-1"),
            primary: Some("DP-1"),
            enabled: ENABLED,
        };
        assert_eq!(
            resolve_new_window_output(context).as_deref(),
            Some("HDMI-A-1")
        );
    }

    #[test]
    fn new_window_policy_rejects_stale_candidates_and_falls_back_deterministically() {
        let stale = ActiveOutputContext {
            pointer: Some("unplugged-pointer"),
            focused_surface: Some("unplugged-focus"),
            recent_interaction: Some("unplugged-recent"),
            primary: Some("HDMI-A-1"),
            enabled: ENABLED,
        };
        assert_eq!(
            resolve_new_window_output(stale).as_deref(),
            Some("HDMI-A-1")
        );
        let no_primary = ActiveOutputContext {
            primary: Some("unplugged-primary"),
            enabled: &["HDMI-A-1", "DP-1"],
            ..Default::default()
        };
        assert_eq!(
            resolve_new_window_output(no_primary).as_deref(),
            Some("DP-1")
        );
    }
}
