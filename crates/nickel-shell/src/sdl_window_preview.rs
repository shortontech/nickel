use std::{collections::HashMap, sync::Arc};

use nickel_core::theme::ThemePalette;
use nickel_ui::{PaintCommand, Point, Rect, TextAlign};

use crate::{
    model::{WindowGroup, WindowId},
    platform::WorkspaceSummary,
};

pub const CARD_WIDTH: f32 = 276.0;
pub const PREVIEW_HEIGHT: f32 = 214.0;
const GAP: f32 = 10.0;
const PADDING: f32 = 12.0;
const CLOSE_SIZE: f32 = 28.0;
pub const MENU_WIDTH: f32 = 220.0;
const MENU_ROW_HEIGHT: f32 = 40.0;
const MENU_ROW_GAP: f32 = 4.0;
const MENU_PADDING: f32 = 8.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PreviewAction {
    Activate(WindowId),
    Close(WindowId),
    OpenMenu(WindowId),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MenuAction {
    Close(WindowId),
    MaximizeRestore(WindowId),
    Minimize(WindowId),
    MoveToWorkspace(WindowId, u64),
}

#[derive(Clone, Debug)]
pub struct WindowPreviewFrame {
    pub commands: Vec<PaintCommand>,
    cards: Vec<(WindowId, Rect, Rect)>,
}

impl WindowPreviewFrame {
    pub fn window(&self, index: usize) -> Option<WindowId> {
        self.cards.get(index).map(|(window, _, _)| *window)
    }

    pub fn window_count(&self) -> usize {
        self.cards.len()
    }

    pub fn action_at(&self, point: Point, right_click: bool) -> Option<PreviewAction> {
        self.cards.iter().find_map(|(window, card, close)| {
            if contains(*close, point) {
                Some(PreviewAction::Close(*window))
            } else if contains(*card, point) {
                Some(if right_click {
                    PreviewAction::OpenMenu(*window)
                } else {
                    PreviewAction::Activate(*window)
                })
            } else {
                None
            }
        })
    }

    pub fn window_at(&self, point: Point) -> Option<WindowId> {
        self.cards
            .iter()
            .find_map(|(window, card, _)| contains(*card, point).then_some(*window))
    }

    pub fn target_point(&self, action: PreviewAction) -> Option<Point> {
        let (window, close) = match action {
            PreviewAction::Activate(window) | PreviewAction::OpenMenu(window) => (window, false),
            PreviewAction::Close(window) => (window, true),
        };
        let (_, card, close_bounds) = self
            .cards
            .iter()
            .find(|(candidate, _, _)| *candidate == window)?;
        Some(center(if close { *close_bounds } else { *card }))
    }
}

#[derive(Clone, Debug)]
pub struct WindowMenuFrame {
    pub commands: Vec<PaintCommand>,
    rows: Vec<(Rect, MenuAction)>,
}

impl WindowMenuFrame {
    pub fn action(&self, index: usize) -> Option<MenuAction> {
        self.rows.get(index).map(|(_, action)| *action)
    }

    pub fn action_count(&self) -> usize {
        self.rows.len()
    }

    pub fn action_at(&self, point: Point) -> Option<MenuAction> {
        self.rows
            .iter()
            .find_map(|(row, action)| contains(*row, point).then_some(*action))
    }

    pub fn target_point(&self, action: MenuAction) -> Option<Point> {
        let (row, _) = self
            .rows
            .iter()
            .find(|(_, candidate)| *candidate == action)?;
        Some(center(*row))
    }
}

pub fn preview_dimensions(window_count: usize) -> (u32, u32) {
    let count = window_count.max(1) as f32;
    (
        (PADDING * 2.0 + count * CARD_WIDTH + (count - 1.0) * GAP).round() as u32,
        PREVIEW_HEIGHT.round() as u32,
    )
}

pub fn build_preview_frame(
    group: &WindowGroup,
    previews: &HashMap<WindowId, Arc<image::RgbaImage>>,
    hovered: Option<WindowId>,
    palette: ThemePalette,
) -> WindowPreviewFrame {
    let (width, height) = preview_dimensions(group.windows.len());
    let mut commands = vec![PaintCommand::RoundedFill {
        rect: Rect::new(0.0, 0.0, width as f32, height as f32),
        color: palette.panel,
        radius: 14.0,
    }];
    let mut cards = Vec::with_capacity(group.windows.len());
    for (index, window) in group.windows.iter().enumerate() {
        let x = PADDING + index as f32 * (CARD_WIDTH + GAP);
        let card = Rect::new(x, PADDING, CARD_WIDTH, PREVIEW_HEIGHT - PADDING * 2.0);
        let close = Rect::new(
            x + CARD_WIDTH - CLOSE_SIZE - 6.0,
            PADDING + 6.0,
            CLOSE_SIZE,
            CLOSE_SIZE,
        );
        commands.push(PaintCommand::RoundedFill {
            rect: card,
            color: if hovered == Some(window.id) {
                palette.surface_hover
            } else {
                palette.surface
            },
            radius: 10.0,
        });
        let image_bounds = Rect::new(x + 8.0, PADDING + 42.0, CARD_WIDTH - 16.0, 116.0);
        if let Some(image) = previews.get(&window.id) {
            commands.push(PaintCommand::Image {
                bounds: image_bounds,
                id: preview_image_id(window.id),
                image: Arc::clone(image),
                high_density: None,
            });
        } else {
            commands.push(PaintCommand::RoundedFill {
                rect: image_bounds,
                color: palette.background,
                radius: 6.0,
            });
        }
        commands.push(text(
            Rect::new(x + 10.0, PADDING + 8.0, CARD_WIDTH - 54.0, 28.0),
            window_title(&window.title, &group.application_name),
            0.85,
            palette.text,
        ));
        commands.push(PaintCommand::RoundedFill {
            rect: close,
            color: palette.surface_hover,
            radius: CLOSE_SIZE / 2.0,
        });
        commands.push(text(close, "×", 1.1, palette.text));
        cards.push((window.id, card, close));
    }
    WindowPreviewFrame { commands, cards }
}

pub fn menu_height(workspaces: &[WorkspaceSummary]) -> f32 {
    let destination_count = workspace_move_destinations(workspaces).len();
    let row_count = 3 + destination_count;
    MENU_PADDING * 2.0
        + row_count as f32 * MENU_ROW_HEIGHT
        + row_count.saturating_sub(1) as f32 * MENU_ROW_GAP
}

pub fn build_menu_frame(
    window: WindowId,
    workspaces: &[WorkspaceSummary],
    selected: Option<usize>,
    palette: ThemePalette,
) -> WindowMenuFrame {
    let mut entries = vec![
        ("Close".to_owned(), MenuAction::Close(window)),
        (
            "Maximize / Restore".to_owned(),
            MenuAction::MaximizeRestore(window),
        ),
        ("Minimize".to_owned(), MenuAction::Minimize(window)),
    ];
    entries.extend(
        workspace_move_destinations(workspaces)
            .into_iter()
            .map(|(label, workspace)| (label, MenuAction::MoveToWorkspace(window, workspace))),
    );
    let rows = entries
        .iter()
        .enumerate()
        .map(|(index, (_, action))| {
            (
                Rect::new(
                    MENU_PADDING,
                    MENU_PADDING + index as f32 * (MENU_ROW_HEIGHT + MENU_ROW_GAP),
                    MENU_WIDTH - MENU_PADDING * 2.0,
                    MENU_ROW_HEIGHT,
                ),
                *action,
            )
        })
        .collect::<Vec<_>>();
    let mut commands = vec![PaintCommand::RoundedFill {
        rect: Rect::new(0.0, 0.0, MENU_WIDTH, menu_height(workspaces)),
        color: palette.panel,
        radius: 10.0,
    }];
    for (index, ((row, _), (label, _))) in rows.iter().zip(&entries).enumerate() {
        if selected == Some(index) {
            commands.push(PaintCommand::RoundedFill {
                rect: *row,
                color: palette.surface_hover,
                radius: 7.0,
            });
        }
        commands.push(text(*row, label, 0.9, palette.text));
    }
    WindowMenuFrame { commands, rows }
}

fn workspace_move_destinations(workspaces: &[WorkspaceSummary]) -> Vec<(String, u64)> {
    let Some(active) = workspaces.iter().position(|workspace| workspace.active) else {
        return Vec::new();
    };
    let mut destinations = Vec::with_capacity(2);
    if let Some(previous) = active
        .checked_sub(1)
        .and_then(|index| workspaces.get(index))
    {
        destinations.push(("Move to previous workspace".to_owned(), previous.id));
    }
    if let Some(next) = workspaces.get(active + 1) {
        destinations.push(("Move to next workspace".to_owned(), next.id));
    }
    destinations
}

pub fn window_title<'a>(title: &'a str, application_name: &'a str) -> &'a str {
    let title = title.trim();
    if !title.is_empty() {
        title
    } else if !application_name.trim().is_empty() {
        application_name
    } else {
        "Untitled window"
    }
}

fn contains(rect: Rect, point: Point) -> bool {
    point.x >= rect.origin.x
        && point.y >= rect.origin.y
        && point.x < rect.origin.x + rect.size.width
        && point.y < rect.origin.y + rect.size.height
}

fn center(rect: Rect) -> Point {
    Point {
        x: rect.origin.x + rect.size.width / 2.0,
        y: rect.origin.y + rect.size.height / 2.0,
    }
}

fn preview_image_id(window: WindowId) -> u16 {
    0x7000 | (window.0 as u16 & 0x0fff)
}

fn text(bounds: Rect, value: &str, scale: f32, color: u32) -> PaintCommand {
    PaintCommand::Text {
        bounds,
        text: value.to_owned(),
        scale,
        color,
        align: TextAlign::Center,
        bold: false,
        wrap: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::OpenWindow;
    use nickel_core::theme::Appearance;

    fn group() -> WindowGroup {
        WindowGroup {
            application_id: None,
            application_name: "Editor".into(),
            windows: vec![
                OpenWindow {
                    id: WindowId(4),
                    application_id: None,
                    active: true,
                    title: "one".into(),
                },
                OpenWindow {
                    id: WindowId(9),
                    application_id: None,
                    active: false,
                    title: String::new(),
                },
            ],
        }
    }

    #[test]
    fn semantic_targets_resolve_through_production_preview_geometry() {
        let frame = build_preview_frame(
            &group(),
            &HashMap::new(),
            None,
            ThemePalette::from_appearance(Appearance::default()),
        );
        for action in [
            PreviewAction::Activate(WindowId(4)),
            PreviewAction::Close(WindowId(9)),
            PreviewAction::OpenMenu(WindowId(9)),
        ] {
            let point = frame.target_point(action).expect("preview target exists");
            assert_eq!(
                frame.action_at(point, matches!(action, PreviewAction::OpenMenu(_))),
                Some(action)
            );
        }
        assert_eq!(frame.window_count(), 2);
        assert_eq!(frame.window(0), Some(WindowId(4)));
        assert_eq!(frame.window(1), Some(WindowId(9)));
        assert_eq!(frame.window(2), None);
    }

    #[test]
    fn menu_targets_route_every_action_to_the_selected_window() {
        let workspaces = [
            WorkspaceSummary {
                id: 1,
                active: false,
            },
            WorkspaceSummary {
                id: 7,
                active: true,
            },
            WorkspaceSummary {
                id: 8,
                active: false,
            },
        ];
        let frame = build_menu_frame(
            WindowId(9),
            &workspaces,
            None,
            ThemePalette::from_appearance(Appearance::default()),
        );
        for action in [
            MenuAction::Close(WindowId(9)),
            MenuAction::MaximizeRestore(WindowId(9)),
            MenuAction::Minimize(WindowId(9)),
            MenuAction::MoveToWorkspace(WindowId(9), 1),
            MenuAction::MoveToWorkspace(WindowId(9), 8),
        ] {
            assert_eq!(
                frame.action_at(frame.target_point(action).expect("menu target exists")),
                Some(action)
            );
        }
        assert_eq!(frame.action_count(), 5);
        assert_eq!(frame.action(0), Some(MenuAction::Close(WindowId(9))));
        assert_eq!(
            frame.action(4),
            Some(MenuAction::MoveToWorkspace(WindowId(9), 8))
        );
        assert_eq!(frame.action(5), None);
    }

    #[test]
    fn empty_titles_fall_back_without_hiding_a_real_title() {
        assert_eq!(window_title(" Notes ", "Editor"), "Notes");
        assert_eq!(window_title("", "Editor"), "Editor");
        assert_eq!(window_title("", ""), "Untitled window");
    }
}
