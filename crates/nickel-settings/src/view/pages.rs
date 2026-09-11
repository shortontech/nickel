use super::*;
use nickel_ui::{
    Collection, CollectionPresentation, CollectionState, Column, ComponentBuilderExt, Container,
    GridColumnSpec, NavigationScope, RadioGroup, RadioOption, Row, SettingsListCard, Text,
    TextField, Track,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RemoteExposurePresentation {
    pub(crate) kind: SettingsStatusKind,
    pub(crate) exposure: String,
    pub(crate) transport: String,
}

pub(crate) fn remote_exposure_presentation(
    snapshot: &nickel_session_protocol::RemoteControlSnapshot,
) -> RemoteExposurePresentation {
    use nickel_session_protocol::RemoteControlEffectiveState as Effective;

    let endpoint = url::Url::parse(&snapshot.endpoint).ok();
    let loopback = endpoint
        .as_ref()
        .and_then(url::Url::host_str)
        .and_then(|host| host.parse::<std::net::IpAddr>().ok())
        .map(|address| address.is_loopback());
    let https = endpoint
        .as_ref()
        .is_some_and(|endpoint| endpoint.scheme() == "https");
    let transport = match (https, loopback, snapshot.host_fingerprint.is_some()) {
        (true, _, true) => {
            "HTTPS with a host certificate fingerprint available for verification".into()
        }
        (true, _, false) => {
            "HTTPS reported, but the host certificate fingerprint is unavailable".into()
        }
        (false, Some(true), _) => "Local HTTP restricted to this computer".into(),
        (false, Some(false), _) => {
            "Unprotected HTTP was reported for a non-loopback address".into()
        }
        (false, None, _) => {
            "Transport state unavailable; the endpoint could not be classified".into()
        }
    };

    let (kind, exposure) = match (snapshot.effective, loopback, https) {
        (Effective::Enabled, Some(false), true) => (
            SettingsStatusKind::Validation,
            "Remote exposure active. New connections have no authority until locally approved.".into(),
        ),
        (Effective::Enabled, Some(false), false) => (
            SettingsStatusKind::Error,
            "Unsafe remote exposure reported: the active non-loopback endpoint is not protected by HTTPS.".into(),
        ),
        (Effective::Enabled, Some(true), _) => (
            SettingsStatusKind::Information,
            "Local only. The listener accepts connections from this computer.".into(),
        ),
        (Effective::Enabled, None, _) => (
            SettingsStatusKind::Unavailable,
            "Exposure state unavailable because the active endpoint could not be classified.".into(),
        ),
        (Effective::Disabled, Some(false), _) => (
            SettingsStatusKind::Unavailable,
            "Not exposed. A remote address is configured, but the listener is disabled.".into(),
        ),
        (Effective::Disabled, _, _) => (
            SettingsStatusKind::Unavailable,
            "Not exposed. The listener is disabled.".into(),
        ),
        (Effective::Rejected, Some(false), _) => (
            SettingsStatusKind::Error,
            "Not exposed. A remote address is configured, but the listener failed to start.".into(),
        ),
        (Effective::Rejected, _, _) => (
            SettingsStatusKind::Error,
            "Not exposed. The listener failed to start.".into(),
        ),
    };
    RemoteExposurePresentation {
        kind,
        exposure,
        transport,
    }
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
    pub(super) fn peripherals_components(&self) -> impl nickel_ui::Component<SettingsMessage> {
        let theme = self.ui_theme();
        let palette = self.palette();
        let pending = self.peripheral_rx.is_some();
        let bytes = |value: u64| {
            const GIB: f64 = 1_073_741_824.0;
            if value >= 1_073_741_824 {
                format!("{:.1} GiB", value as f64 / GIB)
            } else {
                format!("{:.1} MiB", value as f64 / 1_048_576.0)
            }
        };
        let mut content = Column::new().gap(4.0);
        if let Some(status) = &self.peripheral_status {
            content = content.child(Text::new(status).color(palette.muted));
        }
        if let Some(snapshot) = &self.peripheral_snapshot {
            let provider = match &snapshot.provider {
                nickel_platform::PeripheralProvider::LinuxCupsAndUDisks2 {
                    cups_available,
                    udisks2_available,
                } => format!(
                    "Linux · CUPS {} · UDisks2 {}",
                    if *cups_available {
                        "available"
                    } else {
                        "unavailable"
                    },
                    if *udisks2_available {
                        "available"
                    } else {
                        "unavailable"
                    }
                ),
                nickel_platform::PeripheralProvider::WindowsPrintAndStorage => {
                    "Windows print and storage services".into()
                }
                nickel_platform::PeripheralProvider::Unsupported { platform } => {
                    format!("Unsupported platform ({platform})")
                }
            };
            content = content
                .child(SettingsRow::new(theme, "System provider", provider))
                .child(Text::new("Printers").color(palette.text));
            match &snapshot.printers {
                Ok(printers) if printers.is_empty() => {
                    content = content
                        .child(Text::new("No printers were discovered.").color(palette.muted));
                }
                Ok(printers) => {
                    for printer in printers {
                        let id = printer.id.clone();
                        let state = format!(
                            "{:?}{} · {} queued",
                            printer.state,
                            if printer.is_default {
                                " · Default"
                            } else {
                                ""
                            },
                            printer.jobs.len()
                        );
                        let actions = Row::new()
                            .gap(4.0)
                            .child(
                                Button::semantic(
                                    theme,
                                    SettingsMessage::PeripheralAction(
                                        nickel_platform::PeripheralAction::SetDefaultPrinter(
                                            id.clone(),
                                        ),
                                    ),
                                    "Default",
                                    ButtonPresentation::Quiet,
                                )
                                .enabled(!pending && !printer.is_default),
                            )
                            .child(
                                Button::semantic(
                                    theme,
                                    SettingsMessage::PeripheralAction(
                                        nickel_platform::PeripheralAction::PrintTestPage(
                                            id.clone(),
                                        ),
                                    ),
                                    "Test",
                                    ButtonPresentation::Quiet,
                                )
                                .enabled(!pending),
                            )
                            .child(
                                Button::semantic(
                                    theme,
                                    SettingsMessage::PeripheralAction(
                                        nickel_platform::PeripheralAction::RemovePrinter(id),
                                    ),
                                    "Remove",
                                    ButtonPresentation::Quiet,
                                )
                                .enabled(!pending),
                            );
                        content = content.child(
                            SettingsRow::new(theme, printer.name.clone(), state).trailing(actions),
                        );
                        for job in &printer.jobs {
                            content = content.child(
                                SettingsRow::new(
                                    theme,
                                    format!("↳ {}", job.name),
                                    format!("Job {} · {:?}", job.id, job.state),
                                )
                                .trailing(
                                    Button::semantic(
                                        theme,
                                        SettingsMessage::PeripheralAction(
                                            nickel_platform::PeripheralAction::CancelPrintJob {
                                                printer_id: printer.id.clone(),
                                                job_id: job.id.clone(),
                                            },
                                        ),
                                        "Cancel",
                                        ButtonPresentation::Quiet,
                                    )
                                    .enabled(!pending),
                                ),
                            );
                        }
                    }
                }
                Err(error) => content = content.child(Text::new(error).color(palette.muted)),
            }
            content = content.child(Text::new("Removable media").color(palette.text));
            match &snapshot.volumes {
                Ok(volumes) if volumes.is_empty() => {
                    content = content
                        .child(Text::new("No removable media is connected.").color(palette.muted));
                }
                Ok(volumes) => {
                    for volume in volumes {
                        let capacity = volume
                            .capacity_bytes
                            .map(&bytes)
                            .unwrap_or_else(|| "Unknown size".into());
                        let action = match volume.state {
                            nickel_platform::VolumeState::Unmounted => {
                                nickel_platform::PeripheralAction::MountVolume(volume.id.clone())
                            }
                            nickel_platform::VolumeState::Mounted => {
                                nickel_platform::PeripheralAction::UnmountVolume(volume.id.clone())
                            }
                            nickel_platform::VolumeState::Busy
                            | nickel_platform::VolumeState::Error => {
                                nickel_platform::PeripheralAction::EjectVolume(volume.id.clone())
                            }
                        };
                        let label = match volume.state {
                            nickel_platform::VolumeState::Unmounted => "Mount",
                            nickel_platform::VolumeState::Mounted => "Unmount",
                            nickel_platform::VolumeState::Busy
                            | nickel_platform::VolumeState::Error => "Eject",
                        };
                        content = content.child(
                            SettingsRow::new(
                                theme,
                                volume.name.clone(),
                                format!("{:?} · {capacity}", volume.state),
                            )
                            .trailing(
                                Button::semantic(
                                    theme,
                                    SettingsMessage::PeripheralAction(action),
                                    label,
                                    ButtonPresentation::Quiet,
                                )
                                .enabled(!pending),
                            ),
                        );
                    }
                }
                Err(error) => content = content.child(Text::new(error).color(palette.muted)),
            }
            content = content.child(Text::new("Storage usage").color(palette.text));
            match &snapshot.filesystems {
                Ok(filesystems) => {
                    for filesystem in filesystems {
                        let used = filesystem
                            .capacity_bytes
                            .saturating_sub(filesystem.available_bytes);
                        content = content.child(
                            SettingsRow::new(
                                theme,
                                filesystem.name.clone(),
                                format!(
                                    "{} used of {} · {}",
                                    bytes(used),
                                    bytes(filesystem.capacity_bytes),
                                    filesystem.mount_path.display()
                                ),
                            )
                            .trailing(
                                Button::semantic(
                                    theme,
                                    SettingsMessage::PeripheralAction(
                                        nickel_platform::PeripheralAction::OpenCleanupLocation(
                                            filesystem.mount_path.clone(),
                                        ),
                                    ),
                                    "Open in Nickel File",
                                    ButtonPresentation::Quiet,
                                )
                                .enabled(!pending),
                            ),
                        );
                    }
                }
                Err(error) => content = content.child(Text::new(error).color(palette.muted)),
            }
        }
        let add_enabled = !pending && !self.peripheral_address.trim().is_empty();
        let add_action = nickel_platform::PeripheralAction::AddPrinter {
            address: self.peripheral_address.trim().to_owned(),
        };
        Column::new()
            .grow(1.0)
            .padding(Insets {
                top: 16.0,
                right: 24.0,
                bottom: 20.0,
                left: 20.0,
            })
            .gap(8.0)
            .child(
                Row::new()
                    .gap(8.0)
                    .child(
                        TextField::on_change_with_placeholder(
                            &self.peripheral_address,
                            "Printer address or URI",
                            SettingsMessage::PeripheralAddressChanged,
                        )
                        .id("peripheral-printer-address")
                        .width(360.0),
                    )
                    .child(
                        Button::semantic(
                            theme,
                            SettingsMessage::PeripheralAction(add_action),
                            "Add printer",
                            ButtonPresentation::Secondary,
                        )
                        .enabled(add_enabled),
                    )
                    .child(
                        Button::semantic(
                            theme,
                            SettingsMessage::PeripheralRefresh,
                            "Refresh",
                            ButtonPresentation::Secondary,
                        )
                        .enabled(!pending),
                    ),
            )
            .child(
                nickel_ui::VerticalScroll::new(SettingsMessage::PeripheralScroll, 0.0)
                    .height(620.0)
                    .theme(theme)
                    .child(content),
            )
    }

    pub(super) fn security_components(&self) -> impl nickel_ui::Component<SettingsMessage> {
        let theme = self.ui_theme();
        let palette = self.palette();
        let observation = |state: nickel_platform::ObservationState,
                           value: Option<String>,
                           detail: Option<&str>| {
            let state = match state {
                nickel_platform::ObservationState::Current => "Current",
                nickel_platform::ObservationState::Stale => "Stale",
                nickel_platform::ObservationState::Unsupported => "Unsupported",
                nickel_platform::ObservationState::PermissionDenied => "Permission denied",
                nickel_platform::ObservationState::Failed => "Failed",
            };
            [Some(state.to_owned()), value, detail.map(str::to_owned)]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" · ")
        };
        let content = if let Some(snapshot) = &self.maintenance_snapshot {
            let provider = match &snapshot.provider {
                nickel_platform::MaintenanceProvider::LinuxPackageKit { distribution } => {
                    format!("PackageKit ({distribution})")
                }
                nickel_platform::MaintenanceProvider::LinuxUnsupported { distribution } => {
                    format!("No supported update provider ({distribution})")
                }
                nickel_platform::MaintenanceProvider::WindowsUpdateAndSecurity => {
                    "Windows Update and Windows Security".into()
                }
                nickel_platform::MaintenanceProvider::Unsupported { platform } => {
                    format!("Unsupported platform ({platform})")
                }
            };
            let update_value = snapshot.updates.value.as_ref().map(|status| {
                format!(
                    "{} available · {:?}{}",
                    status.available,
                    status.phase,
                    if status.restart_required {
                        " · Restart required"
                    } else {
                        ""
                    }
                )
            });
            let health = |value: Option<nickel_platform::ProtectionHealth>| {
                value.map(|value| match value {
                    nickel_platform::ProtectionHealth::Healthy => "Healthy".into(),
                    nickel_platform::ProtectionHealth::AttentionRequired => {
                        "Attention required".into()
                    }
                    nickel_platform::ProtectionHealth::Unhealthy => "Unhealthy".into(),
                })
            };
            let pending = self.maintenance_rx.is_some();
            let update_supported =
                snapshot.updates.state != nickel_platform::ObservationState::Unsupported;
            let update_actions = ui! { <Row width={272.0} gap={4.0}>
                {Button::semantic(theme, SettingsMessage::MaintenanceAction(
                    nickel_platform::MaintenanceAction::CheckForUpdates), "Check",
                    ButtonPresentation::Quiet).width(72.0).enabled(update_supported && !pending)}
                {Button::semantic(theme, SettingsMessage::MaintenanceAction(
                    nickel_platform::MaintenanceAction::InstallUpdates), "Install",
                    ButtonPresentation::Quiet).width(76.0).enabled(update_supported && !pending)}
                {Button::semantic(theme, SettingsMessage::MaintenanceAction(
                    nickel_platform::MaintenanceAction::ScheduleRestart), "Restart options",
                    ButtonPresentation::Quiet).width(116.0).enabled(!pending)}
            </Row> };
            let mut column = Column::new()
                .gap(2.0)
                .child(SettingsRow::new(theme, "System provider", provider))
                .child(
                    SettingsRow::new(
                        theme,
                        "System updates",
                        observation(
                            snapshot.updates.state,
                            update_value,
                            snapshot.updates.detail.as_deref(),
                        ),
                    )
                    .trailing(update_actions),
                )
                .child(SettingsRow::new(
                    theme,
                    "Firewall",
                    observation(
                        snapshot.protection.firewall.state,
                        health(snapshot.protection.firewall.value),
                        snapshot.protection.firewall.detail.as_deref(),
                    ),
                ))
                .child(SettingsRow::new(
                    theme,
                    "Malware protection",
                    observation(
                        snapshot.protection.malware_protection.state,
                        health(snapshot.protection.malware_protection.value),
                        snapshot.protection.malware_protection.detail.as_deref(),
                    ),
                ))
                .child(
                    SettingsRow::new(
                        theme,
                        "Secure storage",
                        observation(
                            snapshot.secure_storage.state,
                            snapshot
                                .secure_storage
                                .value
                                .as_ref()
                                .map(|value| format!("{value:?}")),
                            snapshot.secure_storage.detail.as_deref(),
                        ),
                    )
                    .trailing(
                        Button::semantic(
                            theme,
                            SettingsMessage::MaintenanceAction(
                                nickel_platform::MaintenanceAction::RecoverSecureStorage,
                            ),
                            "Recovery",
                            ButtonPresentation::Quiet,
                        )
                        .width(100.0)
                        .enabled(!pending),
                    ),
                );
            for permission in &snapshot.permissions {
                let label = match permission.kind {
                    nickel_platform::PermissionKind::Camera => "Camera",
                    nickel_platform::PermissionKind::Microphone => "Microphone",
                    nickel_platform::PermissionKind::Location => "Location",
                    nickel_platform::PermissionKind::Notifications => "Notifications",
                    nickel_platform::PermissionKind::ScreenCapture => "Screen capture",
                };
                let value = permission.global_enabled.value.map(|enabled| {
                    if enabled {
                        "Globally enabled"
                    } else {
                        "Globally disabled"
                    }
                    .into()
                });
                let mut detail = observation(
                    permission.global_enabled.state,
                    value,
                    permission.global_enabled.detail.as_deref(),
                );
                if permission.per_application_consent {
                    detail.push_str(" · Per-application consent");
                }
                let mut row = SettingsRow::new(theme, label, detail);
                if permission.mutation == nickel_platform::PermissionMutation::NativeConsent {
                    row = row.trailing(
                        Button::semantic(
                            theme,
                            SettingsMessage::MaintenanceAction(
                                nickel_platform::MaintenanceAction::OpenNativePermissionSettings(
                                    permission.kind,
                                ),
                            ),
                            "Manage",
                            ButtonPresentation::Quiet,
                        )
                        .width(92.0)
                        .enabled(!pending),
                    );
                }
                column = column.child(row);
            }
            AnyView::new(column)
        } else {
            AnyView::new(
                Text::new(
                    self.maintenance_status
                        .as_deref()
                        .unwrap_or("System status has not been loaded."),
                )
                .color(palette.muted),
            )
        };
        Column::new()
            .grow(1.0)
            .padding(Insets {
                top: 16.0,
                right: 24.0,
                bottom: 20.0,
                left: 20.0,
            })
            .gap(8.0)
            .child(
                Button::semantic(
                    theme,
                    SettingsMessage::MaintenanceRefresh,
                    "Refresh",
                    ButtonPresentation::Secondary,
                )
                .width(120.0)
                .enabled(self.maintenance_rx.is_none()),
            )
            .child(
                nickel_ui::VerticalScroll::new(SettingsMessage::MaintenanceScroll, 0.0)
                    .height(620.0)
                    .theme(theme)
                    .child(content),
            )
    }

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
        let codex = SettingsCard::titled(
            theme,
            "Codex integration",
            "Projects, conversations, and the built-in Codex client",
        )
        .child(
            SettingsRow::new(theme, "Enable Codex", status).trailing(
                Switch::with_state_action(switch_state, switch_action, theme)
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
        .child(retry);
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
            "On-screen keyboard",
            "Type with touch, a controller, or a mouse",
        )
        .child(mode)
        .child(SettingsRow::new(theme, "Current state", status))
        .child(Text::new("Keyboard test · text is not saved").color(theme.text.secondary))
        .child(
            Container::new()
                .fill_width()
                .height(48.0)
                .padding(Insets::all(4.0))
                .background(theme.surfaces.raised)
                .border(theme.borders.subtle, 1.0)
                .radius(theme.radii.control)
                .child(
                    TextField::on_change_with_placeholder(
                        &self.keyboard_preview,
                        "Try typing here — this text is not saved",
                        SettingsMessage::KeyboardPreviewChanged,
                    )
                    .id("on-screen-keyboard-preview")
                    .accessibility_label("Keyboard test")
                    .color(theme.text.primary)
                    .height(40.0),
                ),
        )
        .child(
            Button::semantic(
                theme,
                SettingsMessage::TryOnScreenKeyboard,
                "Try keyboard",
                ButtonPresentation::Secondary,
            )
            .id("on-screen-keyboard-try")
            .height(44.0)
            .width(160.0)
            .enabled(
                self.keyboard_runtime
                    .as_ref()
                    .is_some_and(|runtime| runtime.enabled),
            ),
        );
        let remote_effective = self.remote_control_runtime.effective;
        let remote_exposure = remote_exposure_presentation(&self.remote_control_runtime);
        let remote_switch = match remote_effective {
            nickel_session_protocol::RemoteControlEffectiveState::Enabled => SwitchState::On,
            nickel_session_protocol::RemoteControlEffectiveState::Disabled
                if !self.remote_control_settings.requested_enabled =>
            {
                SwitchState::Off
            }
            nickel_session_protocol::RemoteControlEffectiveState::Disabled
            | nickel_session_protocol::RemoteControlEffectiveState::Rejected => SwitchState::Mixed,
        };
        let pairing = if let Some(pairing) = &self.remote_pairing {
            let qr = self.remote_pairing_qr.as_ref().map_or_else(
                || AnyView::new(ui! { <Column /> }),
                |image| {
                    AnyView::new(
                        Image::new(64001, image.clone())
                            .width(240.0)
                            .height(240.0)
                            .fit(ImageFit::Contain)
                            .accessibility_label("Remote-control phone pairing QR code"),
                    )
                },
            );
            AnyView::new(
                SettingsCard::titled(
                    theme,
                    "Pair a phone",
                    "Scanning or entering this single-use code identifies a client. Control requires a separate local resource lease.",
                )
                .child(qr)
                .child(SettingsRow::new(theme, "Short code", pairing.short_code.clone()))
                .child(SettingsRow::new(
                    theme,
                    "Expires",
                    format!("Unix time {}", pairing.expires_at),
                ))
                .child(
                    Button::semantic(
                        theme,
                        SettingsMessage::CancelRemotePairing,
                        "Cancel pairing",
                        ButtonPresentation::Secondary,
                    )
                    .width(150.0),
                ),
            )
        } else {
            AnyView::new(ui! { <Column /> })
        };
        let pending_clients = self.remote_control_runtime.pending_clients.iter().fold(
            Column::new().fill_width().gap(12.0),
            |column, client| {
                column.child(
                    SettingsCard::titled(
                        theme,
                        format!("{} wants to connect", client.label),
                        "Approve this client identity locally. Desktop control requires a separate resource lease.",
                    )
                    .child(SettingsRow::new(
                        theme,
                        "Claimed client name",
                        client.label.clone(),
                    ))
                    .child(SettingsRow::new(
                        theme,
                        "Verified client identity",
                        "Unavailable until this connection is approved",
                    ))
                    .child(ui! { <Row gap={8.0}>
                        {Button::semantic(
                            theme,
                            SettingsMessage::DecideRemoteClient {
                                client_id: client.id.clone(),
                                decision: nickel_session_protocol::RemoteClientDecision::Deny,
                            },
                            "Deny",
                            ButtonPresentation::Destructive,
                        ).width(88.0)}
                        {Button::semantic(
                            theme,
                            SettingsMessage::DecideRemoteClient {
                                client_id: client.id.clone(),
                                decision: nickel_session_protocol::RemoteClientDecision::AllowOnce,
                            },
                            "Allow client",
                            ButtonPresentation::Primary,
                        ).width(180.0)}
                    </Row> }),
                )
            },
        );
        let pending_leases = self.remote_control_runtime.pending_leases.iter().fold(
            Column::new().fill_width().gap(12.0),
            |column, pending| {
                use nickel_session_protocol::RemoteResourceScope;
                let scope = match &pending.request.scope {
                    RemoteResourceScope::Surface(id) => format!("Nickel surface {}", id.id),
                    RemoteResourceScope::Window(id) => format!("Window {}", id.id),
                    RemoteResourceScope::Application(id) => format!("{id} windows"),
                    RemoteResourceScope::Output(id) => format!("Display {}", id.id),
                    RemoteResourceScope::FullSession if pending.request.full_debug => "Full Control & Debug Nickel".into(),
                    RemoteResourceScope::FullSession => "Full desktop".into(),
                };
                let scope = pending.resource_label.clone().unwrap_or(scope);
                let duration = pending.request.duration_seconds.map_or_else(
                    || "Until logout".to_owned(),
                    |seconds| if seconds % 3600 == 0 { format!("{} hours", seconds / 3600) }
                        else if seconds % 60 == 0 { format!("{} minutes", seconds / 60) }
                        else { format!("{seconds} seconds") },
                );
                let custom_seconds = self.remote_lease_custom_minutes.parse::<u64>().ok()
                    .filter(|minutes| *minutes > 0)
                    .and_then(|minutes| minutes.checked_mul(60));
                let duration_button = |label: String, duration_seconds| Button::semantic(theme,
                    SettingsMessage::ApproveRemoteLeaseDuration {
                        pending_generation: pending.pending_generation, client_id: pending.client_id.clone(), request: pending.request.clone(), duration_seconds,
                    }, label, ButtonPresentation::Secondary).width(160.0);
                let mut changes = Column::new().fill_width().gap(4.0);
                if pending.changes.access_changed {
                    changes = changes.child(SettingsStatus::new(theme, SettingsStatusKind::Validation,
                        "Access changed while this request was pending. Review scope, debug access, and reconnect policy."));
                }
                if pending.changes.duration_increased {
                    changes = changes.child(SettingsStatus::new(theme, SettingsStatusKind::Validation,
                        "A longer duration was requested while this approval was pending."));
                }
                let mut approval = SettingsCard::titled(theme,
                    format!("{} requests {}", pending.client_label,
                        if pending.request.renewal.is_some() { "lease renewal" } else { "control" }), scope.clone());
                if pending.request.full_debug {
                    let origin = self.remote_control_runtime.granted_clients.iter()
                        .find(|client| client.id == pending.client_id)
                        .and_then(|client| client.origin.as_ref())
                        .map_or_else(
                            || "Unavailable — no authenticated peer origin is retained".to_owned(),
                            |origin| format!("{} · {}", origin.address,
                                if origin.tls { "TLS protected" } else { "Local HTTP" }),
                        );
                    approval = approval
                        .child(SettingsStatus::new(
                            theme,
                            SettingsStatusKind::Validation,
                            "Broad approval permits repeated control and bounded diagnostics for this duration.",
                        ))
                        .child(SettingsRow::new(theme, "Resource", scope))
                        .child(SettingsRow::new(
                            theme,
                            "Diagnostic reach",
                            "Ordinary applications and Nickel surfaces; capture, input, bounded diagnostics, logs, traces, safe diagnostic actions, and typed nonprotected settings",
                        ))
                        .child(SettingsRow::new(
                            theme,
                            "Protected boundary",
                            "No lock or authentication surfaces, approval UI, Remote AI Control settings, trusted indicator, emergency controls, credentials, clipboard contents, or typed text history",
                        ))
                        .child(SettingsRow::new(
                            theme,
                            "Claimed client name",
                            pending.client_label.clone(),
                        ))
                        .child(SettingsRow::new(
                            theme,
                            "Verified client identity",
                            pending.client_id.clone(),
                        ))
                        .child(SettingsRow::new(theme, "Authenticated network origin", origin));
                }
                column.child(approval
                    .child(SettingsRow::new(theme, "Requested duration", duration))
                    .child(changes)
                    .child(SettingsRow::new(theme, "Reconnect", if pending.request.allow_resumption {
                        "May resume until this lease expires"
                    } else { "Approval ends on disconnect" }))
                    .child(ui! { <Row gap={8.0}>
                        {if pending.request.full_debug {
                            duration_button("Allow 30 minutes".into(), Some(1800))
                        } else {
                            duration_button("Allow 20 minutes".into(), Some(1200))
                        }}
                        {duration_button("Allow 2 hours".into(), Some(7200))}
                        {duration_button("Allow until logout".into(), None)}
                    </Row> })
                    .child(Text::new("Custom approval duration (minutes)").color(theme.text.secondary))
                    .child(ui! { <Row gap={8.0}>
                        {TextField::on_change_with_placeholder(&self.remote_lease_custom_minutes, "Minutes",
                            SettingsMessage::RemoteLeaseCustomMinutesChanged)
                            .id(format!("remote-lease-minutes-{}", pending.client_id))
                            .accessibility_label("Custom approval duration in minutes")
                            .color(theme.text.primary).width(150.0).height(40.0)}
                        {duration_button("Allow custom duration".into(), custom_seconds)
                            .enabled(custom_seconds.is_some()).width(200.0)}
                    </Row> })
                    .child(ui! { <Row gap={8.0}>
                        {Button::semantic(theme, SettingsMessage::DecideRemoteLease {
                            pending_generation: pending.pending_generation, client_id: pending.client_id.clone(), request: pending.request.clone(), allow: false,
                        }, "Deny", ButtonPresentation::Destructive).width(88.0)}
                        {Button::semantic(theme, SettingsMessage::DecideRemoteLease {
                            pending_generation: pending.pending_generation, client_id: pending.client_id.clone(), request: pending.request.clone(), allow: true,
                        }, if pending.request.full_debug { "Allow full debug" } else { "Allow control" }, ButtonPresentation::Primary).width(150.0)}
                        {Button::semantic(theme, SettingsMessage::BlockRemoteClient {
                            client_id: pending.client_id.clone(), blocked: true,
                        }, "Block client", ButtonPresentation::Destructive).width(130.0)}
                    </Row> }))
            },
        );
        let active_leases = self.remote_control_runtime.active_leases.iter().fold(
            Column::new().fill_width().gap(12.0),
            |column, lease| {
                use nickel_session_protocol::{RemoteLeaseAction, RemoteResourceScope};
                let scope = match &lease.scope {
                    RemoteResourceScope::Surface(id) => format!("Nickel surface {}", id.id),
                    RemoteResourceScope::Window(id) => format!("Window {}", id.id),
                    RemoteResourceScope::Application(id) => format!("{id} windows"),
                    RemoteResourceScope::Output(id) => format!("Display {}", id.id),
                    RemoteResourceScope::FullSession if lease.full_debug => "Full Control & Debug Nickel".into(),
                    RemoteResourceScope::FullSession => "Full desktop".into(),
                };
                let scope = lease.resource_label.clone().unwrap_or(scope);
                let remaining = lease.remaining_seconds.map_or_else(|| "Until logout".to_owned(),
                    |seconds| format!("{}m {}s remaining", seconds / 60, seconds % 60));
                column.child(SettingsCard::titled(theme, lease.client_label.clone(), scope)
                    .child(SettingsRow::new(theme, if lease.suspended { "Paused" } else { "Active" }, remaining))
                    .child(ui! { <Row gap={8.0}>
                        {Button::semantic(theme, SettingsMessage::ManageRemoteLease {
                            lease_id: lease.lease_id,
                            action: if lease.suspended { RemoteLeaseAction::Resume } else { RemoteLeaseAction::Pause },
                        }, if lease.suspended { "Resume" } else { "Pause" }, ButtonPresentation::Secondary).width(100.0)}
                        {Button::semantic(theme, SettingsMessage::ManageRemoteLease {
                            lease_id: lease.lease_id, action: RemoteLeaseAction::Revoke,
                        }, "Revoke", ButtonPresentation::Destructive).width(100.0)}
                    </Row> }))
            },
        );
        let granted_clients = self.remote_control_runtime.granted_clients.iter().fold(
            Column::new().fill_width().gap(12.0),
            |column, client| {
                column.child(
                    SettingsCard::titled(
                        theme,
                        client.label.clone(),
                        if client.blocked {
                            "Blocked: new permission requests are disabled"
                        } else if client.remembered {
                            "Remembered client"
                        } else {
                            "Connected client"
                        },
                    )
                    .child(SettingsRow::new(
                        theme,
                        "Verified client identity",
                        client.id.clone(),
                    ))
                    .child(SettingsRow::new(
                        theme,
                        "Last authenticated peer",
                        client.origin.as_ref().map_or_else(
                            || "Unavailable".to_owned(),
                            |origin| {
                                format!(
                                    "{} · {}",
                                    origin.address,
                                    if origin.tls {
                                        "TLS protected"
                                    } else {
                                        "Local HTTP"
                                    }
                                )
                            },
                        ),
                    ))
                    .child(
                        Button::semantic(
                            theme,
                            SettingsMessage::BlockRemoteClient {
                                client_id: client.id.clone(),
                                blocked: !client.blocked,
                            },
                            if client.blocked {
                                "Unblock client"
                            } else {
                                "Block client"
                            },
                            ButtonPresentation::Destructive,
                        )
                        .width(140.0),
                    )
                    .child(
                        Button::semantic(
                            theme,
                            SettingsMessage::RevokeRemoteClient(client.id.clone()),
                            "Revoke",
                            ButtonPresentation::Destructive,
                        )
                        .width(110.0),
                    ),
                )
            },
        );
        let remote = SettingsCard::titled(
            theme,
            "Remote AI Control",
            "Permission-gated desktop control for Codex and paired devices",
        )
        .child(
            SettingsRow::new(
                theme,
                "Allow remote connections",
                match remote_effective {
                    nickel_session_protocol::RemoteControlEffectiveState::Disabled => "Disabled",
                    nickel_session_protocol::RemoteControlEffectiveState::Enabled => {
                        "Listening. Connections do not grant control."
                    }
                    nickel_session_protocol::RemoteControlEffectiveState::Rejected => "Unavailable",
                },
            )
            .trailing(
                Switch::with_state_action(
                    remote_switch,
                    Some(SettingsMessage::SetRemoteControlEnabled(
                        remote_effective
                            != nickel_session_protocol::RemoteControlEffectiveState::Enabled,
                    )),
                    theme,
                )
                .id("optional-feature-remote-control")
                .accessibility_label("Allow Remote AI Control connections"),
            ),
        )
        .child(SettingsStatus::new(
            theme,
            remote_exposure.kind,
            remote_exposure.exposure,
        ))
        .child(
            SettingsRow::new(theme, "Audible control status", "Play local cues for control changes and expiration. Uses current output volume and mute.")
                .trailing(Switch::with_state_action(
                    if self.remote_control_settings.audible_indications { SwitchState::On } else { SwitchState::Off },
                    Some(SettingsMessage::SetRemoteAudibleIndications(!self.remote_control_settings.audible_indications)), theme,
                ).id("remote-control-audible-indications").accessibility_label("Audible Remote AI Control status")),
        )
        .child(SettingsRow::new(
            theme,
            "MCP endpoint",
            self.remote_control_runtime.endpoint.clone(),
        ))
        .child(SettingsRow::new(
            theme,
            "Protected transport",
            remote_exposure.transport,
        ))
        .child(SettingsRow::new(
            theme,
            "Connected client identities",
            self.remote_control_runtime
                .granted_clients
                .len()
                .to_string(),
        ))
        .child(
            Button::semantic(
                theme,
                SettingsMessage::CopyRemoteConnectionInfo,
                "Copy connection info",
                ButtonPresentation::Secondary,
            )
            .id("remote-control-copy-connection")
            .accessibility_label("Copy MCP endpoint and host fingerprint")
            .width(220.0),
        )
        .child(SettingsRow::new(
            theme,
            "Listener configuration",
            if self.remote_control_runtime.environment_override {
                "Process environment override"
            } else {
                "Default loopback address"
            },
        ))
        .child(SettingsRow::new(
            theme,
            "Host certificate SHA-256",
            self.remote_control_runtime
                .host_fingerprint
                .clone()
                .unwrap_or_else(|| "Local HTTP listener".into()),
        ))
        .child(SettingsRow::new(
            theme,
            "Emergency stop",
            "Press physical Left Control and Right Control together",
        ))
        .child(SettingsRow::new(
            theme,
            "Health",
            self.remote_control_runtime
                .diagnostic
                .clone()
                .unwrap_or_else(|| format!("{:?}", remote_effective)),
        ))
        .child(
            Button::semantic(
                theme,
                SettingsMessage::StartRemotePairing,
                "Pair a phone",
                ButtonPresentation::Secondary,
            )
            .width(150.0)
            .enabled(
                remote_effective == nickel_session_protocol::RemoteControlEffectiveState::Enabled
                    && self.remote_pairing.is_none(),
            ),
        )
        .child(
            Button::semantic(
                theme,
                SettingsMessage::StopRemoteControlNow,
                "Stop now",
                ButtonPresentation::Destructive,
            )
            .width(150.0)
            .enabled(
                remote_effective == nickel_session_protocol::RemoteControlEffectiveState::Enabled,
            ),
        );
        let audit = &self.remote_control_runtime.lease_audit;
        let shown = audit.len().min(16);
        let audit_summary = if audit.is_empty() {
            "No lease activity in this session.".to_owned()
        } else {
            format!(
                "Showing the latest {shown} of {} retained events. {} older events discarded.",
                audit.len(),
                self.remote_control_runtime.lease_audit_evicted,
            )
        };
        let audit_card = audit.iter().rev().take(16).fold(
            SettingsCard::titled(theme, "Recent control activity", audit_summary),
            |card, event| {
                use nickel_session_protocol::{
                    RemoteLeaseScopeKind as Scope, RemoteLeaseTransition as Transition,
                };
                let transition = match event.transition {
                    Transition::Approved => "Approved",
                    Transition::Renewed => "Renewed",
                    Transition::Paused => "Paused",
                    Transition::Resumed => "Resumed",
                    Transition::Expired => "Expired",
                    Transition::Revoked => "Revoked",
                    Transition::Disconnected => "Disconnected",
                    Transition::Reconnected => "Reconnected",
                };
                let scope = match event.scope {
                    Scope::Surface => "Nickel surface",
                    Scope::Window => "Window",
                    Scope::Application => "Application windows",
                    Scope::Output => "Display",
                    Scope::FullSession if event.full_debug => "Full Control & Debug Nickel",
                    Scope::FullSession => "Full desktop",
                };
                let lifetime = event.lifetime_limit_seconds.map_or_else(
                    || "until logout".to_owned(),
                    |seconds| format!("{seconds}s approved lifetime"),
                );
                card.child(SettingsRow::new(
                    theme,
                    format!("Lease {} · {transition}", event.lease_id),
                    format!(
                        "{scope} · {lifetime} · {}s after session start",
                        event.observed_at_us / 1_000_000
                    ),
                ))
            },
        );
        let origins = &self.remote_control_runtime.connection_audit;
        let connection_card = origins.iter().rev().take(16).fold(
            SettingsCard::titled(theme, "Recent connection origins", format!(
                "Latest {} of {} retained peer changes. {} older events discarded. Repeated requests from the same peer are combined.",
                origins.len().min(16), origins.len(), self.remote_control_runtime.connection_audit_evicted,
            )),
            |card, event| card.child(SettingsRow::new(
                theme,
                format!("{} · {}", event.address, if event.tls { "TLS protected" } else { "Local HTTP" }),
                format!("Client {} · {}s after session start", event.client_id, event.observed_at_us / 1_000_000),
            )),
        );
        let trace_audit = &self.remote_control_runtime.trace_audit;
        let trace_summary = if trace_audit.is_empty() {
            "No diagnostic traces in this session.".to_owned()
        } else {
            format!(
                "Showing the latest {} of {} retained events. {} older events discarded.",
                trace_audit.len().min(16),
                trace_audit.len(),
                self.remote_control_runtime.trace_audit_evicted
            )
        };
        let trace_card = trace_audit.iter().rev().take(16).fold(
            SettingsCard::titled(theme, "Recent diagnostic traces", trace_summary),
            |card, event| {
                use nickel_session_protocol::{
                    RemoteTraceCategory as Category, RemoteTraceTransition as Transition,
                };
                let transition = match event.transition {
                    Transition::Started => "Started",
                    Transition::Stopped => "Stopped",
                    Transition::TimedOut => "Time limit reached",
                    Transition::Cancelled => "Cancelled",
                };
                let category = match event.category {
                    Category::NestedFrameDispatch => "Nested frame timing",
                    Category::DrmFrameDispatch => "Display frame timing",
                };
                card.child(SettingsRow::new(
                    theme,
                    format!("Trace {} · {transition}", event.trace_id),
                    format!(
                        "{category} · Client {} · Lease {} · {}s limit · {:.2}s elapsed",
                        event.client_id,
                        event.lease_id,
                        event.duration_limit_seconds,
                        event.elapsed_us as f64 / 1_000_000.0
                    ),
                ))
            },
        );
        Column::new()
            .fill_width()
            .grow(1.0)
            .gap(16.0)
            .overflow_y(nickel_ui::Overflow::Scroll)
            .child(keyboard.shrink(0.0))
            .child(codex.shrink(0.0))
            .child(remote.shrink(0.0))
            .child(pairing)
            .child(pending_clients)
            .child(pending_leases)
            .child(active_leases)
            .child(granted_clients)
            .child(audit_card.shrink(0.0))
            .child(connection_card.shrink(0.0))
            .child(trace_card.shrink(0.0))
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
            AnyView::new(Text::new("Loading file and protocol associations…").color(palette.muted))
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
        let enabled = SettingsRow::new(theme, "Display enabled", "").trailing(
            Switch::new(selected.enabled, SettingsMessage::DisplayEnabled, theme)
                .id("display-enabled")
                .accessibility_label("Display enabled"),
        );
        let scale = SliderField::new(
            theme,
            "Scale",
            "Logical size on this display. Applications may need to redraw.",
            format!("{}%", selected.scale.units() * 100 / 120),
            (selected.scale.units().saturating_sub(60) as f32 / 420.0).clamp(0.0, 1.0),
            display_scale_message,
        )
        .id("display-scale")
        .stacked();
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
            .description("Clear Nickel-owned toolkit overrides and follow compositor scaling."),
            RadioOption::new(
                theme,
                SettingsMessage::ApplicationScaleUnchanged,
                "Leave unchanged",
                self.application_scale_policy == ApplicationScalePolicy::Unchanged,
            )
            .description("Do not write toolkit-wide compatibility settings."),
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
            .description("Use the custom application scale selected below."),
        ])
        .id("application-scale-policy");
        let app_scale = SettingsCard::titled(
            theme,
            "Application compatibility scale",
            "Toolkit compatibility is separate from per-display Wayland scale. Running applications may need a restart.",
        )
        .id("application-scale")
        .child(application_scale_policy_choices)
        .child(
            SliderField::new(
                theme,
                "Custom application scale",
                "Used only when custom compatibility scaling is selected.",
                format!("{}%", app_scale_units * 100 / 120),
                (app_scale_units.saturating_sub(60) as f32 / 420.0).clamp(0.0, 1.0),
                application_scale_message,
            )
            .id("application-custom-scale")
            .stacked(),
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
                    width={if compact_cards { (content_width - 120.0).max(120.0) } else { display.rect.w as f32 }}
                    height={if compact_cards { 220.0 } else { display.rect.h as f32 }}
                    min_width={if compact_cards { 120.0 } else { 160.0 }} min_height={220.0}
                    background={if !display.enabled { palette.background }
                        else if selected { palette.accent_soft } else { palette.surface }}
                    border={(border_color, border_width)} radius={theme.radii.card}
                    padding={Insets::all(18.0)}
                    on_drag={(SettingsMessage::SelectDisplay(index), display_drag_message)}
                    on_press={SettingsMessage::SelectDisplay(index)}
                    semantic_role={SemanticRole::Button}
                    accessibility_label={format!("{} display, {}", display.name, detail)}
                    accessibility_state={if selected { "selected" } else { "not selected" }}>
                    <Column gap={8.0}>
                        <Text scale={1.5} color={palette.text} wrap={true}>{&display.name}</Text>
                        <Text color={palette.muted} wrap={true}>{detail}</Text>
                        <Text bold={true} color={palette.accent}>
                            {if display.primary { "PRIMARY" } else { "" }}
                        </Text>
                    </Column>
                </Container>
            }
        }).collect::<Vec<_>>();
        let display_layout = if compact_cards {
            AnyView::new(Column::new().gap(12.0).children(display_cards))
        } else {
            AnyView::new(Row::new().gap(12.0).children(display_cards))
        };
        ui! {
            <Column grow={1.0} padding={Insets {
                top: 20.0, right: 32.0, bottom: 20.0, left: 20.0,
            }}>
                <VerticalScroll id={"display-page-scroll"} on_scroll={SettingsMessage::DisplayScroll}
                    grow={1.0}
                    offset={0.0} theme={theme}>
                    <Column gap={12.0}>
                        <Container id={"display-plane"} min_height={300.0}
                            background={palette.surface} border={(palette.muted, 1.0)}
                            padding={Insets::all(20.0)} align_items={nickel_ui::Align::Center}
                            justify_content={nickel_ui::Justify::Center}
                            semantic_role={SemanticRole::TabPanel}
                            accessibility_label={"Display arrangement"}>
                            {display_layout}
                        </Container>
                        <Container background={palette.surface} border={(palette.muted, 1.0)}
                            padding={Insets::all(12.0)}>
                            <Column gap={10.0}>
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

    pub(super) fn bluetooth_components(&self) -> impl nickel_ui::Component<SettingsMessage> {
        let palette = self.palette();
        let theme = self.ui_theme();
        let operation_pending = self.bluetooth_operation_rx.is_some();
        let device_list = self.bluetooth.devices.iter().enumerate().fold(
            SettingsListCard::new(theme),
            |list, (index, device)| {
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
                let supporting = if detail.is_empty() {
                    status
                } else {
                    format!("{status} · {detail}")
                };
                let available =
                    self.bluetooth.available && self.bluetooth.powered && !operation_pending;
                let action = if device.connected {
                    self.localizer.text("settings-bluetooth-disconnect")
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
            },
        );

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
        let discoverability = if self.bluetooth.discovering {
            self.localizer.text("settings-bluetooth-discovery-stop")
        } else {
            self.localizer.text("settings-bluetooth-discovery-start")
        };
        let discovery_available =
            self.bluetooth.available && self.bluetooth.powered && !operation_pending;
        let discovery_button = Button::semantic(
            self.ui_theme(),
            SettingsMessage::BluetoothDiscovery,
            discoverability,
            if discovery_available {
                ButtonPresentation::Secondary
            } else {
                ButtonPresentation::Disabled
            },
        )
        .width(150.0);
        let device_list = if self.bluetooth.devices.is_empty() {
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
                    offset={0.0} theme={theme}>{content}</VerticalScroll>
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
        let terminal_text_field =
            |id: &'static str,
             label: &'static str,
             value: String,
             placeholder: &'static str,
             on_change: fn(String) -> SettingsMessage| {
                Column::new()
                    .fill_width()
                    .gap(4.0)
                    .child(Text::new(label).color(theme.text.primary))
                    .child(
                        Container::new()
                            .fill_width()
                            .height(42.0)
                            .padding(Insets::all(4.0))
                            .background(theme.surfaces.raised)
                            .border(theme.borders.subtle, 1.0)
                            .radius(theme.radii.control)
                            .child(
                                TextField::on_change_with_placeholder(
                                    &value,
                                    placeholder,
                                    on_change,
                                )
                                .id(id)
                                .accessibility_label(label)
                                .color(theme.text.primary)
                                .height(34.0),
                            ),
                    )
            };
        let terminal_cursor = RadioGroup::new([
            RadioOption::new(
                theme,
                SettingsMessage::SetTerminalCursorStyle(
                    nickel_core::terminal_settings::TerminalCursorStyle::Block,
                ),
                "Block",
                self.terminal_settings.cursor_style
                    == nickel_core::terminal_settings::TerminalCursorStyle::Block,
            ),
            RadioOption::new(
                theme,
                SettingsMessage::SetTerminalCursorStyle(
                    nickel_core::terminal_settings::TerminalCursorStyle::Beam,
                ),
                "Beam",
                self.terminal_settings.cursor_style
                    == nickel_core::terminal_settings::TerminalCursorStyle::Beam,
            ),
            RadioOption::new(
                theme,
                SettingsMessage::SetTerminalCursorStyle(
                    nickel_core::terminal_settings::TerminalCursorStyle::Underline,
                ),
                "Underline",
                self.terminal_settings.cursor_style
                    == nickel_core::terminal_settings::TerminalCursorStyle::Underline,
            ),
        ])
        .id("terminal-cursor-style");
        let mut terminal_card = SettingsCard::titled(
            theme,
            "Terminal",
            "Defaults for new Nickel Terminal windows",
        )
        .id("appearance-terminal-card")
        .child(terminal_text_field(
            "terminal-shell",
            "Shell executable",
            self.terminal_settings
                .default_shell
                .clone()
                .unwrap_or_default(),
            "Use the platform default shell",
            SettingsMessage::TerminalShellChanged,
        ))
        .child(terminal_text_field(
            "terminal-working-directory",
            "Initial working directory",
            self.terminal_settings
                .initial_working_directory
                .as_deref()
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_default(),
            "Use the launching process directory",
            SettingsMessage::TerminalWorkingDirectoryChanged,
        ))
        .child(terminal_text_field(
            "terminal-font-family",
            "Fixed-width font family",
            self.terminal_settings.font_family.clone(),
            "monospace",
            SettingsMessage::TerminalFontFamilyChanged,
        ))
        .child(
            SliderField::new(
                theme,
                "Font size",
                "Logical pixels",
                format!("{:.1}", self.terminal_settings.font_size()),
                f32::from(self.terminal_settings.font_size_tenths.saturating_sub(60)) / 660.0,
                terminal_font_size_message,
            )
            .id("terminal-font-size"),
        )
        .child(
            SliderField::new(
                theme,
                "Scrollback",
                "Maximum retained history",
                self.terminal_settings.scrollback_lines.to_string(),
                self.terminal_settings.scrollback_lines as f32
                    / nickel_core::terminal_settings::MAX_TERMINAL_SCROLLBACK_LINES as f32,
                terminal_scrollback_message,
            )
            .id("terminal-scrollback"),
        )
        .child(
            Column::new()
                .fill_width()
                .gap(4.0)
                .child(Text::new("Cursor").color(theme.text.primary))
                .child(terminal_cursor),
        )
        .child(terminal_text_field(
            "terminal-foreground",
            "Foreground color",
            self.terminal_foreground_input.clone(),
            "#AARRGGBB",
            SettingsMessage::TerminalForegroundChanged,
        ))
        .child(terminal_text_field(
            "terminal-background",
            "Background color",
            self.terminal_background_input.clone(),
            "#AARRGGBB",
            SettingsMessage::TerminalBackgroundChanged,
        ))
        .child(
            SettingsRow::new(
                theme,
                "Close after successful command",
                "Failed commands remain visible",
            )
            .trailing(
                Switch::new(
                    self.terminal_settings.close_on_successful_exit,
                    SettingsMessage::SetTerminalCloseOnSuccess,
                    theme,
                )
                .id("terminal-close-on-success"),
            ),
        );
        if let Some(status) = &self.terminal_status {
            terminal_card = terminal_card.child(SettingsStatus::<SettingsMessage>::new(
                theme,
                if status.starts_with("Saved") {
                    SettingsStatusKind::Validation
                } else {
                    SettingsStatusKind::Error
                },
                status.clone(),
            ));
        }
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
            .child(terminal_card)
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
