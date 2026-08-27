use crate::Color;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SemanticColors {
    pub window: Color,
    pub sidebar: Color,
    pub card: Color,
    pub raised: Color,
    pub hover: Color,
    pub primary_text: Color,
    pub secondary_text: Color,
    pub accent: Color,
    pub accent_soft: Color,
    pub positive: Color,
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
pub struct SemanticTheme {
    pub colors: SemanticColors,
    pub spacing: SpacingScale,
    pub radii: RadiusScale,
}

impl SemanticTheme {
    pub fn new(colors: SemanticColors) -> Self {
        Self {
            colors,
            spacing: SpacingScale::default(),
            radii: RadiusScale::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn product_neutral_theme_keeps_semantic_roles_explicit() {
        let colors = SemanticColors {
            window: 0x101010,
            sidebar: 0x181818,
            card: 0x202020,
            raised: 0x242424,
            hover: 0x303030,
            primary_text: 0xf0f0f0,
            secondary_text: 0xa0a0a0,
            accent: 0x9050e0,
            accent_soft: 0x402060,
            positive: 0x50c080,
        };
        let theme = SemanticTheme::new(colors);

        assert_eq!(theme.colors, colors);
        assert!(theme.spacing.section > theme.spacing.compact);
        assert!(theme.radii.overlay >= theme.radii.control);
    }
}
