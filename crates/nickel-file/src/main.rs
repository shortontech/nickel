#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        Arc,
        mpsc::{self, Receiver, TryRecvError},
    },
    time::{Duration, Instant},
};

use nickel_core::{
    shell_settings::ShellSettings,
    theme::{ThemeMode, ThemePalette},
};
use nickel_file::{DirectoryBrowser, FileEntry};
use nickel_ui::{
    AnyView, Component, Insets, LinearGradient, PaintCommand, Point, Rect, SdlCanvasPresenter,
    TextAlign, UiStateStore, UiTree, ui,
};
use sdl3::{
    event::{Event, WindowEvent},
    keyboard::{Keycode, Mod},
    mouse::MouseButton,
    pixels::PixelFormat,
    surface::Surface,
    video::Window,
};

const DEFAULT_SIDEBAR_WIDTH: f32 = 190.0;
const MIN_SIDEBAR_WIDTH: f32 = 150.0;
const MAX_SIDEBAR_WIDTH: f32 = 360.0;
const SIDEBAR_RESIZE_WIDTH: f32 = 5.0;
const TOOLBAR_HEIGHT: f32 = 78.0;
const DEFAULT_TILE_WIDTH: f32 = 150.0;
const MIN_TILE_WIDTH: f32 = 110.0;
const MAX_TILE_WIDTH: f32 = 240.0;

fn set_nickel_file_icon(window: &mut Window) {
    let Ok(image) =
        image::load_from_memory(include_bytes!("../../../assets/icons/nickel-file.png"))
    else {
        return;
    };
    let mut image = image.into_rgba8();
    let (width, height) = image.dimensions();
    if let Ok(surface) = Surface::from_data(
        image.as_mut(),
        width,
        height,
        width * 4,
        PixelFormat::ABGR8888,
    ) {
        window.set_icon(&surface);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum FileMessage {
    ContextOpen,
    ContextOpenNewTab,
    ContextRefresh,
    ContextSelectAll,
    ResizeSidebar,
    NewTab,
    SwitchTab(usize),
    CloseTab(usize),
    Back,
    Forward,
    Up,
    Refresh,
    Breadcrumb(PathBuf),
    ToggleFolder(PathBuf),
    OpenFolder(PathBuf),
    Entry(usize),
    FileScroll,
}

struct FileApp {
    presenter: Option<SdlCanvasPresenter>,
    dirty: bool,
    browser: DirectoryBrowser,
    ui: UiTree<FileMessage>,
    cursor: Point,
    selected: Option<usize>,
    selected_entries: HashSet<usize>,
    selection_anchor: Option<usize>,
    active_tab_id: u64,
    next_tab_id: u64,
    status: String,
    last_click: Option<(usize, Instant)>,
    icons: HashMap<PathBuf, (u16, Arc<image::RgbaImage>)>,
    icon_rx: Option<Receiver<(PathBuf, Option<image::RgbaImage>)>>,
    next_icon_id: u16,
    sidebar_width: f32,
    expanded_folders: HashSet<PathBuf>,
    ui_state: UiStateStore,
    control_down: bool,
    shift_down: bool,
    selection_drag: Option<Point>,
    context_menu: Option<(Point, Option<usize>)>,
    resize_deadline: Option<Instant>,
    tile_width: f32,
    tab_icon: Option<(u16, Arc<image::RgbaImage>)>,
    tabs: Vec<Option<FileTab>>,
    active_tab: usize,
    exit_requested: bool,
}

struct FileTab {
    browser: DirectoryBrowser,
    selected: Option<usize>,
    selected_entries: HashSet<usize>,
    selection_anchor: Option<usize>,
    tab_id: u64,
    status: String,
    last_click: Option<(usize, Instant)>,
    tab_icon: Option<(u16, Arc<image::RgbaImage>)>,
}

impl FileApp {
    fn scroll_state_id(tab_id: u64) -> nickel_ui::UiId {
        nickel_ui::UiId::new(format!("file-tab-{tab_id}-scroll"))
    }

    fn scroll_offset(&self) -> f32 {
        self.ui_state
            .state(&Self::scroll_state_id(self.active_tab_id))
            .map(|state| state.scroll_offset)
            .unwrap_or(0.0)
    }

    fn set_scroll_offset(&mut self, offset: f32) {
        self.ui_state
            .state_mut(Self::scroll_state_id(self.active_tab_id))
            .scroll_offset = offset.max(0.0);
    }

    fn hovered_message(&self) -> Option<&FileMessage> {
        self.ui_state
            .hovered()
            .and_then(|id| self.ui.message_for_id(id))
    }

    fn is_resizing_sidebar(&self) -> bool {
        self.ui_state
            .captured()
            .and_then(|id| self.ui.message_for_id(id))
            == Some(&FileMessage::ResizeSidebar)
    }

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
            presenter: None,
            dirty: true,
            browser,
            ui: UiTree::default(),
            cursor: Point { x: 0.0, y: 0.0 },
            selected: None,
            selected_entries: HashSet::new(),
            selection_anchor: None,
            active_tab_id: 0,
            next_tab_id: 1,
            status,
            last_click: None,
            icons: HashMap::new(),
            icon_rx: None,
            next_icon_id: 1,
            sidebar_width: DEFAULT_SIDEBAR_WIDTH,
            expanded_folders: HashSet::new(),
            ui_state: UiStateStore::default(),
            control_down: false,
            shift_down: false,
            selection_drag: None,
            context_menu: None,
            resize_deadline: None,
            tile_width: DEFAULT_TILE_WIDTH,
            tab_icon: None,
            tabs: vec![None],
            active_tab: 0,
            exit_requested: false,
        };
        app.refresh_icons();
        app.refresh_tab_icon();
        app
    }

    fn inactive_tab(&self, index: usize) -> Option<&FileTab> {
        self.tabs.get(index).and_then(Option::as_ref)
    }

    fn switch_tab(&mut self, index: usize) {
        if index == self.active_tab || index >= self.tabs.len() {
            return;
        }
        let Some(mut target) = self.tabs[index].take() else {
            return;
        };
        std::mem::swap(&mut self.browser, &mut target.browser);
        std::mem::swap(&mut self.selected, &mut target.selected);
        std::mem::swap(&mut self.selected_entries, &mut target.selected_entries);
        std::mem::swap(&mut self.selection_anchor, &mut target.selection_anchor);
        std::mem::swap(&mut self.active_tab_id, &mut target.tab_id);
        std::mem::swap(&mut self.status, &mut target.status);
        std::mem::swap(&mut self.last_click, &mut target.last_click);
        std::mem::swap(&mut self.tab_icon, &mut target.tab_icon);
        self.tabs[self.active_tab] = Some(target);
        self.active_tab = index;
        self.refresh_icons();
        self.update_window_title();
        self.request_redraw();
    }

    fn new_tab(&mut self) {
        self.new_tab_at(home_directory());
    }

    fn new_tab_at(&mut self, path: PathBuf) {
        let show_hidden = nickel_platform::show_hidden_files();
        let Ok(browser) = DirectoryBrowser::open_with_hidden(path, show_hidden) else {
            return;
        };
        let tab_id = self.next_tab_id;
        self.next_tab_id = self.next_tab_id.saturating_add(1);
        self.tabs.push(Some(FileTab {
            browser,
            selected: None,
            selected_entries: HashSet::new(),
            selection_anchor: None,
            tab_id,
            status: String::new(),
            last_click: None,
            tab_icon: None,
        }));
        self.switch_tab(self.tabs.len() - 1);
    }

    fn close_tab(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }
        if self.tabs.len() == 1 {
            self.exit_requested = true;
            return;
        }
        if index != self.active_tab {
            self.tabs.remove(index);
            if index < self.active_tab {
                self.active_tab -= 1;
            }
            self.request_redraw();
            return;
        }
        let target = if index + 1 < self.tabs.len() {
            index + 1
        } else {
            index - 1
        };
        self.switch_tab(target);
        self.tabs.remove(index);
        if index < self.active_tab {
            self.active_tab -= 1;
        }
        self.request_redraw();
    }

    fn update_window_title(&mut self) {
        if let Some(presenter) = &mut self.presenter {
            let _ = presenter.window_mut().set_title(&format!(
                "Nickel File — {}",
                self.browser.current().display()
            ));
        }
    }

    fn request_redraw(&mut self) {
        self.dirty = true;
    }

    fn finish_resize_if_due(&mut self) {
        let Some(deadline) = self.resize_deadline else {
            return;
        };
        if Instant::now() < deadline {
            return;
        }
        self.resize_deadline = None;
        self.ensure_selection_visible();
        self.request_redraw();
    }

    fn build_ui(
        &self,
        width: f32,
        height: f32,
        palette: ThemePalette,
        light_mode: bool,
    ) -> UiTree<FileMessage> {
        let tabs = (0..self.tabs.len()).map(|index| {
            let active = index == self.active_tab;
            let tab = if active {
                file_tab_element(
                    index,
                    &self.browser,
                    self.tab_icon.as_ref(),
                    true,
                    palette,
                    light_mode,
                )
            } else {
                let tab = self
                    .inactive_tab(index)
                    .expect("inactive tab slot must contain state");
                file_tab_element(
                    index,
                    &tab.browser,
                    tab.tab_icon.as_ref(),
                    false,
                    palette,
                    light_mode,
                )
            };
            tab.into_element()
        });
        let breadcrumbs = breadcrumb_paths(self.browser.current());
        let tab_strip = ui! {
            <Container height={32.0} background={palette.panel} padding={Insets {
                top: 5.0, right: 10.0, bottom: 0.0, left: 12.0,
            }}>
                <Row gap={3.0} children={tabs}>
                    <Container width={28.0} on_press={FileMessage::NewTab} padding={Insets {
                        top: 1.0, right: 4.0, bottom: 0.0, left: 4.0,
                    }}>
                        <Text width={20.0} scale={1.25} color={palette.muted}>{"+"}</Text>
                    </Container>
                </Row>
            </Container>
        };
        let breadcrumb_row = ui! {
            <Row gap={7.0}>
                {breadcrumbs.iter().enumerate().map(|(index, (label, path))| ui! {
                    <Row gap={7.0}>
                        {if index > 0 {
                            ui! { <Text width={10.0} color={palette.muted}>{"›"}</Text> }
                        } else {
                            ui! { <></> }
                        }}
                        <Container on_press={FileMessage::Breadcrumb(path.clone())} padding={Insets {
                            top: 4.0, right: 2.0, bottom: 3.0, left: 2.0,
                        }}>
                            <Text scale={1.05} color={palette.text}>{label}</Text>
                        </Container>
                    </Row>
                })}
            </Row>
        };
        let navigation = ui! {
            <Container height={46.0} background={palette.surface} padding={Insets {
                top: 6.0, right: 12.0, bottom: 6.0, left: 10.0,
            }}>
                <Row gap={4.0}>
                    <Button on_press={FileMessage::Back} width={34.0} height={34.0}
                        color={if self.browser.can_go_back() { palette.text } else { palette.muted }}>
                        {"←"}
                    </Button>
                    <Button on_press={FileMessage::Forward} width={34.0} height={34.0}
                        color={if self.browser.can_go_forward() { palette.text } else { palette.muted }}>
                        {"→"}
                    </Button>
                    <Button on_press={FileMessage::Up} width={34.0} height={34.0} color={palette.text}>{"↑"}</Button>
                    <Container grow={1.0} background={palette.background} padding={Insets {
                        top: 5.0, right: 12.0, bottom: 4.0, left: 10.0,
                    }}>
                        {breadcrumb_row}
                    </Container>
                    <Button on_press={FileMessage::Refresh} width={34.0} height={34.0} color={palette.text}>{"↻"}</Button>
                </Row>
            </Container>
        };
        let toolbar = ui! {
            <Container height={TOOLBAR_HEIGHT} background={LinearGradient::vertical(palette.panel, palette.surface)}>
                <Column>{tab_strip}{navigation}</Column>
            </Container>
        };
        let folder_rows = sidebar_folder_elements(
            &places(),
            &self.expanded_folders,
            self.browser.current(),
            self.hovered_message(),
            palette,
        );
        let sidebar = ui! {
            <Sidebar width={self.sidebar_width} background={palette.panel} padding={Insets {
                top: 14.0, right: 10.0, bottom: 12.0, left: 10.0,
            }} gap={3.0}>
                <Text height={34.0} scale={1.55} color={palette.text}>{"Nickel File"}</Text>
                <HorizontalRule color={palette.muted} spacing_pair={(5.0, 8.0)} />
                <SidebarSection title={"Places"} color={palette.muted}>{folder_rows.into_iter()}</SidebarSection>
            </Sidebar>
        };
        let tiles = self
            .browser
            .entries()
            .iter()
            .enumerate()
            .map(|(index, entry)| {
                file_tile(
                    index,
                    entry,
                    self.selected_entries.contains(&index),
                    self.icons.get(&entry.path).cloned(),
                    palette,
                    (self.tile_width * 0.42).clamp(42.0, 96.0),
                    light_mode,
                )
            });
        let files = if self.browser.entries().is_empty() {
            ui! {
                <Column grow={1.0} padding={Insets::all(28.0)}>
                    <Text color={palette.muted}>{"This folder is empty."}</Text>
                </Column>
            }
        } else {
            ui! {
                <Column grow={1.0} padding={Insets {
                    top: 14.0, right: 16.0, bottom: 14.0, left: 16.0,
                }}>
                    <VerticalScroll id={"file-list"} on_scroll={FileMessage::FileScroll} offset={self.scroll_offset()}>
                        <FileGrid min_width={self.tile_width} gap={10.0} items={tiles} />
                    </VerticalScroll>
                </Column>
            }
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
        let footer = ui! {
            <Container height={30.0} background={palette.surface} padding={Insets {
                top: 7.0, right: 14.0, bottom: 5.0, left: 14.0,
            }}>
                <Text scale={1.0} color={palette.muted}>{footer_text}</Text>
            </Container>
        };
        let resize_handle = ui! {
            <Container width={SIDEBAR_RESIZE_WIDTH} shrink={0.0}
                background={if self.is_resizing_sidebar() { palette.accent } else { palette.surface_hover }}
                on_press={FileMessage::ResizeSidebar} />
        };
        let content = ui! { <Row grow={1.0}>{sidebar}{resize_handle}{files}</Row> };
        let root = ui! {
            <Column height={height} background={palette.background}>{toolbar}{content}{footer}</Column>
        };
        let mut tree = UiTree::layout(root, Rect::new(0.0, 0.0, width, height));
        if let Some(start) = self.selection_drag {
            let rect = rect_between(start, self.cursor);
            tree.push_overlay_command(PaintCommand::OverlayFill {
                rect,
                color: 0x4068_b8ff,
            });
            tree.push_overlay_command(PaintCommand::OverlayStroke {
                rect,
                color: palette.accent,
                width: 1.0,
            });
        }
        if let Some((point, entry)) = self.context_menu {
            let labels: [(FileMessage, &str); 2] = if entry.is_some() {
                [
                    (FileMessage::ContextOpen, "Open"),
                    (FileMessage::ContextOpenNewTab, "Open in New Tab"),
                ]
            } else {
                [
                    (FileMessage::ContextRefresh, "Refresh"),
                    (FileMessage::ContextSelectAll, "Select All"),
                ]
            };
            let menu_width = 170.0;
            let row_height = 34.0;
            let menu_height = labels.len() as f32 * row_height + 8.0;
            let origin = Point {
                x: point.x.min((width - menu_width - 4.0).max(4.0)),
                y: point.y.min((height - menu_height - 4.0).max(4.0)),
            };
            let menu = Rect::new(origin.x, origin.y, menu_width, menu_height);
            tree.push_overlay_command(PaintCommand::OverlayFill {
                rect: menu,
                color: palette.surface,
            });
            tree.push_overlay_command(PaintCommand::OverlayStroke {
                rect: menu,
                color: palette.muted,
                width: 1.0,
            });
            for (row, (message, label)) in labels.into_iter().enumerate() {
                let bounds = Rect::new(
                    origin.x + 8.0,
                    origin.y + 4.0 + row as f32 * row_height,
                    menu_width - 16.0,
                    row_height,
                );
                tree.push_overlay_command(PaintCommand::Text {
                    bounds,
                    text: label.to_owned(),
                    scale: 1.2,
                    color: palette.text,
                    align: TextAlign::Start,
                    bold: false,
                });
                tree.push_overlay_message(bounds, message);
            }
        }
        tree
    }

    fn render(&mut self, _events: &sdl3::EventPump) {
        let Some(size) = self
            .presenter
            .as_ref()
            .map(|presenter| presenter.window().size())
        else {
            return;
        };
        let appearance =
            ShellSettings::load_default().resolve_appearance(nickel_platform::appearance());
        let palette = ThemePalette::from_appearance(appearance);
        let ui = self.build_ui(
            size.0 as f32,
            size.1 as f32,
            palette,
            appearance.mode == ThemeMode::Light,
        );
        let retained_scrolls = std::iter::once(self.active_tab_id)
            .chain(
                self.tabs
                    .iter()
                    .filter_map(|tab| tab.as_ref().map(|tab| tab.tab_id)),
            )
            .map(Self::scroll_state_id)
            .collect::<Vec<_>>();
        ui.reconcile_state_with(&mut self.ui_state, retained_scrolls);
        self.ui = ui;
        if let Some(presenter) = &mut self.presenter {
            let pixel_width = presenter.window().size_in_pixels().0;
            let scale = pixel_width as f32 / size.0.max(1) as f32;
            let result = presenter.present_accelerated(self.ui.commands(), scale);
            if let Err(error) = result {
                tracing::warn!(%error, "failed to render Nickel File");
            }
        }
        self.dirty = false;
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
        self.selected_entries.clear();
        self.selection_anchor = None;
        self.set_scroll_offset(0.0);
        self.status.clear();
        self.refresh_icons();
        self.refresh_tab_icon();
        self.update_window_title();
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
        let mut paths = entries
            .into_iter()
            .filter(|path| !self.icons.contains_key(path))
            .collect::<Vec<_>>();
        paths.push(self.browser.current().to_path_buf());
        let (tx, rx) = mpsc::channel();
        self.icon_rx = Some(rx);
        let _ = std::thread::Builder::new()
            .name("nickel-file-icons".into())
            .spawn(move || {
                for path in paths {
                    let image = nickel_platform::path_icon(&path);
                    if tx.send((path, image)).is_err() {
                        break;
                    }
                }
            });
    }

    fn poll_icons(&mut self) {
        loop {
            let result = match self.icon_rx.as_ref() {
                Some(rx) => rx.try_recv(),
                None => return,
            };
            match result {
                Ok((path, Some(image))) => {
                    let id = self.next_icon_id;
                    self.next_icon_id = self.next_icon_id.checked_add(1).unwrap_or(1);
                    if path == self.browser.current() {
                        self.tab_icon = Some((id, Arc::new(image)));
                    } else if self
                        .browser
                        .entries()
                        .iter()
                        .any(|entry| entry.path == path)
                    {
                        self.icons.insert(path, (id, Arc::new(image)));
                    }
                    self.request_redraw();
                }
                Ok((_, None)) => {}
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => {
                    self.icon_rx = None;
                    return;
                }
            }
        }
    }

    fn refresh_tab_icon(&mut self) {
        self.tab_icon = None;
    }

    fn select_relative(&mut self, delta: isize) {
        let len = self.browser.entries().len();
        if len == 0 {
            self.selected = None;
            self.selected_entries.clear();
            self.selection_anchor = None;
            return;
        }
        let current = self.selected.unwrap_or(0) as isize;
        let next = (current + delta).clamp(0, len as isize - 1) as usize;
        self.selected = Some(next);
        if self.shift_down {
            let anchor = self.selection_anchor.unwrap_or(current as usize);
            self.selected_entries.clear();
            self.selected_entries
                .extend(anchor.min(next)..=anchor.max(next));
        } else {
            self.selected_entries.clear();
            self.selected_entries.insert(next);
            self.selection_anchor = Some(next);
        }
        self.ensure_selection_visible();
        self.request_redraw();
    }

    fn ensure_selection_visible(&mut self) {
        let Some(selected) = self.selected else {
            return;
        };
        let Some(item) = self.ui.message_layout_rect(&FileMessage::Entry(selected)) else {
            return;
        };
        let Some(viewport) = self.ui.scroll_viewport(&FileMessage::FileScroll) else {
            return;
        };
        if item.origin.y < viewport.origin.y {
            self.set_scroll_offset(
                (self.scroll_offset() - (viewport.origin.y - item.origin.y)).max(0.0),
            );
        } else {
            let item_bottom = item.origin.y + item.size.height;
            let viewport_bottom = viewport.origin.y + viewport.size.height;
            if item_bottom > viewport_bottom {
                self.set_scroll_offset(self.scroll_offset() + item_bottom - viewport_bottom);
            }
        }
    }

    fn scroll(&mut self, delta: f32) {
        let maximum = self
            .ui
            .scroll_extent(&FileMessage::FileScroll)
            .map(|extent| (extent.content.height - extent.viewport.height).max(0.0))
            .unwrap_or(0.0);
        self.set_scroll_offset((self.scroll_offset() + delta).clamp(0.0, maximum));
        self.request_redraw();
    }

    fn resolved_grid_columns(&self) -> usize {
        self.ui.resolved_grid_columns().unwrap_or(1).max(1)
    }

    fn pointer_pressed(&mut self) {
        let Some(message) = self.ui.message_at(self.cursor).cloned() else {
            self.context_menu = None;
            self.selection_drag = Some(self.cursor);
            if !self.control_down && !self.shift_down {
                self.selected = None;
                self.selected_entries.clear();
                self.selection_anchor = None;
            }
            self.request_redraw();
            return;
        };
        if !matches!(
            message,
            FileMessage::ContextOpen
                | FileMessage::ContextOpenNewTab
                | FileMessage::ContextRefresh
                | FileMessage::ContextSelectAll
        ) {
            self.context_menu = None;
        }
        match message {
            FileMessage::ContextOpen => {
                self.context_menu = None;
                self.activate_selected();
            }
            FileMessage::ContextOpenNewTab => {
                let entry = self
                    .selected
                    .and_then(|index| self.browser.entries().get(index))
                    .cloned();
                self.context_menu = None;
                if let Some(entry) = entry {
                    if entry.is_directory || entry.path.is_dir() {
                        self.new_tab_at(entry.path);
                    } else if let Err(error) = open_path(&entry.path) {
                        self.status = format!("Could not open {}: {error}", entry.display_name());
                    }
                }
            }
            FileMessage::ContextRefresh => {
                self.context_menu = None;
                if let Err(error) = self.browser.refresh() {
                    self.status = format!("Could not refresh: {error}");
                }
                self.refresh_icons();
                self.request_redraw();
            }
            FileMessage::ContextSelectAll => {
                self.context_menu = None;
                self.selected_entries = (0..self.browser.entries().len()).collect();
                self.selected = (!self.browser.entries().is_empty()).then_some(0);
                self.selection_anchor = self.selected;
                self.request_redraw();
            }
            FileMessage::ResizeSidebar => {
                self.context_menu = None;
                let id = self.ui.id_for_message(&FileMessage::ResizeSidebar).cloned();
                self.ui_state.set_pressed(id.clone());
                self.ui_state.set_capture(id);
                self.request_redraw();
            }
            FileMessage::NewTab => self.new_tab(),
            FileMessage::Back => self.go_back(),
            FileMessage::Forward => self.go_forward(),
            FileMessage::Up => self.go_up(),
            FileMessage::Refresh => {
                if let Err(error) = self
                    .browser
                    .set_show_hidden(nickel_platform::show_hidden_files())
                {
                    self.status = format!("Could not refresh: {error}");
                }
                self.refresh_icons();
                self.selected = None;
                self.selected_entries.clear();
                self.selection_anchor = None;
                self.set_scroll_offset(0.0);
                self.request_redraw();
            }
            FileMessage::CloseTab(index) => self.close_tab(index),
            FileMessage::SwitchTab(index) => self.switch_tab(index),
            FileMessage::ToggleFolder(path) => {
                if !self.expanded_folders.remove(&path) {
                    self.expanded_folders.insert(path);
                }
                self.request_redraw();
            }
            FileMessage::OpenFolder(path) | FileMessage::Breadcrumb(path) => {
                self.navigate_to(path);
            }
            FileMessage::Entry(index) => {
                let now = Instant::now();
                let activate = self.last_click.is_some_and(|(previous, when)| {
                    previous == index && now.duration_since(when) <= Duration::from_millis(450)
                });
                if self.shift_down {
                    let anchor = self.selection_anchor.unwrap_or(index);
                    self.selected_entries.clear();
                    self.selected_entries
                        .extend(anchor.min(index)..=anchor.max(index));
                } else if self.control_down {
                    if !self.selected_entries.remove(&index) {
                        self.selected_entries.insert(index);
                    }
                    self.selection_anchor = Some(index);
                } else {
                    self.selected_entries.clear();
                    self.selected_entries.insert(index);
                    self.selection_anchor = Some(index);
                }
                self.selected = Some(index);
                self.last_click = Some((index, now));
                if activate && !self.control_down && !self.shift_down {
                    self.activate_selected();
                    self.last_click = None;
                } else {
                    self.request_redraw();
                }
            }
            FileMessage::FileScroll => {
                self.selection_drag = Some(self.cursor);
                if !self.control_down && !self.shift_down {
                    self.selected = None;
                    self.selected_entries.clear();
                    self.selection_anchor = None;
                }
                self.request_redraw();
            }
        }
    }
}

impl FileApp {
    fn attach_window(&mut self, window: Window) {
        self.presenter =
            Some(SdlCanvasPresenter::new(window).expect("create accelerated presenter"));
        self.request_redraw();
    }

    fn handle_event(&mut self, event: Event) -> bool {
        match event {
            Event::Quit { .. }
            | Event::Window {
                win_event: WindowEvent::CloseRequested,
                ..
            } => return false,
            Event::Window {
                win_event: WindowEvent::Exposed,
                ..
            } => self.request_redraw(),
            Event::Window {
                win_event: WindowEvent::Resized(_, _),
                ..
            } => {}
            Event::Window {
                win_event: WindowEvent::PixelSizeChanged(_, _),
                ..
            } => {
                self.resize_deadline = Some(Instant::now() + Duration::from_millis(24));
            }
            Event::MouseMotion { x, y, .. } => {
                self.cursor = Point { x, y };
                if let Some(start) = self.selection_drag {
                    let selection = rect_between(start, self.cursor);
                    let entries = self
                        .ui
                        .messages_intersecting(selection)
                        .into_iter()
                        .filter_map(|message| match message {
                            FileMessage::Entry(index) => Some(*index),
                            _ => None,
                        })
                        .collect::<HashSet<_>>();
                    self.selected_entries = entries;
                    self.selected = self.selected_entries.iter().copied().min();
                    self.request_redraw();
                }
                if self.is_resizing_sidebar() {
                    self.sidebar_width = self.cursor.x.clamp(MIN_SIDEBAR_WIDTH, MAX_SIDEBAR_WIDTH);
                    self.ensure_selection_visible();
                    self.request_redraw();
                }
                let invalidation = self
                    .ui_state
                    .set_hovered(self.ui.id_at(self.cursor).cloned());
                if invalidation != nickel_ui::Invalidation::None {
                    self.request_redraw();
                }
            }
            Event::Window {
                win_event: WindowEvent::MouseLeave,
                ..
            } => {
                self.ui_state.set_hovered(None);
                self.request_redraw();
            }
            Event::MouseButtonDown {
                mouse_btn: MouseButton::Left,
                ..
            } => {
                self.pointer_pressed();
                if self.exit_requested {
                    return false;
                }
            }
            Event::MouseButtonDown {
                mouse_btn: MouseButton::Right,
                ..
            } => {
                let entry = self
                    .ui
                    .message_at(self.cursor)
                    .and_then(|message| match message {
                        FileMessage::Entry(index) => Some(*index),
                        _ => None,
                    });
                if let Some(index) = entry
                    && !self.selected_entries.contains(&index)
                {
                    self.selected_entries.clear();
                    self.selected_entries.insert(index);
                    self.selected = Some(index);
                    self.selection_anchor = Some(index);
                }
                self.context_menu = Some((self.cursor, entry));
                self.selection_drag = None;
                self.request_redraw();
            }
            Event::MouseButtonUp {
                mouse_btn: MouseButton::Left,
                ..
            } => {
                self.ui_state.set_pressed(None);
                self.ui_state.set_capture(None);
                self.selection_drag = None;
                self.request_redraw();
            }
            Event::MouseWheel { y, .. } => {
                if self.control_down {
                    let direction = y.signum();
                    self.tile_width =
                        (self.tile_width + direction * 12.0).clamp(MIN_TILE_WIDTH, MAX_TILE_WIDTH);
                    self.ensure_selection_visible();
                    self.request_redraw();
                    return true;
                }
                self.scroll(-y * 80.0);
            }
            Event::KeyDown {
                keycode, keymod, ..
            } => {
                self.control_down = keymod.intersects(Mod::LCTRLMOD | Mod::RCTRLMOD);
                self.shift_down = keymod.intersects(Mod::LSHIFTMOD | Mod::RSHIFTMOD);
                match keycode {
                    Some(Keycode::Down) => {
                        self.select_relative(self.resolved_grid_columns() as isize)
                    }
                    Some(Keycode::Up) => {
                        self.select_relative(-(self.resolved_grid_columns() as isize))
                    }
                    Some(Keycode::Right) => self.select_relative(1),
                    Some(Keycode::Left) => self.select_relative(-1),
                    Some(Keycode::Return | Keycode::Return2) => self.activate_selected(),
                    Some(Keycode::Backspace) => self.go_back(),
                    Some(Keycode::Escape) => {
                        self.selected = None;
                        self.selected_entries.clear();
                        self.selection_anchor = None;
                        self.request_redraw();
                    }
                    Some(Keycode::A) if self.control_down => {
                        self.selected_entries = (0..self.browser.entries().len()).collect();
                        self.selected = self
                            .selected
                            .or_else(|| (!self.browser.entries().is_empty()).then_some(0));
                        self.selection_anchor = self.selected;
                        self.request_redraw();
                    }
                    Some(Keycode::F5) => {
                        if let Err(error) = self.browser.refresh() {
                            self.status = format!("Could not refresh: {error}");
                        }
                        self.request_redraw();
                    }
                    _ => {}
                }
            }
            Event::KeyUp { keymod, .. } => {
                self.control_down = keymod.intersects(Mod::LCTRLMOD | Mod::RCTRLMOD);
                self.shift_down = keymod.intersects(Mod::LSHIFTMOD | Mod::RSHIFTMOD);
            }
            // SDL keeps text input and IME composition in the event stream. Nickel File
            // currently has no editable field, so these remain intentionally unconsumed.
            Event::TextInput { .. } | Event::TextEditing { .. } => {}
            _ => {}
        }
        true
    }
}

fn file_tab_element(
    index: usize,
    browser: &DirectoryBrowser,
    icon: Option<&(u16, Arc<image::RgbaImage>)>,
    active: bool,
    palette: ThemePalette,
    light_mode: bool,
) -> impl Component<FileMessage> {
    let label = browser
        .current()
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| browser.current().display().to_string());
    ui! {
        <Container width={170.0} on_press={FileMessage::SwitchTab(index)} background={if active {
            if light_mode {
                0xffffff
            } else {
                palette.background
            }
        } else if light_mode {
            mix_rgb(palette.panel, 0xffffff)
        } else {
            palette.panel
        }} top_corner_radius={5.0}>
            <Column>
                <Row height={25.0} gap={6.0} padding={Insets {
                    top: 4.0, right: 4.0, bottom: 3.0, left: 9.0,
                }}>
                    {if let Some((id, image)) = icon {
                        ui! { <Image asset_id={*id} image={image.clone()} width={16.0} height={16.0} /> }
                    } else {
                        ui! { <Container width={16.0} /> }
                    }}
                    <Text width={108.0} height={18.0} scale={1.05}
                        color={if active { palette.text } else { palette.muted }}>{label}</Text>
                    <Container width={20.0} on_press={FileMessage::CloseTab(index)}>
                        <Text width={20.0} color={palette.muted}>{"×"}</Text>
                    </Container>
                </Row>
                <Container height={2.0} background={if active { palette.accent } else { palette.panel }} />
            </Column>
        </Container>
    }
}

fn mix_rgb(left: u32, right: u32) -> u32 {
    let channel = |shift: u32| {
        let left = (left >> shift) & 0xff_u32;
        let right = (right >> shift) & 0xff_u32;
        (left + right) / 2
    };
    (channel(16) << 16) | (channel(8) << 8) | channel(0)
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
    hovered_message: Option<&FileMessage>,
    palette: ThemePalette,
) -> Vec<AnyView<FileMessage>> {
    #[allow(clippy::too_many_arguments)]
    fn append_folder(
        rows: &mut Vec<AnyView<FileMessage>>,
        label: String,
        path: PathBuf,
        depth: usize,
        expanded: &HashSet<PathBuf>,
        current: &Path,
        hovered_message: Option<&FileMessage>,
        palette: ThemePalette,
    ) {
        let is_expanded = expanded.contains(&path);
        let is_active = current == path;
        let toggle_message = FileMessage::ToggleFolder(path.clone());
        let open_message = FileMessage::OpenFolder(path.clone());
        let is_hovered =
            hovered_message == Some(&toggle_message) || hovered_message == Some(&open_message);
        rows.push(AnyView::new(ui! {
            <SidebarFolder on_toggle={toggle_message} on_open={open_message} label={label}
                expanded={is_expanded} foreground={if is_active { palette.text } else { palette.muted }}
                indent={depth} background={if is_active {
                    palette.accent_soft
                } else if is_hovered {
                    palette.surface_hover
                } else {
                    palette.panel
                }} />
        }));
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
                hovered_message,
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
            hovered_message,
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
) -> impl Component<FileMessage> {
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
    ui! {
        <FileGridItem on_press={FileMessage::Entry(index)} label={entry.display_name()}
            asset_id={icon_id} image={icon_image}
            borderless_palette={(if selected {
                palette.accent_soft
            } else if light_mode {
                0xffffff
            } else {
                palette.background
            }, palette.text)} icon_size={icon_size} />
    }
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

fn rect_between(start: Point, end: Point) -> Rect {
    Rect::new(
        start.x.min(end.x),
        start.y.min(end.y),
        (start.x - end.x).abs().max(1.0),
        (start.y - end.y).abs().max(1.0),
    )
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let started = Instant::now();
    let _log_path = nickel_logging::init("nickel-file").ok();
    let path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(home_directory);
    let sdl = sdl3::init()?;
    let video = sdl.video()?;
    let mut events = sdl.event_pump()?;
    let title = format!("Nickel File — {}", path.display());
    let mut window = video
        .window(&title, 860, 620)
        .position_centered()
        .resizable()
        .high_pixel_density()
        .build()?;
    window.set_minimum_size(560, 360)?;
    set_nickel_file_icon(&mut window);

    let mut app = FileApp::new(path);
    app.attach_window(window);
    app.update_window_title();
    app.render(&events);
    tracing::info!(
        target: "nickel",
        elapsed_ms = started.elapsed().as_secs_f64() * 1_000.0,
        "Nickel File first frame presented"
    );
    while !app.exit_requested {
        app.poll_icons();
        app.finish_resize_if_due();
        if app.dirty {
            app.render(&events);
        }
        let Some(event) = events.wait_event_timeout(Duration::from_millis(16)) else {
            continue;
        };
        if !app.handle_event(event) {
            break;
        }
        for event in events.poll_iter() {
            if !app.handle_event(event) {
                app.exit_requested = true;
                break;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod ui_layout_tests {
    use super::*;

    #[test]
    fn file_grid_resolves_responsively_without_application_column_arithmetic() {
        let directory = tempfile::tempdir().unwrap();
        for index in 0..18 {
            std::fs::write(directory.path().join(format!("item-{index}.txt")), b"x").unwrap();
        }
        let app = FileApp::new(directory.path().to_path_buf());
        let palette = ThemePalette::from_appearance(nickel_core::theme::Appearance::default());
        let narrow = app.build_ui(560.0, 420.0, palette, false);
        let wide = app.build_ui(1280.0, 720.0, palette, false);

        assert!(
            narrow.resolved_grid_columns().unwrap() < wide.resolved_grid_columns().unwrap(),
            "auto-fit should add columns as the resolved content pane widens"
        );
        assert!(
            narrow
                .scroll_extent(&FileMessage::FileScroll)
                .is_some_and(|extent| extent.can_scroll()),
            "all files should be measured and remain reachable through scrolling"
        );

        let source = include_str!("main.rs");
        assert!(!source.contains(&["grid_columns", "_for_width"].concat()));
        assert!(!source.contains(&["visible_", "capacity"].concat()));
        assert!(!source.contains(&["grid_", "height"].concat()));
    }
}
