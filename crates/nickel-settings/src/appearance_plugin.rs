//! Appearance rendered by the shared JSX component tree.
//! The host retains persistence, wallpaper resources, popover placement, and validation.

use std::sync::Arc;

use nickel_core::{
    shell_settings::{AnimationLevel, FileIconPreference, ThemePreference},
    theme::accent_from_hue,
    wallpaper_settings::WallpaperPosition,
};
use nickel_plugin_presentation::{
    components::{PanelNode, PluginImages},
    css::StyleSheet,
    page::{JsxPage, STALE_DATA},
};
use nickel_ui::{
    AnyView, Button, ButtonPresentation, Column, FrameOverlay, OverlayAnchor, OverlayStyle,
    Popover, Row, SemanticTheme, SettingsCard, SettingsRow, Size, TextField, UiId,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{SettingsApp, SettingsMessage, SettingsPage};

const STALE_STATUS: &str = STALE_DATA;
const APPEARANCE_SCOPE: &str = "appearance";

pub(super) struct AppearancePage {
    page: JsxPage,
    stylesheet: StyleSheet,
    last_theme: Option<SemanticTheme>,
    images: PluginImages,
}

impl AppearancePage {
    pub(super) fn new() -> Result<Self, String> {
        Ok(Self {
            page: JsxPage::new(
                crate::settings_package::source(crate::settings_package::Script::Appearance)?,
                crate::settings_package::manifest()?.clone(),
                None,
            )?,
            stylesheet: StyleSheet::default(),
            last_theme: None,
            images: PluginImages::new(),
        })
    }

    pub(super) fn retained_bytes(&self) -> usize {
        self.page.retained_bytes() + self.stylesheet.estimated_retained_bytes() as usize
    }

    pub(super) fn render(
        &mut self,
        data: &Value,
        theme: SemanticTheme,
        wallpaper_preview: Option<&Arc<image::RgbaImage>>,
    ) -> Result<AnyView<SettingsMessage>, String> {
        self.page.render(data)?;
        if self.last_theme != Some(theme) {
            self.stylesheet = crate::settings_plugin::stylesheet_template(
                include_str!("../../../assets/plugins/settings/settings-appearance.css"),
                theme,
            )?;
            self.last_theme = Some(theme);
        }
        self.images.clear();
        if let Some(preview) = wallpaper_preview {
            self.images
                .insert("wallpaper-preview".into(), (1, Arc::clone(preview)));
        }
        let node = self.page.node().ok_or("Appearance page is unavailable")?;
        let PanelNode::Div { children, .. } = node else {
            return Err("Appearance needs a shared div root".into());
        };
        if children.len() != 6 || !matches!(&children[2], PanelNode::Dialog { .. }) {
            return Err("Appearance sections or dialog are invalid".into());
        }
        Ok(node.view_as_scoped::<SettingsMessage>(
            &self.images,
            &self.stylesheet,
            Some(APPEARANCE_SCOPE),
        ))
    }

    pub(super) fn dialog_view(&self, _theme: SemanticTheme) -> Option<AnyView<SettingsMessage>> {
        let PanelNode::Dialog {
            open: true,
            children,
            ..
        } = self.page.node()?.dialog("appearance-custom-hue-dialog")?
        else {
            return None;
        };
        Some(AnyView::new(Column::new().children(children.iter().map(
            |child| {
                child.view_as_scoped::<SettingsMessage>(
                    &self.images,
                    &self.stylesheet,
                    Some(APPEARANCE_SCOPE),
                )
            },
        ))))
    }

    fn dispatch(
        &mut self,
        index: usize,
        value: Value,
        current_data: &Value,
    ) -> Result<SettingsMessage, String> {
        self.page.dispatch(index, &value, current_data, |effect| {
            let request: AppearanceRequest =
                serde_json::from_value(effect).map_err(|error| error.to_string())?;
            validate_request(request, current_data)
        })
    }

    #[cfg(test)]
    pub(super) fn action_for_id(&self, id: &str) -> Option<usize> {
        self.page.node().and_then(|node| {
            node.button_action(id)
                .or_else(|| node.text_field_action(id))
                .or_else(|| node.slider_action(id))
        })
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
        .map(|preset| {
            let [red, green, blue] = accent_from_hue(preset);
            json!({
                "hue":preset,
                "color":format!("#{red:02x}{green:02x}{blue:02x}"),
                "selected":hue.abs_diff(preset) < 3,
            })
        })
        .collect::<Vec<_>>();
    let intensity = app
        .shell_settings
        .displayed_intensity(nickel_platform::appearance());
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
        "animationExpanded":app.animation_select_expanded,
        "animations":animations,
        "fileArtworkTitle":"File artwork",
        "fileArtworkDescription":"Choose Nickel artwork or icons supplied by the operating system.",
        "fileArtworkValue":file_artwork_value,
        "fileArtworkExpanded":app.file_icon_provider_select_expanded,
        "fileArtworkOptions":file_artwork_options,
    });
    let wallpaper = json!({
        "wallpaperTitle":app.localizer.text("settings-wallpaper-image"),
        "wallpaperDescription":app.localizer.text("settings-wallpaper-description"),
        "wallpaperName":wallpaper_name,
        "wallpaperHasPreview":app.wallpaper_preview.is_some(),
        "wallpaperDimensions":app.wallpaper_dimensions.map_or_else(String::new, |(width,height)| format!("{width} × {height}")),
        "wallpaperNone":app.localizer.text("settings-wallpaper-none"),
        "wallpaperStatus":app.wallpaper_status.as_deref().unwrap_or("").chars().take(256).collect::<String>(),
        "wallpaperChoose":app.localizer.text("settings-wallpaper-choose"),
        "wallpaperRemove":app.localizer.text("settings-wallpaper-remove"),
        "wallpaperFitTitle":app.localizer.text("settings-wallpaper-fit-label"),
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
