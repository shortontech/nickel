#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use nickel_components::{
    Button, Column, ComponentGpu, Container, Grid, Image, Insets, LinearGradient, Point, Rect, Row,
    Text, TextAlign, UiTree,
};
use nickel_core::shell_settings::ShellSettings;
use nickel_file::{DirectoryBrowser, FileEntry};
use winit::{
    application::ApplicationHandler,
    dpi::LogicalSize,
    event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, EventLoop},
    keyboard::{Key, NamedKey},
    window::{Window, WindowAttributes, WindowId},
};

const BACKGROUND: u32 = 0x16191f;
const HEADER_TOP: u32 = 0x303846;
const HEADER_BOTTOM: u32 = 0x232933;
const TOOLBAR: u32 = 0x1d2129;
const SIDEBAR: u32 = 0x20252d;
const TILE: u32 = 0x1b1f26;
const TILE_SELECTED: u32 = 0x303d50;
const ROW_HOVER: u32 = 0x30394a;
const BORDER: u32 = 0x3b4350;
const PRIMARY: u32 = 0x4b8bd8;
const TEXT: u32 = 0xf2f4f8;
const MUTED: u32 = 0x9ca5b3;
const SIDEBAR_WIDTH: f32 = 190.0;
const TOOLBAR_HEIGHT: f32 = 58.0;
const FOOTER_HEIGHT: f32 = 30.0;
const TILE_HEIGHT: f32 = 122.0;
const TILE_MIN_WIDTH: f32 = 126.0;

struct FileApp {
    window: Option<Arc<Window>>,
    gpu: Option<ComponentGpu>,
    browser: DirectoryBrowser,
    ui: UiTree,
    cursor: Point,
    selected: Option<usize>,
    scroll_offset: usize,
    status: String,
    last_click: Option<(usize, Instant)>,
    icons: HashMap<PathBuf, (u16, Arc<image::RgbaImage>)>,
    next_icon_id: u16,
}

impl FileApp {
    fn new(path: PathBuf) -> Self {
        let show_hidden = nickel_platform::show_hidden_files();
        let (browser, status) = match DirectoryBrowser::open_with_hidden(&path, show_hidden) {
            Ok(browser) => (browser, String::new()),
            Err(error) => {
                let home = home_directory();
                let browser = DirectoryBrowser::open_with_hidden(&home, show_hidden)
                    .unwrap_or_else(|_| {
                        DirectoryBrowser::open_with_hidden(".", show_hidden)
                            .expect("open a directory")
                    });
                (
                    browser,
                    format!("Could not open {}: {error}", path.display()),
                )
            }
        };
        let mut app = Self {
            window: None,
            gpu: None,
            browser,
            ui: UiTree::default(),
            cursor: Point { x: 0.0, y: 0.0 },
            selected: None,
            scroll_offset: 0,
            status,
            last_click: None,
            icons: HashMap::new(),
            next_icon_id: 1,
        };
        app.refresh_icons();
        app
    }

    fn request_redraw(&self) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn grid_columns_for_width(width: f32) -> usize {
        ((width - SIDEBAR_WIDTH - 32.0) / TILE_MIN_WIDTH)
            .floor()
            .max(1.0) as usize
    }

    fn grid_rows_for_height(height: f32) -> usize {
        ((height - TOOLBAR_HEIGHT - FOOTER_HEIGHT - 30.0) / TILE_HEIGHT)
            .floor()
            .max(1.0) as usize
    }

    fn grid_dimensions(&self) -> (usize, usize) {
        self.window
            .as_ref()
            .map(|window| {
                let size = window.inner_size();
                (
                    Self::grid_columns_for_width(size.width as f32),
                    Self::grid_rows_for_height(size.height as f32),
                )
            })
            .unwrap_or((5, 4))
    }

    fn visible_capacity(&self) -> usize {
        let (columns, rows) = self.grid_dimensions();
        columns * rows
    }

    fn build_ui(&self, width: f32, height: f32) -> UiTree {
        let location = self.browser.current().display().to_string();
        let toolbar = Container::new()
            .height(TOOLBAR_HEIGHT)
            .background(LinearGradient::vertical(HEADER_TOP, HEADER_BOTTOM))
            .padding(Insets {
                top: 9.0,
                right: 14.0,
                bottom: 8.0,
                left: 12.0,
            })
            .child(
                Row::new()
                    .gap(8.0)
                    .child(Button::new("nav:back", "<").width(42.0).background(
                        if self.browser.can_go_back() {
                            PRIMARY
                        } else {
                            BORDER
                        },
                    ))
                    .child(Button::new("nav:up", "^").width(42.0).background(
                        if self.browser.can_go_up() {
                            ROW_HOVER
                        } else {
                            BORDER
                        },
                    ))
                    .child(
                        Container::new()
                            .grow(1.0)
                            .background(TOOLBAR)
                            .border(BORDER, 1.0)
                            .padding(Insets {
                                top: 10.0,
                                right: 14.0,
                                bottom: 6.0,
                                left: 14.0,
                            })
                            .child(Text::new(location).scale(1.2).color(TEXT)),
                    )
                    .child(
                        Button::new("nav:refresh", "REFRESH")
                            .width(98.0)
                            .background(ROW_HOVER),
                    ),
            );
        let places = places();
        let sidebar = Container::new()
            .width(SIDEBAR_WIDTH)
            .background(SIDEBAR)
            .border(BORDER, 1.0)
            .padding(Insets {
                top: 18.0,
                right: 10.0,
                bottom: 12.0,
                left: 10.0,
            })
            .child(
                Column::new()
                    .gap(5.0)
                    .child(Text::new("NICKEL FILE").height(42.0).scale(2.0).color(TEXT))
                    .child(Text::new("PLACES").height(28.0).scale(1.0).color(MUTED))
                    .children(places.iter().enumerate().map(|(index, (label, path))| {
                        let active = path == self.browser.current();
                        Container::new()
                            .height(40.0)
                            .background(if active { TILE_SELECTED } else { SIDEBAR })
                            .padding(Insets {
                                top: 9.0,
                                right: 8.0,
                                bottom: 6.0,
                                left: 12.0,
                            })
                            .action(format!("place:{index}"))
                            .child(Text::new(label).scale(1.2).color(if active {
                                TEXT
                            } else {
                                MUTED
                            }))
                    })),
            );
        let capacity = self.visible_capacity();
        let (columns, _) = self.grid_dimensions();
        let visible_count = self
            .browser
            .entries()
            .len()
            .saturating_sub(self.scroll_offset)
            .min(capacity);
        let grid_rows = visible_count.div_ceil(columns);
        let grid_width = (width - SIDEBAR_WIDTH - 32.0).max(TILE_MIN_WIDTH);
        let cell_width = (grid_width - 10.0 * columns.saturating_sub(1) as f32) / columns as f32;
        let grid_height = grid_rows as f32 * cell_width + 10.0 * grid_rows.saturating_sub(1) as f32;
        let tiles = self
            .browser
            .entries()
            .iter()
            .enumerate()
            .skip(self.scroll_offset)
            .take(capacity)
            .map(|(index, entry)| {
                file_tile(
                    index,
                    entry,
                    self.selected == Some(index),
                    self.icons.get(&entry.path).cloned(),
                )
            });
        let files = if self.browser.entries().is_empty() {
            Column::new()
                .grow(1.0)
                .padding(Insets::all(28.0))
                .child(Text::new("This folder is empty.").color(MUTED))
        } else {
            Column::new()
                .grow(1.0)
                .padding(Insets {
                    top: 14.0,
                    right: 16.0,
                    bottom: 14.0,
                    left: 16.0,
                })
                .child(
                    Grid::columns(columns)
                        .gap(10.0)
                        .height(grid_height)
                        .children(tiles),
                )
        };
        let footer_text = if self.status.is_empty() {
            format!(
                "{} item{}",
                self.browser.entries().len(),
                if self.browser.entries().len() == 1 {
                    ""
                } else {
                    "s"
                }
            )
        } else {
            self.status.clone()
        };
        let footer = Container::new()
            .height(30.0)
            .background(TOOLBAR)
            .padding(Insets {
                top: 7.0,
                right: 14.0,
                bottom: 5.0,
                left: 14.0,
            })
            .child(Text::new(footer_text).scale(1.0).color(MUTED));
        let content = Row::new().grow(1.0).child(sidebar).child(files);
        let root = Column::new()
            .height(height)
            .background(BACKGROUND)
            .child(toolbar)
            .child(content)
            .child(footer);
        UiTree::layout(root, Rect::new(0.0, 0.0, width, height))
    }

    fn render(&mut self) {
        let Some(window) = &self.window else {
            return;
        };
        let appearance =
            ShellSettings::load_default().resolve_appearance(nickel_platform::appearance());
        nickel_platform::apply_window_appearance(window, appearance);
        let size = window.inner_size();
        self.ui = self.build_ui(size.width as f32, size.height as f32);
        if let Some(gpu) = &mut self.gpu
            && let Err(error) = gpu.render(self.ui.commands())
        {
            tracing::warn!(%error, "failed to render Nickel File");
        }
    }

    fn activate_selected(&mut self) {
        let Some(index) = self.selected else {
            return;
        };
        let Some(entry) = self.browser.entries().get(index).cloned() else {
            return;
        };
        let is_directory = entry.is_directory || entry.path.is_dir();
        if is_directory && !entry.is_directory {
            tracing::warn!(
                path = %entry.path.display(),
                "entry model missed a directory; using activation-time check"
            );
        }
        if is_directory {
            self.navigate_to(entry.path);
        } else if let Err(error) = open_path(&entry.path) {
            self.status = format!("Could not open {}: {error}", entry.display_name());
            self.request_redraw();
        }
    }

    fn navigate_to(&mut self, path: PathBuf) {
        if let Err(error) = self
            .browser
            .set_show_hidden(nickel_platform::show_hidden_files())
        {
            self.status = format!("Could not update hidden-file visibility: {error}");
        }
        match self.browser.enter(path) {
            Ok(()) => self.navigation_changed(),
            Err(error) => {
                self.status = format!("Could not open folder: {error}");
                self.request_redraw();
            }
        }
    }

    fn go_back(&mut self) {
        match self.browser.back() {
            Ok(true) => self.navigation_changed(),
            Ok(false) => {}
            Err(error) => self.status = format!("Could not go back: {error}"),
        }
        self.request_redraw();
    }

    fn go_up(&mut self) {
        match self.browser.up() {
            Ok(true) => self.navigation_changed(),
            Ok(false) => {}
            Err(error) => self.status = format!("Could not open parent: {error}"),
        }
        self.request_redraw();
    }

    fn navigation_changed(&mut self) {
        self.selected = None;
        self.scroll_offset = 0;
        self.status.clear();
        self.refresh_icons();
        if let Some(window) = &self.window {
            window.set_title(&format!(
                "Nickel File — {}",
                self.browser.current().display()
            ));
        }
        self.request_redraw();
    }

    fn refresh_icons(&mut self) {
        let entries = self
            .browser
            .entries()
            .iter()
            .map(|entry| entry.path.clone())
            .collect::<Vec<_>>();
        self.icons.retain(|path, _| entries.contains(path));
        for path in entries {
            if self.icons.contains_key(&path) {
                continue;
            }
            if let Some(image) = nickel_platform::path_icon(&path) {
                let id = self.next_icon_id;
                self.next_icon_id = self.next_icon_id.checked_add(1).unwrap_or(1);
                self.icons.insert(path, (id, Arc::new(image)));
            }
        }
    }

    fn select_relative(&mut self, delta: isize) {
        let len = self.browser.entries().len();
        if len == 0 {
            self.selected = None;
            return;
        }
        let current = self.selected.unwrap_or(0) as isize;
        self.selected = Some((current + delta).clamp(0, len as isize - 1) as usize);
        self.ensure_selection_visible();
        self.request_redraw();
    }

    fn ensure_selection_visible(&mut self) {
        let Some(selected) = self.selected else {
            return;
        };
        let capacity = self.visible_capacity();
        if selected < self.scroll_offset {
            self.scroll_offset = selected;
        } else if selected >= self.scroll_offset + capacity {
            self.scroll_offset = selected + 1 - capacity;
        }
    }

    fn scroll(&mut self, rows: isize) {
        let maximum = self
            .browser
            .entries()
            .len()
            .saturating_sub(self.visible_capacity());
        self.scroll_offset =
            (self.scroll_offset as isize + rows).clamp(0, maximum as isize) as usize;
        self.request_redraw();
    }

    fn pointer_pressed(&mut self) {
        let Some(action) = self.ui.action_at(self.cursor).map(str::to_owned) else {
            return;
        };
        match action.as_str() {
            "nav:back" => self.go_back(),
            "nav:up" => self.go_up(),
            "nav:refresh" => {
                if let Err(error) = self
                    .browser
                    .set_show_hidden(nickel_platform::show_hidden_files())
                {
                    self.status = format!("Could not refresh: {error}");
                }
                self.refresh_icons();
                self.selected = None;
                self.scroll_offset = 0;
                self.request_redraw();
            }
            _ => {
                if let Some(index) = action
                    .strip_prefix("place:")
                    .and_then(|index| index.parse::<usize>().ok())
                {
                    if let Some((_, path)) = places().get(index) {
                        self.navigate_to(path.clone());
                    }
                    return;
                }
                let Some(index) = action
                    .strip_prefix("entry:")
                    .and_then(|index| index.parse::<usize>().ok())
                else {
                    return;
                };
                let now = Instant::now();
                let activate = self.last_click.is_some_and(|(previous, when)| {
                    previous == index && now.duration_since(when) <= Duration::from_millis(450)
                });
                self.selected = Some(index);
                self.last_click = Some((index, now));
                if activate {
                    self.activate_selected();
                    self.last_click = None;
                } else {
                    self.request_redraw();
                }
            }
        }
    }
}

impl ApplicationHandler for FileApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let window = Arc::new(
            event_loop
                .create_window(
                    WindowAttributes::default()
                        .with_title(format!(
                            "Nickel File — {}",
                            self.browser.current().display()
                        ))
                        .with_inner_size(LogicalSize::new(860.0, 620.0))
                        .with_min_inner_size(LogicalSize::new(560.0, 360.0)),
                )
                .expect("create Nickel File window"),
        );
        let size = window.inner_size();
        self.gpu = Some(
            ComponentGpu::new(window.clone(), size.width, size.height)
                .expect("create Nickel File renderer"),
        );
        self.window = Some(window);
        self.request_redraw();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::RedrawRequested => self.render(),
            WindowEvent::Resized(size) => {
                if let Some(gpu) = &mut self.gpu {
                    gpu.resize(size.width, size.height);
                }
                self.ensure_selection_visible();
                self.request_redraw();
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = Point {
                    x: position.x as f32,
                    y: position.y as f32,
                };
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => self.pointer_pressed(),
            WindowEvent::MouseWheel { delta, .. } => {
                let rows = match delta {
                    MouseScrollDelta::LineDelta(_, y) => {
                        -(y.round() as isize) * self.grid_dimensions().0 as isize
                    }
                    MouseScrollDelta::PixelDelta(position) => {
                        -(position.y / f64::from(TILE_HEIGHT)).round() as isize
                            * self.grid_dimensions().0 as isize
                    }
                };
                self.scroll(rows);
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                match event.logical_key {
                    Key::Named(NamedKey::ArrowDown) => {
                        self.select_relative(self.grid_dimensions().0 as isize)
                    }
                    Key::Named(NamedKey::ArrowUp) => {
                        self.select_relative(-(self.grid_dimensions().0 as isize))
                    }
                    Key::Named(NamedKey::ArrowRight) => self.select_relative(1),
                    Key::Named(NamedKey::ArrowLeft) => self.select_relative(-1),
                    Key::Named(NamedKey::Enter) => self.activate_selected(),
                    Key::Named(NamedKey::Backspace) => self.go_back(),
                    Key::Named(NamedKey::Escape) => self.selected = None,
                    Key::Named(NamedKey::F5) => {
                        if let Err(error) = self.browser.refresh() {
                            self.status = format!("Could not refresh: {error}");
                        }
                        self.request_redraw();
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

fn file_tile(
    index: usize,
    entry: &FileEntry,
    selected: bool,
    icon: Option<(u16, Arc<image::RgbaImage>)>,
) -> Container {
    let (icon_id, icon_image) = icon.unwrap_or_else(|| {
        (
            0,
            Arc::new(image::RgbaImage::from_pixel(
                1,
                1,
                image::Rgba([0, 0, 0, 0]),
            )),
        )
    });
    Container::new()
        .background(if selected { TILE_SELECTED } else { TILE })
        .border(if selected { PRIMARY } else { BORDER }, 1.0)
        .padding(Insets {
            top: 12.0,
            right: 8.0,
            bottom: 8.0,
            left: 8.0,
        })
        .action(format!("entry:{index}"))
        .child(
            Column::new()
                .gap(7.0)
                .child(Image::new(icon_id, icon_image).height(62.0))
                .child(
                    Text::new(entry.display_name())
                        .height(27.0)
                        .scale(1.2)
                        .align(TextAlign::Center)
                        .color(TEXT),
                ),
        )
}

#[cfg(not(target_os = "windows"))]
fn places() -> Vec<(String, PathBuf)> {
    let home = home_directory();
    let mut places = vec![("Home".to_owned(), home.clone())];
    for (label, folder) in [
        ("Desktop", "Desktop"),
        ("Documents", "Documents"),
        ("Downloads", "Downloads"),
        ("Pictures", "Pictures"),
        ("Music", "Music"),
        ("Videos", "Videos"),
    ] {
        let path = home.join(folder);
        if path.is_dir() {
            places.push((label.to_owned(), path));
        }
    }
    places
}

#[cfg(target_os = "windows")]
fn places() -> Vec<(String, PathBuf)> {
    use windows::Win32::UI::Shell::{
        FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Downloads, FOLDERID_Music,
        FOLDERID_Pictures, FOLDERID_Profile, FOLDERID_Videos,
    };

    [
        ("Home", &FOLDERID_Profile),
        ("Desktop", &FOLDERID_Desktop),
        ("Documents", &FOLDERID_Documents),
        ("Downloads", &FOLDERID_Downloads),
        ("Pictures", &FOLDERID_Pictures),
        ("Music", &FOLDERID_Music),
        ("Videos", &FOLDERID_Videos),
    ]
    .into_iter()
    .filter_map(|(label, id)| known_folder_path(id).map(|path| (label.to_owned(), path)))
    .collect()
}

#[cfg(target_os = "windows")]
fn known_folder_path(id: &windows::core::GUID) -> Option<PathBuf> {
    use windows::Win32::{
        System::Com::CoTaskMemFree,
        UI::Shell::{KF_FLAG_DEFAULT, SHGetKnownFolderPath},
    };

    // SAFETY: SHGetKnownFolderPath allocates a terminated string for the supplied known-folder
    // identifier. We copy it into an owned PathBuf and release the allocation with CoTaskMemFree.
    unsafe {
        let value = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).ok()?;
        let path = value.to_string().ok().map(PathBuf::from);
        CoTaskMemFree(Some(value.as_ptr().cast()));
        path.filter(|path| path.is_dir())
    }
}

#[cfg(not(target_os = "windows"))]
fn home_directory() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(target_os = "windows")]
fn home_directory() -> PathBuf {
    use windows::Win32::UI::Shell::FOLDERID_Profile;

    known_folder_path(&FOLDERID_Profile)
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(target_os = "windows")]
fn open_path(path: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
        core::PCWSTR,
    };

    let path = path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let verb = "open\0".encode_utf16().collect::<Vec<_>>();
    // SAFETY: both strings are terminated and live for the synchronous ShellExecuteW call.
    let result = unsafe {
        ShellExecuteW(
            None,
            PCWSTR(verb.as_ptr()),
            PCWSTR(path.as_ptr()),
            None,
            None,
            SW_SHOWNORMAL,
        )
    };
    if result.0 as isize > 32 {
        Ok(())
    } else {
        Err(format!("Windows shell error {}", result.0 as isize))
    }
}

#[cfg(target_os = "linux")]
fn open_path(path: &Path) -> Result<(), String> {
    std::process::Command::new("xdg-open")
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|error| error.to_string())
}

#[cfg(target_os = "macos")]
fn open_path(path: &Path) -> Result<(), String> {
    std::process::Command::new("open")
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|error| error.to_string())
}

#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
fn open_path(_path: &Path) -> Result<(), String> {
    Err("opening files is unsupported on this platform".into())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _log_path = nickel_logging::init("nickel-file").ok();
    let path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(home_directory);
    let event_loop = EventLoop::new()?;
    event_loop.run_app(&mut FileApp::new(path))?;
    Ok(())
}
