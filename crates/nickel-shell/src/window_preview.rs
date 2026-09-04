use std::{collections::HashMap, sync::Arc, time::Instant};

use nickel_core::theme::ThemePalette;
use nickel_ui::backend::PaintCommand;
use nickel_ui::{
    ActionKind, Align, AnyView, Application, Button, Collection, CollectionPresentation,
    CollectionState, ComponentBuilderExt, Container, HostBatch, HostChangeToken, HostEvent,
    HostEventOutcome, Image, Insets, Point, Rect, Row, SemanticAction, SemanticRole, Text,
    TextAlign, UiEvent, UiHost, UiId, ViewContext,
};

use crate::{
    model::{ApplicationId, OpenWindow, WindowGroup, WindowId},
    platform::WorkspaceSummary,
};

pub const CARD_WIDTH: f32 = 276.0;
pub const PREVIEW_HEIGHT: f32 = 214.0;
const GAP: f32 = 10.0;
const PADDING: f32 = 12.0;
const CLOSE_SIZE: f32 = 28.0;
const CARD_PADDING: f32 = 8.0;
const CARD_GAP: f32 = 2.0;
const THUMBNAIL_HEIGHT: f32 = 116.0;
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MenuAction {
    ShowWorkspaces,
    ShowDisplays,
    Back,
    Activate(WindowId),
    Close(WindowId),
    MaximizeRestore(WindowId),
    Minimize(WindowId),
    FullscreenRestore(WindowId),
    MoveToWorkspace(WindowId, u64),
    MoveToDisplay(WindowId, String),
    NewWindow(ApplicationId),
    TogglePin(ApplicationId),
}

pub struct WindowMenuApp {
    window: OpenWindow,
    workspaces: Vec<WorkspaceSummary>,
    outputs: Vec<String>,
    application_id: Option<ApplicationId>,
    pinned: bool,
    palette: ThemePalette,
    effects: Vec<MenuAction>,
    dirty: bool,
    page: WindowMenuPage,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum WindowMenuPage {
    #[default]
    Root,
    Workspaces,
    Displays,
}

impl WindowMenuApp {
    pub fn new(
        window: OpenWindow,
        workspaces: Vec<WorkspaceSummary>,
        outputs: Vec<String>,
        application_id: Option<ApplicationId>,
        pinned: bool,
        palette: ThemePalette,
    ) -> Self {
        Self {
            window,
            workspaces,
            outputs,
            application_id,
            pinned,
            palette,
            effects: Vec::new(),
            dirty: false,
            page: WindowMenuPage::Root,
        }
    }

    pub fn sync(
        &mut self,
        window: &OpenWindow,
        workspaces: &[WorkspaceSummary],
        outputs: &[String],
        palette: ThemePalette,
    ) {
        debug_assert_eq!(self.window.id, window.id);
        self.window.state = window.state.clone();
        self.window.title.clone_from(&window.title);
        self.window.active = window.active;
        self.workspaces = workspaces.to_vec();
        self.outputs = outputs.to_vec();
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
        match message {
            MenuAction::ShowWorkspaces => {
                self.page = WindowMenuPage::Workspaces;
                self.dirty = true;
                return;
            }
            MenuAction::ShowDisplays => {
                self.page = WindowMenuPage::Displays;
                self.dirty = true;
                return;
            }
            MenuAction::Back => {
                self.page = WindowMenuPage::Root;
                self.dirty = true;
                return;
            }
            MenuAction::MoveToWorkspace(_, workspace)
                if self.window.state.workspace == Some(workspace) =>
            {
                return;
            }
            MenuAction::MoveToDisplay(_, ref output)
                if self.window.state.output.as_ref() == Some(output) =>
            {
                return;
            }
            _ => {}
        }
        self.effects.push(message);
    }

    fn view(&self, _context: ViewContext) -> impl nickel_ui::View<Self::Message> {
        let entries = match self.page {
            WindowMenuPage::Root => window_menu_entries(
                &self.window,
                &self.workspaces,
                &self.outputs,
                self.application_id.as_ref(),
                self.pinned,
            ),
            WindowMenuPage::Workspaces => workspace_menu_entries(&self.window, &self.workspaces),
            WindowMenuPage::Displays => display_menu_entries(&self.window, &self.outputs),
        };
        let content = entries.into_iter().enumerate().fold(
            nickel_ui::Column::new().gap(MENU_ROW_GAP),
            |column, (index, (label, action))| {
                column.child(
                    Button::new(action, label)
                        .id(format!("window-menu-action-{index}"))
                        .height(MENU_ROW_HEIGHT)
                        .background(self.palette.panel)
                        .focus_background_tint(self.palette.accent)
                        .controller_focus_background_tint(self.palette.accent)
                        .color(self.palette.text),
                )
            },
        );
        Container::new()
            .id("window-menu-anchor")
            .width(MENU_WIDTH)
            .height(menu_height_for_rows(window_menu_max_rows(
                &self.window,
                &self.workspaces,
                &self.outputs,
                self.application_id.as_ref(),
                self.pinned,
            )))
            .padding(Insets::all(MENU_PADDING))
            .background(self.palette.panel)
            .radius(10.0)
            .child(content)
    }

    fn poll(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }
}

pub(crate) fn window_menu_entries(
    window: &OpenWindow,
    workspaces: &[WorkspaceSummary],
    outputs: &[String],
    application_id: Option<&ApplicationId>,
    pinned: bool,
) -> Vec<(String, MenuAction)> {
    let id = window.id;
    let capabilities = window.state.capabilities;
    let mut entries = Vec::new();
    if capabilities.activate {
        entries.push((
            (if window.state.minimized {
                "Restore"
            } else {
                "Activate"
            })
            .into(),
            MenuAction::Activate(id),
        ));
    }
    if capabilities.minimize {
        entries.push((
            (if window.state.minimized {
                "Unminimize"
            } else {
                "Minimize"
            })
            .into(),
            if window.state.minimized {
                MenuAction::Activate(id)
            } else {
                MenuAction::Minimize(id)
            },
        ));
    }
    if capabilities.maximize {
        entries.push((
            (if window.state.maximized {
                "Restore from Maximized"
            } else {
                "Maximize"
            })
            .into(),
            MenuAction::MaximizeRestore(id),
        ));
    }
    if capabilities.fullscreen {
        entries.push((
            (if window.state.fullscreen {
                "Leave Fullscreen"
            } else {
                "Fullscreen"
            })
            .into(),
            MenuAction::FullscreenRestore(id),
        ));
    }
    if capabilities.move_workspace && !workspaces.is_empty() {
        entries.push(("Move to Workspace ›".into(), MenuAction::ShowWorkspaces));
    }
    if capabilities.move_display && outputs.len() > 1 {
        entries.push(("Move to Display ›".into(), MenuAction::ShowDisplays));
    }
    if let Some(application_id) = application_id {
        entries.push((
            "New Window".into(),
            MenuAction::NewWindow(application_id.clone()),
        ));
        entries.push((
            (if pinned { "Unpin" } else { "Pin" }).into(),
            MenuAction::TogglePin(application_id.clone()),
        ));
    }
    if capabilities.close {
        entries.push(("Close Window".to_owned(), MenuAction::Close(id)));
    }
    entries
}

fn workspace_menu_entries(
    window: &OpenWindow,
    workspaces: &[WorkspaceSummary],
) -> Vec<(String, MenuAction)> {
    let mut entries = vec![("‹ Window Actions".into(), MenuAction::Back)];
    entries.extend(workspace_move_destinations(workspaces).into_iter().map(
        |(label, workspace)| {
            let checked = window.state.workspace == Some(workspace);
            (
                format!("{}Workspace {label}", if checked { "✓ " } else { "" }),
                MenuAction::MoveToWorkspace(window.id, workspace),
            )
        },
    ));
    entries
}

fn display_menu_entries(window: &OpenWindow, outputs: &[String]) -> Vec<(String, MenuAction)> {
    let mut entries = vec![("‹ Window Actions".into(), MenuAction::Back)];
    entries.extend(outputs.iter().map(|output| {
        let checked = window.state.output.as_ref() == Some(output);
        (
            format!("{}{output}", if checked { "✓ " } else { "" }),
            MenuAction::MoveToDisplay(window.id, output.clone()),
        )
    }));
    entries
}

pub(crate) fn window_menu_max_rows(
    window: &OpenWindow,
    workspaces: &[WorkspaceSummary],
    outputs: &[String],
    application_id: Option<&ApplicationId>,
    pinned: bool,
) -> usize {
    window_menu_entries(window, workspaces, outputs, application_id, pinned)
        .len()
        .max(workspaces.len().saturating_add(1))
        .max(outputs.len().saturating_add(1))
}

pub struct WindowPreviewFrame {
    host: UiHost<WindowPreviewApp>,
    change_token: HostChangeToken,
    next_deadline: Option<Instant>,
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
    pub fn ensure_controller_selection(&mut self) -> bool {
        if self.host.inspect().controller_target.is_none() {
            return self
                .step(HostBatch {
                    events: vec![HostEvent::Controller(nickel_ui::ControllerAction::Right)],
                    ..HostBatch::default()
                })
                .changed;
        }
        false
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

    pub fn transition_pointer_hover(&mut self, point: Point) -> Option<WindowId> {
        self.step(HostBatch {
            events: vec![HostEvent::Ui(UiEvent::PointerMoved(point))],
            ..HostBatch::default()
        });
        let hovered = self.host.inspect().pointer_hover?;
        self.window_for_semantic_target(&hovered)
    }

    pub fn controller_selected_window(&self) -> Option<WindowId> {
        let selected = self.host.inspect().controller_target?;
        self.window_for_semantic_target(&selected)
    }

    pub fn close_controller_selected(&mut self) -> bool {
        let Some(window) = self.controller_selected_window() else {
            return false;
        };
        let Ok(target) = self
            .host
            .unique_semantic_target_for_message(&PreviewAction::Close(window))
        else {
            return false;
        };
        self.host
            .perform_semantic_action(target.id, SemanticAction::Invoke(ActionKind::Activate));
        true
    }

    fn window_for_semantic_target(&self, target: &UiId) -> Option<WindowId> {
        self.host
            .application()
            .group
            .windows
            .iter()
            .find_map(|window| {
                [
                    PreviewAction::Activate(window.id),
                    PreviewAction::Close(window.id),
                    PreviewAction::OpenMenu(window.id),
                ]
                .into_iter()
                .flat_map(|action| self.host.semantic_targets_for_message(&action))
                .any(|candidate| {
                    candidate.id == *target
                        || candidate
                            .id
                            .as_str()
                            .strip_prefix(target.as_str())
                            .is_some_and(|suffix| suffix.starts_with('/'))
                })
                .then_some(window.id)
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

#[cfg(any(test, target_os = "windows"))]
pub fn native_thumbnail_bounds(index: usize) -> (i32, i32, i32, i32) {
    let left = PADDING + index as f32 * (CARD_WIDTH + GAP) + CARD_PADDING;
    let top = PADDING + CARD_PADDING + CLOSE_SIZE + CARD_GAP;
    (
        left.round() as i32,
        top.round() as i32,
        (left + CARD_WIDTH - CARD_PADDING * 2.0).round() as i32,
        (top + THUMBNAIL_HEIGHT).round() as i32,
    )
}

pub fn build_preview_frame(
    group: &WindowGroup,
    previews: &HashMap<WindowId, Arc<image::RgbaImage>>,
    hovered: Option<WindowId>,
    palette: ThemePalette,
) -> WindowPreviewFrame {
    let (width, height) = preview_dimensions(group.windows.len());
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
    }
}

fn preview_view(
    group: &WindowGroup,
    previews: &HashMap<WindowId, Arc<image::RgbaImage>>,
    hovered: Option<WindowId>,
    palette: ThemePalette,
) -> impl nickel_ui::View<PreviewAction> {
    let (width, height) = preview_dimensions(group.windows.len());
    let windows = group.windows.iter().collect::<Vec<_>>();
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
                            .width(CARD_WIDTH - CARD_PADDING * 2.0)
                            .height(THUMBNAIL_HEIGHT)
                            .background(palette.background)
                            .radius(6.0),
                    )
                },
                |image| {
                    AnyView::new(
                        Image::new(preview_image_id(window), image)
                            .width(CARD_WIDTH - CARD_PADDING * 2.0)
                            .height(THUMBNAIL_HEIGHT),
                    )
                },
            );
            let card = Container::new()
                .width(CARD_WIDTH)
                .height(PREVIEW_HEIGHT - PADDING * 2.0)
                .padding(Insets::all(CARD_PADDING))
                .gap(CARD_GAP)
                .background(if hovered == Some(window) {
                    palette.surface_hover
                } else {
                    palette.surface
                })
                .border(
                    if hovered == Some(window) {
                        palette.accent
                    } else {
                        palette.surface
                    },
                    if hovered == Some(window) { 3.0 } else { 1.0 },
                )
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
            Container::new().child(card)
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
    let row_count = 4 + destination_count;
    MENU_PADDING * 2.0
        + row_count as f32 * MENU_ROW_HEIGHT
        + row_count.saturating_sub(1) as f32 * MENU_ROW_GAP
}

pub fn menu_height_for_rows(row_count: usize) -> f32 {
    MENU_PADDING * 2.0
        + row_count as f32 * MENU_ROW_HEIGHT
        + row_count.saturating_sub(1) as f32 * MENU_ROW_GAP
}

fn workspace_move_destinations(workspaces: &[WorkspaceSummary]) -> Vec<(String, u64)> {
    workspaces
        .iter()
        .enumerate()
        .map(|(index, workspace)| ((index + 1).to_string(), workspace.id))
        .collect()
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
    use nickel_ui::backend::PaintCommand;
    use nickel_ui::{ActionKind, ImagePresentation, SemanticAction, Size};

    #[test]
    fn native_thumbnails_follow_card_geometry() {
        assert_eq!(native_thumbnail_bounds(0), (20, 50, 280, 166));
        assert_eq!(native_thumbnail_bounds(1), (306, 50, 566, 166));
    }

    #[test]
    fn grouped_preview_source_shapes_do_not_change_card_interaction_geometry() {
        let palette = ThemePalette::from_appearance(Appearance::default());
        let expected_activation = build_preview_frame(&group(), &HashMap::new(), None, palette)
            .semantic_bounds(PreviewAction::Activate(WindowId(4)))
            .expect("activation target exists");

        for source in [(3440, 1440), (1920, 1080), (1200, 1200), (900, 1600)] {
            let mut previews = HashMap::new();
            previews.insert(
                WindowId(4),
                Arc::new(image::RgbaImage::new(source.0, source.1)),
            );
            let frame = build_preview_frame(&group(), &previews, None, palette);
            assert_eq!(
                frame.semantic_bounds(PreviewAction::Activate(WindowId(4))),
                Some(expected_activation),
                "source {source:?} must not move interaction geometry"
            );
        }
    }

    #[test]
    fn grouped_preview_commands_contain_and_center_every_source_shape() {
        let palette = ThemePalette::from_appearance(Appearance::default());
        let viewport = native_thumbnail_bounds(0);
        let viewport = Rect::new(
            viewport.0 as f32,
            viewport.1 as f32,
            (viewport.2 - viewport.0) as f32,
            (viewport.3 - viewport.1) as f32,
        );
        for source in [
            (3440, 1440),
            (1920, 1080),
            (1200, 900),
            (999, 999),
            (900, 1600),
        ] {
            let mut previews = HashMap::new();
            previews.insert(
                WindowId(4),
                Arc::new(image::RgbaImage::new(source.0, source.1)),
            );
            let frame = build_preview_frame(&group(), &previews, None, palette);
            let actual = frame
                .commands()
                .iter()
                .find_map(|command| match command {
                    PaintCommand::Image { id, bounds, .. }
                        if *id == preview_image_id(WindowId(4)) =>
                    {
                        Some(*bounds)
                    }
                    _ => None,
                })
                .expect("grouped preview emits its image command");
            let expected = ImagePresentation::default()
                .bounds(viewport, Size::new(source.0 as f32, source.1 as f32));
            assert_eq!(actual, expected, "source {source:?}");
        }
    }

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
                    state: crate::model::WindowState::default(),
                },
                OpenWindow {
                    id: WindowId(9),
                    application_id: None,
                    active: false,
                    title: String::new(),
                    state: crate::model::WindowState::default(),
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
    }

    #[test]
    fn hover_and_delete_resolve_from_host_semantics() {
        let mut frame = build_preview_frame(
            &group(),
            &HashMap::new(),
            None,
            ThemePalette::from_appearance(Appearance::default()),
        );
        let second = center(
            frame
                .semantic_bounds(PreviewAction::Activate(WindowId(9)))
                .expect("second preview target"),
        );
        assert_eq!(frame.transition_pointer_hover(second), Some(WindowId(9)));

        let _ = frame.ensure_controller_selection();
        assert_eq!(frame.controller_selected_window(), Some(WindowId(4)));
        frame.step(HostBatch {
            events: vec![HostEvent::Controller(nickel_ui::ControllerAction::Right)],
            ..HostBatch::default()
        });
        assert_eq!(frame.controller_selected_window(), Some(WindowId(9)));
        assert!(frame.close_controller_selected());
        assert_eq!(
            frame.take_actions(),
            vec![PreviewAction::Close(WindowId(9))]
        );
    }

    #[test]
    fn transient_selection_has_no_parallel_index_or_hit_test_authority() {
        let live_shell = include_str!("live_shell.rs");
        let preview = include_str!("window_preview.rs");
        for removed in [
            ["preview", "_selected"].concat(),
            ["window_menu", "_selected"].concat(),
        ] {
            assert!(
                !live_shell.contains(&removed),
                "legacy field returned: {removed}"
            );
        }
        let manual_hit_test = ["window", "_at("].concat();
        assert!(
            !preview.contains(&manual_hit_test),
            "preview hit testing must remain owned by UiHost"
        );
        let menu_index = ["selected", ": Option<usize>"].concat();
        assert!(
            !preview.contains(&menu_index),
            "context-menu selection must remain owned by UiHost"
        );
    }

    #[test]
    fn preview_targets_follow_task_switcher_order() {
        let palette = ThemePalette::from_appearance(Appearance::default());
        let original = group();
        let mut reordered = original.clone();
        reordered.windows.reverse();
        let first = build_preview_frame(&original, &HashMap::new(), None, palette);
        let second = build_preview_frame(&reordered, &HashMap::new(), None, palette);

        assert_eq!(
            first.semantic_bounds(PreviewAction::Close(WindowId(4))),
            second.semantic_bounds(PreviewAction::Close(WindowId(9)))
        );
        assert_eq!(
            first.semantic_bounds(PreviewAction::Close(WindowId(9))),
            second.semantic_bounds(PreviewAction::Close(WindowId(4)))
        );
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
        let window = OpenWindow {
            id: WindowId(9),
            application_id: Some(ApplicationId::new("editor")),
            active: false,
            title: "Document".into(),
            state: crate::model::WindowState {
                workspace: Some(7),
                output: Some("left".into()),
                capabilities: crate::model::WindowCapabilities {
                    fullscreen: true,
                    move_workspace: true,
                    move_display: true,
                    ..Default::default()
                },
                ..Default::default()
            },
        };
        let outputs = vec!["left".into(), "right".into()];
        let mut host = UiHost::new(
            WindowMenuApp::new(
                window.clone(),
                workspaces.to_vec(),
                outputs.clone(),
                window.application_id.clone(),
                false,
                ThemePalette::from_appearance(Appearance::default()),
            ),
            MENU_WIDTH as u32,
            menu_height_for_rows(window_menu_max_rows(
                &window,
                &workspaces,
                &outputs,
                window.application_id.as_ref(),
                false,
            )) as u32,
        );
        for action in [
            MenuAction::Activate(WindowId(9)),
            MenuAction::Minimize(WindowId(9)),
            MenuAction::MaximizeRestore(WindowId(9)),
            MenuAction::FullscreenRestore(WindowId(9)),
            MenuAction::NewWindow(ApplicationId::new("editor")),
            MenuAction::TogglePin(ApplicationId::new("editor")),
            MenuAction::Close(WindowId(9)),
        ] {
            let target = host
                .semantic_targets_for_message(&action)
                .into_iter()
                .next()
                .expect("menu target exists");
            host.perform_semantic_action(target.id, SemanticAction::Invoke(ActionKind::Activate));
            assert_eq!(host.application_mut().take_effects(), vec![action]);
        }
        let root = window_menu_entries(
            &window,
            &workspaces,
            &outputs,
            window.application_id.as_ref(),
            false,
        );
        assert!(
            root.iter()
                .any(|(_, action)| *action == MenuAction::ShowWorkspaces)
        );
        assert!(
            root.iter()
                .any(|(_, action)| *action == MenuAction::ShowDisplays)
        );
        let workspace_entries = workspace_menu_entries(&window, &workspaces);
        assert!(
            workspace_entries
                .iter()
                .any(|(label, _)| label == "✓ Workspace 2")
        );
        let display_entries = display_menu_entries(&window, &outputs);
        assert!(display_entries.iter().any(|(label, _)| label == "✓ left"));
        host.application_mut()
            .update(MenuAction::MoveToWorkspace(WindowId(9), 7));
        host.application_mut()
            .update(MenuAction::MoveToDisplay(WindowId(9), "left".into()));
        assert!(host.application_mut().take_effects().is_empty());
    }

    #[test]
    fn open_menu_refreshes_state_and_topology_without_retargeting() {
        let mut captured = OpenWindow {
            id: WindowId(9),
            application_id: Some(ApplicationId::new("editor")),
            active: false,
            title: "Old title".into(),
            state: crate::model::WindowState::default(),
        };
        let mut menu = WindowMenuApp::new(
            captured.clone(),
            vec![],
            vec!["left".into()],
            captured.application_id.clone(),
            false,
            ThemePalette::from_appearance(Appearance::default()),
        );
        captured.active = true;
        captured.title = "New title".into();
        captured.state.maximized = true;
        captured.state.workspace = Some(12);
        captured.state.output = Some("right".into());
        captured.state.capabilities.move_workspace = true;
        captured.state.capabilities.move_display = true;
        let workspaces = vec![WorkspaceSummary {
            id: 12,
            active: true,
        }];
        let outputs = vec!["left".into(), "right".into()];

        menu.sync(
            &captured,
            &workspaces,
            &outputs,
            ThemePalette::from_appearance(Appearance::default()),
        );

        assert_eq!(menu.window.id, WindowId(9));
        assert_eq!(menu.window.title, "New title");
        assert!(
            window_menu_entries(
                &menu.window,
                &menu.workspaces,
                &menu.outputs,
                menu.application_id.as_ref(),
                menu.pinned,
            )
            .iter()
            .any(|(label, action)| label == "Restore from Maximized"
                && *action == MenuAction::MaximizeRestore(WindowId(9)))
        );
        assert!(
            workspace_menu_entries(&menu.window, &menu.workspaces)
                .iter()
                .any(|(label, _)| label == "✓ Workspace 1")
        );
        assert!(
            display_menu_entries(&menu.window, &menu.outputs)
                .iter()
                .any(|(label, _)| label == "✓ right")
        );
    }

    #[test]
    fn empty_titles_fall_back_without_hiding_a_real_title() {
        assert_eq!(window_title(" Notes ", "Editor"), "Notes");
        assert_eq!(window_title("", "Editor"), "Editor");
        assert_eq!(window_title("", ""), "Untitled window");
    }

    #[test]
    fn menu_model_uses_inverse_labels_and_omits_unsupported_commands() {
        let window = OpenWindow {
            id: WindowId(44),
            application_id: None,
            active: false,
            title: "Player".into(),
            state: crate::model::WindowState {
                minimized: true,
                maximized: true,
                fullscreen: true,
                workspace: Some(2),
                output: Some("right".into()),
                capabilities: crate::model::WindowCapabilities {
                    activate: true,
                    close: true,
                    minimize: true,
                    maximize: true,
                    fullscreen: true,
                    move_workspace: false,
                    move_display: false,
                },
            },
        };
        let entries = window_menu_entries(
            &window,
            &[WorkspaceSummary {
                id: 2,
                active: true,
            }],
            &["left".into(), "right".into()],
            None,
            false,
        );
        let labels = entries
            .iter()
            .map(|(label, _)| label.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            labels,
            [
                "Restore",
                "Unminimize",
                "Restore from Maximized",
                "Leave Fullscreen",
                "Close Window"
            ]
        );
        assert!(!entries.iter().any(|(_, action)| matches!(
            action,
            MenuAction::MoveToWorkspace(..)
                | MenuAction::MoveToDisplay(..)
                | MenuAction::NewWindow(..)
                | MenuAction::TogglePin(..)
        )));
    }
}
