//! Ordinary Settings pages rendered by the shared JavaScript component runtime.

use nickel_i18n::Localizer;
use nickel_plugin_presentation::{
    components::{PanelNode, PluginImages, PluginMessage, PluginUiMessage},
    css::StyleSheet,
    page::JsxPage,
};
use nickel_ui::{AnyView, DragGesture, SemanticTheme};
use serde_json::{Value, json};

use crate::SettingsMessage;

impl PluginUiMessage for SettingsMessage {
    fn from_plugin(message: PluginMessage) -> Self {
        match message {
            PluginMessage::Click(action) | PluginMessage::Context(action) => {
                Self::JsxAction(action, "null".into())
            }
            PluginMessage::Value(action, value) => Self::JsxAction(action, value.to_string()),
            PluginMessage::Text(action, value) => Self::JsxAction(
                action,
                serde_json::to_string(&value).expect("a string serializes to JSON"),
            ),
            _ => Self::IgnoredPluginPresentation,
        }
    }

    fn from_plugin_scoped(message: PluginMessage, scope: Option<&str>) -> Self {
        let ordinary = Self::from_plugin(message);
        match (scope, ordinary) {
            (Some(scope), Self::JsxAction(index, value)) => {
                Self::JsxScopedAction(scope.to_owned(), index, value)
            }
            (_, ordinary) => ordinary,
        }
    }

    fn drag(_seed: Self, _gesture: DragGesture) -> Self {
        Self::IgnoredPluginPresentation
    }

    fn value(seed: Self, value: f32) -> Self {
        match seed {
            Self::JsxAction(action, _) => {
                Self::JsxAction(action, value.clamp(0.0, 1.0).to_string())
            }
            Self::JsxScopedAction(scope, action, _) => {
                Self::JsxScopedAction(scope, action, value.clamp(0.0, 1.0).to_string())
            }
            _ => unreachable!("slider seed retains its handler"),
        }
    }
}

pub(super) fn stylesheet_template(
    template: &str,
    theme: SemanticTheme,
) -> Result<StyleSheet, String> {
    let mut source = template.to_owned();
    for (token, value) in [
        ("@content@", format!("{}", theme.spacing.content)),
        ("@compact@", format!("{}", theme.spacing.compact)),
        ("@control@", format!("{}", theme.spacing.control)),
        ("@radius@", format!("{}", theme.radii.card)),
        (
            "@card@",
            format!("#{:06x}", theme.surfaces.card & 0x00ff_ffff),
        ),
        (
            "@window@",
            format!("#{:06x}", theme.surfaces.window & 0x00ff_ffff),
        ),
        (
            "@sidebar@",
            format!("#{:06x}", theme.surfaces.sidebar & 0x00ff_ffff),
        ),
        (
            "@raised@",
            format!("#{:06x}", theme.surfaces.raised & 0x00ff_ffff),
        ),
        (
            "@border@",
            format!("#{:06x}", theme.surfaces.raised & 0x00ff_ffff),
        ),
        (
            "@primary@",
            format!("#{:06x}", theme.text.primary & 0x00ff_ffff),
        ),
        (
            "@secondary@",
            format!("#{:06x}", theme.text.secondary & 0x00ff_ffff),
        ),
        (
            "@accent@",
            format!("#{:06x}", theme.accent.ordinary & 0x00ff_ffff),
        ),
        (
            "@on-accent@",
            format!("#{:06x}", theme.accent.on_accent & 0x00ff_ffff),
        ),
        (
            "@accent-soft@",
            format!("#{:06x}", theme.accent.soft & 0x00ff_ffff),
        ),
        (
            "@selected@",
            format!("#{:06x}", theme.borders.selected & 0x00ff_ffff),
        ),
    ] {
        source = source.replace(token, &value);
    }
    StyleSheet::compile(&source)
}

/// Shared render and theme lifecycle for Settings pages backed by JSX.
pub(super) struct StyledSettingsPage {
    page: JsxPage,
    stylesheet: StyleSheet,
    last_theme: Option<SemanticTheme>,
}

impl StyledSettingsPage {
    pub(super) fn new(page: JsxPage) -> Self {
        Self {
            page,
            stylesheet: StyleSheet::default(),
            last_theme: None,
        }
    }

    pub(super) fn retained_bytes(&self) -> usize {
        self.page.retained_bytes() + self.stylesheet.estimated_retained_bytes() as usize
    }

    pub(super) fn render(
        &mut self,
        data: &Value,
        theme: SemanticTheme,
        css: &str,
    ) -> Result<(&PanelNode, &StyleSheet), String> {
        self.page.render(data)?;
        if self.last_theme != Some(theme) {
            self.stylesheet = stylesheet_template(css, theme)?;
            self.last_theme = Some(theme);
        }
        let node = self.page.node().ok_or("Settings JSX page is unavailable")?;
        Ok((node, &self.stylesheet))
    }

    pub(super) fn node(&self) -> Option<&PanelNode> {
        self.page.node()
    }

    pub(super) fn stylesheet(&self) -> &StyleSheet {
        &self.stylesheet
    }

    pub(super) fn evaluate_with_data<T>(
        &mut self,
        data: &Value,
        expression: &str,
        parse: impl FnOnce(&Value) -> Result<T, String>,
    ) -> Result<T, String> {
        self.page.evaluate_with_data(data, expression, parse)
    }

    pub(super) fn dispatch(
        &mut self,
        index: usize,
        value: &Value,
        data: &Value,
        validate: impl FnOnce(Value) -> Result<SettingsMessage, String>,
    ) -> Result<SettingsMessage, String> {
        self.page.dispatch(index, value, data, validate)
    }
}

pub(super) struct OrdinaryPages {
    page: StyledSettingsPage,
}

impl OrdinaryPages {
    pub(super) fn retained_bytes(&self) -> usize {
        self.page.retained_bytes()
    }

    #[cfg(test)]
    pub(super) fn new() -> Result<Self, String> {
        Self::new_with_page(JsxPage::new(
            crate::settings_package::source(crate::settings_package::Script::OrdinaryPages)?,
            crate::settings_package::manifest()?.clone(),
            None,
        )?)
    }

    pub(super) fn new_with_page(page: JsxPage) -> Result<Self, String> {
        Ok(Self {
            page: StyledSettingsPage::new(page),
        })
    }

    fn render(
        &mut self,
        data: Value,
        theme: SemanticTheme,
    ) -> Result<AnyView<SettingsMessage>, String> {
        let (node, stylesheet) = self.page.render(
            &data,
            theme,
            include_str!("../../../assets/plugins/settings/settings-pages.css"),
        )?;
        Ok(node.view_as::<SettingsMessage>(&PluginImages::new(), stylesheet))
    }

    pub(super) fn render_keyboard(
        &mut self,
        localizer: &Localizer,
        theme: SemanticTheme,
    ) -> Result<AnyView<SettingsMessage>, String> {
        self.render(
            json!({
                "title": localizer.text("settings-keyboard-card-title"),
                "description": localizer.text("settings-keyboard-card-description"),
                "rows": [
                    {"id":"launcher", "label":localizer.text("settings-keyboard-open-launcher"), "value":"Super"},
                    {"id":"search", "label":localizer.text("settings-keyboard-search"), "value":localizer.text("settings-keyboard-search-value")},
                    {"id":"navigate", "label":localizer.text("settings-keyboard-navigate"), "value":"Arrow keys · Tab · Shift+Tab"},
                    {"id":"activate", "label":localizer.text("settings-keyboard-activate"), "value":"Enter"},
                    {"id":"back", "label":localizer.text("settings-keyboard-back"), "value":"Escape"},
                    {"id":"workspaces", "label":localizer.text("settings-keyboard-workspaces"), "value": if cfg!(target_os = "windows") {
                        localizer.text("settings-keyboard-workspaces-unavailable")
                    } else {
                        localizer.text("settings-keyboard-workspaces-value")
                    }}
                ]
            }),
            theme,
        )
    }

    pub(super) fn render_about(
        &mut self,
        localizer: &Localizer,
        theme: SemanticTheme,
    ) -> Result<AnyView<SettingsMessage>, String> {
        self.render(
            json!({
                "title": localizer.text("settings-about-card-title"),
                "description": localizer.text("settings-about-card-description"),
                "rows": [
                    {"id":"version", "label":localizer.text("settings-about-version"), "value":env!("CARGO_PKG_VERSION")},
                    {"id":"platform", "label":localizer.text("settings-about-platform"), "value":format!("{} · {}", std::env::consts::OS, std::env::consts::ARCH)}
                ]
            }),
            theme,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nickel_plugin_presentation::components::{PanelNode, render_panel};
    use nickel_plugin_runtime::JsxRuntime;

    struct PageHost(AnyView<SettingsMessage>);

    impl nickel_ui::Application for PageHost {
        type Message = SettingsMessage;

        fn update(&mut self, _message: Self::Message) {}

        fn view(&self, _context: nickel_ui::ViewContext) -> impl nickel_ui::View<Self::Message> {
            self.0.clone()
        }
    }

    #[test]
    fn bundled_settings_pages_render_through_shared_native_components() {
        let mut pages = OrdinaryPages::new().unwrap();
        let theme = crate::semantic_theme(nickel_core::theme::ThemePalette::from_appearance(
            nickel_core::theme::Appearance::default(),
        ));
        let localizer = Localizer::system();
        let keyboard = pages.render_keyboard(&localizer, theme).unwrap();
        let host = nickel_ui::UiHost::new(PageHost(keyboard.clone()), 700, 500);
        let title = host
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: nickel_ui::SemanticRole::Text,
                name: localizer.text("settings-keyboard-card-title"),
            })
            .unwrap();
        let launcher = host
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: nickel_ui::SemanticRole::Text,
                name: localizer.text("settings-keyboard-open-launcher"),
            })
            .unwrap();
        assert!(title.bounds.size.height >= 20.0);
        assert!(launcher.bounds.origin.y > title.bounds.origin.y);
        let mut renderer = nickel_ui::SoftwareRenderer::new_pixel_buffer(700, 500, 1.0);
        host.render_software(&mut renderer);
        let image = image::ImageBuffer::<image::Rgba<u8>, Vec<u8>>::from_fn(700, 500, |x, y| {
            let pixel = renderer.pixels()[(y * 700 + x) as usize];
            image::Rgba([pixel.r, pixel.g, pixel.b, pixel.a])
        });
        let output = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/nickel-ui-snapshots/settings-keyboard-shared.png");
        std::fs::create_dir_all(output.parent().unwrap()).unwrap();
        image.save(output).unwrap();
        let about = pages.render_about(&localizer, theme).unwrap();
        let _ = (keyboard, about);
        assert!(matches!(
            pages.page.node(),
            Some(PanelNode::Div { children, .. }) if children.len() == 4
        ));
        assert_eq!(
            pages
                .page
                .stylesheet()
                .resolve("div", None, Some("settings-card"))
                .background,
            Some(0xff00_0000 | theme.surfaces.card & 0x00ff_ffff)
        );
        let light = crate::semantic_theme(nickel_core::theme::ThemePalette::from_appearance(
            nickel_core::theme::Appearance {
                mode: nickel_core::theme::ThemeMode::Light,
                ..nickel_core::theme::Appearance::default()
            },
        ));
        let light_page = pages.render_keyboard(&localizer, light).unwrap();
        assert_eq!(
            pages
                .page
                .stylesheet()
                .resolve("div", None, Some("settings-card"))
                .background,
            Some(0xff00_0000 | light.surfaces.card & 0x00ff_ffff)
        );
        let light_host = nickel_ui::UiHost::new(PageHost(light_page), 700, 500);
        let mut light_renderer = nickel_ui::SoftwareRenderer::new_pixel_buffer(700, 500, 1.0);
        light_host.render_software(&mut light_renderer);
        let light_image =
            image::ImageBuffer::<image::Rgba<u8>, Vec<u8>>::from_fn(700, 500, |x, y| {
                let pixel = light_renderer.pixels()[(y * 700 + x) as usize];
                image::Rgba([pixel.r, pixel.g, pixel.b, pixel.a])
            });
        let light_output = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/nickel-ui-snapshots/settings-keyboard-shared-light.png");
        light_image.save(light_output).unwrap();
    }

    #[test]
    fn old_settings_only_component_is_rejected_by_shared_renderer() {
        let mut runtime = JsxRuntime::new(
            "function App() { return h('settings-card', {label: 'Old'}); }",
            None,
        )
        .unwrap();
        assert!(
            render_panel(
                &mut runtime,
                crate::settings_package::manifest().unwrap(),
                None,
                "__nickelRender()",
            )
            .is_err()
        );
    }
}
