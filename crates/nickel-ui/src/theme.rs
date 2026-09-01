use crate::Color;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SurfaceColors {
    pub window: Color,
    pub sidebar: Color,
    pub card: Color,
    pub raised: Color,
    pub hover: Color,
    pub pressed: Color,
    pub selected: Color,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BorderColors {
    pub subtle: Color,
    pub ordinary: Color,
    pub strong: Color,
    pub focus: Color,
    /// Distinct highlight for the control currently targeted by a controller.
    pub controller_focus: Color,
    pub selected: Color,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TextColors {
    pub primary: Color,
    pub secondary: Color,
    pub disabled: Color,
    pub inverse: Color,
    pub accent: Color,
    pub success: Color,
    pub warning: Color,
    pub danger: Color,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AccentColors {
    pub ordinary: Color,
    pub hover: Color,
    pub pressed: Color,
    pub soft: Color,
    pub on_accent: Color,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpacingScale {
    pub compact: f32,
    pub control: f32,
    pub content: f32,
    pub section: f32,
}

impl Default for SpacingScale {
    fn default() -> Self {
        Self {
            compact: 4.0,
            control: 8.0,
            content: 12.0,
            section: 20.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RadiusScale {
    pub control: f32,
    pub card: f32,
    pub overlay: f32,
}

impl Default for RadiusScale {
    fn default() -> Self {
        Self {
            control: 6.0,
            card: 8.0,
            overlay: 10.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SizingScale {
    pub control_height: f32,
    pub compact_control_height: f32,
    pub icon: f32,
    pub large_icon: f32,
    pub focus_ring: f32,
    pub border: f32,
}

impl Default for SizingScale {
    fn default() -> Self {
        Self {
            control_height: 36.0,
            compact_control_height: 28.0,
            icon: 18.0,
            large_icon: 24.0,
            focus_ring: 2.0,
            border: 1.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FontWeight {
    Regular,
    Medium,
    Semibold,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextStyle {
    pub size: f32,
    pub line_height: f32,
    pub weight: FontWeight,
}

impl TextStyle {
    pub const fn new(size: f32, line_height: f32, weight: FontWeight) -> Self {
        Self {
            size,
            line_height,
            weight,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TypographyScale {
    pub page_title: TextStyle,
    pub section_title: TextStyle,
    pub body: TextStyle,
    pub control_label: TextStyle,
    pub supporting_text: TextStyle,
    pub caption: TextStyle,
}

impl Default for TypographyScale {
    fn default() -> Self {
        Self {
            page_title: TextStyle::new(26.0, 34.0, FontWeight::Regular),
            section_title: TextStyle::new(17.0, 24.0, FontWeight::Medium),
            body: TextStyle::new(15.0, 22.0, FontWeight::Regular),
            control_label: TextStyle::new(15.0, 20.0, FontWeight::Medium),
            supporting_text: TextStyle::new(13.0, 19.0, FontWeight::Regular),
            caption: TextStyle::new(12.0, 17.0, FontWeight::Regular),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EasingCurve {
    Linear,
    EaseOut,
    EaseInOut,
    CubicBezier { x1: f32, y1: f32, x2: f32, y2: f32 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MotionScale {
    pub short_ms: u16,
    pub ordinary_ms: u16,
    pub enter: EasingCurve,
    pub change: EasingCurve,
}

impl Default for MotionScale {
    fn default() -> Self {
        Self {
            short_ms: 100,
            ordinary_ms: 180,
            enter: EasingCurve::EaseOut,
            change: EasingCurve::EaseInOut,
        }
    }
}

impl MotionScale {
    pub const fn reduced(self) -> Self {
        Self {
            short_ms: 0,
            ordinary_ms: 0,
            ..self
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AppearancePreference {
    Light,
    Dark,
    Automatic,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResolvedAppearance {
    Light,
    Dark,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContrastPreference {
    Standard,
    High,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransparencyPreference {
    Allow,
    Reduce,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MotionPreference {
    Full,
    Reduce,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AccessibilityPreferences {
    pub contrast: ContrastPreference,
    pub transparency: TransparencyPreference,
    pub motion: MotionPreference,
}

impl Default for AccessibilityPreferences {
    fn default() -> Self {
        Self {
            contrast: ContrastPreference::Standard,
            transparency: TransparencyPreference::Allow,
            motion: MotionPreference::Full,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ThemePreferences {
    pub appearance: AppearancePreference,
    pub accessibility: AccessibilityPreferences,
}

impl Default for ThemePreferences {
    fn default() -> Self {
        Self {
            appearance: AppearancePreference::Automatic,
            accessibility: AccessibilityPreferences::default(),
        }
    }
}

/// Preferences reported by a platform adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlatformThemePreferences {
    pub appearance: ResolvedAppearance,
    pub accessibility: AccessibilityPreferences,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResolvedThemePreferences {
    pub appearance: ResolvedAppearance,
    pub high_contrast: bool,
    pub reduced_transparency: bool,
    pub reduced_motion: bool,
}

impl ThemePreferences {
    /// Resolves automatic appearance and lets either source strengthen an
    /// accessibility preference.
    pub const fn resolve(self, platform: PlatformThemePreferences) -> ResolvedThemePreferences {
        let appearance = match self.appearance {
            AppearancePreference::Light => ResolvedAppearance::Light,
            AppearancePreference::Dark => ResolvedAppearance::Dark,
            AppearancePreference::Automatic => platform.appearance,
        };
        ResolvedThemePreferences {
            appearance,
            high_contrast: matches!(self.accessibility.contrast, ContrastPreference::High)
                || matches!(platform.accessibility.contrast, ContrastPreference::High),
            reduced_transparency: matches!(
                self.accessibility.transparency,
                TransparencyPreference::Reduce
            ) || matches!(
                platform.accessibility.transparency,
                TransparencyPreference::Reduce
            ),
            reduced_motion: matches!(self.accessibility.motion, MotionPreference::Reduce)
                || matches!(platform.accessibility.motion, MotionPreference::Reduce),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SemanticTokenSet {
    pub surfaces: SurfaceColors,
    pub borders: BorderColors,
    pub text: TextColors,
    pub accent: AccentColors,
    pub spacing: SpacingScale,
    pub radii: RadiusScale,
    pub sizing: SizingScale,
    pub typography: TypographyScale,
    pub motion: MotionScale,
}

impl SemanticTokenSet {
    #[allow(clippy::too_many_arguments)]
    pub fn standard(
        window: Color,
        sidebar: Color,
        card: Color,
        raised: Color,
        hover: Color,
        primary_text: Color,
        secondary_text: Color,
        accent: Color,
        accent_soft: Color,
        controller_focus: Color,
        positive: Color,
    ) -> Self {
        Self {
            surfaces: SurfaceColors {
                window,
                sidebar,
                card,
                raised,
                hover,
                pressed: mix(hover, primary_text, 18),
                selected: accent_soft,
            },
            borders: BorderColors {
                subtle: mix(card, primary_text, 12),
                ordinary: mix(card, primary_text, 22),
                strong: mix(card, primary_text, 38),
                focus: accent,
                controller_focus,
                selected: accent,
            },
            text: TextColors {
                primary: primary_text,
                secondary: secondary_text,
                disabled: mix(secondary_text, window, 48),
                inverse: contrasting_text(primary_text),
                accent,
                success: positive,
                warning: 0xe6a23c,
                danger: 0xdc5a66,
            },
            accent: AccentColors {
                ordinary: accent,
                hover: mix(accent, primary_text, 12),
                pressed: mix(accent, window, 18),
                soft: accent_soft,
                on_accent: contrasting_text(accent),
            },
            spacing: SpacingScale::default(),
            radii: RadiusScale::default(),
            sizing: SizingScale::default(),
            typography: TypographyScale::default(),
            motion: MotionScale::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SemanticTheme {
    pub surfaces: SurfaceColors,
    pub borders: BorderColors,
    pub text: TextColors,
    pub accent: AccentColors,
    pub spacing: SpacingScale,
    pub radii: RadiusScale,
    pub sizing: SizingScale,
    pub typography: TypographyScale,
    pub motion: MotionScale,
}

impl SemanticTheme {
    pub const fn from_tokens(tokens: SemanticTokenSet) -> Self {
        Self {
            surfaces: tokens.surfaces,
            borders: tokens.borders,
            text: tokens.text,
            accent: tokens.accent,
            spacing: tokens.spacing,
            radii: tokens.radii,
            sizing: tokens.sizing,
            typography: tokens.typography,
            motion: tokens.motion,
        }
    }

    pub const fn tokens(self) -> SemanticTokenSet {
        SemanticTokenSet {
            surfaces: self.surfaces,
            borders: self.borders,
            text: self.text,
            accent: self.accent,
            spacing: self.spacing,
            radii: self.radii,
            sizing: self.sizing,
            typography: self.typography,
            motion: self.motion,
        }
    }

    pub const fn with_reduced_motion(mut self) -> Self {
        self.motion = self.motion.reduced();
        self
    }

    /// Resolves light/dark palettes and accessibility preferences into the
    /// semantic roles consumed by components.
    pub fn resolve(
        light: SemanticTokenSet,
        dark: SemanticTokenSet,
        preferences: ResolvedThemePreferences,
    ) -> Self {
        let tokens = match preferences.appearance {
            ResolvedAppearance::Light => light,
            ResolvedAppearance::Dark => dark,
        };
        let mut theme = Self::from_tokens(tokens);
        if preferences.high_contrast {
            theme.borders.subtle = mix(theme.surfaces.card, theme.text.primary, 60);
            theme.borders.ordinary = theme.text.primary;
            theme.borders.strong = theme.text.primary;
            theme.text.secondary = mix(theme.text.primary, theme.surfaces.window, 18);
        }
        if preferences.reduced_transparency {
            theme.surfaces.window = opaque(theme.surfaces.window);
            theme.surfaces.sidebar = opaque(theme.surfaces.sidebar);
            theme.surfaces.card = opaque(theme.surfaces.card);
            theme.surfaces.raised = opaque(theme.surfaces.raised);
        }
        if preferences.reduced_motion {
            theme.motion = theme.motion.reduced();
        }
        theme
    }
}

fn contrasting_text(color: Color) -> Color {
    let dark = 0x111111;
    let light = 0xffffff;
    if contrast_ratio(color, dark) >= contrast_ratio(color, light) {
        dark
    } else {
        light
    }
}

fn relative_luminance(color: Color) -> f32 {
    let channel = |shift: u32| {
        let value = ((color >> shift) & 0xff_u32) as f32 / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(16) + 0.7152 * channel(8) + 0.0722 * channel(0)
}

fn contrast_ratio(left: Color, right: Color) -> f32 {
    let left = relative_luminance(left);
    let right = relative_luminance(right);
    (left.max(right) + 0.05) / (left.min(right) + 0.05)
}

fn mix(left: Color, right: Color, right_percent: u32) -> Color {
    let left_percent = 100 - right_percent;
    let channel = |shift: u32| {
        ((((left >> shift) & 0xff_u32) * left_percent
            + ((right >> shift) & 0xff_u32) * right_percent
            + 50)
            / 100)
            << shift
    };
    channel(16) | channel(8) | channel(0)
}

fn opaque(color: Color) -> Color {
    color & 0x00ff_ffff
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens() -> SemanticTokenSet {
        SemanticTokenSet::standard(
            0x101010, 0x181818, 0x202020, 0x242424, 0x303030, 0xf0f0f0, 0xa0a0a0, 0x9050e0,
            0x402060, 0x50c080, 0x50c080,
        )
    }

    #[test]
    fn standard_tokens_expand_into_distinct_roles() {
        let theme = SemanticTheme::from_tokens(tokens());
        assert_eq!(theme.surfaces.selected, 0x402060);
        assert_eq!(theme.borders.focus, 0x9050e0);
        assert_eq!(theme.borders.controller_focus, 0x50c080);
        assert_ne!(theme.borders.controller_focus, theme.borders.focus);
        assert_eq!(theme.text.success, 0x50c080);
        assert_eq!(theme.accent.on_accent, 0xffffff);
        assert_ne!(theme.surfaces.hover, theme.surfaces.pressed);
        assert_ne!(theme.accent.ordinary, theme.accent.hover);
        assert!(theme.spacing.section > theme.spacing.compact);
        assert!(theme.sizing.control_height > theme.sizing.icon);
        assert!(theme.typography.page_title.size > theme.typography.body.size);
    }

    #[test]
    fn explicit_tokens_round_trip_without_policy() {
        let mut tokens = tokens();
        tokens.text.warning = 0xabcdef;
        tokens.motion.ordinary_ms = 240;
        assert_eq!(SemanticTheme::from_tokens(tokens).tokens(), tokens);
    }

    #[test]
    fn automatic_appearance_combines_accessibility_needs() {
        let user = ThemePreferences {
            accessibility: AccessibilityPreferences {
                contrast: ContrastPreference::High,
                ..AccessibilityPreferences::default()
            },
            ..ThemePreferences::default()
        };
        let platform = PlatformThemePreferences {
            appearance: ResolvedAppearance::Dark,
            accessibility: AccessibilityPreferences {
                transparency: TransparencyPreference::Reduce,
                motion: MotionPreference::Reduce,
                ..AccessibilityPreferences::default()
            },
        };
        assert_eq!(
            user.resolve(platform),
            ResolvedThemePreferences {
                appearance: ResolvedAppearance::Dark,
                high_contrast: true,
                reduced_transparency: true,
                reduced_motion: true,
            }
        );
    }

    #[test]
    fn explicit_appearance_only_overrides_platform_appearance() {
        let user = ThemePreferences {
            appearance: AppearancePreference::Light,
            ..ThemePreferences::default()
        };
        let platform = PlatformThemePreferences {
            appearance: ResolvedAppearance::Dark,
            accessibility: AccessibilityPreferences {
                motion: MotionPreference::Reduce,
                ..AccessibilityPreferences::default()
            },
        };
        let resolved = user.resolve(platform);
        assert_eq!(resolved.appearance, ResolvedAppearance::Light);
        assert!(resolved.reduced_motion);
    }

    #[test]
    fn reduced_motion_preserves_roles_and_removes_duration() {
        let theme = SemanticTheme::from_tokens(tokens());
        let reduced = theme.with_reduced_motion();
        assert_eq!(
            (reduced.motion.short_ms, reduced.motion.ordinary_ms),
            (0, 0)
        );
        assert_eq!(reduced.motion.enter, theme.motion.enter);
        assert_eq!(reduced.motion.change, theme.motion.change);
    }

    #[test]
    fn resolved_theme_applies_appearance_contrast_transparency_and_motion() {
        let light = SemanticTokenSet::standard(
            0x80f0f0f0, 0x181818, 0x202020, 0x242424, 0x303030, 0x111111, 0xa0a0a0, 0x9050e0,
            0x402060, 0x50c080, 0x50c080,
        );
        let resolved = SemanticTheme::resolve(
            light,
            tokens(),
            ResolvedThemePreferences {
                appearance: ResolvedAppearance::Light,
                high_contrast: true,
                reduced_transparency: true,
                reduced_motion: true,
            },
        );
        assert_eq!(resolved.surfaces.window, 0xf0f0f0);
        assert_eq!(resolved.borders.ordinary, resolved.text.primary);
        assert_eq!(resolved.motion.ordinary_ms, 0);
    }

    #[test]
    fn color_helpers_are_deterministic() {
        assert_eq!(contrasting_text(0x000000), 0xffffff);
        assert_eq!(contrasting_text(0xffffff), 0x111111);
        assert_eq!(mix(0x000000, 0xffffff, 50), 0x808080);
    }

    #[test]
    fn representative_accent_families_keep_readable_and_distinct_states() {
        for accent in [0xd94b4b, 0x4b8bd8, 0x45a56b] {
            let tokens = SemanticTokenSet::standard(
                0x101010,
                0x181818,
                0x202020,
                0x242424,
                0x303030,
                0xf0f0f0,
                0xa0a0a0,
                accent,
                mix(accent, 0x111111, 68),
                0x50c080,
                0x50c080,
            );
            let theme = SemanticTheme::from_tokens(tokens);
            assert!(contrast_ratio(theme.accent.ordinary, theme.accent.on_accent) >= 4.5);
            assert_ne!(theme.accent.ordinary, theme.accent.hover);
            assert_ne!(theme.accent.hover, theme.accent.pressed);
            assert_ne!(theme.surfaces.hover, theme.surfaces.pressed);
            assert_ne!(theme.borders.focus, theme.surfaces.selected);
        }
    }
}
