use super::*;
use nickel_ui::{CollectionState, Column, ComponentBuilderExt, Container, Row};

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
    fn settings_plugin_recovery(
        &self,
        title: &str,
        error: String,
        extra_action: Option<(SettingsMessage, &str)>,
    ) -> AnyView<SettingsMessage> {
        let theme = self.ui_theme();
        let mut card = SettingsCard::titled(theme, title, error);
        if let Some((message, label)) = extra_action {
            card = card.child(Button::semantic(
                theme,
                message,
                label,
                ButtonPresentation::Secondary,
            ));
        }
        AnyView::new(card.child(Button::semantic(
            theme,
            SettingsMessage::Navigate(SettingsPage::Plugins),
            "Manage plugins",
            ButtonPresentation::Primary,
        )))
    }

    pub(super) fn plugins_components(&self) -> impl nickel_ui::Component<SettingsMessage> {
        let theme = self.ui_theme();
        if self.page != SettingsPage::Plugins {
            return nickel_ui::VerticalScroll::new(SettingsMessage::PluginsScroll, 0.0)
                .grow(1.0)
                .theme(theme)
                .child(Column::new());
        }
        let mut content = Column::new()
            .fill_width()
            .gap(16.0)
            .padding(Insets::all(20.0));
        if let Some(snapshot) = &self.plugin_status
            && let Some((review_id, generation)) = &self.plugin_enable_review
            && *generation == snapshot.activation_generation
            && let Some(plugin) = snapshot
                .plugins
                .iter()
                .find(|plugin| plugin.id == *review_id && !plugin.desired_enabled)
        {
            let grants = if plugin.capabilities.is_empty() {
                "No additional access".to_owned()
            } else {
                plugin.capabilities.join(", ")
            };
            let surfaces = if plugin.surfaces.is_empty() {
                "No surfaces".to_owned()
            } else {
                plugin.surfaces.join(", ")
            };
            let composition = if plugin.composition.is_empty() {
                "No extension changes".to_owned()
            } else {
                plugin.composition.join(", ")
            };
            content = content.child(
                SettingsCard::titled(
                    theme,
                    self.localizer
                        .value("settings-plugin-enable-review", "name", &plugin.name),
                    &plugin.id,
                )
                .child(SettingsRow::new(
                    theme,
                    "Publisher",
                    plugin.author.as_deref().unwrap_or("Unknown"),
                ))
                .child(SettingsRow::new(
                    theme,
                    "Version",
                    plugin.version.as_deref().unwrap_or("Unspecified"),
                ))
                .child(SettingsRow::new(theme, "Access requested", grants))
                .child(SettingsRow::new(theme, "Surfaces affected", surfaces))
                .child(SettingsRow::new(theme, "Composition changes", composition))
                .child(
                    Row::new()
                        .gap(12.0)
                        .child(Button::semantic(
                            theme,
                            SettingsMessage::CancelPluginEnable,
                            "Cancel",
                            ButtonPresentation::Quiet,
                        ))
                        .child(Button::semantic(
                            theme,
                            SettingsMessage::ConfirmPluginEnable,
                            "Enable plugin",
                            ButtonPresentation::Primary,
                        )),
                ),
            );
        }
        let mut status_with_local_memory = self.plugin_status.clone();
        let local_memory = self.settings_plugin_memory();
        *self.settings_jsx_displayed_memory.borrow_mut() = Some(local_memory.clone());
        if let Some(snapshot) = &mut status_with_local_memory
            && let Some(settings) = snapshot
                .plugins
                .iter_mut()
                .find(|plugin| plugin.id == crate::settings_package::ID)
        {
            settings.memory = local_memory;
        }
        let projection = crate::plugin_list::projection(
            &self.localizer,
            status_with_local_memory.as_ref(),
            self.plugin_notice.as_deref(),
            self.plugin_pending.as_ref(),
            self.plugin_setting_pending.as_ref(),
            self.plugin_setting_edit.as_ref(),
        );
        let list = if self.settings_jsx_enabled {
            self.plugin_list
                .borrow_mut()
                .get_or_insert_with(|| {
                    self.shared_settings_page(crate::settings_package::Script::Plugins)
                        .and_then(crate::plugin_list::PluginList::new_with_page)
                })
                .as_mut()
                .map_err(|error| error.clone())
                .and_then(|list| list.render(&projection, theme))
        } else {
            Err("Settings plugin is disabled".into())
        };
        content = content.child(match list {
            Ok(list) => list,
            Err(error) => {
                let mut recovery = SettingsCard::titled(
                    theme,
                    self.localizer.text("settings-plugin-list-unavailable"),
                    error,
                )
                .child(Button::semantic(
                    theme,
                    SettingsMessage::RefreshPlugins,
                    "Refresh",
                    ButtonPresentation::Secondary,
                ));
                if let Some(snapshot) = &self.plugin_status {
                    if snapshot.plugins.iter().any(|plugin| {
                        plugin.id == crate::settings_package::ID && !plugin.desired_enabled
                    }) {
                        recovery = recovery.child(
                            SettingsRow::new(
                                theme,
                                self.localizer.text("settings-plugin-self-name"),
                                self.localizer.text("settings-plugin-self-disabled"),
                            )
                            .trailing(Button::semantic(
                                theme,
                                SettingsMessage::ReviewPluginEnable(
                                    crate::settings_package::ID.into(),
                                ),
                                self.localizer.text("settings-plugin-self-review-enable"),
                                ButtonPresentation::Primary,
                            )),
                        );
                    }
                    for plugin in snapshot
                        .plugins
                        .iter()
                        .filter(|plugin| plugin.desired_enabled)
                    {
                        recovery = recovery.child(
                            SettingsRow::new(theme, &plugin.name, &plugin.id).trailing(
                                Button::semantic(
                                    theme,
                                    SettingsMessage::SetPluginEnabled {
                                        id: plugin.id.clone(),
                                        enabled: false,
                                    },
                                    "Disable",
                                    ButtonPresentation::Destructive,
                                ),
                            ),
                        );
                    }
                }
                AnyView::new(recovery)
            }
        });
        nickel_ui::VerticalScroll::new(SettingsMessage::PluginsScroll, 0.0)
            .grow(1.0)
            .theme(theme)
            .child(content)
    }

    pub(super) fn optional_features_components(&self) -> AnyView<SettingsMessage> {
        if self.page != SettingsPage::OptionalFeatures {
            return AnyView::new(Container::new());
        }
        let theme = self.ui_theme();
        let rendered = if self.settings_jsx_enabled {
            let data = crate::optional_features_plugin::projection(self);
            self.optional_features_page
                .borrow_mut()
                .get_or_insert_with(|| {
                    self.shared_settings_page(crate::settings_package::Script::OptionalFeatures)
                        .and_then(
                            crate::optional_features_plugin::OptionalFeaturesPage::new_with_page,
                        )
                })
                .as_mut()
                .map_err(|error| error.clone())
                .and_then(|page| page.render(&data, theme))
        } else {
            Err("Settings plugin is disabled".into())
        };
        rendered.unwrap_or_else(|error| {
            self.settings_plugin_recovery("Optional Features are unavailable", error, None)
        })
    }

    pub(super) fn default_apps_components(&self) -> AnyView<SettingsMessage> {
        if self.page != SettingsPage::DefaultApps {
            return AnyView::new(Container::new());
        }
        let theme = self.ui_theme();
        let matching_targets = crate::default_apps_plugin::matching_targets(self);
        let rendered = if self.settings_jsx_enabled {
            let data = crate::default_apps_plugin::projection_for_targets(self, &matching_targets);
            self.default_apps_page
                .borrow_mut()
                .get_or_insert_with(|| {
                    self.shared_settings_page(crate::settings_package::Script::DefaultApps)
                        .and_then(crate::default_apps_plugin::DefaultAppsPage::new_with_page)
                })
                .as_mut()
                .map_err(|error| error.clone())
                .and_then(|page| page.render(&data, theme))
        } else {
            Err("Settings plugin is disabled".into())
        };
        let content = match rendered {
            Ok(rendered) => rendered,
            Err(error) => {
                return self.settings_plugin_recovery(
                    "Default Apps settings are unavailable",
                    error,
                    None,
                );
            }
        };
        AnyView::new(ui! {
            <Column grow={1.0} padding={Insets { top: 16.0, right: 24.0, bottom: 20.0, left: 20.0 }} gap={10.0}>
                <VerticalScroll id={"default-apps-list"} on_scroll={SettingsMessage::DefaultAppsPageScroll} offset={0.0} theme={theme}>
                    {content}
                </VerticalScroll>
            </Column>
        })
    }

    pub(crate) fn default_app_overlays(
        &self,
        context: ViewContext,
    ) -> Vec<FrameOverlay<SettingsMessage>> {
        let open_row = context.open_overlay.as_ref().and_then(|overlay| {
            self.default_apps.iter().enumerate().find_map(|(index, _)| {
                (overlay == &OverlayId::new(format!("default-app-picker-{index}"))).then_some(index)
            })
        });
        self.default_app_picker_row.set(open_row);
        if open_row.is_none() {
            self.default_app_picker_page.borrow_mut().take();
        }
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
                let picker_id = format!("default-app-picker-{row_index}");
                let active = self.default_app_picker_row.get() == Some(row_index)
                    && context.open_overlay.as_ref() == Some(&OverlayId::new(picker_id.as_str()));
                let plugin_picker = if active && self.settings_jsx_enabled {
                    let data = crate::default_app_picker_plugin::projection(
                        self,
                        row_index,
                        &state,
                        can_change,
                        discovery_status.as_deref().unwrap_or_default(),
                    );
                    self.default_app_picker_page
                        .borrow_mut()
                        .get_or_insert_with(|| self.shared_settings_page(crate::settings_package::Script::DefaultAppPicker).and_then(crate::default_app_picker_plugin::DefaultAppPickerPage::new_with_page))
                        .as_mut()
                        .map_err(|error| error.clone())
                        .and_then(|page| page.render(&data, theme))
                        .map(Some)
                } else if active {
                    Err("Settings plugin is disabled".into())
                } else {
                    Ok(None)
                };
                let content: AnyView<SettingsMessage> = match plugin_picker {
                    Ok(Some(rendered)) => {
                        AnyView::new(
                            Column::new()
                                .gap(8.0)
                                .padding(Insets::all(10.0))
                                .background(palette.surface)
                                .child(rendered),
                        )
                    }
                    Ok(None) => AnyView::new(Container::new()),
                    Err(error) => self.settings_plugin_recovery(
                        "Default application picker is unavailable",
                        error,
                        None,
                    ),
                };
                Popover::new(
                    picker_id,
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

    pub(super) fn display_components(&self, _content_width: f32) -> AnyView<SettingsMessage> {
        let rendered = if self.settings_jsx_enabled {
            let data = crate::display_plugin::projection(self);
            self.display_page
                .borrow_mut()
                .get_or_insert_with(|| {
                    self.shared_settings_page(crate::settings_package::Script::Display)
                        .and_then(crate::display_plugin::DisplayPage::new_with_page)
                })
                .as_mut()
                .map_err(|error| error.clone())
                .and_then(|page| page.render(&data, self.ui_theme()))
        } else {
            Err("Settings plugin is disabled".into())
        };
        rendered.unwrap_or_else(|error| {
            self.settings_plugin_recovery("Display settings are unavailable", error, None)
        })
    }

    pub(super) fn network_components(&self) -> AnyView<SettingsMessage> {
        if self.page != SettingsPage::Network {
            return AnyView::new(Container::new());
        }
        let plugin_view = if self.settings_jsx_enabled {
            let data = crate::network_plugin::projection(self);
            self.network_page
                .borrow_mut()
                .get_or_insert_with(|| {
                    self.shared_settings_page(crate::settings_package::Script::Network)
                        .and_then(crate::network_plugin::NetworkPage::new_with_page)
                })
                .as_mut()
                .map_err(|error| error.clone())
                .and_then(|page| page.render(&data, self.ui_theme()))
        } else {
            Err("Settings plugin is disabled".into())
        };
        plugin_view.unwrap_or_else(|error| {
            self.settings_plugin_recovery("Network settings are unavailable", error, None)
        })
    }

    pub(super) fn bluetooth_components(&self) -> AnyView<SettingsMessage> {
        let rendered = if self.settings_jsx_enabled {
            let data = crate::bluetooth_plugin::projection(self);
            self.bluetooth_page
                .borrow_mut()
                .get_or_insert_with(|| {
                    self.shared_settings_page(crate::settings_package::Script::Bluetooth)
                        .and_then(crate::bluetooth_plugin::BluetoothPage::new_with_page)
                })
                .as_mut()
                .map_err(|error| error.clone())
                .and_then(|page| page.render(&data, self.ui_theme()))
        } else {
            Err("Settings plugin is disabled".into())
        };
        rendered.unwrap_or_else(|error| {
            self.settings_plugin_recovery("Bluetooth settings are unavailable", error, None)
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

    pub(super) fn bar_components(&self) -> AnyView<SettingsMessage> {
        if self.page != SettingsPage::Bar {
            return AnyView::new(Container::new());
        }
        let result = if self.settings_jsx_enabled {
            let data = crate::bar_plugin::projection(
                &self.localizer,
                &self.shell_settings,
                self.displays.len(),
                self.shell_topology_generation,
            );
            self.bar_page
                .borrow_mut()
                .get_or_insert_with(|| {
                    self.shared_settings_page(crate::settings_package::Script::Bar)
                        .and_then(crate::bar_plugin::BarPage::new_with_page)
                })
                .as_mut()
                .map_err(|error| error.clone())
                .and_then(|page| page.render(&data, self.ui_theme()))
        } else {
            Err("Settings plugin is disabled".into())
        };
        result.unwrap_or_else(|error| {
            self.settings_plugin_recovery("Nickel Bar settings are unavailable", error, None)
        })
    }

    fn appearance_frame(
        &self,
        theme: nickel_ui::SemanticTheme,
        content: impl nickel_ui::Component<SettingsMessage>,
    ) -> AnyView<SettingsMessage> {
        let mut general = nickel_ui::Column::new().gap(10.0).child(content);
        if let Some(notice) = self.appearance_notice.as_ref().map(|notice| match notice {
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
        }) {
            general = general.child(notice);
        }
        AnyView::new(ui! {
            <Column grow={1.0} padding={Insets {
                top: 16.0, right: 24.0, bottom: 20.0, left: 20.0,
            }} gap={10.0}>
                <VerticalScroll id={"appearance-list"} on_scroll={SettingsMessage::AppearanceScroll}
                    offset={0.0} theme={theme}>{general}</VerticalScroll>
            </Column>
        })
    }

    pub(super) fn appearance_components(&self) -> AnyView<SettingsMessage> {
        let theme = self.ui_theme();
        let rendered = if self.settings_jsx_enabled {
            let data = crate::appearance_plugin::projection(self);
            self.appearance_page
                .borrow_mut()
                .get_or_insert_with(|| {
                    self.shared_settings_page(crate::settings_package::Script::Appearance)
                        .and_then(crate::appearance_plugin::AppearancePage::new_with_page)
                })
                .as_mut()
                .map_err(|error| error.clone())
                .and_then(|page| page.render(&data, theme, self.wallpaper_preview.as_ref()))
        } else {
            Err("Settings plugin is disabled".into())
        };
        let content = rendered.unwrap_or_else(|error| {
            self.settings_plugin_recovery(
                "Appearance settings are unavailable",
                error,
                self.custom_hue_open
                    .then_some((SettingsMessage::CancelCustomHue, "Close color picker")),
            )
        });
        self.appearance_frame(theme, content)
    }

    pub(super) fn keyboard_shortcuts_components(&self) -> AnyView<SettingsMessage> {
        if self.page != SettingsPage::KeyboardShortcuts {
            return AnyView::new(Container::new());
        }
        let rendered = if self.settings_jsx_enabled {
            self.ordinary_pages
                .borrow_mut()
                .get_or_insert_with(|| {
                    self.shared_settings_page(crate::settings_package::Script::OrdinaryPages)
                        .and_then(crate::settings_plugin::OrdinaryPages::new_with_page)
                })
                .as_mut()
                .map_err(|error| error.clone())
                .and_then(|pages| pages.render_keyboard(&self.localizer, self.ui_theme()))
        } else {
            Err("Settings plugin is disabled".into())
        };
        rendered.unwrap_or_else(|error| {
            self.settings_plugin_recovery(
                &self.localizer.text("settings-keyboard-page-unavailable"),
                error,
                None,
            )
        })
    }

    pub(super) fn about_components(&self) -> AnyView<SettingsMessage> {
        if self.page != SettingsPage::About {
            return AnyView::new(Container::new());
        }
        let rendered = if self.settings_jsx_enabled {
            self.ordinary_pages
                .borrow_mut()
                .get_or_insert_with(|| {
                    self.shared_settings_page(crate::settings_package::Script::OrdinaryPages)
                        .and_then(crate::settings_plugin::OrdinaryPages::new_with_page)
                })
                .as_mut()
                .map_err(|error| error.clone())
                .and_then(|pages| pages.render_about(&self.localizer, self.ui_theme()))
        } else {
            Err("Settings plugin is disabled".into())
        };
        rendered.unwrap_or_else(|error| {
            self.settings_plugin_recovery(
                &self.localizer.text("settings-about-page-unavailable"),
                error,
                None,
            )
        })
    }
}
