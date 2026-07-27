#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use nickel_components::{
    Button, Column, Component, ComponentGpu, Container, FileGrid, FileGridItem, HorizontalRule,
    Image, Insets, LinearGradient, Point, Rect, Row, Sidebar, SidebarFolder, SidebarSection, Text,
    UiTree,
};
use nickel_core::{
    shell_settings::ShellSettings,
    theme::{ThemeMode, ThemePalette},
};
use nickel_file::{DirectoryBrowser, FileEntry};
use winit::{
    application::ApplicationHandler,
    dpi::LogicalSize,
    event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, EventLoop},
    keyboard::{Key, NamedKey},
    window::{Icon, Window, WindowAttributes, WindowId},
};

const DEFAULT_SIDEBAR_WIDTH: f32 = 190.0;
const MIN_SIDEBAR_WIDTH: f32 = 150.0;
const MAX_SIDEBAR_WIDTH: f32 = 360.0;
const SIDEBAR_RESIZE_WIDTH: f32 = 5.0;
const TOOLBAR_HEIGHT: f32 = 78.0;
const FOOTER_HEIGHT: f32 = 30.0;
const DEFAULT_TILE_WIDTH: f32 = 150.0;
const MIN_TILE_WIDTH: f32 = 110.0;
const MAX_TILE_WIDTH: f32 = 240.0;

fn nickel_file_icon() -> Option<Icon> {
    let image = image::load_from_memory(include_bytes!("../../../assets/icons/nickel-file.png"))
        .ok()?
        .into_rgba8();
    let (width, height) = image.dimensions();
    Icon::from_rgba(image.into_raw(), width, height).ok()
}

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
    sidebar_width: f32,
    resizing_sidebar: bool,
    expanded_folders: HashSet<PathBuf>,
    hovered_action: Option<String>,
    control_down: bool,
    tile_width: f32,
    tab_icon: Option<(u16, Arc<image::RgbaImage>)>,
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
            sidebar_width: DEFAULT_SIDEBAR_WIDTH,
            resizing_sidebar: false,
            expanded_folders: HashSet::new(),
            hovered_action: None,
            control_down: false,
            tile_width: DEFAULT_TILE_WIDTH,
            tab_icon: None,
        };
        app.refresh_icons();
        app.refresh_tab_icon();
        app
    }

    fn request_redraw(&self) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn tile_height(&self) -> f32 {
        (self.tile_width * 0.72).clamp(84.0, 150.0)
    }

    fn grid_columns_for_width(&self, width: f32) -> usize {
        ((width - self.sidebar_width - SIDEBAR_RESIZE_WIDTH - 32.0) / self.tile_width)
            .floor()
            .max(1.0) as usize
    }

    fn grid_rows_for_height(&self, height: f32) -> usize {
        ((height - TOOLBAR_HEIGHT - FOOTER_HEIGHT - 30.0) / self.tile_height())
            .floor()
            .max(1.0) as usize
    }

    fn grid_dimensions(&self) -> (usize, usize) {
        self.window
            .as_ref()
            .map(|window| {
                let size = window.inner_size();
                (
                    self.grid_columns_for_width(size.width as f32),
                    self.grid_rows_for_height(size.height as f32),
                )
            })
            .unwrap_or((5, 4))
    }

    fn visible_capacity(&self) -> usize {
        let (columns, rows) = self.grid_dimensions();
        columns * rows
    }

    fn build_ui(&self, width: f32, height: f32, palette: ThemePalette, light_mode: bool) -> UiTree {
        let tab_name = self
            .browser
            .current()
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.browser.current().display().to_string());
        let breadcrumbs = breadcrumb_paths(self.browser.current());
        let tab_strip = Container::new()
            .height(32.0)
            .background(palette.panel)
            .padding(Insets {
                top: 5.0,
                right: 10.0,
                bottom: 0.0,
                left: 12.0,
            })
            .child(
                Row::new()
                    .gap(16.0)
                    .child(
                        Container::new()
                            .width(190.0)
                            .background(if light_mode {
                                0xffffff
                            } else {
                                palette.background
                            })
                            .top_corner_radius(5.0)
                            .child(
                                Column::new()
                                    .child(
                                        Row::new()
                                            .grow(1.0)
                                            .gap(6.0)
                                            .padding(Insets {
                                                top: 1.0,
                                                right: 4.0,
                                                bottom: 0.0,
                                                left: 9.0,
                                            })
                                            .child(
                                                self.tab_icon
                                                    .as_ref()
                                                    .map(|(id, icon)| {
                                                        Image::new(*id, icon.clone())
                                                            .width(18.0)
                                                            .height(18.0)
                                                            .into_element()
                                                    })
                                                    .unwrap_or_else(|| {
                                                        Container::new().width(18.0).into_element()
                                                    }),
                                            )
                                            .child(
                                                Text::new(tab_name)
                                                    .width(126.0)
                                                    .scale(1.05)
                                                    .color(palette.text),
                                            )
                                            .child(Text::new("×").width(20.0).color(palette.muted)),
                                    )
                                    .child(Container::new().height(2.0).background(palette.accent)),
                            ),
                    )
                    .child(Text::new("+").width(24.0).scale(1.25).color(palette.muted)),
            );
        let breadcrumb_row = Row::new()
            .gap(7.0)
            .children(
                breadcrumbs
                    .iter()
                    .enumerate()
                    .flat_map(|(index, (label, _))| {
                        let mut elements = Vec::new();
                        if index > 0 {
                            elements.push(
                                Text::new("›")
                                    .width(10.0)
                                    .color(palette.muted)
                                    .into_element(),
                            );
                        }
                        elements.push(
                            Container::new()
                                .action(format!("breadcrumb:{index}"))
                                .padding(Insets {
                                    top: 4.0,
                                    right: 2.0,
                                    bottom: 3.0,
                                    left: 2.0,
                                })
                                .child(Text::new(label).scale(1.05).color(palette.text))
                                .into_element(),
                        );
                        elements
                    }),
            );
        let navigation = Container::new()
            .height(46.0)
            .background(palette.surface)
            .padding(Insets {
                top: 6.0,
                right: 12.0,
                bottom: 6.0,
                left: 10.0,
            })
            .child(
                Row::new()
                    .gap(4.0)
                    .child(Button::new("nav:back", "←").width(34.0).height(34.0).color(
                        if self.browser.can_go_back() {
                            palette.text
                        } else {
                            palette.muted
                        },
                    ))
                    .child(
                        Button::new("nav:forward", "→")
                            .width(34.0)
                            .height(34.0)
                            .color(if self.browser.can_go_forward() {
                                palette.text
                            } else {
                                palette.muted
                            }),
                    )
                    .child(
                        Container::new()
                            .grow(1.0)
                            .background(palette.background)
                            .padding(Insets {
                                top: 5.0,
                                right: 12.0,
                                bottom: 4.0,
                                left: 10.0,
                            })
                            .child(breadcrumb_row),
                    )
                    .child(
                        Button::new("nav:refresh", "↻")
                            .width(34.0)
                            .height(34.0)
                            .color(palette.text),
                    ),
            );
        let toolbar = Container::new()
            .height(TOOLBAR_HEIGHT)
            .background(LinearGradient::vertical(palette.panel, palette.surface))
            .child(Column::new().child(tab_strip).child(navigation));
        let folder_rows = sidebar_folder_elements(
            &places(),
            &self.expanded_folders,
            self.browser.current(),
            self.hovered_action.as_deref(),
            palette,
        );
        let sidebar = Sidebar::new(self.sidebar_width)
            .background(palette.panel)
            .padding(Insets {
                top: 14.0,
                right: 10.0,
                bottom: 12.0,
                left: 10.0,
            })
            .gap(3.0)
            .child(
                Text::new("Nickel File")
                    .height(34.0)
                    .scale(1.55)
                    .color(palette.text),
            )
            .child(HorizontalRule::new(palette.muted).spacing(5.0, 8.0))
            .child(SidebarSection::new("Places", palette.muted).children(folder_rows));
        let capacity = self.visible_capacity();
        let (columns, _) = self.grid_dimensions();
        let visible_count = self
            .browser
            .entries()
            .len()
            .saturating_sub(self.scroll_offset)
            .min(capacity);
        let grid_rows = visible_count.div_ceil(columns);
        let grid_height =
            grid_rows as f32 * self.tile_height() + 10.0 * grid_rows.saturating_sub(1) as f32;
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
                    palette,
                    (self.tile_width * 0.42).clamp(42.0, 96.0),
                    light_mode,
                )
            });
        let files = if self.browser.entries().is_empty() {
            Column::new()
                .grow(1.0)
                .padding(Insets::all(28.0))
                .child(Text::new("This folder is empty.").color(palette.muted))
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
                    FileGrid::columns(columns)
                        .gap(10.0)
                        .height(grid_height)
                        .items(tiles),
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
            .background(palette.surface)
            .padding(Insets {
                top: 7.0,
                right: 14.0,
                bottom: 5.0,
                left: 14.0,
            })
            .child(Text::new(footer_text).scale(1.0).color(palette.muted));
        let resize_handle = Container::new()
            .width(SIDEBAR_RESIZE_WIDTH)
            .background(if self.resizing_sidebar {
                palette.accent
            } else {
                palette.surface_hover
            })
            .action("sidebar:resize");
        let content = Row::new()
            .grow(1.0)
            .child(sidebar)
            .child(resize_handle)
            .child(files);
        let root = Column::new()
            .height(height)
            .background(palette.background)
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
        let palette = ThemePalette::from_appearance(appearance);
        let size = window.inner_size();
        self.ui = self.build_ui(
            size.width as f32,
            size.height as f32,
            palette,
            appearance.mode == ThemeMode::Light,
        );
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

    fn go_forward(&mut self) {
        match self.browser.forward() {
            Ok(true) => self.navigation_changed(),
            Ok(false) => {}
            Err(error) => self.status = format!("Could not go forward: {error}"),
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
        self.refresh_tab_icon();
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

    fn refresh_tab_icon(&mut self) {
        self.tab_icon = nickel_platform::path_icon(self.browser.current()).map(|image| {
            let id = self.next_icon_id;
            self.next_icon_id = self.next_icon_id.checked_add(1).unwrap_or(1);
            (id, Arc::new(image))
        });
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
            "sidebar:resize" => {
                self.resizing_sidebar = true;
                self.request_redraw();
            }
            "nav:back" => self.go_back(),
            "nav:forward" => self.go_forward(),
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
                if let Some(path) = action.strip_prefix("folder-toggle:") {
                    let path = PathBuf::from(path);
                    if !self.expanded_folders.remove(&path) {
                        self.expanded_folders.insert(path);
                    }
                    self.request_redraw();
                    return;
                }
                if let Some(path) = action.strip_prefix("folder-open:") {
                    self.navigate_to(PathBuf::from(path));
                    return;
                }
                if let Some(index) = action
                    .strip_prefix("breadcrumb:")
                    .and_then(|index| index.parse::<usize>().ok())
                {
                    if let Some((_, path)) = breadcrumb_paths(self.browser.current()).get(index) {
                        self.navigate_to(path.clone());
                    }
                    return;
                }
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
                        .with_window_icon(nickel_file_icon())
                        .with_inner_size(LogicalSize::new(860.0, 620.0))
                        .with_min_inner_size(LogicalSize::new(560.0, 360.0)),
                )
                .expect("create Nickel File window"),
        );
        #[cfg(target_os = "windows")]
        {
            use winit::platform::windows::WindowExtWindows;
            window.set_taskbar_icon(nickel_file_icon());
        }
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
                if self.resizing_sidebar {
                    self.sidebar_width = self.cursor.x.clamp(MIN_SIDEBAR_WIDTH, MAX_SIDEBAR_WIDTH);
                    self.ensure_selection_visible();
                    self.request_redraw();
                }
                let hovered = self.ui.action_at(self.cursor).map(str::to_owned);
                if hovered != self.hovered_action {
                    self.hovered_action = hovered;
                    self.request_redraw();
                }
            }
            WindowEvent::CursorLeft { .. } => {
                self.hovered_action = None;
                self.request_redraw();
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                self.control_down = modifiers.state().control_key();
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => self.pointer_pressed(),
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: MouseButton::Left,
                ..
            } => {
                self.resizing_sidebar = false;
                self.request_redraw();
            }
            WindowEvent::MouseWheel { delta, .. } => {
                if self.control_down {
                    let direction = match delta {
                        MouseScrollDelta::LineDelta(_, y) => y.signum(),
                        MouseScrollDelta::PixelDelta(position) => position.y.signum() as f32,
                    };
                    self.tile_width =
                        (self.tile_width + direction * 12.0).clamp(MIN_TILE_WIDTH, MAX_TILE_WIDTH);
                    self.ensure_selection_visible();
                    self.request_redraw();
                    return;
                }
                let rows = match delta {
                    MouseScrollDelta::LineDelta(_, y) => {
                        -(y.round() as isize) * self.grid_dimensions().0 as isize
                    }
                    MouseScrollDelta::PixelDelta(position) => {
                        -(position.y / f64::from(self.tile_height())).round() as isize
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

fn breadcrumb_paths(current: &Path) -> Vec<(String, PathBuf)> {
    let anchor = places()
        .into_iter()
        .filter(|(_, path)| current.starts_with(path))
        .max_by_key(|(_, path)| path.components().count());
    let Some((anchor_label, anchor_path)) = anchor else {
        return vec![(current.display().to_string(), current.to_path_buf())];
    };

    let mut breadcrumbs = vec![(anchor_label, anchor_path.clone())];
    let mut path = anchor_path;
    if let Ok(relative) = current.strip_prefix(&path) {
        for component in relative.components() {
            path.push(component.as_os_str());
            breadcrumbs.push((
                component.as_os_str().to_string_lossy().into_owned(),
                path.clone(),
            ));
        }
    }
    breadcrumbs
}

fn sidebar_folder_elements(
    roots: &[(String, PathBuf)],
    expanded: &HashSet<PathBuf>,
    current: &Path,
    hovered_action: Option<&str>,
    palette: ThemePalette,
) -> Vec<nickel_components::ui::Element> {
    fn append_folder(
        rows: &mut Vec<nickel_components::ui::Element>,
        label: String,
        path: PathBuf,
        depth: usize,
        expanded: &HashSet<PathBuf>,
        current: &Path,
        hovered_action: Option<&str>,
        palette: ThemePalette,
    ) {
        let is_expanded = expanded.contains(&path);
        let is_active = current == path;
        let toggle_action = format!("folder-toggle:{}", path.display());
        let open_action = format!("folder-open:{}", path.display());
        let is_hovered = hovered_action == Some(toggle_action.as_str())
            || hovered_action == Some(open_action.as_str());
        rows.push(
            SidebarFolder::new(
                toggle_action,
                open_action,
                label,
                is_expanded,
                if is_active {
                    palette.text
                } else {
                    palette.muted
                },
            )
            .indent(depth)
            .background(if is_active {
                palette.accent_soft
            } else if is_hovered {
                palette.surface_hover
            } else {
                palette.panel
            })
            .into_element(),
        );
        if !is_expanded || depth >= 6 {
            return;
        }
        let Ok(entries) = std::fs::read_dir(&path) else {
            return;
        };
        let mut children = entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                entry
                    .file_type()
                    .ok()
                    .filter(|kind| kind.is_dir())
                    .map(|_| {
                        (
                            entry.file_name().to_string_lossy().into_owned(),
                            entry.path(),
                        )
                    })
            })
            .collect::<Vec<_>>();
        children.sort_by(|left, right| left.0.to_lowercase().cmp(&right.0.to_lowercase()));
        for (label, child) in children {
            append_folder(
                rows,
                label,
                child,
                depth + 1,
                expanded,
                current,
                hovered_action,
                palette,
            );
        }
    }

    let mut rows = Vec::new();
    for (label, path) in roots {
        append_folder(
            &mut rows,
            label.clone(),
            path.clone(),
            0,
            expanded,
            current,
            hovered_action,
            palette,
        );
    }
    rows
}

fn file_tile(
    index: usize,
    entry: &FileEntry,
    selected: bool,
    icon: Option<(u16, Arc<image::RgbaImage>)>,
    palette: ThemePalette,
    icon_size: f32,
    light_mode: bool,
) -> FileGridItem {
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
    FileGridItem::new(
        format!("entry:{index}"),
        entry.display_name(),
        icon_id,
        icon_image,
    )
    .borderless_colors(
        if selected {
            palette.accent_soft
        } else if light_mode {
            0xffffff
        } else {
            palette.background
        },
        palette.text,
    )
    .icon_size(icon_size)
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
