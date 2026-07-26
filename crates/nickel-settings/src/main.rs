#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

#[cfg(target_os = "windows")]
use std::collections::HashMap;

use nickel_components::{
    Button as UiButton, Column, ComponentGpu, Container, Insets, LinearGradient, PaintCommand,
    Point as UiPoint, RadioButton, Rect as UiRect, Row, Slider, Text, TextAlign, UiTree,
};
use nickel_core::{
    shell_settings::{ShellSettings, ThemePreference},
    theme::{ThemeMode, ThemePalette},
};
use nickel_i18n::Localizer;
use winit::{
    application::ApplicationHandler,
    dpi::{LogicalSize, PhysicalPosition},
    event::{ElementState, MouseButton, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    window::{Icon, Window, WindowAttributes, WindowId},
};

const SIDEBAR_WIDTH: i32 = 190;
const DISPLAY_PLANE: Rect = Rect {
    x: 210,
    y: 96,
    w: 600,
    h: 340,
};

#[derive(Clone, Copy)]
struct Rect {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

struct OutputSnapshot {
    name: String,
    model: String,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    physical_width: i32,
    physical_height: i32,
    primary: bool,
}

fn parse_output(line: &str) -> Option<OutputSnapshot> {
    let mut fields = line.split('\t');
    Some(OutputSnapshot {
        name: fields.next()?.to_owned(),
        model: fields.next()?.to_owned(),
        x: fields.next()?.parse().ok()?,
        y: fields.next()?.parse().ok()?,
        width: fields.next()?.parse().ok()?,
        height: fields.next()?.parse().ok()?,
        physical_width: fields.next()?.parse().ok()?,
        physical_height: fields.next()?.parse().ok()?,
        primary: fields.next()? == "1",
    })
}

#[cfg(target_os = "windows")]
fn windows_display_names() -> HashMap<String, String> {
    use std::mem::size_of;
    use windows::Win32::{
        Devices::Display::{
            DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
            DISPLAYCONFIG_DEVICE_INFO_HEADER, DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO,
            DISPLAYCONFIG_SOURCE_DEVICE_NAME, DISPLAYCONFIG_TARGET_DEVICE_NAME,
            DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes, QDC_ONLY_ACTIVE_PATHS,
            QueryDisplayConfig,
        },
        Foundation::ERROR_SUCCESS,
    };

    let mut path_count = 0;
    let mut mode_count = 0;
    // SAFETY: The count pointers are valid writable storage.
    if unsafe {
        GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut path_count, &mut mode_count)
    } != ERROR_SUCCESS
    {
        return HashMap::new();
    }
    let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); path_count as usize];
    let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); mode_count as usize];
    // SAFETY: Both vectors have the capacities reported immediately above and the counts remain
    // writable so Windows can report how many entries it populated.
    if unsafe {
        QueryDisplayConfig(
            QDC_ONLY_ACTIVE_PATHS,
            &mut path_count,
            paths.as_mut_ptr(),
            &mut mode_count,
            modes.as_mut_ptr(),
            None,
        )
    } != ERROR_SUCCESS
    {
        return HashMap::new();
    }
    paths.truncate(path_count as usize);
    let mut names = HashMap::new();
    for path in paths {
        let mut source = DISPLAYCONFIG_SOURCE_DEVICE_NAME {
            header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                r#type: DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
                size: size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32,
                adapterId: path.sourceInfo.adapterId,
                id: path.sourceInfo.id,
            },
            ..Default::default()
        };
        let mut target = DISPLAYCONFIG_TARGET_DEVICE_NAME {
            header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                r#type: DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
                size: size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32,
                adapterId: path.targetInfo.adapterId,
                id: path.targetInfo.id,
            },
            ..Default::default()
        };
        // SAFETY: Each request packet has the documented type, size, adapter, and source/target ID.
        if unsafe { DisplayConfigGetDeviceInfo(&raw mut source.header) } != 0
            || unsafe { DisplayConfigGetDeviceInfo(&raw mut target.header) } != 0
        {
            continue;
        }
        let source_name = wide_text(&source.viewGdiDeviceName);
        let friendly_name = wide_text(&target.monitorFriendlyDeviceName);
        if !source_name.is_empty() && !friendly_name.is_empty() {
            names.insert(source_name, friendly_name);
        }
    }
    names
}

#[cfg(target_os = "windows")]
fn wide_text(buffer: &[u16]) -> String {
    let length = buffer
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..length])
}

#[cfg(target_os = "linux")]
fn session_request(command: &str) -> std::io::Result<String> {
    use std::{os::unix::net::UnixDatagram, path::PathBuf, time::Duration};

    let server = std::env::var_os("NICKEL_SESSION_CONTROL")
        .map(PathBuf::from)
        .ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "Nickel session socket")
        })?;
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let client = runtime.join(format!("nickel-settings-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&client);
    let socket = UnixDatagram::bind(&client)?;
    socket.set_read_timeout(Some(Duration::from_secs(1)))?;
    let result = (|| {
        socket.send_to(command.as_bytes(), server)?;
        let mut response = [0_u8; 4096];
        let length = socket.recv(&mut response)?;
        Ok(String::from_utf8_lossy(&response[..length]).into_owned())
    })();
    let _ = std::fs::remove_file(client);
    result
}

#[cfg(not(target_os = "linux"))]
fn session_request(_command: &str) -> std::io::Result<String> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "live display settings are currently Linux-only",
    ))
}

impl Rect {
    fn contains(self, x: i32, y: i32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }
}

struct DisplayCard {
    connector: String,
    name: String,
    detail: String,
    logical_width: i32,
    logical_height: i32,
    rect: Rect,
    primary: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SettingsPage {
    Display,
    Bar,
    Appearance,
    Network,
}

struct NetworkAdapter {
    name: String,
    description: String,
    connected: bool,
    speed: u64,
}

struct WifiNetwork {
    profile: String,
    signal: u32,
    connected: bool,
    interface: u128,
}

struct SettingsApp {
    localizer: Localizer,
    window: Option<Arc<Window>>,
    gpu: Option<ComponentGpu>,
    displays: Vec<DisplayCard>,
    selected: usize,
    cursor: (i32, i32),
    drag_offset: Option<(i32, i32)>,
    applied: bool,
    pixels_per_logical: f64,
    status: String,
    page: SettingsPage,
    shell_settings: ShellSettings,
    desktop_slider_dragging: bool,
    appearance_slider_dragging: bool,
    intensity_slider_dragging: bool,
    window_icon_color: Option<u32>,
    network_adapters: Vec<NetworkAdapter>,
    wifi_networks: Vec<WifiNetwork>,
    wifi_status: String,
    pending_wifi_profile: Option<String>,
    next_wifi_refresh: Option<Instant>,
    wifi_refreshes_left: u8,
    ui: UiTree,
}

impl Default for SettingsApp {
    fn default() -> Self {
        let localizer = Localizer::system();
        let status = localizer.text("settings-status-changes-not-applied");
        Self {
            localizer,
            window: None,
            gpu: None,
            displays: vec![
                DisplayCard {
                    connector: "DVI-I-1".into(),
                    name: "ASUS MB16ACV".into(),
                    detail: "DISPLAYLINK  1920 X 1080".into(),
                    logical_width: 1920,
                    logical_height: 1080,
                    rect: Rect {
                        x: 125,
                        y: 205,
                        w: 270,
                        h: 160,
                    },
                    primary: false,
                },
                DisplayCard {
                    connector: "DP-3".into(),
                    name: "DP-3".into(),
                    detail: "NVIDIA  1920 X 1080".into(),
                    logical_width: 1920,
                    logical_height: 1080,
                    rect: Rect {
                        x: 405,
                        y: 185,
                        w: 300,
                        h: 180,
                    },
                    primary: true,
                },
            ],
            selected: 1,
            cursor: (0, 0),
            drag_offset: None,
            applied: false,
            pixels_per_logical: 0.14,
            status,
            page: SettingsPage::Display,
            shell_settings: ShellSettings::load_default(),
            desktop_slider_dragging: false,
            appearance_slider_dragging: false,
            intensity_slider_dragging: false,
            window_icon_color: None,
            network_adapters: Vec::new(),
            wifi_networks: Vec::new(),
            wifi_status: String::new(),
            pending_wifi_profile: None,
            next_wifi_refresh: None,
            wifi_refreshes_left: 0,
            ui: UiTree::default(),
        }
    }
}

impl SettingsApp {
    #[cfg(any())]
    fn display_nav() -> Rect {
        Rect {
            x: 12,
            y: 118,
            w: 146,
            h: 46,
        }
    }

    #[cfg(any())]
    fn network_nav() -> Rect {
        Rect {
            x: 12,
            y: 172,
            w: 146,
            h: 46,
        }
    }

    #[cfg(any())]
    fn wifi_row(index: usize) -> Rect {
        Rect {
            x: 190,
            y: 150 + index as i32 * 52,
            w: 620,
            h: 44,
        }
    }

    #[cfg(any())]
    fn primary_button() -> Rect {
        Rect {
            x: 540,
            y: 510,
            w: 145,
            h: 42,
        }
    }

    #[cfg(any())]
    fn identify_button() -> Rect {
        Rect {
            x: 390,
            y: 510,
            w: 135,
            h: 42,
        }
    }

    #[cfg(any())]
    fn apply_button() -> Rect {
        Rect {
            x: 700,
            y: 510,
            w: 105,
            h: 42,
        }
    }

    fn request_redraw(&self) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn palette(&self) -> ThemePalette {
        ThemePalette::from_appearance(
            self.shell_settings
                .resolve_appearance(nickel_platform::appearance()),
        )
    }

    fn navigation_item(
        &self,
        action: &'static str,
        message: &'static str,
        glyph: &'static str,
        selected: bool,
        palette: ThemePalette,
    ) -> Container {
        let label = self.localizer.text(message);
        let underline_width = (label.chars().count() as f32 * 8.0).clamp(24.0, 112.0);
        let mut underline = Container::new().width(underline_width).height(2.0);
        if selected {
            underline = underline.background(palette.accent);
        }
        Container::new()
            .width((SIDEBAR_WIDTH - 24) as f32)
            .height(36.0)
            .padding(Insets {
                top: 4.0,
                right: 8.0,
                bottom: 2.0,
                left: 8.0,
            })
            .action(action)
            .child(
                Row::new()
                    .gap(10.0)
                    .child(Text::new(glyph).width(22.0).scale(1.6).color(if selected {
                        palette.accent
                    } else {
                        palette.muted
                    }))
                    .child(
                        Column::new()
                            .gap(2.0)
                            .child(
                                Text::new(label)
                                    .height(20.0)
                                    .scale(2.0)
                                    .bold(selected)
                                    .color(palette.text),
                            )
                            .child(underline),
                    ),
            )
    }

    fn build_ui(&self, width: f32, height: f32) -> UiTree {
        let palette = self.palette();
        let (title, subtitle) = match self.page {
            SettingsPage::Display => (
                self.localizer.text("settings-display-title"),
                self.localizer.text("settings-display-subtitle"),
            ),
            SettingsPage::Bar => (
                self.localizer.text("settings-bar-title"),
                self.localizer.text("settings-bar-subtitle"),
            ),
            SettingsPage::Appearance => (
                self.localizer.text("settings-appearance-title"),
                self.localizer.text("settings-appearance-subtitle"),
            ),
            SettingsPage::Network => (
                self.localizer.text("settings-network-title"),
                self.localizer.text("settings-network-subtitle"),
            ),
        };
        let header_content = Container::new()
            .grow(1.0)
            .height(72.0)
            .background(palette.panel)
            .padding(Insets {
                top: 11.0,
                right: 40.0,
                bottom: 8.0,
                left: 20.0,
            })
            .child(
                Column::new()
                    .gap(4.0)
                    .child(Text::new(title).scale(3.0).color(palette.text))
                    .child(Text::new(subtitle).scale(1.0).color(palette.muted)),
            );
        let header = Row::new()
            .height(72.0)
            .child(
                Container::new()
                    .width(SIDEBAR_WIDTH as f32)
                    .height(72.0)
                    .background(palette.panel),
            )
            .child(header_content);
        let display_button = self.navigation_item(
            "nav:display",
            "settings-nav-display",
            "▣",
            self.page == SettingsPage::Display,
            palette,
        );
        let bar_button = self.navigation_item(
            "nav:bar",
            "settings-nav-bar",
            "▤",
            self.page == SettingsPage::Bar,
            palette,
        );
        let appearance_button = self.navigation_item(
            "nav:appearance",
            "settings-nav-appearance",
            "◐",
            self.page == SettingsPage::Appearance,
            palette,
        );
        let network_button = self.navigation_item(
            "nav:network",
            "settings-nav-network",
            "⌁",
            self.page == SettingsPage::Network,
            palette,
        );
        let sidebar = Column::new()
            .width(SIDEBAR_WIDTH as f32)
            .background(LinearGradient::vertical(palette.panel, palette.background))
            .padding(Insets {
                top: 20.0,
                right: 12.0,
                bottom: 12.0,
                left: 12.0,
            })
            .gap(4.0)
            .child(display_button)
            .child(bar_button)
            .child(appearance_button)
            .child(network_button);

        let content = match self.page {
            SettingsPage::Display => self.display_components(),
            SettingsPage::Bar => self.bar_components(),
            SettingsPage::Appearance => self.appearance_components(),
            SettingsPage::Network => self.network_components(),
        };
        let root = Column::new()
            .height(height)
            .background(palette.background)
            .child(header)
            .child(Row::new().grow(1.0).child(sidebar).child(content));
        UiTree::layout(root, UiRect::new(0.0, 0.0, width, height))
    }

    fn display_components(&self) -> Column {
        let palette = self.palette();
        let selected = &self.displays[self.selected];
        Column::new()
            .grow(1.0)
            .padding(Insets {
                top: 24.0,
                right: 40.0,
                bottom: 20.0,
                left: 20.0,
            })
            .gap(12.0)
            .child(
                Container::new()
                    .height(340.0)
                    .background(palette.surface)
                    .border(palette.muted, 2.0),
            )
            .child(
                Row::new()
                    .height(42.0)
                    .gap(15.0)
                    .child(Text::new(&selected.name).color(palette.text).width(183.0))
                    .child(
                        UiButton::new(
                            "display:identify",
                            self.localizer.text("settings-display-identify"),
                        )
                        .width(135.0)
                        .color(palette.text)
                        .background(palette.surface_hover)
                        .border(palette.muted, 1.0),
                    )
                    .child(
                        UiButton::new(
                            "display:primary",
                            self.localizer.text("settings-display-make-primary"),
                        )
                        .width(145.0)
                        .color(palette.text)
                        .background(palette.surface_hover)
                        .border(palette.muted, 1.0),
                    )
                    .child(
                        UiButton::new(
                            "display:apply",
                            self.localizer.text("settings-display-apply"),
                        )
                        .width(105.0)
                        .color(palette.text)
                        .background(palette.accent),
                    ),
            )
            .child(
                Text::new(&self.status)
                    .color(if self.applied {
                        palette.complement
                    } else {
                        palette.muted
                    })
                    .height(18.0),
            )
    }

    fn network_components(&self) -> Column {
        let palette = self.palette();
        let wifi_cards = self
            .wifi_networks
            .iter()
            .take(4)
            .enumerate()
            .map(|(index, network)| {
                let detail = if network.connected {
                    self.localizer.number(
                        "settings-network-connected-signal",
                        "signal",
                        i64::from(network.signal),
                    )
                } else if network.signal == 0 {
                    self.localizer.text("settings-network-saved-unavailable")
                } else {
                    self.localizer.number(
                        "settings-network-connect-action",
                        "signal",
                        i64::from(network.signal),
                    )
                };
                Container::new()
                    .height(44.0)
                    .background(palette.surface)
                    .border(palette.muted, 1.0)
                    .padding(Insets {
                        top: 12.0,
                        right: 14.0,
                        bottom: 8.0,
                        left: 14.0,
                    })
                    .action(format!("wifi:{index}"))
                    .child(
                        Row::new()
                            .child(Text::new(&network.profile).color(palette.text).width(316.0))
                            .child(Text::new(detail).scale(1.0).color(if network.connected {
                                palette.complement
                            } else {
                                palette.muted
                            })),
                    )
            });
        let adapter_cards = self.network_adapters.iter().take(2).map(|adapter| {
            let status = if adapter.connected {
                self.localizer.number(
                    "settings-network-connected-speed",
                    "speed",
                    (adapter.speed / 1_000_000) as i64,
                )
            } else {
                self.localizer.text("settings-network-disconnected")
            };
            Container::new()
                .height(72.0)
                .background(palette.surface)
                .border(palette.muted, 1.0)
                .padding(Insets {
                    top: 11.0,
                    right: 14.0,
                    bottom: 8.0,
                    left: 14.0,
                })
                .child(
                    Column::new()
                        .gap(7.0)
                        .child(Text::new(&adapter.name).color(palette.text))
                        .child(
                            Row::new()
                                .child(Text::new(status).color(if adapter.connected {
                                    palette.complement
                                } else {
                                    palette.muted
                                }))
                                .child(
                                    Text::new(&adapter.description)
                                        .scale(1.0)
                                        .color(palette.muted),
                                ),
                        ),
                )
        });
        Column::new()
            .grow(1.0)
            .padding(Insets {
                top: 20.0,
                right: 40.0,
                bottom: 20.0,
                left: 20.0,
            })
            .gap(8.0)
            .child(
                Row::new()
                    .height(26.0)
                    .child(
                        Text::new(self.localizer.text("settings-network-saved-wifi"))
                            .color(palette.text)
                            .width(308.0),
                    )
                    .child(Text::new(&self.wifi_status).scale(1.0).color(palette.muted)),
            )
            .child(
                Column::new()
                    .height((self.wifi_networks.len().min(4) as f32 * 52.0).max(44.0))
                    .gap(8.0)
                    .children(wifi_cards),
            )
            .child(
                Text::new(self.localizer.text("settings-network-adapters"))
                    .color(palette.text)
                    .height(18.0),
            )
            .child(Column::new().gap(12.0).children(adapter_cards))
    }

    fn bar_components(&self) -> Column {
        let palette = self.palette();
        let display_count = self.displays.len().max(1);
        let desktop_choices = (0..self.shell_settings.desktop_count).map(|index| {
            Container::new()
                .width(64.0)
                .height(46.0)
                .background(palette.surface)
                .border(
                    if index == 0 {
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
                    Text::new(format!("{}", index + 1))
                        .align(TextAlign::Center)
                        .scale(1.0)
                        .color(if index == 0 {
                            palette.text
                        } else {
                            palette.muted
                        }),
                )
        });
        Column::new()
            .grow(1.0)
            .padding(Insets {
                top: 24.0,
                right: 40.0,
                bottom: 20.0,
                left: 20.0,
            })
            .gap(14.0)
            .child(
                Text::new(self.localizer.text("settings-bar-show-on"))
                    .color(palette.text)
                    .height(20.0),
            )
            .child(
                Row::new()
                    .height(38.0)
                    .gap(28.0)
                    .child(
                        RadioButton::new(
                            "bar:displays:primary",
                            self.localizer.text("settings-bar-primary-display"),
                            !self.shell_settings.bar_on_all_displays,
                        )
                        .colors(
                            if !self.shell_settings.bar_on_all_displays {
                                palette.accent
                            } else {
                                palette.muted
                            },
                            palette.text,
                        )
                        .width(210.0),
                    )
                    .child(
                        RadioButton::new(
                            "bar:displays:all",
                            self.localizer.number(
                                "settings-bar-all-displays",
                                "count",
                                display_count as i64,
                            ),
                            self.shell_settings.bar_on_all_displays,
                        )
                        .colors(
                            if self.shell_settings.bar_on_all_displays {
                                palette.accent
                            } else {
                                palette.muted
                            },
                            palette.text,
                        )
                        .width(210.0),
                    ),
            )
            .child(
                Text::new(self.localizer.text("settings-bar-window-scope"))
                    .color(palette.text)
                    .height(20.0),
            )
            .child(
                Row::new()
                    .height(38.0)
                    .gap(28.0)
                    .child(
                        RadioButton::new(
                            "bar:windows:display",
                            self.localizer.text("settings-bar-this-display"),
                            !self.shell_settings.all_windows_on_every_bar,
                        )
                        .colors(
                            if !self.shell_settings.all_windows_on_every_bar {
                                palette.accent
                            } else {
                                palette.muted
                            },
                            palette.text,
                        )
                        .width(210.0),
                    )
                    .child(
                        RadioButton::new(
                            "bar:windows:all",
                            self.localizer.text("settings-bar-all-windows"),
                            self.shell_settings.all_windows_on_every_bar,
                        )
                        .colors(
                            if self.shell_settings.all_windows_on_every_bar {
                                palette.accent
                            } else {
                                palette.muted
                            },
                            palette.text,
                        )
                        .width(210.0),
                    ),
            )
            .child(
                Text::new(self.localizer.text("settings-bar-desktops"))
                    .color(palette.text)
                    .height(20.0),
            )
            .child(
                Text::new(self.localizer.number(
                    "settings-bar-desktop-count",
                    "count",
                    i64::from(self.shell_settings.desktop_count),
                ))
                .scale(1.0)
                .color(palette.muted)
                .height(18.0),
            )
            .child(
                Slider::new(
                    "bar:desktop-count",
                    f32::from(self.shell_settings.desktop_count.saturating_sub(1)) / 7.0,
                )
                .width(520.0),
            )
            .child(Row::new().height(46.0).gap(8.0).children(desktop_choices))
    }

    fn appearance_components(&self) -> Column {
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
            Container::new()
                .width(88.0)
                .height(76.0)
                .background(color)
                .border(palette.muted, 1.0)
                .padding(Insets {
                    top: 46.0,
                    right: 4.0,
                    bottom: 4.0,
                    left: 4.0,
                })
                .child(
                    Text::new(self.localizer.text(label))
                        .align(TextAlign::Center)
                        .scale(0.72)
                        .color(palette.text),
                )
        });
        Column::new()
            .grow(1.0)
            .padding(Insets {
                top: 24.0,
                right: 40.0,
                bottom: 20.0,
                left: 20.0,
            })
            .gap(14.0)
            .child(
                Text::new(self.localizer.text("settings-appearance-mode"))
                    .color(palette.text)
                    .height(20.0),
            )
            .child(
                Row::new()
                    .height(38.0)
                    .gap(28.0)
                    .child(
                        RadioButton::new(
                            "appearance:light",
                            self.localizer.text("settings-appearance-light"),
                            appearance.mode == ThemeMode::Light,
                        )
                        .colors(
                            if appearance.mode == ThemeMode::Light {
                                palette.accent
                            } else {
                                palette.muted
                            },
                            palette.text,
                        )
                        .width(180.0),
                    )
                    .child(
                        RadioButton::new(
                            "appearance:dark",
                            self.localizer.text("settings-appearance-dark"),
                            appearance.mode == ThemeMode::Dark,
                        )
                        .colors(
                            if appearance.mode == ThemeMode::Dark {
                                palette.accent
                            } else {
                                palette.muted
                            },
                            palette.text,
                        )
                        .width(180.0),
                    ),
            )
            .child(
                Text::new(self.localizer.text("settings-appearance-starting-hue"))
                    .color(palette.text)
                    .height(20.0),
            )
            .child(
                Text::new(self.localizer.number(
                    "settings-appearance-hue-value",
                    "degrees",
                    i64::from(hue),
                ))
                .scale(1.0)
                .color(palette.muted)
                .height(18.0),
            )
            .child(
                Slider::new("appearance:hue", f32::from(hue) / 359.0)
                    .colors(palette.surface, palette.accent, palette.text)
                    .width(520.0),
            )
            .child(
                Text::new(self.localizer.text("settings-appearance-color-intensity"))
                    .color(palette.text)
                    .height(20.0),
            )
            .child(
                Text::new(self.localizer.number(
                    "settings-appearance-intensity-value",
                    "percent",
                    i64::from(intensity),
                ))
                .scale(1.0)
                .color(palette.muted)
                .height(18.0),
            )
            .child(
                Slider::new("appearance:intensity", f32::from(intensity) / 100.0)
                    .colors(palette.surface, palette.accent, palette.text)
                    .width(520.0),
            )
            .child(
                Text::new(self.localizer.text("settings-appearance-color-palette"))
                    .color(palette.text)
                    .height(20.0),
            )
            .child(Row::new().height(76.0).gap(8.0).children(swatches))
    }

    fn pointer_pressed(&mut self) {
        let (x, y) = self.cursor;
        let point = UiPoint {
            x: x as f32,
            y: y as f32,
        };
        if let Some(("bar:desktop-count", fraction)) =
            self.ui.action_at_with_horizontal_fraction(point)
        {
            self.desktop_slider_dragging = true;
            self.set_desktop_count_from_fraction(fraction);
            self.request_redraw();
            return;
        }
        if let Some(("appearance:hue", fraction)) =
            self.ui.action_at_with_horizontal_fraction(point)
        {
            self.appearance_slider_dragging = true;
            self.set_appearance_hue_from_fraction(fraction);
            self.request_redraw();
            return;
        }
        if let Some(("appearance:intensity", fraction)) =
            self.ui.action_at_with_horizontal_fraction(point)
        {
            self.intensity_slider_dragging = true;
            self.set_appearance_intensity_from_fraction(fraction);
            self.request_redraw();
            return;
        }
        let action = self.ui.action_at(point).map(str::to_owned);
        if let Some(action) = action {
            match action.as_str() {
                "nav:display" => self.page = SettingsPage::Display,
                "nav:bar" => self.page = SettingsPage::Bar,
                "nav:appearance" => self.page = SettingsPage::Appearance,
                "nav:network" => self.page = SettingsPage::Network,
                "appearance:light" => {
                    self.shell_settings.theme = ThemePreference::Light;
                    let _ = self.shell_settings.save_default();
                }
                "appearance:dark" => {
                    self.shell_settings.theme = ThemePreference::Dark;
                    let _ = self.shell_settings.save_default();
                }
                "bar:displays:primary" => {
                    self.shell_settings.bar_on_all_displays = false;
                    let _ = self.shell_settings.save_default();
                }
                "bar:displays:all" => {
                    self.shell_settings.bar_on_all_displays = true;
                    let _ = self.shell_settings.save_default();
                }
                "bar:windows:display" => {
                    self.shell_settings.all_windows_on_every_bar = false;
                    let _ = self.shell_settings.save_default();
                }
                "bar:windows:all" => {
                    self.shell_settings.all_windows_on_every_bar = true;
                    let _ = self.shell_settings.save_default();
                }
                "display:identify" => match session_request("identify-outputs") {
                    Ok(response) if response == "ok" => {
                        self.status = self.localizer.text("settings-status-identifying")
                    }
                    _ => self.status = self.localizer.text("settings-status-identify-failed"),
                },
                "display:primary" => {
                    for (index, display) in self.displays.iter_mut().enumerate() {
                        display.primary = index == self.selected;
                    }
                    self.applied = false;
                    self.status = self.localizer.text("settings-status-changes-not-applied");
                }
                "display:apply" => self.apply_layout(),
                _ if action.starts_with("wifi:") => {
                    if let Ok(index) = action["wifi:".len()..].parse() {
                        self.connect_windows_wifi(index);
                    }
                }
                _ => {}
            }
            self.request_redraw();
            return;
        }
        if self.page != SettingsPage::Display {
            self.request_redraw();
            return;
        } else if let Some(index) = self
            .displays
            .iter()
            .rposition(|display| display.rect.contains(x, y))
        {
            self.selected = index;
            let rect = self.displays[index].rect;
            self.drag_offset = Some((x - rect.x, y - rect.y));
            self.applied = false;
            self.status = self.localizer.text("settings-status-changes-not-applied");
        }
        self.request_redraw();
    }

    fn pointer_moved(&mut self, position: PhysicalPosition<f64>) {
        self.cursor = (position.x.round() as i32, position.y.round() as i32);
        if self.desktop_slider_dragging {
            if let Some(fraction) = self
                .ui
                .horizontal_fraction_for_action("bar:desktop-count", position.x as f32)
            {
                self.set_desktop_count_from_fraction(fraction);
                self.request_redraw();
            }
            return;
        }
        if self.appearance_slider_dragging {
            if let Some(fraction) = self
                .ui
                .horizontal_fraction_for_action("appearance:hue", position.x as f32)
            {
                self.set_appearance_hue_from_fraction(fraction);
                self.request_redraw();
            }
            return;
        }
        if self.intensity_slider_dragging {
            if let Some(fraction) = self
                .ui
                .horizontal_fraction_for_action("appearance:intensity", position.x as f32)
            {
                self.set_appearance_intensity_from_fraction(fraction);
                self.request_redraw();
            }
            return;
        }
        if self.page != SettingsPage::Display {
            return;
        }
        if let Some((offset_x, offset_y)) = self.drag_offset {
            let mut rect = self.displays[self.selected].rect;
            rect.x = self.cursor.0 - offset_x;
            rect.y = self.cursor.1 - offset_y;
            rect = constrain_center(rect, DISPLAY_PLANE);
            rect = self
                .displays
                .iter()
                .enumerate()
                .filter(|(index, _)| *index != self.selected)
                .fold(rect, |moving, (_, other)| snap_rect(moving, other.rect, 42));
            self.displays[self.selected].rect = rect;
            self.applied = false;
            self.status = self.localizer.text("settings-status-changes-not-applied");
            self.request_redraw();
        }
    }

    fn finish_drag(&mut self) {
        self.desktop_slider_dragging = false;
        self.appearance_slider_dragging = false;
        self.intensity_slider_dragging = false;
        if self.page != SettingsPage::Display {
            return;
        }
        if self.drag_offset.take().is_none() {
            return;
        }
        let selected = self.displays[self.selected].rect;
        let snapped = self
            .displays
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != self.selected)
            .map(|(_, other)| attach_rect_centered(selected, other.rect))
            .min_by_key(|rect| {
                let dx = rect.x - selected.x;
                let dy = rect.y - selected.y;
                dx * dx + dy * dy
            })
            .unwrap_or(selected);
        self.displays[self.selected].rect = snapped;
        self.request_redraw();
    }

    fn set_desktop_count_from_fraction(&mut self, fraction: f32) {
        let count = 1 + (fraction.clamp(0.0, 1.0) * 7.0).round() as u8;
        if count == self.shell_settings.desktop_count {
            return;
        }
        self.shell_settings.desktop_count = count;
        self.shell_settings.active_desktop = self
            .shell_settings
            .active_desktop
            .min(count.saturating_sub(1));
        let _ = self.shell_settings.save_default();
    }

    fn set_appearance_hue_from_fraction(&mut self, fraction: f32) {
        let hue = (fraction.clamp(0.0, 1.0) * 359.0).round() as u16;
        if self.shell_settings.accent_hue == Some(hue) {
            return;
        }
        self.shell_settings.accent_hue = Some(hue);
        let _ = self.shell_settings.save_default();
    }

    fn set_appearance_intensity_from_fraction(&mut self, fraction: f32) {
        let intensity = (fraction.clamp(0.0, 1.0) * 100.0).round() as u8;
        if self.shell_settings.accent_intensity == Some(intensity) {
            return;
        }
        self.shell_settings.accent_intensity = Some(intensity);
        let _ = self.shell_settings.save_default();
    }

    fn load_outputs(&mut self) {
        let Ok(payload) = session_request("list-outputs") else {
            self.status = self.localizer.text("settings-status-using-mock-displays");
            return;
        };
        let outputs: Vec<_> = payload.lines().filter_map(parse_output).collect();
        if outputs.is_empty() {
            self.status = self.localizer.text("settings-status-using-mock-displays");
            return;
        }
        let minimum_x = outputs.iter().map(|output| output.x).min().unwrap_or(0);
        let minimum_y = outputs.iter().map(|output| output.y).min().unwrap_or(0);
        let maximum_x = outputs
            .iter()
            .map(|output| output.x + output.width)
            .max()
            .unwrap_or(1);
        let maximum_y = outputs
            .iter()
            .map(|output| output.y + output.height)
            .max()
            .unwrap_or(1);
        let maximum_physical_width = outputs
            .iter()
            .map(|output| output.physical_width)
            .max()
            .unwrap_or(1)
            .max(1);
        let maximum_physical_height = outputs
            .iter()
            .map(|output| output.physical_height)
            .max()
            .unwrap_or(1)
            .max(1);
        let physical_scale = (280.0 / f64::from(maximum_physical_width))
            .min(160.0 / f64::from(maximum_physical_height));
        // Leave enough empty plane around the arrangement to drag one display
        // completely around another before release snapping chooses an edge.
        self.pixels_per_logical = (470.0 / f64::from((maximum_x - minimum_x).max(1)))
            .min(190.0 / f64::from((maximum_y - minimum_y).max(1)))
            .max(0.04);
        self.displays = outputs
            .into_iter()
            .map(|output| DisplayCard {
                connector: output.name.clone(),
                detail: format!("{}  {} X {}", output.name, output.width, output.height),
                name: output.model,
                logical_width: output.width,
                logical_height: output.height,
                rect: Rect {
                    x: 95
                        + (f64::from(output.x - minimum_x) * self.pixels_per_logical).round()
                            as i32,
                    y: 155
                        + (f64::from(output.y - minimum_y) * self.pixels_per_logical).round()
                            as i32,
                    w: (f64::from(output.physical_width.max(1)) * physical_scale).round() as i32,
                    h: (f64::from(output.physical_height.max(1)) * physical_scale).round() as i32,
                },
                primary: output.primary,
            })
            .collect();
        for index in 1..self.displays.len() {
            let previous = self.displays[index - 1].rect;
            self.displays[index].rect = attach_rect_centered(self.displays[index].rect, previous);
        }
        self.selected = self
            .displays
            .iter()
            .position(|display| display.primary)
            .unwrap_or(0);
        self.status.clear();
    }

    #[cfg(target_os = "windows")]
    fn load_windows_outputs(&mut self, event_loop: &ActiveEventLoop) {
        let monitors: Vec<_> = event_loop.available_monitors().collect();
        if monitors.is_empty() {
            self.status = self.localizer.text("settings-status-no-displays");
            return;
        }
        let primary = event_loop.primary_monitor();
        let minimum_x = monitors
            .iter()
            .map(|monitor| monitor.position().x)
            .min()
            .unwrap_or(0);
        let minimum_y = monitors
            .iter()
            .map(|monitor| monitor.position().y)
            .min()
            .unwrap_or(0);
        let maximum_x = monitors
            .iter()
            .map(|monitor| monitor.position().x + monitor.size().width as i32)
            .max()
            .unwrap_or(1);
        let maximum_y = monitors
            .iter()
            .map(|monitor| monitor.position().y + monitor.size().height as i32)
            .max()
            .unwrap_or(1);
        let desktop_width = (maximum_x - minimum_x).max(1);
        let desktop_height = (maximum_y - minimum_y).max(1);
        self.pixels_per_logical = (380.0 / f64::from(desktop_width))
            .min(210.0 / f64::from(desktop_height))
            .max(0.04);
        let rendered_width = (f64::from(desktop_width) * self.pixels_per_logical).round() as i32;
        let rendered_height = (f64::from(desktop_height) * self.pixels_per_logical).round() as i32;
        let origin_x = DISPLAY_PLANE.x + (DISPLAY_PLANE.w - rendered_width) / 2;
        let origin_y = DISPLAY_PLANE.y + (DISPLAY_PLANE.h - rendered_height) / 2;
        let friendly_names = windows_display_names();
        self.displays = monitors
            .into_iter()
            .enumerate()
            .map(|(index, monitor)| {
                let position = monitor.position();
                let size = monitor.size();
                let raw_name = monitor
                    .name()
                    .unwrap_or_else(|| format!("Display {}", index + 1));
                let connector = raw_name
                    .strip_prefix(r"\\.\")
                    .unwrap_or(&raw_name)
                    .to_owned();
                let name = friendly_names
                    .get(&raw_name)
                    .cloned()
                    .unwrap_or_else(|| connector.clone());
                DisplayCard {
                    connector: connector.clone(),
                    name: name.clone(),
                    detail: format!("{}  {} X {}", connector, size.width, size.height),
                    logical_width: size.width as i32,
                    logical_height: size.height as i32,
                    rect: Rect {
                        x: origin_x
                            + (f64::from(position.x - minimum_x) * self.pixels_per_logical).round()
                                as i32,
                        y: origin_y
                            + (f64::from(position.y - minimum_y) * self.pixels_per_logical).round()
                                as i32,
                        w: (f64::from(size.width) * self.pixels_per_logical).round() as i32,
                        h: (f64::from(size.height) * self.pixels_per_logical).round() as i32,
                    },
                    primary: primary.as_ref() == Some(&monitor),
                }
            })
            .collect();
        self.selected = self
            .displays
            .iter()
            .position(|display| display.primary)
            .unwrap_or(0);
        self.applied = true;
        self.status.clear();
    }

    #[cfg(target_os = "windows")]
    fn load_windows_network(&mut self) {
        use std::mem::size_of;
        use windows::Win32::{
            Foundation::{ERROR_BUFFER_OVERFLOW, NO_ERROR},
            NetworkManagement::{
                IpHelper::{
                    GAA_FLAG_INCLUDE_PREFIX, GetAdaptersAddresses, IP_ADAPTER_ADDRESSES_LH,
                },
                Ndis::IfOperStatusUp,
            },
            Networking::WinSock::AF_UNSPEC,
        };

        let mut byte_count = 0;
        let first = unsafe {
            GetAdaptersAddresses(
                AF_UNSPEC.0 as u32,
                GAA_FLAG_INCLUDE_PREFIX,
                None,
                None,
                &mut byte_count,
            )
        };
        if first != ERROR_BUFFER_OVERFLOW.0 || byte_count == 0 {
            return;
        }
        let words = (byte_count as usize).div_ceil(size_of::<usize>());
        let mut storage = vec![0_usize; words];
        let first_adapter = storage.as_mut_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
        let result = unsafe {
            GetAdaptersAddresses(
                AF_UNSPEC.0 as u32,
                GAA_FLAG_INCLUDE_PREFIX,
                None,
                Some(first_adapter),
                &mut byte_count,
            )
        };
        if result != NO_ERROR.0 {
            return;
        }

        let mut adapters = Vec::new();
        let mut current = first_adapter;
        while let Some(adapter) = unsafe { current.as_ref() } {
            let name = unsafe { adapter.FriendlyName.to_string() }.unwrap_or_default();
            let description = unsafe { adapter.Description.to_string() }.unwrap_or_default();
            if !name.is_empty() && adapter.IfType != 24 {
                adapters.push(NetworkAdapter {
                    name,
                    description,
                    connected: adapter.OperStatus == IfOperStatusUp,
                    speed: adapter.ReceiveLinkSpeed.max(adapter.TransmitLinkSpeed),
                });
            }
            current = adapter.Next;
        }
        adapters.sort_by_key(|adapter| (!adapter.connected, adapter.name.to_ascii_lowercase()));
        self.network_adapters = adapters;
    }

    #[cfg(not(target_os = "windows"))]
    fn load_windows_network(&mut self) {}

    #[cfg(target_os = "windows")]
    fn load_windows_wifi(&mut self) {
        use std::{collections::HashMap, slice};
        use windows::Win32::{
            Foundation::{HANDLE, NO_ERROR},
            NetworkManagement::WiFi::{
                WLAN_AVAILABLE_NETWORK_CONNECTED, WLAN_AVAILABLE_NETWORK_LIST,
                WLAN_INTERFACE_INFO_LIST, WLAN_PROFILE_INFO_LIST, WlanCloseHandle,
                WlanEnumInterfaces, WlanFreeMemory, WlanGetAvailableNetworkList,
                WlanGetProfileList, WlanOpenHandle,
            },
        };

        let mut negotiated = 0;
        let mut handle = HANDLE::default();
        if unsafe { WlanOpenHandle(2, None, &mut negotiated, &mut handle) } != NO_ERROR.0 {
            self.wifi_status = self.localizer.text("settings-network-service-unavailable");
            return;
        }
        let mut interface_list = std::ptr::null_mut::<WLAN_INTERFACE_INFO_LIST>();
        if unsafe { WlanEnumInterfaces(handle, None, &mut interface_list) } != NO_ERROR.0
            || interface_list.is_null()
        {
            unsafe {
                WlanCloseHandle(handle, None);
            }
            self.wifi_status = self
                .localizer
                .text("settings-network-interface-unavailable");
            return;
        }

        let interfaces = unsafe {
            slice::from_raw_parts(
                (*interface_list).InterfaceInfo.as_ptr(),
                (*interface_list).dwNumberOfItems as usize,
            )
        };
        let mut networks_by_profile = HashMap::<String, WifiNetwork>::new();
        for interface in interfaces {
            let interface_id = interface.InterfaceGuid.to_u128();
            let mut available_profiles = HashMap::<String, (u32, bool)>::new();
            let mut available = std::ptr::null_mut::<WLAN_AVAILABLE_NETWORK_LIST>();
            if unsafe {
                WlanGetAvailableNetworkList(
                    handle,
                    &raw const interface.InterfaceGuid,
                    0,
                    None,
                    &mut available,
                )
            } == NO_ERROR.0
                && !available.is_null()
            {
                let entries = unsafe {
                    slice::from_raw_parts(
                        (*available).Network.as_ptr(),
                        (*available).dwNumberOfItems as usize,
                    )
                };
                for network in entries {
                    let profile = wide_text(&network.strProfileName);
                    if !profile.is_empty() {
                        available_profiles.insert(
                            profile.to_ascii_lowercase(),
                            (
                                network.wlanSignalQuality,
                                network.dwFlags & WLAN_AVAILABLE_NETWORK_CONNECTED != 0,
                            ),
                        );
                    }
                }
                unsafe { WlanFreeMemory(available.cast()) };
            }

            let mut profile_list = std::ptr::null_mut::<WLAN_PROFILE_INFO_LIST>();
            if unsafe {
                WlanGetProfileList(
                    handle,
                    &raw const interface.InterfaceGuid,
                    None,
                    &mut profile_list,
                )
            } != NO_ERROR.0
                || profile_list.is_null()
            {
                continue;
            }
            let profiles = unsafe {
                slice::from_raw_parts(
                    (*profile_list).ProfileInfo.as_ptr(),
                    (*profile_list).dwNumberOfItems as usize,
                )
            };
            for saved in profiles {
                let profile = wide_text(&saved.strProfileName);
                if profile.is_empty() {
                    continue;
                }
                let key = profile.to_ascii_lowercase();
                let (signal, connected) =
                    available_profiles.get(&key).copied().unwrap_or((0, false));
                networks_by_profile.entry(key).or_insert(WifiNetwork {
                    profile,
                    signal,
                    connected,
                    interface: interface_id,
                });
            }
            unsafe { WlanFreeMemory(profile_list.cast()) };
        }
        unsafe {
            WlanFreeMemory(interface_list.cast());
            WlanCloseHandle(handle, None);
        }
        let mut networks: Vec<_> = networks_by_profile.into_values().collect();
        networks.sort_by_key(|network| {
            (
                !network.connected,
                network.signal == 0,
                std::cmp::Reverse(network.signal),
                network.profile.to_ascii_lowercase(),
            )
        });
        self.wifi_status = if networks.is_empty() {
            self.localizer.text("settings-network-no-saved-profiles")
        } else {
            self.localizer.number(
                "settings-network-saved-profile-count",
                "count",
                networks.len() as i64,
            )
        };
        self.wifi_networks = networks;
    }

    #[cfg(not(target_os = "windows"))]
    fn load_windows_wifi(&mut self) {}

    #[cfg(target_os = "windows")]
    fn connect_windows_wifi(&mut self, index: usize) {
        use windows::{
            Win32::{
                Foundation::{HANDLE, NO_ERROR},
                NetworkManagement::WiFi::{
                    WLAN_CONNECTION_PARAMETERS, WlanCloseHandle, WlanConnect, WlanOpenHandle,
                    dot11_BSS_type_any, wlan_connection_mode_profile,
                },
            },
            core::{GUID, PCWSTR},
        };

        let Some(network) = self.wifi_networks.get(index) else {
            return;
        };
        let profile = network.profile.clone();
        let interface = GUID::from_u128(network.interface);
        let profile_wide: Vec<u16> = profile.encode_utf16().chain([0]).collect();
        let mut negotiated = 0;
        let mut handle = HANDLE::default();
        if unsafe { WlanOpenHandle(2, None, &mut negotiated, &mut handle) } != NO_ERROR.0 {
            self.wifi_status = self.localizer.text("settings-network-service-unavailable");
            return;
        }
        let parameters = WLAN_CONNECTION_PARAMETERS {
            wlanConnectionMode: wlan_connection_mode_profile,
            strProfile: PCWSTR(profile_wide.as_ptr()),
            dot11BssType: dot11_BSS_type_any,
            ..Default::default()
        };
        let result =
            unsafe { WlanConnect(handle, &raw const interface, &raw const parameters, None) };
        unsafe {
            WlanCloseHandle(handle, None);
        }
        self.wifi_status = if result == NO_ERROR.0 {
            self.pending_wifi_profile = Some(profile.clone());
            self.next_wifi_refresh = Some(Instant::now() + Duration::from_millis(400));
            self.wifi_refreshes_left = 15;
            self.localizer
                .value("settings-network-connecting", "profile", &profile)
        } else {
            self.localizer.value(
                "settings-network-connection-failed",
                "error",
                &result.to_string(),
            )
        };
    }

    #[cfg(not(target_os = "windows"))]
    fn connect_windows_wifi(&mut self, _index: usize) {}

    fn apply_layout(&mut self) {
        let primary = self
            .displays
            .iter()
            .find(|display| display.primary)
            .map(|display| display.connector.as_str())
            .unwrap_or(self.displays[self.selected].connector.as_str());
        let placements = logical_placements(&self.displays);

        #[cfg(target_os = "windows")]
        {
            match apply_windows_layout(&self.displays, &placements, primary) {
                Ok(()) => {
                    self.applied = true;
                    self.status = self.localizer.text("settings-status-layout-applied");
                }
                Err(error) => {
                    self.applied = false;
                    self.status = self.localizer.value(
                        "settings-status-apply-failed",
                        "error",
                        &error.to_string(),
                    );
                }
            }
            return;
        }

        #[cfg(not(target_os = "windows"))]
        let mut command = format!("apply-outputs\nprimary\t{primary}\n");
        #[cfg(not(target_os = "windows"))]
        for (display, (x, y)) in self.displays.iter().zip(placements) {
            command.push_str(&format!("{}\t{x}\t{y}\n", display.connector));
        }
        #[cfg(not(target_os = "windows"))]
        match session_request(&command) {
            Ok(response) if response == "ok" => {
                self.applied = true;
                self.status = self.localizer.text("settings-status-layout-applied");
            }
            Ok(response) => {
                self.applied = false;
                self.status = self.localizer.value(
                    "settings-status-apply-failed",
                    "error",
                    response.strip_prefix("error\t").unwrap_or(&response),
                );
            }
            Err(_) => {
                self.applied = false;
                self.status = self.localizer.text("settings-status-session-unavailable");
            }
        }
    }

    fn render(&mut self) {
        let Some(window) = self.window.clone() else {
            return;
        };
        let appearance = self
            .shell_settings
            .resolve_appearance(nickel_platform::appearance());
        nickel_platform::apply_window_appearance(&window, appearance);
        let palette = ThemePalette::from_appearance(appearance);
        if self.window_icon_color != Some(palette.accent) {
            if let Some(icon) = themed_window_icon(palette.accent) {
                window.set_window_icon(Some(icon));
                #[cfg(target_os = "windows")]
                {
                    use winit::platform::windows::WindowExtWindows;
                    window.set_taskbar_icon(themed_window_icon(palette.accent));
                }
            }
            self.window_icon_color = Some(palette.accent);
        }
        let size = window.inner_size();
        self.ui = self.build_ui(size.width as f32, size.height as f32);
        let mut commands = self.ui.commands().to_vec();
        if self.page == SettingsPage::Display {
            for (index, display) in self.displays.iter().enumerate() {
                let rect = UiRect::new(
                    display.rect.x as f32,
                    display.rect.y as f32,
                    display.rect.w as f32,
                    display.rect.h as f32,
                );
                commands.push(PaintCommand::Fill {
                    rect,
                    color: if index == self.selected {
                        palette.accent_soft
                    } else {
                        palette.surface
                    },
                });
                commands.push(PaintCommand::Stroke {
                    rect,
                    color: if display.primary {
                        palette.accent
                    } else {
                        palette.muted
                    },
                    width: if display.primary { 4.0 } else { 2.0 },
                });
                commands.push(PaintCommand::Text {
                    bounds: UiRect::new(
                        (display.rect.x + 18) as f32,
                        (display.rect.y + 20) as f32,
                        (display.rect.w - 36) as f32,
                        32.0,
                    ),
                    text: display.name.clone(),
                    scale: 3.0,
                    color: palette.text,
                    align: TextAlign::Start,
                    bold: false,
                });
                commands.push(PaintCommand::Text {
                    bounds: UiRect::new(
                        (display.rect.x + 18) as f32,
                        (display.rect.y + 58) as f32,
                        (display.rect.w - 36) as f32,
                        24.0,
                    ),
                    text: display.detail.clone(),
                    scale: 2.0,
                    color: palette.muted,
                    align: TextAlign::Start,
                    bold: false,
                });
                if display.primary {
                    commands.push(PaintCommand::Text {
                        bounds: UiRect::new(
                            (display.rect.x + 18) as f32,
                            (display.rect.y + display.rect.h - 30) as f32,
                            (display.rect.w - 36) as f32,
                            24.0,
                        ),
                        text: "PRIMARY".into(),
                        scale: 2.0,
                        color: palette.accent,
                        align: TextAlign::Start,
                        bold: true,
                    });
                }
            }
        }
        if let Some(gpu) = &mut self.gpu {
            gpu.render(&commands).expect("render settings components");
        }
    }

    #[cfg(any())]
    #[allow(dead_code)]
    fn render_legacy(&mut self) {
        let Some(window) = &self.window else { return };
        let Some(surface) = &mut self.surface else {
            return;
        };
        let size = window.inner_size();
        let Some(width) = NonZeroU32::new(size.width) else {
            return;
        };
        let Some(height) = NonZeroU32::new(size.height) else {
            return;
        };
        surface
            .resize(width, height)
            .expect("resize settings surface");
        let mut buffer = surface.buffer_mut().expect("settings framebuffer");
        let width = size.width as usize;
        let height = size.height as usize;
        buffer.fill(BACKGROUND);

        fill_rect(
            &mut buffer,
            width,
            height,
            Rect {
                x: 0,
                y: 0,
                w: size.width as i32,
                h: 96,
            },
            PANEL,
        );
        fill_rect(
            &mut buffer,
            width,
            height,
            Rect {
                x: 0,
                y: 96,
                w: SIDEBAR_WIDTH,
                h: size.height as i32 - 96,
            },
            PANEL,
        );
        draw_text(
            &mut buffer,
            width,
            height,
            190,
            29,
            4,
            match self.page {
                SettingsPage::Display => "DISPLAY SETTINGS",
                SettingsPage::Network => "NETWORK SETTINGS",
            },
            TEXT,
        );
        draw_text(
            &mut buffer,
            width,
            height,
            192,
            68,
            2,
            match self.page {
                SettingsPage::Display => "DRAG DISPLAYS TO MATCH THEIR PHYSICAL POSITION",
                SettingsPage::Network => "AVAILABLE CONNECTIONS",
            },
            MUTED,
        );

        button(
            &mut buffer,
            width,
            height,
            Self::display_nav(),
            "DISPLAY",
            self.page == SettingsPage::Display,
        );
        button(
            &mut buffer,
            width,
            height,
            Self::network_nav(),
            "NETWORK",
            self.page == SettingsPage::Network,
        );

        if self.page == SettingsPage::Network {
            Self::render_network(
                &self.wifi_networks,
                &self.wifi_status,
                &self.network_adapters,
                &mut buffer,
                width,
                height,
            );
            buffer.present().expect("present settings framebuffer");
            return;
        }

        fill_rect(&mut buffer, width, height, DISPLAY_PLANE, 0x00202429);
        stroke_rect(&mut buffer, width, height, DISPLAY_PLANE, BORDER, 2);

        for (index, display) in self.displays.iter().enumerate() {
            let color = if index == self.selected {
                CARD_SELECTED
            } else {
                CARD
            };
            fill_rect(&mut buffer, width, height, display.rect, color);
            stroke_rect(
                &mut buffer,
                width,
                height,
                display.rect,
                if display.primary { PRIMARY } else { BORDER },
                if display.primary { 4 } else { 2 },
            );
            draw_text(
                &mut buffer,
                width,
                height,
                display.rect.x + 18,
                display.rect.y + 20,
                3,
                &display.name,
                TEXT,
            );
            draw_text(
                &mut buffer,
                width,
                height,
                display.rect.x + 18,
                display.rect.y + 58,
                2,
                &display.detail,
                MUTED,
            );
            if display.primary {
                draw_text(
                    &mut buffer,
                    width,
                    height,
                    display.rect.x + 18,
                    display.rect.y + display.rect.h - 30,
                    2,
                    "PRIMARY",
                    PRIMARY,
                );
            }
        }

        let selected = &self.displays[self.selected];
        draw_text(
            &mut buffer,
            width,
            height,
            192,
            486,
            2,
            &selected.name,
            TEXT,
        );
        button(
            &mut buffer,
            width,
            height,
            Self::identify_button(),
            "IDENTIFY",
            false,
        );
        button(
            &mut buffer,
            width,
            height,
            Self::primary_button(),
            "MAKE PRIMARY",
            false,
        );
        button(
            &mut buffer,
            width,
            height,
            Self::apply_button(),
            "APPLY",
            true,
        );
        draw_text(
            &mut buffer,
            width,
            height,
            192,
            535,
            2,
            &self.status,
            if self.applied { SUCCESS } else { MUTED },
        );
        buffer.present().expect("present settings framebuffer");
    }

    #[cfg(any())]
    fn render_network(
        wifi_networks: &[WifiNetwork],
        wifi_status: &str,
        adapters: &[NetworkAdapter],
        buffer: &mut [u32],
        width: usize,
        height: usize,
    ) {
        draw_text(buffer, width, height, 192, 116, 2, "SAVED WI-FI", TEXT);
        draw_text(buffer, width, height, 500, 118, 1, wifi_status, MUTED);
        for (index, network) in wifi_networks.iter().take(4).enumerate() {
            let rect = Self::wifi_row(index);
            fill_rect(buffer, width, height, rect, CARD);
            stroke_rect(
                buffer,
                width,
                height,
                rect,
                if network.connected { SUCCESS } else { BORDER },
                2,
            );
            draw_text(
                buffer,
                width,
                height,
                rect.x + 14,
                rect.y + 13,
                2,
                &network.profile,
                TEXT,
            );
            let detail = if network.connected {
                format!("CONNECTED  {}%", network.signal)
            } else {
                format!("{}%  CLICK TO CONNECT", network.signal)
            };
            draw_text(
                buffer,
                width,
                height,
                rect.x + 330,
                rect.y + 15,
                1,
                &detail,
                if network.connected { SUCCESS } else { MUTED },
            );
        }

        let adapter_top = 378;
        draw_text(
            buffer,
            width,
            height,
            192,
            adapter_top - 24,
            2,
            "ADAPTERS",
            TEXT,
        );
        if adapters.is_empty() {
            draw_text(
                buffer,
                width,
                height,
                200,
                adapter_top,
                2,
                "NO NETWORK ADAPTERS FOUND",
                MUTED,
            );
            return;
        }
        for (index, adapter) in adapters.iter().take(2).enumerate() {
            let top = adapter_top + index as i32 * 86;
            let rect = Rect {
                x: 190,
                y: top,
                w: 620,
                h: 72,
            };
            fill_rect(buffer, width, height, rect, CARD);
            stroke_rect(
                buffer,
                width,
                height,
                rect,
                if adapter.connected { SUCCESS } else { BORDER },
                2,
            );
            draw_text(buffer, width, height, 206, top + 12, 2, &adapter.name, TEXT);
            let status = if adapter.connected {
                format!("CONNECTED  {} MBPS", adapter.speed / 1_000_000)
            } else {
                "DISCONNECTED".to_owned()
            };
            draw_text(
                buffer,
                width,
                height,
                206,
                top + 38,
                2,
                &status,
                if adapter.connected { SUCCESS } else { MUTED },
            );
            if !adapter.description.eq_ignore_ascii_case(&adapter.name) {
                draw_text(
                    buffer,
                    width,
                    height,
                    470,
                    top + 38,
                    1,
                    &adapter.description,
                    MUTED,
                );
            }
        }
    }
}

impl ApplicationHandler for SettingsApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        #[cfg(target_os = "windows")]
        {
            self.load_windows_outputs(event_loop);
            self.load_windows_network();
            self.load_windows_wifi();
        }
        let attributes = WindowAttributes::default()
            .with_title("Nickel Settings")
            .with_inner_size(LogicalSize::new(850.0, 580.0))
            .with_min_inner_size(LogicalSize::new(850.0, 580.0));
        let window = Arc::new(
            event_loop
                .create_window(attributes)
                .expect("create settings window"),
        );
        let size = window.inner_size();
        let gpu = ComponentGpu::new(window.clone(), size.width, size.height)
            .expect("create settings component renderer");
        self.window = Some(window);
        self.gpu = Some(gpu);
        self.request_redraw();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::CursorMoved { position, .. } => self.pointer_moved(position),
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => self.pointer_pressed(),
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: MouseButton::Left,
                ..
            } => self.finish_drag(),
            WindowEvent::RedrawRequested => self.render(),
            WindowEvent::Resized(size) => {
                if let Some(gpu) = &mut self.gpu {
                    gpu.resize(size.width, size.height);
                }
                self.request_redraw();
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let Some(refresh_at) = self.next_wifi_refresh else {
            event_loop.set_control_flow(ControlFlow::Wait);
            return;
        };
        if Instant::now() < refresh_at {
            event_loop.set_control_flow(ControlFlow::WaitUntil(refresh_at));
            return;
        }

        self.load_windows_wifi();
        let connected = self.pending_wifi_profile.as_ref().is_some_and(|profile| {
            self.wifi_networks
                .iter()
                .any(|network| network.connected && network.profile.eq_ignore_ascii_case(profile))
        });
        if connected {
            let profile = self.pending_wifi_profile.take().unwrap_or_default();
            self.wifi_status =
                self.localizer
                    .value("settings-network-connected-to", "profile", &profile);
            self.next_wifi_refresh = None;
            self.wifi_refreshes_left = 0;
        } else if self.wifi_refreshes_left > 1 {
            self.wifi_refreshes_left -= 1;
            self.next_wifi_refresh = Some(Instant::now() + Duration::from_millis(400));
            if let Some(profile) = &self.pending_wifi_profile {
                self.wifi_status =
                    self.localizer
                        .value("settings-network-connecting", "profile", profile);
            }
        } else {
            let profile = self.pending_wifi_profile.take().unwrap_or_default();
            self.wifi_status =
                self.localizer
                    .value("settings-network-connection-timeout", "profile", &profile);
            self.next_wifi_refresh = None;
            self.wifi_refreshes_left = 0;
        }
        self.request_redraw();
    }
}

fn snap_rect(mut moving: Rect, fixed: Rect, threshold: i32) -> Rect {
    let horizontal_candidates = [fixed.x - moving.w, fixed.x + fixed.w];
    if let Some(x) = horizontal_candidates
        .into_iter()
        .min_by_key(|candidate| (moving.x - candidate).abs())
        .filter(|candidate| (moving.x - candidate).abs() <= threshold)
    {
        moving.x = x;
        let vertical_candidates = [fixed.y, fixed.y + fixed.h - moving.h];
        if let Some(y) = vertical_candidates
            .into_iter()
            .min_by_key(|candidate| (moving.y - candidate).abs())
            .filter(|candidate| (moving.y - candidate).abs() <= threshold)
        {
            moving.y = y;
        }
        return moving;
    }

    let vertical_candidates = [fixed.y - moving.h, fixed.y + fixed.h];
    if let Some(y) = vertical_candidates
        .into_iter()
        .min_by_key(|candidate| (moving.y - candidate).abs())
        .filter(|candidate| (moving.y - candidate).abs() <= threshold)
    {
        moving.y = y;
        let horizontal_alignment = [fixed.x, fixed.x + fixed.w - moving.w];
        if let Some(x) = horizontal_alignment
            .into_iter()
            .min_by_key(|candidate| (moving.x - candidate).abs())
            .filter(|candidate| (moving.x - candidate).abs() <= threshold)
        {
            moving.x = x;
        }
    }
    moving
}

fn attach_rect_centered(moving: Rect, fixed: Rect) -> Rect {
    let candidates = [
        Rect {
            x: fixed.x - moving.w,
            y: fixed.y + (fixed.h - moving.h) / 2,
            ..moving
        },
        Rect {
            x: fixed.x + fixed.w,
            y: fixed.y + (fixed.h - moving.h) / 2,
            ..moving
        },
        Rect {
            x: fixed.x + (fixed.w - moving.w) / 2,
            y: fixed.y - moving.h,
            ..moving
        },
        Rect {
            x: fixed.x + (fixed.w - moving.w) / 2,
            y: fixed.y + fixed.h,
            ..moving
        },
    ];
    candidates
        .into_iter()
        .min_by_key(|candidate| {
            let dx = candidate.x - moving.x;
            let dy = candidate.y - moving.y;
            dx * dx + dy * dy
        })
        .unwrap_or(moving)
}

fn logical_placements(displays: &[DisplayCard]) -> Vec<(i32, i32)> {
    if displays.is_empty() {
        return Vec::new();
    }
    let mut placements = vec![(0, 0); displays.len()];
    for index in 1..displays.len() {
        let moving = &displays[index];
        let fixed = &displays[index - 1];
        let (fixed_x, fixed_y) = placements[index - 1];
        let edge_distances = [
            (moving.rect.x + moving.rect.w - fixed.rect.x).abs(),
            (moving.rect.x - fixed.rect.x - fixed.rect.w).abs(),
            (moving.rect.y + moving.rect.h - fixed.rect.y).abs(),
            (moving.rect.y - fixed.rect.y - fixed.rect.h).abs(),
        ];
        let edge = edge_distances
            .iter()
            .enumerate()
            .min_by_key(|(_, distance)| *distance)
            .map(|(edge, _)| edge)
            .unwrap_or(1);
        placements[index] = match edge {
            0 => (
                fixed_x - moving.logical_width,
                fixed_y + (fixed.logical_height - moving.logical_height) / 2,
            ),
            1 => (
                fixed_x + fixed.logical_width,
                fixed_y + (fixed.logical_height - moving.logical_height) / 2,
            ),
            2 => (
                fixed_x + (fixed.logical_width - moving.logical_width) / 2,
                fixed_y - moving.logical_height,
            ),
            _ => (
                fixed_x + (fixed.logical_width - moving.logical_width) / 2,
                fixed_y + fixed.logical_height,
            ),
        };
    }
    let minimum_x = placements.iter().map(|(x, _)| *x).min().unwrap_or(0);
    let minimum_y = placements.iter().map(|(_, y)| *y).min().unwrap_or(0);
    for (x, y) in &mut placements {
        *x -= minimum_x;
        *y -= minimum_y;
    }
    placements
}

#[cfg(target_os = "windows")]
fn apply_windows_layout(
    displays: &[DisplayCard],
    placements: &[(i32, i32)],
    primary: &str,
) -> Result<(), i32> {
    use std::mem::size_of;
    use windows::{
        Win32::Graphics::Gdi::{
            CDS_NORESET, CDS_SET_PRIMARY, CDS_TYPE, CDS_UPDATEREGISTRY, ChangeDisplaySettingsExW,
            DEVMODEW, DISP_CHANGE_SUCCESSFUL, DM_POSITION, ENUM_CURRENT_SETTINGS,
            EnumDisplaySettingsW,
        },
        core::PCWSTR,
    };

    let (primary_x, primary_y) = displays
        .iter()
        .zip(placements)
        .find(|(display, _)| display.connector == primary)
        .map(|(_, placement)| *placement)
        .unwrap_or((0, 0));

    for (display, &(x, y)) in displays.iter().zip(placements) {
        let device_name = format!(r"\\.\{}", display.connector);
        let device_wide: Vec<u16> = device_name.encode_utf16().chain([0]).collect();
        let device = PCWSTR(device_wide.as_ptr());
        let mut mode = DEVMODEW {
            dmSize: size_of::<DEVMODEW>() as u16,
            ..Default::default()
        };
        // SAFETY: `device` and `mode` remain valid for the duration of each Win32 call.
        if !unsafe { EnumDisplaySettingsW(device, ENUM_CURRENT_SETTINGS, &raw mut mode) }.as_bool()
        {
            return Err(-1);
        }
        mode.dmFields |= DM_POSITION;
        mode.Anonymous1.Anonymous2.dmPosition.x = x - primary_x;
        mode.Anonymous1.Anonymous2.dmPosition.y = y - primary_y;
        let mut flags = CDS_UPDATEREGISTRY | CDS_NORESET;
        if display.connector == primary {
            flags |= CDS_SET_PRIMARY;
        }
        // SAFETY: The device name and initialized DEVMODEW are valid for this synchronous call.
        let result =
            unsafe { ChangeDisplaySettingsExW(device, Some(&raw const mode), None, flags, None) };
        if result != DISP_CHANGE_SUCCESSFUL {
            return Err(result.0);
        }
    }

    // Commit all staged changes together to avoid transient intermediate layouts.
    // SAFETY: A null device and mode apply the previously staged changes.
    let result =
        unsafe { ChangeDisplaySettingsExW(PCWSTR::null(), None, None, CDS_TYPE::default(), None) };
    if result == DISP_CHANGE_SUCCESSFUL {
        Ok(())
    } else {
        Err(result.0)
    }
}

fn constrain_center(mut monitor: Rect, plane: Rect) -> Rect {
    let half_width = monitor.w / 2;
    let half_height = monitor.h / 2;
    monitor.x = monitor
        .x
        .clamp(plane.x - half_width, plane.x + plane.w - half_width);
    monitor.y = monitor
        .y
        .clamp(plane.y - half_height, plane.y + plane.h - half_height);
    monitor
}

#[cfg(any())]
fn button(buffer: &mut [u32], width: usize, height: usize, rect: Rect, label: &str, accent: bool) {
    fill_rect(
        buffer,
        width,
        height,
        rect,
        if accent { PRIMARY } else { CARD },
    );
    stroke_rect(
        buffer,
        width,
        height,
        rect,
        if accent { PRIMARY } else { BORDER },
        2,
    );
    let text_width = label.chars().count() as i32 * 12;
    draw_text(
        buffer,
        width,
        height,
        rect.x + (rect.w - text_width) / 2,
        rect.y + 14,
        2,
        label,
        TEXT,
    );
}

#[cfg(any())]
fn fill_rect(buffer: &mut [u32], width: usize, height: usize, rect: Rect, color: u32) {
    let left = rect.x.max(0) as usize;
    let top = rect.y.max(0) as usize;
    let right = (rect.x + rect.w).clamp(0, width as i32) as usize;
    let bottom = (rect.y + rect.h).clamp(0, height as i32) as usize;
    for y in top..bottom {
        buffer[y * width + left..y * width + right].fill(color);
    }
}

#[cfg(any())]
fn stroke_rect(
    buffer: &mut [u32],
    width: usize,
    height: usize,
    rect: Rect,
    color: u32,
    thickness: i32,
) {
    fill_rect(
        buffer,
        width,
        height,
        Rect {
            x: rect.x,
            y: rect.y,
            w: rect.w,
            h: thickness,
        },
        color,
    );
    fill_rect(
        buffer,
        width,
        height,
        Rect {
            x: rect.x,
            y: rect.y + rect.h - thickness,
            w: rect.w,
            h: thickness,
        },
        color,
    );
    fill_rect(
        buffer,
        width,
        height,
        Rect {
            x: rect.x,
            y: rect.y,
            w: thickness,
            h: rect.h,
        },
        color,
    );
    fill_rect(
        buffer,
        width,
        height,
        Rect {
            x: rect.x + rect.w - thickness,
            y: rect.y,
            w: thickness,
            h: rect.h,
        },
        color,
    );
}

#[allow(clippy::too_many_arguments)]
#[cfg(any())]
fn draw_text(
    buffer: &mut [u32],
    width: usize,
    height: usize,
    x: i32,
    y: i32,
    scale: i32,
    text: &str,
    color: u32,
) {
    let mut cursor = x;
    for character in text.chars() {
        let rows = glyph(character.to_ascii_uppercase());
        for (row, bits) in rows.iter().enumerate() {
            for column in 0..5 {
                if bits & (1 << (4 - column)) != 0 {
                    fill_rect(
                        buffer,
                        width,
                        height,
                        Rect {
                            x: cursor + column * scale,
                            y: y + row as i32 * scale,
                            w: scale,
                            h: scale,
                        },
                        color,
                    );
                }
            }
        }
        cursor += 6 * scale;
    }
}

#[cfg(any())]
fn glyph(character: char) -> [u8; 7] {
    match character {
        'A' => [14, 17, 17, 31, 17, 17, 17],
        'B' => [30, 17, 17, 30, 17, 17, 30],
        'C' => [14, 17, 16, 16, 16, 17, 14],
        'D' => [30, 17, 17, 17, 17, 17, 30],
        'E' => [31, 16, 16, 30, 16, 16, 31],
        'F' => [31, 16, 16, 30, 16, 16, 16],
        'G' => [14, 17, 16, 23, 17, 17, 15],
        'H' => [17, 17, 17, 31, 17, 17, 17],
        'I' => [14, 4, 4, 4, 4, 4, 14],
        'J' => [7, 2, 2, 2, 18, 18, 12],
        'K' => [17, 18, 20, 24, 20, 18, 17],
        'L' => [16, 16, 16, 16, 16, 16, 31],
        'M' => [17, 27, 21, 21, 17, 17, 17],
        'N' => [17, 25, 21, 19, 17, 17, 17],
        'O' => [14, 17, 17, 17, 17, 17, 14],
        'P' => [30, 17, 17, 30, 16, 16, 16],
        'Q' => [14, 17, 17, 17, 21, 18, 13],
        'R' => [30, 17, 17, 30, 20, 18, 17],
        'S' => [15, 16, 16, 14, 1, 1, 30],
        'T' => [31, 4, 4, 4, 4, 4, 4],
        'U' => [17, 17, 17, 17, 17, 17, 14],
        'V' => [17, 17, 17, 17, 17, 10, 4],
        'W' => [17, 17, 17, 21, 21, 21, 10],
        'X' => [17, 17, 10, 4, 10, 17, 17],
        'Y' => [17, 17, 10, 4, 4, 4, 4],
        'Z' => [31, 1, 2, 4, 8, 16, 31],
        '0' => [14, 17, 19, 21, 25, 17, 14],
        '1' => [4, 12, 4, 4, 4, 4, 14],
        '2' => [14, 17, 1, 2, 4, 8, 31],
        '3' => [30, 1, 1, 14, 1, 1, 30],
        '4' => [2, 6, 10, 18, 31, 2, 2],
        '5' => [31, 16, 16, 30, 1, 1, 30],
        '6' => [14, 16, 16, 30, 17, 17, 14],
        '7' => [31, 1, 2, 4, 8, 8, 8],
        '8' => [14, 17, 17, 14, 17, 17, 14],
        '9' => [14, 17, 17, 15, 1, 1, 14],
        '-' => [0, 0, 0, 31, 0, 0, 0],
        '.' => [0, 0, 0, 0, 0, 12, 12],
        ' ' => [0; 7],
        _ => [31, 17, 2, 4, 4, 0, 4],
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _log_path = nickel_logging::init("nickel-settings").ok();
    let event_loop = EventLoop::new()?;
    let mut app = SettingsApp::default();
    app.load_outputs();
    event_loop.run_app(&mut app)?;
    Ok(())
}

fn themed_window_icon(color: u32) -> Option<Icon> {
    let mut image =
        image::load_from_memory(include_bytes!("../../../assets/icons/nickel-panel.png"))
            .ok()?
            .into_rgba8();
    let rgb = [
        ((color >> 16) & 0xff) as u8,
        ((color >> 8) & 0xff) as u8,
        (color & 0xff) as u8,
    ];
    for pixel in image.pixels_mut() {
        if pixel.0[3] != 0 {
            pixel.0[..3].copy_from_slice(&rgb);
        }
    }
    let (width, height) = image.dimensions();
    Icon::from_rgba(image.into_raw(), width, height).ok()
}

#[cfg(test)]
mod tests {
    use super::{Rect, attach_rect_centered, constrain_center, snap_rect};

    #[test]
    fn display_center_remains_inside_plane_while_edges_may_leave() {
        let plane = Rect {
            x: 40,
            y: 120,
            w: 770,
            h: 340,
        };
        let monitor = Rect {
            x: -500,
            y: -500,
            w: 300,
            h: 180,
        };

        let constrained = constrain_center(monitor, plane);
        assert_eq!(constrained.x, plane.x - monitor.w / 2);
        assert_eq!(constrained.y, plane.y - monitor.h / 2);
    }

    #[test]
    fn nearby_monitor_edges_snap_together_and_align() {
        let fixed = Rect {
            x: 400,
            y: 180,
            w: 300,
            h: 180,
        };
        let moving = Rect {
            x: 105,
            y: 190,
            w: 270,
            h: 160,
        };

        let snapped = snap_rect(moving, fixed, 42);
        assert_eq!(snapped.x + snapped.w, fixed.x);
        assert_eq!(snapped.y, fixed.y);
    }

    #[test]
    fn released_monitor_touches_and_centers_on_nearest_edge() {
        let fixed = Rect {
            x: 400,
            y: 180,
            w: 220,
            h: 125,
        };
        let moving = Rect {
            x: 150,
            y: 205,
            w: 220,
            h: 125,
        };

        let attached = attach_rect_centered(moving, fixed);

        assert_eq!(attached.x + attached.w, fixed.x);
        assert_eq!(attached.y + attached.h / 2, fixed.y + fixed.h / 2);
    }

    #[test]
    fn distant_monitors_keep_freeform_position() {
        let fixed = Rect {
            x: 400,
            y: 180,
            w: 300,
            h: 180,
        };
        let moving = Rect {
            x: 40,
            y: 430,
            w: 200,
            h: 120,
        };

        let snapped = snap_rect(moving, fixed, 42);
        assert_eq!((snapped.x, snapped.y), (moving.x, moving.y));
    }
}
