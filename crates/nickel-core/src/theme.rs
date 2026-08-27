#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ThemeMode {
    Light,
    Dark,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Appearance {
    pub mode: ThemeMode,
    pub accent: [u8; 3],
    pub intensity: u8,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            mode: ThemeMode::Dark,
            accent: [0x4b, 0x8b, 0xd8],
            intensity: 85,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ThemePalette {
    pub background: u32,
    pub panel: u32,
    pub surface: u32,
    pub surface_hover: u32,
    pub text: u32,
    pub muted: u32,
    pub accent: u32,
    pub accent_soft: u32,
    pub complement: u32,
}

impl ThemePalette {
    pub fn from_appearance(appearance: Appearance) -> Self {
        let seed = Oklch::from_srgb(appearance.accent);
        let intensity = 0.15 + (f32::from(appearance.intensity.min(100)) / 100.0) * 1.10;
        let accent = Oklch::new(seed.l, seed.c * intensity, seed.h).to_rgb();
        let dark = appearance.mode == ThemeMode::Dark;
        Self {
            background: Oklch::new(if dark { 0.115 } else { 0.965 }, 0.006 * intensity, seed.h)
                .to_rgb(),
            panel: Oklch::new(
                if dark { 0.220 } else { 0.820 },
                if dark {
                    0.018 * intensity
                } else {
                    0.032 * intensity
                },
                seed.h,
            )
            .to_rgb(),
            surface: Oklch::new(if dark { 0.205 } else { 0.875 }, 0.010 * intensity, seed.h)
                .to_rgb(),
            surface_hover: Oklch::new(if dark { 0.275 } else { 0.815 }, 0.016 * intensity, seed.h)
                .to_rgb(),
            text: Oklch::new(if dark { 0.955 } else { 0.185 }, 0.008, seed.h).to_rgb(),
            muted: Oklch::new(if dark { 0.710 } else { 0.455 }, 0.018, seed.h).to_rgb(),
            accent,
            accent_soft: Oklch::new(if dark { 0.315 } else { 0.865 }, 0.040 * intensity, seed.h)
                .to_rgb(),
            complement: Oklch::new(
                if dark { 0.720 } else { 0.525 },
                (seed.c * intensity * 0.72).clamp(0.02, 0.20),
                seed.h + 180.0,
            )
            .to_rgb(),
        }
    }
}

#[derive(Clone, Copy)]
struct Oklch {
    l: f32,
    c: f32,
    h: f32,
}

impl Oklch {
    fn new(l: f32, c: f32, h: f32) -> Self {
        Self {
            l,
            c,
            h: h.rem_euclid(360.0),
        }
    }

    fn from_srgb(rgb: [u8; 3]) -> Self {
        let [r, g, b] = rgb.map(|channel| {
            let value = f32::from(channel) / 255.0;
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        });
        let l = 0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b;
        let m = 0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b;
        let s = 0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b;
        let (l, m, s) = (l.cbrt(), m.cbrt(), s.cbrt());
        let lightness = 0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s;
        let a = 1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s;
        let b = 0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s;
        Self::new(lightness, (a * a + b * b).sqrt(), b.atan2(a).to_degrees())
    }

    fn to_rgb(self) -> u32 {
        let radians = self.h.to_radians();
        let a = self.c * radians.cos();
        let b = self.c * radians.sin();
        let l = self.l + 0.396_337_78 * a + 0.215_803_76 * b;
        let m = self.l - 0.105_561_346 * a - 0.063_854_17 * b;
        let s = self.l - 0.089_484_18 * a - 1.291_485_5 * b;
        let (l, m, s) = (l * l * l, m * m * m, s * s * s);
        let channels = [
            4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s,
            -1.268_438 * l + 2.609_757_4 * m - 0.341_319_38 * s,
            -0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s,
        ]
        .map(|value| {
            let value = if value <= 0.0031308 {
                value * 12.92
            } else {
                1.055 * value.max(0.0).powf(1.0 / 2.4) - 0.055
            };
            (value.clamp(0.0, 1.0) * 255.0).round() as u8
        });
        (u32::from(channels[0]) << 16) | (u32::from(channels[1]) << 8) | u32::from(channels[2])
    }
}

pub fn accent_hue(rgb: [u8; 3]) -> u16 {
    Oklch::from_srgb(rgb).h.round().rem_euclid(360.0) as u16
}

pub fn accent_from_hue(hue: u16) -> [u8; 3] {
    let rgb = Oklch::new(0.62, 0.17, f32::from(hue.min(359))).to_rgb();
    [
        ((rgb >> 16) & 0xff) as u8,
        ((rgb >> 8) & 0xff) as u8,
        (rgb & 0xff) as u8,
    ]
}

#[cfg(test)]
mod tests {
    use super::{Appearance, ThemeMode, ThemePalette};

    #[test]
    fn windows_blue_produces_distinct_semantic_colors() {
        let palette = ThemePalette::from_appearance(Appearance {
            mode: ThemeMode::Dark,
            accent: [0x00, 0x78, 0xd4],
            intensity: 85,
        });
        assert_ne!(palette.background, palette.panel);
        assert_ne!(palette.accent, palette.complement);
    }

    #[test]
    fn light_and_dark_keep_the_same_accent_family() {
        let dark = ThemePalette::from_appearance(Appearance::default());
        let light = ThemePalette::from_appearance(Appearance {
            mode: ThemeMode::Light,
            ..Appearance::default()
        });
        assert_ne!(dark.background, light.background);
        assert_ne!(dark.text, light.text);
    }
}
