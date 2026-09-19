//! Runtime-independent launcher scene and hit testing.
//!
//! `LiveShell` owns input and mutates [`Launcher`]. This module turns that state
//! into component paint commands and stable semantic actions.

use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    path::PathBuf,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
};

use crate::{
    icons,
    launcher::{Application, Launcher, LauncherMode, LauncherView, SettingsDestination},
    platform,
};
use image::RgbaImage;
use nickel_core::theme::ThemePalette;
use nickel_i18n::{ActionLabel, Localizer};
use nickel_ui::{
    AccountSummaryRow, ActionLegend, ActionLegendActions, ActionLegendEntry, AnyView,
    Application as UiApplication, Column, ComponentBuilderExt, Container, ControllerFamily,
    FallbackAvatar, FileGrid, FilePlaneItem, FrameOverlay, Image, InputModality, Insets,
    LauncherSearchField, OverlayAnchor, OverlayMenu, OverlayMenuItem, OverlayStyle,
    ReadingDirection, START_MENU_SINGLE_PANE_BREAKPOINT, SectionHeader, SemanticControllerAction,
    SemanticTheme, Shortcut, ShortcutRow, ShortcutState, StartMenuNarrowPane, StartMenuShell, Text,
    UiId, VerticalScroll, ViewContext,
};

const PANEL_MAX_WIDTH: f32 = 800.0;
const DASHBOARD_MAX_WIDTH: f32 = 720.0;
const SIDEBAR_WIDTH: f32 = 148.0;
const GRID_GAP: f32 = 10.0;
const TILE_MIN_WIDTH: f32 = 142.0;
const TILE_HEIGHT: f32 = 108.0;
const ICON_SIZE: f32 = 48.0;

fn search_result_anchor(id: &str) -> UiId {
    UiId::new(format!("launcher-search-results/{id}"))
}

fn dashboard_application_anchor(id: &str) -> UiId {
    UiId::new(format!("launcher-applications/{id}"))
}

fn dashboard_applications(launcher: &Launcher) -> Vec<&Application> {
    let mut seen = std::collections::HashSet::new();
    match launcher.view() {
        LauncherView::Favorites => {
            let home = launcher
                .favorite_applications()
                .into_iter()
                .chain(launcher.recent_applications())
                .filter(|application| seen.insert(application.id().to_owned()))
                .take(12)
                .collect::<Vec<_>>();
            if home.is_empty() {
                (0..launcher.result_count())
                    .filter_map(|index| launcher.result_at(index))
                    .take(12)
                    .collect()
            } else {
                home
            }
        }
        LauncherView::Applications | LauncherView::Places => (0..launcher.result_count())
            .filter_map(|index| launcher.result_at(index))
            .collect(),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LauncherAction {
    SetView(LauncherView),
    ActivateResult(usize),
    TogglePin(String),
    RetryPreferencePersistence,
    LaunchApplication(String),
    OpenProject(String),
    SeeAllProjects,
    OpenSettings(SettingsDestination),
    OpenAccount,
    RequestLogout,
    ShowNarrowPrimary,
    SetQuery(String),
    SearchScroll,
    DashboardScroll,
    Dismiss,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LauncherShellEffect {
    ActivateResult(usize),
    TogglePin(String),
    RetryPreferencePersistence,
    LaunchApplication(String),
    OpenProject(String),
    SeeAllProjects,
    OpenSettings(SettingsDestination),
    OpenAccount,
    RequestLogout,
    Dismiss,
}

pub struct LauncherApplication {
    launcher: Launcher,
    state: RefCell<LauncherViewState>,
    icons: RefCell<LauncherIconCache>,
    palette: ThemePalette,
    status: Option<String>,
    effects: Vec<LauncherAction>,
    dirty: bool,
    reading_direction: Option<ReadingDirection>,
    icon_revision: u64,
}

impl LauncherApplication {
    pub fn new(
        launcher: Launcher,
        state: LauncherViewState,
        icons: LauncherIconCache,
        palette: ThemePalette,
    ) -> Self {
        let icon_revision = icons.revision();
        Self {
            launcher,
            state: RefCell::new(state),
            icons: RefCell::new(icons),
            palette,
            status: None,
            effects: Vec::new(),
            dirty: false,
            reading_direction: None,
            icon_revision,
        }
    }

    pub fn take_effects(&mut self) -> Vec<LauncherAction> {
        std::mem::take(&mut self.effects)
    }

    pub(crate) fn preferred_surface_size(&self, maximum: (u32, u32)) -> (u32, u32) {
        if self.launcher.mode() != LauncherMode::Dashboard {
            return maximum;
        }
        let applications = dashboard_applications(&self.launcher);
        let geometry = dashboard_geometry(
            &self.launcher,
            &applications,
            &launcher_semantic_theme(self.palette),
            maximum,
        );
        (geometry.width.ceil() as u32, geometry.height.ceil() as u32)
    }

    pub fn sync(&mut self, launcher: &Launcher, palette: ThemePalette, status: Option<String>) {
        self.launcher = launcher.clone();
        self.palette = palette;
        self.status = status;
        self.dirty = true;
    }

    pub fn set_controller_family(&mut self, family: ControllerFamily) {
        self.state.borrow_mut().set_controller_family(family);
        self.dirty = true;
    }

    /// Overrides locale-derived direction for deterministic fixture rendering.
    #[allow(dead_code)] // The binary also compiles this module without fixture support.
    pub fn set_reading_direction(&mut self, direction: ReadingDirection) {
        self.reading_direction = Some(direction);
        self.dirty = true;
    }
}

impl UiApplication for LauncherApplication {
    type Message = LauncherAction;

    fn update(&mut self, message: Self::Message) {
        let evidence = message.clone();
        let _ = reduce_launcher_action(&mut self.launcher, &mut self.state.borrow_mut(), message);
        self.effects.push(evidence);
    }

    fn view(&self, context: ViewContext) -> impl nickel_ui::View<Self::Message> {
        let base = build_launcher_view_directional(
            &self.launcher,
            &self.state.borrow(),
            &mut self.icons.borrow_mut(),
            LauncherViewContext {
                viewport: (
                    context.viewport.size.width.max(1.0) as u32,
                    context.viewport.size.height.max(1.0) as u32,
                ),
                palette: self.palette,
                modality: context.modality,
                status: self.status.as_deref(),
                legend_actions: ActionLegendActions::from_view_context(&context),
            },
            self.reading_direction
                .unwrap_or_else(launcher_reading_direction),
        );
        let width = context.viewport.size.width;
        let height = context.viewport.size.height;
        AnyView::new(
            Container::new()
                .width(width)
                .height(height)
                .align_items(nickel_ui::Align::Start)
                .child(base),
        )
    }

    fn frame_overlays(&self, context: ViewContext) -> Vec<FrameOverlay<Self::Message>> {
        let theme = launcher_semantic_theme(self.palette);
        let style = OverlayStyle::from_theme(&theme);
        let direction = launcher_reading_direction();
        let dashboard_detail_visible = context.viewport.size.width
            >= START_MENU_SINGLE_PANE_BREAKPOINT
            || self.state.borrow().dashboard_narrow_page != DashboardNarrowPage::Primary;
        let applications = if self.launcher.mode() == LauncherMode::Search {
            (0..self.launcher.result_count())
                .filter_map(|index| {
                    self.launcher
                        .result_at(index)
                        .map(|application| (index, application))
                })
                .collect::<Vec<_>>()
        } else if dashboard_detail_visible {
            dashboard_applications(&self.launcher)
                .into_iter()
                .map(|application| (0, application))
                .collect()
        } else {
            Vec::new()
        };
        let mut overlays = applications
            .into_iter()
            .map(|(index, application)| {
                let id = application.id().to_owned();
                let (anchor, launch) = if self.launcher.mode() == LauncherMode::Search {
                    (
                        search_result_anchor(&id),
                        LauncherAction::ActivateResult(index),
                    )
                } else {
                    (
                        dashboard_application_anchor(&id),
                        LauncherAction::LaunchApplication(id.clone()),
                    )
                };
                let mut menu = OverlayMenu::new(
                    format!("application-menu-{id}"),
                    OverlayAnchor::InvocationTarget(anchor),
                )
                .semantic_style(style)
                .direction(direction)
                .item(OverlayMenuItem::action("launch", "Launch", launch))
                .item(OverlayMenuItem::action(
                    "toggle-pin",
                    if self.launcher.is_pinned(&id) {
                        "Unpin from Nickel Bar"
                    } else {
                        "Pin to Nickel Bar"
                    },
                    LauncherAction::TogglePin(id.clone()),
                ));
                menu.border_width = 0.0;
                if self.status.as_deref().is_some_and(|status| {
                    status.starts_with("Launcher preferences could not be saved:")
                }) {
                    menu = menu.item(OverlayMenuItem::action(
                        "retry-preferences",
                        "Retry saving favorites",
                        LauncherAction::RetryPreferencePersistence,
                    ));
                }
                FrameOverlay::Menu(menu)
            })
            .collect::<Vec<_>>();
        if self.launcher.mode() == LauncherMode::Dashboard && self.launcher.logout_available() {
            overlays.push(FrameOverlay::Menu(
                OverlayMenu::new(
                    "session-actions-menu",
                    OverlayAnchor::InvocationTarget(UiId::new("launcher-account")),
                )
                .semantic_style(style)
                .direction(direction)
                .item(OverlayMenuItem::action(
                    "logout",
                    "Log out",
                    LauncherAction::RequestLogout,
                )),
            ));
        }
        overlays
    }

    fn poll(&mut self) -> bool {
        let revision = self.icons.borrow().revision();
        let icons_changed = revision != self.icon_revision;
        self.icon_revision = revision;
        std::mem::take(&mut self.dirty) || icons_changed
    }

    fn shortcut_outcome(&mut self, shortcut: Shortcut) -> nickel_ui::ShortcutOutcome {
        nickel_ui::ShortcutOutcome::from_changed(match shortcut {
            Shortcut::Submit if self.launcher.mode() == LauncherMode::Search => {
                if self.launcher.result_count() == 0 {
                    return nickel_ui::ShortcutOutcome::from_changed(false);
                }
                self.effects.push(LauncherAction::ActivateResult(
                    self.launcher.selected_index(),
                ));
                true
            }
            Shortcut::Escape => {
                if self.launcher.query().is_empty() {
                    self.effects.push(LauncherAction::Dismiss);
                } else {
                    self.update(LauncherAction::SetQuery(String::new()));
                }
                true
            }
            _ => false,
        })
    }
}

fn launcher_semantic_theme(palette: ThemePalette) -> SemanticTheme {
    SemanticTheme::from_tokens(nickel_ui::SemanticTokenSet::standard(
        palette.background,
        palette.panel,
        palette.surface,
        palette.surface_hover,
        palette.surface_hover,
        palette.text,
        palette.muted,
        palette.accent,
        palette.accent_soft,
        palette.complement,
        palette.complement,
    ))
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DashboardNarrowPage {
    #[default]
    Primary,
    Projects,
}

#[derive(Clone, Debug, Default)]
pub struct LauncherViewState {
    pub dashboard_selected: usize,
    pub dashboard_narrow_page: DashboardNarrowPage,
    controller_family: ControllerFamily,
}

impl LauncherViewState {
    pub fn set_controller_family(&mut self, family: ControllerFamily) {
        self.controller_family = family;
    }
}

pub fn reduce_launcher_action(
    launcher: &mut Launcher,
    view: &mut LauncherViewState,
    action: LauncherAction,
) -> Option<LauncherShellEffect> {
    match action {
        LauncherAction::SetView(next) => {
            launcher.set_view(next);
            view.dashboard_narrow_page = DashboardNarrowPage::Projects;
            None
        }
        LauncherAction::ActivateResult(index) => Some(LauncherShellEffect::ActivateResult(index)),
        LauncherAction::TogglePin(id) => Some(LauncherShellEffect::TogglePin(id)),
        LauncherAction::RetryPreferencePersistence => {
            Some(LauncherShellEffect::RetryPreferencePersistence)
        }
        LauncherAction::LaunchApplication(id) => Some(LauncherShellEffect::LaunchApplication(id)),
        LauncherAction::OpenProject(id) => Some(LauncherShellEffect::OpenProject(id)),
        LauncherAction::SeeAllProjects => Some(LauncherShellEffect::SeeAllProjects),
        LauncherAction::OpenSettings(destination) => {
            Some(LauncherShellEffect::OpenSettings(destination))
        }
        LauncherAction::OpenAccount => Some(LauncherShellEffect::OpenAccount),
        LauncherAction::RequestLogout => Some(LauncherShellEffect::RequestLogout),
        LauncherAction::ShowNarrowPrimary => {
            view.dashboard_narrow_page = DashboardNarrowPage::Primary;
            view.dashboard_selected = 0;
            None
        }
        LauncherAction::SetQuery(query) => {
            launcher.set_query(&query);
            None
        }
        LauncherAction::SearchScroll | LauncherAction::DashboardScroll => None,
        LauncherAction::Dismiss => Some(LauncherShellEffect::Dismiss),
    }
}

/// Bounded CPU image cache with stable renderer resource IDs.
#[derive(Clone)]
pub struct LauncherIconCache {
    shared: Arc<LauncherIconCacheShared>,
}

struct LauncherIconCacheShared {
    state: Mutex<LauncherIconCacheState>,
    requests: mpsc::Sender<LauncherIconRequest>,
    revision: AtomicU64,
}

struct LauncherIconCacheState {
    icons: HashMap<String, CachedIcon>,
    insertion_order: VecDeque<String>,
    next_id: u16,
    evictions: u64,
    generation: u64,
}

const LAUNCHER_ICON_CACHE_CAPACITY: usize = 512;
const LAUNCHER_ICON_MAX_SIDE: u32 = 96;
const LAUNCHER_ICON_CACHE_MAX_BYTES: usize = LAUNCHER_ICON_CACHE_CAPACITY
    * LAUNCHER_ICON_MAX_SIDE as usize
    * LAUNCHER_ICON_MAX_SIDE as usize
    * 4;

#[derive(Clone)]
struct CachedIcon {
    id: u16,
    image: Option<Arc<RgbaImage>>,
    pending: bool,
}

struct LauncherIconRequest {
    key: String,
    id: u16,
    generation: u64,
    icon_path: Option<PathBuf>,
    icon_reference: Option<String>,
}

impl LauncherIconCache {
    pub fn new() -> Self {
        let (requests, receiver) = mpsc::channel();
        let shared = Arc::new(LauncherIconCacheShared {
            state: Mutex::new(LauncherIconCacheState {
                icons: HashMap::new(),
                insertion_order: VecDeque::new(),
                // Keep launcher IDs away from wallpaper and panel fixed IDs.
                next_id: 0x4000,
                evictions: 0,
                generation: 0,
            }),
            requests,
            revision: AtomicU64::new(0),
        });
        let worker_shared = Arc::downgrade(&shared);
        std::thread::Builder::new()
            .name("nickel-launcher-icons".into())
            .spawn(move || launcher_icon_worker(worker_shared, receiver))
            .expect("launcher icon worker must start");
        Self { shared }
    }

    pub fn diagnostics(&self) -> LauncherIconCacheDiagnostics {
        let state = self.shared.state.lock().expect("launcher icon cache lock");
        LauncherIconCacheDiagnostics {
            entries: state.icons.len(),
            capacity: LAUNCHER_ICON_CACHE_CAPACITY,
            retained_pixel_bytes: state
                .icons
                .values()
                .filter_map(|cached| cached.image.as_ref())
                .map(|image| image.as_raw().len())
                .sum(),
            byte_capacity: LAUNCHER_ICON_CACHE_MAX_BYTES,
            evictions: state.evictions,
        }
    }

    fn revision(&self) -> u64 {
        self.shared.revision.load(Ordering::Acquire)
    }

    pub fn begin_visual_generation(&mut self) {
        let mut state = self.shared.state.lock().expect("launcher icon cache lock");
        state.icons.retain(|key, _| !key.starts_with("structural:"));
        state
            .insertion_order
            .retain(|key| !key.starts_with("structural:"));
    }

    pub(crate) fn invalidate_application_inventory(&mut self) {
        let mut state = self.shared.state.lock().expect("launcher icon cache lock");
        state.generation = state.generation.wrapping_add(1);
        state.icons.clear();
        state.insertion_order.clear();
    }

    pub(crate) fn resolve(&mut self, application: &Application) -> Option<(u16, Arc<RgbaImage>)> {
        let mut state = self.shared.state.lock().expect("launcher icon cache lock");
        if let Some(cached) = state.icons.get(application.id()) {
            return cached
                .image
                .as_ref()
                .map(|image| (cached.id, Arc::clone(image)));
        }
        let id = state.next_id;
        state.next_id = state.next_id.checked_add(1).unwrap_or(0x4000);
        let key = application.id().to_owned();
        let generation = state.generation;
        insert_launcher_icon(
            &mut state,
            key.clone(),
            CachedIcon {
                id,
                image: None,
                pending: true,
            },
        );
        drop(state);
        if self
            .shared
            .requests
            .send(LauncherIconRequest {
                key: key.clone(),
                id,
                generation,
                icon_path: application.icon_path().map(PathBuf::from),
                icon_reference: application.icon().map(str::to_owned),
            })
            .is_err()
        {
            let mut state = self.shared.state.lock().expect("launcher icon cache lock");
            if let Some(cached) = state.icons.get_mut(&key) {
                cached.pending = false;
            }
        }
        None
    }

    pub(crate) fn resolve_window_icon(
        &mut self,
        window: crate::model::WindowId,
        image: Arc<RgbaImage>,
    ) -> (u16, Arc<RgbaImage>) {
        let key = format!("window:{}", window.0);
        let mut state = self.shared.state.lock().expect("launcher icon cache lock");
        if let Some(cached) = state.icons.get(&key)
            && cached
                .image
                .as_ref()
                .is_some_and(|cached_image| cached_image.as_raw() == image.as_raw())
        {
            return (cached.id, Arc::clone(cached.image.as_ref().unwrap()));
        }
        let id = state.next_id;
        state.next_id = state.next_id.checked_add(1).unwrap_or(0x4000);
        insert_launcher_icon(
            &mut state,
            key,
            CachedIcon {
                id,
                image: Some(Arc::clone(&image)),
                pending: false,
            },
        );
        (id, image)
    }

    fn structural(
        &mut self,
        name: &str,
        bytes: &[u8],
        color: u32,
    ) -> Option<(u16, Arc<RgbaImage>)> {
        let key = format!("structural:{name}:{color:06x}");
        let mut state = self.shared.state.lock().expect("launcher icon cache lock");
        if let Some(cached) = state.icons.get(&key) {
            return cached
                .image
                .as_ref()
                .map(|image| (cached.id, Arc::clone(image)));
        }
        let image = icons::load_svg_bytes(bytes, 48).map(|mut image| {
            let red = ((color >> 16) & 0xff) as u8;
            let green = ((color >> 8) & 0xff) as u8;
            let blue = (color & 0xff) as u8;
            for pixel in image.pixels_mut() {
                if pixel[3] != 0 {
                    pixel[0] = red;
                    pixel[1] = green;
                    pixel[2] = blue;
                }
            }
            Arc::new(image)
        });
        let id = state.next_id;
        state.next_id = state.next_id.checked_add(1).unwrap_or(0x4000);
        insert_launcher_icon(
            &mut state,
            key,
            CachedIcon {
                id,
                image: image.clone(),
                pending: false,
            },
        );
        image.map(|image| (id, image))
    }
}

fn insert_launcher_icon(state: &mut LauncherIconCacheState, key: String, cached: CachedIcon) {
    while state.icons.len() >= LAUNCHER_ICON_CACHE_CAPACITY {
        let Some(oldest) = state.insertion_order.pop_front() else {
            break;
        };
        if state.icons.remove(&oldest).is_some() {
            state.evictions = state.evictions.saturating_add(1);
        }
    }
    state.insertion_order.push_back(key.clone());
    state.icons.insert(key, cached);
}

fn launcher_icon_worker(
    shared: std::sync::Weak<LauncherIconCacheShared>,
    receiver: mpsc::Receiver<LauncherIconRequest>,
) {
    while let Ok(request) = receiver.recv() {
        let image = if request.key.starts_with("place:") {
            request
                .icon_reference
                .as_deref()
                .map(|path| place_icon(std::path::Path::new(path)))
        } else {
            request
                .icon_path
                .as_deref()
                .and_then(icons::load)
                .or_else(|| {
                    request
                        .icon_reference
                        .as_deref()
                        .and_then(platform::application_icon)
                })
        }
        .filter(has_visible_pixel)
        .map(normalize_launcher_icon)
        .map(Arc::new);
        let Some(shared) = shared.upgrade() else {
            return;
        };
        let mut state = shared.state.lock().expect("launcher icon cache lock");
        if state.generation != request.generation {
            continue;
        }
        let Some(cached) = state.icons.get_mut(&request.key) else {
            continue;
        };
        if cached.id != request.id || !cached.pending {
            continue;
        }
        cached.image = image;
        cached.pending = false;
        drop(state);
        shared.revision.fetch_add(1, Ordering::Release);
    }
}

fn place_icon(path: &std::path::Path) -> RgbaImage {
    use nickel_file::icons::{ArtworkAppearance, ArtworkRequest, SemanticIconKind};

    let settings = nickel_core::shell_settings::ShellSettings::load_default();
    let is_home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .is_some_and(|home| path == std::path::Path::new(&home));
    let kind = if is_home {
        SemanticIconKind::HomeFolder
    } else {
        nickel_file::icons::semantic_kind(path, true)
    };
    let request = ArtworkRequest {
        path,
        kind,
        logical_size: 96,
        scale_milli: 1000,
        appearance: ArtworkAppearance::Dark,
    };
    if is_home {
        return nickel_file::icons::resolve_artwork(
            nickel_core::shell_settings::FileIconPreference::Nickel,
            &request,
        )
        .pixels
        .as_ref()
        .clone();
    }
    nickel_platform::path_icon_with_theme_at_size(path, settings.file_icon_theme.as_deref(), 96)
        .unwrap_or_else(|| {
            nickel_file::icons::resolve_artwork(
                nickel_core::shell_settings::FileIconPreference::Nickel,
                &request,
            )
            .pixels
            .as_ref()
            .clone()
        })
}

fn normalize_launcher_icon(image: RgbaImage) -> RgbaImage {
    let bounds = image
        .enumerate_pixels()
        .filter(|(_, _, pixel)| pixel[3] != 0)
        .fold(None::<(u32, u32, u32, u32)>, |bounds, (x, y, _)| {
            Some(match bounds {
                None => (x, y, x, y),
                Some((left, top, right, bottom)) => {
                    (left.min(x), top.min(y), right.max(x), bottom.max(y))
                }
            })
        });
    let image = if let Some((left, top, right, bottom)) = bounds {
        image::imageops::crop_imm(&image, left, top, right - left + 1, bottom - top + 1).to_image()
    } else {
        image
    };
    if image.width() > LAUNCHER_ICON_MAX_SIDE || image.height() > LAUNCHER_ICON_MAX_SIDE {
        icons::resized(&image, LAUNCHER_ICON_MAX_SIDE, LAUNCHER_ICON_MAX_SIDE)
    } else {
        image
    }
}

impl Default for LauncherIconCache {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LauncherIconCacheDiagnostics {
    pub entries: usize,
    pub capacity: usize,
    pub retained_pixel_bytes: usize,
    pub byte_capacity: usize,
    pub evictions: u64,
}

#[derive(Clone)]
struct LauncherViewContext<'a> {
    viewport: (u32, u32),
    palette: ThemePalette,
    modality: InputModality,
    status: Option<&'a str>,
    legend_actions: ActionLegendActions,
}

#[allow(dead_code)] // Directly exercised by focused module tests.
fn build_launcher_view(
    launcher: &Launcher,
    state: &LauncherViewState,
    icons: &mut LauncherIconCache,
    context: LauncherViewContext<'_>,
) -> AnyView<LauncherAction> {
    build_launcher_view_directional(
        launcher,
        state,
        icons,
        context,
        launcher_reading_direction(),
    )
}

fn build_launcher_view_directional(
    launcher: &Launcher,
    state: &LauncherViewState,
    icons: &mut LauncherIconCache,
    context: LauncherViewContext<'_>,
    direction: ReadingDirection,
) -> AnyView<LauncherAction> {
    let (viewport_width, _viewport_height) = context.viewport;
    if launcher.mode() == LauncherMode::Dashboard {
        return build_dashboard_view_directional(launcher, state, icons, context, direction);
    }
    let panel_width = PANEL_MAX_WIDTH.min(viewport_width.max(1) as f32).max(320.0);
    let content_width = (panel_width - SIDEBAR_WIDTH - 34.0).max(TILE_MIN_WIDTH);
    let columns = ((content_width + GRID_GAP) / (TILE_MIN_WIDTH + GRID_GAP))
        .floor()
        .max(1.0) as usize;

    let theme = SemanticTheme::from_tokens(nickel_ui::SemanticTokenSet::standard(
        context.palette.background,
        context.palette.panel,
        context.palette.surface,
        context.palette.surface_hover,
        context.palette.surface_hover,
        context.palette.text,
        context.palette.muted,
        context.palette.accent,
        context.palette.accent_soft,
        context.palette.complement,
        context.palette.complement,
    ));
    let sidebar = Column::new()
        .gap(theme.spacing.compact)
        .padding(Insets::all(theme.spacing.control))
        .child(ShortcutRow::new_directional(
            theme,
            Text::new("⌂"),
            "Favorites",
            "Pinned and recent applications",
            Some(LauncherAction::SetView(LauncherView::Favorites)),
            ShortcutState {
                selected: launcher.view() == LauncherView::Favorites,
                ..ShortcutState::default()
            },
            direction,
        ))
        .child(ShortcutRow::new_directional(
            theme,
            Text::new("▦"),
            "Applications",
            "Installed applications",
            Some(LauncherAction::SetView(LauncherView::Applications)),
            ShortcutState {
                selected: launcher.view() == LauncherView::Applications,
                ..ShortcutState::default()
            },
            direction,
        ))
        .child(ShortcutRow::new_directional(
            theme,
            Text::new("□"),
            "Places",
            "Files and locations",
            Some(LauncherAction::SetView(LauncherView::Places)),
            ShortcutState {
                selected: launcher.view() == LauncherView::Places,
                ..ShortcutState::default()
            },
            direction,
        ));
    let cards = (0..launcher.result_count())
        .filter_map(|index| {
            launcher.result_at(index).map(|app| {
                let icon = icons.resolve(app).unwrap_or_else(|| {
                    icons
                        .structural(
                            "applications",
                            include_bytes!("../../../assets/icons/start-menu/applications.svg"),
                            theme.text.secondary,
                        )
                        .expect("embedded application icon must rasterize")
                });
                (index, app.id().to_owned(), app.name().to_owned(), icon)
            })
        })
        .collect::<Vec<_>>();
    let selected = launcher.selected_result().map(Application::id);
    let collection = FileGrid::columns(columns)
        .items(cards.into_iter().map(|(index, id, name, icon)| {
            let accessible_name = name.clone();
            let is_selected = selected == Some(id.as_str());
            FilePlaneItem::new(LauncherAction::ActivateResult(index), name, icon.0, icon.1)
                .id(id)
                .min_height(TILE_HEIGHT)
                .padding(Insets::all(theme.spacing.control))
                .radius(theme.radii.card)
                .icon_size(ICON_SIZE)
                .label_height(36.0)
                .foreground(theme.text.primary)
                .selected_background(is_selected, theme.surfaces.selected)
                .interaction_backgrounds(theme.surfaces.hover, theme.surfaces.pressed)
                .focus_background_tint(theme.borders.focus)
                .controller_focus_background_tint(theme.borders.controller_focus)
                .semantic_role(nickel_ui::SemanticRole::Button)
                .accessibility_label(accessible_name)
        }))
        .id("launcher-search-results")
        .accessibility_label("Search results")
        .scroll_owner("launcher-search-scroll")
        .gap(GRID_GAP);
    let result_content = if launcher.result_count() == 0 {
        AnyView::new(Text::new("No matching applications").color(theme.text.secondary))
    } else {
        AnyView::new(
            VerticalScroll::new(LauncherAction::SearchScroll, 0.0)
                .id("launcher-search-scroll")
                .theme(theme)
                .grow(1.0)
                .child(collection),
        )
    };
    let detail = Column::new()
        .fill_width()
        .fill_height()
        .gap(theme.spacing.content)
        .padding(Insets::all(theme.spacing.content))
        .child(
            SectionHeader::new(
                theme,
                if launcher.query().is_empty() {
                    view_title(launcher.view())
                } else {
                    launcher.query()
                },
            )
            .direction(direction),
        )
        .child(result_content);
    let search = Container::new()
        .id("launcher-search-focus")
        .accessibility_label("Focus application search")
        .child(LauncherSearchField::new_directional(
            theme,
            structural_icon(
                icons,
                "search",
                include_bytes!("../../../assets/icons/settings/search.svg"),
                theme.text.secondary,
            ),
            launcher.query(),
            launcher.preedit(),
            "Search applications…",
            query_action,
            direction,
        ));
    let mut shell = StartMenuShell::new(theme, panel_width, sidebar, detail)
        .direction(direction)
        .header(search);
    if context.modality == InputModality::Controller {
        shell = shell.legend(launcher_action_legend(
            theme,
            state.controller_family,
            &context.legend_actions,
            panel_width,
        ));
    }
    if let Some(status) = context.status {
        shell = shell.detail_footer(launcher_status(theme, status));
    }
    AnyView::new(shell)
}

fn query_action(value: String) -> LauncherAction {
    LauncherAction::SetQuery(value)
}

#[allow(dead_code)] // Directly exercised by focused module tests.
fn build_dashboard_view(
    launcher: &Launcher,
    state: &LauncherViewState,
    icons: &mut LauncherIconCache,
    context: LauncherViewContext<'_>,
) -> AnyView<LauncherAction> {
    build_dashboard_view_directional(
        launcher,
        state,
        icons,
        context,
        launcher_reading_direction(),
    )
}

struct DashboardGeometry {
    width: f32,
    height: f32,
    sidebar_width: f32,
    grid_cell_width: f32,
}

fn dashboard_geometry(
    launcher: &Launcher,
    applications: &[&Application],
    theme: &SemanticTheme,
    viewport: (u32, u32),
) -> DashboardGeometry {
    let (viewport_width, viewport_height) = viewport;
    let viewport_width = viewport_width.max(1) as f32;
    let narrow = viewport_width < START_MENU_SINGLE_PANE_BREAKPOINT;
    let recent_project_names = match launcher.dashboard_projects() {
        crate::launcher::DashboardSection::Ready(projects) if launcher.codex_available() => {
            let mut recent = projects
                .iter()
                .filter(|project| project.last_used_at.is_some())
                .collect::<Vec<_>>();
            recent.sort_by_key(|project| std::cmp::Reverse(project.last_used_at));
            recent
                .into_iter()
                .take(3)
                .map(|project| nickel_ui::intrinsic_text_width(&project.name, 1.0))
                .collect::<Vec<_>>()
        }
        _ => Vec::new(),
    };
    let place_count = launcher.place_applications().count();
    let recent_count = recent_project_names.len();
    let sidebar_label_width = launcher
        .place_applications()
        .map(|place| nickel_ui::intrinsic_text_width(place.name(), 1.0))
        .chain(
            (launcher.codex_available())
                .then_some(nickel_ui::intrinsic_text_width("Recent projects", 0.8)),
        )
        .chain(recent_project_names)
        .chain(
            launcher
                .codex_available()
                .then_some(nickel_ui::intrinsic_text_width("All projects", 0.9)),
        )
        .fold(nickel_ui::intrinsic_text_width("Places", 0.8), f32::max);
    let account_width = match launcher.dashboard_account() {
        crate::launcher::DashboardSection::Ready(account) => {
            nickel_ui::intrinsic_text_width(&account.display_name, 1.0) + 64.0
        }
        _ => 120.0,
    };
    let sidebar_width = (sidebar_label_width + 20.0 + theme.spacing.control + 12.0)
        .max(account_width)
        .clamp(148.0, 240.0);
    let tile_width = applications
        .iter()
        .map(|application| nickel_ui::intrinsic_text_width(application.name(), 0.9))
        .fold(0.0, f32::max)
        .clamp(100.0, 142.0);
    let columns = 3.0;
    let detail_width = columns * tile_width + (columns - 1.0) * 2.0;
    let width = if narrow {
        (sidebar_width.max(detail_width) + 2.0 * theme.spacing.content)
            .max(320.0)
            .min(viewport_width)
    } else {
        (sidebar_width + detail_width + 5.0 * theme.spacing.content)
            .clamp(START_MENU_SINGLE_PANE_BREAKPOINT, DASHBOARD_MAX_WIDTH)
            .min(viewport_width)
    };
    let sidebar_body_height = 20.0
        + place_count as f32 * 38.0
        + if launcher.codex_available() {
            20.0 + recent_count as f32 * 38.0 + 32.0
        } else {
            0.0
        };
    let grid_rows = applications.len().max(1).div_ceil(3) as f32;
    let detail_body_height = 24.0 + grid_rows * 96.0 + 58.0 + 3.0 * theme.spacing.content;
    let height =
        (sidebar_body_height.max(detail_body_height) + 52.0 + 52.0 + 3.0 * theme.spacing.content)
            .min(viewport_height.max(1) as f32);
    let grid_cell_width = if narrow {
        (width - 4.0 * theme.spacing.content - 4.0) / 3.0
    } else {
        (width - sidebar_width - 5.0 * theme.spacing.content - 4.0) / 3.0
    }
    .max(48.0);
    DashboardGeometry {
        width,
        height,
        sidebar_width,
        grid_cell_width,
    }
}

fn build_dashboard_view_directional(
    launcher: &Launcher,
    state: &LauncherViewState,
    icons: &mut LauncherIconCache,
    context: LauncherViewContext<'_>,
    direction: ReadingDirection,
) -> AnyView<LauncherAction> {
    let applications = dashboard_applications(launcher);
    let theme = launcher_semantic_theme(context.palette);
    let geometry = dashboard_geometry(launcher, &applications, &theme, context.viewport);
    let width = geometry.width;
    let sidebar_width = geometry.sidebar_width;
    let grid_cell_width = geometry.grid_cell_width;
    let preferred_height = geometry.height;
    let viewport_width = context.viewport.0.max(1) as f32;
    let narrow = viewport_width < START_MENU_SINGLE_PANE_BREAKPOINT;
    let nav_icon = |icons: &mut LauncherIconCache, name, bytes: &[u8]| {
        structural_icon(icons, name, bytes, theme.text.secondary)
    };

    let mut sidebar = Column::new()
        .gap(2.0)
        .child(Text::new("Places").scale(0.8).color(theme.text.secondary));
    for place in launcher.place_applications() {
        let icon = icons.resolve(place).unwrap_or_else(|| {
            icons
                .structural(
                    "places",
                    include_bytes!("../../../assets/icons/start-menu/project.svg"),
                    theme.text.secondary,
                )
                .expect("embedded Places icon must rasterize")
        });
        sidebar = sidebar.child(dashboard_link_row(
            theme,
            Image::new(icon.0, icon.1).width(20.0).height(20.0),
            place.name(),
            LauncherAction::LaunchApplication(place.id().to_owned()),
        ));
    }
    if launcher.codex_available() {
        sidebar = sidebar.child(
            Text::new("Recent projects")
                .scale(0.8)
                .color(theme.text.secondary),
        );
        if let crate::launcher::DashboardSection::Ready(projects) = launcher.dashboard_projects() {
            let mut recent = projects
                .iter()
                .filter(|project| project.last_used_at.is_some())
                .collect::<Vec<_>>();
            recent.sort_by_key(|project| std::cmp::Reverse(project.last_used_at));
            for project in recent.into_iter().take(3) {
                sidebar = sidebar.child(dashboard_link_row(
                    theme,
                    nav_icon(
                        icons,
                        "project",
                        include_bytes!("../../../assets/icons/start-menu/project.svg"),
                    )
                    .width(20.0)
                    .height(20.0),
                    &project.name,
                    LauncherAction::OpenProject(project.id.clone()),
                ));
            }
        }
        sidebar = sidebar.child(dashboard_text_link(
            theme,
            "All projects",
            LauncherAction::SeeAllProjects,
        ));
    }
    if narrow {
        sidebar = sidebar
            .child(dashboard_text_link(
                theme,
                "Pinned & recent",
                LauncherAction::SetView(LauncherView::Favorites),
            ))
            .child(dashboard_text_link(
                theme,
                "All applications",
                LauncherAction::SetView(LauncherView::Applications),
            ));
    }
    let mut settings_button = Some(
        Container::new()
            .shrink(0.0)
            .min_height(44.0)
            .padding(Insets::symmetric(6.0, 10.0))
            .radius(theme.radii.control)
            .background(theme.surfaces.raised)
            .interaction_backgrounds(theme.surfaces.hover, theme.surfaces.pressed)
            .focus_background_tint(theme.borders.focus)
            .controller_focus_background_tint(theme.borders.controller_focus)
            .message(LauncherAction::OpenSettings(SettingsDestination::Nickel))
            .semantic_role(nickel_ui::SemanticRole::Button)
            .accessibility_label("Settings")
            .align_items(nickel_ui::Align::Center)
            .justify_content(nickel_ui::Justify::Center)
            .child(
                nickel_ui::Row::new()
                    .align_items(nickel_ui::Align::Center)
                    .gap(theme.spacing.compact)
                    .child(
                        nav_icon(
                            icons,
                            "settings",
                            include_bytes!("../../../assets/icons/start-menu/settings.svg"),
                        )
                        .width(24.0)
                        .height(24.0),
                    )
                    .child(Text::new("Settings").color(theme.text.primary)),
            ),
    );

    let account = match launcher.dashboard_account() {
        crate::launcher::DashboardSection::Ready(account) => {
            AnyView::new(AccountSummaryRow::new_directional(
                theme,
                FallbackAvatar::new(theme, &account.display_name),
                &account.display_name,
                "",
                Some(LauncherAction::OpenAccount),
                ShortcutState::default(),
                direction,
            ))
        }
        _ => AnyView::new(AccountSummaryRow::new_directional(
            theme,
            nav_icon(
                icons,
                "account",
                include_bytes!("../../../assets/icons/start-menu/account.svg"),
            ),
            "Local session",
            "",
            Some(LauncherAction::OpenAccount),
            ShortcutState::default(),
            direction,
        )),
    };
    let sidebar_footer = if narrow && state.dashboard_narrow_page == DashboardNarrowPage::Primary {
        AnyView::new(
            nickel_ui::Row::new()
                .fill_width()
                .align_items(nickel_ui::Align::Center)
                .child(
                    Container::new()
                        .grow(1.0)
                        .child(account.id("launcher-account")),
                )
                .child(settings_button.take().expect("settings button available")),
        )
    } else {
        AnyView::new(account.id("launcher-account"))
    };

    let application_cards = applications
        .into_iter()
        .map(|application| {
            let id = application.id().to_owned();
            let icon = icons.resolve(application).unwrap_or_else(|| {
                icons
                    .structural(
                        "applications",
                        include_bytes!("../../../assets/icons/start-menu/applications.svg"),
                        theme.text.secondary,
                    )
                    .expect("embedded applications icon must rasterize")
            });
            (id, application.name().to_owned(), icon)
        })
        .collect::<Vec<_>>();
    let applications_empty = application_cards.is_empty();
    let application_collection = FileGrid::columns(3)
        .width(grid_cell_width * 3.0 + 4.0)
        .items(application_cards.into_iter().map(|(id, name, icon)| {
            let accessible_name = name.clone();
            FilePlaneItem::new(
                LauncherAction::LaunchApplication(id.clone()),
                name,
                icon.0,
                icon.1,
            )
            .id(id)
            .min_height(94.0)
            .padding(Insets::symmetric(5.0, 2.0))
            .radius(theme.radii.card)
            .icon_size(48.0)
            .label_height(34.0)
            .label_scale(0.9)
            .gap(2.0)
            .center_content()
            .foreground(theme.text.primary)
            .interaction_backgrounds(theme.surfaces.hover, theme.surfaces.pressed)
            .focus_background_tint(theme.borders.focus)
            .controller_focus_background_tint(theme.borders.controller_focus)
            .semantic_role(nickel_ui::SemanticRole::Button)
            .accessibility_label(accessible_name)
        }))
        .id("launcher-applications")
        .scroll_owner("launcher-dashboard-scroll")
        .gap(2.0)
        .accessibility_label("Applications");

    let title = match launcher.view() {
        LauncherView::Favorites => "Pinned & recent",
        LauncherView::Applications => "All applications",
        LauncherView::Places => "Places",
    };
    let mut detail = Column::new()
        .fill_width()
        .fill_height()
        .gap(theme.spacing.content)
        .padding(Insets::all(theme.spacing.content));
    if narrow {
        detail = detail.child(ShortcutRow::new_directional(
            theme,
            nav_icon(
                icons,
                "back",
                include_bytes!("../../../assets/icons/start-menu/applications.svg"),
            ),
            "Back",
            "Navigation",
            Some(LauncherAction::ShowNarrowPrimary),
            ShortcutState::default(),
            direction,
        ));
    }
    detail = detail.child(SectionHeader::new(theme, title).direction(direction));
    if launcher.view() == LauncherView::Applications {
        detail = detail.child(dashboard_view_switch(
            theme,
            icons,
            launcher.view(),
            direction,
            grid_cell_width,
        ));
    }
    if applications_empty {
        detail = detail.child(
            Container::new()
                .fill_width()
                .padding(Insets::all(theme.spacing.content))
                .child(
                    Text::new(match launcher.view() {
                        LauncherView::Favorites => "Pin or launch applications to add them here.",
                        LauncherView::Applications => "No installed applications are available.",
                        LauncherView::Places => "No places are available.",
                    })
                    .color(theme.text.secondary),
                ),
        );
    } else {
        detail = detail.child(application_collection);
    }
    if launcher.view() != LauncherView::Applications {
        detail = detail.child(dashboard_view_switch(
            theme,
            icons,
            launcher.view(),
            direction,
            grid_cell_width,
        ));
    }

    let search = Container::new()
        .id("launcher-search-focus")
        .accessibility_label("Focus application search")
        .child(LauncherSearchField::new_directional(
            theme,
            structural_icon(
                icons,
                "search",
                include_bytes!("../../../assets/icons/settings/search.svg"),
                theme.text.secondary,
            ),
            launcher.query(),
            launcher.preedit(),
            "Search applications…",
            query_action,
            direction,
        ));
    let detail = VerticalScroll::new(LauncherAction::DashboardScroll, 0.0)
        .id("launcher-dashboard-scroll")
        .theme(theme)
        .grow(1.0)
        .child(detail);
    let mut shell = StartMenuShell::new(theme, width, sidebar, detail)
        .primary_width(sidebar_width)
        .viewport_width(viewport_width)
        .preferred_height(preferred_height)
        .direction(direction)
        .header(search)
        .primary_footer(sidebar_footer)
        .narrow_pane(
            if narrow && state.dashboard_narrow_page != DashboardNarrowPage::Primary {
                StartMenuNarrowPane::Detail
            } else {
                StartMenuNarrowPane::Primary
            },
        );
    if context.modality == InputModality::Controller {
        shell = shell.legend(launcher_action_legend(
            theme,
            state.controller_family,
            &context.legend_actions,
            width,
        ));
    }
    let mut detail_footer = nickel_ui::Row::new()
        .fill_width()
        .align_items(nickel_ui::Align::Center)
        .justify_content(nickel_ui::Justify::End)
        .gap(theme.spacing.compact);
    if let Some(status) = context.status {
        detail_footer = detail_footer.child(
            Container::new()
                .grow(1.0)
                .child(launcher_status(theme, status)),
        );
    }
    if let Some(settings_button) = settings_button {
        shell = shell.detail_footer(detail_footer.child(settings_button));
    } else if context.status.is_some() {
        shell = shell.detail_footer(detail_footer);
    }
    AnyView::new(shell)
}

fn launcher_action_legend(
    theme: SemanticTheme,
    family: ControllerFamily,
    actions: &ActionLegendActions,
    available_width: f32,
) -> ActionLegend<LauncherAction> {
    let localizer = Localizer::system();
    let overlay_open = actions.is_overlay();
    let entries = actions.iter().map(|action| {
        let label = match action {
            SemanticControllerAction::Confirm if overlay_open => ActionLabel::Select,
            SemanticControllerAction::Confirm => ActionLabel::Open,
            SemanticControllerAction::ContextMenu => ActionLabel::Actions,
            SemanticControllerAction::PreviousSection => ActionLabel::Sidebar,
            SemanticControllerAction::NextSection => ActionLabel::Content,
            SemanticControllerAction::Cancel if overlay_open => ActionLabel::Back,
            SemanticControllerAction::Cancel => ActionLabel::Close,
            SemanticControllerAction::Pin => ActionLabel::Pin,
            SemanticControllerAction::Unpin => ActionLabel::Unpin,
            SemanticControllerAction::ToggleLauncher => ActionLabel::Launcher,
        };
        ActionLegendEntry::localized(action, label, &localizer)
    });
    ActionLegend::new_localized(theme, family, entries, &localizer, available_width, true)
}

fn launcher_status(theme: SemanticTheme, status: &str) -> Container<LauncherAction> {
    Container::new()
        .fill_width()
        .padding(Insets::all(theme.spacing.control))
        .radius(theme.radii.control)
        .background(theme.surfaces.raised)
        .accessibility_label("Launcher status")
        .child(Text::new(status).color(theme.text.warning).wrap(true))
}

#[allow(dead_code)]
fn launcher_reading_direction() -> ReadingDirection {
    static DIRECTION: OnceLock<ReadingDirection> = OnceLock::new();
    *DIRECTION.get_or_init(|| {
        let locale = ["LC_ALL", "LC_MESSAGES", "LANG"]
            .into_iter()
            .find_map(|name| std::env::var(name).ok().filter(|value| !value.is_empty()))
            .unwrap_or_default();
        reading_direction_for_locale(&locale)
    })
}

fn reading_direction_for_locale(locale: &str) -> ReadingDirection {
    let language = locale
        .split(['_', '-', '.', '@'])
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if matches!(language.as_str(), "ar" | "fa" | "he" | "ur") {
        ReadingDirection::RightToLeft
    } else {
        ReadingDirection::LeftToRight
    }
}

fn structural_icon(
    icons: &mut LauncherIconCache,
    name: &str,
    bytes: &[u8],
    color: u32,
) -> Image<LauncherAction> {
    let (id, image) = icons
        .structural(name, bytes, color)
        .expect("embedded Start Menu icon must rasterize");
    Image::new(id, image).width(28.0).height(28.0)
}

fn dashboard_link_row(
    theme: SemanticTheme,
    icon: Image<LauncherAction>,
    label: &str,
    action: LauncherAction,
) -> Container<LauncherAction> {
    Container::new()
        .fill_width()
        .min_height(36.0)
        .padding(Insets::symmetric(5.0, 6.0))
        .radius(theme.radii.control)
        .interaction_backgrounds(theme.surfaces.hover, theme.surfaces.pressed)
        .focus_background_tint(theme.borders.focus)
        .controller_focus_background_tint(theme.borders.controller_focus)
        .message(action)
        .semantic_role(nickel_ui::SemanticRole::Button)
        .accessibility_label(label)
        .child(
            nickel_ui::Row::new()
                .fill_width()
                .align_items(nickel_ui::Align::Center)
                .gap(theme.spacing.control)
                .child(icon)
                .child(Text::new(label).color(theme.text.primary).ellipsis(true)),
        )
}

fn dashboard_text_link(
    theme: SemanticTheme,
    label: &str,
    action: LauncherAction,
) -> Container<LauncherAction> {
    Container::new()
        .fill_width()
        .min_height(30.0)
        .padding(Insets::symmetric(5.0, 6.0))
        .radius(theme.radii.control)
        .interaction_backgrounds(theme.surfaces.hover, theme.surfaces.pressed)
        .focus_background_tint(theme.borders.focus)
        .controller_focus_background_tint(theme.borders.controller_focus)
        .message(action)
        .semantic_role(nickel_ui::SemanticRole::Button)
        .accessibility_label(label)
        .child(Text::new(label).scale(0.9).color(theme.text.secondary))
}

fn dashboard_view_switch(
    theme: SemanticTheme,
    icons: &mut LauncherIconCache,
    view: LauncherView,
    direction: ReadingDirection,
    grid_cell_width: f32,
) -> AnyView<LauncherAction> {
    if view == LauncherView::Applications {
        AnyView::new(dashboard_text_link(
            theme,
            "Pinned & recent",
            LauncherAction::SetView(LauncherView::Favorites),
        ))
    } else {
        let content = nickel_ui::Row::new()
            .fill_width()
            .align_items(nickel_ui::Align::Center)
            .gap(theme.spacing.compact)
            .child(
                Container::new()
                    .width(grid_cell_width)
                    .shrink(0.0)
                    .align_items(nickel_ui::Align::Center)
                    .justify_content(nickel_ui::Justify::Center)
                    .child(
                        structural_icon(
                            icons,
                            "all-applications",
                            include_bytes!("../../../assets/icons/start-menu/applications.svg"),
                            theme.text.secondary,
                        )
                        .id("launcher-all-applications-icon")
                        .width(48.0)
                        .height(48.0),
                    ),
            )
            .child(
                Column::new()
                    .gap(2.0)
                    .child(Text::new("All applications").color(theme.text.primary))
                    .child(
                        Text::new("Browse installed applications")
                            .scale(0.9)
                            .color(theme.text.secondary),
                    ),
            );
        let content = if direction == ReadingDirection::RightToLeft {
            content.reverse()
        } else {
            content
        };
        AnyView::new(
            Container::new()
                .fill_width()
                .min_height(58.0)
                .radius(theme.radii.control)
                .interaction_backgrounds(theme.surfaces.hover, theme.surfaces.pressed)
                .focus_background_tint(theme.borders.focus)
                .controller_focus_background_tint(theme.borders.controller_focus)
                .message(LauncherAction::SetView(LauncherView::Applications))
                .semantic_role(nickel_ui::SemanticRole::Button)
                .accessibility_label("All applications")
                .child(content),
        )
    }
}

fn view_title(view: LauncherView) -> &'static str {
    match view {
        LauncherView::Favorites => "Favorites",
        LauncherView::Applications => "Applications",
        LauncherView::Places => "Places",
    }
}

fn has_visible_pixel(image: &RgbaImage) -> bool {
    image.pixels().any(|pixel| pixel[3] != 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nickel_input::{
        DeviceId, EventOrder, InputEvent, KeyCode, KeyEdge, KeyEvent, KeyLocation, LogicalKey,
        ModifierState, NamedKey, PhysicalKey, PointerEvent, TextEvent, Vector,
    };
    use nickel_ui::{
        ActionKind, ControllerAction, HostBatch, HostEvent, Point, SemanticAction,
        SemanticValueInput, Shortcut, UiEvent, UiHost,
    };
    use nickel_ui_testkit::{
        ReachabilityModality, ReachabilityPolicy, Scenario, Selector, audit_reachability,
    };

    fn palette() -> ThemePalette {
        ThemePalette {
            background: 0x101114,
            panel: 0x15171b,
            surface: 0x1b1e23,
            surface_hover: 0x32363e,
            text: 0xf2f3f5,
            muted: 0xa8abb2,
            accent: 0x9b62e8,
            accent_soft: 0x45305f,
            complement: 0x55b982,
        }
    }

    fn launcher_host() -> UiHost<LauncherApplication> {
        UiHost::new(
            LauncherApplication::new(
                Launcher::default(),
                LauncherViewState::default(),
                LauncherIconCache::new(),
                palette(),
            ),
            920,
            680,
        )
    }

    #[test]
    fn dashboard_surface_matches_content_and_search_can_expand() {
        let mut application = LauncherApplication::new(
            Launcher::default(),
            LauncherViewState::default(),
            LauncherIconCache::new(),
            palette(),
        );
        let maximum = (960, 720);
        let dashboard = application.preferred_surface_size(maximum);
        assert!((620..960).contains(&dashboard.0));
        assert!(dashboard.1 < maximum.1);
        assert!(application.preferred_surface_size((480, 500)).0 <= 480);

        application.launcher.open_search();
        assert_eq!(application.preferred_surface_size(maximum), maximum);
    }

    fn accessibility_labels(host: &UiHost<LauncherApplication>) -> Vec<String> {
        host.accessibility_nodes()
            .iter()
            .filter_map(|node| node.label.clone())
            .collect()
    }

    fn launcher_scenario() -> Scenario<LauncherApplication> {
        Scenario::new(
            LauncherApplication::new(
                Launcher::default(),
                LauncherViewState::default(),
                LauncherIconCache::new(),
                palette(),
            ),
            920,
            680,
        )
    }

    fn navigation_key(order: u64, physical: KeyCode, logical: NamedKey) -> InputEvent {
        InputEvent::Key(KeyEvent {
            device: DeviceId(1),
            order: EventOrder(order),
            physical: PhysicalKey::Code(physical),
            logical: LogicalKey::Named(logical),
            location: KeyLocation::Standard,
            edge: KeyEdge::Pressed,
            repeat: false,
            modifiers: ModifierState::default(),
        })
    }

    fn populated_launcher_scenario() -> Scenario<LauncherApplication> {
        let mut launcher = Launcher::new(
            (0..30)
                .map(|index| {
                    Application::new(
                        format!("application-{index:02}"),
                        format!("Application {index:02}"),
                        None,
                        None,
                        None,
                    )
                })
                .collect(),
        );
        launcher.set_view(LauncherView::Applications);
        Scenario::new(
            LauncherApplication::new(
                launcher,
                LauncherViewState::default(),
                LauncherIconCache::new(),
                palette(),
            ),
            920,
            680,
        )
    }

    fn populated_search_scenario() -> Scenario<LauncherApplication> {
        let mut launcher = Launcher::new(
            (0..30)
                .map(|index| {
                    Application::new(
                        format!("application-{index:02}"),
                        format!("Application {index:02}"),
                        None,
                        None,
                        None,
                    )
                })
                .collect(),
        );
        launcher.open_search();
        Scenario::new(
            LauncherApplication::new(
                launcher,
                LauncherViewState::default(),
                LauncherIconCache::new(),
                palette(),
            ),
            920,
            680,
        )
    }

    fn controller_target(scenario: &Scenario<LauncherApplication>) -> String {
        scenario
            .host()
            .inspect()
            .controller_target
            .expect("controller input establishes a useful launcher target")
            .as_str()
            .to_owned()
    }

    #[test]
    fn opening_launcher_focuses_search_and_accepts_typing_without_navigation() {
        for width in [480, 920] {
            for direction in [ReadingDirection::LeftToRight, ReadingDirection::RightToLeft] {
                let mut application = LauncherApplication::new(
                    Launcher::default(),
                    LauncherViewState::default(),
                    LauncherIconCache::new(),
                    palette(),
                );
                application.set_reading_direction(direction);
                let mut scenario = Scenario::new(application, width, 680);
                let search = scenario
                    .host()
                    .query_unique(&nickel_ui::SemanticSelector::Role(
                        nickel_ui::SemanticRole::TextField,
                    ))
                    .expect("launcher search");
                scenario.host_mut().step(HostBatch {
                    window_focused: Some(true),
                    ..HostBatch::default()
                });
                assert_eq!(
                    scenario.host().inspect().keyboard_focus,
                    Some(search.id.clone())
                );
                scenario.host_mut().step(HostBatch {
                    events: vec![HostEvent::Normalized {
                        input: InputEvent::Text(TextEvent::Commit {
                            device: DeviceId(7),
                            order: EventOrder(1),
                            text: "fire".into(),
                        }),
                        clipboard_text: None,
                    }],
                    ..HostBatch::default()
                });
                assert_eq!(scenario.host().application().launcher.query(), "fire");
                assert_eq!(
                    scenario.host_mut().application_mut().take_effects(),
                    [LauncherAction::SetQuery("fire".into())]
                );
                assert_eq!(scenario.host().inspect().keyboard_focus, Some(search.id));
            }
        }
    }

    #[test]
    fn launcher_search_spans_both_panes_without_sidebar_branding() {
        let host = launcher_host();
        let search = host
            .query_unique(&nickel_ui::SemanticSelector::Role(
                nickel_ui::SemanticRole::TextField,
            ))
            .expect("launcher search");
        let all_applications = host
            .unique_semantic_target_for_message(&LauncherAction::SetView(
                LauncherView::Applications,
            ))
            .expect("All applications action");
        let application = host
            .unique_semantic_target_for_message(&LauncherAction::LaunchApplication(
                "firefox".into(),
            ))
            .expect("Home application");
        assert!(
            search.bounds.origin.y + search.bounds.size.height <= all_applications.bounds.origin.y
        );
        assert!(search.bounds.origin.y + search.bounds.size.height <= application.bounds.origin.y);
        let header = host
            .accessibility_nodes()
            .iter()
            .find(|node| node.label.as_deref() == Some("Focus application search"))
            .expect("search header");
        assert!(header.rect.origin.x <= all_applications.bounds.origin.x);
        assert!(
            header.rect.origin.x + header.rect.size.width
                >= application.bounds.origin.x + application.bounds.size.width
        );
        assert!(
            !accessibility_labels(&host)
                .iter()
                .any(|label| label == "Nickel")
        );
    }

    #[test]
    fn controller_scenario_switches_peer_panes_and_contains_dpad() {
        let mut scenario = populated_launcher_scenario();

        scenario.controller(ControllerAction::Down).unwrap();
        let first_target = controller_target(&scenario);
        assert!(
            first_target.contains("start-menu-detail-pane"),
            "selected {first_target}"
        );
        scenario.controller(ControllerAction::PreviousPane).unwrap();
        let sidebar_target = controller_target(&scenario);
        assert!(sidebar_target.contains("start-menu-primary-pane"));

        scenario.controller(ControllerAction::NextPane).unwrap();
        assert_eq!(controller_target(&scenario), first_target);
        scenario.controller(ControllerAction::Down).unwrap();
        let content_home = controller_target(&scenario);
        assert!(
            content_home.contains("start-menu-detail-pane"),
            "selected {content_home}"
        );
        for _ in 0..5 {
            scenario.controller(ControllerAction::Down).unwrap();
        }
        let content_moved = controller_target(&scenario);
        assert!(
            content_moved.contains("start-menu-detail-pane"),
            "D-pad escaped content pane: {content_moved}"
        );

        scenario.controller(ControllerAction::PreviousPane).unwrap();
        assert_eq!(controller_target(&scenario), sidebar_target);
        scenario.controller(ControllerAction::NextPane).unwrap();
        assert_eq!(controller_target(&scenario), content_moved);
    }

    #[test]
    fn controller_left_backs_out_of_content_to_the_sidebar() {
        let mut scenario = populated_launcher_scenario();

        scenario.controller(ControllerAction::Down).unwrap();
        assert!(controller_target(&scenario).contains("start-menu-detail-pane"));

        for _ in 0..4 {
            scenario.controller(ControllerAction::Left).unwrap();
            if controller_target(&scenario).contains("start-menu-primary-pane") {
                return;
            }
        }
        panic!(
            "Left did not back out to the sidebar; selected {}",
            controller_target(&scenario)
        );
    }

    #[test]
    fn controller_moves_from_recent_project_toward_home_tiles() {
        let mut launcher = Launcher::default();
        launcher.set_codex_available(true);
        launcher.set_dashboard_projects(crate::launcher::DashboardSection::Ready(vec![
            crate::launcher::DashboardProject {
                id: "project".into(),
                name: "Project".into(),
                roots: Vec::new(),
                chat_count: Some(1),
                activity: crate::launcher::ProjectActivity::Idle,
                last_used_at: None,
            },
        ]));
        let mut host = UiHost::new(
            LauncherApplication::new(
                launcher.clone(),
                LauncherViewState::default(),
                LauncherIconCache::new(),
                palette(),
            ),
            920,
            680,
        );
        for _ in 0..12 {
            host.application_mut().sync(&launcher, palette(), None);
            host.step(HostBatch {
                events: vec![HostEvent::Controller(ControllerAction::Down)],
                ..HostBatch::default()
            });
            if host
                .inspect()
                .controller_target
                .as_ref()
                .is_some_and(|target| target.as_str().ends_with("/launcher-applications"))
            {
                break;
            }
        }

        let target = host
            .inspect()
            .controller_target
            .expect("controller navigation selects the application grid");
        assert!(
            target.as_str().ends_with("/launcher-applications"),
            "{target:?}"
        );
        host.application_mut().sync(&launcher, palette(), None);
        host.step(HostBatch {
            events: vec![HostEvent::Controller(ControllerAction::Right)],
            ..HostBatch::default()
        });
        let target = host
            .inspect()
            .controller_target
            .expect("Right enters the selected application grid");
        assert!(
            target.as_str().contains("launcher-applications/"),
            "{target:?}"
        );
        assert!(
            host.accessibility_nodes()
                .iter()
                .any(|node| node.id == target)
        );
        let labels = accessibility_labels(&host);
        assert!(labels.contains(&"confirm control: Open".to_owned()));
        assert!(labels.contains(&"menu control: Actions".to_owned()));
    }

    #[test]
    fn normalized_keyboard_enters_the_all_applications_grid() {
        let mut launcher = Launcher::default();
        launcher.set_view(LauncherView::Applications);
        let mut host = UiHost::new(
            LauncherApplication::new(
                launcher,
                LauncherViewState::default(),
                LauncherIconCache::new(),
                palette(),
            ),
            920,
            680,
        );
        host.handle_input(
            &navigation_key(1, KeyCode::ArrowDown, NamedKey::ArrowDown),
            None,
        );
        for order in 2..=5 {
            host.handle_input(
                &navigation_key(order, KeyCode::ArrowDown, NamedKey::ArrowDown),
                None,
            );
            if host
                .inspect()
                .controller_target
                .as_ref()
                .is_some_and(|id| id.as_str().ends_with("/launcher-applications"))
            {
                break;
            }
        }
        assert!(
            host.inspect()
                .controller_target
                .as_ref()
                .is_some_and(|id| id.as_str().ends_with("/launcher-applications"))
        );
        host.handle_input(
            &navigation_key(6, KeyCode::ArrowRight, NamedKey::ArrowRight),
            None,
        );
        assert!(
            host.inspect()
                .controller_target
                .as_ref()
                .is_some_and(|id| { id.as_str().contains("/launcher-applications/") })
        );
        assert_eq!(host.inspect().modality, InputModality::Keyboard);
        assert!(
            !accessibility_labels(&host)
                .iter()
                .any(|label| label.contains("confirm control"))
        );
    }

    #[test]
    fn controller_scenario_home_requests_open_and_close_without_local_polling() {
        let mut scenario = launcher_scenario();
        scenario.controller(ControllerAction::Launcher).unwrap();
        scenario.controller(ControllerAction::Launcher).unwrap();

        let operations = scenario.operation_trace();
        assert_eq!(operations.len(), 2);
        for operation in operations {
            assert_eq!(operation.outcome.global_actions, ["ToggleLauncher"]);
            assert!(!operation.outcome.rebuilt);
        }
    }

    #[test]
    fn submit_shortcut_activates_the_selected_search_result() {
        let mut launcher = Launcher::new(vec![Application::new(
            "org.nickel.Terminal".into(),
            "Nickel Terminal".into(),
            None,
            None,
            None,
        )]);
        launcher.set_query("terminal");
        let mut host = UiHost::new(
            LauncherApplication::new(
                launcher,
                LauncherViewState::default(),
                LauncherIconCache::new(),
                palette(),
            ),
            920,
            680,
        );

        let outcome = host.step(HostBatch {
            events: vec![HostEvent::Shortcut(Shortcut::Submit)],
            ..HostBatch::default()
        });

        assert!(outcome.changed);
        assert_eq!(
            host.application_mut().take_effects(),
            [LauncherAction::ActivateResult(0)]
        );
    }

    #[test]
    fn populated_launcher_emits_machine_readable_controller_reachability() {
        let report = audit_reachability(
            populated_launcher_scenario,
            &ReachabilityPolicy {
                modalities: [ReachabilityModality::Controller].into_iter().collect(),
                maximum_path_length: 32,
                maximum_state_count: 64,
                wall_time_ms: 1_000,
                require_semantic_change: false,
            },
        );
        let launch = report
            .paths
            .iter()
            .find(|path| {
                path.target.contains("launcher-applications/application-00")
                    && path.action == "Activate"
                    && path.modality == ReachabilityModality::Controller
            })
            .expect("representative populated application has a controller path");
        assert!(
            launch.reached,
            "path: {launch:?}; issues: {:?}",
            report.issues
        );
        let json = report
            .to_json()
            .expect("reachability report is serializable");
        assert!(json.contains("launcher-applications/application-00"));
    }

    #[test]
    fn controller_reaches_and_reveals_the_last_search_result() {
        let mut scenario = populated_search_scenario();
        scenario.controller(ControllerAction::Down).unwrap();
        scenario.controller(ControllerAction::Down).unwrap();
        scenario.controller(ControllerAction::Right).unwrap();
        for _ in 0..40 {
            if controller_target(&scenario).contains("application-28") {
                break;
            }
            scenario.controller(ControllerAction::Down).unwrap();
        }
        scenario.controller(ControllerAction::Right).unwrap();
        assert!(
            controller_target(&scenario).contains("application-29"),
            "controller did not traverse the complete search collection"
        );
        let selected = scenario
            .semantic_nodes()
            .into_iter()
            .find(|node| node.id.as_str().contains("application-29"))
            .expect("last result remains semantic after scrolling");
        assert!(selected.bounds.origin.y >= 0.0);
        assert!(selected.bounds.origin.y + selected.bounds.size.height <= 680.0);
        scenario.controller(ControllerAction::Confirm).unwrap();
        assert_eq!(
            scenario.host_mut().application_mut().take_effects(),
            [LauncherAction::ActivateResult(29)]
        );
    }

    #[test]
    fn keyboard_boundary_and_page_navigation_reveal_and_activate_results() {
        for (mut scenario, expected) in [
            (
                populated_search_scenario(),
                LauncherAction::ActivateResult(29),
            ),
            (
                populated_launcher_scenario(),
                LauncherAction::LaunchApplication("application-29".into()),
            ),
        ] {
            for (order, physical, logical) in [
                (1, KeyCode::ArrowDown, NamedKey::ArrowDown),
                (2, KeyCode::ArrowDown, NamedKey::ArrowDown),
                (3, KeyCode::ArrowRight, NamedKey::ArrowRight),
                (4, KeyCode::PageDown, NamedKey::PageDown),
            ] {
                scenario
                    .host_mut()
                    .handle_input(&navigation_key(order, physical, logical), None);
            }
            let page_target = controller_target(&scenario);
            assert!(
                !page_target.contains("application-00"),
                "PageDown did not advance the application viewport: {page_target}"
            );

            scenario
                .host_mut()
                .handle_input(&navigation_key(5, KeyCode::End, NamedKey::End), None);
            assert!(controller_target(&scenario).contains("application-29"));
            let last = scenario
                .semantic_nodes()
                .into_iter()
                .find(|node| node.id.as_str().contains("application-29"))
                .expect("End retains the last application semantically");
            assert!(last.bounds.origin.y >= 0.0);
            assert!(last.bounds.origin.y + last.bounds.size.height <= 680.0);
            scenario
                .host_mut()
                .handle_input(&navigation_key(6, KeyCode::Enter, NamedKey::Enter), None);
            assert_eq!(
                scenario.host_mut().application_mut().take_effects(),
                [expected]
            );

            scenario
                .host_mut()
                .handle_input(&navigation_key(7, KeyCode::Home, NamedKey::Home), None);
            assert!(controller_target(&scenario).contains("application-00"));

            scenario
                .host_mut()
                .handle_input(&navigation_key(8, KeyCode::End, NamedKey::End), None);
            scenario
                .host_mut()
                .handle_input(&navigation_key(9, KeyCode::PageUp, NamedKey::PageUp), None);
            let page_up_target = controller_target(&scenario);
            assert!(
                !page_up_target.contains("application-29"),
                "PageUp did not move away from the final entry"
            );
            scenario.host_mut().handle_input(
                &navigation_key(10, KeyCode::ArrowUp, NamedKey::ArrowUp),
                None,
            );
            assert_ne!(controller_target(&scenario), page_up_target);
        }
    }

    #[test]
    fn launcher_wheel_accepts_line_and_pixel_deltas_inside_its_viewport() {
        for (mut scenario, scroll_action) in [
            (populated_search_scenario(), LauncherAction::SearchScroll),
            (
                populated_launcher_scenario(),
                LauncherAction::DashboardScroll,
            ),
        ] {
            let scroll = scenario
                .host()
                .semantic_targets_for_message(&scroll_action)
                .into_iter()
                .next()
                .expect("application results expose a scroll viewport");
            let point = nickel_input::Point {
                x: f64::from(scroll.bounds.origin.x + scroll.bounds.size.width / 2.0),
                y: f64::from(scroll.bounds.origin.y + scroll.bounds.size.height / 2.0),
            };
            let first_y = |scenario: &Scenario<LauncherApplication>| {
                scenario
                    .semantic_nodes()
                    .into_iter()
                    .find(|node| node.id.as_str().contains("application-00"))
                    .expect("first application remains in the authoritative collection")
                    .bounds
                    .origin
                    .y
            };
            let before = first_y(&scenario);
            scenario.host_mut().handle_input(
                &InputEvent::Pointer(PointerEvent::Axis {
                    device: DeviceId(1),
                    order: EventOrder(1),
                    delta: Vector { x: 0.0, y: -2.0 },
                    discrete: Some((0, -2)),
                    position: Some(point),
                }),
                None,
            );
            let after_lines = first_y(&scenario);
            assert!(after_lines < before);
            scenario.host_mut().handle_input(
                &InputEvent::Pointer(PointerEvent::Axis {
                    device: DeviceId(1),
                    order: EventOrder(2),
                    delta: Vector { x: 0.0, y: -2.5 },
                    discrete: None,
                    position: Some(point),
                }),
                None,
            );
            let after_pixels = first_y(&scenario);
            assert!(after_pixels < after_lines);
            assert!((after_lines - after_pixels - 2.5).abs() < 0.01);
        }
    }

    #[test]
    fn launcher_scroll_is_contained_and_filtering_reconciles_the_viewport() {
        let mut scenario = populated_search_scenario();
        let scroll = scenario
            .host()
            .semantic_targets_for_message(&LauncherAction::SearchScroll)
            .into_iter()
            .next()
            .expect("application results expose a scroll viewport");
        let inside = nickel_input::Point {
            x: f64::from(scroll.bounds.origin.x + scroll.bounds.size.width / 2.0),
            y: f64::from(scroll.bounds.origin.y + scroll.bounds.size.height / 2.0),
        };
        let axis = |order, position| {
            InputEvent::Pointer(PointerEvent::Axis {
                device: DeviceId(1),
                order: EventOrder(order),
                delta: Vector { x: 0.0, y: -40.0 },
                discrete: None,
                position: Some(position),
            })
        };
        scenario.host_mut().handle_input(&axis(1, inside), None);
        let scrolled_y = scenario
            .semantic_nodes()
            .into_iter()
            .find(|node| node.id.as_str().contains("application-00"))
            .unwrap()
            .bounds
            .origin
            .y;
        let outside = nickel_input::Point { x: 2.0, y: 2.0 };
        let outside_outcome = scenario.host_mut().handle_input(&axis(2, outside), None);
        assert!(!outside_outcome.changed);
        let unchanged_y = scenario
            .semantic_nodes()
            .into_iter()
            .find(|node| node.id.as_str().contains("application-00"))
            .unwrap()
            .bounds
            .origin
            .y;
        assert_eq!(
            unchanged_y, scrolled_y,
            "outside input reached the menu viewport"
        );

        let mut replacement = Launcher::new(vec![Application::new(
            "application-00".into(),
            "Application 00".into(),
            None,
            None,
            None,
        )]);
        replacement.open_search();
        scenario
            .host_mut()
            .application_mut()
            .sync(&replacement, palette(), None);
        scenario.host_mut().step(HostBatch {
            events: vec![HostEvent::Poll],
            ..HostBatch::default()
        });
        let only = scenario
            .semantic_nodes()
            .into_iter()
            .find(|node| node.id.as_str().contains("application-00"))
            .expect("filtered result remains reachable");
        assert!(
            only.bounds.origin.y >= scroll.bounds.origin.y,
            "item={:?} viewport={:?}",
            only.bounds,
            scroll.bounds
        );
        assert!(
            only.bounds.origin.y + only.bounds.size.height
                <= scroll.bounds.origin.y + scroll.bounds.size.height
        );
    }

    #[test]
    fn production_launcher_host_keeps_preedit_transient_and_commits_once() {
        let mut host = launcher_host();
        let search = host
            .query_unique(&nickel_ui::SemanticSelector::Role(
                nickel_ui::SemanticRole::TextField,
            ))
            .expect("launcher search text field");
        assert!(host.request_focus(search.id).changed);
        host.application_mut().take_effects();
        let committed_before = host.application().launcher.query().to_owned();
        let commands_before = host.commands().to_vec();

        let preedit = host.handle_input(
            &InputEvent::Text(TextEvent::Preedit {
                device: DeviceId(7),
                order: EventOrder(1),
                text: "にほ".into(),
                selection: Some((6, 6)),
            }),
            None,
        );
        assert!(preedit.changed);
        assert_eq!(host.application().launcher.query(), committed_before);
        assert!(host.application_mut().take_effects().is_empty());
        assert_ne!(host.commands(), commands_before);

        let commit = host.handle_input(
            &InputEvent::Text(TextEvent::Commit {
                device: DeviceId(7),
                order: EventOrder(2),
                text: "日本".into(),
            }),
            None,
        );
        assert!(commit.changed);
        assert_eq!(host.application().launcher.query(), "日本");
        assert_eq!(
            host.application_mut().take_effects(),
            [LauncherAction::SetQuery("日本".into())]
        );
        let focused = host
            .inspect()
            .keyboard_focus
            .clone()
            .expect("search focus survives dashboard-to-search reconstruction");
        assert!(focused.as_str().contains("launcher-search-focus"));

        let second_commit = host.handle_input(
            &InputEvent::Text(TextEvent::Commit {
                device: DeviceId(7),
                order: EventOrder(3),
                text: "語".into(),
            }),
            None,
        );
        assert!(second_commit.changed);
        assert_eq!(host.application().launcher.query(), "日本語");
        assert_eq!(
            host.application_mut().take_effects(),
            [LauncherAction::SetQuery("日本語".into())]
        );
    }

    #[test]
    fn logical_launcher_search_semantics_survive_supported_scale_variants() {
        let mut baseline = None;
        for scale in [1.0, 1.25, 2.0] {
            let mut launcher = Launcher::default();
            launcher.set_query("fire");
            let mut host = UiHost::new(
                LauncherApplication::new(
                    launcher,
                    LauncherViewState::default(),
                    LauncherIconCache::new(),
                    palette(),
                ),
                920,
                680,
            );
            host.step(HostBatch {
                surface_size: Some((920, 680)),
                scale_factor: Some(scale),
                events: vec![HostEvent::Poll],
                ..HostBatch::default()
            });
            assert_eq!(host.inspect().scale_factor, scale);
            let ids = host
                .semantic_nodes()
                .iter()
                .map(|node| node.id.clone())
                .collect::<Vec<_>>();
            if let Some(expected) = &baseline {
                assert_eq!(&ids, expected, "semantic drift at scale {scale}");
            } else {
                baseline = Some(ids);
            }
            assert!(
                !host
                    .semantic_targets_for_message(&LauncherAction::ActivateResult(0))
                    .is_empty()
            );
        }
    }

    #[test]
    fn launcher_escape_clears_search_then_requests_boundary_dismissal() {
        let mut host = launcher_host();
        host.application_mut()
            .update(LauncherAction::SetQuery("fire".into()));
        host.application_mut().take_effects();

        assert!(host.shortcut(Shortcut::Escape));
        assert_eq!(host.application().launcher.query(), "");
        assert_eq!(
            host.application_mut().take_effects(),
            [LauncherAction::SetQuery(String::new())]
        );
        assert!(host.shortcut(Shortcut::Escape));
        assert_eq!(
            host.application_mut().take_effects(),
            [LauncherAction::Dismiss]
        );
    }

    #[test]
    fn controller_scenario_restores_dashboard_after_search_and_launches_selected_app() {
        let mut scenario = launcher_scenario();
        scenario.controller(ControllerAction::Down).unwrap();
        scenario.controller(ControllerAction::Down).unwrap();
        let dashboard_target = controller_target(&scenario);

        let search = Selector::Role(nickel_ui::SemanticRole::TextField);
        scenario
            .semantic_operation(
                &search,
                SemanticAction::SetValue(SemanticValueInput::Text("fire".into())),
            )
            .unwrap();
        assert!(
            scenario
                .semantic_nodes()
                .iter()
                .any(|node| node.id.as_str().contains("launcher-search-results/firefox"))
        );
        scenario
            .semantic_operation(
                &search,
                SemanticAction::SetValue(SemanticValueInput::Text("".into())),
            )
            .unwrap();
        assert!(
            scenario
                .semantic_nodes()
                .iter()
                .any(|node| node.id.as_str() == dashboard_target)
        );

        scenario
            .controller_semantic_action(
                &Selector::keyed_item("launcher-applications", "firefox"),
                ActionKind::Activate,
            )
            .unwrap();
        assert_eq!(
            scenario.host_mut().application_mut().take_effects(),
            [
                LauncherAction::SetQuery("fire".into()),
                LauncherAction::SetQuery("".into()),
                LauncherAction::LaunchApplication("firefox".into())
            ]
        );
    }

    #[test]
    fn controller_scenario_cancel_closes_nested_menu_before_launcher_boundary() {
        let mut scenario = launcher_scenario();
        scenario
            .controller_semantic_action(
                &Selector::keyed_item("launcher-applications", "firefox"),
                ActionKind::ContextMenu,
            )
            .unwrap();
        assert!(scenario.host().inspect().open_overlay.is_some());

        scenario.controller(ControllerAction::Cancel).unwrap();
        assert!(scenario.host().inspect().open_overlay.is_none());
        scenario.controller(ControllerAction::Cancel).unwrap();
        assert!(scenario.host().inspect().open_overlay.is_none());
    }

    #[test]
    fn production_host_exposes_launcher_navigation_semantics() {
        let host = launcher_host();
        for action in [
            LauncherAction::SetView(LauncherView::Applications),
            LauncherAction::LaunchApplication("firefox".into()),
        ] {
            assert!(
                !host.semantic_targets_for_message(&action).is_empty(),
                "missing production semantic target for {action:?}"
            );
        }
    }

    #[test]
    fn dashboard_sidebar_launches_places_and_only_three_recent_projects() {
        let mut launcher = Launcher::default();
        launcher.set_places(vec![Application::new(
            "place:/home/test/Documents".into(),
            "Documents".into(),
            Some("/home/test/Documents".into()),
            None,
            Some(vec!["nickel-file".into(), "/home/test/Documents".into()]),
        )]);
        launcher.set_codex_available(true);
        launcher.set_dashboard_projects(crate::launcher::DashboardSection::Ready(
            (0..5)
                .map(|index| crate::launcher::DashboardProject {
                    id: format!("project-{index}"),
                    name: format!("Project {index}"),
                    roots: Vec::new(),
                    chat_count: None,
                    activity: crate::launcher::ProjectActivity::Unknown,
                    last_used_at: Some(index),
                })
                .collect(),
        ));
        let host = UiHost::new(
            LauncherApplication::new(
                launcher,
                LauncherViewState::default(),
                LauncherIconCache::new(),
                palette(),
            ),
            920,
            680,
        );
        assert_eq!(
            host.semantic_targets_for_message(&LauncherAction::LaunchApplication(
                "place:/home/test/Documents".into(),
            ))
            .len(),
            1,
        );
        for index in 2..5 {
            assert_eq!(
                host.semantic_targets_for_message(&LauncherAction::OpenProject(format!(
                    "project-{index}"
                )))
                .len(),
                1,
            );
        }
        for index in 0..2 {
            assert!(
                host.semantic_targets_for_message(&LauncherAction::OpenProject(format!(
                    "project-{index}"
                )))
                .is_empty()
            );
        }
        assert_eq!(
            host.semantic_targets_for_message(&LauncherAction::SeeAllProjects)
                .len(),
            1,
        );
    }

    #[test]
    fn narrow_dashboard_can_open_the_application_pane() {
        let mut host = UiHost::new(
            LauncherApplication::new(
                Launcher::default(),
                LauncherViewState::default(),
                LauncherIconCache::new(),
                palette(),
            ),
            560,
            680,
        );
        let target = host
            .unique_semantic_target_for_message(&LauncherAction::SetView(
                LauncherView::Applications,
            ))
            .expect("narrow sidebar exposes all applications");
        host.perform_semantic_action(target.id, SemanticAction::Invoke(ActionKind::Activate));
        assert_eq!(
            host.application().launcher.view(),
            LauncherView::Applications
        );
        assert!(
            !host
                .semantic_targets_for_message(&LauncherAction::LaunchApplication("firefox".into(),))
                .is_empty()
        );
    }

    #[test]
    fn launcher_icon_removes_only_transparent_outer_padding() {
        let mut image = RgbaImage::new(12, 12);
        for y in 3..9 {
            for x in 2..10 {
                image.put_pixel(x, y, image::Rgba([20, 40, 60, 255]));
            }
        }
        let trimmed = normalize_launcher_icon(image);
        assert_eq!((trimmed.width(), trimmed.height()), (8, 6));
        assert_eq!(trimmed.get_pixel(0, 0).0, [20, 40, 60, 255]);
    }

    #[test]
    fn dashboard_uses_up_to_four_rows_of_three_applications() {
        let launcher = Launcher::new(
            (0..15)
                .map(|index| {
                    Application::new(
                        format!("application-{index:02}"),
                        format!("Application {index:02}"),
                        None,
                        None,
                        None,
                    )
                })
                .collect(),
        );
        assert_eq!(dashboard_applications(&launcher).len(), 12);
        let host = UiHost::new(
            LauncherApplication::new(
                launcher,
                LauncherViewState::default(),
                LauncherIconCache::new(),
                palette(),
            ),
            920,
            680,
        );
        let bounds = (0..12)
            .map(|index| {
                host.unique_semantic_target_for_message(&LauncherAction::LaunchApplication(
                    format!("application-{index:02}"),
                ))
                .unwrap()
                .bounds
            })
            .collect::<Vec<_>>();
        for row in 0..4 {
            assert!((bounds[row * 3].origin.y - bounds[row * 3 + 2].origin.y).abs() < 0.01);
        }
        assert!(bounds[3].origin.y > bounds[0].origin.y);
        assert!(bounds[9].origin.y > bounds[6].origin.y);
    }

    #[test]
    fn long_application_name_keeps_dashboard_grid_cells_bounded() {
        let tile_width = |name: &str| {
            let mut launcher = Launcher::new(vec![Application::new(
                "long-name-test".into(),
                name.into(),
                None,
                None,
                None,
            )]);
            launcher.set_view(LauncherView::Applications);
            let host = UiHost::new(
                LauncherApplication::new(
                    launcher,
                    LauncherViewState::default(),
                    LauncherIconCache::new(),
                    palette(),
                ),
                920,
                680,
            );
            host.unique_semantic_target_for_message(&LauncherAction::LaunchApplication(
                "long-name-test".into(),
            ))
            .unwrap()
            .bounds
            .size
            .width
        };
        let short = tile_width("Game");
        let long =
            tile_width("Heroes of Might and Magic III: Shadows of Amn and an even longer subtitle");
        assert!(short < long, "short={short}, long={long}");
        assert!(long <= 142.0, "long={long}");
    }

    #[test]
    fn settings_action_is_labeled_at_dashboard_bottom_right() {
        let host = UiHost::new(
            LauncherApplication::new(
                Launcher::default(),
                LauncherViewState::default(),
                LauncherIconCache::new(),
                palette(),
            ),
            920,
            680,
        );
        let settings = host
            .unique_semantic_target_for_message(&LauncherAction::OpenSettings(
                SettingsDestination::Nickel,
            ))
            .unwrap();
        let account = host
            .unique_semantic_target_for_message(&LauncherAction::OpenAccount)
            .unwrap();
        assert_eq!(settings.name.as_deref(), Some("Settings"));
        assert!(settings.bounds.origin.x > account.bounds.origin.x);
        assert!(settings.bounds.origin.y >= account.bounds.origin.y);
    }

    #[test]
    fn all_applications_action_opens_the_complete_scrollable_list() {
        let mut scenario = populated_launcher_scenario();
        let pinned = scenario
            .host()
            .unique_semantic_target_for_message(&LauncherAction::SetView(LauncherView::Favorites))
            .unwrap();
        scenario.pointer_activate(&Selector::id(pinned.id)).unwrap();
        scenario.host_mut().application_mut().take_effects();
        let applications = scenario
            .host()
            .unique_semantic_target_for_message(&LauncherAction::SetView(
                LauncherView::Applications,
            ))
            .unwrap();
        assert!(applications.bounds.size.height >= 44.0);
        scenario
            .pointer_activate(&Selector::id(applications.id))
            .unwrap();
        assert_eq!(
            scenario.host().application().launcher.view(),
            LauncherView::Applications
        );
        assert_eq!(
            scenario.host_mut().application_mut().take_effects(),
            [LauncherAction::SetView(LauncherView::Applications)]
        );
        assert!(scenario.semantic_nodes().iter().any(|node| {
            node.id
                .as_str()
                .ends_with("launcher-applications/application-29")
        }));
    }

    #[test]
    fn semantic_navigation_runs_the_application_update_path() {
        let mut host = launcher_host();
        let action = LauncherAction::SetView(LauncherView::Applications);
        let target = host
            .unique_semantic_target_for_message(&action)
            .expect("Applications navigation must be unique");
        let outcome =
            host.perform_semantic_action(target.id, SemanticAction::Invoke(ActionKind::Activate));
        assert!(outcome.semantic_failures.is_empty());
        assert_eq!(host.application_mut().take_effects(), [action]);
    }

    #[test]
    fn context_action_presents_application_menu_through_the_host() {
        let mut host = launcher_host();
        let action = LauncherAction::LaunchApplication("firefox".into());
        let target = host
            .semantic_targets_for_message(&action)
            .into_iter()
            .next()
            .expect("default launcher must expose Firefox");
        let outcome = host
            .perform_semantic_action(target.id, SemanticAction::Invoke(ActionKind::ContextMenu));
        assert!(outcome.semantic_failures.is_empty());
        assert!(
            !host
                .semantic_targets_for_message(&LauncherAction::TogglePin("firefox".into()))
                .is_empty(),
            "overlay failures: {:?}; labels: {:?}",
            host.inspect().overlay_failures,
            host.accessibility_nodes()
                .iter()
                .filter_map(|node| node.label.as_deref())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn launcher_context_entry_routes_dispatch_the_identical_typed_launch_action() {
        for route in ["pointer", "keyboard", "controller", "accessibility"] {
            let mut host = launcher_host();
            let target = host
                .unique_semantic_target_for_message(&LauncherAction::LaunchApplication(
                    "firefox".into(),
                ))
                .expect("unique Firefox launcher presentation");
            let target_id = target.id.clone();
            let center = Point {
                x: target.bounds.origin.x + target.bounds.size.width / 2.0,
                y: target.bounds.origin.y + target.bounds.size.height / 2.0,
            };
            match route {
                "pointer" => {
                    host.step(HostBatch {
                        events: vec![HostEvent::Ui(UiEvent::PointerContext(center))],
                        ..HostBatch::default()
                    });
                }
                "keyboard" => {
                    host.step(HostBatch {
                        events: vec![
                            HostEvent::Ui(UiEvent::AccessibilityFocus(target_id.clone())),
                            HostEvent::Ui(UiEvent::KeyboardContextMenu),
                        ],
                        ..HostBatch::default()
                    });
                }
                "controller" => {
                    host.step(HostBatch {
                        events: vec![HostEvent::ControllerSemantic {
                            target: target_id,
                            action: SemanticAction::Invoke(ActionKind::ContextMenu),
                        }],
                        ..HostBatch::default()
                    });
                }
                "accessibility" => {
                    host.step(HostBatch {
                        events: vec![HostEvent::Accessibility {
                            target: target_id,
                            action: SemanticAction::Invoke(ActionKind::ContextMenu),
                        }],
                        ..HostBatch::default()
                    });
                }
                _ => unreachable!(),
            }
            assert!(
                host.inspect().open_overlay.is_some(),
                "{route} did not open the shared launcher menu: {:?}",
                host.inspect().overlay_failures
            );
            let launch = host
                .accessibility_nodes()
                .iter()
                .find(|node| {
                    node.semantic_role == Some(nickel_ui::SemanticRole::MenuItem)
                        && node.label.as_deref() == Some("Launch")
                })
                .expect("open menu exposes its typed Launch item")
                .id
                .clone();
            host.perform_semantic_action(launch, SemanticAction::Invoke(ActionKind::Activate));
            assert_eq!(
                host.application_mut().take_effects(),
                [LauncherAction::LaunchApplication("firefox".into())],
                "{route} must converge on the same typed action"
            );
        }
    }

    #[test]
    fn controller_legend_tracks_search_and_modal_menu_semantics() {
        let mut host = launcher_host();
        host.application_mut()
            .set_controller_family(ControllerFamily::PlayStation);
        host.application_mut()
            .update(LauncherAction::SetQuery("f".into()));
        host.step(HostBatch {
            events: vec![
                HostEvent::Poll,
                HostEvent::Controller(ControllerAction::Down),
                HostEvent::Controller(ControllerAction::Down),
                HostEvent::Controller(ControllerAction::Right),
            ],
            ..HostBatch::default()
        });
        let search_labels = accessibility_labels(&host);
        assert!(
            search_labels.contains(&"Cross: Open".to_owned()),
            "search labels: {search_labels:?}"
        );
        assert!(search_labels.contains(&"Circle: Close".to_owned()));

        let target = host
            .semantic_targets_for_message(&LauncherAction::ActivateResult(0))
            .into_iter()
            .next()
            .expect("selected search result");
        host.perform_semantic_action(target.id, SemanticAction::Invoke(ActionKind::ContextMenu));
        let menu_labels = accessibility_labels(&host);
        assert!(
            host.inspect().open_overlay.is_some(),
            "application menu must own the modal scope"
        );
        assert_eq!(
            host.inspect().available_semantic_actions,
            [ActionKind::Activate]
        );
        assert!(menu_labels.contains(&"Launch".to_owned()));
        assert!(!menu_labels.iter().any(|label| label == "Options: Actions"));
        assert!(!menu_labels.iter().any(|label| label.contains("Sidebar")));
        assert!(
            !menu_labels
                .iter()
                .any(|label| label.starts_with("Cross:") || label.starts_with("Circle:")),
            "modal accessibility must not expose the obscured base legend: {menu_labels:?}"
        );
    }

    #[test]
    fn controller_legend_omits_context_menu_when_selected_target_lacks_it() {
        let mut host = launcher_host();
        host.step(HostBatch {
            surface_size: Some((480, 680)),
            events: vec![HostEvent::Controller(ControllerAction::Down)],
            ..HostBatch::default()
        });
        let labels = host
            .accessibility_nodes()
            .iter()
            .filter_map(|node| node.label.as_deref())
            .collect::<Vec<_>>();
        assert!(
            !labels.iter().any(|label| label.contains("menu control")),
            "labels: {labels:?}"
        );
        assert!(host.accessibility_nodes().iter().all(|node| {
            node.rect.origin.x >= 0.0 && node.rect.origin.x + node.rect.size.width <= 480.0
        }));
    }

    #[test]
    fn rebuilding_the_host_keeps_stable_image_resources_and_semantics() {
        let first = launcher_host();
        let second = launcher_host();
        assert_eq!(first.commands(), second.commands());
        assert_eq!(first.semantic_nodes(), second.semantic_nodes());
    }

    #[test]
    fn visible_status_stays_inside_the_compact_launcher_footer() {
        let mut host = launcher_host();
        let launcher = Launcher::default();
        host.application_mut().sync(
            &launcher,
            palette(),
            Some("Some application entries could not be discovered.".into()),
        );
        host.step(nickel_ui::HostBatch {
            events: vec![nickel_ui::HostEvent::Poll],
            ..nickel_ui::HostBatch::default()
        });

        let status = host
            .accessibility_nodes()
            .iter()
            .find(|node| node.label.as_deref() == Some("Launcher status"))
            .expect("launcher status remains accessible");
        assert!(status.rect.origin.y >= 0.0);
        assert!(status.rect.origin.y + status.rect.size.height <= 680.0);
    }

    #[test]
    fn locale_direction_is_structural_and_deterministic() {
        assert_eq!(
            reading_direction_for_locale("ar_EG.UTF-8"),
            ReadingDirection::RightToLeft
        );
        assert_eq!(
            reading_direction_for_locale("en_US.UTF-8"),
            ReadingDirection::LeftToRight
        );
    }

    #[test]
    fn icon_cache_is_bounded() {
        let mut cache = LauncherIconCache::new();
        for index in 0..(LAUNCHER_ICON_CACHE_CAPACITY + 8) {
            let application = Application::new(
                format!("application-{index}"),
                format!("Application {index}"),
                None,
                None,
                None,
            );
            let _ = cache.resolve(&application);
        }
        let diagnostics = cache.diagnostics();
        assert!(diagnostics.entries <= diagnostics.capacity);
        assert!(diagnostics.retained_pixel_bytes <= diagnostics.byte_capacity);
        assert!(diagnostics.evictions > 0);
    }

    #[test]
    fn application_icon_decode_completes_after_the_render_path_returns() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("application.png");
        RgbaImage::from_pixel(256, 256, image::Rgba([20, 80, 220, 255]))
            .save(&path)
            .unwrap();
        let application = Application::new(
            "application".into(),
            "Application".into(),
            None,
            Some(path),
            None,
        );
        let mut cache = LauncherIconCache::new();

        assert!(
            cache.resolve(&application).is_none(),
            "the render path must enqueue decoding instead of doing it inline"
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        let (_, icon) = loop {
            if let Some(icon) = cache.resolve(&application) {
                break icon;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "background icon decode did not complete"
            );
            std::thread::yield_now();
        };

        assert_eq!(
            icon.dimensions(),
            (LAUNCHER_ICON_MAX_SIDE, LAUNCHER_ICON_MAX_SIDE)
        );
    }

    #[test]
    fn native_window_icons_receive_distinct_stable_renderer_ids() {
        let mut cache = LauncherIconCache::new();
        let blue = Arc::new(RgbaImage::from_pixel(
            32,
            32,
            image::Rgba([20, 80, 220, 255]),
        ));
        let red = Arc::new(RgbaImage::from_pixel(
            32,
            32,
            image::Rgba([220, 60, 50, 255]),
        ));

        let (blue_id, _) = cache.resolve_window_icon(crate::model::WindowId(10), Arc::clone(&blue));
        let (red_id, _) = cache.resolve_window_icon(crate::model::WindowId(20), red);
        let (blue_again_id, _) = cache.resolve_window_icon(crate::model::WindowId(10), blue);

        assert_ne!(blue_id, red_id);
        assert_eq!(blue_id, blue_again_id);
    }
}
