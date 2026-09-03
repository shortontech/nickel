use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{
        Arc,
        mpsc::{self, Receiver, TryRecvError},
    },
    time::{Duration, Instant},
};

use crate::{
    host::FileHostAdapter,
    icons, layout,
    layout::{rect_between, visible_file_range},
    platform::{home_directory, open_path},
};
use nickel_core::{
    shell_settings::{FileIconPreference, ShellSettings},
    theme::{ThemeMode, ThemePalette},
};
use nickel_file::{DirectoryBrowser, FileEntry};
use nickel_ui::{
    AnyView, Application, FrameOverlay, Insets, OverlayAnchor, OverlayMenu, OverlayMenuItem, Point,
    UiId, ViewContext,
};

const DEFAULT_SIDEBAR_WIDTH: f32 = 190.0;
pub(crate) const MIN_SIDEBAR_WIDTH: f32 = 150.0;
pub(crate) const MAX_SIDEBAR_WIDTH: f32 = 360.0;
pub(crate) const SIDEBAR_RESIZE_WIDTH: f32 = 5.0;
pub(crate) const TOOLBAR_HEIGHT: f32 = 78.0;
const DEFAULT_TILE_WIDTH: f32 = 150.0;
pub(crate) const NARROW_WORKSPACE_BREAKPOINT: f32 = 720.0;
pub(crate) const MIN_TILE_WIDTH: f32 = 110.0;
pub(crate) const MAX_TILE_WIDTH: f32 = 240.0;

#[derive(Clone, Debug, PartialEq)]
pub enum FileMessage {
    ContextEntry(usize),
    ContextBackground,
    ContextOpen,
    ContextOpenNewTab,
    ContextRefresh,
    ContextSelectAll,
    ResizeSidebar,
    TogglePlaces,
    NewTab,
    SwitchTab(usize),
    CloseTab(usize),
    Back,
    Forward,
    Up,
    Refresh,
    SetViewMode(FileViewMode),
    Breadcrumb(PathBuf),
    ToggleFolder(PathBuf),
    OpenFolder(PathBuf),
    Entry(usize),
    SelectionSurface,
    FileScroll(f32),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileViewMode {
    Grid,
    Details,
}

pub struct FileApp {
    pub(crate) browser: DirectoryBrowser,
    pub(crate) cursor: Point,
    pub(crate) selected: Option<usize>,
    pub(crate) selected_entries: HashSet<usize>,
    pub(crate) selection_anchor: Option<usize>,
    pub(crate) active_tab_id: u64,
    pub(crate) next_tab_id: u64,
    pub(crate) status: String,
    pub(crate) last_click: Option<(usize, Instant)>,
    pub(crate) icons: icons::ArtworkCache,
    pub(crate) icon_rx: Option<Receiver<(u64, PathBuf, icons::ResolvedArtwork)>>,
    pub(crate) icon_poll_delay: std::time::Duration,
    pub(crate) icon_generation: u64,
    pub(crate) icon_preference: FileIconPreference,
    pub(crate) next_icon_id: u16,
    pub(crate) sidebar_width: f32,
    pub(crate) expanded_folders: HashSet<PathBuf>,
    pub(crate) control_down: bool,
    pub(crate) shift_down: bool,
    pub(crate) selection_drag: Option<Point>,
    pub(crate) resizing_sidebar: bool,
    pub(crate) places_open: bool,
    pub(crate) tile_width: f32,
    pub(crate) file_scroll_offset: f32,
    pub(crate) view_mode: FileViewMode,
    pub(crate) tab_icon: Option<(u16, Arc<image::RgbaImage>)>,
    pub(crate) tabs: Vec<Option<FileTab>>,
    pub(crate) active_tab: usize,
    pub(crate) pending_ensure_visible: bool,
    pub(crate) resolved_grid_columns: usize,
    pub(crate) exit_requested: bool,
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

pub(crate) struct FileTab {
    pub(crate) browser: DirectoryBrowser,
    pub(crate) selected: Option<usize>,
    pub(crate) selected_entries: HashSet<usize>,
    pub(crate) selection_anchor: Option<usize>,
    pub(crate) tab_id: u64,
    pub(crate) status: String,
    pub(crate) last_click: Option<(usize, Instant)>,
    pub(crate) file_scroll_offset: f32,
    pub(crate) view_mode: FileViewMode,
    pub(crate) tab_icon: Option<(u16, Arc<image::RgbaImage>)>,
}

impl FileApp {
    pub(crate) fn is_resizing_sidebar(&self) -> bool {
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
            icons: icons::ArtworkCache::default(),
            icon_rx: None,
            icon_poll_delay: std::time::Duration::from_millis(16),
            icon_generation: 0,
            icon_preference: ShellSettings::load_default().file_icon_provider,
            next_icon_id: 1,
            sidebar_width: DEFAULT_SIDEBAR_WIDTH,
            expanded_folders: HashSet::new(),
            control_down: false,
            shift_down: false,
            selection_drag: None,
            resizing_sidebar: false,
            places_open: false,
            tile_width: DEFAULT_TILE_WIDTH,
            file_scroll_offset: 0.0,
            view_mode: FileViewMode::Grid,
            tab_icon: None,
            tabs: vec![None],
            active_tab: 0,
            pending_ensure_visible: false,
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
        self.file_scroll_offset = offset.max(0.0);
    }

    pub(crate) fn inactive_tab(&self, index: usize) -> Option<&FileTab> {
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
        std::mem::swap(&mut self.file_scroll_offset, &mut target.file_scroll_offset);
        std::mem::swap(&mut self.view_mode, &mut target.view_mode);
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
            file_scroll_offset: 0.0,
            view_mode: FileViewMode::Grid,
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
        layout::build_view(self, _width, height, palette, light_mode)
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

    pub(crate) fn go_back(&mut self) {
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

    pub(crate) fn refresh_icons(&mut self) {
        let preference = ShellSettings::load_default().file_icon_provider;
        let appearance = ShellSettings::load_default()
            .resolve_appearance(nickel_platform::appearance())
            .mode;
        if preference != self.icon_preference {
            self.icon_preference = preference;
            self.icons.clear();
            self.tab_icon = None;
        }
        self.icon_generation = self.icon_generation.wrapping_add(1);
        let generation = self.icon_generation;
        let entries = self
            .browser
            .entries()
            .iter()
            .map(|entry| (entry.path.clone(), entry.is_directory))
            .collect::<Vec<_>>();
        self.icons
            .retain(|path| entries.iter().any(|(entry, _)| entry == path));
        let mut paths = entries
            .into_iter()
            .filter(|(path, _)| !self.icons.contains_key(path))
            .collect::<Vec<_>>();
        paths.push((self.browser.current().to_path_buf(), true));
        #[cfg(debug_assertions)]
        if std::env::var_os("NICKEL_FILE_PROFILE_ICONS").is_some() {
            eprintln!(
                "nickel-file icon-profile: fetch batch requested={} retained_cache={}",
                paths.len(),
                self.icons.len()
            );
        }
        let (tx, rx) = mpsc::channel();
        self.icon_rx = Some(rx);
        self.icon_poll_delay = std::time::Duration::from_millis(16);
        let _ = std::thread::Builder::new()
            .name("nickel-file-icons".into())
            .spawn(move || {
                for (path, is_directory) in paths {
                    #[cfg(debug_assertions)]
                    let profile_started = Instant::now();
                    let artwork = icons::resolve_artwork(
                        preference,
                        &icons::ArtworkRequest {
                            path: &path,
                            kind: icons::semantic_kind(&path, is_directory),
                            logical_size: 96,
                            scale_milli: 1_000,
                            appearance: if appearance == ThemeMode::Light {
                                icons::ArtworkAppearance::Light
                            } else {
                                icons::ArtworkAppearance::Dark
                            },
                        },
                    );
                    #[cfg(debug_assertions)]
                    if std::env::var_os("NICKEL_FILE_PROFILE_ICONS").is_some() {
                        let dimensions =
                            format!("{}x{}", artwork.pixels.width(), artwork.pixels.height());
                        eprintln!(
                            "nickel-file icon-profile: fetched path={} result={} time={:.2?}",
                            path.display(),
                            dimensions,
                            profile_started.elapsed()
                        );
                    }
                    if tx.send((generation, path, artwork)).is_err() {
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
                Ok((generation, path, artwork)) if generation == self.icon_generation => {
                    self.icon_poll_delay = std::time::Duration::from_millis(16);
                    let id = self.next_icon_id;
                    self.next_icon_id = self.next_icon_id.checked_add(1).unwrap_or(1);
                    if path == self.browser.current() {
                        self.tab_icon = Some((id, artwork.pixels));
                    } else if self
                        .browser
                        .entries()
                        .iter()
                        .any(|entry| entry.path == path)
                    {
                        self.icons.insert(path, (id, artwork.pixels));
                    }
                }
                Ok(_) => {}
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

    pub(crate) fn select_relative(&mut self, delta: isize) {
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

    pub(crate) fn ensure_selection_visible(&mut self) {
        self.pending_ensure_visible = self.selected.is_some();
    }

    pub(crate) fn resolved_grid_columns(&self) -> usize {
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
            FileMessage::TogglePlaces => self.places_open = !self.places_open,
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
            FileMessage::SetViewMode(mode) => {
                self.view_mode = mode;
                self.set_scroll_offset(0.0);
                self.ensure_selection_visible();
            }
            FileMessage::CloseTab(index) => self.close_tab(index),
            FileMessage::SwitchTab(index) => self.switch_tab(index),
            FileMessage::ToggleFolder(path) => {
                if !self.expanded_folders.remove(&path) {
                    self.expanded_folders.insert(path);
                }
            }
            FileMessage::OpenFolder(path) | FileMessage::Breadcrumb(path) => {
                self.places_open = false;
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
            FileMessage::FileScroll(offset) => self.file_scroll_offset = offset.max(0.0),
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
        let visible_entries = visible_file_range(
            self,
            context.viewport.size.width,
            context.viewport.size.height,
        );
        let mut overlays = self
            .browser
            .entries()
            .iter()
            .enumerate()
            .filter(|(index, _)| visible_entries.contains(index))
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

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let _log_path = nickel_logging::init("nickel-file").ok();
    let path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(home_directory);
    nickel_ui::run_with_adapter(FileApp::new(path), FileHostAdapter::default())
}

#[cfg(test)]
#[path = "ui_layout_tests.rs"]
mod ui_layout_tests;
