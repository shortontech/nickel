use super::*;

impl SettingsApp {
    pub(super) fn optional_features_components(
        &self,
    ) -> impl nickel_ui::Component<SettingsMessage> {
        let theme = self.ui_theme();
        let state = &self.codex_feature;
        let available = state.capability.support == FeatureSupport::Supported
            && state.capability.installation == FeatureInstallation::Installed;
        let switch_state = match (state.requested_enabled, available, state.editable()) {
            (true, true, true) => SwitchState::On,
            (false, true, true) => SwitchState::Off,
            (true, _, _) => SwitchState::DisabledOn,
            (false, _, _) => SwitchState::DisabledOff,
        };
        let status = match state.effective {
            FeatureEffectiveState::Disabled => "Disabled",
            FeatureEffectiveState::Enabling => "Applying…",
            FeatureEffectiveState::Enabled => state.apply_label(),
            FeatureEffectiveState::Unavailable => "Unavailable",
            FeatureEffectiveState::Rejected => "Change rejected",
            FeatureEffectiveState::Stale => "Ignoring stale runtime state",
        };
        let detail = state
            .capability
            .diagnostic
            .as_deref()
            .unwrap_or(&state.capability.source_label);
        let selected_source = match &self.optional_features.codex_source {
            CodexSource::CompatibleInstalled => "Compatible installed Codex".to_owned(),
            CodexSource::Bundled => "Bundled Codex".to_owned(),
            CodexSource::ApprovedRemote => "Approved remote host".to_owned(),
            CodexSource::Executable(path) => format!("Executable — {}", path.display()),
        };
        let source_options: Vec<(String, SettingsMessage)> = vec![
            (
                "Compatible installed Codex".into(),
                SettingsMessage::SetCodexSource(CodexSource::CompatibleInstalled),
            ),
            (
                "Bundled Codex".into(),
                SettingsMessage::SetCodexSource(CodexSource::Bundled),
            ),
            (
                "Approved remote host".into(),
                SettingsMessage::SetCodexSource(CodexSource::ApprovedRemote),
            ),
        ];
        let source = SelectField::new(
            theme,
            "Backend source",
            "Choose an installed, bundled, remote, or explicit compatible executable.",
            SettingsMessage::ToggleCodexSourceSelect,
            selected_source,
            source_options,
            self.codex_source_select_expanded,
        )
        .id("optional-feature-codex-source");
        let executable = SettingsRow::new(theme, "Explicit executable", "Absolute path").trailing(
            ui! { <Row width={420.0} gap={8.0}>
                <Container width={310.0} padding={Insets::all(6.0)}>
                    {TextField::on_change_with_placeholder(&self.codex_executable_path,
                        "/path/to/codex", SettingsMessage::CodexExecutablePathChanged)
                        .id("optional-feature-codex-executable")}
                </Container>
                {Button::semantic(theme, SettingsMessage::ApplyCodexExecutable,
                    "Use", ButtonPresentation::Secondary).width(76.0)}
            </Row> },
        );
        let retry = Button::semantic(
            theme,
            SettingsMessage::RetryCodexProbe,
            "Retry probe",
            ButtonPresentation::Secondary,
        )
        .width(130.0);
        let policy = state
            .capability
            .policy_source
            .as_deref()
            .unwrap_or("User preference");
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
        SettingsCard::titled(
            theme,
            "Codex integration",
            "Projects, conversations, and the built-in Codex client",
        )
        .child(
            SettingsRow::new(theme, "Enable Codex", status).trailing(
                Switch::with_state(
                    switch_state,
                    (available && state.editable())
                        .then_some(SettingsMessage::SetCodexEnabled as fn(bool) -> SettingsMessage),
                    theme,
                )
                .id("optional-feature-codex-enabled")
                .accessibility_label("Enable Codex integration"),
            ),
        )
        .child(confirmation)
        .child(source)
        .child(executable)
        .child(SettingsRow::new(theme, "Source", detail))
        .child(SettingsRow::new(theme, "Policy", policy))
        .child(SettingsRow::new(
            theme,
            "Health",
            format!("{:?}", state.capability.health),
        ))
        .child(SettingsRow::new(
            theme,
            "Required permissions",
            if state.capability.required_permissions.is_empty() {
                "None".into()
            } else {
                state.capability.required_permissions.join(" · ")
            },
        ))
        .child(SettingsRow::new(
            theme,
            "Diagnostics",
            format!(
                "workers {} · subscriptions {} · warm surfaces {} · cache entries {}",
                self.optional_feature_runtime.background_workers,
                self.optional_feature_runtime.subscriptions,
                self.optional_feature_runtime.warm_surfaces,
                self.optional_feature_runtime.cache_entries
            ),
        ))
        .child(SettingsRow::new(
            theme,
            "Resource and privacy impact",
            if state.requested_enabled {
                "May run an app-server and poll project and conversation metadata"
            } else {
                "No background workers, subscriptions, or warm Codex surfaces"
            },
        ))
        .child(retry)
    }

    pub(super) fn default_apps_components(&self) -> impl nickel_ui::Component<SettingsMessage> {
        let theme = self.ui_theme();
        let palette = self.palette();
        let rows = self.default_apps.iter().enumerate().map(|(index, row)| {
            let current = row.snapshot.as_ref().and_then(|snapshot| snapshot.effective.as_ref())
                .map(|handler| handler.name.clone()).unwrap_or_else(|| "No effective handler reported".into());
            let options: Vec<(String, SettingsMessage)> = row.snapshot.as_ref().map(|snapshot| snapshot.handlers.iter().map(|handler| {
                (handler.name.clone(), SettingsMessage::SetDefaultApp { row: index, handler_id: handler.id.clone() })
            }).collect()).unwrap_or_default();
            let detail = row.status.clone().or_else(|| row.snapshot.as_ref().map(|snapshot| snapshot.detail.clone())).unwrap_or_else(|| "Not loaded".into());
            let mutable = row.snapshot.as_ref().is_some_and(|snapshot| snapshot.capability == nickel_platform::AssociationCapability::DirectUserChange);
            let selector = SelectField::new(theme, row.label.clone(), detail.clone(),
                SettingsMessage::ToggleDefaultAppSelect(index), current.clone(), options,
                self.default_app_select_expanded == Some(index)).id(format!("default-app-{index}"));
            ui! {
                <Container background={palette.surface} border={(palette.muted, 1.0)} padding={Insets::all(4.0)}>
                    {if mutable { AnyView::new(selector) } else {
                        AnyView::new(SettingsRow::new(theme, format!("{} — {}", row.label, detail), current))
                    }}
                </Container>
            }
        });
        let note = SettingsStatus::<SettingsMessage>::new(
            theme,
            SettingsStatusKind::Validation,
            "These are operating-system associations. Terminal and file-manager preferences are separate Nickel-owned settings.",
        );
        ui! {
            <Column grow={1.0} padding={Insets { top: 16.0, right: 24.0, bottom: 20.0, left: 20.0 }} gap={10.0}>
                <VerticalScroll id={"default-apps-list"} on_scroll={SettingsMessage::DefaultAppsScroll} offset={0.0}>
                    <Column gap={10.0}>{note}<Column gap={8.0} children={rows} /></Column>
                </VerticalScroll>
            </Column>
        }
    }

    pub(super) fn display_components(&self) -> impl nickel_ui::Component<SettingsMessage> {
        let palette = self.palette();
        let theme = self.ui_theme();
        let selected = &self.displays[self.selected];
        let identify = Button::semantic(
            theme,
            SettingsMessage::DisplayIdentify,
            self.localizer.text("settings-display-identify"),
            ButtonPresentation::Secondary,
        )
        .width(135.0);
        let make_primary = Button::semantic(
            theme,
            SettingsMessage::DisplayPrimary,
            self.localizer.text("settings-display-make-primary"),
            ButtonPresentation::Secondary,
        )
        .width(145.0);
        let enabled = SettingsRow::new(theme, "Display enabled", "").trailing(
            Switch::new(selected.enabled, SettingsMessage::DisplayEnabled, theme)
                .id("display-enabled")
                .accessibility_label("Display enabled"),
        );
        let apply = Button::semantic(
            theme,
            SettingsMessage::DisplayApply,
            self.localizer.text("settings-display-apply"),
            ButtonPresentation::Primary,
        )
        .width(105.0);
        let display_cards = self.displays.iter().enumerate().map(|(index, display)| {
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
            ui! {
                <Container id={format!("display-card-{index}")}
                    width={display.rect.w as f32} height={display.rect.h as f32}
                    min_width={160.0} min_height={104.0}
                    background={if !display.enabled { palette.background }
                        else if selected { palette.accent_soft } else { palette.surface }}
                    border={(border_color, border_width)} radius={theme.radii.card}
                    padding={Insets::all(18.0)}
                    on_drag={(SettingsMessage::SelectDisplay(index), display_drag_message)}
                    on_press={SettingsMessage::SelectDisplay(index)}>
                    <Column gap={8.0}>
                        <Text scale={1.5} color={palette.text}>{&display.name}</Text>
                        <Text color={palette.muted}>{detail}</Text>
                        <Text bold={true} color={palette.accent}>
                            {if display.primary { "PRIMARY" } else { "" }}
                        </Text>
                    </Column>
                </Container>
            }
        });
        ui! {
            <Column grow={1.0} padding={Insets {
                top: 20.0, right: 32.0, bottom: 20.0, left: 20.0,
            }} gap={12.0}>
                <Container id={"display-plane"} grow={1.0} min_height={300.0}
                    background={palette.surface} border={(palette.muted, 1.0)}
                    padding={Insets::all(20.0)} align_items={nickel_ui::Align::Center}
                    justify_content={nickel_ui::Justify::Center}
                    semantic_role={SemanticRole::TabPanel}
                    accessibility_label={"Display arrangement"}>
                    <Row gap={12.0} children={display_cards} />
                </Container>
                <Container background={palette.surface} border={(palette.muted, 1.0)}
                    padding={Insets::all(12.0)}>
                    <Column gap={10.0}>
                        <Row height={36.0} gap={12.0}>
                            <Column grow={1.0} gap={3.0}>
                                <Text color={palette.text}>{&selected.name}</Text>
                                <Text scale={0.9} color={palette.muted}>{&selected.detail}</Text>
                            </Column>
                            <Text bold={true} color={if selected.primary { palette.accent } else { palette.muted }}>
                                {if selected.primary {
                                    self.localizer.text("settings-display-primary")
                                } else { String::new() }}
                            </Text>
                        </Row>
                        {enabled}
                        <Row height={42.0} gap={12.0}>
                            {identify}{make_primary}{apply}
                        </Row>
                    </Column>
                </Container>
                <Text color={if self.applied { palette.complement } else { palette.muted }} height={18.0}>
                    {&self.status}
                </Text>
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
                    <Container height={44.0}
                        background={palette.surface}
                        hover_background={palette.surface_hover}
                        pressed_background={palette.surface_hover}
                        border={(if network.connected { palette.accent } else { palette.muted },
                            if network.connected { 2.0 } else { 1.0 })}
                        padding={Insets { top: 12.0, right: 14.0, bottom: 8.0, left: 14.0 }}
                        on_press={SettingsMessage::WifiNetwork(index)}>
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
                    offset={0.0}>{content}</VerticalScroll>
            </Column>
        }
    }

    pub(super) fn bluetooth_components(&self) -> impl nickel_ui::Component<SettingsMessage> {
        let palette = self.palette();
        let theme = self.ui_theme();
        let device_cards = self
            .bluetooth
            .devices
            .iter()
            .enumerate()
            .map(|(index, device)| {
                let status = if device.connected {
                    self.localizer.text("settings-bluetooth-connected")
                } else if device.paired {
                    self.localizer.text("settings-bluetooth-paired")
                } else {
                    self.localizer.text("settings-bluetooth-available")
                };
                let detail = device
                    .battery_percent
                    .map(|percent| format!("{percent}%"))
                    .unwrap_or_default();
                ui! {
                    <Container height={68.0}
                        background={palette.surface}
                        hover_background={palette.surface_hover}
                        pressed_background={palette.surface_hover}
                        border={(if device.connected { palette.accent } else { palette.muted },
                            if device.connected { 2.0 } else { 1.0 })}
                        on_press={SettingsMessage::BluetoothDevice(index)}
                        padding={Insets { top: 12.0, right: 14.0, bottom: 10.0, left: 14.0 }}>
                        <Row>
                            <Column grow={1.0} gap={7.0}>
                                <Text color={palette.text}>{&device.name}</Text>
                                <Text scale={1.0} color={if device.connected { palette.complement } else { palette.muted }}>
                                    {status}
                                </Text>
                            </Column>
                            <Text color={palette.muted}>{detail}</Text>
                        </Row>
                    </Container>
                }
            });

        let adapter_status = if !self.bluetooth.available {
            self.localizer
                .text("settings-bluetooth-service-unavailable")
        } else if self.bluetooth.powered {
            self.localizer.text("settings-bluetooth-on")
        } else {
            self.localizer.text("settings-bluetooth-off")
        };
        let discoverability = if self.bluetooth.discovering {
            self.localizer.text("settings-bluetooth-discovery-stop")
        } else {
            self.localizer.text("settings-bluetooth-discovery-start")
        };
        let discovery_button = Button::semantic(
            self.ui_theme(),
            SettingsMessage::BluetoothDiscovery,
            discoverability,
            ButtonPresentation::Secondary,
        )
        .width(150.0);
        let device_list = if self.bluetooth.devices.is_empty() {
            ui! { <Column><Text color={palette.muted}>{if self.bluetooth.available {
                self.localizer.text("settings-bluetooth-no-devices")
            } else {
                self.localizer
                    .text("settings-bluetooth-service-unavailable")
            }}</Text></Column> }
        } else {
            ui! { <Column gap={10.0} children={device_cards} /> }
        };
        let bluetooth_switch_state = if !self.bluetooth.available {
            SwitchState::DisabledOff
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
                self.bluetooth
                    .available
                    .then_some(bluetooth_power_message as fn(bool) -> SettingsMessage),
                theme,
            )
            .id("bluetooth-power")
            .accessibility_label(bluetooth_label),
        );
        let content = ui! {
            <Column gap={12.0}>
                {bluetooth_power}
                <Text scale={1.0} color={palette.muted}>{adapter_status}</Text>
                <Row height={36.0}>
                    <Text width={390.0} color={palette.text}>{self.localizer.text("settings-bluetooth-devices")}</Text>
                    {discovery_button}
                </Row>
                {device_list}
            </Column>
        };

        ui! {
            <Column grow={1.0} padding={Insets {
                top: 20.0, right: 40.0, bottom: 20.0, left: 20.0,
            }}>
                <VerticalScroll id={"bluetooth-list"} on_scroll={SettingsMessage::BluetoothScroll}
                    offset={0.0}>{content}</VerticalScroll>
            </Column>
        }
    }

    pub(super) fn bar_components(&self) -> impl nickel_ui::Component<SettingsMessage> {
        let palette = self.palette();
        let theme = self.ui_theme();
        let display_count = self.displays.len().max(1);
        let desktop_choices = (0..self.shell_settings.desktop_count).map(|index| {
            ui! {
                <Container width={64.0} height={46.0} background={palette.surface}
                    border={(if index == 0 { palette.accent } else { palette.muted }, 2.0)}
                    padding={Insets { top: 9.0, right: 4.0, bottom: 4.0, left: 4.0 }}>
                    <Text align={TextAlign::Center} scale={1.0}
                        color={if index == 0 { palette.text } else { palette.muted }}>
                        {format!("{}", index + 1)}
                    </Text>
                </Container>
            }
        });
        ui! {
            <Column grow={1.0} padding={Insets {
                top: 24.0, right: 40.0, bottom: 20.0, left: 20.0,
            }} gap={14.0}>
                <Text color={palette.text} height={20.0}>{self.localizer.text("settings-bar-show-on")}</Text>
                <Row height={38.0} gap={28.0}>
                    <RadioButton on_press={SettingsMessage::BarPrimaryDisplay}
                        label={self.localizer.text("settings-bar-primary-display")}
                        selected={!self.shell_settings.bar_on_all_displays}
                        colors_pair={(if !self.shell_settings.bar_on_all_displays { palette.accent } else { palette.muted }, palette.text)}
                        width={210.0} />
                    <RadioButton on_press={SettingsMessage::BarAllDisplays}
                        label={self.localizer.number("settings-bar-all-displays", "count", display_count as i64)}
                        selected={self.shell_settings.bar_on_all_displays}
                        colors_pair={(if self.shell_settings.bar_on_all_displays { palette.accent } else { palette.muted }, palette.text)}
                        width={210.0} />
                </Row>
                <Text color={palette.text} height={20.0}>{self.localizer.text("settings-bar-window-scope")}</Text>
                <Row height={38.0} gap={28.0}>
                    <RadioButton on_press={SettingsMessage::BarDisplayWindows}
                        label={self.localizer.text("settings-bar-this-display")}
                        selected={!self.shell_settings.all_windows_on_every_bar}
                        colors_pair={(if !self.shell_settings.all_windows_on_every_bar { palette.accent } else { palette.muted }, palette.text)}
                        width={210.0} />
                    <RadioButton on_press={SettingsMessage::BarAllWindows}
                        label={self.localizer.text("settings-bar-all-windows")}
                        selected={self.shell_settings.all_windows_on_every_bar}
                        colors_pair={(if self.shell_settings.all_windows_on_every_bar { palette.accent } else { palette.muted }, palette.text)}
                        width={210.0} />
                </Row>
                <Text color={palette.text} height={20.0}>{self.localizer.text("settings-bar-desktops")}</Text>
                <Text scale={1.0} color={palette.muted} height={18.0}>
                    {self.localizer.number("settings-bar-desktop-count", "count", i64::from(self.shell_settings.desktop_count))}
                </Text>
                <Slider id={"bar-desktop-count"}
                    value={f32::from(self.shell_settings.desktop_count.saturating_sub(1)) / 7.0}
                    on_change={desktop_count_message} width={520.0}
                    adjustment_step={1.0 / 7.0}
                    focus_background_tint={theme.borders.focus}
                    controller_focus_background_tint={theme.borders.controller_focus} />
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
                    offset={0.0}>{general}</VerticalScroll>
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
