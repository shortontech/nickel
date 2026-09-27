//! JSX-owned appearance choices with native previews and color controls.

use nickel_core::{
    shell_settings::ThemePreference,
    theme::{Appearance, ThemeMode, ThemePalette, accent_from_hue},
};
use nickel_plugin_runtime::JsxRuntime;
use nickel_ui::{
    AnyView, Button, ButtonPresentation, ChoiceCard, ChoiceCardGroup, ColorSwatch, Column,
    FrameOverlay, Insets, OverlayAnchor, OverlayStyle, Popover, Row, SemanticTheme, SettingsCard,
    SettingsRow, Size, Surface, SurfaceRole, TextField, UiId, ui,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{SettingsApp, SettingsMessage, SettingsPage};

const STALE_STATUS: &str = "Appearance changed; refresh the page";

#[derive(Clone)]
struct Choice {
    id: String,
    label: String,
    selected: bool,
    action: usize,
}

#[derive(Clone)]
struct ModeTree {
    title: String,
    description: String,
    choices: Vec<Choice>,
}

#[derive(Clone)]
struct Swatch {
    id: String,
    hue: u16,
    selected: bool,
    custom: bool,
    action: usize,
}

#[derive(Clone)]
struct AccentTree {
    title: String,
    description: String,
    swatches: Vec<Swatch>,
}

#[derive(Clone)]
struct AppearanceTree {
    mode: ModeTree,
    accent: AccentTree,
    dialog: DialogTree,
    accent_first: bool,
}

#[derive(Clone)]
struct DialogTree {
    title: String,
    description: String,
    open: bool,
    draft: String,
    input_label: String,
    input_placeholder: String,
    input_action: usize,
    apply_label: String,
    apply_action: usize,
    cancel_label: String,
    cancel_action: usize,
}

impl DialogTree {
    fn parse(value: &Value) -> Result<Self, String> {
        if value.get("kind").and_then(Value::as_str) != Some("settings-hue-dialog")
            || value.get("id").and_then(Value::as_str) != Some("appearance-custom-hue-dialog")
        {
            return Err("Custom hue dialog is invalid".into());
        }
        let children = value
            .get("children")
            .and_then(Value::as_array)
            .ok_or("Custom hue dialog has no controls")?;
        if children.len() != 3
            || children[0].get("kind").and_then(Value::as_str) != Some("settings-input")
            || children[0].get("id").and_then(Value::as_str) != Some("appearance-custom-hue-input")
            || children[1].get("kind").and_then(Value::as_str) != Some("settings-button")
            || children[1].get("id").and_then(Value::as_str) != Some("appearance-custom-hue-apply")
            || children[2].get("kind").and_then(Value::as_str) != Some("settings-button")
            || children[2].get("id").and_then(Value::as_str) != Some("appearance-custom-hue-cancel")
        {
            return Err("Custom hue dialog controls are invalid".into());
        }
        let action = |child: &Value| {
            child
                .get("action")
                .and_then(Value::as_u64)
                .and_then(|index| usize::try_from(index).ok())
                .ok_or("Custom hue dialog action is invalid".to_owned())
        };
        Ok(Self {
            title: text(value, "label")?,
            description: text(value, "value")?,
            open: value.get("open").and_then(Value::as_bool).unwrap_or(false),
            draft: text(&children[0], "value")?,
            input_label: text(&children[0], "label")?,
            input_placeholder: text(&children[0], "placeholder")?,
            input_action: action(&children[0])?,
            apply_label: text(&children[1], "label")?,
            apply_action: action(&children[1])?,
            cancel_label: text(&children[2], "label")?,
            cancel_action: action(&children[2])?,
        })
    }

    fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.title.capacity()
            + self.description.capacity()
            + self.draft.capacity()
            + self.input_label.capacity()
            + self.input_placeholder.capacity()
            + self.apply_label.capacity()
            + self.cancel_label.capacity()
    }

    fn view(&self, theme: SemanticTheme) -> AnyView<SettingsMessage> {
        let action = self.input_action;
        AnyView::new(
            SettingsCard::titled(theme, &self.title, &self.description)
                .child(
                    SettingsRow::new(theme, &self.input_label, "").trailing(
                        TextField::on_change_with_placeholder_mapped(
                            &self.draft,
                            &self.input_placeholder,
                            move |value| SettingsMessage::AppearanceChoicesJsxInput(action, value),
                        )
                        .id("appearance-custom-hue-input"),
                    ),
                )
                .child(
                    Row::new()
                        .gap(8.0)
                        .child(
                            Button::semantic(
                                theme,
                                SettingsMessage::AppearanceChoicesJsxAction(self.apply_action),
                                &self.apply_label,
                                ButtonPresentation::Primary,
                            )
                            .id("appearance-custom-hue-apply"),
                        )
                        .child(
                            Button::semantic(
                                theme,
                                SettingsMessage::AppearanceChoicesJsxAction(self.cancel_action),
                                &self.cancel_label,
                                ButtonPresentation::Secondary,
                            )
                            .id("appearance-custom-hue-cancel"),
                        ),
                ),
        )
    }

    #[cfg(test)]
    fn action_for_id(&self, id: &str) -> Option<usize> {
        match id {
            "appearance-custom-hue-input" => Some(self.input_action),
            "appearance-custom-hue-apply" => Some(self.apply_action),
            "appearance-custom-hue-cancel" => Some(self.cancel_action),
            _ => None,
        }
    }
}

impl AppearanceTree {
    fn parse(value: &Value) -> Result<Self, String> {
        if value.get("kind").and_then(Value::as_str) != Some("settings-appearance-choices") {
            return Err("Appearance choices root is invalid".into());
        }
        let children = value
            .get("children")
            .and_then(Value::as_array)
            .ok_or("Appearance choice cards are missing")?;
        if children.len() != 3 {
            return Err("Appearance needs mode, accent, and dialog components".into());
        }
        let accent_first =
            children[0].get("kind").and_then(Value::as_str) == Some("settings-accent-choices");
        let (mode, accent) = if accent_first {
            (
                ModeTree::parse(&children[1])?,
                AccentTree::parse(&children[0])?,
            )
        } else {
            (
                ModeTree::parse(&children[0])?,
                AccentTree::parse(&children[1])?,
            )
        };
        Ok(Self {
            mode,
            accent,
            dialog: DialogTree::parse(&children[2])?,
            accent_first,
        })
    }

    fn retained_bytes(&self) -> usize {
        self.mode.retained_bytes() + self.accent.retained_bytes() + self.dialog.retained_bytes()
    }

    fn view(&self, theme: SemanticTheme, appearance: Appearance) -> AnyView<SettingsMessage> {
        let (first, second) = if self.accent_first {
            (self.accent.view(theme), self.mode.view(theme, appearance))
        } else {
            (self.mode.view(theme, appearance), self.accent.view(theme))
        };
        AnyView::new(Column::new().gap(10.0).child(first).child(second))
    }

    fn dialog_view(&self, theme: SemanticTheme) -> Option<AnyView<SettingsMessage>> {
        self.dialog.open.then(|| self.dialog.view(theme))
    }

    #[cfg(test)]
    fn action_for_id(&self, id: &str) -> Option<usize> {
        self.mode
            .action_for_id(id)
            .or_else(|| self.dialog.action_for_id(id))
            .or_else(|| {
                self.accent
                    .swatches
                    .iter()
                    .find(|swatch| swatch.id == id)
                    .map(|swatch| swatch.action)
            })
    }
}

impl AccentTree {
    fn parse(value: &Value) -> Result<Self, String> {
        if value.get("kind").and_then(Value::as_str) != Some("settings-accent-choices") {
            return Err("Accent choices card is invalid".into());
        }
        let children = value
            .get("children")
            .and_then(Value::as_array)
            .ok_or("Accent swatches are missing")?;
        if !(2..=17).contains(&children.len()) {
            return Err("Accent swatch count is invalid".into());
        }
        let mut swatches = Vec::with_capacity(children.len());
        for child in children {
            if child.get("kind").and_then(Value::as_str) != Some("settings-swatch") {
                return Err("Accent swatch is invalid".into());
            }
            let hue = child
                .get("hue")
                .and_then(Value::as_u64)
                .and_then(|hue| u16::try_from(hue).ok())
                .filter(|hue| *hue <= 359)
                .ok_or("Accent hue is invalid")?;
            let custom = child
                .get("custom")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let id = text(child, "id")?;
            if id
                != if custom {
                    "appearance-accent-custom".to_owned()
                } else {
                    format!("appearance-accent-{hue}")
                }
            {
                return Err("Accent swatch ID is invalid".into());
            }
            let selected = child
                .get("selected")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let action = child
                .get("action")
                .and_then(Value::as_u64)
                .and_then(|index| usize::try_from(index).ok())
                .ok_or("Accent action is invalid")?;
            if swatches.iter().any(|swatch: &Swatch| swatch.id == id) {
                return Err("Accent swatches must be unique".into());
            }
            swatches.push(Swatch {
                id,
                hue,
                selected,
                custom,
                action,
            });
        }
        if swatches.iter().filter(|swatch| swatch.custom).count() != 1 {
            return Err("Accent card needs one custom swatch".into());
        }
        Ok(Self {
            title: text(value, "label")?,
            description: text(value, "value")?,
            swatches,
        })
    }

    fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.title.capacity()
            + self.description.capacity()
            + self.swatches.capacity() * std::mem::size_of::<Swatch>()
            + self
                .swatches
                .iter()
                .map(|swatch| swatch.id.capacity())
                .sum::<usize>()
    }

    fn view(&self, theme: SemanticTheme) -> AnyView<SettingsMessage> {
        let swatches = self
            .swatches
            .iter()
            .map(|swatch| {
                let message = SettingsMessage::AppearanceChoicesJsxAction(swatch.action);
                if swatch.custom {
                    ColorSwatch::custom(theme, message).id(swatch.id.as_str())
                } else {
                    let [red, green, blue] = accent_from_hue(swatch.hue);
                    ColorSwatch::color(
                        theme,
                        message,
                        (u32::from(red) << 16) | (u32::from(green) << 8) | u32::from(blue),
                        swatch.selected,
                    )
                    .id(swatch.id.as_str())
                }
            })
            .collect::<Vec<_>>();
        AnyView::new(
            SettingsCard::titled(theme, &self.title, &self.description)
                .id("appearance-accent-card")
                .child(Row::new().height(44.0).gap(10.0).children(swatches)),
        )
    }
}

impl ModeTree {
    fn parse(value: &Value) -> Result<Self, String> {
        if value.get("kind").and_then(Value::as_str) != Some("settings-appearance-modes") {
            return Err("Appearance mode root is invalid".into());
        }
        let children = value
            .get("children")
            .and_then(Value::as_array)
            .ok_or("Appearance modes have no choices")?;
        if children.len() != 3 {
            return Err("Appearance modes need three choices".into());
        }
        let expected = [
            "appearance-mode-light",
            "appearance-mode-dark",
            "appearance-mode-system",
        ];
        let choices: Vec<Choice> = children
            .iter()
            .map(|child| {
                let id = child
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or("Appearance mode choice has no ID")?;
                if child.get("kind").and_then(Value::as_str) != Some("settings-choice")
                    || !expected.contains(&id)
                {
                    return Err("Appearance mode choice is invalid".into());
                }
                Ok(Choice {
                    id: id.into(),
                    label: text(child, "label")?,
                    selected: child
                        .get("selected")
                        .and_then(Value::as_bool)
                        .ok_or("Appearance mode selection is invalid")?,
                    action: child
                        .get("action")
                        .and_then(Value::as_u64)
                        .and_then(|index| usize::try_from(index).ok())
                        .ok_or("Appearance mode action is invalid")?,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        if expected
            .iter()
            .any(|id| choices.iter().filter(|choice| choice.id == *id).count() != 1)
        {
            return Err("Appearance mode choices must be unique".into());
        }
        if choices
            .iter()
            .filter(|choice: &&Choice| choice.selected)
            .count()
            != 1
        {
            return Err("Appearance modes need one selection".into());
        }
        Ok(Self {
            title: text(value, "label")?,
            description: text(value, "value")?,
            choices,
        })
    }

    fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.title.capacity()
            + self.description.capacity()
            + self.choices.capacity() * std::mem::size_of::<Choice>()
            + self
                .choices
                .iter()
                .map(|choice| choice.id.capacity() + choice.label.capacity())
                .sum::<usize>()
    }

    fn view(&self, theme: SemanticTheme, appearance: Appearance) -> AnyView<SettingsMessage> {
        let light = ThemePalette::from_appearance(Appearance {
            mode: ThemeMode::Light,
            ..appearance
        });
        let dark = ThemePalette::from_appearance(Appearance {
            mode: ThemeMode::Dark,
            ..appearance
        });
        let preview =
            |palette: ThemePalette| {
                Surface::new(theme, SurfaceRole::Raised)
            .height(82.0).radius(theme.radii.control).padding(Insets::all(8.0))
            .child(ui! { <Row gap={6.0}>
                <Container width={22.0} background={palette.panel} radius={3.0} />
                <Column grow={1.0} gap={6.0}>
                    <Container height={12.0} background={palette.surface_hover} radius={3.0} />
                    <Container height={28.0} background={palette.background} radius={3.0} />
                </Column>
            </Row> })
            };
        let choices =
            self.choices
                .iter()
                .map(|choice| {
                    let picture = match choice.id.as_str() {
                "appearance-mode-light" => AnyView::new(preview(light)),
                "appearance-mode-dark" => AnyView::new(preview(dark)),
                _ => AnyView::new(Surface::new(theme, SurfaceRole::Raised)
                    .height(82.0).radius(theme.radii.control).padding(Insets::all(8.0))
                    .child(ui! { <Row height={66.0} gap={3.0}>
                        <Container grow={1.0} background={light.background} radius={3.0} />
                        <Container grow={1.0} background={dark.background} radius={3.0} />
                    </Row> })),
            };
                    ChoiceCard::new(
                        theme,
                        SettingsMessage::AppearanceChoicesJsxAction(choice.action),
                        &choice.label,
                        choice.selected,
                        picture,
                    )
                    .id(choice.id.as_str())
                })
                .collect::<Vec<_>>();
        AnyView::new(
            SettingsCard::titled(theme, &self.title, &self.description)
                .id("appearance-mode-card")
                .child(ChoiceCardGroup::new(choices)),
        )
    }

    #[cfg(test)]
    fn action_for_id(&self, id: &str) -> Option<usize> {
        self.choices
            .iter()
            .find(|choice| choice.id == id)
            .map(|choice| choice.action)
    }
}

fn text(value: &Value, key: &str) -> Result<String, String> {
    let text = value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Appearance mode {key} is missing"))?;
    if text.chars().count() > 256 {
        return Err(format!("Appearance mode {key} is too long"));
    }
    Ok(text.into())
}

pub(super) struct AppearanceChoicesPage {
    runtime: JsxRuntime,
    last_data: Option<String>,
    tree: Option<AppearanceTree>,
}

impl AppearanceChoicesPage {
    pub(super) fn new() -> Result<Self, String> {
        Ok(Self {
            runtime: JsxRuntime::new(
                crate::settings_package::source(
                    crate::settings_package::Script::AppearanceChoices,
                )?,
                None,
            )?,
            last_data: None,
            tree: None,
        })
    }

    pub(super) fn retained_bytes(&self) -> usize {
        self.last_data.as_ref().map_or(0, String::capacity)
            + self.tree.as_ref().map_or(0, AppearanceTree::retained_bytes)
    }

    pub(super) fn render(
        &mut self,
        data: &Value,
        theme: SemanticTheme,
        appearance: Appearance,
    ) -> Result<AnyView<SettingsMessage>, String> {
        let serialized = serde_json::to_string(data).map_err(|error| error.to_string())?;
        if self.last_data.as_deref() != Some(&serialized) {
            self.runtime.set_data(&serialized)?;
            self.tree = Some(
                self.runtime
                    .render("__nickelRender()", AppearanceTree::parse)?,
            );
            self.last_data = Some(serialized);
        }
        Ok(self
            .tree
            .as_ref()
            .ok_or("Appearance choices are unavailable")?
            .view(theme, appearance))
    }

    pub(super) fn dialog_view(&self, theme: SemanticTheme) -> Option<AnyView<SettingsMessage>> {
        self.tree.as_ref()?.dialog_view(theme)
    }

    fn dispatch(
        &mut self,
        index: usize,
        value: Value,
        current_data: &Value,
    ) -> Result<SettingsMessage, String> {
        let serialized = serde_json::to_string(current_data).map_err(|error| error.to_string())?;
        if self.last_data.as_deref() != Some(&serialized) {
            return Err(STALE_STATUS.into());
        }
        let rendered = self.runtime.render(
            &format!("__nickelDispatch({index},{value})"),
            AppearanceTree::parse,
        );
        let effects = if rendered.is_ok() {
            self.runtime.take_effects()
        } else {
            Ok(Vec::new())
        };
        let result: Result<(AppearanceTree, SettingsMessage), String> = (|| {
            let tree = rendered?;
            let effects = effects?;
            if effects.len() != 1 {
                return Err("Appearance action needs one request".into());
            }
            let request: AppearanceRequest =
                serde_json::from_value(effects[0].clone()).map_err(|error| error.to_string())?;
            Ok((tree, validate_request(request, current_data)?))
        })();
        self.runtime.finish_event(result.is_ok())?;
        let (tree, message) = result?;
        self.tree = Some(tree);
        Ok(message)
    }

    #[cfg(test)]
    pub(super) fn action_for_id(&self, id: &str) -> Option<usize> {
        self.tree.as_ref()?.action_for_id(id)
    }
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
enum AppearanceRequest {
    Mode { value: String },
    AccentHue { hue: u16 },
    OpenCustomHue,
    CustomHueDraft { value: String },
    ApplyCustomHue { value: String },
    CancelCustomHue,
}

fn validate_request(request: AppearanceRequest, data: &Value) -> Result<SettingsMessage, String> {
    if data.get("selected").and_then(Value::as_str).is_none() {
        return Err("Appearance preference is unavailable".into());
    }
    match request {
        AppearanceRequest::Mode { value } => match value.as_str() {
            "light" => Ok(SettingsMessage::AppearanceLight),
            "dark" => Ok(SettingsMessage::AppearanceDark),
            "system" => Ok(SettingsMessage::AppearanceSystem),
            _ => Err("Appearance mode is invalid".into()),
        },
        AppearanceRequest::AccentHue { hue } => {
            if hue > 359
                || (data.get("hue").and_then(Value::as_u64) != Some(u64::from(hue))
                    && !data
                        .get("swatches")
                        .and_then(Value::as_array)
                        .is_some_and(|swatches| swatches.iter().any(|swatch| swatch["hue"] == hue)))
            {
                return Err("Accent hue is unavailable".into());
            }
            Ok(SettingsMessage::SetAccentHue(hue))
        }
        AppearanceRequest::OpenCustomHue => {
            if data.get("customHueOpen").and_then(Value::as_bool) != Some(false) {
                return Err("Custom hue dialog is already open".into());
            }
            Ok(SettingsMessage::OpenCustomHue)
        }
        AppearanceRequest::CustomHueDraft { value } => {
            if data.get("customHueOpen").and_then(Value::as_bool) != Some(true)
                || value.chars().count() > 16
            {
                return Err("Custom hue input is unavailable".into());
            }
            Ok(SettingsMessage::CustomHueDraftChanged(value))
        }
        AppearanceRequest::ApplyCustomHue { value } => {
            if data.get("customHueOpen").and_then(Value::as_bool) != Some(true)
                || data.get("customHueDraft").and_then(Value::as_str) != Some(value.as_str())
            {
                return Err("Custom hue draft changed".into());
            }
            Ok(SettingsMessage::ApplyCustomHue(value))
        }
        AppearanceRequest::CancelCustomHue => {
            if data.get("customHueOpen").and_then(Value::as_bool) != Some(true) {
                return Err("Custom hue dialog is unavailable".into());
            }
            Ok(SettingsMessage::CancelCustomHue)
        }
    }
}

pub(super) fn projection(app: &SettingsApp) -> Value {
    let hue = app
        .shell_settings
        .displayed_hue(nickel_platform::appearance());
    let swatches = [224_u16, 188, 154, 78, 38, 16, 340, 305]
        .into_iter()
        .map(|preset| json!({"hue":preset,"selected":hue.abs_diff(preset) < 3}))
        .collect::<Vec<_>>();
    json!({
        "title":app.localizer.text("settings-appearance-mode"),
        "description":app.localizer.text("settings-appearance-mode-description"),
        "light":app.localizer.text("settings-appearance-light"),
        "dark":app.localizer.text("settings-appearance-dark"),
        "automatic":app.localizer.text("settings-appearance-automatic"),
        "selected":match app.shell_settings.theme {
            ThemePreference::Light => "light", ThemePreference::Dark => "dark",
            ThemePreference::System => "system",
        },
        "accentTitle":app.localizer.text("settings-appearance-accent"),
        "accentDescription":app.localizer.text("settings-appearance-accent-description"),
        "hue":hue,
        "swatches":swatches,
        "customHueOpen":app.custom_hue_open,
        "customHueDraft":app.custom_hue_draft,
        "customHueTitle":app.localizer.text("settings-appearance-custom-hue-title"),
        "customHueDescription":app.custom_hue_error.clone().unwrap_or_else(|| app.localizer.text("settings-appearance-custom-hue-description")),
        "customHueApply":app.localizer.text("settings-appearance-custom-hue-apply"),
        "customHueCancel":app.localizer.text("settings-appearance-custom-hue-cancel"),
        "customHueField":app.localizer.text("settings-appearance-custom-hue-field"),
        "customHuePlaceholder":app.localizer.text("settings-appearance-custom-hue-placeholder"),
    })
}

impl SettingsApp {
    pub(super) fn handle_appearance_choices_jsx_action(&mut self, index: usize, value: Value) {
        if self.page != SettingsPage::Appearance {
            return;
        }
        let data = projection(self);
        let result = self
            .appearance_choices_page
            .borrow_mut()
            .as_mut()
            .ok_or_else(|| "Appearance choices are not loaded".to_owned())
            .and_then(|page| page.as_mut().map_err(|error| error.clone()))
            .and_then(|page| page.dispatch(index, value, &data));
        match result {
            Ok(message) => self.handle_settings_message(message),
            Err(error) => {
                if error != STALE_STATUS {
                    *self.appearance_choices_page.borrow_mut() = Some(Err(error));
                }
                self.request_redraw();
            }
        }
    }

    pub(super) fn appearance_hue_overlay(&self) -> Vec<FrameOverlay<SettingsMessage>> {
        if !self.custom_hue_open {
            return Vec::new();
        }
        let theme = self.ui_theme();
        let content = self
            .appearance_choices_page
            .borrow()
            .as_ref()
            .and_then(|page| page.as_ref().ok())
            .and_then(|page| page.dialog_view(theme))
            .unwrap_or_else(|| {
                AnyView::new(
                    SettingsCard::titled(
                        theme,
                        self.localizer.text("settings-appearance-custom-hue-title"),
                        self.custom_hue_error.clone().unwrap_or_else(|| {
                            self.localizer
                                .text("settings-appearance-custom-hue-description")
                        }),
                    )
                    .child(
                        SettingsRow::new(
                            theme,
                            self.localizer.text("settings-appearance-custom-hue-field"),
                            "",
                        )
                        .trailing(
                            TextField::on_change_with_placeholder_mapped(
                                &self.custom_hue_draft,
                                &self
                                    .localizer
                                    .text("settings-appearance-custom-hue-placeholder"),
                                SettingsMessage::CustomHueDraftChanged,
                            )
                            .id("appearance-custom-hue-input"),
                        ),
                    )
                    .child(
                        Row::new()
                            .gap(8.0)
                            .child(
                                Button::semantic(
                                    theme,
                                    SettingsMessage::ApplyCustomHue(self.custom_hue_draft.clone()),
                                    self.localizer.text("settings-appearance-custom-hue-apply"),
                                    ButtonPresentation::Primary,
                                )
                                .id("appearance-custom-hue-apply"),
                            )
                            .child(
                                Button::semantic(
                                    theme,
                                    SettingsMessage::CancelCustomHue,
                                    self.localizer.text("settings-appearance-custom-hue-cancel"),
                                    ButtonPresentation::Secondary,
                                )
                                .id("appearance-custom-hue-cancel"),
                            ),
                    ),
                )
            });
        vec![
            Popover::new(
                "appearance-custom-hue-dialog",
                OverlayAnchor::Node(UiId::from("appearance-accent-custom")),
                self.localizer.text("settings-appearance-custom-hue-title"),
                Size::new(360.0, 216.0),
                OverlayStyle::from_theme(&theme),
                content,
            )
            .focus(nickel_ui::OverlayFocusPolicy::FirstItem)
            .dismiss(nickel_ui::DismissPolicy {
                cancel: true,
                outside_pointer: true,
                action: false,
            })
            .focus_return("appearance-accent-custom")
            .into(),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_tree_accepts_plugin_order_but_rejects_missing_choices() {
        let choice = |id: &str, selected: bool| {
            json!({
                "kind":"settings-choice","id":id,"label":id,
                "selected":selected,"action":0,
            })
        };
        let root = json!({
            "kind":"settings-appearance-modes","label":"Mode","value":"",
            "children":[
                choice("appearance-mode-system", true),
                choice("appearance-mode-light", false),
                choice("appearance-mode-dark", false),
            ],
        });
        let tree = ModeTree::parse(&root).unwrap();
        assert_eq!(tree.choices[0].id, "appearance-mode-system");
        let mut missing = root;
        missing["children"][2]["id"] = Value::from("appearance-mode-light");
        assert!(ModeTree::parse(&missing).is_err());
    }

    #[test]
    fn appearance_cards_follow_plugin_order() {
        let modes = json!({"kind":"settings-appearance-modes","label":"Mode","value":"",
        "children":[
            {"kind":"settings-choice","id":"appearance-mode-light","label":"Light","selected":true,"action":0},
            {"kind":"settings-choice","id":"appearance-mode-dark","label":"Dark","selected":false,"action":1},
            {"kind":"settings-choice","id":"appearance-mode-system","label":"Auto","selected":false,"action":2},
        ]});
        let accents = json!({"kind":"settings-accent-choices","label":"Accent","value":"",
        "children":[
            {"kind":"settings-swatch","id":"appearance-accent-224","hue":224,"selected":true,"action":3},
            {"kind":"settings-swatch","id":"appearance-accent-custom","hue":224,"custom":true,"action":4},
        ]});
        let dialog = json!({"kind":"settings-hue-dialog","id":"appearance-custom-hue-dialog",
        "label":"Custom","value":"","open":false,"children":[
                {"kind":"settings-input","id":"appearance-custom-hue-input","label":"Hue","placeholder":"0–359","value":"224","action":5},
            {"kind":"settings-button","id":"appearance-custom-hue-apply","label":"Apply","action":6},
            {"kind":"settings-button","id":"appearance-custom-hue-cancel","label":"Cancel","action":7},
        ]});
        let root =
            json!({"kind":"settings-appearance-choices","children":[accents, modes, dialog]});
        let tree = AppearanceTree::parse(&root).unwrap();
        assert!(tree.accent_first);
        assert_eq!(tree.action_for_id("appearance-accent-224"), Some(3));
    }

    #[test]
    fn jsx_mode_choice_requests_typed_preference_and_checks_current_projection() {
        let mut app = SettingsApp::with_initial_page(SettingsPage::Appearance);
        app.shell_settings.theme = ThemePreference::Light;
        let data = projection(&app);
        let mut page = AppearanceChoicesPage::new().unwrap();
        page.render(&data, app.ui_theme(), nickel_platform::appearance())
            .unwrap();
        let action = page.action_for_id("appearance-mode-dark").unwrap();
        assert_eq!(
            page.dispatch(action, Value::Null, &data).unwrap(),
            SettingsMessage::AppearanceDark
        );
        app.shell_settings.theme = ThemePreference::Dark;
        assert_eq!(
            page.dispatch(action, Value::Null, &projection(&app))
                .unwrap_err(),
            STALE_STATUS
        );
    }

    #[test]
    fn jsx_accent_choice_uses_projected_hue_and_rejects_stale_actions() {
        let mut app = SettingsApp::with_initial_page(SettingsPage::Appearance);
        app.shell_settings.accent_hue = Some(224);
        let data = projection(&app);
        let mut page = AppearanceChoicesPage::new().unwrap();
        page.render(&data, app.ui_theme(), nickel_platform::appearance())
            .unwrap();
        let action = page.action_for_id("appearance-accent-188").unwrap();
        assert_eq!(
            page.dispatch(action, Value::Null, &data).unwrap(),
            SettingsMessage::SetAccentHue(188)
        );
        assert!(validate_request(AppearanceRequest::AccentHue { hue: 101 }, &data).is_err());
        app.shell_settings.accent_hue = Some(305);
        assert_eq!(
            page.dispatch(action, Value::Null, &projection(&app))
                .unwrap_err(),
            STALE_STATUS
        );
    }
}
