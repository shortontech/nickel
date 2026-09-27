//! JSX Bar settings with native presentation and typed shell transactions.

use nickel_core::{
    shell_settings::{MAX_CONFIGURED_WORKSPACES, ShellSettings},
    theme::ThemePalette,
};
use nickel_i18n::Localizer;
use nickel_plugin_runtime::JsxRuntime;
use nickel_ui::{
    AnyView, Column, Container, Insets, RadioGroup, RadioOption, Row, SemanticTheme, SliderField,
    Text, TextAlign,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{SettingsApp, SettingsMessage};

const STALE_STATUS: &str = "Bar status changed; refresh the page";

#[derive(Clone, Debug)]
struct Radio {
    id: String,
    label: String,
    selected: bool,
    action: usize,
}

#[derive(Clone, Debug)]
struct RadioGroupModel {
    id: String,
    options: Vec<Radio>,
}

#[derive(Clone, Debug)]
struct SliderModel {
    id: String,
    label: String,
    description: String,
    value_label: String,
    percent: f32,
    action: usize,
}

#[derive(Clone, Debug)]
struct Desktop {
    number: u8,
    selected: bool,
}

#[derive(Clone, Debug)]
struct BarTree {
    show_on: String,
    display_scope: RadioGroupModel,
    window_scope_label: String,
    window_scope: RadioGroupModel,
    slider: SliderModel,
    desktops: Vec<Desktop>,
}

impl BarTree {
    fn retained_bytes(&self) -> usize {
        let radio_bytes = |group: &RadioGroupModel| {
            group.id.capacity()
                + group.options.capacity() * std::mem::size_of::<Radio>()
                + group
                    .options
                    .iter()
                    .map(|option| option.id.capacity() + option.label.capacity())
                    .sum::<usize>()
        };
        std::mem::size_of::<Self>()
            + self.show_on.capacity()
            + radio_bytes(&self.display_scope)
            + self.window_scope_label.capacity()
            + radio_bytes(&self.window_scope)
            + self.slider.id.capacity()
            + self.slider.label.capacity()
            + self.slider.description.capacity()
            + self.slider.value_label.capacity()
            + self.desktops.capacity() * std::mem::size_of::<Desktop>()
    }

    fn parse(value: &Value) -> Result<Self, String> {
        if kind(value) != Some("settings-bar") {
            return Err("Bar page root is invalid".into());
        }
        let nodes = children(value)?;
        if nodes.len() != 6 {
            return Err("Bar page has an unexpected component count".into());
        }
        if kind(&nodes[0]) != Some("settings-text") || kind(&nodes[2]) != Some("settings-text") {
            return Err("Bar page headings are invalid".into());
        }
        if kind(&nodes[5]) != Some("settings-desktops") {
            return Err("Bar workspace preview is invalid".into());
        }
        let desktops = children(&nodes[5])?
            .iter()
            .map(|item| {
                if kind(item) != Some("settings-desktop") {
                    return Err("Bar workspace item is invalid".into());
                }
                let number = item
                    .get("count")
                    .and_then(Value::as_u64)
                    .and_then(|number| u8::try_from(number).ok())
                    .ok_or("Bar workspace number is invalid")?;
                if number == 0 || number > MAX_CONFIGURED_WORKSPACES {
                    return Err("Bar workspace number is out of range".into());
                }
                Ok(Desktop {
                    number,
                    selected: selected(item)?,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        if desktops.is_empty() || desktops.len() > usize::from(MAX_CONFIGURED_WORKSPACES) {
            return Err("Bar workspace count is invalid".into());
        }
        if desktops
            .iter()
            .enumerate()
            .any(|(index, desktop)| desktop.number != index as u8 + 1)
            || desktops.iter().filter(|desktop| desktop.selected).count() > 1
        {
            return Err("Bar workspace preview is inconsistent".into());
        }
        Ok(Self {
            show_on: text(&nodes[0], "label")?,
            display_scope: RadioGroupModel::parse(&nodes[1])?,
            window_scope_label: text(&nodes[2], "label")?,
            window_scope: RadioGroupModel::parse(&nodes[3])?,
            slider: SliderModel::parse(&nodes[4])?,
            desktops,
        })
    }

    fn view(&self, theme: SemanticTheme, palette: ThemePalette) -> AnyView<SettingsMessage> {
        let radio_group = |group: &RadioGroupModel| {
            RadioGroup::new(
                group
                    .options
                    .iter()
                    .map(|option| {
                        RadioOption::new(
                            theme,
                            SettingsMessage::BarJsxAction(option.action),
                            &option.label,
                            option.selected,
                        )
                        .id(option.id.as_str())
                    })
                    .collect::<Vec<_>>(),
            )
            .id(group.id.as_str())
        };
        let desktop_choices = self
            .desktops
            .iter()
            .map(|desktop| {
                let selected = desktop.selected;
                Container::new()
                    .width(64.0)
                    .height(46.0)
                    .background(palette.surface)
                    .border(
                        if selected {
                            palette.accent
                        } else {
                            palette.muted
                        },
                        2.0,
                    )
                    .padding(Insets {
                        top: 9.0,
                        right: 4.0,
                        bottom: 4.0,
                        left: 4.0,
                    })
                    .child(
                        Text::new(desktop.number.to_string())
                            .align(TextAlign::Center)
                            .scale(1.0)
                            .color(if selected {
                                palette.text
                            } else {
                                palette.muted
                            }),
                    )
            })
            .collect::<Vec<_>>();
        AnyView::new(
            Column::new()
                .grow(1.0)
                .padding(Insets {
                    top: 24.0,
                    right: 40.0,
                    bottom: 20.0,
                    left: 20.0,
                })
                .gap(14.0)
                .child(Text::new(&self.show_on).color(palette.text).height(20.0))
                .child(radio_group(&self.display_scope))
                .child(
                    Text::new(&self.window_scope_label)
                        .color(palette.text)
                        .height(20.0),
                )
                .child(radio_group(&self.window_scope))
                .child(
                    SliderField::new(
                        theme,
                        &self.slider.label,
                        &self.slider.description,
                        &self.slider.value_label,
                        self.slider.percent,
                        bar_slider_message,
                    )
                    .id(self.slider.id.as_str()),
                )
                .child(Row::new().height(46.0).gap(8.0).children(desktop_choices)),
        )
    }

    #[cfg(test)]
    fn action_for_id(&self, id: &str) -> Option<usize> {
        self.display_scope
            .options
            .iter()
            .chain(self.window_scope.options.iter())
            .find(|radio| radio.id == id)
            .map(|radio| radio.action)
    }
}

impl RadioGroupModel {
    fn parse(value: &Value) -> Result<Self, String> {
        if kind(value) != Some("settings-radio-group") {
            return Err("Bar radio group is invalid".into());
        }
        let options = children(value)?;
        if options.len() != 2 {
            return Err("Bar radio group must have two options".into());
        }
        let options = options
            .iter()
            .map(|item| {
                if kind(item) != Some("settings-radio") {
                    return Err("Bar radio option is invalid".into());
                }
                Ok(Radio {
                    id: text(item, "id")?,
                    label: text(item, "label")?,
                    selected: selected(item)?,
                    action: action(item)?,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        if options.iter().filter(|option| option.selected).count() != 1 {
            return Err("Bar radio group must have one selected option".into());
        }
        Ok(Self {
            id: text(value, "id")?,
            options,
        })
    }
}

impl SliderModel {
    fn parse(value: &Value) -> Result<Self, String> {
        if kind(value) != Some("settings-slider") {
            return Err("Bar slider is invalid".into());
        }
        let children = children(value)?;
        if children.len() != 1 || kind(&children[0]) != Some("settings-description") {
            return Err("Bar slider description is invalid".into());
        }
        let percent = value
            .get("percent")
            .and_then(Value::as_f64)
            .ok_or("Bar slider value is invalid")?;
        if !(0.0..=1.0).contains(&percent) {
            return Err("Bar slider value is out of range".into());
        }
        Ok(Self {
            id: text(value, "id")?,
            label: text(value, "label")?,
            description: text(&children[0], "label")?,
            value_label: text(value, "value")?,
            percent: percent as f32,
            action: action(value)?,
        })
    }
}

fn kind(value: &Value) -> Option<&str> {
    value.get("kind").and_then(Value::as_str)
}
fn children(value: &Value) -> Result<&Vec<Value>, String> {
    value
        .get("children")
        .and_then(Value::as_array)
        .ok_or("Bar component has no children".into())
}
fn selected(value: &Value) -> Result<bool, String> {
    value
        .get("selected")
        .and_then(Value::as_bool)
        .ok_or("Bar selection is invalid".into())
}
fn action(value: &Value) -> Result<usize, String> {
    value
        .get("action")
        .and_then(Value::as_u64)
        .and_then(|action| usize::try_from(action).ok())
        .ok_or("Bar action is invalid".into())
}
fn text(value: &Value, key: &str) -> Result<String, String> {
    let text = value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Bar component is missing {key}"))?;
    if text.chars().count() > 256 {
        return Err(format!("Bar component {key} is too long"));
    }
    Ok(text.into())
}

fn bar_slider_message(fraction: f32) -> SettingsMessage {
    SettingsMessage::BarJsxSlider((fraction.clamp(0.0, 1.0) * f32::from(u16::MAX)).round() as u16)
}

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
    runtime: JsxRuntime,
    last_data: Option<String>,
    tree: Option<BarTree>,
}

impl BarPage {
    pub(super) fn retained_bytes(&self) -> usize {
        self.last_data.as_ref().map_or(0, String::capacity)
            + self.tree.as_ref().map_or(0, BarTree::retained_bytes)
    }

    pub(super) fn new() -> Result<Self, String> {
        Ok(Self {
            runtime: JsxRuntime::new(
                crate::settings_package::source(crate::settings_package::Script::Bar)?,
                None,
            )?,
            last_data: None,
            tree: None,
        })
    }

    pub(super) fn render(
        &mut self,
        data: &Value,
        theme: SemanticTheme,
        palette: ThemePalette,
    ) -> Result<AnyView<SettingsMessage>, String> {
        let data = serde_json::to_string(data).map_err(|error| error.to_string())?;
        if self.last_data.as_deref() != Some(&data) {
            self.runtime.set_data(&data)?;
            self.tree = Some(self.runtime.render("__nickelRender()", BarTree::parse)?);
            self.last_data = Some(data);
        }
        Ok(self
            .tree
            .as_ref()
            .ok_or("Bar page is unavailable")?
            .view(theme, palette))
    }

    fn slider_action(&self) -> Option<usize> {
        self.tree.as_ref().map(|tree| tree.slider.action)
    }

    #[cfg(test)]
    pub(super) fn action_for_id(&self, id: &str) -> Option<usize> {
        self.tree.as_ref()?.action_for_id(id)
    }

    fn dispatch(
        &mut self,
        index: usize,
        value: Value,
        current_data: &Value,
    ) -> Result<SettingsMessage, String> {
        let data = serde_json::to_string(current_data).map_err(|error| error.to_string())?;
        if self.last_data.as_deref() != Some(&data) {
            return Err(STALE_STATUS.into());
        }
        let rendered = self.runtime.render(
            &format!("__nickelDispatch({index},{value})"),
            BarTree::parse,
        );
        let effects = if rendered.is_ok() {
            self.runtime.take_effects()
        } else {
            Ok(Vec::new())
        };
        let result: Result<(BarTree, SettingsMessage), String> = (|| {
            let tree = rendered?;
            let mut effects = effects?;
            if effects.len() != 1 {
                return Err("Bar action must request one operation".into());
            }
            let request: BarRequest =
                serde_json::from_value(effects.remove(0)).map_err(|error| error.to_string())?;
            Ok((tree, validate_request(&request)?))
        })();
        self.runtime.finish_event(result.is_ok())?;
        let (tree, message) = result?;
        self.tree = Some(tree);
        Ok(message)
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
                if error != STALE_STATUS {
                    *self.bar_page.borrow_mut() = Some(Err(error));
                }
                self.request_redraw();
            }
        }
    }

    pub(super) fn handle_bar_jsx_slider(&mut self, fraction: f32) {
        let action = self
            .bar_page
            .borrow()
            .as_ref()
            .and_then(|page| page.as_ref().ok())
            .and_then(BarPage::slider_action);
        if let Some(action) = action {
            self.handle_bar_jsx_action(action, Value::from(fraction));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jsx_bar_page_renders_and_routes_typed_radio() {
        let settings = ShellSettings::default();
        let localizer = Localizer::system();
        let data = projection(&localizer, &settings, 2, 1);
        let theme = crate::semantic_theme(ThemePalette::from_appearance(
            nickel_core::theme::Appearance::default(),
        ));
        let mut page = BarPage::new().unwrap();
        let _ = page
            .render(
                &data,
                theme,
                ThemePalette::from_appearance(nickel_core::theme::Appearance::default()),
            )
            .unwrap();
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
