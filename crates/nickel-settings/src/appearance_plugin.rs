//! JSX-owned appearance controls with native previews and widgets.

use nickel_core::{
    shell_settings::{AnimationLevel, FileIconPreference, ThemePreference},
    theme::{Appearance, ThemeMode, ThemePalette, accent_from_hue},
    wallpaper_settings::WallpaperPosition,
};
use nickel_plugin_runtime::JsxRuntime;
use nickel_ui::{
    Align, AnyView, Button, ButtonPresentation, ChoiceCard, ChoiceCardGroup, ColorSwatch, Column,
    FrameOverlay, Image, ImageFit, Insets, Justify, OverlayAnchor, OverlayStyle, Popover,
    PreviewTile, Row, SelectField, SemanticTheme, SettingsCard, SettingsRow, SettingsStatus,
    SettingsStatusKind, Size, SliderField, Surface, SurfaceRole, Switch, Text, TextField, UiId, ui,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

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
    wallpaper: WallpaperTree,
    interface: InterfaceTree,
    reset: ResetTree,
    order: Vec<AppearanceSection>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AppearanceSection {
    Mode,
    Accent,
    Wallpaper,
    Interface,
    Reset,
}

#[derive(Clone)]
struct WallpaperTree {
    title: String,
    description: String,
    name: String,
    dimensions: String,
    unavailable: String,
    status: String,
    choose_label: String,
    choose_action: usize,
    remove_label: String,
    remove_action: usize,
    position: SelectTree,
}

#[derive(Clone)]
struct ResetTree {
    label: String,
    action: usize,
}

#[derive(Clone)]
struct InterfaceTree {
    title: String,
    controls: Vec<InterfaceControl>,
}

#[derive(Clone)]
enum InterfaceControl {
    Slider(SliderTree),
    Transparency(TransparencyTree),
    Select(SelectTree),
}

#[derive(Clone)]
struct SliderTree {
    id: String,
    label: String,
    description: String,
    value_label: String,
    percent: f32,
    action: usize,
}

#[derive(Clone)]
struct SelectTree {
    id: String,
    label: String,
    description: String,
    value_label: String,
    expanded: bool,
    toggle_action: usize,
    options: Vec<SelectOption>,
}

#[derive(Clone)]
struct SelectOption {
    id: String,
    label: String,
    selected: bool,
    action: usize,
}

impl WallpaperTree {
    fn parse(value: &Value) -> Result<Self, String> {
        if value.get("kind").and_then(Value::as_str) != Some("settings-wallpaper")
            || value.get("id").and_then(Value::as_str) != Some("appearance-wallpaper-card")
        {
            return Err("Appearance wallpaper card is invalid".into());
        }
        let children = value
            .get("children")
            .and_then(Value::as_array)
            .ok_or("Wallpaper controls are missing")?;
        if children.len() != 4
            || children[0].get("kind").and_then(Value::as_str) != Some("settings-wallpaper-preview")
            || children[0].get("id").and_then(Value::as_str) != Some("appearance-wallpaper-preview")
            || children[1].get("kind").and_then(Value::as_str) != Some("settings-button")
            || children[1].get("id").and_then(Value::as_str) != Some("appearance-wallpaper-choose")
            || children[2].get("kind").and_then(Value::as_str) != Some("settings-button")
            || children[2].get("id").and_then(Value::as_str) != Some("appearance-wallpaper-remove")
        {
            return Err("Wallpaper controls are invalid".into());
        }
        let position = SelectTree::parse(&children[3])?;
        if position.id != "appearance-wallpaper-position"
            || position.options.len() != 6
            || position
                .options
                .iter()
                .filter(|option| option.selected)
                .count()
                != 1
            || ["fill", "fit", "stretch", "center", "tile", "span"]
                .iter()
                .any(|id| {
                    position
                        .options
                        .iter()
                        .filter(|option| option.id == format!("appearance-wallpaper-position-{id}"))
                        .count()
                        != 1
                })
        {
            return Err("Wallpaper position choices are invalid".into());
        }
        Ok(Self {
            title: text(value, "label")?,
            description: text(value, "value")?,
            name: text(&children[0], "label")?,
            dimensions: text(&children[0], "value")?,
            unavailable: text(&children[0], "placeholder")?,
            status: text(&children[0], "state")?,
            choose_label: text(&children[1], "label")?,
            choose_action: action(&children[1])?,
            remove_label: text(&children[2], "label")?,
            remove_action: action(&children[2])?,
            position,
        })
    }

    fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.title.capacity()
            + self.description.capacity()
            + self.name.capacity()
            + self.dimensions.capacity()
            + self.unavailable.capacity()
            + self.status.capacity()
            + self.choose_label.capacity()
            + self.remove_label.capacity()
            + self.position.id.capacity()
            + self.position.label.capacity()
            + self.position.description.capacity()
            + self.position.value_label.capacity()
            + self.position.options.capacity() * std::mem::size_of::<SelectOption>()
            + self
                .position
                .options
                .iter()
                .map(|option| option.id.capacity() + option.label.capacity())
                .sum::<usize>()
    }

    fn view(
        &self,
        theme: SemanticTheme,
        preview: Option<&Arc<image::RgbaImage>>,
    ) -> AnyView<SettingsMessage> {
        let image = preview
            .map(|image| {
                PreviewTile::new(
                    theme,
                    Image::new(1, image.clone())
                        .fit(ImageFit::Cover)
                        .width(124.0)
                        .height(96.0),
                )
            })
            .unwrap_or_else(|| PreviewTile::unavailable(theme, &self.unavailable))
            .width(124.0)
            .height(96.0);
        let mut details = Column::new()
            .grow(1.0)
            .gap(8.0)
            .child(Text::new(&self.name).color(theme.text.primary));
        if !self.dimensions.is_empty() {
            details = details.child(Text::new(&self.dimensions).color(theme.text.secondary));
        }
        details = details.child(
            Row::new()
                .gap(10.0)
                .child(
                    Button::semantic(
                        theme,
                        SettingsMessage::AppearanceJsxAction(self.choose_action),
                        &self.choose_label,
                        ButtonPresentation::Primary,
                    )
                    .id("appearance-wallpaper-choose")
                    .width(168.0),
                )
                .child(
                    Button::semantic(
                        theme,
                        SettingsMessage::AppearanceJsxAction(self.remove_action),
                        &self.remove_label,
                        ButtonPresentation::Secondary,
                    )
                    .id("appearance-wallpaper-remove")
                    .width(100.0),
                ),
        );
        if !self.status.is_empty() {
            details = details.child(SettingsStatus::new(
                theme,
                SettingsStatusKind::Error,
                &self.status,
            ));
        }
        let options = self.position.options.iter().map(|option| {
            (
                option.label.as_str(),
                SettingsMessage::AppearanceJsxAction(option.action),
            )
        });
        AnyView::new(
            SettingsCard::titled(theme, &self.title, &self.description)
                .id("appearance-wallpaper-card")
                .child(
                    Row::new()
                        .gap(14.0)
                        .align_items(Align::Center)
                        .child(image)
                        .child(details),
                )
                .child(
                    SelectField::new(
                        theme,
                        &self.position.label,
                        &self.position.description,
                        SettingsMessage::AppearanceJsxAction(self.position.toggle_action),
                        &self.position.value_label,
                        options,
                        self.position.expanded,
                    )
                    .id("appearance-wallpaper-position"),
                ),
        )
    }

    #[cfg(test)]
    fn action_for_id(&self, id: &str) -> Option<usize> {
        match id {
            "appearance-wallpaper-choose" => Some(self.choose_action),
            "appearance-wallpaper-remove" => Some(self.remove_action),
            _ => self.position.action_for_id(id),
        }
    }
}

impl ResetTree {
    fn parse(value: &Value) -> Result<Self, String> {
        if value.get("kind").and_then(Value::as_str) != Some("settings-reset")
            || value.get("id").and_then(Value::as_str) != Some("appearance-reset")
        {
            return Err("Appearance reset control is invalid".into());
        }
        Ok(Self {
            label: text(value, "label")?,
            action: action(value)?,
        })
    }

    fn view(&self, theme: SemanticTheme) -> AnyView<SettingsMessage> {
        AnyView::new(
            Row::new().justify_content(Justify::End).child(
                Button::semantic(
                    theme,
                    SettingsMessage::AppearanceJsxAction(self.action),
                    &self.label,
                    ButtonPresentation::Secondary,
                )
                .id("appearance-reset")
                .width(220.0),
            ),
        )
    }
}

impl InterfaceTree {
    fn parse(value: &Value) -> Result<Self, String> {
        if value.get("kind").and_then(Value::as_str) != Some("settings-interface")
            || value.get("id").and_then(Value::as_str) != Some("appearance-interface-card")
        {
            return Err("Appearance interface card is invalid".into());
        }
        let children = value
            .get("children")
            .and_then(Value::as_array)
            .ok_or("Appearance interface controls are missing")?;
        if children.len() != 5 {
            return Err("Appearance interface needs five controls".into());
        }
        let controls = children
            .iter()
            .map(InterfaceControl::parse)
            .collect::<Result<Vec<_>, _>>()?;
        let ids = [
            "appearance-hue",
            "appearance-intensity",
            "appearance-transparency",
            "appearance-animations",
            "appearance-file-artwork",
        ];
        if ids.iter().any(|id| {
            controls
                .iter()
                .filter(|control| control.id() == *id)
                .count()
                != 1
        }) {
            return Err("Appearance interface controls are incomplete".into());
        }
        Ok(Self {
            title: text(value, "label")?,
            controls,
        })
    }

    fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.title.capacity()
            + self.controls.capacity() * std::mem::size_of::<InterfaceControl>()
            + self
                .controls
                .iter()
                .map(InterfaceControl::heap_bytes)
                .sum::<usize>()
    }

    fn view(&self, theme: SemanticTheme) -> AnyView<SettingsMessage> {
        AnyView::new(
            SettingsCard::titled(theme, &self.title, "")
                .id("appearance-interface-card")
                .children(self.controls.iter().map(|control| control.view(theme))),
        )
    }

    fn action_for_id(&self, id: &str) -> Option<usize> {
        self.controls
            .iter()
            .find_map(|control| control.action_for_id(id))
    }
}

impl InterfaceControl {
    fn parse(value: &Value) -> Result<Self, String> {
        match value.get("kind").and_then(Value::as_str) {
            Some("settings-slider") => Ok(Self::Slider(SliderTree::parse(value)?)),
            Some("settings-transparency") => {
                Ok(Self::Transparency(TransparencyTree::parse(value)?))
            }
            Some("settings-select") => Ok(Self::Select(SelectTree::parse(value)?)),
            _ => Err("Appearance interface control is invalid".into()),
        }
    }

    fn id(&self) -> &str {
        match self {
            Self::Slider(control) => &control.id,
            Self::Transparency(_) => "appearance-transparency",
            Self::Select(control) => &control.id,
        }
    }

    fn heap_bytes(&self) -> usize {
        match self {
            Self::Slider(control) => {
                control.id.capacity()
                    + control.label.capacity()
                    + control.description.capacity()
                    + control.value_label.capacity()
            }
            Self::Transparency(control) => {
                control.label.capacity() + control.description.capacity()
            }
            Self::Select(control) => {
                control.id.capacity()
                    + control.label.capacity()
                    + control.description.capacity()
                    + control.value_label.capacity()
                    + control.options.capacity() * std::mem::size_of::<SelectOption>()
                    + control
                        .options
                        .iter()
                        .map(|option| option.id.capacity() + option.label.capacity())
                        .sum::<usize>()
            }
        }
    }

    fn view(&self, theme: SemanticTheme) -> AnyView<SettingsMessage> {
        match self {
            Self::Slider(control) => AnyView::new(
                SliderField::new(
                    theme,
                    &control.label,
                    &control.description,
                    &control.value_label,
                    control.percent,
                    if control.id == "appearance-hue" {
                        appearance_jsx_hue_message
                    } else {
                        appearance_jsx_intensity_message
                    },
                )
                .id(control.id.as_str()),
            ),
            Self::Transparency(control) => control.view(theme),
            Self::Select(control) => AnyView::new(
                SelectField::new(
                    theme,
                    &control.label,
                    &control.description,
                    SettingsMessage::AppearanceJsxAction(control.toggle_action),
                    &control.value_label,
                    control.options.iter().map(|option| {
                        (
                            option.label.as_str(),
                            SettingsMessage::AppearanceJsxAction(option.action),
                        )
                    }),
                    control.expanded,
                )
                .id(control.id.as_str()),
            ),
        }
    }

    fn action_for_id(&self, id: &str) -> Option<usize> {
        match self {
            Self::Slider(control) => (id == control.id).then_some(control.action),
            Self::Transparency(control) => {
                (id == "appearance-transparency").then_some(control.action)
            }
            Self::Select(control) => control.action_for_id(id),
        }
    }
}

impl SliderTree {
    fn parse(value: &Value) -> Result<Self, String> {
        let id = text(value, "id")?;
        if id != "appearance-hue" && id != "appearance-intensity" {
            return Err("Appearance slider ID is invalid".into());
        }
        let percent = value
            .get("percent")
            .and_then(Value::as_f64)
            .ok_or("Appearance slider value is invalid")?;
        if !(0.0..=1.0).contains(&percent) {
            return Err("Appearance slider value is out of range".into());
        }
        Ok(Self {
            id,
            label: text(value, "label")?,
            description: text(value, "placeholder")?,
            value_label: text(value, "value")?,
            percent: percent as f32,
            action: action(value)?,
        })
    }
}

impl SelectTree {
    fn parse(value: &Value) -> Result<Self, String> {
        let id = text(value, "id")?;
        if id != "appearance-animations"
            && id != "appearance-file-artwork"
            && id != "appearance-wallpaper-position"
        {
            return Err("Appearance select ID is invalid".into());
        }
        let children = value
            .get("children")
            .and_then(Value::as_array)
            .ok_or("Appearance select options are missing")?;
        if !(2..=66).contains(&children.len()) {
            return Err("Appearance select option count is invalid".into());
        }
        let options = children
            .iter()
            .map(|child| {
                if child.get("kind").and_then(Value::as_str) != Some("settings-option") {
                    return Err("Appearance select option is invalid".into());
                }
                Ok(SelectOption {
                    id: text(child, "id")?,
                    label: text(child, "label")?,
                    selected: child
                        .get("selected")
                        .and_then(Value::as_bool)
                        .ok_or("Appearance select option state is invalid")?,
                    action: action(child)?,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        if options.iter().filter(|option| option.selected).count() > 1
            || options
                .iter()
                .map(|option| &option.id)
                .collect::<std::collections::HashSet<_>>()
                .len()
                != options.len()
        {
            return Err("Appearance select options are inconsistent".into());
        }
        Ok(Self {
            id,
            label: text(value, "label")?,
            description: text(value, "placeholder")?,
            value_label: text(value, "value")?,
            expanded: value
                .get("open")
                .and_then(Value::as_bool)
                .ok_or("Appearance select expansion is invalid")?,
            toggle_action: action(value)?,
            options,
        })
    }

    fn action_for_id(&self, id: &str) -> Option<usize> {
        (id == self.id).then_some(self.toggle_action).or_else(|| {
            self.options
                .iter()
                .find(|option| option.id == id)
                .map(|option| option.action)
        })
    }
}

fn appearance_jsx_hue_message(fraction: f32) -> SettingsMessage {
    SettingsMessage::AppearanceJsxHue(
        (fraction.clamp(0.0, 1.0) * f32::from(u16::MAX)).round() as u16
    )
}

fn appearance_jsx_intensity_message(fraction: f32) -> SettingsMessage {
    SettingsMessage::AppearanceJsxIntensity(
        (fraction.clamp(0.0, 1.0) * f32::from(u16::MAX)).round() as u16,
    )
}

fn action(value: &Value) -> Result<usize, String> {
    value
        .get("action")
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or("Appearance action is invalid".into())
}

#[derive(Clone)]
struct TransparencyTree {
    label: String,
    description: String,
    selected: bool,
    action: usize,
}

impl TransparencyTree {
    fn parse(value: &Value) -> Result<Self, String> {
        if value.get("kind").and_then(Value::as_str) != Some("settings-transparency")
            || value.get("id").and_then(Value::as_str) != Some("appearance-transparency")
        {
            return Err("Appearance transparency control is invalid".into());
        }
        Ok(Self {
            label: text(value, "label")?,
            description: text(value, "value")?,
            selected: value
                .get("selected")
                .and_then(Value::as_bool)
                .ok_or("Appearance transparency value is invalid")?,
            action: value
                .get("action")
                .and_then(Value::as_u64)
                .and_then(|value| usize::try_from(value).ok())
                .ok_or("Appearance transparency action is invalid")?,
        })
    }

    fn view(&self, theme: SemanticTheme) -> AnyView<SettingsMessage> {
        AnyView::new(
            SettingsRow::new(theme, &self.label, &self.description).trailing(
                Switch::with_state_action(
                    if self.selected {
                        nickel_ui::SwitchState::On
                    } else {
                        nickel_ui::SwitchState::Off
                    },
                    Some(SettingsMessage::AppearanceJsxAction(self.action)),
                    theme,
                )
                .id("appearance-transparency")
                .accessibility_label(&self.label),
            ),
        )
    }
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
                            move |value| SettingsMessage::AppearanceJsxInput(action, value),
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
                                SettingsMessage::AppearanceJsxAction(self.apply_action),
                                &self.apply_label,
                                ButtonPresentation::Primary,
                            )
                            .id("appearance-custom-hue-apply"),
                        )
                        .child(
                            Button::semantic(
                                theme,
                                SettingsMessage::AppearanceJsxAction(self.cancel_action),
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
        if value.get("kind").and_then(Value::as_str) != Some("settings-appearance-page") {
            return Err("Appearance choices root is invalid".into());
        }
        let children = value
            .get("children")
            .and_then(Value::as_array)
            .ok_or("Appearance choice cards are missing")?;
        if children.len() != 6 {
            return Err("Appearance needs six page components".into());
        }
        let find = |kind: &str| -> Result<&Value, String> {
            let mut matches = children
                .iter()
                .filter(|child| child.get("kind").and_then(Value::as_str) == Some(kind));
            let found = matches
                .next()
                .ok_or_else(|| format!("Appearance {kind} is missing"))?;
            if matches.next().is_some() {
                return Err(format!("Appearance {kind} is duplicated"));
            }
            Ok(found)
        };
        let order = children
            .iter()
            .filter_map(|child| match child.get("kind").and_then(Value::as_str) {
                Some("settings-appearance-modes") => Some(AppearanceSection::Mode),
                Some("settings-accent-choices") => Some(AppearanceSection::Accent),
                Some("settings-wallpaper") => Some(AppearanceSection::Wallpaper),
                Some("settings-interface") => Some(AppearanceSection::Interface),
                Some("settings-reset") => Some(AppearanceSection::Reset),
                _ => None,
            })
            .collect::<Vec<_>>();
        if order.len() != 5 {
            return Err("Appearance page component order is invalid".into());
        }
        Ok(Self {
            mode: ModeTree::parse(find("settings-appearance-modes")?)?,
            accent: AccentTree::parse(find("settings-accent-choices")?)?,
            dialog: DialogTree::parse(find("settings-hue-dialog")?)?,
            wallpaper: WallpaperTree::parse(find("settings-wallpaper")?)?,
            interface: InterfaceTree::parse(find("settings-interface")?)?,
            reset: ResetTree::parse(find("settings-reset")?)?,
            order,
        })
    }

    fn retained_bytes(&self) -> usize {
        self.mode.retained_bytes()
            + self.accent.retained_bytes()
            + self.dialog.retained_bytes()
            + self.wallpaper.retained_bytes()
            + self.interface.retained_bytes()
            + self.reset.label.capacity()
            + self.order.capacity() * std::mem::size_of::<AppearanceSection>()
    }

    fn view(
        &self,
        theme: SemanticTheme,
        appearance: Appearance,
        wallpaper_preview: Option<&Arc<image::RgbaImage>>,
    ) -> AnyView<SettingsMessage> {
        AnyView::new(Column::new().gap(10.0).children(self.order.iter().map(
            |section| match section {
                AppearanceSection::Mode => self.mode.view(theme, appearance),
                AppearanceSection::Accent => self.accent.view(theme),
                AppearanceSection::Wallpaper => self.wallpaper.view(theme, wallpaper_preview),
                AppearanceSection::Interface => self.interface.view(theme),
                AppearanceSection::Reset => self.reset.view(theme),
            },
        )))
    }

    fn dialog_view(&self, theme: SemanticTheme) -> Option<AnyView<SettingsMessage>> {
        self.dialog.open.then(|| self.dialog.view(theme))
    }

    #[cfg(test)]
    fn action_for_id(&self, id: &str) -> Option<usize> {
        self.mode
            .action_for_id(id)
            .or_else(|| self.dialog.action_for_id(id))
            .or_else(|| self.wallpaper.action_for_id(id))
            .or_else(|| self.interface.action_for_id(id))
            .or_else(|| (id == "appearance-reset").then_some(self.reset.action))
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
                let message = SettingsMessage::AppearanceJsxAction(swatch.action);
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
                        SettingsMessage::AppearanceJsxAction(choice.action),
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

pub(super) struct AppearancePage {
    runtime: JsxRuntime,
    last_data: Option<String>,
    tree: Option<AppearanceTree>,
}

impl AppearancePage {
    pub(super) fn new() -> Result<Self, String> {
        Ok(Self {
            runtime: JsxRuntime::new(
                crate::settings_package::source(crate::settings_package::Script::Appearance)?,
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
        wallpaper_preview: Option<&Arc<image::RgbaImage>>,
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
            .view(theme, appearance, wallpaper_preview))
    }

    pub(super) fn dialog_view(&self, theme: SemanticTheme) -> Option<AnyView<SettingsMessage>> {
        self.tree.as_ref()?.dialog_view(theme)
    }

    fn action_for_control(&self, id: &str) -> Option<usize> {
        self.tree.as_ref()?.interface.action_for_id(id)
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
    AppearanceHue { fraction: f32 },
    AppearanceIntensity { fraction: f32 },
    ReduceTransparency { value: bool },
    ToggleAnimationSelect,
    Animation { value: String },
    ToggleFileArtworkSelect,
    FileArtwork { value: String },
    WallpaperChoose,
    WallpaperRemove,
    ToggleWallpaperPosition,
    WallpaperPosition { value: String },
    AppearanceReset,
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
        AppearanceRequest::ReduceTransparency { value } => {
            if data.get("reduceTransparency").and_then(Value::as_bool) != Some(!value) {
                return Err("Transparency preference changed".into());
            }
            Ok(SettingsMessage::SetReduceTransparency(value))
        }
        AppearanceRequest::AppearanceHue { fraction } => {
            if !fraction.is_finite() || !(0.0..=1.0).contains(&fraction) {
                return Err("Appearance hue is invalid".into());
            }
            Ok(SettingsMessage::SetAppearanceHue(
                (fraction * 359.0).round() as u16,
            ))
        }
        AppearanceRequest::AppearanceIntensity { fraction } => {
            if !fraction.is_finite() || !(0.0..=1.0).contains(&fraction) {
                return Err("Appearance intensity is invalid".into());
            }
            Ok(SettingsMessage::SetAppearanceIntensity(
                (fraction * 100.0).round() as u8,
            ))
        }
        AppearanceRequest::ToggleAnimationSelect => Ok(SettingsMessage::ToggleAnimationSelect),
        AppearanceRequest::Animation { value } => match value.as_str() {
            "off" => Ok(SettingsMessage::SetAnimationLevel(AnimationLevel::Off)),
            "reduced" => Ok(SettingsMessage::SetAnimationLevel(AnimationLevel::Reduced)),
            "normal" => Ok(SettingsMessage::SetAnimationLevel(AnimationLevel::Normal)),
            _ => Err("Appearance animation level is invalid".into()),
        },
        AppearanceRequest::ToggleFileArtworkSelect => {
            Ok(SettingsMessage::ToggleFileIconProviderSelect)
        }
        AppearanceRequest::FileArtwork { value } => match value.as_str() {
            "nickel" => Ok(SettingsMessage::SetFileIconProvider(
                FileIconPreference::Nickel,
            )),
            "system" => Ok(SettingsMessage::SetFileIconProvider(
                FileIconPreference::System,
            )),
            _ => {
                let Some(theme) = value.strip_prefix("theme:") else {
                    return Err("File artwork choice is invalid".into());
                };
                if theme.is_empty()
                    || !data
                        .get("fileArtworkOptions")
                        .and_then(Value::as_array)
                        .is_some_and(|options| options.iter().any(|option| option["id"] == value))
                {
                    return Err("File artwork theme is unavailable".into());
                }
                Ok(SettingsMessage::SetFileIconTheme(theme.to_owned()))
            }
        },
        AppearanceRequest::WallpaperChoose => {
            if data.get("wallpaperDialogPending").and_then(Value::as_bool) != Some(false) {
                return Err("Wallpaper chooser is already open".into());
            }
            Ok(SettingsMessage::WallpaperChoose)
        }
        AppearanceRequest::WallpaperRemove => Ok(SettingsMessage::WallpaperRemove),
        AppearanceRequest::ToggleWallpaperPosition => {
            Ok(SettingsMessage::ToggleWallpaperPositionSelect)
        }
        AppearanceRequest::WallpaperPosition { value } => {
            if data
                .get("wallpaperPositionExpanded")
                .and_then(Value::as_bool)
                != Some(true)
            {
                return Err("Wallpaper position choices are closed".into());
            }
            let position = match value.as_str() {
                "fill" => WallpaperPosition::Fill,
                "fit" => WallpaperPosition::Fit,
                "stretch" => WallpaperPosition::Stretch,
                "center" => WallpaperPosition::Center,
                "tile" => WallpaperPosition::Tile,
                "span" => WallpaperPosition::Span,
                _ => return Err("Wallpaper position is invalid".into()),
            };
            Ok(SettingsMessage::WallpaperPosition(position))
        }
        AppearanceRequest::AppearanceReset => Ok(SettingsMessage::AppearanceReset),
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
    let intensity = app
        .shell_settings
        .displayed_intensity(nickel_platform::appearance());
    let animation_id = match app.shell_settings.animations {
        AnimationLevel::Off => "off",
        AnimationLevel::Reduced => "reduced",
        AnimationLevel::Normal => "normal",
    };
    let animations = [
        ("off", "settings-animations-off"),
        ("reduced", "settings-animations-reduced"),
        ("normal", "settings-animations-normal"),
    ]
    .into_iter()
    .map(|(id, key)| json!({"id":id,"label":app.localizer.text(key)}))
    .collect::<Vec<_>>();
    let installed_themes = nickel_platform::installed_icon_themes();
    let configured_theme = app.shell_settings.file_icon_theme.as_deref();
    let configured_theme_available = configured_theme
        .is_none_or(|configured| installed_themes.iter().any(|theme| theme == configured));
    let file_artwork_id = match (app.shell_settings.file_icon_provider, configured_theme) {
        (FileIconPreference::Nickel, _) => "nickel".to_owned(),
        (FileIconPreference::System, None) => "system".to_owned(),
        (FileIconPreference::System, Some(theme)) => format!("theme:{theme}"),
    };
    let file_artwork_value = match (app.shell_settings.file_icon_provider, configured_theme) {
        (FileIconPreference::Nickel, _) => "Nickel".to_owned(),
        (FileIconPreference::System, None) => "System".to_owned(),
        (FileIconPreference::System, Some(theme)) if configured_theme_available => {
            format!("System — {theme}")
        }
        (FileIconPreference::System, Some(theme)) => {
            format!("System — {theme} (unavailable)")
        }
    };
    let mut file_artwork_options = vec![
        json!({"id":"nickel","label":"Nickel"}),
        json!({"id":"system","label":"System"}),
    ];
    file_artwork_options.extend(
        installed_themes.into_iter().take(64).map(
            |theme| json!({"id":format!("theme:{theme}"),"label":format!("System — {theme}")}),
        ),
    );
    let wallpaper_positions = [
        ("fill", "settings-wallpaper-fill"),
        ("fit", "settings-wallpaper-fit"),
        ("stretch", "settings-wallpaper-stretch"),
        ("center", "settings-wallpaper-center"),
        ("tile", "settings-wallpaper-tile"),
        ("span", "settings-wallpaper-span"),
    ]
    .into_iter()
    .map(|(id, key)| json!({"id":id,"label":app.localizer.text(key)}))
    .collect::<Vec<_>>();
    let wallpaper_position_id = match app.wallpaper_settings.position {
        WallpaperPosition::Fill => "fill",
        WallpaperPosition::Fit => "fit",
        WallpaperPosition::Stretch => "stretch",
        WallpaperPosition::Center => "center",
        WallpaperPosition::Tile => "tile",
        WallpaperPosition::Span => "span",
    };
    let wallpaper_name = app
        .wallpaper_settings
        .image
        .as_deref()
        .and_then(|path| path.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| app.localizer.text("settings-wallpaper-none"));
    let mut page = json!({
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
        "transparencyTitle":app.localizer.text("settings-reduce-transparency"),
        "transparencyDescription":app.localizer.text("settings-reduce-transparency-description"),
        "reduceTransparency":app.shell_settings.reduce_transparency,
        "interfaceTitle":app.localizer.text("settings-interface-settings"),
        "hueTitle":app.localizer.text("settings-appearance-starting-hue"),
        "hueDescription":app.localizer.text("settings-appearance-hue-description"),
        "hueValue":app.localizer.number("settings-appearance-hue-value", "degrees", i64::from(hue)),
        "intensity":intensity,
        "intensityTitle":app.localizer.text("settings-appearance-color-intensity"),
        "intensityDescription":app.localizer.text("settings-appearance-intensity-description"),
        "intensityValue":app.localizer.number("settings-appearance-intensity-value", "percent", i64::from(intensity)),
        "animationTitle":app.localizer.text("settings-animations"),
        "animationDescription":app.localizer.text("settings-animations-description"),
        "animationValue":app.localizer.text(match app.shell_settings.animations {
            AnimationLevel::Off => "settings-animations-off",
            AnimationLevel::Reduced => "settings-animations-reduced",
            AnimationLevel::Normal => "settings-animations-normal",
        }),
        "animationId":animation_id,
        "animationExpanded":app.animation_select_expanded,
        "animations":animations,
        "fileArtworkTitle":"File artwork",
        "fileArtworkDescription":"Choose Nickel artwork or icons supplied by the operating system.",
        "fileArtworkValue":file_artwork_value,
        "fileArtworkId":file_artwork_id,
        "fileArtworkExpanded":app.file_icon_provider_select_expanded,
        "fileArtworkOptions":file_artwork_options,
    });
    let wallpaper = json!({
        "wallpaperTitle":app.localizer.text("settings-wallpaper-image"),
        "wallpaperDescription":app.localizer.text("settings-wallpaper-description"),
        "wallpaperName":wallpaper_name,
        "wallpaperDimensions":app.wallpaper_dimensions.map_or_else(String::new, |(width,height)| format!("{width} × {height}")),
        "wallpaperNone":app.localizer.text("settings-wallpaper-none"),
        "wallpaperStatus":app.wallpaper_status.as_deref().unwrap_or("").chars().take(256).collect::<String>(),
        "wallpaperChoose":app.localizer.text("settings-wallpaper-choose"),
        "wallpaperRemove":app.localizer.text("settings-wallpaper-remove"),
        "wallpaperFitTitle":app.localizer.text("settings-wallpaper-fit-label"),
        "wallpaperFitDescription":app.localizer.text("settings-wallpaper-fit-description"),
        "wallpaperPositionId":wallpaper_position_id,
        "wallpaperPositionValue":app.localizer.text(match app.wallpaper_settings.position {
            WallpaperPosition::Fill => "settings-wallpaper-fill",
            WallpaperPosition::Fit => "settings-wallpaper-fit",
            WallpaperPosition::Stretch => "settings-wallpaper-stretch",
            WallpaperPosition::Center => "settings-wallpaper-center",
            WallpaperPosition::Tile => "settings-wallpaper-tile",
            WallpaperPosition::Span => "settings-wallpaper-span",
        }),
        "wallpaperPositionExpanded":app.wallpaper_position_select_expanded,
        "wallpaperDialogPending":app.wallpaper_dialog_rx.is_some(),
        "wallpaperPositions":wallpaper_positions,
        "resetLabel":app.localizer.text("settings-appearance-reset"),
    });
    page.as_object_mut()
        .expect("Appearance projection is an object")
        .extend(
            wallpaper
                .as_object()
                .expect("Wallpaper projection is an object")
                .clone(),
        );
    page
}

impl SettingsApp {
    pub(super) fn handle_appearance_jsx_slider(&mut self, id: &str, position: u16) {
        let action = self
            .appearance_page
            .borrow()
            .as_ref()
            .and_then(|page| page.as_ref().ok())
            .and_then(|page| page.action_for_control(id));
        if let Some(action) = action {
            self.handle_appearance_jsx_action(
                action,
                Value::from(f32::from(position) / f32::from(u16::MAX)),
            );
        }
    }

    pub(super) fn handle_appearance_jsx_action(&mut self, index: usize, value: Value) {
        if self.page != SettingsPage::Appearance {
            return;
        }
        let data = projection(self);
        let result = self
            .appearance_page
            .borrow_mut()
            .as_mut()
            .ok_or_else(|| "Appearance choices are not loaded".to_owned())
            .and_then(|page| page.as_mut().map_err(|error| error.clone()))
            .and_then(|page| page.dispatch(index, value, &data));
        match result {
            Ok(message) => self.handle_settings_message(message),
            Err(error) => {
                if error != STALE_STATUS {
                    *self.appearance_page.borrow_mut() = Some(Err(error));
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
            .appearance_page
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
        let interface = json!({"kind":"settings-interface","id":"appearance-interface-card",
        "label":"Interface","children":[
            {"kind":"settings-slider","id":"appearance-hue","label":"Hue","value":"224°","placeholder":"Hue description","percent":0.5,"action":8},
            {"kind":"settings-slider","id":"appearance-intensity","label":"Intensity","value":"100%","placeholder":"Intensity description","percent":1.0,"action":9},
            {"kind":"settings-transparency","id":"appearance-transparency","label":"Transparency","value":"Description","selected":false,"action":10},
            {"kind":"settings-select","id":"appearance-animations","label":"Animations","placeholder":"Description","value":"Normal","open":false,"action":11,
                "children":[
                    {"kind":"settings-option","id":"appearance-animation-off","label":"Off","selected":false,"action":12},
                    {"kind":"settings-option","id":"appearance-animation-reduced","label":"Reduced","selected":false,"action":13},
                    {"kind":"settings-option","id":"appearance-animation-normal","label":"Normal","selected":true,"action":14}
                ]},
            {"kind":"settings-select","id":"appearance-file-artwork","label":"Artwork","placeholder":"Description","value":"Nickel","open":false,"action":15,
                "children":[
                    {"kind":"settings-option","id":"appearance-file-artwork-option-0","label":"Nickel","selected":true,"action":16},
                    {"kind":"settings-option","id":"appearance-file-artwork-option-1","label":"System","selected":false,"action":17}
                ]}
            ]});
        let wallpaper = json!({"kind":"settings-wallpaper","id":"appearance-wallpaper-card",
        "label":"Wallpaper","value":"Description","children":[
            {"kind":"settings-wallpaper-preview","id":"appearance-wallpaper-preview","label":"None","value":"","placeholder":"None","state":""},
            {"kind":"settings-button","id":"appearance-wallpaper-choose","label":"Choose","action":18},
            {"kind":"settings-button","id":"appearance-wallpaper-remove","label":"Remove","action":19},
            {"kind":"settings-select","id":"appearance-wallpaper-position","label":"Fit","placeholder":"Description","value":"Fill","open":false,"action":20,
                "children":[
                    {"kind":"settings-option","id":"appearance-wallpaper-position-fill","label":"Fill","selected":true,"action":21},
                    {"kind":"settings-option","id":"appearance-wallpaper-position-fit","label":"Fit","selected":false,"action":22},
                    {"kind":"settings-option","id":"appearance-wallpaper-position-stretch","label":"Stretch","selected":false,"action":23},
                    {"kind":"settings-option","id":"appearance-wallpaper-position-center","label":"Center","selected":false,"action":24},
                    {"kind":"settings-option","id":"appearance-wallpaper-position-tile","label":"Tile","selected":false,"action":25},
                    {"kind":"settings-option","id":"appearance-wallpaper-position-span","label":"Span","selected":false,"action":26}
                ]}
        ]});
        let reset =
            json!({"kind":"settings-reset","id":"appearance-reset","label":"Reset","action":27});
        let root = json!({"kind":"settings-appearance-page",
            "children":[wallpaper, interface, accents, modes, dialog, reset]});
        let tree = AppearanceTree::parse(&root).unwrap();
        assert_eq!(tree.order[0], AppearanceSection::Wallpaper);
        assert_eq!(tree.order[1], AppearanceSection::Interface);
        assert_eq!(tree.order[2], AppearanceSection::Accent);
        assert_eq!(tree.action_for_id("appearance-accent-224"), Some(3));
        assert_eq!(tree.action_for_id("appearance-transparency"), Some(10));
        assert_eq!(
            tree.action_for_id("appearance-wallpaper-position-fit"),
            Some(22)
        );
    }

    #[test]
    fn jsx_mode_choice_requests_typed_preference_and_checks_current_projection() {
        let mut app = SettingsApp::with_initial_page(SettingsPage::Appearance);
        app.shell_settings.theme = ThemePreference::Light;
        let data = projection(&app);
        let mut page = AppearancePage::new().unwrap();
        page.render(&data, app.ui_theme(), nickel_platform::appearance(), None)
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
        let mut page = AppearancePage::new().unwrap();
        page.render(&data, app.ui_theme(), nickel_platform::appearance(), None)
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

    #[test]
    fn jsx_transparency_switch_checks_current_value() {
        let mut app = SettingsApp::with_initial_page(SettingsPage::Appearance);
        app.shell_settings.reduce_transparency = false;
        let data = projection(&app);
        let mut page = AppearancePage::new().unwrap();
        page.render(&data, app.ui_theme(), nickel_platform::appearance(), None)
            .unwrap();
        let action = page.action_for_id("appearance-transparency").unwrap();
        assert_eq!(
            page.dispatch(action, Value::Null, &data).unwrap(),
            SettingsMessage::SetReduceTransparency(true)
        );
        assert!(
            validate_request(
                AppearanceRequest::ReduceTransparency { value: false },
                &data,
            )
            .is_err()
        );
    }

    #[test]
    fn interface_effects_reject_out_of_range_and_unknown_choices() {
        let app = SettingsApp::with_initial_page(SettingsPage::Appearance);
        let data = projection(&app);
        assert!(
            validate_request(AppearanceRequest::AppearanceHue { fraction: 1.1 }, &data).is_err()
        );
        assert!(
            validate_request(
                AppearanceRequest::AppearanceIntensity { fraction: -0.1 },
                &data
            )
            .is_err()
        );
        assert!(
            validate_request(
                AppearanceRequest::Animation {
                    value: "slow".into()
                },
                &data
            )
            .is_err()
        );
        assert!(
            validate_request(
                AppearanceRequest::FileArtwork {
                    value: "theme:missing-theme".into()
                },
                &data
            )
            .is_err()
        );
        assert_eq!(
            validate_request(AppearanceRequest::AppearanceHue { fraction: 0.5 }, &data).unwrap(),
            SettingsMessage::SetAppearanceHue(180)
        );
    }

    #[test]
    fn wallpaper_requests_require_current_menu_and_chooser_state() {
        let mut app = SettingsApp::with_initial_page(SettingsPage::Appearance);
        let closed = projection(&app);
        assert!(
            validate_request(
                AppearanceRequest::WallpaperPosition {
                    value: "tile".into()
                },
                &closed,
            )
            .is_err()
        );
        app.wallpaper_position_select_expanded = true;
        let open = projection(&app);
        assert_eq!(
            validate_request(
                AppearanceRequest::WallpaperPosition {
                    value: "tile".into()
                },
                &open,
            )
            .unwrap(),
            SettingsMessage::WallpaperPosition(WallpaperPosition::Tile)
        );
        assert!(
            validate_request(
                AppearanceRequest::WallpaperPosition {
                    value: "other".into()
                },
                &open,
            )
            .is_err()
        );
        assert_eq!(
            validate_request(AppearanceRequest::WallpaperChoose, &open).unwrap(),
            SettingsMessage::WallpaperChoose
        );
    }
}
