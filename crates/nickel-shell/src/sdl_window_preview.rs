use std::{collections::HashMap, sync::Arc, time::Instant};

use nickel_core::theme::ThemePalette;
use nickel_ui::backend::PaintCommand;
use nickel_ui::{
    Align, AnyView, Application, Button, Collection, CollectionPresentation, CollectionState,
    ComponentBuilderExt, Container, HostBatch, HostChangeToken, HostEvent, HostEventOutcome, Image,
    Insets, NavigationScope, Point, Rect, Row, SemanticRole, Text, TextAlign, UiEvent, UiHost,
    ViewContext,
};

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
    Dismiss,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MenuAction {
    Close(WindowId),
    MaximizeRestore(WindowId),
    Minimize(WindowId),
    MoveToWorkspace(WindowId, u64),
}

pub struct WindowMenuApp {
    window: WindowId,
    workspaces: Vec<WorkspaceSummary>,
    selected: Option<usize>,
    palette: ThemePalette,
    effects: Vec<MenuAction>,
    dirty: bool,
}

impl WindowMenuApp {
    pub fn new(
        window: WindowId,
        workspaces: Vec<WorkspaceSummary>,
        selected: Option<usize>,
        palette: ThemePalette,
    ) -> Self {
        Self {
            window,
            workspaces,
            selected,
            palette,
            effects: Vec::new(),
            dirty: false,
        }
    }

    pub fn sync(
        &mut self,
        window: WindowId,
        workspaces: &[WorkspaceSummary],
        selected: Option<usize>,
        palette: ThemePalette,
    ) {
        self.window = window;
        self.workspaces = workspaces.to_vec();
        self.selected = selected;
        self.palette = palette;
        self.dirty = true;
    }

    pub fn take_effects(&mut self) -> Vec<MenuAction> {
        std::mem::take(&mut self.effects)
    }
}

impl Application for WindowMenuApp {
    type Message = MenuAction;

    fn update(&mut self, message: Self::Message) {
        self.effects.push(message);
    }

    fn view(&self, _context: ViewContext) -> impl nickel_ui::View<Self::Message> {
        let entries = window_menu_entries(self.window, &self.workspaces);
        let content = entries.into_iter().enumerate().fold(
            nickel_ui::Column::new().gap(MENU_ROW_GAP),
            |column, (index, (label, action))| {
                column.child(
                    Button::new(action, label)
                        .id(format!("window-menu-action-{index}"))
                        .height(MENU_ROW_HEIGHT)
                        .background(if self.selected == Some(index) {
                            self.palette.surface_hover
                        } else {
                            self.palette.panel
                        })
                        .color(self.palette.text),
                )
            },
        );
        Container::new()
            .id("window-menu-anchor")
            .width(MENU_WIDTH)
            .height(menu_height(&self.workspaces))
            .padding(Insets::all(MENU_PADDING))
            .background(self.palette.panel)
            .radius(10.0)
            .child(content)
    }

    fn poll(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }
}

fn window_menu_entries(
    window: WindowId,
    workspaces: &[WorkspaceSummary],
) -> Vec<(String, MenuAction)> {
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
    entries
}

pub struct WindowPreviewFrame {
    host: UiHost<WindowPreviewApp>,
    change_token: HostChangeToken,
    next_deadline: Option<Instant>,
    windows: Vec<WindowId>,
}

pub struct WindowPreviewApp {
    group: WindowGroup,
    previews: HashMap<WindowId, Arc<image::RgbaImage>>,
    hovered: Option<WindowId>,
    palette: ThemePalette,
    effects: Vec<PreviewAction>,
    dirty: bool,
}

impl Application for WindowPreviewApp {
    type Message = PreviewAction;

    fn update(&mut self, message: Self::Message) {
        self.effects.push(message);
    }

    fn view(&self, _context: ViewContext) -> impl nickel_ui::View<Self::Message> {
        preview_view(&self.group, &self.previews, self.hovered, self.palette)
    }

    fn poll(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    fn shortcut(&mut self, shortcut: nickel_ui::Shortcut) -> bool {
        if shortcut == nickel_ui::Shortcut::Escape {
            self.effects.push(PreviewAction::Dismiss);
            true
        } else {
            false
        }
    }
}

#[cfg(any(test, feature = "workbench-fixtures"))]
impl WindowPreviewApp {
    #[allow(dead_code)] // Binary and fixture library compile this shared module separately.
    pub fn fixture(
        group: WindowGroup,
        previews: HashMap<WindowId, Arc<image::RgbaImage>>,
        palette: ThemePalette,
    ) -> Self {
        Self {
            group,
            previews,
            hovered: None,
            palette,
            effects: Vec::new(),
            dirty: false,
        }
    }
}

impl WindowPreviewFrame {
    pub fn window(&self, index: usize) -> Option<WindowId> {
        self.windows.get(index).copied()
    }

    pub fn window_count(&self) -> usize {
        self.windows.len()
    }

    pub fn transition_pointer(&mut self, point: Point, right_click: bool) -> Option<PreviewAction> {
        let events = if right_click {
            vec![HostEvent::Ui(UiEvent::PointerContext(point))]
        } else {
            vec![
                HostEvent::Ui(UiEvent::PointerPressed(point)),
                HostEvent::Ui(UiEvent::PointerReleased(point)),
            ]
        };
        self.step(HostBatch {
            events,
            ..HostBatch::default()
        });
        self.host.application_mut().effects.drain(..).next()
    }

    pub fn window_at(&self, point: Point) -> Option<WindowId> {
        self.windows.iter().copied().find(|window| {
            [
                PreviewAction::Activate(*window),
                PreviewAction::OpenMenu(*window),
            ]
            .into_iter()
            .flat_map(|action| self.host.semantic_targets_for_message(&action))
            .any(|target| {
                point.x >= target.bounds.origin.x
                    && point.x <= target.bounds.origin.x + target.bounds.size.width
                    && point.y >= target.bounds.origin.y
                    && point.y <= target.bounds.origin.y + target.bounds.size.height
            })
        })
    }

    pub fn semantic_bounds(&self, action: PreviewAction) -> Option<Rect> {
        let action = match action {
            PreviewAction::OpenMenu(window) => PreviewAction::Activate(window),
            action => action,
        };
        self.host
            .semantic_targets_for_message(&action)
            .into_iter()
            .next()
            .map(|target| target.bounds)
    }

    pub fn commands(&self) -> &[PaintCommand] {
        self.host.commands()
    }

    pub fn sync(
        &mut self,
        group: &WindowGroup,
        previews: &HashMap<WindowId, Arc<image::RgbaImage>>,
        hovered: Option<WindowId>,
        palette: ThemePalette,
    ) -> HostEventOutcome {
        let (width, height) = preview_dimensions(group.windows.len());
        let app = self.host.application_mut();
        app.group = group.clone();
        app.previews = previews.clone();
        app.hovered = hovered;
        app.palette = palette;
        app.dirty = true;
        self.windows = group.windows.iter().map(|window| window.id).collect();
        self.windows.sort_by_key(|window| window.0);
        self.step(HostBatch {
            surface_size: Some((width, height)),
            events: vec![HostEvent::Poll],
            ..HostBatch::default()
        })
    }

    pub fn step(&mut self, batch: HostBatch) -> HostEventOutcome {
        let outcome = self.host.step(batch);
        self.change_token = outcome.change_token;
        self.next_deadline = outcome.next_deadline;
        outcome
    }

    pub fn take_actions(&mut self) -> Vec<PreviewAction> {
        std::mem::take(&mut self.host.application_mut().effects)
    }

    pub fn change_token(&self) -> HostChangeToken {
        self.change_token
    }

    pub fn next_deadline(&self) -> Option<Instant> {
        self.next_deadline
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
    let mut windows = group
        .windows
        .iter()
        .map(|window| window.id)
        .collect::<Vec<_>>();
    windows.sort_by_key(|window| window.0);
    WindowPreviewFrame {
        host: UiHost::new(
            WindowPreviewApp {
                group: group.clone(),
                previews: previews.clone(),
                hovered,
                palette,
                effects: Vec::new(),
                dirty: false,
            },
            width,
            height,
        ),
        change_token: HostChangeToken::default(),
        next_deadline: None,
        windows,
    }
}

fn preview_view(
    group: &WindowGroup,
    previews: &HashMap<WindowId, Arc<image::RgbaImage>>,
    hovered: Option<WindowId>,
    palette: ThemePalette,
) -> impl nickel_ui::View<PreviewAction> {
    let (width, height) = preview_dimensions(group.windows.len());
    let mut windows = group.windows.iter().collect::<Vec<_>>();
    windows.sort_by_key(|window| window.id.0);
    let window_ids = windows.iter().map(|window| window.id).collect::<Vec<_>>();
    let cards = windows
        .into_iter()
        .map(|window| {
            let image = previews.get(&window.id).cloned();
            (
                window.id,
                window_title(&window.title, &group.application_name).to_owned(),
                image,
            )
        })
        .collect::<Vec<_>>();
    let collection = Collection::try_new(
        CollectionState::Ready(cards),
        |(window, _, _)| window.0,
        move |(window, title, image)| {
            let preview: AnyView<PreviewAction> = image.map_or_else(
                || {
                    AnyView::new(
                        Container::new()
                            .width(CARD_WIDTH - 16.0)
                            .height(116.0)
                            .background(palette.background)
                            .radius(6.0),
                    )
                },
                |image| {
                    AnyView::new(
                        Image::new(preview_image_id(window), image)
                            .width(CARD_WIDTH - 16.0)
                            .height(116.0),
                    )
                },
            );
            let card = Container::new()
                .width(CARD_WIDTH)
                .height(PREVIEW_HEIGHT - PADDING * 2.0)
                .padding(Insets::all(8.0))
                .gap(2.0)
                .background(if hovered == Some(window) {
                    palette.surface_hover
                } else {
                    palette.surface
                })
                .radius(10.0)
                .child(
                    Row::new()
                        .height(CLOSE_SIZE)
                        .align_items(Align::Center)
                        .child(
                            Text::new(title.clone())
                                .scale(0.85)
                                .color(palette.text)
                                .align(TextAlign::Center)
                                .grow(1.0),
                        )
                        .child(
                            Container::new()
                                .width(CLOSE_SIZE)
                                .height(CLOSE_SIZE)
                                .background(palette.surface_hover)
                                .radius(CLOSE_SIZE / 2.0)
                                .message(PreviewAction::Close(window))
                                .semantic_role(SemanticRole::Button)
                                .accessibility_label(format!("Close {title}"))
                                .child(
                                    Text::new("×")
                                        .scale(1.1)
                                        .color(palette.text)
                                        .align(TextAlign::Center),
                                ),
                        ),
                )
                .child(
                    Container::new()
                        .message(PreviewAction::Activate(window))
                        .context_message(PreviewAction::OpenMenu(window))
                        .semantic_role(SemanticRole::Button)
                        .accessibility_label(title.clone())
                        .child(preview),
                );
            Container::new()
                .navigation_scope(NavigationScope::group())
                .child(card)
        },
    )
    .expect("window ids must be unique")
    .id("window-previews")
    .presentation(CollectionPresentation::UniformGrid {
        columns: window_ids.len().max(1),
    })
    .gap(GAP);
    Container::new()
        .width(width as f32)
        .height(height as f32)
        .padding(Insets::all(PADDING))
        .background(palette.panel)
        .radius(14.0)
        .child(collection)
}

pub fn menu_height(workspaces: &[WorkspaceSummary]) -> f32 {
    let destination_count = workspace_move_destinations(workspaces).len();
    let row_count = 3 + destination_count;
    MENU_PADDING * 2.0
        + row_count as f32 * MENU_ROW_HEIGHT
        + row_count.saturating_sub(1) as f32 * MENU_ROW_GAP
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

fn preview_image_id(window: WindowId) -> u16 {
    0x7000 | (window.0 as u16 & 0x0fff)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::OpenWindow;
    use nickel_core::theme::Appearance;
    use nickel_ui::{ActionKind, SemanticAction};

    fn center(rect: Rect) -> Point {
        Point {
            x: rect.origin.x + rect.size.width / 2.0,
            y: rect.origin.y + rect.size.height / 2.0,
        }
    }

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
        let mut frame = build_preview_frame(
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
            let point = center(
                frame
                    .semantic_bounds(action)
                    .unwrap_or_else(|| panic!("preview target exists for {action:?}")),
            );
            assert_eq!(
                frame.transition_pointer(point, matches!(action, PreviewAction::OpenMenu(_))),
                Some(action)
            );
            assert!(frame.change_token().frame_generation > 0);
        }
        assert_eq!(frame.window_count(), 2);
        assert_eq!(frame.window(0), Some(WindowId(4)));
        assert_eq!(frame.window(1), Some(WindowId(9)));
        assert_eq!(frame.window(2), None);
    }

    #[test]
    fn preview_targets_do_not_move_when_stacking_order_changes() {
        let palette = ThemePalette::from_appearance(Appearance::default());
        let original = group();
        let mut reordered = original.clone();
        reordered.windows.reverse();
        let first = build_preview_frame(&original, &HashMap::new(), None, palette);
        let second = build_preview_frame(&reordered, &HashMap::new(), None, palette);

        for window in [WindowId(4), WindowId(9)] {
            assert_eq!(
                first.semantic_bounds(PreviewAction::Close(window)),
                second.semantic_bounds(PreviewAction::Close(window))
            );
        }
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
        let mut host = UiHost::new(
            WindowMenuApp::new(
                WindowId(9),
                workspaces.to_vec(),
                None,
                ThemePalette::from_appearance(Appearance::default()),
            ),
            MENU_WIDTH as u32,
            menu_height(&workspaces) as u32,
        );
        for action in [
            MenuAction::Close(WindowId(9)),
            MenuAction::MaximizeRestore(WindowId(9)),
            MenuAction::Minimize(WindowId(9)),
            MenuAction::MoveToWorkspace(WindowId(9), 1),
            MenuAction::MoveToWorkspace(WindowId(9), 8),
        ] {
            let target = host
                .semantic_targets_for_message(&action)
                .into_iter()
                .next()
                .expect("menu target exists");
            host.perform_semantic_action(target.id, SemanticAction::Invoke(ActionKind::Activate));
            assert_eq!(host.application_mut().take_effects(), vec![action]);
        }
        assert_eq!(host.semantic_nodes().len(), 5);
    }

    #[test]
    fn empty_titles_fall_back_without_hiding_a_real_title() {
        assert_eq!(window_title(" Notes ", "Editor"), "Notes");
        assert_eq!(window_title("", "Editor"), "Editor");
        assert_eq!(window_title("", ""), "Untitled window");
    }
}
