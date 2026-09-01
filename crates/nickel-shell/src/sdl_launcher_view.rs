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
use nickel_ui::{
    AccountSummaryRow, ActionLegend, ActionLegendEntry, AnyView, Application as UiApplication,
    Collection, CollectionPresentation, CollectionState, Column, ComponentBuilderExt, Container,
    ControllerFamily, FallbackAvatar, FrameOverlay, Image, InputModality, Insets,
    LauncherSearchField, OverlayAnchor, OverlayMenu, OverlayMenuItem, ProjectStatusRow,
    ReadingDirection, Row, START_MENU_SINGLE_PANE_BREAKPOINT, SectionHeader, SemanticColors,
    SemanticControllerAction, SemanticTheme, ShortcutRow, ShortcutState, StartMenuNarrowPane,
    StartMenuShell, Text, TextAlign, UiId, ViewContext,
};

const PANEL_MAX_WIDTH: f32 = 920.0;
const PANEL_MAX_HEIGHT: f32 = 680.0;
const SIDEBAR_WIDTH: f32 = 148.0;
const HEADER_HEIGHT: f32 = 72.0;
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
}

pub struct LauncherApplication {
    launcher: Launcher,
    state: RefCell<LauncherViewState>,
    icons: RefCell<LauncherIconCache>,
    palette: ThemePalette,
    status: Option<String>,
    effects: Vec<LauncherAction>,
    dirty: bool,
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
}

impl UiApplication for LauncherApplication {
    type Message = LauncherAction;

    fn update(&mut self, message: Self::Message) {
        let evidence = message.clone();
        let _ = reduce_launcher_action(&mut self.launcher, &mut self.state.borrow_mut(), message);
        self.effects.push(evidence);
    }

    fn view(&self, context: ViewContext) -> impl nickel_ui::View<Self::Message> {
        let base = build_launcher_view(
            &self.launcher,
            &self.state.borrow(),
            &mut self.icons.borrow_mut(),
            context.viewport.size.width.max(1.0) as u32,
            context.viewport.size.height.max(1.0) as u32,
            self.palette,
            context.modality,
        );
        let width = context.viewport.size.width;
        let height = context.viewport.size.height;
        let mut root = Container::new().width(width).height(height).child(base);
        if let Some(status) = &self.status {
            root = root.child(
                Column::new()
                    .width(width)
                    .height(height)
                    .child(nickel_ui::Spacer::vertical(72.0))
                    .child(
                        Row::new()
                            .height(28.0)
                            .child(nickel_ui::Spacer::fixed(250.0))
                            .child(
                                Text::new(status)
                                    .width((width - 280.0).max(1.0))
                                    .height(28.0)
                                    .scale(14.0)
                                    .color(0xd98a32),
                            ),
                    ),
            );
        }
        AnyView::new(root)
    }

    fn frame_overlays(&self, _context: ViewContext) -> Vec<FrameOverlay<Self::Message>> {
        let mut overlays = (0..self.launcher.result_count())
            .filter_map(|index| {
                self.launcher
                    .result_at(index)
                    .map(|application| (index, application))
            })
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
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DashboardNarrowPage {
    #[default]
    Primary,
    Projects,
}

#[derive(Clone, Debug, Default)]
pub struct LauncherViewState {
    pub scroll_row: usize,
    dashboard_scroll_row: usize,
    search_scroll_row: usize,
    pub dashboard_selected: usize,
    pub dashboard_narrow_page: DashboardNarrowPage,
    controller_family: ControllerFamily,
}

impl LauncherViewState {
    pub fn set_controller_family(&mut self, family: ControllerFamily) {
        self.controller_family = family;
    }

    pub fn transition_mode(&mut self, previous: LauncherMode, next: LauncherMode) {
        if previous == next {
            return;
        }
        match previous {
            LauncherMode::Dashboard => self.dashboard_scroll_row = self.scroll_row,
            LauncherMode::Search => self.search_scroll_row = self.scroll_row,
        }
        self.scroll_row = match next {
            LauncherMode::Dashboard => self.dashboard_scroll_row,
            LauncherMode::Search => self.search_scroll_row,
        };
    }

    pub fn reset_active_scroll(&mut self, mode: LauncherMode) {
        self.scroll_row = 0;
        match mode {
            LauncherMode::Dashboard => self.dashboard_scroll_row = 0,
            LauncherMode::Search => self.search_scroll_row = 0,
        }
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
            view.scroll_row = 0;
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
            let previous = launcher.mode();
            launcher.set_query(&query);
            view.transition_mode(previous, launcher.mode());
            view.reset_active_scroll(launcher.mode());
            None
        }
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

fn build_launcher_view(
    launcher: &Launcher,
    state: &LauncherViewState,
    icons: &mut LauncherIconCache,
    viewport_width: u32,
    viewport_height: u32,
    palette: ThemePalette,
    modality: InputModality,
) -> AnyView<LauncherAction> {
    if launcher.mode() == LauncherMode::Dashboard {
        return build_dashboard_view(
            launcher,
            state,
            icons,
            viewport_width,
            viewport_height,
            palette,
            modality,
        );
    }
    let panel_width = PANEL_MAX_WIDTH.min(viewport_width.max(1) as f32).max(320.0);
    let panel_height = PANEL_MAX_HEIGHT
        .min(viewport_height.max(1) as f32)
        .max(280.0);
    let content_width = (panel_width - SIDEBAR_WIDTH - 34.0).max(TILE_MIN_WIDTH);
    let columns = ((content_width + GRID_GAP) / (TILE_MIN_WIDTH + GRID_GAP))
        .floor()
        .max(1.0) as usize;
    let grid_height = (panel_height - HEADER_HEIGHT - 26.0).max(TILE_HEIGHT);
    let visible_rows = ((grid_height + GRID_GAP) / (TILE_HEIGHT + GRID_GAP))
        .floor()
        .max(1.0) as usize;
    let selected_row = launcher.selected_index() / columns.max(1);
    let mut scroll_row = state.scroll_row;
    if selected_row < scroll_row {
        scroll_row = selected_row;
    } else if selected_row >= scroll_row + visible_rows.max(1) {
        scroll_row = selected_row + 1 - visible_rows.max(1);
    }
    let maximum_row = launcher
        .result_count()
        .div_ceil(columns)
        .saturating_sub(visible_rows);
    scroll_row = scroll_row.min(maximum_row);

    let theme = SemanticTheme::new(SemanticColors {
        window: palette.background,
        sidebar: palette.panel,
        card: palette.surface,
        raised: palette.surface_hover,
        hover: palette.surface_hover,
        primary_text: palette.text,
        secondary_text: palette.muted,
        accent: palette.accent,
        accent_soft: palette.accent_soft,
        secondary_accent: palette.complement,
        positive: palette.complement,
    });
    let direction = launcher_reading_direction();
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
    let first = scroll_row * columns;
    let end = (first + visible_rows * columns).min(launcher.result_count());
    let cards = (first..end)
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
                .controller_focus_border(theme.borders.controller_focus)
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
    .on_activate(move |id| {
        LauncherAction::ActivateResult(
            *activation_indices
                .get(id)
                .expect("collection key must retain its result index"),
        )
    });
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
        .child(if launcher.result_count() == 0 {
            AnyView::new(Text::new("No matching applications").color(theme.text.secondary))
        } else {
            AnyView::new(collection)
        });
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
    let shell = StartMenuShell::new(theme, panel_width, sidebar, detail)
        .direction(direction)
        .header(search);
    AnyView::new(shell)
}

fn query_action(value: String) -> LauncherAction {
    LauncherAction::SetQuery(value)
}

fn build_dashboard_view(
    launcher: &Launcher,
    state: &LauncherViewState,
    icons: &mut LauncherIconCache,
    viewport_width: u32,
    viewport_height: u32,
    palette: ThemePalette,
    modality: InputModality,
) -> AnyView<LauncherAction> {
    build_dashboard_view_directional(
        launcher,
        state,
        icons,
        (viewport_width, viewport_height),
        palette,
        launcher_reading_direction(),
        modality,
    )
}

fn build_dashboard_view_directional(
    launcher: &Launcher,
    state: &LauncherViewState,
    icons: &mut LauncherIconCache,
    viewport: (u32, u32),
    palette: ThemePalette,
    direction: ReadingDirection,
    modality: InputModality,
) -> AnyView<LauncherAction> {
    let (viewport_width, _viewport_height) = viewport;
    let width = PANEL_MAX_WIDTH.min(viewport_width.max(1) as f32);
    let theme = SemanticTheme::new(SemanticColors {
        window: palette.background,
        sidebar: palette.panel,
        card: palette.surface,
        raised: palette.surface_hover,
        hover: palette.surface_hover,
        primary_text: palette.text,
        secondary_text: palette.muted,
        accent: palette.accent,
        accent_soft: palette.accent_soft,
        secondary_accent: palette.complement,
        positive: palette.complement,
    });
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

    let mut seen = std::collections::HashSet::new();
    let applications = match launcher.view() {
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
            .collect::<Vec<_>>(),
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
                .controller_focus_border(theme.borders.controller_focus)
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
        .id("launcher-dashboard-search-focus")
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
    if modality == InputModality::Controller {
        let entries = vec![
            ActionLegendEntry::available(SemanticControllerAction::Confirm, "Open"),
            ActionLegendEntry::available(SemanticControllerAction::ContextMenu, "Actions"),
            ActionLegendEntry::available(SemanticControllerAction::PreviousSection, "Sidebar"),
            ActionLegendEntry::available(SemanticControllerAction::NextSection, "Content"),
            ActionLegendEntry::available(SemanticControllerAction::Cancel, "Close"),
        ];
        shell = shell.legend(ActionLegend::new_directional(
            theme,
            state.controller_family,
            entries,
            direction,
        ));
    }
    AnyView::new(shell)
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
    use nickel_ui::{ActionKind, SemanticAction, UiHost};

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
    fn rebuilding_the_host_keeps_stable_image_resources_and_semantics() {
        let first = launcher_host();
        let second = launcher_host();
        assert_eq!(first.commands(), second.commands());
        assert_eq!(first.semantic_nodes(), second.semantic_nodes());
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
