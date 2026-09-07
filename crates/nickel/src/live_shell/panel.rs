use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use jiff::Zoned;
use nickel_core::theme::ThemePalette;
#[cfg(test)]
use nickel_ui::Rect;
use nickel_ui::{
    AnyView, Column, Container, DragGesture, DragPhase, FrameOverlay, Image, Insets, OverlayAnchor,
    OverlayMenu, OverlayMenuItem, Row, SemanticRole, Spacer, Text, TextAlign, UiId, ViewContext,
};

#[cfg(test)]
use super::PANEL_CONTROL_GAP;
use super::{
    PANEL_CLOCK_WIDTH, PANEL_CODEX_ICON_SIZE, PANEL_CODEX_WIDTH, PANEL_ITEM_WIDTH,
    PANEL_TRAY_ICON_SIZE, PANEL_TRAY_WIDTH,
};
use crate::{launcher::TaskbarApplication, model::TrayItem};

#[cfg(any(test, feature = "workbench-fixtures"))]
use crate::{launcher::Launcher, model::OpenWindow};

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct PanelStatusLayout {
    pub(super) control_start: f32,
    pub(super) tray_start: f32,
    pub(super) codex_start: f32,
}

#[cfg(test)]
impl PanelStatusLayout {
    pub(super) fn codex_icon_bounds(self) -> Rect {
        Rect::new(
            self.codex_start + (PANEL_CODEX_WIDTH - PANEL_CODEX_ICON_SIZE) / 2.0,
            (56.0 - PANEL_CODEX_ICON_SIZE) / 2.0,
            PANEL_CODEX_ICON_SIZE,
            PANEL_CODEX_ICON_SIZE,
        )
    }
}

#[cfg(test)]
pub(super) fn panel_status_layout(
    width: u32,
    tray_count: usize,
    codex_available: bool,
) -> PanelStatusLayout {
    let control_start = panel_control_start(width);
    let tray_start = control_start - tray_count.min(4) as f32 * PANEL_TRAY_WIDTH;
    PanelStatusLayout {
        control_start,
        tray_start,
        codex_start: tray_start
            - if codex_available {
                PANEL_CODEX_WIDTH
            } else {
                0.0
            },
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PanelHover {
    OnScreenKeyboard,
    Launcher,
    Task(usize),
    Codex,
    Tray(usize),
    Control,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PanelAction {
    OnScreenKeyboard,
    Launcher,
    Task(usize),
    TaskContext(usize),
    ToggleTaskPin(String),
    MoveTaskPinLeft(String),
    MoveTaskPinRight(String),
    TaskDrag(usize, DragGesture),
    Codex,
    Tray(String),
    TrayContext(String),
    Control,
}

#[derive(Clone)]
pub struct PanelApplication {
    pub(super) keyboard_enabled: bool,
    pub(super) keyboard_visible: bool,
    pub(super) groups: Arc<Vec<TaskbarApplication>>,
    pub(super) codex_available: bool,
    pub(super) tray: Vec<TrayItem>,
    pub(super) tray_icons: Vec<Arc<image::RgbaImage>>,
    pub(super) panel_icon: Arc<image::RgbaImage>,
    pub(super) codex_icon: Arc<image::RgbaImage>,
    pub(super) task_icons: Vec<Option<(u16, Arc<image::RgbaImage>)>>,
    pub(super) palette: ThemePalette,
    pub(super) panel_hover: Option<PanelHover>,
    pub(super) launcher_visible: bool,
    pub(super) codex_project_menu_visible: bool,
    pub(super) control_visible: bool,
    pub(super) clock: String,
    pub(super) date: String,
    pub(super) effects: Vec<PanelAction>,
    pub(super) task_drag: Option<(usize, isize)>,
}

fn map_task_drag(seed: PanelAction, gesture: DragGesture) -> PanelAction {
    let PanelAction::Task(index) = seed else {
        unreachable!("task drag seeds retain their task index")
    };
    PanelAction::TaskDrag(index, gesture)
}

impl nickel_ui::Application for PanelApplication {
    type Message = PanelAction;

    fn update(&mut self, message: Self::Message) {
        if let PanelAction::TaskDrag(index, gesture) = message {
            let direction = if gesture.position.x < gesture.bounds.origin.x {
                -1
            } else if gesture.position.x > gesture.bounds.origin.x + gesture.bounds.size.width {
                1
            } else {
                0
            };
            match gesture.phase {
                DragPhase::Started | DragPhase::Moved => {
                    self.task_drag = (direction != 0).then_some((index, direction));
                }
                DragPhase::Ended => {
                    let pending = self
                        .task_drag
                        .take()
                        .or((direction != 0).then_some((index, direction)));
                    if let Some((index, direction)) = pending
                        && let Some(id) = self
                            .groups
                            .get(index)
                            .filter(|task| task.pinned)
                            .and_then(|task| task.application_id.as_ref())
                    {
                        self.effects.push(if direction < 0 {
                            PanelAction::MoveTaskPinLeft(id.as_str().to_owned())
                        } else {
                            PanelAction::MoveTaskPinRight(id.as_str().to_owned())
                        });
                    }
                }
                DragPhase::Cancelled => self.task_drag = None,
            }
            return;
        }
        self.effects.push(message);
    }

    fn view(&self, context: ViewContext) -> impl nickel_ui::View<Self::Message> {
        self.panel_view(context.viewport.size.width, context.viewport.size.height)
    }

    fn frame_overlays(&self, _context: ViewContext) -> Vec<FrameOverlay<Self::Message>> {
        let tasks = &self.groups;
        let pinned_count = tasks.iter().take_while(|task| task.pinned).count();
        tasks
            .iter()
            .take(12)
            .enumerate()
            .filter_map(|(index, task)| {
                if !task.windows.is_empty() {
                    return None;
                }
                let application_id = task.application_id.as_ref()?;
                let id = application_id.as_str().to_owned();
                let mut menu = OverlayMenu::new(
                    format!("panel-task-menu-{id}"),
                    OverlayAnchor::InvocationTarget(UiId::new(format!("panel-task-{index}"))),
                );
                if task.pinned {
                    if index > 0 {
                        menu = menu.item(OverlayMenuItem::action(
                            "move-left",
                            "Move Left",
                            PanelAction::MoveTaskPinLeft(id.clone()),
                        ));
                    }
                    if index + 1 < pinned_count {
                        menu = menu.item(OverlayMenuItem::action(
                            "move-right",
                            "Move Right",
                            PanelAction::MoveTaskPinRight(id.clone()),
                        ));
                    }
                }
                Some(FrameOverlay::Menu(menu.item(OverlayMenuItem::action(
                    "toggle-pin",
                    if task.pinned {
                        "Unpin from Nickel Bar"
                    } else {
                        "Pin to Nickel Bar"
                    },
                    PanelAction::ToggleTaskPin(id),
                ))))
            })
            .collect()
    }

    fn title(&self) -> &str {
        "Nickel Panel"
    }

    fn poll(&mut self) -> bool {
        let (clock, date) = panel_clock_text();
        if self.clock == clock && self.date == date {
            return false;
        }
        self.clock = clock;
        self.date = date;
        true
    }

    fn poll_interval(&self) -> Option<Duration> {
        Some(duration_until_next_minute())
    }
}

#[cfg(any(test, feature = "workbench-fixtures"))]
impl PanelApplication {
    #[allow(dead_code)] // Binary and fixture library compile this shared module separately.
    pub fn fixture(launcher: Launcher, palette: ThemePalette) -> Self {
        let icon = Arc::new(image::RgbaImage::from_pixel(
            1,
            1,
            image::Rgba([120, 90, 220, 255]),
        ));
        let (clock, date) = panel_clock_text();
        Self {
            keyboard_enabled: false,
            keyboard_visible: false,
            groups: Arc::new(launcher.taskbar_applications(&[])),
            codex_available: launcher.codex_available(),
            tray: Vec::new(),
            tray_icons: Vec::new(),
            panel_icon: Arc::clone(&icon),
            codex_icon: icon,
            task_icons: Vec::new(),
            palette,
            panel_hover: None,
            launcher_visible: false,
            codex_project_menu_visible: false,
            control_visible: false,
            clock,
            date,
            effects: Vec::new(),
            task_drag: None,
        }
    }

    #[allow(dead_code)] // Used by the library workbench fixture, not the shell binary.
    pub fn populated_fixture(mut launcher: Launcher, palette: ThemePalette) -> Self {
        launcher.set_codex_available(true);
        let mut application = Self::fixture(launcher.clone(), palette);
        let windows = vec![
            OpenWindow {
                id: crate::model::WindowId(101),
                application_id: Some(crate::model::ApplicationId::new("fixture.browser")),
                active: true,
                title: "Fixture Browser".into(),
                state: crate::model::WindowState::default(),
            },
            OpenWindow {
                id: crate::model::WindowId(202),
                application_id: Some(crate::model::ApplicationId::new("fixture.editor")),
                active: false,
                title: "Fixture Editor".into(),
                state: crate::model::WindowState::default(),
            },
        ];
        application.groups = Arc::new(launcher.taskbar_applications(&windows));
        application.task_icons = [
            image::Rgba([40, 140, 240, 255]),
            image::Rgba([220, 90, 120, 255]),
        ]
        .into_iter()
        .enumerate()
        .map(|(index, color)| {
            Some((
                0x6000 + index as u16,
                Arc::new(image::RgbaImage::from_pixel(32, 32, color)),
            ))
        })
        .collect();
        application.tray = vec![TrayItem {
            id: "fixture-tray".into(),
            title: "Fixture notification icon".into(),
            icon: image::RgbaImage::from_pixel(18, 18, image::Rgba([80, 210, 140, 255])),
        }];
        application.tray_icons = application
            .tray
            .iter()
            .map(|item| Arc::new(item.icon.clone()))
            .collect();
        application
    }
}

pub(super) fn panel_clock_text() -> (String, String) {
    let now = Zoned::now();
    (
        now.strftime("%-I:%M %p").to_string(),
        now.strftime("%-m/%-d/%Y").to_string(),
    )
}

fn duration_until_next_minute() -> Duration {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let elapsed_in_minute = Duration::new(elapsed.as_secs() % 60, elapsed.subsec_nanos());
    Duration::from_secs(60)
        .saturating_sub(elapsed_in_minute)
        .max(Duration::from_millis(1))
}

impl PanelApplication {
    fn panel_view(&self, width: f32, height: f32) -> impl nickel_ui::View<PanelAction> {
        let interactive_background = |hovered: bool, active: bool| {
            if active {
                self.palette.accent_soft
            } else if hovered {
                self.palette.surface_hover
            } else {
                self.palette.panel
            }
        };
        let mut row = Row::new().width(width).height(height).child(
            Container::new()
                .id("panel-launcher")
                .accessibility_label("Open Nickel Start")
                .semantic_role(SemanticRole::Button)
                .message(PanelAction::Launcher)
                .width(PANEL_ITEM_WIDTH)
                .height(height)
                .padding(Insets::all(12.0))
                .background(interactive_background(
                    self.panel_hover == Some(PanelHover::Launcher),
                    self.launcher_visible,
                ))
                .radius(8.0)
                .child(
                    Image::new(2, Arc::clone(&self.panel_icon))
                        .width(32.0)
                        .height(32.0),
                ),
        );
        let groups = &self.groups;
        for (index, group) in groups.iter().take(12).enumerate() {
            let hovered = self.panel_hover == Some(PanelHover::Task(index));
            let icon = self.task_icons.get(index).cloned().flatten();
            let visual = if let Some((id, image)) = icon {
                AnyView::new(Image::new(id, image).width(32.0).height(32.0))
            } else {
                let initial = group
                    .application_name
                    .chars()
                    .next()
                    .unwrap_or('?')
                    .to_uppercase()
                    .to_string();
                AnyView::new(
                    Text::new(initial)
                        .height(32.0)
                        .scale(1.0)
                        .color(self.palette.text)
                        .align(TextAlign::Center)
                        .bold(true),
                )
            };
            let indicator_width = if group.active() { 20.0 } else { 12.0 };
            let indicator_color = if group.active() {
                self.palette.accent
            } else if group.windows.is_empty() {
                self.palette.panel
            } else {
                self.palette.muted
            };
            let label = if !group.available {
                format!("{} (Unavailable, pinned)", group.application_name)
            } else if group.pinned && group.windows.is_empty() {
                format!("{} (Pinned)", group.application_name)
            } else if group.pinned {
                format!("{} (Pinned, running)", group.application_name)
            } else {
                group.application_name.clone()
            };
            row = row.child(
                Container::new()
                    .id(format!("panel-task-{index}"))
                    .accessibility_label(label)
                    .semantic_role(SemanticRole::Button)
                    .message(PanelAction::Task(index))
                    .context_message(PanelAction::TaskContext(index))
                    .on_drag((PanelAction::Task(index), map_task_drag))
                    .width(PANEL_ITEM_WIDTH)
                    .height(height)
                    .padding(Insets {
                        top: 7.0,
                        right: 10.0,
                        bottom: 3.0,
                        left: 10.0,
                    })
                    .background(
                        if self.task_drag.is_some_and(|(dragged, _)| dragged == index) {
                            self.palette.accent_soft
                        } else {
                            interactive_background(hovered, group.active())
                        },
                    )
                    .radius(8.0)
                    .child(
                        Column::new()
                            .child(visual)
                            .child(Spacer::vertical(3.0))
                            .child(
                                Container::new()
                                    .align_self(nickel_ui::Align::Center)
                                    .width(indicator_width)
                                    .height(3.0)
                                    .background(indicator_color)
                                    .radius(1.5),
                            ),
                    ),
            );
        }
        row = row.child(Spacer::flex());
        if self.keyboard_enabled {
            row = row.child(
                Container::new()
                    .id("panel-on-screen-keyboard")
                    .accessibility_label("On-screen keyboard")
                    .semantic_role(SemanticRole::Button)
                    .message(PanelAction::OnScreenKeyboard)
                    .width(48.0)
                    .height(height)
                    .justify_content(nickel_ui::Justify::Center)
                    .align_items(nickel_ui::Align::Center)
                    .background(interactive_background(
                        self.panel_hover == Some(PanelHover::OnScreenKeyboard),
                        self.keyboard_visible,
                    ))
                    .radius(8.0)
                    .child(Text::new("⌨").scale(24.0).color(self.palette.text)),
            );
        }
        if self.codex_available {
            row = row.child(
                Container::new()
                    .id("panel-codex")
                    .accessibility_label("Codex projects")
                    .semantic_role(SemanticRole::Button)
                    .message(PanelAction::Codex)
                    .width(PANEL_CODEX_WIDTH)
                    .height(height)
                    .padding(Insets {
                        top: 14.0,
                        right: 4.0,
                        bottom: 14.0,
                        left: 4.0,
                    })
                    .background(interactive_background(
                        self.panel_hover == Some(PanelHover::Codex),
                        self.codex_project_menu_visible,
                    ))
                    .radius(8.0)
                    .child(
                        Image::new(0x5000, Arc::clone(&self.codex_icon))
                            .width(PANEL_CODEX_ICON_SIZE)
                            .height(PANEL_CODEX_ICON_SIZE),
                    ),
            );
        }
        for (index, item) in self.tray.iter().rev().take(4).rev().enumerate() {
            let Some(image) = self.tray_icons.get(index) else {
                continue;
            };
            row = row.child(
                Container::new()
                    .id(format!("panel-tray-{}", item.id))
                    .accessibility_label(&item.title)
                    .semantic_role(SemanticRole::Button)
                    .message(PanelAction::Tray(item.id.clone()))
                    .context_message(PanelAction::TrayContext(item.id.clone()))
                    .width(PANEL_TRAY_WIDTH)
                    .height(height)
                    .padding(Insets {
                        top: 19.0,
                        right: 5.0,
                        bottom: 19.0,
                        left: 5.0,
                    })
                    .background(interactive_background(
                        self.panel_hover == Some(PanelHover::Tray(index)),
                        false,
                    ))
                    .radius(7.0)
                    .child(
                        Image::new(0x6000 + index as u16, Arc::clone(image))
                            .width(18.0)
                            .height(18.0),
                    ),
            );
        }
        row = row.child(
            Container::new()
                .id("panel-control")
                .accessibility_label("Open Quick Settings")
                .semantic_role(SemanticRole::Button)
                .message(PanelAction::Control)
                .width(PANEL_CLOCK_WIDTH)
                .height(height)
                .padding(Insets {
                    top: 6.0,
                    right: 8.0,
                    bottom: 8.0,
                    left: 0.0,
                })
                .background(interactive_background(
                    self.panel_hover == Some(PanelHover::Control),
                    self.control_visible,
                ))
                .radius(8.0)
                .child(
                    Column::new()
                        .child(
                            Text::new(&self.clock)
                                .height(22.0)
                                .scale(1.0)
                                .color(self.palette.text)
                                .align(TextAlign::Center),
                        )
                        .child(
                            Text::new(&self.date)
                                .height(20.0)
                                .scale(0.72)
                                .color(self.palette.text)
                                .align(TextAlign::Center),
                        ),
                ),
        );
        Container::new()
            .width(width)
            .height(height)
            .background(self.palette.panel)
            .child(row)
    }
}

#[cfg(test)]
fn panel_control_start(width: u32) -> f32 {
    width as f32 - PANEL_CLOCK_WIDTH - PANEL_CONTROL_GAP
}

pub(super) fn panel_tray_icons(items: &[TrayItem]) -> Vec<Arc<image::RgbaImage>> {
    items
        .iter()
        .rev()
        .take(4)
        .rev()
        .map(|item| {
            Arc::new(crate::icons::resized(
                &item.icon,
                PANEL_TRAY_ICON_SIZE,
                PANEL_TRAY_ICON_SIZE,
            ))
        })
        .collect()
}

pub(super) fn normalize_tray_items(items: Vec<TrayItem>) -> Vec<TrayItem> {
    let keep_from = items.len().saturating_sub(4);
    items.into_iter().skip(keep_from).collect()
}

#[cfg(test)]
pub(super) fn visible_tray_item(items: &[TrayItem], visual_index: usize) -> Option<&TrayItem> {
    items.get(items.len().saturating_sub(4).checked_add(visual_index)?)
}

pub(super) fn tint_panel_icon(mut icon: image::RgbaImage, color: u32) -> image::RgbaImage {
    let tint = [
        ((color >> 16) & 0xff) as u8,
        ((color >> 8) & 0xff) as u8,
        (color & 0xff) as u8,
    ];
    for pixel in icon.pixels_mut() {
        let coverage = u8::MAX - pixel.0[0];
        pixel.0[3] = ((u16::from(pixel.0[3]) * u16::from(coverage)) / u16::from(u8::MAX)) as u8;
        pixel.0[..3].copy_from_slice(&tint);
    }
    icon
}
