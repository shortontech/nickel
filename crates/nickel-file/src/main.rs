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
use nickel_input::{
    AggregateModifier, InputEvent, KeyCode, KeyEdge, PhysicalKey, PointerButton, PointerEvent,
};
use nickel_ui::{
    AdapterOutcome, AnyView, Application, Component, ComponentBuilderExt, FrameOverlay,
    HostAdapter, HostServices, Insets, LinearGradient, NavigationScope, OverlayAnchor, OverlayMenu,
    OverlayMenuItem, Point, Rect, UiHost, UiId, ViewContext, ui,
};
use sdl3::{event::Event, pixels::PixelFormat, surface::Surface, video::Window};

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
pub enum FileMessage {
    ContextEntry(usize),
    ContextBackground,
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
    SelectionSurface,
    FileScroll,
}

pub struct FileApp {
    browser: DirectoryBrowser,
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
    icon_poll_delay: std::time::Duration,
    next_icon_id: u16,
    sidebar_width: f32,
    expanded_folders: HashSet<PathBuf>,
    control_down: bool,
    shift_down: bool,
    selection_drag: Option<Point>,
    resizing_sidebar: bool,
    tile_width: f32,
    tab_icon: Option<(u16, Arc<image::RgbaImage>)>,
    tabs: Vec<Option<FileTab>>,
    active_tab: usize,
    pending_ensure_visible: bool,
    pending_scroll_reset: bool,
    resolved_grid_columns: usize,
    exit_requested: bool,
}

pub struct FileFixtureProvider;

pub struct FileWorkbenchFixture;

const FILE_FIXTURE_VARIANTS: &[nickel_ui_testkit::FixtureVariant] = &[
    nickel_ui_testkit::FixtureVariant {
        id: "wide",
        title: "Wide",
        viewport: nickel_ui_testkit::ViewportPreset {
            id: "wide",
            width: 960,
            height: 640,
        },
        theme: nickel_ui_testkit::FixtureTheme::Dark,
        locale: nickel_ui_testkit::DEFAULT_LOCALE,
        scale: nickel_ui_testkit::DEFAULT_SCALE,
        controller_family: nickel_ui::ControllerFamily::Generic,
        accessibility: nickel_ui_testkit::DEFAULT_ACCESSIBILITY,
    },
    nickel_ui_testkit::FixtureVariant {
        id: "narrow-200",
        title: "Narrow 200%",
        viewport: nickel_ui_testkit::ViewportPreset {
            id: "narrow",
            width: 540,
            height: 420,
        },
        theme: nickel_ui_testkit::FixtureTheme::Dark,
        locale: nickel_ui_testkit::DEFAULT_LOCALE,
        scale: nickel_ui_testkit::ScalePreset {
            id: "2x",
            factor: 2.0,
        },
        controller_family: nickel_ui::ControllerFamily::Generic,
        accessibility: nickel_ui_testkit::DEFAULT_ACCESSIBILITY,
    },
];

static FILE_FIXTURE_METADATA: nickel_ui_testkit::FixtureMetadata =
    nickel_ui_testkit::FixtureMetadata {
        id: "file.browser",
        title: "Nickel File",
        description: "Production Nickel File browser surface",
        tags: &["file", "browser", "collection", "context-menu"],
        source: nickel_ui_testkit::FixtureSource {
            crate_name: "nickel-file",
            file: file!(),
            line: line!(),
        },
        variants: FILE_FIXTURE_VARIANTS,
        assets: &[],
        simulated_effects: &[],
    };

impl nickel_ui_testkit::Fixture for FileWorkbenchFixture {
    type App = FileApp;
    fn metadata() -> &'static nickel_ui_testkit::FixtureMetadata {
        &FILE_FIXTURE_METADATA
    }
    fn create() -> Self::App {
        FileApp::fixture()
    }
    fn create_variant(_: &nickel_ui_testkit::FixtureVariant) -> Self::App {
        FileApp::fixture()
    }
    fn surface_size() -> (u32, u32) {
        (960, 640)
    }
    fn default_activation() -> Option<nickel_ui_testkit::Selector> {
        Some(nickel_ui_testkit::Selector::role_name(
            nickel_ui::SemanticRole::Button,
            "report.txt",
        ))
    }
}

impl nickel_ui_testkit::FixtureProvider for FileFixtureProvider {
    fn register(
        &self,
        registry: &mut nickel_ui_testkit::FixtureRegistry,
    ) -> Result<(), nickel_ui_testkit::RegistryError> {
        registry.register::<FileWorkbenchFixture>()
    }
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
    fn is_resizing_sidebar(&self) -> bool {
        self.resizing_sidebar
    }

    pub fn new(path: PathBuf) -> Self {
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
        Self::with_browser(browser, status)
    }

    fn with_browser(browser: DirectoryBrowser, status: String) -> Self {
        let mut app = Self {
            browser,
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
            icon_poll_delay: std::time::Duration::from_millis(16),
            next_icon_id: 1,
            sidebar_width: DEFAULT_SIDEBAR_WIDTH,
            expanded_folders: HashSet::new(),
            control_down: false,
            shift_down: false,
            selection_drag: None,
            resizing_sidebar: false,
            tile_width: DEFAULT_TILE_WIDTH,
            tab_icon: None,
            tabs: vec![None],
            active_tab: 0,
            pending_ensure_visible: false,
            pending_scroll_reset: false,
            resolved_grid_columns: 1,
            exit_requested: false,
        };
        app.refresh_icons();
        app.refresh_tab_icon();
        app
    }

    fn fixture() -> Self {
        let entries = [
            ("Documents", true, None),
            ("report.txt", false, Some(128)),
            ("notes.md", false, Some(512)),
        ]
        .into_iter()
        .map(|(name, is_directory, size)| FileEntry {
            name: name.into(),
            path: PathBuf::from("/fixture").join(name),
            is_directory,
            size,
        })
        .collect();
        Self::with_browser(DirectoryBrowser::fixture(entries), String::new())
    }

    fn set_scroll_offset(&mut self, offset: f32) {
        if offset <= 0.0 {
            self.pending_scroll_reset = true;
        }
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
    }

    fn build_view(
        &self,
        _width: f32,
        height: f32,
        palette: ThemePalette,
        light_mode: bool,
    ) -> AnyView<FileMessage> {
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
            <Container id={"tab-strip"} height={32.0} background={palette.panel} padding={Insets {
                top: 5.0, right: 10.0, bottom: 0.0, left: 12.0,
            }}>
                <Row gap={3.0} children={tabs}>
                    <Container width={28.0} on_press={FileMessage::NewTab}
                        focus_border={palette.accent} controller_focus_border={palette.complement} padding={Insets {
                        top: 1.0, right: 4.0, bottom: 0.0, left: 4.0,
                    }} accessibility_label={"New tab"}>
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
                        <Container on_press={FileMessage::Breadcrumb(path.clone())}
                            focus_border={palette.accent} controller_focus_border={palette.complement} padding={Insets {
                            top: 4.0, right: 2.0, bottom: 3.0, left: 2.0,
                        }} accessibility_label={format!("Open {label}")}>
                            <Text scale={1.05} color={palette.text}>{label}</Text>
                        </Container>
                    </Row>
                })}
            </Row>
        };
        let navigation = ui! {
            <Container id={"navigation-toolbar"} height={46.0} background={palette.surface} padding={Insets {
                top: 6.0, right: 12.0, bottom: 6.0, left: 10.0,
            }}>
                <Row gap={4.0}>
                    <Button on_press={FileMessage::Back} width={34.0} height={34.0}
                        focus_border={palette.accent} controller_focus_border={palette.complement}
                        color={if self.browser.can_go_back() { palette.text } else { palette.muted }} accessibility_label={"Back"}>
                        {"←"}
                    </Button>
                    <Button on_press={FileMessage::Forward} width={34.0} height={34.0}
                        focus_border={palette.accent} controller_focus_border={palette.complement}
                        color={if self.browser.can_go_forward() { palette.text } else { palette.muted }} accessibility_label={"Forward"}>
                        {"→"}
                    </Button>
                    <Button on_press={FileMessage::Up} width={34.0} height={34.0} color={palette.text}
                        focus_border={palette.accent} controller_focus_border={palette.complement} accessibility_label={"Up one folder"}>{"↑"}</Button>
                    <Container grow={1.0} background={palette.background} padding={Insets {
                        top: 5.0, right: 12.0, bottom: 4.0, left: 10.0,
                    }}>
                        {breadcrumb_row}
                    </Container>
                    <Button on_press={FileMessage::Refresh} width={34.0} height={34.0} color={palette.text}
                        focus_border={palette.accent} controller_focus_border={palette.complement} accessibility_label={"Refresh"}>{"↻"}</Button>
                </Row>
            </Container>
        };
        let toolbar = ui! {
            <Container id={"toolbar-pane"} height={TOOLBAR_HEIGHT}
                navigation_scope={NavigationScope::pane(false)} navigation_scope_highlight={palette.complement}
                background={LinearGradient::vertical(palette.panel, palette.surface)}>
                <Column>{tab_strip}{navigation}</Column>
            </Container>
        };
        let folder_rows = sidebar_folder_elements(
            &places(),
            &self.expanded_folders,
            self.browser.current(),
            None,
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
                <Container id={"file-content"} grow={1.0} padding={Insets::all(28.0)}
                    on_press={FileMessage::SelectionSurface} context_message={FileMessage::ContextBackground}
                    focus_border={palette.accent} controller_focus_border={palette.complement}
                    accessibility_label={"Files"}>
                    <Text color={palette.muted}>{"This folder is empty."}</Text>
                </Container>
            }
        } else {
            ui! {
                <Column grow={1.0} padding={Insets {
                    top: 14.0, right: 16.0, bottom: 14.0, left: 16.0,
                }}>
                    <VerticalScroll id={"file-list"} on_scroll={FileMessage::FileScroll} offset={0.0}>
                        <FileGrid min_width={self.tile_width} gap={10.0} items={tiles} />
                    </VerticalScroll>
                    <Container id={"file-content"} height={1.0} on_press={FileMessage::SelectionSurface}
                        context_message={FileMessage::ContextBackground} focus_border={palette.accent}
                        controller_focus_border={palette.complement} accessibility_label={"Files background"} />
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
            <Container id={"sidebar-resize"} width={SIDEBAR_RESIZE_WIDTH} shrink={0.0}
                background={if self.is_resizing_sidebar() { palette.accent } else { palette.surface_hover }}
                on_press={FileMessage::ResizeSidebar} focus_border={palette.accent}
                controller_focus_border={palette.complement} accessibility_label={"Resize sidebar"} />
        };
        let content = ui! {
            <Container id={"file-layout"} grow={1.0} accessibility_label={"Files"}>
                <Row grow={1.0}><Container id={"sidebar-pane"} navigation_scope={NavigationScope::pane(false)}
                    navigation_scope_highlight={palette.complement}><Row>{sidebar}{resize_handle}</Row></Container>
                    <Container id={"files-pane"} grow={1.0}
                    navigation_scope={NavigationScope::pane(true)} navigation_scope_highlight={palette.complement}>{files}</Container></Row>
            </Container>
        };
        let root = ui! {
            <Column height={height} background={palette.background}>{toolbar}{content}{footer}</Column>
        };
        AnyView::new(root)
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
            }
        }
    }

    fn go_back(&mut self) {
        match self.browser.back() {
            Ok(true) => self.navigation_changed(),
            Ok(false) => {}
            Err(error) => self.status = format!("Could not go back: {error}"),
        }
    }

    fn go_forward(&mut self) {
        match self.browser.forward() {
            Ok(true) => self.navigation_changed(),
            Ok(false) => {}
            Err(error) => self.status = format!("Could not go forward: {error}"),
        }
    }

    fn go_up(&mut self) {
        match self.browser.up() {
            Ok(true) => self.navigation_changed(),
            Ok(false) => {}
            Err(error) => self.status = format!("Could not open parent: {error}"),
        }
    }

    fn navigation_changed(&mut self) {
        self.selected = None;
        self.selected_entries.clear();
        self.selection_anchor = None;
        self.set_scroll_offset(0.0);
        self.status.clear();
        self.refresh_icons();
        self.refresh_tab_icon();
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
        self.icon_poll_delay = std::time::Duration::from_millis(16);
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
                    self.icon_poll_delay = std::time::Duration::from_millis(16);
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
                }
                Ok((_, None)) => {}
                Err(TryRecvError::Empty) => {
                    self.icon_poll_delay = self
                        .icon_poll_delay
                        .saturating_mul(2)
                        .min(std::time::Duration::from_millis(250));
                    return;
                }
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
    }

    fn ensure_selection_visible(&mut self) {
        self.pending_ensure_visible = self.selected.is_some();
    }

    fn resolved_grid_columns(&self) -> usize {
        self.resolved_grid_columns.max(1)
    }

    fn update_message(&mut self, message: FileMessage) {
        match message {
            FileMessage::ContextEntry(index) => {
                if !self.selected_entries.contains(&index) {
                    self.selected_entries.clear();
                    self.selected_entries.insert(index);
                    self.selected = Some(index);
                    self.selection_anchor = Some(index);
                }
                self.selection_drag = None;
            }
            FileMessage::ContextBackground => {
                self.selection_drag = None;
            }
            FileMessage::ContextOpen => {
                self.activate_selected();
            }
            FileMessage::ContextOpenNewTab => {
                let entry = self
                    .selected
                    .and_then(|index| self.browser.entries().get(index))
                    .cloned();
                if let Some(entry) = entry {
                    if entry.is_directory || entry.path.is_dir() {
                        self.new_tab_at(entry.path);
                    } else if let Err(error) = open_path(&entry.path) {
                        self.status = format!("Could not open {}: {error}", entry.display_name());
                    }
                }
            }
            FileMessage::ContextRefresh => {
                if let Err(error) = self.browser.refresh() {
                    self.status = format!("Could not refresh: {error}");
                }
                self.refresh_icons();
            }
            FileMessage::ContextSelectAll => {
                self.selected_entries = (0..self.browser.entries().len()).collect();
                self.selected = (!self.browser.entries().is_empty()).then_some(0);
                self.selection_anchor = self.selected;
            }
            FileMessage::ResizeSidebar => {
                self.resizing_sidebar = true;
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
            }
            FileMessage::CloseTab(index) => self.close_tab(index),
            FileMessage::SwitchTab(index) => self.switch_tab(index),
            FileMessage::ToggleFolder(path) => {
                if !self.expanded_folders.remove(&path) {
                    self.expanded_folders.insert(path);
                }
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
                }
            }
            FileMessage::SelectionSurface => {
                self.selection_drag = Some(self.cursor);
                if !self.control_down && !self.shift_down {
                    self.selected = None;
                    self.selected_entries.clear();
                    self.selection_anchor = None;
                }
            }
            FileMessage::FileScroll => {}
        }
    }
}

impl Application for FileApp {
    type Message = FileMessage;

    fn update(&mut self, message: Self::Message) {
        self.update_message(message);
    }

    fn view(&self, context: ViewContext) -> impl nickel_ui::View<Self::Message> {
        let appearance =
            ShellSettings::load_default().resolve_appearance(nickel_platform::appearance());
        self.build_view(
            context.viewport.size.width,
            context.viewport.size.height,
            ThemePalette::from_appearance(appearance),
            appearance.mode == ThemeMode::Light,
        )
    }

    fn frame_overlays(&self, context: ViewContext) -> Vec<FrameOverlay<Self::Message>> {
        let appearance =
            ShellSettings::load_default().resolve_appearance(nickel_platform::appearance());
        let palette = ThemePalette::from_appearance(appearance);
        let invocation_anchor = |target: UiId| match context.modality {
            nickel_ui::InputModality::Pointer => OverlayAnchor::Point {
                invocation_target: target,
                point: self.cursor,
            },
            nickel_ui::InputModality::Keyboard
            | nickel_ui::InputModality::Controller
            | nickel_ui::InputModality::Accessibility => {
                OverlayAnchor::InvocationTargetCenter(target)
            }
        };
        let configure = |mut menu: OverlayMenu<FileMessage>| {
            menu.width = 170.0;
            menu.row_height = 34.0;
            menu.padding = Insets::all(4.0);
            menu.radius = 7.0;
            menu.background = palette.surface;
            menu.foreground = palette.text;
            menu.text_scale = 1.2;
            menu.item_hover = Some(palette.surface_hover);
            menu.item_pressed = Some(palette.accent_soft);
            menu.item_selected = Some(palette.accent_soft);
            menu.item_radius = 5.0;
            menu
        };
        let mut overlays = self
            .browser
            .entries()
            .iter()
            .enumerate()
            .map(|(index, _)| {
                FrameOverlay::Menu(configure(
                    OverlayMenu::new(
                        format!("file-entry-{index}-context"),
                        invocation_anchor(UiId::new(format!("file-entry-{index}"))),
                    )
                    .item(OverlayMenuItem::action(
                        "open",
                        "Open",
                        FileMessage::ContextOpen,
                    ))
                    .item(OverlayMenuItem::action(
                        "open-new-tab",
                        "Open in New Tab",
                        FileMessage::ContextOpenNewTab,
                    )),
                ))
            })
            .collect::<Vec<_>>();
        overlays.push(FrameOverlay::Menu(configure(
            OverlayMenu::new(
                "file-background-context",
                invocation_anchor(UiId::from("file-content")),
            )
            .item(OverlayMenuItem::action(
                "refresh",
                "Refresh",
                FileMessage::ContextRefresh,
            ))
            .item(OverlayMenuItem::action(
                "select-all",
                "Select All",
                FileMessage::ContextSelectAll,
            )),
        )));
        if let Some(start) = self.selection_drag {
            overlays.push(FrameOverlay::SelectionMarquee {
                rect: rect_between(start, self.cursor),
                fill: Some(0x4068_b8ff),
                stroke: palette.accent,
                width: 1.0,
            });
        }
        overlays
    }

    fn poll(&mut self) -> bool {
        let before = self.next_icon_id;
        self.poll_icons();
        before != self.next_icon_id
    }

    fn poll_interval(&self) -> Option<std::time::Duration> {
        self.icon_rx.as_ref().map(|_| self.icon_poll_delay)
    }

    fn title(&self) -> &str {
        "Nickel File"
    }

    fn initial_size(&self) -> (u32, u32) {
        (860, 620)
    }
}

struct FileHostAdapter {
    input: nickel_input::sdl::Adapter,
    sync_requested: bool,
}

impl Default for FileHostAdapter {
    fn default() -> Self {
        Self {
            input: nickel_input::sdl::Adapter::default(),
            sync_requested: true,
        }
    }
}

impl HostAdapter<FileApp> for FileHostAdapter {
    fn next_deadline(&self, now: Instant) -> Option<Instant> {
        self.sync_requested.then_some(now)
    }

    fn started(
        &mut self,
        _host: &mut UiHost<FileApp>,
        mut services: HostServices<'_>,
    ) -> Result<AdapterOutcome, Box<dyn std::error::Error>> {
        services.window().set_minimum_size(560, 360)?;
        set_nickel_file_icon(services.window());
        Ok(AdapterOutcome::default())
    }

    fn event(
        &mut self,
        host: &mut UiHost<FileApp>,
        event: &Event,
        _services: HostServices<'_>,
    ) -> Result<AdapterOutcome, Box<dyn std::error::Error>> {
        let Some(event) = self.input.normalize(event) else {
            return Ok(AdapterOutcome::default());
        };
        let mut changed = false;
        match event {
            InputEvent::Key(key) => {
                let app = host.application_mut();
                app.control_down = key.modifiers.aggregate(AggregateModifier::Control);
                app.shift_down = key.modifiers.aggregate(AggregateModifier::Shift);
                if key.edge != KeyEdge::Pressed || key.repeat {
                    return Ok(AdapterOutcome::default());
                }
                let PhysicalKey::Code(key) = key.physical else {
                    return Ok(AdapterOutcome::default());
                };
                match key {
                    KeyCode::ArrowDown => app.select_relative(app.resolved_grid_columns() as isize),
                    KeyCode::ArrowUp => {
                        app.select_relative(-(app.resolved_grid_columns() as isize))
                    }
                    KeyCode::ArrowRight => app.select_relative(1),
                    KeyCode::ArrowLeft => app.select_relative(-1),
                    KeyCode::Backspace => app.go_back(),
                    KeyCode::Escape => {
                        app.selected = None;
                        app.selected_entries.clear();
                        app.selection_anchor = None;
                    }
                    KeyCode::KeyA if app.control_down => {
                        app.selected_entries = (0..app.browser.entries().len()).collect();
                        app.selected = app
                            .selected
                            .or_else(|| (!app.browser.entries().is_empty()).then_some(0));
                        app.selection_anchor = app.selected;
                    }
                    KeyCode::F5 => {
                        if let Err(error) = app.browser.refresh() {
                            app.status = format!("Could not refresh: {error}");
                        }
                    }
                    _ => {}
                }
                changed = true;
            }
            InputEvent::Pointer(PointerEvent::Motion { position, .. }) => {
                let cursor = Point {
                    x: position.x as f32,
                    y: position.y as f32,
                };
                let selection_drag = host.application().selection_drag;
                let resizing = host.application().is_resizing_sidebar();
                let selected_entries = selection_drag.map(|start| {
                    let selection = rect_between(start, cursor);
                    host.semantic_nodes()
                        .into_iter()
                        .filter_map(|node| {
                            node.id
                                .as_str()
                                .rsplit('/')
                                .next()
                                .unwrap_or_default()
                                .strip_prefix("file-entry-")
                                .and_then(|value| value.parse::<usize>().ok())
                                .filter(|_| rects_intersect(selection, node.bounds))
                        })
                        .collect::<HashSet<_>>()
                });
                let app = host.application_mut();
                app.cursor = cursor;
                if let Some(entries) = selected_entries {
                    app.selected_entries = entries;
                    app.selected = app.selected_entries.iter().copied().min();
                }
                if resizing {
                    app.sidebar_width = cursor.x.clamp(MIN_SIDEBAR_WIDTH, MAX_SIDEBAR_WIDTH);
                    app.ensure_selection_visible();
                }
                changed = selection_drag.is_some() || resizing;
            }
            InputEvent::Pointer(PointerEvent::Button {
                button: PointerButton::Secondary,
                edge: KeyEdge::Pressed,
                position: Some(position),
                ..
            }) => {
                host.application_mut().cursor = Point {
                    x: position.x as f32,
                    y: position.y as f32,
                };
                changed = true;
            }
            InputEvent::Pointer(PointerEvent::Button {
                button: PointerButton::Primary,
                edge: KeyEdge::Released,
                ..
            }) => {
                let app = host.application_mut();
                app.resizing_sidebar = false;
                app.selection_drag = None;
                changed = true;
            }
            InputEvent::Pointer(PointerEvent::Axis { delta, .. }) => {
                let y = delta.y as f32;
                let app = host.application_mut();
                if app.control_down {
                    app.tile_width =
                        (app.tile_width + y.signum() * 12.0).clamp(MIN_TILE_WIDTH, MAX_TILE_WIDTH);
                    app.ensure_selection_visible();
                    changed = true;
                }
            }
            InputEvent::FocusLost { .. } | InputEvent::DeviceRemoved { .. } => {
                let app = host.application_mut();
                app.control_down = false;
                app.shift_down = false;
                app.resizing_sidebar = false;
                app.selection_drag = None;
            }
            _ => {}
        }
        self.sync_requested |= changed;
        Ok(AdapterOutcome {
            changed,
            consume: false,
            exit: host.application().exit_requested,
        })
    }

    fn poll(
        &mut self,
        host: &mut UiHost<FileApp>,
        mut services: HostServices<'_>,
    ) -> Result<AdapterOutcome, Box<dyn std::error::Error>> {
        self.sync_requested = false;
        host.application_mut().resolved_grid_columns =
            host.resolved_grid_columns().unwrap_or(1).max(1);
        let pending_reset = std::mem::take(&mut host.application_mut().pending_scroll_reset);
        if pending_reset {
            host.reset_scroll(&FileMessage::FileScroll);
        }
        let pending_ensure = std::mem::take(&mut host.application_mut().pending_ensure_visible);
        let selected = host.application().selected;
        if pending_ensure && let Some(selected) = selected {
            host.ensure_message_visible(&FileMessage::Entry(selected), &FileMessage::FileScroll);
        }
        let title = format!(
            "Nickel File — {}",
            host.application().browser.current().display()
        );
        let _ = services.window().set_title(&title);
        Ok(if host.application().exit_requested {
            AdapterOutcome::exit()
        } else {
            AdapterOutcome::default()
        })
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
        <Container width={170.0} background={if active {
            if light_mode {
                0xffffff
            } else {
                palette.background
            }
        } else if light_mode {
            mix_rgb(palette.panel, 0xffffff)
        } else {
            palette.panel
        }} top_corner_radius={5.0} accessibility_label={format!("Tab {label}")}>
            <Column>
                <Row height={25.0} gap={6.0} padding={Insets {
                    top: 4.0, right: 4.0, bottom: 3.0, left: 9.0,
                }}>
                    <Container width={125.0} height={25.0} on_press={FileMessage::SwitchTab(index)}
                        focus_border={palette.accent} controller_focus_border={palette.complement}
                        accessibility_label={format!("Tab {label}")}><Row gap={6.0}>
                        {if let Some((id, image)) = icon {
                            ui! { <Image asset_id={*id} image={image.clone()} width={16.0} height={16.0} /> }
                        } else {
                            ui! { <Container width={16.0} /> }
                        }}
                        <Text width={97.0} height={18.0} scale={1.05}
                            color={if active { palette.text } else { palette.muted }}>{label.clone()}</Text>
                        </Row></Container>
                    <Container width={20.0} on_press={FileMessage::CloseTab(index)}
                        focus_border={palette.accent} controller_focus_border={palette.complement} accessibility_label={format!("Close {label}")}>
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
            <SidebarFolder on_toggle={toggle_message} on_open={open_message} label={label.clone()}
                accessibility_labels={(format!("Toggle {label}"), format!("Open {label}"))}
                focus_borders={(palette.accent, palette.complement)}
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
            }, palette.text)} icon_size={icon_size} focus_border={palette.accent}
            controller_focus_border={palette.complement} id={format!("file-entry-{index}")}
            context_message={FileMessage::ContextEntry(index)} semantic_role={nickel_ui::SemanticRole::Button}
            accessibility_label={entry.display_name()} />
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

fn rects_intersect(left: Rect, right: Rect) -> bool {
    left.origin.x < right.origin.x + right.size.width
        && left.origin.x + left.size.width > right.origin.x
        && left.origin.y < right.origin.y + right.size.height
        && left.origin.y + left.size.height > right.origin.y
}

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let _log_path = nickel_logging::init("nickel-file").ok();
    let path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(home_directory);
    sdl3::hint::set("SDL_APP_ID", "nickel-file");
    nickel_ui::run_with_adapter(FileApp::new(path), FileHostAdapter::default())
}

#[cfg(test)]
mod ui_layout_tests {
    use super::*;
    use nickel_ui::ActionKind;
    use nickel_ui_testkit::{FocusDirection, Scenario, ScenarioBudget, Selector};

    #[test]
    fn idle_file_host_declares_no_poll_deadline() {
        let mut app = FileApp::new(home_directory());
        app.icon_rx = None;
        assert_eq!(Application::poll_interval(&app), None);
    }

    #[test]
    fn pending_icon_work_uses_bounded_backoff() {
        let mut app = FileApp::new(home_directory());
        let (_sender, receiver) = std::sync::mpsc::channel();
        app.icon_rx = Some(receiver);
        app.icon_poll_delay = std::time::Duration::from_millis(16);
        app.poll_icons();
        assert_eq!(app.icon_poll_delay, std::time::Duration::from_millis(32));
        for _ in 0..8 {
            app.poll_icons();
        }
        assert_eq!(app.icon_poll_delay, std::time::Duration::from_millis(250));
    }

    #[test]
    fn component_owned_fixture_registers_the_production_file_app() {
        use nickel_ui_testkit::FixtureProvider;
        let mut registry = nickel_ui_testkit::FixtureRegistry::new();
        FileFixtureProvider.register(&mut registry).unwrap();
        let entries = registry.finish();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].metadata.id, "file.browser");
        let session = entries[0].open();
        assert!(
            session
                .semantic_nodes()
                .iter()
                .any(|node| node.id.as_str().ends_with("/file-entry-0"))
        );
    }

    #[test]
    fn every_advertised_controller_action_has_a_bounded_production_path() {
        use nickel_ui_testkit::{FixtureProvider, ReachabilityModality, ReachabilityPolicy};

        let mut registry = nickel_ui_testkit::FixtureRegistry::new();
        FileFixtureProvider.register(&mut registry).unwrap();
        let session = registry.finish().remove(0).open();
        let report = session.reachability_report(&ReachabilityPolicy {
            modalities: [ReachabilityModality::Controller].into_iter().collect(),
            ..ReachabilityPolicy::default()
        });

        assert!(report.issues.is_empty(), "{:#?}", report.issues);
        assert!(
            report.paths.iter().all(|path| path.reached),
            "unreached controller paths: {:#?}",
            report
                .paths
                .iter()
                .filter(|path| !path.reached)
                .collect::<Vec<_>>()
        );
    }

    fn pixel(raster: &nickel_ui_testkit::HeadlessRaster, x: u32, y: u32) -> [u8; 4] {
        let offset = ((y * raster.width + x) * 4) as usize;
        raster.rgba[offset..offset + 4].try_into().unwrap()
    }

    fn changed_pixels_in(
        before: &nickel_ui_testkit::HeadlessRaster,
        after: &nickel_ui_testkit::HeadlessRaster,
        rect: Rect,
    ) -> usize {
        let x0 = rect.origin.x.max(0.0) as u32;
        let y0 = rect.origin.y.max(0.0) as u32;
        let x1 = (rect.origin.x + rect.size.width).min(before.width as f32) as u32;
        let y1 = (rect.origin.y + rect.size.height).min(before.height as f32) as u32;
        (y0..y1)
            .flat_map(|y| (x0..x1).map(move |x| (x, y)))
            .filter(|(x, y)| pixel(before, *x, *y) != pixel(after, *x, *y))
            .count()
    }

    #[test]
    fn file_grid_resolves_responsively_without_application_column_arithmetic() {
        let directory = tempfile::tempdir().unwrap();
        for index in 0..18 {
            std::fs::write(directory.path().join(format!("item-{index}.txt")), b"x").unwrap();
        }
        let app = FileApp::new(directory.path().to_path_buf());
        let palette = ThemePalette::from_appearance(
            ShellSettings::load_default().resolve_appearance(nickel_platform::appearance()),
        );
        let narrow = nickel_ui::UiFrame::layout(
            app.build_view(560.0, 420.0, palette, false),
            Rect::new(0.0, 0.0, 560.0, 420.0),
        );
        let wide = nickel_ui::UiFrame::layout(
            app.build_view(1280.0, 720.0, palette, false),
            Rect::new(0.0, 0.0, 1280.0, 720.0),
        );

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
    }

    #[test]
    fn context_menu_is_one_semantic_controller_and_accessibility_surface() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("report.txt"), b"x").unwrap();
        let mut host = UiHost::new(FileApp::new(directory.path().to_path_buf()), 860, 620);
        let entry = host
            .semantic_nodes()
            .into_iter()
            .find(|node| node.id.as_str().ends_with("/file-entry-0"))
            .expect("entry must retain its explicit local identity")
            .id;

        assert!(
            host.inspect().overlay_failures.is_empty(),
            "{:?}",
            host.inspect().overlay_failures
        );
        assert!(
            host.resolve_effective_target(&entry, ActionKind::ContextMenu)
                .is_ok()
        );
        host.handle_event(nickel_ui::UiEvent::AccessibilityContextMenu(entry.clone()));

        let menu_items = host.query(&nickel_ui::SemanticSelector::Role(
            nickel_ui::SemanticRole::MenuItem,
        ));
        assert_eq!(menu_items.len(), 2);
        assert!(menu_items.iter().all(|item| {
            item.actions.contains(&ActionKind::Activate)
                && host
                    .accessibility_nodes()
                    .iter()
                    .any(|node| node.id == item.id)
        }));

        host.handle_event(nickel_ui::UiEvent::ControllerDown);
        assert!(host.inspect().controller_target.is_some());
        host.handle_event(nickel_ui::UiEvent::ControllerBack);
        assert!(host.inspect().open_overlay.is_none());
    }

    #[test]
    fn controller_context_uses_target_geometry_not_stale_pointer_position() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("report.txt"), b"x").unwrap();
        let mut host = UiHost::new(FileApp::new(directory.path().to_path_buf()), 860, 620);
        host.handle_event(nickel_ui::UiEvent::FocusGained);
        host.handle_event(nickel_ui::UiEvent::ControllerDown);
        assert_eq!(
            host.inspect().modality,
            nickel_ui::InputModality::Controller
        );
        let background = host
            .semantic_nodes()
            .into_iter()
            .find(|node| node.id.as_str().ends_with("/file-content"))
            .unwrap()
            .id;
        host.perform_semantic_action(
            background,
            nickel_ui::SemanticAction::Invoke(ActionKind::ContextMenu),
        );

        let menu = host
            .query(&nickel_ui::SemanticSelector::Role(
                nickel_ui::SemanticRole::Menu,
            ))
            .pop()
            .unwrap();
        assert!(menu.bounds.origin.x > 100.0 && menu.bounds.origin.y > 100.0);
        let items = host.query(&nickel_ui::SemanticSelector::Role(
            nickel_ui::SemanticRole::MenuItem,
        ));
        let selected = items
            .iter()
            .find(|item| item.controller_selected)
            .expect("controller menu must visibly select its first action");
        let unselected = items
            .iter()
            .find(|item| !item.controller_selected)
            .expect("menu fixture has a second unselected action");
        let raster = nickel_ui_testkit::render_host(&host, 860, 620, 1.0);
        let first_id = selected.id.clone();
        let second_id = unselected.id.clone();
        let first_point = (
            selected.bounds.origin.x as u32 + 8,
            selected.bounds.origin.y as u32 + 8,
        );
        let second_point = (
            unselected.bounds.origin.x as u32 + 8,
            unselected.bounds.origin.y as u32 + 8,
        );
        let first_selected_color = pixel(&raster, first_point.0, first_point.1);
        let unselected_color = pixel(&raster, second_point.0, second_point.1);
        assert_ne!(
            first_selected_color, unselected_color,
            "controller-selected menu row must have a distinct raster fill"
        );

        host.handle_event(nickel_ui::UiEvent::ControllerDown);
        assert_eq!(
            host.inspect().controller_target,
            Some(second_id.clone()),
            "items after navigation: {:?}",
            host.query(&nickel_ui::SemanticSelector::Role(
                nickel_ui::SemanticRole::MenuItem
            ))
        );
        assert_ne!(host.inspect().controller_target, Some(first_id.clone()));
        let moved_items = host.query(&nickel_ui::SemanticSelector::Role(
            nickel_ui::SemanticRole::MenuItem,
        ));
        assert!(
            moved_items
                .iter()
                .any(|item| item.id == first_id && !item.focused && !item.controller_selected)
        );
        assert!(
            moved_items
                .iter()
                .any(|item| item.id == second_id && item.focused && item.controller_selected)
        );
        let moved = nickel_ui_testkit::render_host(&host, 860, 620, 1.0);
        assert!(
            changed_pixels_in(&raster, &moved, selected.bounds) > 0,
            "the former menu target must visibly lose its selected treatment"
        );
        assert!(
            changed_pixels_in(&raster, &moved, unselected.bounds) > 0,
            "the new controller target must visibly gain selected treatment"
        );
    }

    #[test]
    fn ordinary_controller_target_has_a_distinct_visible_focus_ring() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("report.txt"), b"x").unwrap();
        let mut host = UiHost::new(FileApp::new(directory.path().to_path_buf()), 860, 620);
        host.handle_event(nickel_ui::UiEvent::FocusGained);
        let before = nickel_ui_testkit::render_host(&host, 860, 620, 1.0);
        let selected = (0..8)
            .find_map(|_| {
                host.handle_event(nickel_ui::UiEvent::ControllerDown);
                let target = host.inspect().controller_target?;
                if let Some(node) = host
                    .semantic_nodes()
                    .into_iter()
                    .find(|node| node.id == target)
                {
                    return Some(node);
                }
                host.handle_event(nickel_ui::UiEvent::ControllerActivate);
                None
            })
            .expect("controller must enter a pane and select an ordinary semantic target");
        let after = nickel_ui_testkit::render_host(&host, 860, 620, 1.0);
        assert!(
            changed_pixels_in(&before, &after, selected.bounds) > 0,
            "controller focus must visibly change pixels inside {selected:?}"
        );
    }

    fn entry_selector(scenario: &Scenario<FileApp>, suffix: &str) -> Selector {
        Selector::id(
            scenario
                .semantic_nodes()
                .into_iter()
                .find(|node| node.id.as_str().ends_with(suffix))
                .unwrap_or_else(|| panic!("missing semantic File target {suffix}"))
                .id,
        )
    }

    #[test]
    fn semantic_scenario_covers_context_routes_dismiss_scroll_and_resize() {
        let directory = tempfile::tempdir().unwrap();
        for index in 0..30 {
            std::fs::write(directory.path().join(format!("item-{index:02}.txt")), b"x").unwrap();
        }
        let mut scenario = Scenario::with_budget(
            FileApp::new(directory.path().to_path_buf()),
            960,
            640,
            ScenarioBudget {
                operations: 160,
                frames: 160,
                trace_steps: 160,
            },
        );
        let entry = entry_selector(&scenario, "/file-entry-0");

        scenario
            .accessibility_action(&entry, ActionKind::ContextMenu)
            .unwrap();
        assert!(scenario.host().inspect().open_overlay.is_some());
        scenario
            .controller(nickel_ui::ControllerAction::Cancel)
            .unwrap();
        assert!(scenario.host().inspect().open_overlay.is_none());

        for _ in 0..64 {
            if scenario.host().inspect().keyboard_focus.as_ref()
                == match &entry {
                    Selector::Id(id) => Some(id),
                    _ => None,
                }
            {
                break;
            }
            scenario.keyboard_focus(FocusDirection::Next).unwrap();
        }
        scenario.keyboard_context_focused().unwrap();
        assert!(scenario.host().inspect().open_overlay.is_some());
        scenario
            .controller(nickel_ui::ControllerAction::Cancel)
            .unwrap();

        scenario
            .controller(nickel_ui::ControllerAction::ContextMenu)
            .unwrap();
        assert!(scenario.host().inspect().open_overlay.is_some());
        scenario
            .controller(nickel_ui::ControllerAction::Cancel)
            .unwrap();

        let content = entry_selector(&scenario, "/file-content");
        scenario.pointer_scroll(&content, 0.0, 240.0).unwrap();
        scenario.resize(540, 420, 1.0).unwrap();
        scenario
            .assert_accessibility()
            .unwrap()
            .assert_no_diagnostics()
            .unwrap();
        let narrow_width = scenario
            .semantic_nodes()
            .into_iter()
            .find(|node| node.id.as_str().ends_with("/file-content"))
            .unwrap()
            .bounds
            .size
            .width;
        scenario.resize(1280, 720, 2.0).unwrap();
        let wide_width = scenario
            .semantic_nodes()
            .into_iter()
            .find(|node| node.id.as_str().ends_with("/file-content"))
            .unwrap()
            .bounds
            .size
            .width;
        assert!(wide_width > narrow_width);
        let operations = scenario.operation_trace();
        assert!(operations.iter().any(|step| matches!(
            step.operation,
            nickel_ui_testkit::ScenarioOperation::KeyboardContext
        )));
        assert!(operations.iter().any(|step| matches!(
            step.operation,
            nickel_ui_testkit::ScenarioOperation::Controller { .. }
        )));
        assert!(operations.iter().any(|step| matches!(
            step.operation,
            nickel_ui_testkit::ScenarioOperation::Accessibility { .. }
        )));
    }

    #[test]
    fn semantic_scenarios_cover_empty_and_failure_states() {
        let empty = tempfile::tempdir().unwrap();
        let empty_scenario = Scenario::new(FileApp::new(empty.path().to_path_buf()), 720, 480);
        assert!(
            empty_scenario
                .host()
                .accessibility_nodes()
                .iter()
                .any(|node| { node.label.as_deref() == Some("This folder is empty.") })
        );
        empty_scenario.assert_accessibility().unwrap();

        let missing = empty.path().join("does-not-exist");
        let failed = Scenario::new(FileApp::new(missing.clone()), 720, 480);
        let status = format!("Could not open {}:", missing.display());
        assert!(failed.host().accessibility_nodes().iter().any(|node| {
            node.label
                .as_deref()
                .is_some_and(|name| name.starts_with(&status))
        }));
        failed.assert_accessibility().unwrap();
    }

    #[test]
    fn scenario_resource_lifecycle_releases_build_scratch_and_suspend_state() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("report.txt"), b"x").unwrap();
        let mut scenario = Scenario::new(FileApp::new(directory.path().to_path_buf()), 860, 620);
        let before = scenario.host().inspect().resources;
        assert_eq!(before.retained_build_scratch_bytes, 0);
        assert!(before.node_count > 0 && before.accessibility_node_count > 0);
        scenario.window_focus(false).unwrap();
        scenario.platform_capability("controller", false).unwrap();
        scenario.suspend().unwrap();
        let after = scenario.host().inspect();
        assert!(after.keyboard_focus.is_none());
        assert!(after.controller_target.is_none());
        assert!(after.pointer_capture.is_none());
        assert_eq!(after.resources.retained_build_scratch_bytes, 0);
    }
}
