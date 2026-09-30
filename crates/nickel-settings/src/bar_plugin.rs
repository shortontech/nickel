//! JSX Bar settings rendered through the shared native component path.

use nickel_core::shell_settings::{MAX_CONFIGURED_WORKSPACES, ShellSettings};
use nickel_i18n::Localizer;
use nickel_plugin_presentation::{
    components::PluginImages,
    page::{JsxPage, STALE_DATA},
};
use nickel_ui::{AnyView, SemanticTheme};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{SettingsApp, SettingsMessage, settings_plugin::StyledSettingsPage};

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
enum BarRequest {
    DisplayScope { scope: String },
    WindowScope { scope: String },
    DesktopCount { fraction: f32 },
}

fn validate_request(request: &BarRequest) -> Result<SettingsMessage, String> {
    match request {
        BarRequest::DisplayScope { scope } => match scope.as_str() {
            "primary" => Ok(SettingsMessage::BarPrimaryDisplay),
            "all" => Ok(SettingsMessage::BarAllDisplays),
            _ => Err("Bar display scope is invalid".into()),
        },
        BarRequest::WindowScope { scope } => match scope.as_str() {
            "display" => Ok(SettingsMessage::BarDisplayWindows),
            "all" => Ok(SettingsMessage::BarAllWindows),
            _ => Err("Bar window scope is invalid".into()),
        },
        BarRequest::DesktopCount { fraction } => {
            if !fraction.is_finite() {
                return Err("Bar desktop count is invalid".into());
            }
            Ok(crate::desktop_count_message(*fraction))
        }
    }
}

pub(super) struct BarPage {
    page: StyledSettingsPage,
}

impl BarPage {
    pub(super) fn retained_bytes(&self) -> usize {
        self.page.retained_bytes()
    }

    #[cfg(test)]
    pub(super) fn new() -> Result<Self, String> {
        Self::new_with_page(JsxPage::new(
            crate::settings_package::source(crate::settings_package::Script::Bar)?,
            crate::settings_package::manifest()?.clone(),
            None,
        )?)
    }

    pub(super) fn new_with_page(page: JsxPage) -> Result<Self, String> {
        Ok(Self {
            page: StyledSettingsPage::new(page),
        })
    }

    pub(super) fn render(
        &mut self,
        data: &Value,
        theme: SemanticTheme,
    ) -> Result<AnyView<SettingsMessage>, String> {
        let (node, stylesheet) = self.page.render(
            data,
            theme,
            include_str!("../../../assets/plugins/settings/settings-bar.css"),
        )?;
        Ok(node.view_as::<SettingsMessage>(&PluginImages::new(), stylesheet))
    }

    #[cfg(test)]
    pub(super) fn slider_action(&self) -> Option<usize> {
        self.page.node()?.slider_action("bar-desktop-count")
    }

    #[cfg(test)]
    pub(super) fn action_for_id(&self, id: &str) -> Option<usize> {
        self.page.node()?.button_action(id)
    }

    fn dispatch(
        &mut self,
        index: usize,
        value: Value,
        current_data: &Value,
    ) -> Result<SettingsMessage, String> {
        self.page.dispatch(index, &value, current_data, |effect| {
            let request: BarRequest =
                serde_json::from_value(effect).map_err(|error| error.to_string())?;
            validate_request(&request)
        })
    }
}

pub(super) fn projection(
    localizer: &Localizer,
    settings: &ShellSettings,
    display_count: usize,
    generation: u64,
) -> Value {
    json!({
        "showOn":localizer.text("settings-bar-show-on"),
        "primaryDisplay":localizer.text("settings-bar-primary-display"),
        "allDisplays":localizer.number("settings-bar-all-displays","count",display_count.max(1) as i64),
        "barOnAllDisplays":settings.bar_on_all_displays,
        "windowScope":localizer.text("settings-bar-window-scope"),
        "thisDisplay":localizer.text("settings-bar-this-display"),
        "allWindows":localizer.text("settings-bar-all-windows"),
        "allWindowsOnEveryBar":settings.all_windows_on_every_bar,
        "desktopsLabel":localizer.text("settings-bar-desktops"),
        "desktopCountLabel":localizer.number("settings-bar-desktop-count","count",i64::from(settings.desktop_count)),
        "desktopCount":settings.desktop_count,
        "activeDesktop":settings.active_desktop,
        "maxDesktops":MAX_CONFIGURED_WORKSPACES,
        "generation":generation,
    })
}

impl SettingsApp {
    pub(super) fn handle_bar_jsx_action(&mut self, index: usize, value: Value) {
        if self.page != crate::SettingsPage::Bar || !self.settings_jsx_enabled {
            return;
        }
        let data = projection(
            &self.localizer,
            &self.shell_settings,
            self.displays.len(),
            self.shell_topology_generation,
        );
        let result = self
            .bar_page
            .borrow_mut()
            .as_mut()
            .ok_or_else(|| "Bar page is not loaded".to_owned())
            .and_then(|page| page.as_mut().map_err(|error| error.clone()))
            .and_then(|page| page.dispatch(index, value, &data));
        match result {
            Ok(message) => self.handle_settings_message(message),
            Err(error) => {
                if error != STALE_DATA {
                    *self.bar_page.borrow_mut() = Some(Err(error));
                }
                self.request_redraw();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nickel_core::theme::ThemePalette;

    fn save_host_snapshot(host: &nickel_ui::UiHost<SettingsApp>, name: &str) {
        const WIDTH: u32 = 850;
        const HEIGHT: u32 = 580;
        let mut renderer = nickel_ui::SoftwareRenderer::new_pixel_buffer(WIDTH, HEIGHT, 1.0);
        host.render_software(&mut renderer);
        let image =
            image::ImageBuffer::<image::Rgba<u8>, Vec<u8>>::from_fn(WIDTH, HEIGHT, |x, y| {
                let pixel = renderer.pixels()[(y * WIDTH + x) as usize];
                image::Rgba([pixel.r, pixel.g, pixel.b, pixel.a])
            });
        let output = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/nickel-ui-snapshots")
            .join(name);
        std::fs::create_dir_all(output.parent().unwrap()).unwrap();
        image.save(output).unwrap();
    }

    #[test]
    fn shared_bar_page_renders() {
        let jsx = nickel_ui::UiHost::new(
            SettingsApp::with_initial_page(crate::SettingsPage::Bar),
            850,
            580,
        );
        save_host_snapshot(&jsx, "settings-bar-shared.png");
    }

    #[test]
    fn jsx_bar_page_renders_and_routes_typed_radio() {
        let settings = ShellSettings::default();
        let localizer = Localizer::system();
        let data = projection(&localizer, &settings, 2, 1);
        let theme = crate::semantic_theme(ThemePalette::from_appearance(
            nickel_core::theme::Appearance::default(),
        ));
        let mut page = BarPage::new().unwrap();
        let _ = page.render(&data, theme).unwrap();
        let action = page.action_for_id("bar-all-displays").unwrap();
        assert_eq!(
            page.dispatch(action, Value::Null, &data).unwrap(),
            SettingsMessage::BarAllDisplays
        );
        let slider = page.slider_action().unwrap();
        assert_eq!(
            page.dispatch(slider, Value::from(0.5), &data).unwrap(),
            SettingsMessage::SetDesktopCount(6)
        );
        let mut stale = data.clone();
        stale["generation"] = Value::from(2);
        assert!(page.dispatch(action, Value::Null, &stale).is_err());
    }
}
