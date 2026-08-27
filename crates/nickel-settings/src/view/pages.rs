use super::*;

impl SettingsApp {
    pub(super) fn display_components(&self) -> impl nickel_ui::Component<SettingsMessage> {
        let palette = self.palette();
        let selected = &self.displays[self.selected];
        ui! {
            <Column grow={1.0} padding={Insets {
                top: 24.0, right: 40.0, bottom: 20.0, left: 20.0,
            }} gap={12.0}>
                <Container height={340.0} background={palette.surface} border={(palette.muted, 2.0)} />
                <Row height={42.0} gap={15.0}>
                    <Text color={palette.text} width={183.0}>{&selected.name}</Text>
                    <Button on_press={SettingsMessage::DisplayIdentify} width={135.0} color={palette.text}
                        background={palette.surface_hover} border={(palette.muted, 1.0)}>
                        {self.localizer.text("settings-display-identify")}
                    </Button>
                    <Button on_press={SettingsMessage::DisplayPrimary} width={145.0} color={palette.text}
                        background={palette.surface_hover} border={(palette.muted, 1.0)}>
                        {self.localizer.text("settings-display-make-primary")}
                    </Button>
                    <Button on_press={SettingsMessage::DisplayApply} width={105.0} color={palette.text}
                        background={palette.accent}>{self.localizer.text("settings-display-apply")}</Button>
                </Row>
                <Text color={if self.applied { palette.complement } else { palette.muted }} height={18.0}>
                    {&self.status}
                </Text>
            </Column>
        }
    }

    pub(super) fn network_components(&self) -> impl nickel_ui::Component<SettingsMessage> {
        let palette = self.palette();
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
                        background={if self.hovered_message() == Some(&SettingsMessage::WifiNetwork(index)) {
                            palette.surface_hover
                        } else { palette.surface }}
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
        let content = ui! {
            <Column gap={12.0}>
                <Container height={58.0} background={palette.surface}
                    border={(if self.wifi_enabled { palette.accent } else { palette.muted }, 2.0)}
                    on_press={SettingsMessage::WifiPower}
                    padding={Insets { top: 10.0, right: 14.0, bottom: 10.0, left: 14.0 }}>
                    <Row>
                        <Text width={500.0} color={palette.text}>{self.localizer.text("settings-network-wifi")}</Text>
                        <Text bold={true} color={if self.wifi_enabled { palette.accent } else { palette.muted }}>
                            {if self.wifi_enabled {
                                self.localizer.text("settings-network-wifi-on")
                            } else if !self.network_available {
                                self.localizer.text("settings-network-wifi-unavailable")
                            } else {
                                self.localizer.text("settings-network-wifi-off")
                            }}
                        </Text>
                    </Row>
                </Container>
                <Row height={26.0}>
                    <Text color={palette.text} width={308.0}>{self.localizer.text("settings-network-visible-wifi")}</Text>
                    <Text scale={1.0} color={palette.muted}>{&self.wifi_status}</Text>
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
                    offset={self.transient_scroll(&SettingsMessage::NetworkScroll)}>{content}</VerticalScroll>
            </Column>
        }
    }

    pub(super) fn bluetooth_components(&self) -> impl nickel_ui::Component<SettingsMessage> {
        let palette = self.palette();
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
                        background={if self.hovered_message() == Some(&SettingsMessage::BluetoothDevice(index)) {
                            palette.surface_hover
                        } else { palette.surface }}
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
        let content = ui! {
            <Column gap={12.0}>
                <Container height={58.0} background={palette.surface} border={(palette.accent, 2.0)}
                    on_press={SettingsMessage::BluetoothPower}
                    padding={Insets { top: 10.0, right: 14.0, bottom: 10.0, left: 14.0 }}>
                    <Row>
                        <Column grow={1.0} gap={5.0}>
                            <Text color={palette.text}>{self.localizer.text("settings-bluetooth-enabled")}</Text>
                            <Text scale={1.0} color={palette.muted}>{if self.bluetooth.adapter_name.is_empty() {
                                self.localizer.text("settings-bluetooth-adapter-unnamed")
                            } else {
                                self.bluetooth.adapter_name.clone()
                            }}</Text>
                        </Column>
                        <Text bold={true} color={if self.bluetooth.powered { palette.accent } else { palette.muted }}>
                            {adapter_status}
                        </Text>
                    </Row>
                </Container>
                <Row height={36.0}>
                    <Text width={390.0} color={palette.text}>{self.localizer.text("settings-bluetooth-devices")}</Text>
                    <Button on_press={SettingsMessage::BluetoothDiscovery} width={150.0} color={palette.text}
                        background={palette.surface_hover} border={(palette.muted, 1.0)}>{discoverability}</Button>
                </Row>
                {device_list}
            </Column>
        };

        ui! {
            <Column grow={1.0} padding={Insets {
                top: 20.0, right: 40.0, bottom: 20.0, left: 20.0,
            }}>
                <VerticalScroll id={"bluetooth-list"} on_scroll={SettingsMessage::BluetoothScroll}
                    offset={self.transient_scroll(&SettingsMessage::BluetoothScroll)}>{content}</VerticalScroll>
            </Column>
        }
    }

    pub(super) fn bar_components(&self) -> impl nickel_ui::Component<SettingsMessage> {
        let palette = self.palette();
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
                    on_change={desktop_count_message} width={520.0} />
                <Row height={46.0} gap={8.0} children={desktop_choices} />
            </Column>
        }
    }

    pub(super) fn appearance_components(&self) -> impl nickel_ui::Component<SettingsMessage> {
        let system = nickel_platform::appearance();
        let appearance = self.shell_settings.resolve_appearance(system);
        let palette = ThemePalette::from_appearance(appearance);
        let hue = self.shell_settings.displayed_hue(system);
        let intensity = self.shell_settings.displayed_intensity(system);
        let swatches = [
            ("settings-swatch-background", palette.background),
            ("settings-swatch-panel", palette.panel),
            ("settings-swatch-surface", palette.surface),
            ("settings-swatch-hover", palette.surface_hover),
            ("settings-swatch-accent", palette.accent),
            ("settings-swatch-complement", palette.complement),
        ]
        .into_iter()
        .map(|(label, color)| {
            ui! {
                <Container width={88.0} height={76.0} background={color}
                    border={(palette.muted, 1.0)}
                    padding={Insets { top: 46.0, right: 4.0, bottom: 4.0, left: 4.0 }}>
                    <Text align={TextAlign::Center} scale={0.72} color={palette.text}>
                        {self.localizer.text(label)}
                    </Text>
                </Container>
            }
        });
        let wallpaper_name = self
            .wallpaper_settings
            .image
            .as_deref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.localizer.text("settings-wallpaper-none"));
        let positions = [
            ("settings-wallpaper-fill", WallpaperPosition::Fill),
            ("settings-wallpaper-fit", WallpaperPosition::Fit),
            ("settings-wallpaper-stretch", WallpaperPosition::Stretch),
            ("settings-wallpaper-center", WallpaperPosition::Center),
            ("settings-wallpaper-tile", WallpaperPosition::Tile),
            ("settings-wallpaper-span", WallpaperPosition::Span),
        ]
        .into_iter()
        .map(|(label, position)| ui! {
            <RadioButton on_press={SettingsMessage::WallpaperPosition(position)}
                label={self.localizer.text(label)} selected={self.wallpaper_settings.position == position}
                colors_pair={(palette.accent, palette.text)} width={82.0} />
        });
        ui! {
            <Column grow={1.0} padding={Insets {
                top: 24.0, right: 40.0, bottom: 20.0, left: 20.0,
            }} gap={14.0}>
                <Text color={palette.text} height={20.0}>{self.localizer.text("settings-appearance-mode")}</Text>
                <Row height={38.0} gap={28.0}>
                    <RadioButton on_press={SettingsMessage::AppearanceLight}
                        label={self.localizer.text("settings-appearance-light")}
                        selected={appearance.mode == ThemeMode::Light}
                        colors_pair={(if appearance.mode == ThemeMode::Light { palette.accent } else { palette.muted }, palette.text)}
                        width={180.0} />
                    <RadioButton on_press={SettingsMessage::AppearanceDark}
                        label={self.localizer.text("settings-appearance-dark")}
                        selected={appearance.mode == ThemeMode::Dark}
                        colors_pair={(if appearance.mode == ThemeMode::Dark { palette.accent } else { palette.muted }, palette.text)}
                        width={180.0} />
                </Row>
                <Text color={palette.text} height={20.0}>{self.localizer.text("settings-wallpaper-image")}</Text>
                <Row height={38.0} gap={12.0}>
                    <Button on_press={SettingsMessage::WallpaperChoose} width={150.0}
                        color={palette.text} background={palette.surface_hover}>
                        {self.localizer.text("settings-wallpaper-choose")}
                    </Button>
                    <Text color={palette.muted} width={370.0}>{wallpaper_name}</Text>
                </Row>
                <Row height={34.0} gap={10.0} children={positions} />
                <Text color={palette.text} height={20.0}>{self.localizer.text("settings-appearance-starting-hue")}</Text>
                <Text scale={1.0} color={palette.muted} height={18.0}>
                    {self.localizer.number("settings-appearance-hue-value", "degrees", i64::from(hue))}
                </Text>
                <Slider id={"appearance-hue"} value={f32::from(hue) / 359.0}
                    on_change={appearance_hue_message}
                    colors_triplet={(palette.surface, palette.accent, palette.text)} width={520.0} />
                <Text color={palette.text} height={20.0}>{self.localizer.text("settings-appearance-color-intensity")}</Text>
                <Text scale={1.0} color={palette.muted} height={18.0}>
                    {self.localizer.number("settings-appearance-intensity-value", "percent", i64::from(intensity))}
                </Text>
                <Slider id={"appearance-intensity"} value={f32::from(intensity) / 100.0}
                    on_change={appearance_intensity_message}
                    colors_triplet={(palette.surface, palette.accent, palette.text)} width={520.0} />
                <Text color={palette.text} height={20.0}>{self.localizer.text("settings-appearance-color-palette")}</Text>
                <Row height={76.0} gap={8.0} children={swatches} />
            </Column>
        }
    }
}
