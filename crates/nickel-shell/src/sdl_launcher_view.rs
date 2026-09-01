//! SDL-independent launcher scene and hit testing.
//!
//! `LiveShell` owns input and mutates [`Launcher`]. This module turns that state
//! into component paint commands and stable semantic actions.

use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    sync::{Arc, OnceLock},
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
    Application as UiApplication, Collection, CollectionPresentation, CollectionState, Column,
    ComponentBuilderExt, Container, ControllerFamily, FallbackAvatar, FrameOverlay, Image,
    InputModality, Insets, LauncherSearchField, NavigationScope, NavigationTraversal,
    OverlayAnchor, OverlayMenu, OverlayMenuItem, OverlayStyle, ProjectStatusRow, ReadingDirection,
    Row, START_MENU_SINGLE_PANE_BREAKPOINT, SectionHeader, SemanticControllerAction, SemanticTheme,
    Shortcut, ShortcutRow, ShortcutState, StartMenuNarrowPane, StartMenuShell, Text, TextAlign,
    UiId, VerticalScroll, ViewContext,
};

const PANEL_MAX_WIDTH: f32 = 920.0;
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
                .take(8)
                .collect::<Vec<_>>();
            if home.is_empty() {
                (0..launcher.result_count())
                    .filter_map(|index| launcher.result_at(index))
                    .take(8)
                    .collect()
            } else {
                home
            }
        }
        LauncherView::Applications | LauncherView::Places => (0..launcher.result_count())
            .filter_map(|index| launcher.result_at(index))
            .take(24)
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
}

impl LauncherApplication {
    pub fn new(
        launcher: Launcher,
        state: LauncherViewState,
        icons: LauncherIconCache,
        palette: ThemePalette,
    ) -> Self {
        Self {
            launcher,
            state: RefCell::new(state),
            icons: RefCell::new(icons),
            palette,
            status: None,
            effects: Vec::new(),
            dirty: false,
            reading_direction: None,
        }
    }

    pub fn take_effects(&mut self) -> Vec<LauncherAction> {
        std::mem::take(&mut self.effects)
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
        AnyView::new(Container::new().width(width).height(height).child(base))
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
                        "Unpin"
                    } else {
                        "Pin"
                    },
                    LauncherAction::TogglePin(id.clone()),
                ));
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
        std::mem::take(&mut self.dirty)
    }

    fn shortcut(&mut self, shortcut: Shortcut) -> bool {
        match shortcut {
            Shortcut::Submit if self.launcher.mode() == LauncherMode::Search => {
                if self.launcher.result_count() == 0 {
                    return false;
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
        }
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
        LauncherAction::SearchScroll => None,
        LauncherAction::Dismiss => Some(LauncherShellEffect::Dismiss),
    }
}

/// Bounded CPU image cache with stable renderer resource IDs.
#[derive(Clone)]
pub struct LauncherIconCache {
    icons: HashMap<String, CachedIcon>,
    insertion_order: VecDeque<String>,
    next_id: u16,
    evictions: u64,
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
}

impl LauncherIconCache {
    pub fn new() -> Self {
        Self {
            icons: HashMap::new(),
            insertion_order: VecDeque::new(),
            // Keep launcher IDs away from wallpaper and panel fixed IDs.
            next_id: 0x4000,
            evictions: 0,
        }
    }

    pub fn diagnostics(&self) -> LauncherIconCacheDiagnostics {
        LauncherIconCacheDiagnostics {
            entries: self.icons.len(),
            capacity: LAUNCHER_ICON_CACHE_CAPACITY,
            retained_pixel_bytes: self
                .icons
                .values()
                .filter_map(|cached| cached.image.as_ref())
                .map(|image| image.as_raw().len())
                .sum(),
            byte_capacity: LAUNCHER_ICON_CACHE_MAX_BYTES,
            evictions: self.evictions,
        }
    }

    pub fn begin_visual_generation(&mut self) {
        self.icons.retain(|key, _| !key.starts_with("structural:"));
        self.insertion_order
            .retain(|key| !key.starts_with("structural:"));
    }

    fn insert(&mut self, key: String, cached: CachedIcon) {
        let evictions_before = self.evictions;
        while self.icons.len() >= LAUNCHER_ICON_CACHE_CAPACITY {
            let Some(oldest) = self.insertion_order.pop_front() else {
                break;
            };
            if self.icons.remove(&oldest).is_some() {
                self.evictions = self.evictions.saturating_add(1);
            }
        }
        self.insertion_order.push_back(key.clone());
        self.icons.insert(key, cached);
        if self.evictions != evictions_before {
            let diagnostics = self.diagnostics();
            tracing::debug!(
                entries = diagnostics.entries,
                capacity = diagnostics.capacity,
                evictions = diagnostics.evictions,
                "launcher icon cache evicted its oldest entry"
            );
        }
    }

    pub(crate) fn resolve(&mut self, application: &Application) -> Option<(u16, Arc<RgbaImage>)> {
        if let Some(cached) = self.icons.get(application.id()) {
            return cached
                .image
                .as_ref()
                .map(|image| (cached.id, Arc::clone(image)));
        }
        let image = application
            .icon_path()
            .and_then(icons::load)
            .or_else(|| application.icon().and_then(platform::application_icon))
            .filter(has_visible_pixel)
            .map(normalize_launcher_icon)
            .map(Arc::new);
        let id = self.next_id;
        self.next_id = self.next_id.checked_add(1).unwrap_or(0x4000);
        self.insert(
            application.id().to_owned(),
            CachedIcon {
                id,
                image: image.clone(),
            },
        );
        image.map(|image| (id, image))
    }

    fn structural(
        &mut self,
        name: &str,
        bytes: &[u8],
        color: u32,
    ) -> Option<(u16, Arc<RgbaImage>)> {
        let key = format!("structural:{name}:{color:06x}");
        if let Some(cached) = self.icons.get(&key) {
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
        let id = self.next_id;
        self.next_id = self.next_id.checked_add(1).unwrap_or(0x4000);
        self.insert(
            key,
            CachedIcon {
                id,
                image: image.clone(),
            },
        );
        image.map(|image| (id, image))
    }
}

fn normalize_launcher_icon(image: RgbaImage) -> RgbaImage {
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
    let result_indices = Arc::new(
        cards
            .iter()
            .map(|(index, id, _, _)| (id.clone(), *index))
            .collect::<HashMap<_, _>>(),
    );
    let activation_indices = Arc::clone(&result_indices);
    let collection = Collection::try_new(
        CollectionState::Ready(cards),
        |(_, id, _, _)| id.clone(),
        move |(_, _, name, icon)| {
            Container::new()
                .min_height(TILE_HEIGHT)
                .padding(Insets::all(theme.spacing.control))
                .radius(theme.radii.card)
                .background(theme.surfaces.raised)
                .border(theme.borders.ordinary, theme.sizing.border)
                .align_items(nickel_ui::Align::Center)
                .child(
                    Image::new(icon.0, icon.1)
                        .width(ICON_SIZE)
                        .height(ICON_SIZE),
                )
                .child(
                    Text::new(name)
                        .color(theme.text.primary)
                        .align(TextAlign::Center)
                        .max_lines(2)
                        .ellipsis(true),
                )
        },
    )
    .expect("launcher result indices must be unique")
    .id("launcher-search-results")
    .presentation(CollectionPresentation::UniformGrid { columns })
    .gap(GRID_GAP)
    .item_focus_border(theme.borders.focus)
    .item_controller_focus_border(theme.borders.controller_focus)
    .navigation_scope(NavigationScope::group().traversal(NavigationTraversal::Grid))
    .controller_scope_background(theme.surfaces.selected)
    .on_activate(move |id| {
        LauncherAction::ActivateResult(
            *activation_indices
                .get(id)
                .expect("collection key must retain its result index"),
        )
    });
    let result_content = if launcher.result_count() == 0 {
        AnyView::new(Text::new("No matching applications").color(theme.text.secondary))
    } else {
        AnyView::new(
            VerticalScroll::new(LauncherAction::SearchScroll, 0.0)
                .id("launcher-search-scroll")
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

fn build_dashboard_view_directional(
    launcher: &Launcher,
    state: &LauncherViewState,
    icons: &mut LauncherIconCache,
    context: LauncherViewContext<'_>,
    direction: ReadingDirection,
) -> AnyView<LauncherAction> {
    let viewport = context.viewport;
    let (viewport_width, _viewport_height) = viewport;
    let width = PANEL_MAX_WIDTH.min(viewport_width.max(1) as f32);
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
    let narrow = width < START_MENU_SINGLE_PANE_BREAKPOINT;
    let selected = |view| launcher.view() == view;
    let nav_state = |view| ShortcutState {
        selected: selected(view),
        focused: selected(view),
        ..ShortcutState::default()
    };
    let nav_icon = |icons: &mut LauncherIconCache, name, bytes: &[u8]| {
        structural_icon(icons, name, bytes, theme.text.secondary)
    };

    let mut sidebar = Column::new()
        .gap(theme.spacing.compact)
        .child(
            Row::new()
                .align_items(nickel_ui::Align::Center)
                .gap(theme.spacing.control)
                .child(
                    structural_icon(
                        icons,
                        "nickel",
                        include_bytes!("../../../assets/icons/start-menu/nickel.svg"),
                        theme.text.accent,
                    )
                    .width(30.0)
                    .height(30.0),
                )
                .child(Text::new("Nickel").color(theme.text.primary).scale(1.25)),
        )
        .child(ShortcutRow::new_directional(
            theme,
            nav_icon(
                icons,
                "home",
                include_bytes!("../../../assets/icons/start-menu/applications.svg"),
            ),
            "Home",
            "Favorites and recent applications",
            Some(LauncherAction::SetView(LauncherView::Favorites)),
            nav_state(LauncherView::Favorites),
            direction,
        ))
        .child(ShortcutRow::new_directional(
            theme,
            nav_icon(
                icons,
                "applications",
                include_bytes!("../../../assets/icons/start-menu/applications.svg"),
            ),
            "Applications",
            "Browse installed applications",
            Some(LauncherAction::SetView(LauncherView::Applications)),
            nav_state(LauncherView::Applications),
            direction,
        ))
        .child(ShortcutRow::new_directional(
            theme,
            nav_icon(
                icons,
                "places",
                include_bytes!("../../../assets/icons/start-menu/project.svg"),
            ),
            "Places",
            "Open files and locations",
            Some(LauncherAction::SetView(LauncherView::Places)),
            nav_state(LauncherView::Places),
            direction,
        ));
    sidebar = sidebar
        .child(nickel_ui::Spacer::flex())
        .child(ShortcutRow::new_directional(
            theme,
            nav_icon(
                icons,
                "settings",
                include_bytes!("../../../assets/icons/start-menu/settings.svg"),
            ),
            "Settings",
            "Nickel appearance and behavior",
            Some(LauncherAction::OpenSettings(SettingsDestination::Nickel)),
            ShortcutState::default(),
            direction,
        ));

    let account = match launcher.dashboard_account() {
        crate::launcher::DashboardSection::Ready(account) => {
            AnyView::new(AccountSummaryRow::new_directional(
                theme,
                FallbackAvatar::new(theme, &account.display_name),
                &account.display_name,
                &account.supporting_text,
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
            "Account details unavailable",
            Some(LauncherAction::OpenAccount),
            ShortcutState::default(),
            direction,
        )),
    };
    let sidebar_footer = Column::new()
        .gap(theme.spacing.compact)
        .child(account.id("launcher-account"));

    let application_cards = dashboard_applications(launcher)
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
    let application_collection = Collection::try_new(
        CollectionState::Ready(application_cards),
        |(id, _, _)| id.clone(),
        move |(_, name, icon)| {
            Container::new()
                .min_height(104.0)
                .padding(Insets::all(theme.spacing.control))
                .gap(theme.spacing.compact)
                .radius(theme.radii.card)
                .background(theme.surfaces.raised)
                .border(theme.borders.ordinary, theme.sizing.border)
                .align_items(nickel_ui::Align::Center)
                .child(Image::new(icon.0, icon.1).width(48.0).height(48.0))
                .child(
                    Text::new(name)
                        .color(theme.text.primary)
                        .align(TextAlign::Center)
                        .max_lines(2)
                        .ellipsis(true),
                )
        },
    )
    .expect("launcher application ids must be unique")
    .id("launcher-applications")
    .presentation(CollectionPresentation::UniformGrid {
        columns: if width >= 820.0 { 4 } else { 3 },
    })
    .gap(theme.spacing.control)
    .item_focus_border(theme.borders.focus)
    .item_controller_focus_border(theme.borders.controller_focus)
    .navigation_scope(NavigationScope::group().traversal(NavigationTraversal::Grid))
    .controller_scope_background(theme.surfaces.selected)
    .on_activate(|id| LauncherAction::LaunchApplication(id.clone()));

    let title = match launcher.view() {
        LauncherView::Favorites => "Home",
        LauncherView::Applications => "Applications",
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
    if applications_empty {
        detail = detail.child(
            Container::new()
                .fill_width()
                .padding(Insets::all(theme.spacing.content))
                .child(
                    Text::new(match launcher.view() {
                        LauncherView::Favorites => {
                            "Pin applications or launch them to populate Home."
                        }
                        LauncherView::Applications => "No installed applications are available.",
                        LauncherView::Places => "No places are available.",
                    })
                    .color(theme.text.secondary),
                ),
        );
    } else {
        detail = detail.child(application_collection);
    }
    if launcher.view() == LauncherView::Favorites && launcher.codex_available() {
        let project_rows = match launcher.dashboard_projects() {
            crate::launcher::DashboardSection::Loading => vec![AnyView::new(
                Text::new("Loading projects…").color(theme.text.secondary),
            )],
            crate::launcher::DashboardSection::Empty => vec![AnyView::new(
                Text::new("No recent projects").color(theme.text.secondary),
            )],
            crate::launcher::DashboardSection::Unavailable(reason) => {
                vec![AnyView::new(ProjectStatusRow::new_directional(
                    theme,
                    nav_icon(
                        icons,
                        "project",
                        include_bytes!("../../../assets/icons/start-menu/project.svg"),
                    ),
                    "Projects unavailable",
                    reason,
                    None,
                    None,
                    ShortcutState::default(),
                    direction,
                ))]
            }
            crate::launcher::DashboardSection::Failed {
                message,
                recoverable,
            } => vec![AnyView::new(ProjectStatusRow::new_directional(
                theme,
                nav_icon(
                    icons,
                    "project",
                    include_bytes!("../../../assets/icons/start-menu/project.svg"),
                ),
                if *recoverable {
                    "Projects temporarily unavailable"
                } else {
                    "Projects unavailable"
                },
                message,
                None,
                None,
                ShortcutState::default(),
                direction,
            ))],
            crate::launcher::DashboardSection::Ready(projects) => projects
                .iter()
                .take(2)
                .map(|project| {
                    let action = LauncherAction::OpenProject(project.id.clone());
                    AnyView::new(ProjectStatusRow::new_directional(
                        theme,
                        nav_icon(
                            icons,
                            "project",
                            include_bytes!("../../../assets/icons/start-menu/project.svg"),
                        ),
                        &project.name,
                        match project.activity {
                            crate::launcher::ProjectActivity::Active => "Active",
                            crate::launcher::ProjectActivity::Idle => "Idle",
                            crate::launcher::ProjectActivity::Unknown => "Status unknown",
                        },
                        project.chat_count,
                        Some(action),
                        ShortcutState::default(),
                        direction,
                    ))
                })
                .collect(),
        };
        detail = detail.child(
            Column::new()
                .gap(theme.spacing.compact)
                .child(
                    SectionHeader::new(theme, "Recent projects")
                        .action(theme, "See all", LauncherAction::SeeAllProjects)
                        .direction(direction),
                )
                .children(project_rows),
        );
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
    let mut shell = StartMenuShell::new(theme, width, sidebar, detail)
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
    if let Some(status) = context.status {
        shell = shell.detail_footer(launcher_status(theme, status));
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
        .child(Text::new(status).color(0xd98a32).wrap(true))
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
        ModifierState, NamedKey, PhysicalKey, TextEvent,
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
    fn controller_scenario_switches_peer_panes_and_contains_dpad() {
        let mut scenario = populated_launcher_scenario();

        scenario.controller(ControllerAction::Down).unwrap();
        let first_target = controller_target(&scenario);
        assert!(
            first_target.contains("launcher-search-focus"),
            "selected {first_target}"
        );
        scenario.controller(ControllerAction::PreviousPane).unwrap();
        let sidebar_home = controller_target(&scenario);
        assert!(sidebar_home.contains("start-menu-primary-pane"));

        scenario.controller(ControllerAction::NextPane).unwrap();
        assert_eq!(controller_target(&scenario), first_target);
        scenario.controller(ControllerAction::Down).unwrap();
        let content_home = controller_target(&scenario);
        assert!(
            content_home.contains("launcher-applications"),
            "selected {content_home}"
        );
        for _ in 0..5 {
            scenario.controller(ControllerAction::Down).unwrap();
        }
        let content_moved = controller_target(&scenario);
        assert!(
            content_moved.contains("launcher-applications"),
            "D-pad escaped content pane: {content_moved}"
        );

        scenario.controller(ControllerAction::PreviousPane).unwrap();
        assert_eq!(controller_target(&scenario), sidebar_home);
        scenario.controller(ControllerAction::NextPane).unwrap();
        assert_eq!(controller_target(&scenario), content_moved);
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
        for _ in 0..4 {
            host.application_mut().sync(&launcher, palette(), None);
            host.step(HostBatch {
                events: vec![HostEvent::Controller(ControllerAction::Down)],
                ..HostBatch::default()
            });
        }
        host.application_mut().sync(&launcher, palette(), None);
        host.step(HostBatch {
            events: vec![HostEvent::Controller(ControllerAction::Up)],
            ..HostBatch::default()
        });

        let target = host
            .inspect()
            .controller_target
            .expect("Up from Recent projects selects the application grid");
        assert!(
            target.as_str().ends_with("/launcher-applications"),
            "{target:?}"
        );
        let scope_background = launcher_semantic_theme(palette()).surfaces.selected;
        assert!(host.commands().iter().any(|command| matches!(
            command,
            nickel_ui::backend::PaintCommand::RoundedFill { color, .. }
                if *color == scope_background
        )));

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
        let bounds = host
            .accessibility_nodes()
            .iter()
            .find(|node| node.id == target)
            .expect("selected tile remains in accessibility tree")
            .rect;
        let focus = launcher_semantic_theme(palette()).borders.controller_focus;
        assert!(host.commands().iter().any(|command| {
            matches!(
                command,
                nickel_ui::backend::PaintCommand::Stroke { rect, color, width }
                    if *rect == bounds && *color == focus && *width >= 2.0
            )
        }));
        let labels = accessibility_labels(&host);
        assert!(labels.contains(&"confirm control: Open".to_owned()));
        assert!(labels.contains(&"menu control: Actions".to_owned()));
    }

    #[test]
    fn normalized_keyboard_enters_the_home_application_grid() {
        let launcher = Launcher::default();
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
        host.handle_input(
            &navigation_key(2, KeyCode::ArrowDown, NamedKey::ArrowDown),
            None,
        );
        assert!(
            host.inspect()
                .controller_target
                .as_ref()
                .is_some_and(|id| id.as_str().ends_with("/launcher-applications"))
        );
        host.handle_input(
            &navigation_key(3, KeyCode::ArrowRight, NamedKey::ArrowRight),
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
            "org.kde.konsole".into(),
            "Konsole".into(),
            None,
            None,
            None,
        )]);
        launcher.set_query("konsole");
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
        for _ in 0..40 {
            if controller_target(&scenario).contains("application-29") {
                break;
            }
            scenario.controller(ControllerAction::Down).unwrap();
        }
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
            LauncherAction::SetView(LauncherView::Favorites),
            LauncherAction::SetView(LauncherView::Applications),
            LauncherAction::SetView(LauncherView::Places),
        ] {
            assert!(
                !host.semantic_targets_for_message(&action).is_empty(),
                "missing production semantic target for {action:?}"
            );
        }
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
    fn controller_legend_tracks_search_selection_and_open_menu_semantics() {
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
        assert!(search_labels.contains(&"Options: Actions".to_owned()));
        assert!(search_labels.contains(&"Circle: Close".to_owned()));

        let target = host
            .semantic_targets_for_message(&LauncherAction::ActivateResult(0))
            .into_iter()
            .next()
            .expect("selected search result");
        host.perform_semantic_action(target.id, SemanticAction::Invoke(ActionKind::ContextMenu));
        let menu_labels = accessibility_labels(&host);
        assert!(
            menu_labels.contains(&"Cross: Select".to_owned()),
            "menu labels: {menu_labels:?}; inspection: {:?}",
            host.inspect()
        );
        assert!(menu_labels.contains(&"Circle: Back".to_owned()));
        assert!(!menu_labels.iter().any(|label| label == "Options: Actions"));
        assert!(!menu_labels.iter().any(|label| label.contains("Sidebar")));
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
    fn visible_status_does_not_shrink_the_launcher_surface() {
        let mut host = launcher_host();
        let launcher = Launcher::default();
        host.application_mut().sync(
            &launcher,
            palette(),
            Some("Some applications could not be loaded.".into()),
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
        assert!(status.rect.origin.y > 500.0);
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
}
