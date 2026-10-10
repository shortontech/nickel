//! Nickel's shipped CSS contracts exercised through generic Twinkle presentation.
use nickel_core::theme::{Appearance, ThemePalette};
use twinkle::Length;
use twinkle_presentation::css::StyleSheet;

#[test]
fn stock_shell_taskbar_follows_host_accent_and_mode() {
    let source = concat!(
        include_str!("../../../assets/plugins/nickel-default/src/styles/controls.css"),
        include_str!("../../../assets/plugins/nickel-default/src/styles/taskbar.css"),
        include_str!("../../../assets/plugins/nickel-default/src/styles/settings.css"),
        include_str!("../../../assets/plugins/nickel-default/src/styles/appearance.css"),
    );
    let mut sheet = StyleSheet::compile(source).unwrap();
    for mode in [
        nickel_core::theme::ThemeMode::Dark,
        nickel_core::theme::ThemeMode::Light,
    ] {
        let mut backgrounds = Vec::new();
        for accent in [[220, 130, 30], [45, 100, 220]] {
            let palette = ThemePalette::from_appearance(Appearance {
                mode,
                accent,
                intensity: 100,
            });
            sheet.set_palette(palette).unwrap();
            let background = sheet.resolve("window", None, Some("taskbar")).background;
            assert_eq!(background, Some(0xff00_0000 | palette.panel));
            for (element, class, base) in [
                ("div", "settings-sidebar", palette.panel),
                ("window", "settings-window", palette.background),
                ("div", "settings-detail", palette.background),
            ] {
                let actual = sheet
                    .resolve(element, None, Some(class))
                    .background
                    .unwrap()
                    & 0x00ff_ffff;
                if mode == nickel_core::theme::ThemeMode::Light {
                    for shift in [0, 8, 16] {
                        assert!(((actual >> shift) & 255) > ((base >> shift) & 255));
                    }
                    assert_ne!(actual, 0x00ff_ffff);
                } else {
                    assert_eq!(actual, base);
                }
            }
            assert_eq!(
                sheet
                    .resolve("button", None, Some("launcher-button"))
                    .icon_color,
                Some(0xff00_0000 | palette.text)
            );
            assert_eq!(
                sheet
                    .resolve("button", None, Some("settings-destination active"))
                    .background,
                Some(0xff00_0000 | palette.accent_soft)
            );
            let card = sheet.resolve("column", None, Some("appearance-card"));
            if mode == nickel_core::theme::ThemeMode::Light {
                assert_eq!(card.border_width, Some(0.0));
                assert_ne!(
                    card.background,
                    sheet
                        .resolve("div", None, Some("settings-detail"))
                        .background
                );
            } else {
                assert_eq!(card.background, Some(0xff00_0000 | palette.surface));
            }
            backgrounds.push(background);
        }
        assert_ne!(backgrounds[0], backgrounds[1]);
    }
}

#[test]
fn stock_shell_launcher_stylesheet_compiles() {
    let source = include_str!("../../../assets/plugins/nickel-default/src/styles/launcher.css");
    StyleSheet::compile(source).unwrap();
}

#[test]
fn stock_compound_controls_are_external_css() {
    let sheet = StyleSheet::compile(include_str!(
        "../../../assets/plugins/nickel-default/src/styles/controls.css"
    ))
    .unwrap();
    assert_eq!(
        sheet.resolve("switch-thumb", None, None).width,
        Some(Length::Px(18.0))
    );
    assert!(
        sheet
            .resolve("progress-fill", None, None)
            .background
            .is_some()
    );
}
