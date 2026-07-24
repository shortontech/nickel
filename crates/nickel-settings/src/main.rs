use std::{num::NonZeroU32, sync::Arc};

#[cfg(target_os = "windows")]
use std::collections::HashMap;

use softbuffer::{Context, Surface};
use winit::{
    application::ApplicationHandler,
    dpi::{LogicalSize, PhysicalPosition},
    event::{ElementState, MouseButton, WindowEvent},
    event_loop::{ActiveEventLoop, EventLoop},
    window::{Window, WindowAttributes, WindowId},
};

const BACKGROUND: u32 = 0x001b1d21;
const PANEL: u32 = 0x0025282e;
const CARD: u32 = 0x00323842;
const CARD_SELECTED: u32 = 0x003c5878;
const PRIMARY: u32 = 0x002d8fdd;
const BORDER: u32 = 0x006d7685;
const TEXT: u32 = 0x00f2f4f8;
const MUTED: u32 = 0x00aab1bd;
const SUCCESS: u32 = 0x0047b881;
const DISPLAY_PLANE: Rect = Rect {
    x: 40,
    y: 120,
    w: 770,
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

struct SettingsApp {
    window: Option<Arc<Window>>,
    context: Option<Context<Arc<Window>>>,
    surface: Option<Surface<Arc<Window>, Arc<Window>>>,
    displays: Vec<DisplayCard>,
    selected: usize,
    cursor: (i32, i32),
    drag_offset: Option<(i32, i32)>,
    applied: bool,
    pixels_per_logical: f64,
    status: String,
}

impl Default for SettingsApp {
    fn default() -> Self {
        Self {
            window: None,
            context: None,
            surface: None,
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
            status: "CHANGES NOT APPLIED".into(),
        }
    }
}

impl SettingsApp {
    fn primary_button() -> Rect {
        Rect {
            x: 540,
            y: 510,
            w: 145,
            h: 42,
        }
    }

    fn identify_button() -> Rect {
        Rect {
            x: 390,
            y: 510,
            w: 135,
            h: 42,
        }
    }

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

    fn pointer_pressed(&mut self) {
        let (x, y) = self.cursor;
        if Self::identify_button().contains(x, y) {
            match session_request("identify-outputs") {
                Ok(response) if response == "ok" => self.status = "IDENTIFYING DISPLAYS".into(),
                _ => self.status = "IDENTIFY FAILED".into(),
            }
        } else if Self::primary_button().contains(x, y) {
            for (index, display) in self.displays.iter_mut().enumerate() {
                display.primary = index == self.selected;
            }
            self.applied = false;
            self.status = "CHANGES NOT APPLIED".into();
        } else if Self::apply_button().contains(x, y) {
            self.apply_layout();
        } else if let Some(index) = self
            .displays
            .iter()
            .rposition(|display| display.rect.contains(x, y))
        {
            self.selected = index;
            let rect = self.displays[index].rect;
            self.drag_offset = Some((x - rect.x, y - rect.y));
            self.applied = false;
            self.status = "CHANGES NOT APPLIED".into();
        }
        self.request_redraw();
    }

    fn pointer_moved(&mut self, position: PhysicalPosition<f64>) {
        self.cursor = (position.x.round() as i32, position.y.round() as i32);
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
            self.status = "CHANGES NOT APPLIED".into();
            self.request_redraw();
        }
    }

    fn finish_drag(&mut self) {
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

    fn load_outputs(&mut self) {
        let Ok(payload) = session_request("list-outputs") else {
            self.status = "USING MOCK OUTPUTS".into();
            return;
        };
        let outputs: Vec<_> = payload.lines().filter_map(parse_output).collect();
        if outputs.is_empty() {
            self.status = "USING MOCK OUTPUTS".into();
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
        self.status = "LIVE OUTPUTS LOADED".into();
    }

    #[cfg(target_os = "windows")]
    fn load_windows_outputs(&mut self, event_loop: &ActiveEventLoop) {
        let monitors: Vec<_> = event_loop.available_monitors().collect();
        if monitors.is_empty() {
            self.status = "NO WINDOWS DISPLAYS FOUND".into();
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
        self.status = "WINDOWS DISPLAYS LOADED".into();
    }

    fn apply_layout(&mut self) {
        let primary = self
            .displays
            .iter()
            .find(|display| display.primary)
            .map(|display| display.connector.as_str())
            .unwrap_or(self.displays[self.selected].connector.as_str());
        let mut command = format!("apply-outputs\nprimary\t{primary}\n");
        let placements = logical_placements(&self.displays);
        for (display, (x, y)) in self.displays.iter().zip(placements) {
            command.push_str(&format!("{}\t{x}\t{y}\n", display.connector));
        }
        match session_request(&command) {
            Ok(response) if response == "ok" => {
                self.applied = true;
                self.status = "LIVE LAYOUT APPLIED".into();
            }
            Ok(response) => {
                self.applied = false;
                self.status = response
                    .strip_prefix("error\t")
                    .unwrap_or("APPLY FAILED")
                    .to_ascii_uppercase();
            }
            Err(_) => {
                self.applied = false;
                self.status = "SESSION NOT AVAILABLE".into();
            }
        }
    }

    fn render(&mut self) {
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
        draw_text(
            &mut buffer,
            width,
            height,
            38,
            29,
            4,
            "DISPLAY SETTINGS",
            TEXT,
        );
        draw_text(
            &mut buffer,
            width,
            height,
            40,
            68,
            2,
            "DRAG DISPLAYS TO MATCH THEIR PHYSICAL POSITION",
            MUTED,
        );

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
        draw_text(&mut buffer, width, height, 42, 486, 2, &selected.name, TEXT);
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
            42,
            535,
            2,
            &self.status,
            if self.applied { SUCCESS } else { MUTED },
        );
        buffer.present().expect("present settings framebuffer");
    }
}

impl ApplicationHandler for SettingsApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        #[cfg(target_os = "windows")]
        self.load_windows_outputs(event_loop);
        let attributes = WindowAttributes::default()
            .with_title("Nickel Settings")
            .with_inner_size(LogicalSize::new(850.0, 580.0))
            .with_min_inner_size(LogicalSize::new(850.0, 580.0));
        let window = Arc::new(
            event_loop
                .create_window(attributes)
                .expect("create settings window"),
        );
        let context = Context::new(window.clone()).expect("create settings context");
        let surface = Surface::new(&context, window.clone()).expect("create settings surface");
        self.window = Some(window);
        self.context = Some(context);
        self.surface = Some(surface);
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
            WindowEvent::Resized(_) => self.request_redraw(),
            _ => {}
        }
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

fn fill_rect(buffer: &mut [u32], width: usize, height: usize, rect: Rect, color: u32) {
    let left = rect.x.max(0) as usize;
    let top = rect.y.max(0) as usize;
    let right = (rect.x + rect.w).clamp(0, width as i32) as usize;
    let bottom = (rect.y + rect.h).clamp(0, height as i32) as usize;
    for y in top..bottom {
        buffer[y * width + left..y * width + right].fill(color);
    }
}

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
    let event_loop = EventLoop::new()?;
    let mut app = SettingsApp::default();
    app.load_outputs();
    event_loop.run_app(&mut app)?;
    Ok(())
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
