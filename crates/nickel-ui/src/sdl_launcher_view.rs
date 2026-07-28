//! SDL-independent launcher scene and hit testing.
//!
//! `LiveShell` owns input and mutates [`Launcher`]. This module turns that state
//! into component paint commands and stable semantic actions.

use std::{collections::HashMap, sync::Arc};

use image::RgbaImage;
use nickel_components::{LinearGradient, PaintCommand, Point, Rect, TextAlign};
use nickel_core::theme::ThemePalette;

use crate::{
    icons,
    launcher::{Application, Launcher, LauncherView},
    platform,
};

const PANEL_MAX_WIDTH: f32 = 760.0;
const PANEL_MAX_HEIGHT: f32 = 560.0;
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
}

#[derive(Clone, Debug, PartialEq)]
pub struct LauncherHitTarget {
    pub bounds: Rect,
    pub action: LauncherAction,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LauncherViewState {
    pub scroll_row: usize,
    pub hovered: Option<LauncherAction>,
}

impl LauncherViewState {
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
}

impl LauncherFrame {
    pub fn action_at(&self, point: Point) -> Option<&LauncherAction> {
        self.hits
            .iter()
            .rev()
            .find(|target| contains(target.bounds, point))
            .map(|target| &target.action)
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
}

pub fn build_launcher_frame(
    launcher: &Launcher,
    state: &mut LauncherViewState,
    icons: &mut LauncherIconCache,
    viewport_width: u32,
    viewport_height: u32,
    palette: ThemePalette,
) -> LauncherFrame {
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
            });
        } else {
            commands.push(text(
                Rect::new(
                    bounds.origin.x,
                    bounds.origin.y + 12.0,
                    bounds.size.width,
                    ICON_SIZE,
                ),
                &application
                    .name()
                    .chars()
                    .next()
                    .unwrap_or('?')
                    .to_uppercase()
                    .to_string(),
                1.4,
                palette.text,
                TextAlign::Center,
                true,
            ));
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
    }
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
