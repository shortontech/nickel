use super::*;
use nickel_ui::{
    Collection, CollectionPresentation, CollectionState, Column, ComponentBuilderExt,
    GridColumnSpec, Layer, NavigationScope, Point, RadioGroup, RadioOption, Row, SettingsListCard,
    Text, Track,
};

fn bluetooth_kind_icon(kind: &str) -> Option<String> {
    let icon = match kind {
        "audio-card" | "audio-headphones" | "audio-headset" => "🎧",
        "input-keyboard" => "⌨",
        "input-mouse" => "🖱",
        "input-gaming" => "🎮",
        "phone" => "📱",
        "computer" => "💻",
        _ => return None,
    };
    Some(icon.to_owned())
}

fn bluetooth_signal_label(signal_dbm: i16) -> String {
    format!("{signal_dbm} dBm")
}

pub(crate) fn codex_switch_state(state: &FeatureState) -> SwitchState {
    let available = state.capability.support == FeatureSupport::Supported
        && state.capability.installation == FeatureInstallation::Installed;
    if !available || !state.editable() {
        return if state.requested_enabled {
            SwitchState::DisabledOn
        } else {
            SwitchState::DisabledOff
        };
    }
    match state.effective {
        FeatureEffectiveState::Disabled => SwitchState::Off,
        FeatureEffectiveState::Enabled => SwitchState::On,
        FeatureEffectiveState::Enabling => {
            if state.requested_enabled {
                SwitchState::DisabledOn
            } else {
                SwitchState::DisabledOff
            }
        }
        // A rejected or impossible-to-order acknowledgement is neither On nor
        // Off. In particular, never paint the switch On merely because the
        // requested preference was persisted before runtime application failed.
        FeatureEffectiveState::Rejected | FeatureEffectiveState::Stale => SwitchState::Mixed,
        FeatureEffectiveState::Unavailable => {
            if state.requested_enabled {
                SwitchState::DisabledOn
            } else {
                SwitchState::DisabledOff
            }
        }
    }
}

impl SettingsApp {
    pub(super) fn optional_features_components(
        &self,
    ) -> impl nickel_ui::Component<SettingsMessage> {
        let theme = self.ui_theme();
        let state = &self.codex_feature;
        let switch_state = codex_switch_state(state);
        let switch_action = match switch_state {
            SwitchState::On => Some(SettingsMessage::SetCodexEnabled(false)),
            SwitchState::Off => Some(SettingsMessage::SetCodexEnabled(true)),
            SwitchState::Mixed => Some(SettingsMessage::SetCodexEnabled(false)),
            SwitchState::MixedUnavailable | SwitchState::DisabledOff | SwitchState::DisabledOn => {
                None
            }
        };
        let status = match state.effective {
            FeatureEffectiveState::Disabled => "Off",
            FeatureEffectiveState::Enabling | FeatureEffectiveState::Stale => "Starting Codex…",
            FeatureEffectiveState::Enabled => "On",
            FeatureEffectiveState::Unavailable => "Codex is unavailable",
            FeatureEffectiveState::Rejected => "Codex could not start",
        };
        let confirmation = if self.codex_disable_confirmation {
            AnyView::new(
                SettingsRow::new(
                    theme,
                    format!(
                        "Close {} built-in Codex window(s) and disable?",
                        self.optional_feature_runtime.active_windows
                    ),
                    "External Codex clients and upstream conversation history are not affected.",
                )
                .trailing(ui! { <Row width={190.0} gap={8.0}>
                    {Button::semantic(theme, SettingsMessage::ConfirmDisableCodex,
                        "Close & disable", ButtonPresentation::Primary).width(118.0)}
                    {Button::semantic(theme, SettingsMessage::CancelDisableCodex,
                        "Cancel", ButtonPresentation::Quiet).width(64.0)}
                </Row> }),
            )
        } else {
            AnyView::new(ui! { <Column /> })
        };
        let codex = SettingsCard::titled(
            theme,
            self.localizer.text("ui-pages-codex"),
            self.localizer
                .text("ui-pages-use-codex-projects-and-conversations-in-nickel"),
        )
        .child(
            SettingsRow::new(theme, "Enable Codex", status).trailing(
                Switch::with_state_action(switch_state, switch_action, theme)
                    .id("optional-feature-codex-enabled")
                    .accessibility_label("Enable Codex integration"),
            ),
        )
        .child(confirmation)
        .child(
            if matches!(
                state.effective,
                FeatureEffectiveState::Unavailable | FeatureEffectiveState::Rejected
            ) {
                AnyView::new(Button::semantic(
                    theme,
                    SettingsMessage::RetryCodexProbe,
                    "Try again",
                    ButtonPresentation::Secondary,
                ))
            } else {
                AnyView::new(ui! { <Column /> })
            },
        );
        use nickel_core::on_screen_keyboard::KeyboardPreference;
        let preference = self.optional_features.on_screen_keyboard;
        let editable = !self
            .keyboard_runtime
            .as_ref()
            .is_some_and(|runtime| runtime.environment_override);
        let mode = RadioGroup::new([
            RadioOption::new(
                theme,
                SettingsMessage::SetOnScreenKeyboard(KeyboardPreference::Automatic),
                "Automatic",
                preference == KeyboardPreference::Automatic,
            )
            .description("Enable when a touchscreen is connected.")
            .enabled(editable),
            RadioOption::new(
                theme,
                SettingsMessage::SetOnScreenKeyboard(KeyboardPreference::Enabled),
                "On",
                preference == KeyboardPreference::Enabled,
            )
            .description("Available even without a touchscreen.")
            .enabled(editable),
            RadioOption::new(
                theme,
                SettingsMessage::SetOnScreenKeyboard(KeyboardPreference::Disabled),
                "Off",
                preference == KeyboardPreference::Disabled,
            )
            .description("Hide the keyboard and its tray control.")
            .enabled(editable),
        ])
        .id("on-screen-keyboard-mode");
        let status = if let Some(error) = &self.keyboard_error {
            format!("Could not save: {error}")
        } else if let Some(runtime) = &self.keyboard_runtime {
            if runtime.generation != self.optional_features.on_screen_keyboard_generation {
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
        let keyboard = SettingsCard::titled(
            theme,
            self.localizer.text("ui-pages-on-screen-keyboard"),
            self.localizer
                .text("ui-pages-type-with-touch-a-controller-or-a-mouse"),
        )
        .child(mode)
        .child(SettingsRow::new(theme, "Current state", status));
        nickel_ui::VerticalScroll::new(SettingsMessage::OptionalFeaturesScroll, 0.0)
            .grow(1.0)
            .theme(theme)
            .child(
                Column::new()
                    .fill_width()
                    .gap(16.0)
                    .padding(Insets {
                        top: 0.0,
                        right: 12.0,
                        bottom: 24.0,
                        left: 0.0,
                    })
                    .child(keyboard.shrink(0.0))
                    .child(codex.shrink(0.0)),
            )
    }

    pub(super) fn default_apps_components(&self) -> impl nickel_ui::Component<SettingsMessage> {
        let theme = self.ui_theme();
        let palette = self.palette();
        let rows = self.default_apps.iter().enumerate().map(|(index, row)| {
            let current = row
                .snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.effective.as_ref())
                .map(|handler| handler.name.clone())
                .unwrap_or_else(|| "No default".into());
            // The handler name already communicates the ordinary state. Reserve the
            // supporting line for actionable outcomes and failures instead of repeating
            // operating-system ownership beneath every compact row.
            let detail = row.status.clone().unwrap_or_default();
            ui! {
                <Container background={palette.surface} padding={Insets { top: 2.0, right: 4.0, bottom: 2.0, left: 4.0 }}>
                    {SettingsRow::new(theme, row.label.clone(), detail).compact().trailing(
                        Button::semantic(
                            theme,
                            SettingsMessage::ToggleDefaultAppSelect(index),
                            current,
                            ButtonPresentation::Quiet,
                        )
                        .id(format!("default-app-{index}"))
                        .width(220.0)
                    )}
                </Container>
            }
        });
        let target_query = self.default_app_target_query.trim().to_lowercase();
        let matching_targets = self
            .default_app_targets
            .iter()
            .filter(|target| {
                (target_query.is_empty()
                    || target.platform_key().to_lowercase().contains(&target_query))
                    && self
                        .default_app_target_family
                        .is_none_or(|family| target.family() == family)
                    && !self.default_apps.iter().any(|row| row.target == **target)
            })
            .cloned()
            .collect::<Vec<_>>();
        let target_results = if self.default_apps_loading && self.default_app_targets.is_empty() {
            AnyView::new(
                Text::new(
                    self.localizer
                        .text("ui-pages-loading-file-and-protocol-associations"),
                )
                .color(palette.muted),
            )
        } else if matching_targets.is_empty() {
            AnyView::new(
                Text::new(if self.default_app_target_status.is_some() {
                    "The operating-system association catalog is unavailable."
                } else {
                    "No additional registered file or link types match."
                })
                .color(palette.muted),
            )
        } else {
            let collection = Collection::try_new(
                CollectionState::Ready(matching_targets),
                |target| target.platform_key(),
                move |target: nickel_platform::AssociationTarget| {
                    let key = target.platform_key();
                    let kind = target.family().label();
                    SettingsRow::new(theme, key, kind).trailing(
                        Button::semantic(
                            theme,
                            SettingsMessage::BrowseDefaultAppTarget(target),
                            "Choose app",
                            ButtonPresentation::Quiet,
                        )
                        .width(112.0),
                    )
                },
            )
            .expect("platform association targets are deduplicated")
            .id("default-app-target-catalog")
            .accessibility_label("Registered file and protocol associations")
            .gap(2.0)
            .navigation_scope(NavigationScope::group())
            .presentation(CollectionPresentation::VirtualList {
                item_height: 58.0,
                offset: self.default_app_catalog_scroll_offset,
                viewport_height: 300.0,
                overscan: 116.0,
            });
            AnyView::new(
                nickel_ui::VerticalScroll::new(
                    SettingsMessage::DefaultAppsScroll(
                        self.default_app_catalog_scroll_offset.to_bits(),
                    ),
                    self.default_app_catalog_scroll_offset,
                )
                .on_scroll(default_apps_scroll_message)
                .controlled(true)
                .height(300.0)
                .id("default-app-target-scroll")
                .navigation_scope(NavigationScope::group())
                .theme(theme)
                .child(collection),
            )
        };
        let advanced = SettingsRow::new(
            theme,
            "File types and links",
            self.default_app_target_status
                .as_deref()
                .unwrap_or_default(),
        )
        .compact()
        .trailing(
            SettingsSearchField::new(
                theme,
                "default-app-advanced-target",
                &self.default_app_target_query,
                "Search file types and protocols",
                default_app_target_search_message,
            )
            .width(320.0),
        );
        let families = [
            nickel_platform::AssociationFamily::Web,
            nickel_platform::AssociationFamily::Documents,
            nickel_platform::AssociationFamily::Images,
            nickel_platform::AssociationFamily::Audio,
            nickel_platform::AssociationFamily::Video,
            nickel_platform::AssociationFamily::Archives,
            nickel_platform::AssociationFamily::OtherFiles,
            nickel_platform::AssociationFamily::Protocols,
        ];
        let family_buttons =
            std::iter::once((None, format!("All ({})", self.default_app_targets.len())))
                .chain(families.into_iter().filter_map(|family| {
                    let count = self
                        .default_app_targets
                        .iter()
                        .filter(|target| target.family() == family)
                        .count();
                    (count > 0).then_some((Some(family), format!("{} ({count})", family.label())))
                }))
                .map(|(family, label)| {
                    Button::semantic(
                        theme,
                        SettingsMessage::DefaultAppTargetFamily(family),
                        label,
                        if self.default_app_target_family == family {
                            ButtonPresentation::Primary
                        } else {
                            ButtonPresentation::Quiet
                        },
                    )
                });
        let family_filters =
            nickel_ui::Grid::auto_fit(Track::minmax(Track::px(110.0), Track::fr(1.0)))
                .gap(4.0)
                .children(family_buttons);
        ui! {
            <Column grow={1.0} padding={Insets { top: 16.0, right: 24.0, bottom: 20.0, left: 20.0 }} gap={10.0}>
                <VerticalScroll id={"default-apps-list"} on_scroll={SettingsMessage::DefaultAppsPageScroll} offset={0.0} theme={theme}>
                    <Column gap={10.0}><Column gap={2.0} children={rows} />{advanced}{family_filters}{target_results}</Column>
                </VerticalScroll>
            </Column>
        }
    }

    pub(crate) fn default_app_overlays(
        &self,
        context: ViewContext,
    ) -> Vec<FrameOverlay<SettingsMessage>> {
        let theme = self.ui_theme();
        let palette = self.palette();
        let query = self.default_app_handler_query.trim().to_lowercase();
        self.default_apps
            .iter()
            .enumerate()
            .map(|(row_index, row)| {
                let target_key = row.target.platform_key().to_lowercase();
                let target_family = row.target.family().label().to_lowercase();
                let effective_id = row
                    .snapshot
                    .as_ref()
                    .and_then(|snapshot| snapshot.effective.as_ref())
                    .map(|handler| handler.id.clone());
                let state = match &row.snapshot {
                    Some(snapshot) => {
                        let mut handlers = snapshot.handlers.clone();
                        if let Some(effective) = snapshot.effective.as_ref()
                            && !handlers.iter().any(|handler| handler.id == effective.id)
                        {
                            handlers.push(effective.clone());
                        }
                        handlers.retain(|handler| {
                            query.is_empty()
                                || handler.name.to_lowercase().contains(&query)
                                || handler.id.to_lowercase().contains(&query)
                                || handler.source.to_lowercase().contains(&query)
                                || target_key.contains(&query)
                                || target_family.contains(&query)
                        });
                        handlers.sort_by(|left, right| {
                            (Some(&left.id) != effective_id.as_ref())
                                .cmp(&(Some(&right.id) != effective_id.as_ref()))
                                .then_with(|| {
                                    left.name.to_lowercase().cmp(&right.name.to_lowercase())
                                })
                                .then_with(|| left.id.cmp(&right.id))
                        });
                        CollectionState::Ready(handlers)
                    }
                    None if row.status.is_some() => {
                        CollectionState::Error(row.status.clone().unwrap_or_default())
                    }
                    None => CollectionState::Loading,
                };
                let can_change = row.snapshot.as_ref().is_some_and(|snapshot| {
                    matches!(
                        snapshot.capability,
                        nickel_platform::AssociationCapability::DirectUserChange
                            | nickel_platform::AssociationCapability::NativeConsent
                    )
                });
                let discovery_status = match row.snapshot.as_ref() {
                    None if row.status.is_some() => row.status.clone(),
                    None => Some("Discovering compatible applications…".into()),
                    Some(snapshot) => row.status.clone().or_else(|| {
                        Some(match snapshot.capability {
                            nickel_platform::AssociationCapability::DirectUserChange => {
                                "Choose an application below.".into()
                            }
                            nickel_platform::AssociationCapability::NativeConsent => {
                                "Choosing an application opens the operating system consent flow."
                                    .into()
                            }
                            nickel_platform::AssociationCapability::ReadOnly => {
                                "The operating system reports this association as read-only.".into()
                            }
                            nickel_platform::AssociationCapability::Unsupported => {
                                "Changing this association is unsupported on this platform.".into()
                            }
                        })
                    }),
                };
                let current = effective_id.clone();
                let collection = Collection::try_new(
                    state,
                    |handler: &nickel_platform::ApplicationHandler| handler.id.clone(),
                    move |handler: nickel_platform::ApplicationHandler| {
                        let is_current = current.as_ref() == Some(&handler.id);
                        SettingsRow::new(
                            theme,
                            handler.name.clone(),
                            if is_current {
                                format!("Current • {}", handler.id)
                            } else {
                                handler.id.clone()
                            },
                        )
                        .trailing(
                            Button::semantic(
                                theme,
                                SettingsMessage::SetDefaultApp {
                                    row: row_index,
                                    handler_id: handler.id,
                                },
                                if is_current { "Current" } else { "Choose" },
                                if is_current || !can_change {
                                    ButtonPresentation::Disabled
                                } else {
                                    ButtonPresentation::Quiet
                                },
                            )
                            .width(88.0)
                            .enabled(can_change && !is_current),
                        )
                    },
                )
                .expect("application handler identities are unique")
                .id(format!("default-app-handler-list-{row_index}"))
                .accessibility_label(format!("Applications for {}", row.label))
                .item_label(|handler| handler.name.clone())
                .empty_label("No installed applications match")
                .loading_label("Loading installed applications")
                .error_prefix("Applications could not be loaded: ")
                .gap(2.0)
                .navigation_scope(NavigationScope::group())
                .reveal_on_focus(&context)
                .presentation(CollectionPresentation::VirtualList {
                    item_height: 58.0,
                    offset: self.default_app_handler_scroll_offset,
                    viewport_height: 320.0,
                    overscan: 116.0,
                });
                let results = nickel_ui::VerticalScroll::new(
                    SettingsMessage::DefaultAppHandlerScroll(
                        self.default_app_handler_scroll_offset.to_bits(),
                    ),
                    self.default_app_handler_scroll_offset,
                )
                .on_scroll(default_app_handler_scroll_message)
                .controlled(true)
                .height(320.0)
                .id(format!("default-app-handler-scroll-{row_index}"))
                .navigation_scope(NavigationScope::group())
                .theme(theme)
                .child(collection);
                let mut content = Column::new()
                    .gap(8.0)
                    .padding(Insets::all(10.0))
                    .background(palette.surface);
                if let Some(status) = discovery_status {
                    content = content.child(Text::new(status).color(palette.muted));
                }
                let content = content
                    .child(SettingsSearchField::new(
                        theme,
                        format!("default-app-handler-search-{row_index}"),
                        &self.default_app_handler_query,
                        "Search installed applications",
                        default_app_handler_search_message,
                    ))
                    .child(results);
                Popover::new(
                    format!("default-app-picker-{row_index}"),
                    OverlayAnchor::Node(UiId::from(format!("default-app-{row_index}"))),
                    format!("Choose an application for {}", row.label),
                    Size::new(520.0, 392.0),
                    OverlayStyle {
                        background: palette.surface,
                        foreground: palette.text,
                        border: palette.muted,
                        selected: palette.accent_soft,
                        radius: 10,
                    },
                    content,
                )
                .focus(nickel_ui::OverlayFocusPolicy::FirstItem)
                .focus_return(UiId::from(format!("default-app-{row_index}")))
                .into()
            })
            .collect()
    }

    pub(super) fn display_components(
        &self,
        content_width: f32,
    ) -> impl nickel_ui::Component<SettingsMessage> {
        let palette = self.palette();
        let theme = self.ui_theme();
        let selected = &self.displays[self.selected];
        let identify = Button::semantic(
            theme,
            SettingsMessage::DisplayIdentify,
            self.localizer.text("settings-display-identify"),
            ButtonPresentation::Secondary,
        )
        .max_lines(3);
        let make_primary = Button::semantic(
            theme,
            SettingsMessage::DisplayPrimary,
            self.localizer.text("settings-display-make-primary"),
            ButtonPresentation::Secondary,
        )
        .max_lines(3);
        let enabled = SettingsRow::new(theme, "Display enabled", "")
            .trailing(
                Switch::new(selected.enabled, SettingsMessage::DisplayEnabled, theme)
                    .id("display-enabled")
                    .accessibility_label("Display enabled"),
            )
            .compact();
        let scale = SliderField::new(
            theme,
            "Scale",
            "",
            format!("{}%", selected.scale.units() * 100 / 120),
            (selected.scale.units().saturating_sub(60) as f32 / 420.0).clamp(0.0, 1.0),
            display_scale_message,
        )
        .id("display-scale")
        .compact();
        let mut resolutions = selected
            .modes
            .iter()
            .map(|mode| (mode.width, mode.height))
            .collect::<Vec<_>>();
        resolutions.sort_unstable_by(|left, right| right.cmp(left));
        resolutions.dedup();
        let resolution = SelectField::new(
            theme,
            "Resolution",
            "",
            SettingsMessage::ToggleDisplayResolutionSelect,
            format!("{} × {}", selected.mode.width, selected.mode.height),
            resolutions.into_iter().map(|(width, height)| {
                (
                    format!("{width} × {height}"),
                    SettingsMessage::SetDisplayResolution { width, height },
                )
            }),
            self.display_resolution_select_expanded,
        )
        .id("display-resolution")
        .compact();
        let mut refresh_rates = selected
            .modes
            .iter()
            .filter(|mode| mode.width == selected.mode.width && mode.height == selected.mode.height)
            .map(|mode| mode.refresh_millihz)
            .collect::<Vec<_>>();
        refresh_rates.sort_unstable_by(|left, right| right.cmp(left));
        refresh_rates.dedup();
        let refresh_rate = SelectField::new(
            theme,
            "Refresh rate",
            "",
            SettingsMessage::ToggleDisplayRefreshSelect,
            format!(
                "{:.2} Hz",
                f64::from(selected.mode.refresh_millihz) / 1000.0
            ),
            refresh_rates.into_iter().map(|refresh| {
                (
                    format!("{:.2} Hz", f64::from(refresh) / 1000.0),
                    SettingsMessage::SetDisplayRefresh(refresh),
                )
            }),
            self.display_refresh_select_expanded,
        )
        .id("display-refresh-rate")
        .compact();
        let app_scale_units = match self.application_scale_policy {
            ApplicationScalePolicy::Custom(scale) => scale.units(),
            _ => 120,
        };
        let application_scale_policy_choices = RadioGroup::new([
            RadioOption::new(
                theme,
                SettingsMessage::ApplicationScaleFollow,
                "Follow Nickel",
                self.application_scale_policy == ApplicationScalePolicy::FollowNickel,
            )
            .compact(),
            RadioOption::new(
                theme,
                SettingsMessage::ApplicationScaleUnchanged,
                "Leave unchanged",
                self.application_scale_policy == ApplicationScalePolicy::Unchanged,
            )
            .compact(),
            RadioOption::new(
                theme,
                SettingsMessage::SetApplicationScale(
                    app_scale_units.saturating_sub(60).min(420) / 30,
                ),
                "Custom",
                matches!(
                    self.application_scale_policy,
                    ApplicationScalePolicy::Custom(_)
                ),
            )
            .compact(),
        ])
        .id("application-scale-policy");
        let app_scale = SettingsCard::titled(
            theme,
            self.localizer.text("ui-pages-application-compatibility-scale"),
            self.localizer.text("ui-pages-toolkit-scale-can-differ-from-display-scale-applications-may-need-a-restart"),
        )
        .id("application-scale")
        .child(application_scale_policy_choices)
        .child(
            SliderField::new(
                theme,
                "Custom application scale",
                "",
                format!("{}%", app_scale_units * 100 / 120),
                (app_scale_units.saturating_sub(60) as f32 / 420.0).clamp(0.0, 1.0),
                application_scale_message,
            )
            .id("application-custom-scale")
            .compact(),
        )
        .child(nickel_ui::Text::new(&self.toolkit_scale_status).color(palette.muted));
        let apply = Button::semantic(
            theme,
            SettingsMessage::DisplayApply,
            self.localizer.text("settings-display-apply"),
            ButtonPresentation::Primary,
        )
        .max_lines(3);
        let confirmation: AnyView<SettingsMessage> = if self.pending_display_revert.is_some() {
            AnyView::new(
                Row::new()
                    .gap(8.0)
                    .child(Button::semantic(
                        theme,
                        SettingsMessage::DisplayKeep,
                        "Keep",
                        ButtonPresentation::Primary,
                    ))
                    .child(Button::semantic(
                        theme,
                        SettingsMessage::DisplayRevert,
                        "Revert",
                        ButtonPresentation::Secondary,
                    )),
            )
        } else {
            AnyView::new(nickel_ui::Container::new())
        };
        let compact_cards = content_width < 520.0;
        let mut display_order = (0..self.displays.len()).collect::<Vec<_>>();
        display_order.sort_by_key(|index| (*index == self.selected) as u8);
        let display_cards = display_order.into_iter().map(|index| {
            let display = &self.displays[index];
            let selected = index == self.selected;
            let detail = if display.enabled {
                display.detail.clone()
            } else {
                format!("{}  DISABLED", display.detail)
            };
            let border_color = if !display.enabled {
                palette.muted
            } else if display.primary {
                palette.accent
            } else {
                palette.muted
            };
            let border_width = if display.primary && display.enabled {
                4.0
            } else {
                2.0
            };
            let card = ui! {
                <Container id={format!("display-card-{index}")}
                    width={if compact_cards { (content_width - 120.0).max(120.0) } else { display.rect.w as f32 }}
                    height={if compact_cards { 140.0 } else { display.rect.h as f32 }}
                    min_width={if compact_cards { 120.0 } else { 160.0 }} min_height={if compact_cards { 140.0 } else { 80.0 }}
                    background={if !display.enabled { palette.background }
                        else if selected { palette.accent_soft } else { palette.surface }}
                    border={(border_color, border_width)} radius={theme.radii.card}
                    padding={Insets::all(12.0)}
                    on_drag={(SettingsMessage::SelectDisplay(index), display_drag_message)}
                    on_press={SettingsMessage::SelectDisplay(index)}
                    semantic_role={SemanticRole::Button}
                    accessibility_label={format!("{} display, {}", display.name, detail)}
                    accessibility_state={if selected { "selected" } else { "not selected" }}>
                    <Column gap={4.0}>
                        <Text color={palette.text} wrap={true}>{&display.name}</Text>
                        <Text scale={0.9} color={palette.muted} wrap={true}>{detail}</Text>
                        <Text scale={0.9} bold={true} color={palette.accent}>
                            {if display.primary { "PRIMARY" } else { "" }}
                        </Text>
                    </Column>
                </Container>
            };
            if compact_cards {
                card
            } else {
                card.position(Point {
                    x: (display.rect.x - self.display_plane.x - 12) as f32,
                    y: (display.rect.y - self.display_plane.y - 12) as f32,
                })
            }
        }).collect::<Vec<_>>();
        let display_layout = if compact_cards {
            AnyView::new(Column::new().gap(12.0).children(display_cards))
        } else {
            AnyView::new(
                Layer::new()
                    .fill_width()
                    .height(216.0)
                    .children(display_cards),
            )
        };
        ui! {
            <Column grow={1.0} padding={Insets {
                top: 20.0, right: 32.0, bottom: 20.0, left: 20.0,
            }}>
                <VerticalScroll id={"display-page-scroll"} on_scroll={SettingsMessage::DisplayScroll}
                    grow={1.0}
                    offset={0.0} theme={theme}>
                    <Column gap={8.0}>
                        <Container id={"display-plane"} min_height={240.0}
                            background={palette.surface} border={(palette.muted, 1.0)}
                            padding={Insets::all(12.0)} align_items={nickel_ui::Align::Center}
                            justify_content={nickel_ui::Justify::Center}
                            semantic_role={SemanticRole::TabPanel}
                            accessibility_label={"Display arrangement"}>
                            {display_layout}
                        </Container>
                        <Container background={palette.surface} border={(palette.muted, 1.0)}
                            padding={Insets::all(8.0)}>
                            <Column gap={6.0}>
                                <Row gap={12.0}>
                                    <Column grow={1.0} gap={3.0}>
                                        <Text color={palette.text} wrap={true}>{&selected.name}</Text>
                                        <Text scale={0.9} color={palette.muted} wrap={true}>{&selected.detail}</Text>
                                    </Column>
                                    <Text bold={true} wrap={true} color={if selected.primary { palette.accent } else { palette.muted }}>
                                        {if selected.primary {
                                            self.localizer.text("settings-display-primary")
                                        } else { String::new() }}
                                    </Text>
                                </Row>
                                {enabled}
                                {resolution}
                                {refresh_rate}
                                {scale}
                                <Grid columns={GridColumnSpec::AutoFit(Track::minmax(120.0, Track::fr(1.0)))} gap={12.0}>
                                    {identify}{make_primary}{apply}
                                </Grid>
                                {confirmation}
                            </Column>
                        </Container>
                        <Text color={if self.applied { palette.complement } else { palette.muted }} wrap={true}>
                            {&self.status}
                        </Text>
                        {app_scale}
                    </Column>
                </VerticalScroll>
            </Column>
        }
    }

    pub(super) fn network_components(&self) -> impl nickel_ui::Component<SettingsMessage> {
        let palette = self.palette();
        let theme = self.ui_theme();
        let wifi_cards = self
            .wifi_networks
            .iter()
            .enumerate()
            .map(|(index, network)| {
                let detail = if network.connected {
                    self.localizer.number(
                        "settings-network-connected-signal",
                        "signal",
                        i64::from(network.signal),
                    )
                } else if !network.saved {
                    self.localizer.number(
                        if network.secure {
                            "settings-network-secured-signal"
                        } else {
                            "settings-network-open-signal"
                        },
                        "signal",
                        i64::from(network.signal),
                    )
                } else {
                    self.localizer.number(
                        "settings-network-connect-action",
                        "signal",
                        i64::from(network.signal),
                    )
                };
                ui! {
                    <Container id={format!("wifi-network-{index}")} height={44.0}
                        background={palette.surface}
                        hover_background={palette.surface_hover}
                        pressed_background={palette.surface_hover}
                        border={(if network.connected { palette.accent } else { palette.muted },
                            if network.connected { 2.0 } else { 1.0 })}
                        padding={Insets { top: 12.0, right: 14.0, bottom: 8.0, left: 14.0 }}
                        on_press={SettingsMessage::WifiNetwork(index)}
                        enabled={self.pending_wifi_profile.is_none()}
                        semantic_role={SemanticRole::Button}
                        accessibility_label={format!("{}, {}", network.profile, detail)}
                        accessibility_state={if network.connected { "connected" } else { "not connected" }}>
                        <Row>
                            <Text color={palette.text} width={316.0}>{&network.profile}</Text>
                            <Text scale={1.0} color={if network.connected { palette.complement } else { palette.muted }}>
                                {detail}
                            </Text>
                        </Row>
                    </Container>
                }
            });
        let adapter_cards = self.network_adapters.iter().map(|adapter| {
            let status = if adapter.connected {
                if adapter.speed > 0 {
                    self.localizer.number(
                        "settings-network-connected-speed",
                        "speed",
                        (adapter.speed / 1_000_000) as i64,
                    )
                } else {
                    self.localizer.text("settings-network-connected")
                }
            } else {
                self.localizer.text("settings-network-disconnected")
            };
            ui! {
                <Container height={72.0} background={palette.surface} border={(palette.muted, 1.0)}
                    padding={Insets { top: 11.0, right: 14.0, bottom: 8.0, left: 14.0 }}>
                    <Column gap={7.0}>
                        <Text color={palette.text}>{&adapter.name}</Text>
                        <Row>
                            <Text color={if adapter.connected { palette.complement } else { palette.muted }}>{status}</Text>
                            <Text scale={1.0} color={palette.muted}>{&adapter.description}</Text>
                        </Row>
                    </Column>
                </Container>
            }
        });
        let wifi_list = if self.wifi_networks.is_empty() {
            ui! { <Column><Text scale={1.0} color={palette.muted}>{&self.wifi_status}</Text></Column> }
        } else {
            ui! { <Column gap={8.0} children={wifi_cards} /> }
        };
        let adapter_list = if self.network_adapters.is_empty() {
            ui! {
                <Column><Text scale={1.0} color={palette.muted}>
                    {self.localizer.text("settings-network-no-adapters")}
                </Text></Column>
            }
        } else {
            ui! { <Column gap={12.0} children={adapter_cards} /> }
        };
        let wifi_power_available = self.network_available && cfg!(target_os = "linux");
        let wifi_switch_state = if !wifi_power_available && self.wifi_enabled {
            SwitchState::DisabledOn
        } else if !wifi_power_available {
            SwitchState::DisabledOff
        } else if self.wifi_power_rx.is_some() && self.wifi_enabled {
            SwitchState::DisabledOn
        } else if self.wifi_power_rx.is_some() {
            SwitchState::DisabledOff
        } else if self.wifi_enabled {
            SwitchState::On
        } else {
            SwitchState::Off
        };
        let wifi_label = self.localizer.text("settings-network-wifi");
        let wifi_power = SettingsRow::new(theme, wifi_label.clone(), self.wifi_status.clone())
            .trailing(
                Switch::with_state(
                    wifi_switch_state,
                    (wifi_power_available && self.wifi_power_rx.is_none())
                        .then_some(wifi_power_message as fn(bool) -> SettingsMessage),
                    theme,
                )
                .id("network-wifi-power")
                .accessibility_label(wifi_label),
            );
        let content = ui! {
            <Column gap={12.0}>
                {wifi_power}
                <Row height={26.0}>
                    <Text color={palette.text} width={308.0}>{self.localizer.text("settings-network-visible-wifi")}</Text>
                </Row>
                {wifi_list}
                <Text color={palette.text} height={18.0}>{self.localizer.text("settings-network-adapters")}</Text>
                {adapter_list}
            </Column>
        };

        ui! {
            <Column grow={1.0} padding={Insets {
                top: 20.0, right: 40.0, bottom: 20.0, left: 20.0,
            }}>
                <VerticalScroll id={"network-list"} on_scroll={SettingsMessage::NetworkScroll}
                    offset={0.0} theme={theme}>{content}</VerticalScroll>
            </Column>
        }
    }

    pub(super) fn bluetooth_components(&self) -> AnyView<SettingsMessage> {
        let palette = self.palette();
        let theme = self.ui_theme();
        let pairing = self.page == SettingsPage::BluetoothPair;
        let operation_pending = self.bluetooth_operation_rx.is_some();
        let device_list = self
            .bluetooth
            .devices
            .iter()
            .enumerate()
            .filter(|(_, device)| {
                if pairing {
                    !device.paired && !device.connected
                } else {
                    device.paired || device.connected
                }
            })
            .fold(SettingsListCard::new(theme), |list, (index, device)| {
                let status = if device.connected {
                    self.localizer.text("settings-bluetooth-connected")
                } else if device.paired {
                    self.localizer.text("settings-bluetooth-paired")
                } else {
                    self.localizer.text("settings-bluetooth-available")
                };
                let detail = if pairing {
                    let kind = device.kind.as_deref().and_then(bluetooth_kind_icon);
                    let signal = device.signal_dbm.map(bluetooth_signal_label);
                    [kind, signal]
                        .into_iter()
                        .flatten()
                        .collect::<Vec<_>>()
                        .join(" · ")
                } else {
                    device
                        .battery_percent
                        .map(|percent| format!("{percent}%"))
                        .unwrap_or_default()
                };
                let supporting = if detail.is_empty() {
                    status
                } else {
                    format!("{status} · {detail}")
                };
                let available =
                    self.bluetooth.available && self.bluetooth.powered && !operation_pending;
                let action = if device.connected {
                    self.localizer.text("settings-bluetooth-disconnect")
                } else if !device.paired {
                    self.localizer.text("settings-bluetooth-pair")
                } else {
                    self.localizer.text("settings-bluetooth-connect")
                };
                list.row(
                    SettingsRow::new(theme, &device.name, supporting)
                        .id(format!("bluetooth-device-{index}"))
                        .trailing(
                            Button::semantic(
                                theme,
                                SettingsMessage::BluetoothDevice(index),
                                action,
                                if available {
                                    ButtonPresentation::Secondary
                                } else {
                                    ButtonPresentation::Disabled
                                },
                            )
                            .id(format!("bluetooth-device-{index}-action")),
                        ),
                )
            });

        let adapter_status = if let Some(operation) = &self.bluetooth_operation {
            match operation {
                BluetoothOperation::SetPower(true) => {
                    self.localizer.text("settings-bluetooth-powering-on")
                }
                BluetoothOperation::SetPower(false) => {
                    self.localizer.text("settings-bluetooth-powering-off")
                }
                BluetoothOperation::SetDiscovery(true) => {
                    self.localizer.text("settings-bluetooth-discovery-starting")
                }
                BluetoothOperation::SetDiscovery(false) => {
                    self.localizer.text("settings-bluetooth-discovery-stopping")
                }
                BluetoothOperation::ToggleDevice(device) => {
                    let name = self
                        .bluetooth
                        .devices
                        .iter()
                        .find(|candidate| candidate.id == *device)
                        .map(|candidate| candidate.name.as_str())
                        .unwrap_or("device");
                    self.localizer
                        .value("settings-bluetooth-device-updating", "device", name)
                }
            }
        } else if let Some(error) = &self.bluetooth_status {
            error.clone()
        } else if !self.bluetooth.available {
            self.localizer
                .text("settings-bluetooth-service-unavailable")
        } else if self.bluetooth.powered {
            self.localizer.text("settings-bluetooth-on")
        } else {
            self.localizer.text("settings-bluetooth-off")
        };
        let discoverability = if pairing && self.bluetooth.discovering {
            self.localizer.text("settings-bluetooth-discovery-stop")
        } else if pairing {
            self.localizer.text("settings-bluetooth-discovery-start")
        } else {
            self.localizer.text("settings-bluetooth-pair-devices")
        };
        let discovery_available =
            self.bluetooth.available && self.bluetooth.powered && !operation_pending;
        let discovery_button = Button::semantic(
            self.ui_theme(),
            if pairing {
                SettingsMessage::BluetoothDiscovery
            } else {
                SettingsMessage::OpenBluetoothPairing
            },
            discoverability,
            if discovery_available {
                ButtonPresentation::Secondary
            } else {
                ButtonPresentation::Disabled
            },
        )
        .width(150.0);
        let visible_device_count = self
            .bluetooth
            .devices
            .iter()
            .filter(|device| {
                if pairing {
                    !device.paired && !device.connected
                } else {
                    device.paired || device.connected
                }
            })
            .count();
        let device_list = if visible_device_count == 0 {
            AnyView::new(
                ui! { <Column><Text color={palette.muted}>{if self.bluetooth.available {
                    self.localizer.text("settings-bluetooth-no-devices")
                } else {
                    self.localizer
                        .text("settings-bluetooth-service-unavailable")
                }}</Text></Column> },
            )
        } else {
            AnyView::new(device_list.id("bluetooth-devices"))
        };
        let bluetooth_switch_state = if !self.bluetooth.available || operation_pending {
            if self.bluetooth.powered {
                SwitchState::DisabledOn
            } else {
                SwitchState::DisabledOff
            }
        } else if self.bluetooth.powered {
            SwitchState::On
        } else {
            SwitchState::Off
        };
        let bluetooth_label = self.localizer.text("settings-bluetooth-enabled");
        let bluetooth_power = SettingsRow::new(
            theme,
            bluetooth_label.clone(),
            if self.bluetooth.adapter_name.is_empty() {
                self.localizer.text("settings-bluetooth-adapter-unnamed")
            } else {
                self.bluetooth.adapter_name.clone()
            },
        )
        .trailing(
            Switch::with_state(
                bluetooth_switch_state,
                (self.bluetooth.available && !operation_pending)
                    .then_some(bluetooth_power_message as fn(bool) -> SettingsMessage),
                theme,
            )
            .id("bluetooth-power")
            .accessibility_label(bluetooth_label),
        );
        let content = if pairing {
            AnyView::new(ui! {
                <Column gap={12.0}>
                    <Text scale={1.0} color={palette.muted}>{adapter_status}</Text>
                    <Row height={36.0}>
                        <Text color={palette.text} grow={1.0}>{self.localizer.text("settings-bluetooth-nearby-devices")}</Text>
                        {discovery_button}
                    </Row>
                    {device_list}
                </Column>
            })
        } else {
            AnyView::new(ui! {
                <Column gap={12.0}>
                    {bluetooth_power}
                    <Text scale={1.0} color={palette.muted}>{adapter_status}</Text>
                    <Row height={36.0}>
                        <Text width={390.0} color={palette.text}>{self.localizer.text("settings-bluetooth-devices")}</Text>
                        {discovery_button}
                    </Row>
                    {device_list}
                </Column>
            })
        };

        let content_padding = if pairing {
            Insets::all(0.0)
        } else {
            Insets {
                top: 20.0,
                right: 40.0,
                bottom: 20.0,
                left: 20.0,
            }
        };
        AnyView::new(ui! {
            <Column grow={1.0} padding={content_padding}>
                <VerticalScroll id={"bluetooth-list"} on_scroll={SettingsMessage::BluetoothScroll}
                    offset={0.0} theme={theme}>{content}</VerticalScroll>
            </Column>
        })
    }

    pub(super) fn bluetooth_pairing_view(&self) -> AnyView<SettingsMessage> {
        let theme = self.ui_theme();
        AnyView::new(ui! {
            <Column grow={1.0} padding={Insets::all(20.0)} gap={12.0}>
                {PageHeader::new(
                    theme,
                    self.localizer.text("settings-bluetooth-pair-title"),
                    self.localizer.text("settings-bluetooth-pair-subtitle"),
                )}
                {self.bluetooth_components()}
            </Column>
        })
    }

    pub(super) fn bar_components(&self) -> impl nickel_ui::Component<SettingsMessage> {
        let palette = self.palette();
        let theme = self.ui_theme();
        let display_count = self.displays.len().max(1);
        let desktop_choices = (0..self.shell_settings.desktop_count).map(|index| {
            ui! {
                <Container width={64.0} height={46.0} background={palette.surface}
                    border={(if index == self.shell_settings.active_desktop { palette.accent } else { palette.muted }, 2.0)}
                    padding={Insets { top: 9.0, right: 4.0, bottom: 4.0, left: 4.0 }}>
                    <Text align={TextAlign::Center} scale={1.0}
                        color={if index == self.shell_settings.active_desktop { palette.text } else { palette.muted }}>
                        {format!("{}", index + 1)}
                    </Text>
                </Container>
            }
        });
        let bar_display_scope = RadioGroup::new([
            RadioOption::new(
                theme,
                SettingsMessage::BarPrimaryDisplay,
                self.localizer.text("settings-bar-primary-display"),
                !self.shell_settings.bar_on_all_displays,
            ),
            RadioOption::new(
                theme,
                SettingsMessage::BarAllDisplays,
                self.localizer
                    .number("settings-bar-all-displays", "count", display_count as i64),
                self.shell_settings.bar_on_all_displays,
            ),
        ])
        .id("bar-display-scope");
        let bar_window_scope = RadioGroup::new([
            RadioOption::new(
                theme,
                SettingsMessage::BarDisplayWindows,
                self.localizer.text("settings-bar-this-display"),
                !self.shell_settings.all_windows_on_every_bar,
            ),
            RadioOption::new(
                theme,
                SettingsMessage::BarAllWindows,
                self.localizer.text("settings-bar-all-windows"),
                self.shell_settings.all_windows_on_every_bar,
            ),
        ])
        .id("bar-window-scope");
        let desktop_count = SliderField::new(
            theme,
            self.localizer.text("settings-bar-desktops"),
            "The number of persistent workspaces available to the session.",
            self.localizer.number(
                "settings-bar-desktop-count",
                "count",
                i64::from(self.shell_settings.desktop_count),
            ),
            f32::from(self.shell_settings.desktop_count.saturating_sub(1))
                / f32::from(nickel_core::shell_settings::MAX_CONFIGURED_WORKSPACES - 1),
            desktop_count_message,
        )
        .id("bar-desktop-count");
        ui! {
            <Column grow={1.0} padding={Insets {
                top: 24.0, right: 40.0, bottom: 20.0, left: 20.0,
            }} gap={14.0}>
                <Text color={palette.text} height={20.0}>{self.localizer.text("settings-bar-show-on")}</Text>
                {bar_display_scope}
                <Text color={palette.text} height={20.0}>{self.localizer.text("settings-bar-window-scope")}</Text>
                {bar_window_scope}
                {desktop_count}
                <Row height={46.0} gap={8.0} children={desktop_choices} />
            </Column>
        }
    }

    pub(super) fn appearance_components(&self) -> impl nickel_ui::Component<SettingsMessage> {
        let system = nickel_platform::appearance();
        let appearance = self.shell_settings.resolve_appearance(system);
        let palette = ThemePalette::from_appearance(appearance);
        let theme = self.ui_theme();
        let hue = self.shell_settings.displayed_hue(system);
        let intensity = self.shell_settings.displayed_intensity(system);
        let preview = |preview_palette: ThemePalette| {
            Surface::new(theme, SurfaceRole::Raised)
                .height(82.0)
                .radius(theme.radii.control)
                .padding(Insets::all(8.0))
                .child(ui! {
                    <Row gap={6.0}>
                        <Container width={22.0} background={preview_palette.panel} radius={3.0} />
                        <Column grow={1.0} gap={6.0}>
                            <Container height={12.0} background={preview_palette.surface_hover} radius={3.0} />
                            <Container height={28.0} background={preview_palette.background} radius={3.0} />
                        </Column>
                    </Row>
                })
        };
        let light_preview = ThemePalette::from_appearance(Appearance {
            mode: ThemeMode::Light,
            accent: appearance.accent,
            intensity: appearance.intensity,
        });
        let dark_preview = ThemePalette::from_appearance(Appearance {
            mode: ThemeMode::Dark,
            accent: appearance.accent,
            intensity: appearance.intensity,
        });
        let mode_choices = [
            ChoiceCard::new(
                theme,
                SettingsMessage::AppearanceLight,
                self.localizer.text("settings-appearance-light"),
                self.shell_settings.theme == ThemePreference::Light,
                preview(light_preview),
            )
            .id("appearance-mode-light"),
            ChoiceCard::new(
                theme,
                SettingsMessage::AppearanceDark,
                self.localizer.text("settings-appearance-dark"),
                self.shell_settings.theme == ThemePreference::Dark,
                preview(dark_preview),
            )
            .id("appearance-mode-dark"),
            ChoiceCard::new(
                theme,
                SettingsMessage::AppearanceSystem,
                self.localizer.text("settings-appearance-automatic"),
                self.shell_settings.theme == ThemePreference::System,
                Surface::new(theme, SurfaceRole::Raised)
                    .height(82.0)
                    .radius(theme.radii.control)
                    .padding(Insets::all(8.0))
                    .child(ui! {
                        <Row height={66.0} gap={3.0}>
                            <Container grow={1.0} background={light_preview.background} radius={3.0} />
                            <Container grow={1.0} background={dark_preview.background} radius={3.0} />
                        </Row>
                    }),
            )
            .id("appearance-mode-system"),
        ];
        let preset_hues = [224_u16, 188, 154, 78, 38, 16, 340, 305];
        let swatches = preset_hues.into_iter().map(|preset| {
            let [red, green, blue] = accent_from_hue(preset);
            ColorSwatch::color(
                theme,
                SettingsMessage::SetAccentHue(preset),
                (u32::from(red) << 16) | (u32::from(green) << 8) | u32::from(blue),
                hue.abs_diff(preset) < 3,
            )
        });
        let wallpaper_name = self
            .wallpaper_settings
            .image
            .as_deref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.localizer.text("settings-wallpaper-none"));
        let position_options = [
            ("settings-wallpaper-fill", WallpaperPosition::Fill),
            ("settings-wallpaper-fit", WallpaperPosition::Fit),
            ("settings-wallpaper-stretch", WallpaperPosition::Stretch),
            ("settings-wallpaper-center", WallpaperPosition::Center),
            ("settings-wallpaper-tile", WallpaperPosition::Tile),
            ("settings-wallpaper-span", WallpaperPosition::Span),
        ]
        .into_iter()
        .map(|(label, position)| {
            (
                self.localizer.text(label),
                SettingsMessage::WallpaperPosition(position),
            )
        });
        let position_label = self.localizer.text(match self.wallpaper_settings.position {
            WallpaperPosition::Fill => "settings-wallpaper-fill",
            WallpaperPosition::Fit => "settings-wallpaper-fit",
            WallpaperPosition::Stretch => "settings-wallpaper-stretch",
            WallpaperPosition::Center => "settings-wallpaper-center",
            WallpaperPosition::Tile => "settings-wallpaper-tile",
            WallpaperPosition::Span => "settings-wallpaper-span",
        });
        let wallpaper_preview = self
            .wallpaper_preview
            .as_ref()
            .map(|image| {
                PreviewTile::new(
                    theme,
                    Image::new(1, image.clone())
                        .fit(ImageFit::Cover)
                        .width(124.0)
                        .height(96.0),
                )
            })
            .unwrap_or_else(|| {
                PreviewTile::unavailable(theme, self.localizer.text("settings-wallpaper-none"))
            })
            .width(124.0)
            .height(96.0);
        let wallpaper_dimensions = self
            .wallpaper_dimensions
            .map(|(width, height)| format!("{width} × {height}"));
        let mode_group = ChoiceCardGroup::new(mode_choices);
        let swatch_row = ui! {
            <Row height={44.0} gap={10.0} children={swatches}>
                {ColorSwatch::custom(theme, SettingsMessage::SetAccentHue(hue))}
            </Row>
        };
        let mode_card = SettingsCard::titled(
            theme,
            self.localizer.text("settings-appearance-mode"),
            self.localizer.text("settings-appearance-mode-description"),
        )
        .id("appearance-mode-card")
        .child(mode_group);
        let accent_card = SettingsCard::titled(
            theme,
            self.localizer.text("settings-appearance-accent"),
            self.localizer
                .text("settings-appearance-accent-description"),
        )
        .id("appearance-accent-card")
        .child(swatch_row);
        let wallpaper_card = SettingsCard::titled(
            theme,
            self.localizer.text("settings-wallpaper-image"),
            self.localizer.text("settings-wallpaper-description"),
        )
        .id("appearance-wallpaper-card")
        .child({
            let choose = Button::semantic(
                theme,
                SettingsMessage::WallpaperChoose,
                self.localizer.text("settings-wallpaper-choose"),
                ButtonPresentation::Primary,
            )
            .width(168.0);
            let remove = Button::semantic(
                theme,
                SettingsMessage::WallpaperRemove,
                self.localizer.text("settings-wallpaper-remove"),
                ButtonPresentation::Secondary,
            )
            .width(100.0);
            ui! {
            <Row gap={14.0} align_items={nickel_ui::Align::Center}>
                {wallpaper_preview}
                <Column grow={1.0} gap={8.0}>
                    <Text color={palette.text}>{wallpaper_name}</Text>
                    {wallpaper_dimensions.map(|dimensions| nickel_ui::Text::new(dimensions).color(palette.muted))}
                    <Row gap={10.0}>
                        {choose}{remove}
                    </Row>
                    {self.wallpaper_status.as_ref().map(|status| SettingsStatus::new(theme, SettingsStatusKind::Error, status.clone()))}
                </Column>
            </Row>
            }
        })
        .child(SelectField::new(
            theme,
            self.localizer.text("settings-wallpaper-fit-label"),
            self.localizer.text("settings-wallpaper-fit-description"),
            SettingsMessage::ToggleWallpaperPositionSelect,
            position_label,
            position_options,
            self.wallpaper_position_select_expanded,
        ));
        let transparency_row = SettingsRow::new(
            theme,
            self.localizer.text("settings-reduce-transparency"),
            self.localizer
                .text("settings-reduce-transparency-description"),
        )
        .trailing(
            Switch::new(
                self.shell_settings.reduce_transparency,
                reduce_transparency_message,
                theme,
            )
            .id("appearance-transparency"),
        );
        let animation_label = self.localizer.text(match self.shell_settings.animations {
            AnimationLevel::Off => "settings-animations-off",
            AnimationLevel::Reduced => "settings-animations-reduced",
            AnimationLevel::Normal => "settings-animations-normal",
        });
        let animation_row = SelectField::new(
            theme,
            self.localizer.text("settings-animations"),
            self.localizer.text("settings-animations-description"),
            SettingsMessage::ToggleAnimationSelect,
            animation_label,
            [
                (
                    self.localizer.text("settings-animations-off"),
                    SettingsMessage::SetAnimationLevel(AnimationLevel::Off),
                ),
                (
                    self.localizer.text("settings-animations-reduced"),
                    SettingsMessage::SetAnimationLevel(AnimationLevel::Reduced),
                ),
                (
                    self.localizer.text("settings-animations-normal"),
                    SettingsMessage::SetAnimationLevel(AnimationLevel::Normal),
                ),
            ],
            self.animation_select_expanded,
        )
        .id("appearance-animations");
        let file_icon_provider = self.shell_settings.file_icon_provider;
        let configured_icon_theme = self.shell_settings.file_icon_theme.as_deref();
        let installed_icon_themes = nickel_platform::installed_icon_themes();
        let configured_theme_available = configured_icon_theme.is_none_or(|configured| {
            installed_icon_themes
                .iter()
                .any(|theme| theme == configured)
        });
        let selected_file_artwork = match (file_icon_provider, configured_icon_theme) {
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
            (
                "Nickel".to_owned(),
                SettingsMessage::SetFileIconProvider(FileIconPreference::Nickel),
            ),
            (
                "System".to_owned(),
                SettingsMessage::SetFileIconProvider(FileIconPreference::System),
            ),
        ];
        file_artwork_options.extend(installed_icon_themes.into_iter().map(|theme| {
            (
                format!("System — {theme}"),
                SettingsMessage::SetFileIconTheme(theme),
            )
        }));
        let file_icon_provider_row = SelectField::new(
            theme,
            "File artwork",
            "Choose Nickel artwork or icons supplied by the operating system.",
            SettingsMessage::ToggleFileIconProviderSelect,
            selected_file_artwork,
            file_artwork_options,
            self.file_icon_provider_select_expanded,
        )
        .id("appearance-file-artwork");
        let interface_card = SettingsCard::titled(
            theme,
            self.localizer.text("settings-interface-settings"),
            "",
        )
        .id("appearance-interface-card")
        .child(
            SliderField::new(
                theme,
                self.localizer.text("settings-appearance-starting-hue"),
                self.localizer.text("settings-appearance-hue-description"),
                self.localizer
                    .number("settings-appearance-hue-value", "degrees", i64::from(hue)),
                f32::from(hue) / 359.0,
                appearance_hue_message,
            )
            .id("appearance-hue"),
        )
        .child(
            SliderField::new(
                theme,
                self.localizer.text("settings-appearance-color-intensity"),
                self.localizer
                    .text("settings-appearance-intensity-description"),
                self.localizer.number(
                    "settings-appearance-intensity-value",
                    "percent",
                    i64::from(intensity),
                ),
                f32::from(intensity) / 100.0,
                appearance_intensity_message,
            )
            .id("appearance-intensity"),
        )
        .child(transparency_row)
        .child(animation_row)
        .child(file_icon_provider_row);
        let reset = Button::semantic(
            theme,
            SettingsMessage::AppearanceReset,
            self.localizer.text("settings-appearance-reset"),
            ButtonPresentation::Secondary,
        )
        .id("appearance-reset")
        .width(220.0);
        let appearance_notice = self.appearance_notice.as_ref().map(|notice| match notice {
            AppearanceNotice::Confirmation(message) => SettingsStatus::<SettingsMessage>::new(
                theme,
                SettingsStatusKind::Validation,
                message.clone(),
            ),
            AppearanceNotice::Error(message) => SettingsStatus::<SettingsMessage>::new(
                theme,
                SettingsStatusKind::Error,
                message.clone(),
            ),
        });
        let mut general = nickel_ui::Column::new()
            .gap(10.0)
            .child(mode_card)
            .child(accent_card)
            .child(wallpaper_card)
            .child(interface_card)
            .child(
                nickel_ui::Row::new()
                    .justify_content(nickel_ui::Justify::End)
                    .child(reset),
            );
        if let Some(notice) = appearance_notice {
            general = general.child(notice);
        }
        ui! {
            <Column grow={1.0} padding={Insets {
                top: 16.0, right: 24.0, bottom: 20.0, left: 20.0,
            }} gap={10.0}>
                <VerticalScroll id={"appearance-list"} on_scroll={SettingsMessage::AppearanceScroll}
                    offset={0.0} theme={theme}>{general}</VerticalScroll>
            </Column>
        }
    }

    pub(super) fn keyboard_shortcuts_components(
        &self,
    ) -> impl nickel_ui::Component<SettingsMessage> {
        let theme = self.ui_theme();
        SettingsCard::titled(
            theme,
            self.localizer.text("settings-keyboard-card-title"),
            self.localizer.text("settings-keyboard-card-description"),
        )
        .child(SettingsRow::new(
            theme,
            self.localizer.text("settings-keyboard-open-launcher"),
            "Super",
        ))
        .child(SettingsRow::new(
            theme,
            self.localizer.text("settings-keyboard-search"),
            self.localizer.text("settings-keyboard-search-value"),
        ))
        .child(SettingsRow::new(
            theme,
            self.localizer.text("settings-keyboard-navigate"),
            "Arrow keys · Tab · Shift+Tab",
        ))
        .child(SettingsRow::new(
            theme,
            self.localizer.text("settings-keyboard-activate"),
            "Enter",
        ))
        .child(SettingsRow::new(
            theme,
            self.localizer.text("settings-keyboard-back"),
            "Escape",
        ))
        .child(SettingsRow::new(
            theme,
            self.localizer.text("settings-keyboard-workspaces"),
            if cfg!(target_os = "windows") {
                self.localizer
                    .text("settings-keyboard-workspaces-unavailable")
            } else {
                self.localizer.text("settings-keyboard-workspaces-value")
            },
        ))
    }

    pub(super) fn about_components(&self) -> impl nickel_ui::Component<SettingsMessage> {
        let theme = self.ui_theme();
        let platform = format!("{} · {}", std::env::consts::OS, std::env::consts::ARCH);
        SettingsCard::titled(
            theme,
            self.localizer.text("settings-about-card-title"),
            self.localizer.text("settings-about-card-description"),
        )
        .child(SettingsRow::new(
            theme,
            self.localizer.text("settings-about-version"),
            env!("CARGO_PKG_VERSION"),
        ))
        .child(SettingsRow::new(
            theme,
            self.localizer.text("settings-about-platform"),
            platform,
        ))
    }
}
