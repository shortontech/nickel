//! SDL-independent launcher scene and hit testing.
//!
//! `LiveShell` owns input and mutates [`Launcher`]. This module turns that state
//! into component paint commands and stable semantic actions.

use std::{
    collections::HashMap,
    sync::{Arc, OnceLock},
};

use image::RgbaImage;
use nickel_core::theme::ThemePalette;
use nickel_ui::{
    AccountSummaryRow, Column, FallbackAvatar, Image, Insets, LauncherSearchField, LinearGradient,
    PaintCommand, Point, ProjectStatusRow, ReadingDirection, Rect, Row,
    START_MENU_SINGLE_PANE_BREAKPOINT, SectionHeader, SemanticColors, SemanticTheme,
    SessionActionRow, ShortcutRow, ShortcutState, StartMenuNarrowPane, StartMenuShell, Text,
    TextAlign, UiTree,
};

use crate::{
    icons,
    launcher::{Application, Launcher, LauncherMode, LauncherView, SettingsDestination},
    platform,
};

const PANEL_MAX_WIDTH: f32 = 920.0;
const PANEL_MAX_HEIGHT: f32 = 680.0;
const SIDEBAR_WIDTH: f32 = 148.0;
const HEADER_HEIGHT: f32 = 72.0;
const GRID_GAP: f32 = 10.0;
const TILE_MIN_WIDTH: f32 = 142.0;
const TILE_HEIGHT: f32 = 108.0;
const ICON_SIZE: f32 = 48.0;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LauncherAction {
    Dismiss,
    FocusSearch,
    SetView(LauncherView),
    ActivateResult(usize),
    TogglePin(String),
    LaunchApplication(String),
    OpenProject(String),
    SeeAllProjects,
    OpenSettings(SettingsDestination),
    OpenAccount,
    RequestLogout,
    ShowNarrowPrimary,
    ShowNarrowProjects,
    ShowNarrowSettings,
    SetQuery(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LauncherShellEffect {
    Dismiss,
    ActivateResult(usize),
    LaunchApplication(String),
    OpenProject(String),
    SeeAllProjects,
    OpenSettings(SettingsDestination),
    OpenAccount,
    RequestLogout,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DashboardNarrowPage {
    #[default]
    Primary,
    Projects,
    Settings,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LauncherHitTarget {
    pub bounds: Rect,
    pub action: LauncherAction,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LauncherViewState {
    pub scroll_row: usize,
    dashboard_scroll_row: usize,
    search_scroll_row: usize,
    pub hovered: Option<LauncherAction>,
    pub dashboard_selected: usize,
    pub dashboard_narrow_page: DashboardNarrowPage,
}

impl LauncherViewState {
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

    pub fn select_dashboard_next(&mut self, action_count: usize) {
        if action_count > 0 {
            self.dashboard_selected = (self.dashboard_selected + 1) % action_count;
        }
    }

    pub fn select_dashboard_previous(&mut self, action_count: usize) {
        if action_count > 0 {
            self.dashboard_selected = self
                .dashboard_selected
                .checked_sub(1)
                .unwrap_or(action_count - 1);
        }
    }

    pub fn scroll(
        &mut self,
        rows: isize,
        result_count: usize,
        columns: usize,
        visible_rows: usize,
    ) {
        let total_rows = result_count.div_ceil(columns.max(1));
        let maximum = total_rows.saturating_sub(visible_rows);
        self.scroll_row = (self.scroll_row as isize + rows).clamp(0, maximum as isize) as usize;
    }

    pub fn ensure_selected_visible(
        &mut self,
        selected: usize,
        columns: usize,
        visible_rows: usize,
    ) {
        let row = selected / columns.max(1);
        if row < self.scroll_row {
            self.scroll_row = row;
        } else if row >= self.scroll_row + visible_rows.max(1) {
            self.scroll_row = row + 1 - visible_rows.max(1);
        }
    }
}

#[derive(Clone, Debug)]
pub struct LauncherFrame {
    pub commands: Vec<PaintCommand>,
    pub hits: Vec<LauncherHitTarget>,
    pub columns: usize,
    pub visible_rows: usize,
    pub navigable_actions: Vec<LauncherAction>,
}

impl LauncherFrame {
    pub fn action_at(&self, point: Point) -> Option<&LauncherAction> {
        self.hits
            .iter()
            .rev()
            .find(|target| contains(target.bounds, point))
            .map(|target| &target.action)
    }

    #[cfg(test)]
    pub fn target_point(&self, action: &LauncherAction) -> Option<Point> {
        self.hits
            .iter()
            .find(|target| &target.action == action)
            .map(|target| Point {
                x: target.bounds.origin.x + target.bounds.size.width / 2.0,
                y: target.bounds.origin.y + target.bounds.size.height / 2.0,
            })
    }
}

pub fn reduce_launcher_action(
    launcher: &mut Launcher,
    view: &mut LauncherViewState,
    action: LauncherAction,
) -> Option<LauncherShellEffect> {
    match action {
        LauncherAction::Dismiss => Some(LauncherShellEffect::Dismiss),
        LauncherAction::FocusSearch => {
            let previous = launcher.mode();
            launcher.open_search();
            view.transition_mode(previous, launcher.mode());
            None
        }
        LauncherAction::SetView(next) => {
            launcher.set_view(next);
            view.scroll_row = 0;
            None
        }
        LauncherAction::ActivateResult(index) => Some(LauncherShellEffect::ActivateResult(index)),
        LauncherAction::TogglePin(id) => {
            launcher.toggle_pin(&id);
            None
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
        LauncherAction::ShowNarrowProjects => {
            view.dashboard_narrow_page = DashboardNarrowPage::Projects;
            view.dashboard_selected = 0;
            None
        }
        LauncherAction::ShowNarrowSettings => {
            view.dashboard_narrow_page = DashboardNarrowPage::Settings;
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

/// CPU image cache with stable IDs for `PaintCommand::Image`.
#[derive(Default)]
pub struct LauncherIconCache {
    icons: HashMap<String, CachedIcon>,
    next_id: u16,
}

struct CachedIcon {
    id: u16,
    image: Option<Arc<RgbaImage>>,
}

impl LauncherIconCache {
    pub fn new() -> Self {
        Self {
            icons: HashMap::new(),
            // Keep launcher IDs away from wallpaper and panel fixed IDs.
            next_id: 0x4000,
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
            .map(Arc::new);
        let id = self.next_id;
        self.next_id = self.next_id.checked_add(1).unwrap_or(0x4000);
        self.icons.insert(
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
        self.icons.insert(
            key,
            CachedIcon {
                id,
                image: image.clone(),
            },
        );
        image.map(|image| (id, image))
    }
}

pub fn build_launcher_frame(
    launcher: &Launcher,
    state: &mut LauncherViewState,
    icons: &mut LauncherIconCache,
    viewport_width: u32,
    viewport_height: u32,
    palette: ThemePalette,
) -> LauncherFrame {
    if launcher.mode() == LauncherMode::Dashboard {
        return build_dashboard_frame(
            launcher,
            state,
            icons,
            viewport_width,
            viewport_height,
            palette,
        );
    }
    let viewport = Rect::new(
        0.0,
        0.0,
        viewport_width.max(1) as f32,
        viewport_height.max(1) as f32,
    );
    let panel_width = PANEL_MAX_WIDTH.min(viewport.size.width.max(320.0));
    let panel_height = PANEL_MAX_HEIGHT.min(viewport.size.height.max(280.0));
    let panel = Rect::new(0.0, 0.0, panel_width, panel_height);
    let content_x = panel.origin.x + SIDEBAR_WIDTH + 18.0;
    let content_width = (panel.size.width - SIDEBAR_WIDTH - 34.0).max(TILE_MIN_WIDTH);
    let columns = ((content_width + GRID_GAP) / (TILE_MIN_WIDTH + GRID_GAP))
        .floor()
        .max(1.0) as usize;
    let tile_width =
        ((content_width - GRID_GAP * columns.saturating_sub(1) as f32) / columns as f32).max(1.0);
    let grid_y = panel.origin.y + HEADER_HEIGHT + 10.0;
    let grid_height = (panel.size.height - HEADER_HEIGHT - 26.0).max(TILE_HEIGHT);
    let visible_rows = ((grid_height + GRID_GAP) / (TILE_HEIGHT + GRID_GAP))
        .floor()
        .max(1.0) as usize;
    state.ensure_selected_visible(launcher.selected_index(), columns, visible_rows);
    let maximum_row = launcher
        .result_count()
        .div_ceil(columns)
        .saturating_sub(visible_rows);
    state.scroll_row = state.scroll_row.min(maximum_row);

    let mut commands = vec![
        PaintCommand::Fill {
            rect: panel,
            color: palette.surface,
        },
        PaintCommand::Gradient {
            rect: Rect::new(
                panel.origin.x,
                panel.origin.y,
                SIDEBAR_WIDTH,
                panel.size.height,
            ),
            gradient: LinearGradient::vertical(palette.panel, palette.background),
        },
    ];
    let mut hits = vec![
        LauncherHitTarget {
            bounds: viewport,
            action: LauncherAction::Dismiss,
        },
        LauncherHitTarget {
            bounds: panel,
            action: LauncherAction::FocusSearch,
        },
    ];

    commands.push(text(
        Rect::new(content_x, panel.origin.y + 18.0, content_width, 34.0),
        if launcher.query().is_empty() {
            view_title(launcher.view())
        } else {
            launcher.query()
        },
        1.25,
        palette.text,
        TextAlign::Start,
        true,
    ));
    if launcher.query().is_empty() {
        commands.push(text(
            Rect::new(content_x, panel.origin.y + 48.0, content_width, 20.0),
            "Type to search",
            0.82,
            palette.muted,
            TextAlign::Start,
            false,
        ));
    }

    for (index, (view, label)) in [
        (LauncherView::Favorites, "Favorites"),
        (LauncherView::Applications, "Applications"),
        (LauncherView::Places, "Places"),
    ]
    .into_iter()
    .enumerate()
    {
        let bounds = Rect::new(
            panel.origin.x + 10.0,
            panel.origin.y + 18.0 + index as f32 * 48.0,
            SIDEBAR_WIDTH - 20.0,
            40.0,
        );
        let action = LauncherAction::SetView(view);
        if launcher.view() == view || state.hovered.as_ref() == Some(&action) {
            commands.push(PaintCommand::RoundedFill {
                rect: bounds,
                color: if launcher.view() == view {
                    palette.accent_soft
                } else {
                    palette.surface_hover
                },
                radius: 8.0,
            });
        }
        commands.push(text(
            Rect::new(
                bounds.origin.x + 12.0,
                bounds.origin.y + 8.0,
                bounds.size.width - 24.0,
                24.0,
            ),
            label,
            0.9,
            if launcher.view() == view {
                palette.text
            } else {
                palette.muted
            },
            TextAlign::Start,
            launcher.view() == view,
        ));
        hits.push(LauncherHitTarget { bounds, action });
    }

    commands.push(PaintCommand::PushClip(Rect::new(
        content_x,
        grid_y,
        content_width,
        grid_height,
    )));
    let first = state.scroll_row * columns;
    let end = (first + visible_rows * columns).min(launcher.result_count());
    for result_index in first..end {
        let Some(application) = launcher.result_at(result_index) else {
            continue;
        };
        let visible_index = result_index - first;
        let row = visible_index / columns;
        let column = visible_index % columns;
        let bounds = Rect::new(
            content_x + column as f32 * (tile_width + GRID_GAP),
            grid_y + row as f32 * (TILE_HEIGHT + GRID_GAP),
            tile_width,
            TILE_HEIGHT,
        );
        let activate = LauncherAction::ActivateResult(result_index);
        let selected = launcher.selected_index() == result_index;
        commands.push(PaintCommand::RoundedFill {
            rect: bounds,
            color: if selected {
                palette.accent_soft
            } else {
                palette.surface
            },
            radius: 10.0,
        });
        if let Some((id, image)) = icons.resolve(application) {
            commands.push(PaintCommand::Image {
                bounds: Rect::new(
                    bounds.origin.x + (bounds.size.width - ICON_SIZE) / 2.0,
                    bounds.origin.y + 10.0,
                    ICON_SIZE,
                    ICON_SIZE,
                ),
                id,
                image,
                high_density: None,
            });
        } else {
            paint_application_fallback(&mut commands, bounds, palette);
        }
        commands.push(text(
            Rect::new(
                bounds.origin.x + 8.0,
                bounds.origin.y + 68.0,
                bounds.size.width - 16.0,
                26.0,
            ),
            application.name(),
            0.82,
            palette.text,
            TextAlign::Center,
            selected,
        ));
        if launcher.is_pinned(application.id()) {
            commands.push(text(
                Rect::new(
                    bounds.origin.x + bounds.size.width - 28.0,
                    bounds.origin.y + 6.0,
                    20.0,
                    20.0,
                ),
                "★",
                0.72,
                palette.accent,
                TextAlign::Center,
                false,
            ));
        }
        hits.push(LauncherHitTarget {
            bounds,
            action: activate,
        });
        hits.push(LauncherHitTarget {
            bounds: Rect::new(
                bounds.origin.x + bounds.size.width - 32.0,
                bounds.origin.y + 2.0,
                30.0,
                28.0,
            ),
            action: LauncherAction::TogglePin(application.id().to_owned()),
        });
    }
    commands.push(PaintCommand::PopClip);

    if launcher.result_count() == 0 {
        commands.push(text(
            Rect::new(content_x, grid_y + 40.0, content_width, 40.0),
            "No matching applications",
            1.0,
            palette.muted,
            TextAlign::Center,
            false,
        ));
    } else if maximum_row > 0 {
        let progress = state.scroll_row as f32 / maximum_row as f32;
        let thumb_height =
            (grid_height * visible_rows as f32 / (maximum_row + visible_rows) as f32).max(28.0);
        commands.push(PaintCommand::RoundedFill {
            rect: Rect::new(
                panel.origin.x + panel.size.width - 7.0,
                grid_y + progress * (grid_height - thumb_height),
                3.0,
                thumb_height,
            ),
            color: palette.muted,
            radius: 2.0,
        });
    }

    LauncherFrame {
        commands,
        hits,
        columns,
        visible_rows,
        navigable_actions: Vec::new(),
    }
}

fn query_action(value: String) -> LauncherAction {
    LauncherAction::SetQuery(value)
}

fn build_dashboard_frame(
    launcher: &Launcher,
    state: &mut LauncherViewState,
    icons: &mut LauncherIconCache,
    viewport_width: u32,
    viewport_height: u32,
    palette: ThemePalette,
) -> LauncherFrame {
    build_dashboard_frame_directional(
        launcher,
        state,
        icons,
        viewport_width,
        viewport_height,
        palette,
        launcher_reading_direction(),
    )
}

fn build_dashboard_frame_directional(
    launcher: &Launcher,
    state: &mut LauncherViewState,
    icons: &mut LauncherIconCache,
    viewport_width: u32,
    viewport_height: u32,
    palette: ThemePalette,
    direction: ReadingDirection,
) -> LauncherFrame {
    let width = PANEL_MAX_WIDTH.min(viewport_width.max(1) as f32);
    let height = PANEL_MAX_HEIGHT.min(viewport_height.max(1) as f32);
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
        positive: palette.complement,
    });
    let narrow = width < START_MENU_SINGLE_PANE_BREAKPOINT;
    let project_limit = 3;
    let actions = dashboard_actions(
        launcher,
        project_limit,
        narrow.then_some(state.dashboard_narrow_page),
    );
    state.dashboard_selected = state
        .dashboard_selected
        .min(actions.len().saturating_sub(1));
    let selected_action = actions.get(state.dashboard_selected);
    let row_state = |action: &LauncherAction| ShortcutState {
        selected: selected_action == Some(action),
        focused: selected_action == Some(action),
        hovered: state.hovered.as_ref() == Some(action),
        ..ShortcutState::default()
    };
    let favorite_applications = launcher
        .favorite_applications()
        .into_iter()
        .take(if narrow { 1 } else { 7 })
        .cloned()
        .collect::<Vec<_>>();
    let favorites = favorite_applications
        .iter()
        .map(|application| {
            let action = LauncherAction::LaunchApplication(application.id().to_owned());
            let icon = icons.resolve(application).unwrap_or_else(|| {
                icons
                    .structural(
                        "applications",
                        include_bytes!("../../../assets/icons/start-menu/applications.svg"),
                        theme.text.secondary,
                    )
                    .expect("embedded applications icon must rasterize")
            });
            ShortcutRow::new_directional(
                theme,
                Image::new(icon.0, icon.1).width(28.0).height(28.0),
                application.name(),
                "",
                Some(action.clone()),
                row_state(&action),
                direction,
            )
        })
        .collect::<Vec<_>>();
    let account = match launcher.dashboard_account() {
        crate::launcher::DashboardSection::Loading => AccountSummaryRow::new_directional(
            theme,
            structural_icon(
                icons,
                "account",
                include_bytes!("../../../assets/icons/start-menu/account.svg"),
                theme.text.secondary,
            ),
            "Loading account…",
            "Local session",
            None,
            ShortcutState::default(),
            direction,
        ),
        crate::launcher::DashboardSection::Empty => AccountSummaryRow::new_directional(
            theme,
            structural_icon(
                icons,
                "account",
                include_bytes!("../../../assets/icons/start-menu/account.svg"),
                theme.text.secondary,
            ),
            "Local session",
            "Account details unavailable",
            None,
            ShortcutState::default(),
            direction,
        ),
        crate::launcher::DashboardSection::Ready(account) => AccountSummaryRow::new_directional(
            theme,
            FallbackAvatar::new(theme, &account.display_name),
            &account.display_name,
            &account.supporting_text,
            Some(LauncherAction::OpenAccount),
            row_state(&LauncherAction::OpenAccount),
            direction,
        ),
        crate::launcher::DashboardSection::Unavailable(reason) => {
            AccountSummaryRow::new_directional(
                theme,
                structural_icon(
                    icons,
                    "account",
                    include_bytes!("../../../assets/icons/start-menu/account.svg"),
                    theme.text.secondary,
                ),
                "Local session",
                reason,
                None,
                ShortcutState::default(),
                direction,
            )
        }
        crate::launcher::DashboardSection::Failed {
            message,
            recoverable,
        } => AccountSummaryRow::new_directional(
            theme,
            structural_icon(
                icons,
                "account",
                include_bytes!("../../../assets/icons/start-menu/account.svg"),
                theme.text.secondary,
            ),
            if *recoverable {
                "Account temporarily unavailable"
            } else {
                "Account unavailable"
            },
            message,
            None,
            ShortcutState::default(),
            direction,
        ),
    };
    let primary_footer = Column::new()
        .child(account)
        .child(SessionActionRow::new_directional(
            theme,
            structural_icon(
                icons,
                "logout",
                include_bytes!("../../../assets/icons/start-menu/logout.svg"),
                theme.text.secondary,
            ),
            "Log out",
            LauncherAction::RequestLogout,
            false,
            row_state(&LauncherAction::RequestLogout),
            direction,
        ));
    let mut primary = Column::new()
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
        .child(SectionHeader::new(theme, "APPLICATIONS").direction(direction))
        .children(favorites)
        .child(ShortcutRow::new_directional(
            theme,
            structural_icon(
                icons,
                "applications",
                include_bytes!("../../../assets/icons/start-menu/applications.svg"),
                theme.text.secondary,
            ),
            "All Applications",
            if narrow {
                ""
            } else {
                "Search and browse installed applications"
            },
            Some(LauncherAction::FocusSearch),
            row_state(&LauncherAction::FocusSearch),
            direction,
        ));
    if narrow {
        for (label, _supporting, icon_name, icon_bytes, action) in [
            (
                "Projects",
                "Recent Codex projects",
                "project",
                include_bytes!("../../../assets/icons/start-menu/project.svg").as_slice(),
                LauncherAction::ShowNarrowProjects,
            ),
            (
                "Settings",
                "System and Nickel settings",
                "settings",
                include_bytes!("../../../assets/icons/start-menu/settings.svg").as_slice(),
                LauncherAction::ShowNarrowSettings,
            ),
        ] {
            primary = primary.child(ShortcutRow::new_directional(
                theme,
                structural_icon(icons, icon_name, icon_bytes, theme.text.secondary),
                label,
                "",
                Some(action.clone()),
                row_state(&action),
                direction,
            ));
        }
    }
    let project_rows = match launcher.dashboard_projects() {
        crate::launcher::DashboardSection::Loading => vec![nickel_ui::AnyView::new(
            Text::new("Loading projects…").color(theme.text.secondary),
        )],
        crate::launcher::DashboardSection::Unavailable(reason) => {
            vec![nickel_ui::AnyView::new(ProjectStatusRow::new_directional(
                theme,
                structural_icon(
                    icons,
                    "project",
                    include_bytes!("../../../assets/icons/start-menu/project.svg"),
                    theme.text.secondary,
                ),
                "Projects unavailable",
                reason,
                None,
                None,
                ShortcutState::default(),
                direction,
            ))]
        }
        crate::launcher::DashboardSection::Empty => vec![nickel_ui::AnyView::new(
            Text::new("No recent projects").color(theme.text.secondary),
        )],
        crate::launcher::DashboardSection::Failed {
            message,
            recoverable,
        } => vec![nickel_ui::AnyView::new(ProjectStatusRow::new_directional(
            theme,
            structural_icon(
                icons,
                "project",
                include_bytes!("../../../assets/icons/start-menu/project.svg"),
                theme.text.secondary,
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
        crate::launcher::DashboardSection::Ready(projects) => {
            let projects = projects
                .iter()
                .take(project_limit)
                .cloned()
                .collect::<Vec<_>>();
            projects
                .into_iter()
                .map(|project| {
                    let status = match project.activity {
                        crate::launcher::ProjectActivity::Active => "Active",
                        crate::launcher::ProjectActivity::Idle => "Idle",
                        crate::launcher::ProjectActivity::Unknown => "Status unknown",
                    };
                    let action = LauncherAction::OpenProject(project.id.clone());
                    nickel_ui::AnyView::new(ProjectStatusRow::new_directional(
                        theme,
                        structural_icon(
                            icons,
                            "project",
                            include_bytes!("../../../assets/icons/start-menu/project.svg"),
                            theme.text.accent,
                        ),
                        project.name,
                        status,
                        project.chat_count,
                        Some(action.clone()),
                        row_state(&action),
                        direction,
                    ))
                })
                .collect()
        }
    };
    let projects_section = Column::new()
        .gap(theme.spacing.compact)
        .child(
            SectionHeader::new(theme, "PROJECTS")
                .action(theme, "See all projects", LauncherAction::SeeAllProjects)
                .direction(direction),
        )
        .children(project_rows);
    let settings_section = Column::new()
        .gap(theme.spacing.compact)
        .child(SectionHeader::new(theme, "SETTINGS").direction(direction))
        .child(ShortcutRow::new_directional(
            theme,
            structural_icon(
                icons,
                "system-settings",
                include_bytes!("../../../assets/icons/start-menu/settings.svg"),
                theme.text.secondary,
            ),
            "System Settings",
            "Display, sound, network, power",
            Some(LauncherAction::OpenSettings(SettingsDestination::System)),
            row_state(&LauncherAction::OpenSettings(SettingsDestination::System)),
            direction,
        ))
        .child(ShortcutRow::new_directional(
            theme,
            structural_icon(
                icons,
                "nickel-settings",
                include_bytes!("../../../assets/icons/start-menu/settings.svg"),
                theme.text.accent,
            ),
            "Nickel Settings",
            "Shell, bar, behavior, appearance",
            Some(LauncherAction::OpenSettings(SettingsDestination::Nickel)),
            row_state(&LauncherAction::OpenSettings(SettingsDestination::Nickel)),
            direction,
        ))
        .child(ShortcutRow::new_directional(
            theme,
            structural_icon(
                icons,
                "keyboard",
                include_bytes!("../../../assets/icons/start-menu/keyboard.svg"),
                theme.text.secondary,
            ),
            "Keyboard Shortcuts",
            "View Nickel shortcuts",
            Some(LauncherAction::OpenSettings(
                SettingsDestination::KeyboardShortcuts,
            )),
            row_state(&LauncherAction::OpenSettings(
                SettingsDestination::KeyboardShortcuts,
            )),
            direction,
        ))
        .child(ShortcutRow::new_directional(
            theme,
            structural_icon(
                icons,
                "about",
                include_bytes!("../../../assets/icons/start-menu/about.svg"),
                theme.text.secondary,
            ),
            "About Nickel",
            "System information and updates",
            Some(LauncherAction::OpenSettings(SettingsDestination::About)),
            row_state(&LauncherAction::OpenSettings(SettingsDestination::About)),
            direction,
        ));
    let back_action = LauncherAction::ShowNarrowPrimary;
    let mut back_row = || {
        ShortcutRow::new_directional(
            theme,
            structural_icon(
                icons,
                "applications",
                include_bytes!("../../../assets/icons/start-menu/applications.svg"),
                theme.text.secondary,
            ),
            "Applications and account",
            "Back to the main menu",
            Some(back_action.clone()),
            row_state(&back_action),
            direction,
        )
    };
    let detail = match (narrow, state.dashboard_narrow_page) {
        (true, DashboardNarrowPage::Projects) => Column::new()
            .gap(theme.spacing.compact)
            .padding(Insets::all(theme.spacing.content))
            .child(back_row())
            .child(projects_section),
        (true, DashboardNarrowPage::Settings) => Column::new()
            .gap(theme.spacing.compact)
            .padding(Insets::all(theme.spacing.content))
            .child(back_row())
            .child(settings_section),
        _ => Column::new()
            .gap(theme.spacing.compact)
            .padding(Insets::all(theme.spacing.content))
            .child(projects_section)
            .child(nickel_ui::Spacer::flex())
            .child(settings_section),
    };
    let detail_footer = LauncherSearchField::new_directional(
        theme,
        structural_icon(
            icons,
            "search",
            include_bytes!("../../../assets/icons/settings/search.svg"),
            theme.text.secondary,
        ),
        launcher.query(),
        launcher.preedit(),
        "Search applications, projects, files…",
        query_action,
        direction,
    );
    let tree = UiTree::layout(
        StartMenuShell::new(theme, width, primary, detail)
            .direction(direction)
            .narrow_pane(
                if narrow && state.dashboard_narrow_page != DashboardNarrowPage::Primary {
                    StartMenuNarrowPane::Detail
                } else {
                    StartMenuNarrowPane::Primary
                },
            )
            .primary_footer(primary_footer)
            .detail_footer(detail_footer),
        Rect::new(0.0, 0.0, width, height),
    );
    let mut hits = Vec::new();
    hits.extend(actions.iter().cloned().filter_map(|action| {
        tree.message_rect(&action)
            .map(|bounds| LauncherHitTarget { bounds, action })
    }));
    LauncherFrame {
        commands: tree.commands().to_vec(),
        hits,
        columns: 1,
        visible_rows: 1,
        navigable_actions: actions,
    }
}

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

fn dashboard_actions(
    launcher: &Launcher,
    project_limit: usize,
    narrow_page: Option<DashboardNarrowPage>,
) -> Vec<LauncherAction> {
    let primary_content = || {
        let favorite_limit = if narrow_page.is_some() { 1 } else { 7 };
        let mut actions = launcher
            .favorite_applications()
            .into_iter()
            .take(favorite_limit)
            .map(|application| LauncherAction::LaunchApplication(application.id().to_owned()))
            .collect::<Vec<_>>();
        actions.push(LauncherAction::FocusSearch);
        actions
    };
    let account_session = || {
        let mut actions = Vec::new();
        if matches!(
            launcher.dashboard_account(),
            crate::launcher::DashboardSection::Ready(_)
        ) {
            actions.push(LauncherAction::OpenAccount);
        }
        if launcher.logout_available() {
            actions.push(LauncherAction::RequestLogout);
        }
        actions
    };
    let projects = || {
        let mut actions = vec![LauncherAction::SeeAllProjects];
        if let crate::launcher::DashboardSection::Ready(projects) = launcher.dashboard_projects() {
            actions.extend(
                projects
                    .iter()
                    .take(project_limit)
                    .map(|project| LauncherAction::OpenProject(project.id.clone())),
            );
        }
        actions
    };
    let settings = || {
        vec![
            LauncherAction::OpenSettings(SettingsDestination::System),
            LauncherAction::OpenSettings(SettingsDestination::Nickel),
            LauncherAction::OpenSettings(SettingsDestination::KeyboardShortcuts),
            LauncherAction::OpenSettings(SettingsDestination::About),
        ]
    };
    match narrow_page {
        None => primary_content()
            .into_iter()
            .chain(account_session())
            .chain(projects())
            .chain(settings())
            .collect(),
        Some(DashboardNarrowPage::Primary) => primary_content()
            .into_iter()
            .chain([
                LauncherAction::ShowNarrowProjects,
                LauncherAction::ShowNarrowSettings,
            ])
            .chain(account_session())
            .collect(),
        Some(DashboardNarrowPage::Projects) => std::iter::once(LauncherAction::ShowNarrowPrimary)
            .chain(projects())
            .collect(),
        Some(DashboardNarrowPage::Settings) => std::iter::once(LauncherAction::ShowNarrowPrimary)
            .chain(settings())
            .collect(),
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

fn text(
    bounds: Rect,
    value: &str,
    scale: f32,
    color: u32,
    align: TextAlign,
    bold: bool,
) -> PaintCommand {
    PaintCommand::Text {
        bounds,
        text: value.to_owned(),
        scale,
        color,
        align,
        bold,
        wrap: false,
    }
}

fn contains(bounds: Rect, point: Point) -> bool {
    point.x >= bounds.origin.x
        && point.y >= bounds.origin.y
        && point.x < bounds.origin.x + bounds.size.width
        && point.y < bounds.origin.y + bounds.size.height
}

fn has_visible_pixel(image: &RgbaImage) -> bool {
    image.pixels().any(|pixel| pixel[3] != 0)
}

fn paint_application_fallback(commands: &mut Vec<PaintCommand>, tile: Rect, palette: ThemePalette) {
    let icon = Rect::new(
        tile.origin.x + (tile.size.width - ICON_SIZE) / 2.0,
        tile.origin.y + 10.0,
        ICON_SIZE,
        ICON_SIZE,
    );
    commands.push(PaintCommand::RoundedFill {
        rect: icon,
        color: palette.surface_hover,
        radius: 10.0,
    });
    let cell = 11.0;
    let gap = 4.0;
    let origin_x = icon.origin.x + (icon.size.width - cell * 2.0 - gap) / 2.0;
    let origin_y = icon.origin.y + (icon.size.height - cell * 2.0 - gap) / 2.0;
    for row in 0..2 {
        for column in 0..2 {
            commands.push(PaintCommand::RoundedFill {
                rect: Rect::new(
                    origin_x + column as f32 * (cell + gap),
                    origin_y + row as f32 * (cell + gap),
                    cell,
                    cell,
                ),
                color: palette.muted,
                radius: 2.5,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::launcher::{DashboardAccount, DashboardProject, DashboardSection, ProjectActivity};
    use nickel_ui::SdlComponentRenderer;
    use sha2::{Digest, Sha256};

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

    struct LauncherScenario {
        launcher: Launcher,
        view: LauncherViewState,
    }

    impl LauncherScenario {
        fn new() -> Self {
            Self {
                launcher: Launcher::default(),
                view: LauncherViewState::default(),
            }
        }

        fn text(mut self, text: &str) -> Self {
            let previous = self.launcher.mode();
            let _ = self
                .launcher
                .reduce_input(crate::launcher::LauncherInput::Text(text.into()));
            self.view.transition_mode(previous, self.launcher.mode());
            self.view.reset_active_scroll(self.launcher.mode());
            self
        }

        fn preedit(mut self, text: &str) -> Self {
            let previous = self.launcher.mode();
            let _ = self
                .launcher
                .reduce_input(crate::launcher::LauncherInput::Preedit(text.into()));
            self.view.transition_mode(previous, self.launcher.mode());
            self.view.reset_active_scroll(self.launcher.mode());
            self
        }

        fn escape(mut self) -> Self {
            let previous = self.launcher.mode();
            let _ = self
                .launcher
                .reduce_input(crate::launcher::LauncherInput::Escape);
            self.view.transition_mode(previous, self.launcher.mode());
            self
        }

        fn backspace(mut self) -> Self {
            let previous = self.launcher.mode();
            let _ = self
                .launcher
                .reduce_input(crate::launcher::LauncherInput::Backspace);
            self.view.transition_mode(previous, self.launcher.mode());
            if self.launcher.mode() == LauncherMode::Search {
                self.view.reset_active_scroll(LauncherMode::Search);
            }
            self
        }

        fn scroll(mut self, rows: isize, columns: usize, visible_rows: usize) -> Self {
            self.view
                .scroll(rows, self.launcher.result_count(), columns, visible_rows);
            self
        }

        fn expect_mode(self, mode: LauncherMode) -> Self {
            assert_eq!(self.launcher.mode(), mode);
            self
        }

        fn expect_query(self, query: &str) -> Self {
            assert_eq!(self.launcher.query(), query);
            self
        }

        fn expect_preedit(self, preedit: &str) -> Self {
            assert_eq!(self.launcher.preedit(), preedit);
            self
        }

        fn expect_scroll_row(self, row: usize) -> Self {
            assert_eq!(self.view.scroll_row, row);
            self
        }
    }

    #[test]
    fn fluent_launcher_inputs_cover_search_ime_escape_order_and_scroll() {
        LauncherScenario::new()
            .text("f")
            .expect_mode(LauncherMode::Search)
            .expect_query("f")
            .preedit("ir")
            .expect_preedit("ir")
            .escape()
            .expect_mode(LauncherMode::Search)
            .expect_query("f")
            .expect_preedit("")
            .escape()
            .expect_mode(LauncherMode::Dashboard);

        LauncherScenario::new()
            .text("a")
            .backspace()
            .expect_mode(LauncherMode::Dashboard)
            .text("a")
            .scroll(1, 1, 1)
            .expect_scroll_row(1);
    }

    #[test]
    fn dashboard_and_search_restore_independent_scroll_positions() {
        let mut state = LauncherViewState {
            scroll_row: 2,
            ..LauncherViewState::default()
        };
        state.transition_mode(LauncherMode::Dashboard, LauncherMode::Search);
        assert_eq!(state.scroll_row, 0);

        state.scroll_row = 5;
        state.transition_mode(LauncherMode::Search, LauncherMode::Dashboard);
        assert_eq!(state.scroll_row, 2);

        state.transition_mode(LauncherMode::Dashboard, LauncherMode::Search);
        assert_eq!(state.scroll_row, 5);
    }

    #[test]
    fn dashboard_exposes_one_hit_for_every_typed_action() {
        let mut launcher = Launcher::default();
        launcher.set_pins(vec![("firefox".into(), 1)]);
        launcher.set_dashboard_projects(DashboardSection::Ready(vec![DashboardProject {
            id: "nickel".into(),
            name: "Nickel".into(),
            roots: vec![PathBuf::from("/projects/nickel")],
            chat_count: Some(2),
            activity: ProjectActivity::Active,
            last_used_at: Some(10),
        }]));
        let mut state = LauncherViewState::default();
        let mut icons = LauncherIconCache::new();

        let frame = build_launcher_frame(&launcher, &mut state, &mut icons, 920, 680, palette());

        assert_eq!(
            frame.navigable_actions,
            dashboard_actions(&launcher, 3, None)
        );
        for action in &frame.navigable_actions {
            assert_eq!(
                frame
                    .hits
                    .iter()
                    .filter(|hit| &hit.action == action)
                    .count(),
                1,
                "{action:?} must have exactly one hit region"
            );
        }
    }

    #[test]
    fn dashboard_background_has_no_dismiss_authority() {
        let launcher = Launcher::default();
        let mut state = LauncherViewState::default();
        let mut icons = LauncherIconCache::new();
        let frame = build_launcher_frame(&launcher, &mut state, &mut icons, 920, 560, palette());
        assert!(
            frame
                .hits
                .iter()
                .all(|target| target.action != LauncherAction::Dismiss),
            "inside-surface hit testing cannot own outside dismissal"
        );
    }

    #[test]
    fn semantic_click_uses_production_hit_resolution_and_action_reducer() {
        let mut launcher = Launcher::default();
        let mut state = LauncherViewState::default();
        let mut icons = LauncherIconCache::new();
        let frame = build_launcher_frame(&launcher, &mut state, &mut icons, 920, 560, palette());
        let point = frame
            .target_point(&LauncherAction::FocusSearch)
            .expect("Search has a production hit target");
        let action = frame
            .action_at(point)
            .cloned()
            .expect("ordinary pointer resolution reaches Search");
        assert_eq!(action, LauncherAction::FocusSearch);
        assert_eq!(
            reduce_launcher_action(&mut launcher, &mut state, action),
            None
        );
        assert_eq!(launcher.mode(), LauncherMode::Search);
    }

    #[test]
    fn semantic_targeting_uses_each_production_layout_geometry() {
        for (width, height) in [(520, 420), (920, 560), (1280, 760)] {
            let mut launcher = Launcher::default();
            let mut state = LauncherViewState::default();
            let mut icons = LauncherIconCache::new();
            let frame =
                build_launcher_frame(&launcher, &mut state, &mut icons, width, height, palette());
            let point = frame
                .target_point(&LauncherAction::FocusSearch)
                .expect("production layout exposes Search semantically");
            let action = frame
                .action_at(point)
                .cloned()
                .expect("production hit testing resolves its own target point");
            assert_eq!(
                reduce_launcher_action(&mut launcher, &mut state, action),
                None
            );
            assert_eq!(launcher.mode(), LauncherMode::Search);
        }
    }

    #[test]
    fn semantic_target_admission_rejects_an_incorrect_resolver() {
        type Resolver = fn(&LauncherFrame, &LauncherAction) -> Option<Point>;

        fn production(frame: &LauncherFrame, action: &LauncherAction) -> Option<Point> {
            frame.target_point(action)
        }

        fn incorrect(_: &LauncherFrame, _: &LauncherAction) -> Option<Point> {
            Some(Point { x: -1.0, y: -1.0 })
        }

        fn drive(resolver: Resolver) {
            let mut launcher = Launcher::default();
            let mut state = LauncherViewState::default();
            let mut icons = LauncherIconCache::new();
            let frame =
                build_launcher_frame(&launcher, &mut state, &mut icons, 920, 560, palette());
            let point = resolver(&frame, &LauncherAction::FocusSearch)
                .expect("semantic resolver must locate Search");
            let action = frame
                .action_at(point)
                .cloned()
                .expect("resolved point must survive production hit testing");
            let _ = reduce_launcher_action(&mut launcher, &mut state, action);
            assert_eq!(launcher.mode(), LauncherMode::Search);
        }

        drive(production);
        assert!(
            std::panic::catch_unwind(|| drive(incorrect)).is_err(),
            "the harness must detect a resolver that bypasses production geometry"
        );
    }

    #[test]
    fn narrow_dashboard_keeps_every_typed_action_reachable() {
        let mut launcher = Launcher::default();
        launcher.set_dashboard_account(DashboardSection::Ready(DashboardAccount {
            display_name: "Steven".into(),
            supporting_text: "Local session".into(),
        }));
        launcher.set_dashboard_projects(DashboardSection::Ready(vec![DashboardProject {
            id: "nickel".into(),
            name: "Nickel".into(),
            roots: vec![PathBuf::from("/projects/nickel")],
            chat_count: Some(2),
            activity: ProjectActivity::Active,
            last_used_at: Some(10),
        }]));
        let mut icons = LauncherIconCache::new();
        let mut reachable = Vec::new();

        for page in [
            DashboardNarrowPage::Primary,
            DashboardNarrowPage::Projects,
            DashboardNarrowPage::Settings,
        ] {
            let mut state = LauncherViewState {
                dashboard_narrow_page: page,
                ..LauncherViewState::default()
            };
            let frame =
                build_launcher_frame(&launcher, &mut state, &mut icons, 360, 416, palette());
            for action in &frame.navigable_actions {
                let matching = frame
                    .hits
                    .iter()
                    .filter(|hit| &hit.action == action)
                    .collect::<Vec<_>>();
                assert_eq!(
                    matching.len(),
                    1,
                    "{action:?} must have one hit on {page:?}"
                );
                let bounds = matching[0].bounds;
                assert!(
                    bounds.origin.x >= 0.0
                        && bounds.origin.y >= 0.0
                        && bounds.origin.x + bounds.size.width <= 360.0
                        && bounds.origin.y + bounds.size.height <= 416.0,
                    "{action:?} is outside the reachable narrow viewport on {page:?}: {bounds:?}"
                );
                if !reachable.contains(action) {
                    reachable.push(action.clone());
                }
            }
        }
        for action in dashboard_actions(&launcher, 3, None) {
            assert!(reachable.contains(&action), "{action:?} is unreachable");
        }
        assert!(reachable.contains(&LauncherAction::ShowNarrowProjects));
        assert!(reachable.contains(&LauncherAction::ShowNarrowSettings));
        assert!(reachable.contains(&LauncherAction::ShowNarrowPrimary));
    }

    #[test]
    fn dashboard_navigation_wraps_in_both_directions() {
        let mut state = LauncherViewState::default();
        state.select_dashboard_previous(4);
        assert_eq!(state.dashboard_selected, 3);
        state.select_dashboard_next(4);
        assert_eq!(state.dashboard_selected, 0);
    }

    #[test]
    fn locale_direction_and_dashboard_panes_mirror_for_rtl_languages() {
        assert_eq!(
            reading_direction_for_locale("ar_SA.UTF-8"),
            ReadingDirection::RightToLeft
        );
        assert_eq!(
            reading_direction_for_locale("es-ES"),
            ReadingDirection::LeftToRight
        );

        let launcher = Launcher::default();
        let mut state = LauncherViewState::default();
        let mut icons = LauncherIconCache::new();
        let frame = build_dashboard_frame_directional(
            &launcher,
            &mut state,
            &mut icons,
            920,
            560,
            palette(),
            ReadingDirection::RightToLeft,
        );
        let text_x = |wanted: &str| {
            frame
                .commands
                .iter()
                .find_map(|command| match command {
                    PaintCommand::Text { bounds, text, .. } if text == wanted => {
                        Some(bounds.origin.x)
                    }
                    _ => None,
                })
                .expect("dashboard text")
        };
        assert!(text_x("PROJECTS") < text_x("Nickel"));
        assert!(frame.commands.iter().any(|command| matches!(
            command,
            PaintCommand::Text { text, .. } if text.contains('‹')
        )));
    }

    #[test]
    fn search_frame_keeps_grid_navigation_and_no_dashboard_actions() {
        let mut launcher = Launcher::default();
        launcher.insert("fire");
        let mut state = LauncherViewState::default();
        let mut icons = LauncherIconCache::new();

        let frame = build_launcher_frame(&launcher, &mut state, &mut icons, 920, 680, palette());

        assert!(frame.navigable_actions.is_empty());
        assert!(frame.columns > 1);
        assert!(
            frame
                .hits
                .iter()
                .any(|hit| matches!(hit.action, LauncherAction::ActivateResult(0)))
        );
    }

    #[test]
    fn launcher_transition_renderer_paths_stay_within_interactive_budgets() {
        use std::time::Instant;

        let mut launcher = Launcher::new(
            (0..60)
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
        let mut state = LauncherViewState::default();
        let mut icons = LauncherIconCache::new();
        let mut renderer = SdlComponentRenderer::new_pixel_buffer(920, 680, 1.0);

        let warm = build_launcher_frame(&launcher, &mut state, &mut icons, 920, 680, palette());
        let _ = renderer.render(&warm.commands);

        let first_character_started = Instant::now();
        launcher.insert("app");
        state.reset_active_scroll(LauncherMode::Search);
        let search = build_launcher_frame(&launcher, &mut state, &mut icons, 920, 680, palette());
        let _ = renderer.render(&search.commands);
        let first_character = first_character_started.elapsed();

        let scroll_started = Instant::now();
        state.scroll(
            1,
            launcher.result_count(),
            search.columns,
            search.visible_rows,
        );
        assert!(
            state.scroll_row > 0,
            "scroll fixture must overflow the grid"
        );
        let scrolled = build_launcher_frame(&launcher, &mut state, &mut icons, 920, 680, palette());
        let _ = renderer.render(&scrolled.commands);
        let scroll = scroll_started.elapsed();

        if std::env::var_os("NICKEL_PERF_METRICS").is_some() {
            eprintln!(
                "launcher_first_character_renderer_ms={:.3}",
                first_character.as_secs_f64() * 1_000.0
            );
            eprintln!(
                "launcher_scroll_renderer_ms={:.3}",
                scroll.as_secs_f64() * 1_000.0
            );
        }
        assert!(
            first_character.as_millis() < 250,
            "first-character renderer path took {first_character:?}"
        );
        assert!(
            scroll.as_millis() < 250,
            "scroll renderer path took {scroll:?}"
        );
    }

    #[test]
    fn visual_fixture_manifest_admits_the_exact_reference() {
        let manifest: toml::Table =
            toml::from_str(include_str!("../../../assets/visual-fixtures.toml"))
                .expect("visual fixture manifest parses");
        let reference = manifest["reference"]
            .as_array()
            .and_then(|entries| {
                entries
                    .iter()
                    .find(|entry| entry["id"].as_str() == Some("nickel-start-menu-2026-08-27"))
            })
            .expect("Start Menu reference entry");
        let image = include_bytes!("../../../assets/references/nickel-start-menu.png");
        let digest = format!("{:x}", Sha256::digest(image));
        assert_eq!(reference["width"].as_integer(), Some(1401));
        assert_eq!(reference["height"].as_integer(), Some(1123));
        assert_eq!(reference["sha256"].as_str(), Some(digest.as_str()));
        assert!(
            !reference["authorship"]
                .as_str()
                .unwrap_or_default()
                .is_empty()
        );
        assert!(
            !reference["usage_status"]
                .as_str()
                .unwrap_or_default()
                .is_empty()
        );
    }

    #[test]
    fn dashboard_fixture_matrix_rasterizes_without_empty_or_nonfinite_frames() {
        let light = ThemePalette {
            background: 0xf4f6f8,
            panel: 0xe7ebef,
            surface: 0xffffff,
            surface_hover: 0xdce3e9,
            text: 0x18202a,
            muted: 0x5d6874,
            accent: 0x177ec1,
            accent_soft: 0xb9def5,
            complement: 0x7b4db2,
        };
        let high_contrast = ThemePalette {
            background: 0x000000,
            panel: 0x000000,
            surface: 0x101010,
            surface_hover: 0x252525,
            text: 0xffffff,
            muted: 0xd8d8d8,
            accent: 0x59c7ff,
            accent_soft: 0x164e70,
            complement: 0x7dff9b,
        };
        let alternate_accent = ThemePalette {
            accent: 0xf07a42,
            accent_soft: 0x60321f,
            complement: 0x43c7c0,
            ..palette()
        };
        for (section_name, projects) in [
            ("loading", DashboardSection::Loading),
            ("empty", DashboardSection::Empty),
            (
                "populated",
                DashboardSection::Ready(vec![DashboardProject {
                    id: "nickel".into(),
                    name: "Nickel".into(),
                    roots: vec![PathBuf::from("/projects/nickel")],
                    chat_count: Some(2),
                    activity: ProjectActivity::Active,
                    last_used_at: Some(10),
                }]),
            ),
            (
                "localized",
                DashboardSection::Ready(vec![
                    DashboardProject {
                        id: "spanish".into(),
                        name: "Proyecto de documentación ampliada".into(),
                        roots: vec![PathBuf::from("/projects/spanish")],
                        chat_count: Some(1),
                        activity: ProjectActivity::Idle,
                        last_used_at: Some(9),
                    },
                    DashboardProject {
                        id: "chinese".into(),
                        name: "中文桌面外壳项目".into(),
                        roots: vec![PathBuf::from("/projects/chinese")],
                        chat_count: None,
                        activity: ProjectActivity::Unknown,
                        last_used_at: Some(8),
                    },
                    DashboardProject {
                        id: "arabic".into(),
                        name: "مشروع سطح المكتب".into(),
                        roots: vec![PathBuf::from("/projects/arabic")],
                        chat_count: Some(4),
                        activity: ProjectActivity::Active,
                        last_used_at: Some(7),
                    },
                ]),
            ),
            (
                "recoverable-failure",
                DashboardSection::Failed {
                    message: "Retrying the Codex connection".into(),
                    recoverable: true,
                },
            ),
            (
                "unavailable",
                DashboardSection::Unavailable(
                    "Projektinformationen sind vorübergehend nicht verfügbar".into(),
                ),
            ),
        ] {
            for (theme_name, palette) in [
                ("dark", palette()),
                ("light", light),
                ("high-contrast", high_contrast),
                ("alternate-accent", alternate_accent),
                ("reduced-transparency", palette()),
            ] {
                for (width, height) in [(920_u32, 560_u32), (560, 640), (360, 480)] {
                    for scale in [1.0_f32, 1.25, 2.0] {
                        let mut launcher = Launcher::default();
                        launcher.set_dashboard_account(DashboardSection::Ready(DashboardAccount {
                            display_name: "Steven".into(),
                            supporting_text: "Local session".into(),
                        }));
                        launcher.set_dashboard_projects(projects.clone());
                        let mut state = LauncherViewState::default();
                        let mut icons = LauncherIconCache::new();
                        let frame = build_launcher_frame(
                            &launcher, &mut state, &mut icons, width, height, palette,
                        );
                        assert!(
                            frame_commands_are_finite(&frame.commands),
                            "{section_name}/{theme_name}/{width}x{height}@{scale}"
                        );
                        let mut renderer = SdlComponentRenderer::new_pixel_buffer(
                            (width as f32 * scale) as u32,
                            (height as f32 * scale) as u32,
                            scale,
                        );
                        assert!(!renderer.render(&frame.commands).is_empty());
                        assert!(renderer.pixels().iter().any(|pixel| pixel.a != 0));
                    }
                }
            }
        }
    }

    #[test]
    fn every_narrow_dashboard_page_rasterizes_at_supported_scales() {
        let mut launcher = Launcher::default();
        launcher.set_dashboard_account(DashboardSection::Ready(DashboardAccount {
            display_name: "Expanded Local Account Name".into(),
            supporting_text: "Sesión local".into(),
        }));
        launcher.set_dashboard_projects(DashboardSection::Ready(vec![DashboardProject {
            id: "arabic".into(),
            name: "مشروع سطح المكتب".into(),
            roots: vec![PathBuf::from("/projects/arabic")],
            chat_count: Some(4),
            activity: ProjectActivity::Active,
            last_used_at: Some(7),
        }]));
        let mut icons = LauncherIconCache::new();
        for page in [
            DashboardNarrowPage::Primary,
            DashboardNarrowPage::Projects,
            DashboardNarrowPage::Settings,
        ] {
            for scale in [1.0_f32, 1.25, 2.0] {
                let mut state = LauncherViewState {
                    dashboard_narrow_page: page,
                    ..LauncherViewState::default()
                };
                let frame =
                    build_launcher_frame(&launcher, &mut state, &mut icons, 360, 480, palette());
                assert!(
                    frame_commands_are_finite(&frame.commands),
                    "{page:?}@{scale}"
                );
                let mut renderer = SdlComponentRenderer::new_pixel_buffer(
                    (360.0 * scale) as u32,
                    (480.0 * scale) as u32,
                    scale,
                );
                assert!(!renderer.render(&frame.commands).is_empty());
                assert!(renderer.pixels().iter().any(|pixel| pixel.a != 0));
            }
        }
    }

    fn frame_commands_are_finite(commands: &[PaintCommand]) -> bool {
        commands.iter().all(|command| {
            let rect = match command {
                PaintCommand::Fill { rect, .. }
                | PaintCommand::TopRoundedFill { rect, .. }
                | PaintCommand::Gradient { rect, .. }
                | PaintCommand::RoundedFill { rect, .. }
                | PaintCommand::Stroke { rect, .. }
                | PaintCommand::OverlayFill { rect, .. }
                | PaintCommand::OverlayStroke { rect, .. } => Some(*rect),
                PaintCommand::Text { bounds, .. }
                | PaintCommand::StyledText { bounds, .. }
                | PaintCommand::Image { bounds, .. } => Some(*bounds),
                PaintCommand::PushClip(rect) => Some(*rect),
                PaintCommand::PopClip => None,
            };
            rect.is_none_or(|rect| {
                rect.origin.x.is_finite()
                    && rect.origin.y.is_finite()
                    && rect.size.width.is_finite()
                    && rect.size.height.is_finite()
                    && rect.size.width >= 0.0
                    && rect.size.height >= 0.0
            })
        })
    }
}
