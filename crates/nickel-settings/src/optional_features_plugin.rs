//! Optional Features layout from JSX, with policy and persistence owned by Rust.

use nickel_core::{
    on_screen_keyboard::KeyboardPreference, optional_features::FeatureEffectiveState,
};
use nickel_ui::{AnyView, Column, Insets, SemanticTheme, VerticalScroll};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    SettingsApp, SettingsMessage, SettingsPage,
    settings_components::{Node, SettingsJsxContext},
    view::codex_switch_state,
};

const STALE_STATUS: &str = "Optional feature status changed; refresh the page";

pub(super) struct OptionalFeaturesPage {
    context: SettingsJsxContext,
}

impl OptionalFeaturesPage {
    pub(super) fn retained_bytes(&self) -> usize {
        self.context.retained_bytes()
    }

    pub(super) fn new() -> Result<Self, String> {
        Ok(Self {
            context: SettingsJsxContext::new(
                crate::settings_package::source(crate::settings_package::Script::OptionalFeatures)?,
                parse_tree,
                STALE_STATUS,
                "Optional feature action must request one operation",
            )?,
        })
    }

    pub(super) fn render(
        &mut self,
        data: &Value,
        theme: SemanticTheme,
    ) -> Result<AnyView<SettingsMessage>, String> {
        let node = self.context.render(data)?;
        Ok(AnyView::new(
            VerticalScroll::new(SettingsMessage::OptionalFeaturesScroll, 0.0)
                .grow(1.0)
                .theme(theme)
                .child(
                    Column::new()
                        .fill_width()
                        .padding(Insets {
                            top: 0.0,
                            right: 12.0,
                            bottom: 24.0,
                            left: 0.0,
                        })
                        .child(node.view(theme, "", SettingsMessage::OptionalFeaturesJsxAction)),
                ),
        ))
    }

    fn dispatch(&mut self, index: usize, data: &Value) -> Result<SettingsMessage, String> {
        self.context.dispatch(index, &Value::Null, data, |effect| {
            let request: OptionalRequest =
                serde_json::from_value(effect.clone()).map_err(|error| error.to_string())?;
            validate_request(request, data)
        })
    }

    #[cfg(test)]
    pub(super) fn action_for_id(&self, id: &str) -> Option<usize> {
        self.context.action_for_id(id)
    }
}

fn parse_tree(value: &Value) -> Result<Node, String> {
    if value.get("kind").and_then(Value::as_str) != Some("settings-features")
        || value
            .get("children")
            .and_then(Value::as_array)
            .is_none_or(|children| {
                children.len() != 2
                    || children.iter().any(|child| {
                        child.get("kind").and_then(Value::as_str) != Some("settings-card")
                    })
            })
    {
        return Err("Optional Features page structure is invalid".into());
    }
    let node = Node::parse(value)?;
    if node.contains_input() {
        return Err("Optional Features cannot request text input".into());
    }
    Ok(node)
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
enum OptionalRequest {
    KeyboardMode { mode: String },
    CodexEnabled { enabled: bool },
    ConfirmDisable,
    CancelDisable,
    RetryCodex,
}

fn validate_request(request: OptionalRequest, data: &Value) -> Result<SettingsMessage, String> {
    match request {
        OptionalRequest::KeyboardMode { mode } => {
            if data.pointer("/keyboard/editable").and_then(Value::as_bool) != Some(true) {
                return Err("Keyboard mode is controlled by the environment".into());
            }
            let mode = match mode.as_str() {
                "automatic" => KeyboardPreference::Automatic,
                "enabled" => KeyboardPreference::Enabled,
                "disabled" => KeyboardPreference::Disabled,
                _ => return Err("Keyboard mode is invalid".into()),
            };
            Ok(SettingsMessage::SetOnScreenKeyboard(mode))
        }
        OptionalRequest::CodexEnabled { enabled } => {
            if data.pointer("/codex/editable").and_then(Value::as_bool) != Some(true)
                || data.pointer("/codex/nextEnabled").and_then(Value::as_bool) != Some(enabled)
            {
                return Err("Codex switch is unavailable".into());
            }
            Ok(SettingsMessage::SetCodexEnabled(enabled))
        }
        OptionalRequest::ConfirmDisable | OptionalRequest::CancelDisable => {
            if data
                .pointer("/codex/confirmation")
                .is_none_or(Value::is_null)
            {
                return Err("Codex disable confirmation is unavailable".into());
            }
            Ok(if matches!(request, OptionalRequest::ConfirmDisable) {
                SettingsMessage::ConfirmDisableCodex
            } else {
                SettingsMessage::CancelDisableCodex
            })
        }
        OptionalRequest::RetryCodex => {
            if data.pointer("/codex/retry").and_then(Value::as_bool) != Some(true) {
                return Err("Codex retry is unavailable".into());
            }
            Ok(SettingsMessage::RetryCodexProbe)
        }
    }
}

pub(super) fn projection(app: &SettingsApp) -> Value {
    let state = &app.codex_feature;
    let switch_state = codex_switch_state(state);
    let switch = match switch_state {
        nickel_ui::SwitchState::On => "on",
        nickel_ui::SwitchState::Off => "off",
        nickel_ui::SwitchState::Mixed => "mixed",
        nickel_ui::SwitchState::MixedUnavailable => "mixed-unavailable",
        nickel_ui::SwitchState::DisabledOn => "disabled-on",
        nickel_ui::SwitchState::DisabledOff => "disabled-off",
    };
    let codex_editable = matches!(
        switch_state,
        nickel_ui::SwitchState::On | nickel_ui::SwitchState::Off | nickel_ui::SwitchState::Mixed
    );
    let codex_status = match state.effective {
        FeatureEffectiveState::Disabled => "Off",
        FeatureEffectiveState::Enabling | FeatureEffectiveState::Stale => "Starting Codex…",
        FeatureEffectiveState::Enabled => "On",
        FeatureEffectiveState::Unavailable => "Codex is unavailable",
        FeatureEffectiveState::Rejected => "Codex could not start",
    };
    let keyboard_editable = !app
        .keyboard_runtime
        .as_ref()
        .is_some_and(|runtime| runtime.environment_override);
    let keyboard_status = if let Some(error) = &app.keyboard_error {
        format!("Could not save: {error}")
    } else if let Some(runtime) = &app.keyboard_runtime {
        if runtime.generation != app.optional_features.on_screen_keyboard_generation {
            "Saved; waiting for the shell".into()
        } else if runtime.environment_override {
            format!(
                "{} for this session · controlled by the shell environment",
                if runtime.enabled { "On" } else { "Off" }
            )
        } else {
            format!(
                "{} · {}",
                if runtime.enabled { "On" } else { "Off" },
                if runtime.touchscreen_present {
                    "Touchscreen detected"
                } else {
                    "No touchscreen detected"
                }
            )
        }
    } else {
        "Shell keyboard status unavailable".into()
    };
    let preference = app.optional_features.on_screen_keyboard;
    let confirmation = app.codex_disable_confirmation.then(|| json!({
        "title":format!("Close {} built-in Codex window(s) and disable?", app.optional_feature_runtime.active_windows),
        "description":"External Codex clients and upstream conversation history are not affected.",
        "confirm":"Close & disable", "cancel":"Cancel",
    }));
    json!({
        "keyboard":{
            "title":app.localizer.text("ui-pages-on-screen-keyboard"),
            "description":app.localizer.text("ui-pages-type-with-touch-a-controller-or-a-mouse"),
            "editable":keyboard_editable,
            "statusLabel":"Current state", "status":keyboard_status,
            "options":[
                {"value":"automatic","label":"Automatic","description":"Enable when a touchscreen is connected.","selected":preference == KeyboardPreference::Automatic},
                {"value":"enabled","label":"On","description":"Available even without a touchscreen.","selected":preference == KeyboardPreference::Enabled},
                {"value":"disabled","label":"Off","description":"Hide the keyboard and its tray control.","selected":preference == KeyboardPreference::Disabled},
            ]
        },
        "codex":{
            "title":app.localizer.text("ui-pages-codex"),
            "description":app.localizer.text("ui-pages-use-codex-projects-and-conversations-in-nickel"),
            "enableLabel":"Enable Codex", "accessibilityLabel":"Enable Codex integration",
            "status":codex_status, "switchState":switch,
            "editable":codex_editable,
            "nextEnabled":switch_state == nickel_ui::SwitchState::Off,
            "confirmation":confirmation,
            "retry":matches!(state.effective, FeatureEffectiveState::Unavailable | FeatureEffectiveState::Rejected),
            "retryLabel":"Try again",
        }
    })
}

impl SettingsApp {
    pub(super) fn handle_optional_features_jsx_action(&mut self, index: usize) {
        if self.page != SettingsPage::OptionalFeatures {
            return;
        }
        let data = projection(self);
        let result = self
            .optional_features_page
            .borrow_mut()
            .as_mut()
            .ok_or_else(|| "Optional Features page is not loaded".to_owned())
            .and_then(|page| page.as_mut().map_err(|error| error.clone()))
            .and_then(|page| page.dispatch(index, &data));
        match result {
            Ok(message) => self.handle_settings_message(message),
            Err(error) => {
                if error != STALE_STATUS {
                    *self.optional_features_page.borrow_mut() = Some(Err(error));
                }
                self.request_redraw();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nickel_core::optional_features::{FeatureInstallation, FeaturePolicy, FeatureSupport};

    #[test]
    fn jsx_codex_confirmation_requires_current_confirmation_state() {
        let mut app = SettingsApp::with_initial_page(SettingsPage::OptionalFeatures);
        app.persistence_enabled = false;
        app.codex_feature.capability.installation = FeatureInstallation::Installed;
        app.codex_feature.capability.support = FeatureSupport::Supported;
        app.codex_feature.capability.policy = FeaturePolicy::Editable;
        app.codex_feature.effective = FeatureEffectiveState::Enabled;
        app.codex_feature.requested_enabled = true;
        app.optional_features.codex_enabled = true;
        app.optional_feature_runtime.active_windows = 2;
        app.codex_disable_confirmation = true;
        let data = projection(&app);
        let theme = app.ui_theme();
        let mut page = OptionalFeaturesPage::new().unwrap();
        page.render(&data, theme).unwrap();
        let action = page.action_for_id("codex-confirm-disable").unwrap();
        assert_eq!(
            page.dispatch(action, &data).unwrap(),
            SettingsMessage::ConfirmDisableCodex
        );

        app.codex_disable_confirmation = false;
        let changed = projection(&app);
        assert_eq!(page.dispatch(action, &changed).unwrap_err(), STALE_STATUS);
        assert!(validate_request(OptionalRequest::ConfirmDisable, &changed).is_err());
    }

    #[test]
    fn jsx_codex_switch_cannot_override_policy() {
        let mut app = SettingsApp::with_initial_page(SettingsPage::OptionalFeatures);
        app.codex_feature.capability.policy = FeaturePolicy::ForceDisabled;
        let data = projection(&app);
        assert!(validate_request(OptionalRequest::CodexEnabled { enabled: true }, &data,).is_err());
    }
}
