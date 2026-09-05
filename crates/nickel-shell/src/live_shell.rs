use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use jiff::Zoned;
use nickel_core::task_switcher::{SwitchWindow, TaskSwitchEffect, TaskSwitcher};
use nickel_core::{
    launcher_preferences::LauncherPreferences,
    shell_settings::ShellSettings,
    theme::{Appearance, ThemePalette},
    wallpaper_settings::WallpaperSettings,
};
use nickel_file::{
    DirectoryBrowser, DirectoryWatch,
    desktop::{
        Arrangement as DesktopArrangement, DesktopEntryId, DesktopFileAction, DesktopLayout,
        DesktopOutput, FolderGrouping, Point as DesktopPoint, Rect as DesktopRect,
        SelectionModifiers, SortDirection as DesktopSortDirection, SortKey as DesktopSortKey,
    },
};
use nickel_session_protocol::ShellRole;
#[cfg(any(target_os = "linux", test))]
use nickel_session_protocol::{
    AnchorSide, Geometry, PointerInteraction, PreviewTargetAction, ResolvedShellTarget,
    ShellPopoverAnchor, ShellSemanticTarget, WindowMenuTargetAction,
};
use nickel_ui::Rect;
use nickel_ui::backend::PaintCommand;
use nickel_ui::{
    AnyView, Column, Container, ControllerAction, DragGesture, DragPhase, FilePlaneItem,
    FrameOverlay, HostBatch, HostChangeToken, HostEvent, Image, ImageFit, Insets, Layer,
    OverlayAnchor, OverlayMenu, OverlayMenuItem, Point, Row, SemanticRole, Shortcut, Spacer, Text,
    TextAlign, TextField, UiEvent, UiId, ViewContext,
};

use crate::{
    control_view::{ControlAction, ControlCenterApp, ControlCenterHost},
    launcher::{DashboardAccount, DashboardProject, DashboardSection, Launcher},
    launcher_view::{
        LauncherAction, LauncherApplication, LauncherIconCache, LauncherShellEffect,
        LauncherViewState, reduce_launcher_action,
    },
    model::{Application, OpenWindow, TrayItem, WindowGroup},
    notification::DesktopNotification,
    notification_view::{NotificationApp, NotificationEffect, NotificationHost},
    platform::{
        self, AudioStatus, BluetoothStatus, FeedState, FeedStatus, NetworkStatus, NotificationFeed,
        NotificationSource, ShellCommand, TrayFeed, TraySource, WindowAction, WindowFeed,
    },
    screenshot::ScreenshotTool,
    window_preview::{
        MENU_WIDTH, MenuAction, PreviewAction, WindowMenuApp, WindowPreviewFrame,
        build_preview_frame, menu_height, menu_height_for_rows, preview_dimensions,
        semantic_theme_from_palette, window_menu_max_rows,
    },
    winit_shell::SurfaceRole,
};
use nickel_input::KeyCode;
use zeroize::{Zeroize, Zeroizing};

fn launcher_controller_host_event(action: ControllerAction, overlay_open: bool) -> HostEvent {
    if action == ControllerAction::Cancel && !overlay_open {
        HostEvent::Shortcut(Shortcut::Escape)
    } else {
        HostEvent::Controller(action)
    }
}

const PANEL_ITEM_WIDTH: f32 = 52.0;
const PANEL_CLOCK_WIDTH: f32 = 96.0;
#[cfg(test)]
const PANEL_CONTROL_GAP: f32 = 8.0;
const PANEL_TRAY_WIDTH: f32 = 28.0;
const PANEL_TRAY_ICON_SIZE: u32 = 18;
const PANEL_CODEX_WIDTH: f32 = 36.0;
const PANEL_CODEX_ICON_SIZE: f32 = 28.0;
const PREVIEW_LEAVE_DELAY: Duration = Duration::from_millis(500);
const PREVIEW_HOVER_DELAY: Duration = Duration::from_millis(350);
const PREVIEW_REFRESH_INTERVAL: Duration = Duration::from_millis(500);
#[cfg(target_os = "linux")]
const RECURRING_DIAGNOSTIC_INTERVAL: Duration = Duration::from_secs(30);
const WALLPAPER_MAX_WIDTH: u32 = 7680;
const WALLPAPER_MAX_HEIGHT: u32 = 4320;
const PREVIEW_CACHE_CAPACITY: usize = 32;

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq)]
struct PanelStatusLayout {
    control_start: f32,
    tray_start: f32,
    codex_start: f32,
}

#[cfg(test)]
impl PanelStatusLayout {
    fn codex_icon_bounds(self) -> Rect {
        Rect::new(
            self.codex_start + (PANEL_CODEX_WIDTH - PANEL_CODEX_ICON_SIZE) / 2.0,
            (56.0 - PANEL_CODEX_ICON_SIZE) / 2.0,
            PANEL_CODEX_ICON_SIZE,
            PANEL_CODEX_ICON_SIZE,
        )
    }
}

#[cfg(test)]
fn panel_status_layout(width: u32, tray_count: usize, codex_available: bool) -> PanelStatusLayout {
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
enum PanelHover {
    Launcher,
    Task(usize),
    Codex,
    Tray(usize),
    Control,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PanelAction {
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

#[derive(Clone, Debug, PartialEq)]
struct PendingPopoverAnchor {
    role: ShellRole,
    control: String,
    output: String,
    bounds: Rect,
}

pub struct DesktopApplication {
    wallpaper: Option<Arc<image::RgbaImage>>,
    palette: ThemePalette,
    browser: Option<DirectoryBrowser>,
    watch: Option<DirectoryWatch>,
    layout: DesktopLayout,
    active_output: String,
    output_origin: DesktopPoint,
    active_scale: f32,
    icon_cache: HashMap<std::path::PathBuf, Arc<image::RgbaImage>>,
    pointer_down: Option<(DesktopEntryId, DesktopPoint)>,
    /// Pointer position at which the last snapped desktop move was committed.
    /// Keeping this separate from every motion event lets ordinary small motion
    /// accumulate until it crosses a grid-cell boundary.
    drag_commit_position: Option<DesktopPoint>,
    selection_start: Option<DesktopPoint>,
    pointer_position: DesktopPoint,
    pointer_seen: bool,
    pointer_dragged: bool,
    last_click: Option<(DesktopEntryId, Instant)>,
    modifiers: SelectionModifiers,
    context_menu: Option<DesktopMenuContext>,
    topology_generation: u64,
    directory_generation: u64,
    outputs: Vec<DesktopOutput>,
    workspace: Option<u64>,
    persist_layout: bool,
    operation_tx: std::sync::mpsc::Sender<Result<String, String>>,
    operation_rx: std::sync::mpsc::Receiver<Result<String, String>>,
    paste_in_progress: bool,
    error: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
struct DesktopMenuContext {
    anchor: Option<DesktopPoint>,
    entry: Option<DesktopEntryId>,
    output: String,
    topology_generation: u64,
    directory_generation: u64,
    selection: std::collections::HashSet<DesktopEntryId>,
    workspace: Option<u64>,
    paste_available: bool,
    desktop_writable: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DesktopCommand {
    IconsVisible(bool),
    IconSize(f32, f32),
    Sort(DesktopSortKey, DesktopSortDirection),
    FolderGrouping(FolderGrouping),
    Manual,
    AlignGrid,
    AutoArrange,
    Refresh,
    Paste,
    NewFolder,
    DisplaySettings,
    Personalize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum SettingsDestination {
    Appearance,
    Display { output: String },
}

impl SettingsDestination {
    fn arguments(&self) -> Vec<String> {
        match self {
            Self::Appearance => vec!["--screen".into(), "appearance".into()],
            Self::Display { output } => vec![
                "--screen".into(),
                "display".into(),
                "--output".into(),
                output.clone(),
            ],
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum DesktopMessage {
    Activate(DesktopEntryId),
    Context(DesktopEntryId),
    Cut(DesktopEntryId),
    Copy(DesktopEntryId),
    Rename(DesktopEntryId),
    Properties(DesktopEntryId),
    BackgroundContext,
    Command(DesktopCommand),
}

impl DesktopApplication {
    fn new(wallpaper: Option<Arc<image::RgbaImage>>, palette: ThemePalette) -> Self {
        let (operation_tx, operation_rx) = std::sync::mpsc::channel();
        let path = nickel_file::desktop_directory();
        let browser = DirectoryBrowser::open(&path).ok();
        let watch = DirectoryWatch::start(&path).ok();
        let mut layout = DesktopLayout::new(Vec::new());
        if let Some(browser) = &browser {
            layout.reconcile(desktop_snapshot(browser));
            let _ = layout.restore(desktop_layout_path());
        }
        Self {
            wallpaper,
            palette,
            browser,
            watch,
            layout,
            active_output: "primary".into(),
            output_origin: DesktopPoint::default(),
            active_scale: 1.0,
            icon_cache: HashMap::new(),
            pointer_down: None,
            drag_commit_position: None,
            selection_start: None,
            pointer_position: DesktopPoint::default(),
            pointer_seen: false,
            pointer_dragged: false,
            last_click: None,
            modifiers: SelectionModifiers::default(),
            context_menu: None,
            topology_generation: 0,
            directory_generation: 0,
            outputs: Vec::new(),
            workspace: None,
            persist_layout: true,
            operation_tx,
            operation_rx,
            paste_in_progress: false,
            error: None,
        }
    }

    fn refresh_directory(&mut self, force: bool) -> bool {
        let invalidated = force
            || self
                .watch
                .as_ref()
                .is_some_and(DirectoryWatch::take_invalidation);
        if !invalidated {
            return false;
        }
        let Some(browser) = &mut self.browser else {
            return false;
        };
        match browser.refresh() {
            Ok(()) => {
                let snapshot = desktop_snapshot(browser);
                self.layout.reconcile(snapshot);
                self.directory_generation = self.directory_generation.wrapping_add(1);
                if self
                    .context_menu
                    .as_ref()
                    .is_some_and(|menu| menu.directory_generation != self.directory_generation)
                {
                    self.context_menu = None;
                }
                self.icon_cache.clear();
                self.error = None;
                true
            }
            Err(error) => {
                self.error = Some(format!("Desktop could not be refreshed: {error}"));
                true
            }
        }
    }

    fn set_outputs(&mut self, outputs: Vec<DesktopOutput>) {
        if self.outputs == outputs {
            return;
        }
        self.topology_generation = self.topology_generation.wrapping_add(1);
        let menu_output_exists = self
            .context_menu
            .as_ref()
            .is_none_or(|menu| outputs.iter().any(|output| output.id == menu.output));
        self.outputs.clone_from(&outputs);
        self.layout.set_outputs(outputs);
        if self.context_menu.as_ref().is_some_and(|menu| {
            menu.topology_generation != self.topology_generation || !menu_output_exists
        }) {
            self.context_menu = None;
        }
    }

    fn set_active_output(&mut self, id: String, origin: DesktopPoint, scale: f32) {
        if self
            .context_menu
            .as_ref()
            .is_some_and(|menu| menu.output != id)
        {
            self.context_menu = None;
        }
        self.active_output = id;
        self.output_origin = origin;
        self.active_scale = scale.max(1.0);
    }

    fn set_workspace(&mut self, workspace: Option<u64>) {
        if self.workspace != workspace {
            self.workspace = workspace;
            self.context_menu = None;
        }
    }

    fn save_layout(&mut self) {
        if !self.persist_layout {
            return;
        }
        if let Err(error) = self.layout.save(desktop_layout_path()) {
            self.error = Some(format!("Desktop arrangement could not be saved: {error}"));
        }
    }

    fn hit(&self, local: DesktopPoint) -> Option<DesktopEntryId> {
        if !self.layout.icons_visible() {
            return None;
        }
        let (cell_width, cell_height) = self.layout.grid();
        let global = DesktopPoint {
            x: local.x + self.output_origin.x,
            y: local.y + self.output_origin.y,
        };
        self.layout
            .items()
            .iter()
            .rev()
            .find(|item| {
                item.output == self.active_output
                    && global.x >= item.position.x
                    && global.x < item.position.x + cell_width
                    && global.y >= item.position.y
                    && global.y < item.position.y + cell_height
            })
            .map(|item| item.id)
    }

    fn pointer_press(
        &mut self,
        local: DesktopPoint,
        secondary: bool,
        modifiers: SelectionModifiers,
    ) -> bool {
        self.pointer_position = local;
        self.pointer_seen = true;
        let hit = self.hit(local);
        if secondary {
            if let Some(id) = hit
                && !self.layout.selected().contains(&id)
            {
                self.layout.select(id, SelectionModifiers::default());
            }
            self.context_menu = Some(DesktopMenuContext {
                anchor: Some(local),
                entry: hit,
                output: self.active_output.clone(),
                topology_generation: self.topology_generation,
                directory_generation: self.directory_generation,
                selection: self.layout.selected().clone(),
                workspace: self.workspace,
                paste_available: hit.is_none() && nickel_file::native_file_clipboard_available(),
                desktop_writable: hit.is_none()
                    && nickel_file::directory_is_writable(&nickel_file::desktop_directory()),
            });
            return true;
        }
        self.context_menu = None;
        if let Some(id) = hit {
            self.layout.select(id, modifiers);
            self.pointer_down = Some((id, local));
            self.drag_commit_position = Some(local);
            self.selection_start = None;
            self.pointer_dragged = false;
        } else {
            if !modifiers.toggle {
                self.layout.clear_selection();
            }
            self.pointer_down = None;
            self.drag_commit_position = None;
            self.selection_start = Some(local);
        }
        true
    }

    fn pointer_motion(&mut self, local: DesktopPoint) -> bool {
        self.pointer_position = local;
        self.pointer_seen = true;
        let Some((id, pressed)) = self.pointer_down else {
            let Some(start) = self.selection_start else {
                return false;
            };
            let x = start.x.min(local.x) + self.output_origin.x;
            let y = start.y.min(local.y) + self.output_origin.y;
            self.layout.select_region(
                DesktopRect {
                    x,
                    y,
                    width: (local.x - start.x).abs(),
                    height: (local.y - start.y).abs(),
                },
                self.modifiers.toggle,
            );
            return true;
        };
        let committed = self.drag_commit_position.unwrap_or(pressed);
        let delta = DesktopPoint {
            x: local.x - committed.x,
            y: local.y - committed.y,
        };
        if delta.x.abs() < 2.0 && delta.y.abs() < 2.0 {
            return false;
        }
        let (cell_width, cell_height) = self.layout.grid();
        if delta.x.abs() < cell_width / 2.0 && delta.y.abs() < cell_height / 2.0 {
            return false;
        }
        let snapped_delta = DesktopPoint {
            x: (delta.x / cell_width).round() * cell_width,
            y: (delta.y / cell_height).round() * cell_height,
        };
        self.layout
            .move_group(id, snapped_delta, &self.active_output);
        self.drag_commit_position = Some(DesktopPoint {
            x: committed.x + snapped_delta.x,
            y: committed.y + snapped_delta.y,
        });
        self.pointer_dragged = true;
        true
    }

    fn pointer_release(&mut self, local: DesktopPoint, now: Instant) -> bool {
        if self.selection_start.take().is_some() {
            return true;
        }
        let Some((id, pressed)) = self.pointer_down.take() else {
            return false;
        };
        self.drag_commit_position = None;
        let moved = self.pointer_dragged
            || (local.x - pressed.x).abs() >= 2.0
            || (local.y - pressed.y).abs() >= 2.0;
        self.pointer_dragged = false;
        if moved {
            self.save_layout();
        } else if self.last_click.is_some_and(|(last, at)| {
            last == id && now.duration_since(at) <= Duration::from_millis(500)
        }) {
            self.last_click = None;
            self.activate(id);
        } else {
            self.last_click = Some((id, now));
        }
        true
    }

    fn activate(&mut self, id: DesktopEntryId) {
        if let Some(action) = self.layout.activate(id) {
            let result = match action {
                DesktopFileAction::Browse(path) => launch_nickel_file(&path),
                DesktopFileAction::Open(path) => nickel_file::open_path(&path),
            };
            if let Err(error) = result {
                let path = self
                    .layout
                    .items()
                    .iter()
                    .find(|item| item.id == id)
                    .map(|item| item.entry.path.as_path())
                    .unwrap_or_else(|| std::path::Path::new("Desktop item"));
                self.error = Some(format!("Could not open {}: {error}", path.display()));
            }
        }
    }

    fn open_background_context(&mut self, anchor: Option<DesktopPoint>) {
        self.context_menu = Some(DesktopMenuContext {
            anchor,
            entry: None,
            output: self.active_output.clone(),
            topology_generation: self.topology_generation,
            directory_generation: self.directory_generation,
            selection: self.layout.selected().clone(),
            workspace: self.workspace,
            paste_available: nickel_file::native_file_clipboard_available(),
            desktop_writable: nickel_file::directory_is_writable(&nickel_file::desktop_directory()),
        });
    }

    fn open_keyboard_context(&mut self) {
        if let Some(id) = self.layout.active()
            && self.layout.items().iter().any(|item| item.id == id)
        {
            if !self.layout.selected().contains(&id) {
                self.layout.select(id, SelectionModifiers::default());
            }
            self.context_menu = Some(DesktopMenuContext {
                anchor: None,
                entry: Some(id),
                output: self.active_output.clone(),
                topology_generation: self.topology_generation,
                directory_generation: self.directory_generation,
                selection: self.layout.selected().clone(),
                workspace: self.workspace,
                paste_available: false,
                desktop_writable: false,
            });
        } else {
            self.open_background_context(None);
        }
    }

    fn apply_desktop_command(&mut self, command: DesktopCommand) {
        let Some(context) = self.context_menu.clone() else {
            return;
        };
        if context.output != self.active_output
            || context.topology_generation != self.topology_generation
            || context.directory_generation != self.directory_generation
            || context.selection != *self.layout.selected()
            || context.workspace != self.workspace
        {
            self.context_menu = None;
            return;
        }
        match command {
            DesktopCommand::IconsVisible(value) => self.layout.set_icons_visible(value),
            DesktopCommand::IconSize(width, height) => self.layout.set_grid(width, height),
            DesktopCommand::Sort(key, direction) => {
                let grouping = self.layout.folder_grouping();
                self.layout
                    .set_arrangement(DesktopArrangement::Sorted { key, direction }, grouping);
            }
            DesktopCommand::FolderGrouping(grouping) => {
                let arrangement = self.layout.arrangement();
                self.layout.set_arrangement(arrangement, grouping);
            }
            DesktopCommand::Manual => {
                let grouping = self.layout.folder_grouping();
                self.layout
                    .set_arrangement(DesktopArrangement::Manual, grouping);
            }
            DesktopCommand::AlignGrid => self.layout.align_to_grid(),
            DesktopCommand::AutoArrange => self.layout.clean_up(),
            DesktopCommand::Refresh => {
                self.refresh_directory(true);
            }
            DesktopCommand::Paste => {
                if !self.paste_in_progress {
                    self.paste_in_progress = true;
                    let sender = self.operation_tx.clone();
                    let destination = nickel_file::desktop_directory();
                    std::thread::spawn(move || {
                        let result = nickel_file::paste_native_file_clipboard(&destination)
                            .map(|count| format!("Pasted {count} item(s)"))
                            .map_err(|error| format!("Could not paste: {error}"));
                        let _ = sender.send(result);
                    });
                }
            }
            DesktopCommand::NewFolder => {
                match nickel_file::create_new_folder(&nickel_file::desktop_directory()) {
                    Ok(_) => {
                        self.refresh_directory(true);
                    }
                    Err(error) => self.error = Some(format!("Could not create folder: {error}")),
                }
            }
            DesktopCommand::DisplaySettings => self.launch_settings(SettingsDestination::Display {
                output: context.output.clone(),
            }),
            DesktopCommand::Personalize => self.launch_settings(SettingsDestination::Appearance),
        }
        if !matches!(
            command,
            DesktopCommand::Refresh
                | DesktopCommand::Paste
                | DesktopCommand::NewFolder
                | DesktopCommand::DisplaySettings
                | DesktopCommand::Personalize
        ) {
            self.save_layout();
        }
        self.context_menu = None;
    }

    fn launch_settings(&mut self, destination: SettingsDestination) {
        let result = std::env::current_exe()
            .map_err(|error| error.to_string())
            .and_then(|exe| {
                let exe = exe.with_file_name(if cfg!(target_os = "windows") {
                    "nickel-settings.exe"
                } else {
                    "nickel-settings"
                });
                let mut command = std::process::Command::new(exe);
                command.args(destination.arguments());
                command
                    .spawn()
                    .map(|_| ())
                    .map_err(|error| error.to_string())
            });
        if let Err(error) = result {
            self.error = Some(format!("Could not open Settings: {error}"));
        }
    }

    fn key(&mut self, key: &nickel_input::KeyEvent) -> bool {
        use nickel_input::{AggregateModifier, PhysicalKey};
        self.modifiers = SelectionModifiers {
            toggle: key.modifiers.aggregate(AggregateModifier::Control),
            range: key.modifiers.aggregate(AggregateModifier::Shift),
            additive_range: key.modifiers.aggregate(AggregateModifier::Control)
                && key.modifiers.aggregate(AggregateModifier::Shift),
        };
        if key.edge != nickel_input::KeyEdge::Pressed {
            return false;
        }
        let PhysicalKey::Code(code) = key.physical else {
            return false;
        };
        let move_selected = key.modifiers.aggregate(AggregateModifier::Alt);
        match code {
            KeyCode::ContextMenu | KeyCode::F10
                if code == KeyCode::ContextMenu || self.modifiers.range =>
            {
                self.open_keyboard_context();
            }
            KeyCode::F5 => {
                self.open_background_context(None);
                self.apply_desktop_command(DesktopCommand::Refresh);
            }
            KeyCode::KeyV if self.modifiers.toggle => {
                self.open_background_context(None);
                self.apply_desktop_command(DesktopCommand::Paste);
            }
            KeyCode::KeyC | KeyCode::KeyX if self.modifiers.toggle => {
                if let Some(id) = self.layout.active() {
                    <Self as nickel_ui::Application>::update(
                        self,
                        if code == KeyCode::KeyX {
                            DesktopMessage::Cut(id)
                        } else {
                            DesktopMessage::Copy(id)
                        },
                    );
                }
            }
            KeyCode::KeyD if self.modifiers.toggle && self.modifiers.range => {
                self.open_background_context(None);
                self.apply_desktop_command(DesktopCommand::IconsVisible(
                    !self.layout.icons_visible(),
                ));
            }
            KeyCode::KeyA if self.modifiers.toggle => self.layout.select_all(),
            KeyCode::F2 => {
                if let Some(id) = self.layout.active() {
                    <Self as nickel_ui::Application>::update(self, DesktopMessage::Rename(id));
                }
            }
            KeyCode::ArrowLeft if move_selected => self.move_selected_by(-96.0, 0.0),
            KeyCode::ArrowRight if move_selected => self.move_selected_by(96.0, 0.0),
            KeyCode::ArrowUp if move_selected => self.move_selected_by(0.0, -112.0),
            KeyCode::ArrowDown if move_selected => self.move_selected_by(0.0, 112.0),
            KeyCode::ArrowLeft if self.modifiers.toggle && !self.modifiers.range => {
                self.layout.focus_direction(-1, 0)
            }
            KeyCode::ArrowRight if self.modifiers.toggle && !self.modifiers.range => {
                self.layout.focus_direction(1, 0)
            }
            KeyCode::ArrowUp if self.modifiers.toggle && !self.modifiers.range => {
                self.layout.focus_direction(0, -1)
            }
            KeyCode::ArrowDown if self.modifiers.toggle && !self.modifiers.range => {
                self.layout.focus_direction(0, 1)
            }
            KeyCode::ArrowLeft => {
                self.layout
                    .select_direction_with_modifiers(-1, 0, self.modifiers)
            }
            KeyCode::ArrowRight => {
                self.layout
                    .select_direction_with_modifiers(1, 0, self.modifiers)
            }
            KeyCode::ArrowUp => self
                .layout
                .select_direction_with_modifiers(0, -1, self.modifiers),
            KeyCode::ArrowDown => self
                .layout
                .select_direction_with_modifiers(0, 1, self.modifiers),
            KeyCode::Space => {
                if let Some(id) = self.layout.active() {
                    self.layout.select(
                        id,
                        SelectionModifiers {
                            toggle: true,
                            ..SelectionModifiers::default()
                        },
                    );
                }
            }
            KeyCode::Enter | KeyCode::NumpadEnter => {
                if let Some(id) = self.layout.active() {
                    self.activate(id);
                }
            }
            KeyCode::Escape => {
                self.context_menu = None;
                self.layout.clear_selection();
            }
            _ => return false,
        }
        true
    }

    fn move_selected_by(&mut self, x: f32, y: f32) {
        if let Some(id) = self.layout.active() {
            self.layout
                .move_group(id, DesktopPoint { x, y }, &self.active_output);
            self.save_layout();
        }
    }

    fn prepare_icons(&mut self) -> bool {
        if !self.layout.icons_visible() {
            return false;
        }
        let previous_len = self.icon_cache.len();
        let preference = ShellSettings::load_default().file_icon_provider;
        let appearance = if self.palette.background & 0xff > 0x80 {
            nickel_file::icons::ArtworkAppearance::Light
        } else {
            nickel_file::icons::ArtworkAppearance::Dark
        };
        for item in self
            .layout
            .items()
            .iter()
            .filter(|item| item.output == self.active_output)
        {
            self.icon_cache
                .entry(item.entry.path.clone())
                .or_insert_with(|| {
                    nickel_file::icons::resolve_artwork(
                        preference,
                        &nickel_file::icons::ArtworkRequest {
                            path: &item.entry.path,
                            kind: nickel_file::icons::semantic_kind(
                                &item.entry.path,
                                item.entry.is_directory,
                            ),
                            logical_size: 48,
                            scale_milli: (self.active_scale * 1000.0).round() as u16,
                            appearance,
                        },
                    )
                    .pixels
                });
        }
        self.icon_cache.len() != previous_len
    }
}

fn desktop_snapshot(
    browser: &DirectoryBrowser,
) -> Vec<(nickel_file::FileIdentity, nickel_file::FileEntry)> {
    browser
        .entries()
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| {
            browser
                .identity_at(index)
                .map(|identity| (identity, entry.clone()))
        })
        .collect()
}

fn desktop_layout_path() -> std::path::PathBuf {
    let root = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("LOCALAPPDATA").map(std::path::PathBuf::from))
        .or_else(|| {
            std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".config"))
        })
        .unwrap_or_else(|| ".".into());
    root.join("nickel").join("desktop-layout")
}

fn launch_nickel_file(path: &std::path::Path) -> Result<(), String> {
    let executable = std::env::current_exe()
        .map_err(|error| error.to_string())?
        .with_file_name(if cfg!(target_os = "windows") {
            "nickel-file.exe"
        } else {
            "nickel-file"
        });
    std::process::Command::new(executable)
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn launch_nickel_file_properties(path: &std::path::Path) -> Result<(), String> {
    let executable = std::env::current_exe()
        .map_err(|error| error.to_string())?
        .with_file_name(if cfg!(target_os = "windows") {
            "nickel-file.exe"
        } else {
            "nickel-file"
        });
    std::process::Command::new(executable)
        .arg("--properties")
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn launch_nickel_file_rename(path: &std::path::Path) -> Result<(), String> {
    let executable = std::env::current_exe()
        .map_err(|error| error.to_string())?
        .with_file_name(if cfg!(target_os = "windows") {
            "nickel-file.exe"
        } else {
            "nickel-file"
        });
    std::process::Command::new(executable)
        .arg("--rename")
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|error| error.to_string())
}

impl nickel_ui::Application for DesktopApplication {
    type Message = DesktopMessage;

    fn update(&mut self, message: Self::Message) {
        match message {
            DesktopMessage::Activate(id) => self.activate(id),
            DesktopMessage::Context(id) => {
                if let Some(item) = self.layout.items().iter().find(|item| item.id == id) {
                    self.context_menu = Some(DesktopMenuContext {
                        anchor: Some(DesktopPoint {
                            x: item.position.x - self.output_origin.x,
                            y: item.position.y - self.output_origin.y,
                        }),
                        entry: Some(id),
                        output: self.active_output.clone(),
                        topology_generation: self.topology_generation,
                        directory_generation: self.directory_generation,
                        selection: self.layout.selected().clone(),
                        workspace: self.workspace,
                        paste_available: false,
                        desktop_writable: false,
                    });
                }
            }
            DesktopMessage::Cut(id) | DesktopMessage::Copy(id) => {
                let cut = matches!(message, DesktopMessage::Cut(_));
                if !self.layout.selected().contains(&id) {
                    self.layout.select(id, SelectionModifiers::default());
                }
                let paths = self
                    .layout
                    .items()
                    .iter()
                    .filter(|item| self.layout.selected().contains(&item.id))
                    .map(|item| item.entry.path.clone())
                    .collect::<Vec<_>>();
                if let Err(error) = nickel_file::publish_file_clipboard(&paths, cut) {
                    self.error = Some(format!("Could not update file clipboard: {error}"));
                }
                self.context_menu = None;
            }
            DesktopMessage::Rename(id) | DesktopMessage::Properties(id) => {
                let rename = matches!(message, DesktopMessage::Rename(_));
                if let Some(path) = self
                    .layout
                    .items()
                    .iter()
                    .find(|item| item.id == id)
                    .map(|item| item.entry.path.clone())
                {
                    let result = if rename {
                        launch_nickel_file_rename(&path)
                    } else {
                        launch_nickel_file_properties(&path)
                    };
                    if let Err(error) = result {
                        self.error = Some(error);
                    }
                }
                self.context_menu = None;
            }
            DesktopMessage::BackgroundContext => self.open_background_context(None),
            DesktopMessage::Command(command) => self.apply_desktop_command(command),
        }
    }

    fn frame_overlays(&self, view_context: ViewContext) -> Vec<FrameOverlay<Self::Message>> {
        let Some(context) = &self.context_menu else {
            return Vec::new();
        };
        if context.entry.is_none() {
            let anchor = context.anchor.map_or_else(
                || OverlayAnchor::InvocationTargetCenter(UiId::new("desktop")),
                |point| OverlayAnchor::Point {
                    invocation_target: UiId::new("desktop"),
                    point: Point {
                        x: point.x,
                        y: point.y,
                    },
                },
            );
            let visible = self.layout.icons_visible();
            let grid = self.layout.grid();
            let arrangement = self.layout.arrangement();
            let grouping = self.layout.folder_grouping();
            let checked = |selected: bool, label: &str| {
                if selected {
                    format!("✓ {label}")
                } else {
                    label.to_owned()
                }
            };
            let paste = if self.paste_in_progress {
                OverlayMenuItem::disabled_with_reason(
                    "paste",
                    "Paste",
                    "A desktop paste is already in progress",
                )
                .shortcut("Ctrl+V")
            } else if context.paste_available {
                OverlayMenuItem::action(
                    "paste",
                    "Paste",
                    DesktopMessage::Command(DesktopCommand::Paste),
                )
                .shortcut("Ctrl+V")
            } else {
                OverlayMenuItem::disabled_with_reason("paste", "Paste", "File clipboard is empty")
                    .shortcut("Ctrl+V")
            };
            let new_folder = if context.desktop_writable {
                OverlayMenuItem::action(
                    "new-folder",
                    "New Folder",
                    DesktopMessage::Command(DesktopCommand::NewFolder),
                )
                .separator_before(true)
            } else {
                OverlayMenuItem::disabled_with_reason(
                    "new-folder",
                    "New Folder",
                    "Desktop location is not writable",
                )
                .separator_before(true)
            };
            let mut menu = OverlayMenu::new("desktop-background-context", anchor)
                .item(
                    OverlayMenuItem::action(
                        "show-icons",
                        if visible {
                            "Hide desktop icons"
                        } else {
                            "Show desktop icons"
                        },
                        DesktopMessage::Command(DesktopCommand::IconsVisible(!visible)),
                    )
                    .shortcut("Ctrl+Shift+D"),
                )
                .item(OverlayMenuItem::action(
                    "small-icons",
                    if grid.0 <= 72.0 {
                        "✓ Small icons"
                    } else {
                        "Small icons"
                    },
                    DesktopMessage::Command(DesktopCommand::IconSize(72.0, 88.0)),
                ))
                .item(OverlayMenuItem::action(
                    "medium-icons",
                    if grid.0 > 72.0 && grid.0 < 128.0 {
                        "✓ Medium icons"
                    } else {
                        "Medium icons"
                    },
                    DesktopMessage::Command(DesktopCommand::IconSize(96.0, 112.0)),
                ))
                .item(OverlayMenuItem::action(
                    "large-icons",
                    if grid.0 >= 128.0 {
                        "✓ Large icons"
                    } else {
                        "Large icons"
                    },
                    DesktopMessage::Command(DesktopCommand::IconSize(128.0, 144.0)),
                ))
                .item(
                    OverlayMenuItem::action(
                        "sort-name",
                        checked(
                            arrangement
                                == DesktopArrangement::Sorted {
                                    key: DesktopSortKey::Name,
                                    direction: DesktopSortDirection::Ascending,
                                },
                            "Name (ascending)",
                        ),
                        DesktopMessage::Command(DesktopCommand::Sort(
                            DesktopSortKey::Name,
                            DesktopSortDirection::Ascending,
                        )),
                    )
                    .separator_before(true),
                )
                .item(OverlayMenuItem::action(
                    "sort-name-descending",
                    checked(
                        arrangement
                            == DesktopArrangement::Sorted {
                                key: DesktopSortKey::Name,
                                direction: DesktopSortDirection::Descending,
                            },
                        "Name (descending)",
                    ),
                    DesktopMessage::Command(DesktopCommand::Sort(
                        DesktopSortKey::Name,
                        DesktopSortDirection::Descending,
                    )),
                ))
                .item(OverlayMenuItem::action(
                    "sort-kind",
                    checked(
                        arrangement
                            == DesktopArrangement::Sorted {
                                key: DesktopSortKey::Kind,
                                direction: DesktopSortDirection::Ascending,
                            },
                        "Type (ascending)",
                    ),
                    DesktopMessage::Command(DesktopCommand::Sort(
                        DesktopSortKey::Kind,
                        DesktopSortDirection::Ascending,
                    )),
                ))
                .item(OverlayMenuItem::action(
                    "sort-kind-descending",
                    checked(
                        arrangement
                            == DesktopArrangement::Sorted {
                                key: DesktopSortKey::Kind,
                                direction: DesktopSortDirection::Descending,
                            },
                        "Type (descending)",
                    ),
                    DesktopMessage::Command(DesktopCommand::Sort(
                        DesktopSortKey::Kind,
                        DesktopSortDirection::Descending,
                    )),
                ))
                .item(OverlayMenuItem::action(
                    "sort-size",
                    checked(
                        arrangement
                            == DesktopArrangement::Sorted {
                                key: DesktopSortKey::Size,
                                direction: DesktopSortDirection::Ascending,
                            },
                        "Size (ascending)",
                    ),
                    DesktopMessage::Command(DesktopCommand::Sort(
                        DesktopSortKey::Size,
                        DesktopSortDirection::Ascending,
                    )),
                ))
                .item(OverlayMenuItem::action(
                    "sort-size-descending",
                    checked(
                        arrangement
                            == DesktopArrangement::Sorted {
                                key: DesktopSortKey::Size,
                                direction: DesktopSortDirection::Descending,
                            },
                        "Size (descending)",
                    ),
                    DesktopMessage::Command(DesktopCommand::Sort(
                        DesktopSortKey::Size,
                        DesktopSortDirection::Descending,
                    )),
                ))
                .item(OverlayMenuItem::action(
                    "sort-modified",
                    checked(
                        arrangement
                            == DesktopArrangement::Sorted {
                                key: DesktopSortKey::Modified,
                                direction: DesktopSortDirection::Descending,
                            },
                        "Modified (newest first)",
                    ),
                    DesktopMessage::Command(DesktopCommand::Sort(
                        DesktopSortKey::Modified,
                        DesktopSortDirection::Descending,
                    )),
                ))
                .item(OverlayMenuItem::action(
                    "sort-modified-ascending",
                    checked(
                        arrangement
                            == DesktopArrangement::Sorted {
                                key: DesktopSortKey::Modified,
                                direction: DesktopSortDirection::Ascending,
                            },
                        "Modified (oldest first)",
                    ),
                    DesktopMessage::Command(DesktopCommand::Sort(
                        DesktopSortKey::Modified,
                        DesktopSortDirection::Ascending,
                    )),
                ))
                .item(
                    OverlayMenuItem::action(
                        "manual",
                        checked(
                            arrangement == DesktopArrangement::Manual,
                            "Manual arrangement",
                        ),
                        DesktopMessage::Command(DesktopCommand::Manual),
                    )
                    .separator_before(true),
                )
                .item(OverlayMenuItem::action(
                    "align",
                    "Align to Grid",
                    DesktopMessage::Command(DesktopCommand::AlignGrid),
                ))
                .item(OverlayMenuItem::action(
                    "auto-arrange",
                    "Auto Arrange",
                    DesktopMessage::Command(DesktopCommand::AutoArrange),
                ))
                .item(
                    OverlayMenuItem::action(
                        "folders-first",
                        checked(grouping == FolderGrouping::FoldersFirst, "Folders first"),
                        DesktopMessage::Command(DesktopCommand::FolderGrouping(
                            FolderGrouping::FoldersFirst,
                        )),
                    )
                    .separator_before(true),
                )
                .item(OverlayMenuItem::action(
                    "folders-mixed",
                    checked(grouping == FolderGrouping::Mixed, "Mix folders and files"),
                    DesktopMessage::Command(DesktopCommand::FolderGrouping(FolderGrouping::Mixed)),
                ))
                .item(
                    OverlayMenuItem::action(
                        "refresh",
                        "Refresh",
                        DesktopMessage::Command(DesktopCommand::Refresh),
                    )
                    .shortcut("F5")
                    .separator_before(true),
                )
                .item(paste)
                .item(new_folder)
                .item(
                    OverlayMenuItem::action(
                        "display-settings",
                        "Display Settings",
                        DesktopMessage::Command(DesktopCommand::DisplaySettings),
                    )
                    .separator_before(true),
                )
                .item(OverlayMenuItem::action(
                    "personalize",
                    "Personalize",
                    DesktopMessage::Command(DesktopCommand::Personalize),
                ));
            menu.background = self.palette.surface;
            menu.border = self.palette.muted;
            menu.foreground = self.palette.text;
            menu.item_hover = Some(self.palette.surface_hover);
            menu.item_selected = Some(self.palette.accent_soft);
            menu.row_height = ((view_context.viewport.size.height - 8.0) / menu.items.len() as f32)
                .clamp(18.0, menu.row_height);
            return vec![FrameOverlay::Menu(menu)];
        }
        let id = context.entry.unwrap();
        let anchor = UiId::new(format!("desktop-entry-{}-{}", id.0.0, id.0.1));
        vec![FrameOverlay::Menu(
            OverlayMenu::new(
                format!("desktop-entry-{}-{}-context", id.0.0, id.0.1),
                OverlayAnchor::InvocationTarget(anchor),
            )
            .item(
                OverlayMenuItem::action("open", "Open", DesktopMessage::Activate(id))
                    .shortcut("Enter"),
            )
            .item(
                OverlayMenuItem::action("cut", "Cut", DesktopMessage::Cut(id))
                    .shortcut("Ctrl+X")
                    .separator_before(true),
            )
            .item(
                OverlayMenuItem::action("copy", "Copy", DesktopMessage::Copy(id))
                    .shortcut("Ctrl+C"),
            )
            .item(
                OverlayMenuItem::action("rename", "Rename", DesktopMessage::Rename(id))
                    .shortcut("F2")
                    .separator_before(true),
            )
            .item(OverlayMenuItem::disabled_with_reason(
                "trash",
                "Move to Trash",
                "Trash integration is not implemented yet",
            ))
            .item(
                OverlayMenuItem::disabled_with_reason(
                    "open-terminal",
                    "Open in Terminal",
                    "Terminal integration is not implemented yet",
                )
                .separator_before(true),
            )
            .item(OverlayMenuItem::action(
                "properties",
                "Properties",
                DesktopMessage::Properties(id),
            )),
        )]
    }

    fn view(&self, context: ViewContext) -> impl nickel_ui::View<Self::Message> {
        let width = context.viewport.size.width;
        let height = context.viewport.size.height;
        let mut layer = Layer::new().width(width).height(height).child(
            Container::new()
                .width(width)
                .height(height)
                .background(self.palette.background),
        );
        if let Some(wallpaper) = &self.wallpaper {
            layer = layer.child(
                Image::new(1, Arc::clone(wallpaper))
                    .width(width)
                    .height(height)
                    .fit(ImageFit::Stretch)
                    .decorative(),
            );
        }
        let hovered = self
            .pointer_seen
            .then(|| self.hit(self.pointer_position))
            .flatten();
        let (cell_width, cell_height) = self.layout.grid();
        for (index, item) in self
            .layout
            .items()
            .iter()
            .filter(|item| self.layout.icons_visible() && item.output == self.active_output)
            .enumerate()
        {
            let position = Point {
                x: item.position.x - self.output_origin.x,
                y: item.position.y - self.output_origin.y,
            };
            let selected = self.layout.selected().contains(&item.id);
            let focused = self.layout.active() == Some(item.id);
            let icon = self
                .icon_cache
                .get(&item.entry.path)
                .cloned()
                .unwrap_or_else(|| {
                    Arc::new(image::RgbaImage::from_pixel(
                        1,
                        1,
                        image::Rgba([0, 0, 0, 0]),
                    ))
                });
            let label_height = (cell_height - 74.0).max(1.0);
            let mut tile = FilePlaneItem::new_with_generation(
                DesktopMessage::Activate(item.id),
                item.entry.display_name(),
                10_000_u16.saturating_add(index as u16),
                icon,
                self.directory_generation,
            )
            .id(format!("desktop-entry-{}-{}", item.id.0.0, item.id.0.1))
            .position(position)
            .width(cell_width)
            .height(cell_height - 4.0)
            .padding(Insets {
                top: 6.0,
                right: 3.0,
                bottom: 8.0,
                left: 3.0,
            })
            .radius(8.0)
            .context_message(DesktopMessage::Context(item.id))
            .semantic_role(SemanticRole::GridCell)
            .accessibility_label(item.entry.display_name())
            .interaction_backgrounds(self.palette.surface_hover, self.palette.accent_soft)
            .selected_background(selected, self.palette.accent_soft)
            .hovered_background(
                !selected && (hovered == Some(item.id) || focused),
                self.palette.surface_hover,
            )
            .focus_background_tint(self.palette.accent)
            .controller_focus_background_tint(self.palette.complement)
            .icon_size(48.0)
            .label_height(label_height)
            .label_scale(0.85)
            .foreground(self.palette.text)
            .label_background(self.palette.panel, 5.0)
            .gap(8.0);
            if self.pointer_dragged && selected {
                tile = tile.border(self.palette.accent, 2.0);
            }
            layer = layer.child(tile);
        }
        if let Some(start) = self.selection_start {
            let current = self.pointer_position;
            layer = layer.child(
                Container::new()
                    .position(Point {
                        x: start.x.min(current.x),
                        y: start.y.min(current.y),
                    })
                    .width((current.x - start.x).abs())
                    .height((current.y - start.y).abs())
                    .background(self.palette.accent_soft),
            );
        }
        if let Some(error) = &self.error {
            layer = layer.child(
                Container::new()
                    .position(Point { x: 20.0, y: 20.0 })
                    .width(500.0)
                    .height(42.0)
                    .padding(Insets::symmetric(12.0, 5.0))
                    .background(self.palette.surface)
                    .radius(8.0)
                    .child(
                        Text::new(error.clone())
                            .width(476.0)
                            .height(32.0)
                            .scale(0.9)
                            .color(self.palette.text),
                    ),
            );
        }
        Container::new()
            .id("desktop")
            .semantic_role(SemanticRole::ApplicationPresentation)
            .accessibility_label("Desktop")
            .context_message(DesktopMessage::BackgroundContext)
            .background(self.palette.background)
            .width(width)
            .height(height)
            .child(layer)
    }

    fn title(&self) -> &str {
        "Nickel Desktop"
    }

    fn poll(&mut self) -> bool {
        let mut changed = self.refresh_directory(false);
        while let Ok(result) = self.operation_rx.try_recv() {
            self.paste_in_progress = false;
            self.error = Some(result.unwrap_or_else(|error| error));
            self.refresh_directory(true);
            changed = true;
        }
        changed |= self.prepare_icons();
        changed
    }

    fn poll_interval(&self) -> Option<Duration> {
        Some(Duration::from_millis(250))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LockMessage {
    Password(String),
}

enum LockEffect {
    Authenticate(Zeroizing<String>),
}

pub struct LockApplication {
    password: Zeroizing<String>,
    status: Option<String>,
    effects: Vec<LockEffect>,
}

struct VolumeOsdApplication {
    label: String,
    percent: u8,
    palette: ThemePalette,
}

impl nickel_ui::Application for VolumeOsdApplication {
    type Message = ();

    fn update(&mut self, (): Self::Message) {}

    fn view(&self, context: ViewContext) -> impl nickel_ui::View<Self::Message> {
        let width = context.viewport.size.width;
        let height = context.viewport.size.height;
        let track_width = (width - 48.0).max(0.0);
        Container::new()
            .id("volume-osd")
            .semantic_role(SemanticRole::Status)
            .accessibility_label(self.label.clone())
            .width(width)
            .height(height)
            .background(self.palette.panel)
            .radius(14.0)
            .child(
                Layer::new()
                    .width(width)
                    .height(height)
                    .child(
                        Container::new()
                            .position(Point { x: 24.0, y: 14.0 })
                            .width(track_width)
                            .height(32.0)
                            .child(
                                Text::new(self.label.clone())
                                    .width(track_width)
                                    .height(32.0)
                                    .scale(1.15)
                                    .color(self.palette.text)
                                    .align(TextAlign::Center)
                                    .bold(true),
                            ),
                    )
                    .child(
                        Container::new()
                            .position(Point {
                                x: 24.0,
                                y: height - 28.0,
                            })
                            .width(track_width)
                            .height(8.0)
                            .background(self.palette.surface_hover)
                            .radius(4.0),
                    )
                    .child(
                        Container::new()
                            .position(Point {
                                x: 24.0,
                                y: height - 28.0,
                            })
                            .width(track_width * f32::from(self.percent) / 100.0)
                            .height(8.0)
                            .background(self.palette.accent)
                            .radius(4.0),
                    ),
            )
    }
}

pub struct PanelApplication {
    launcher: Launcher,
    windows: Vec<OpenWindow>,
    tray: Vec<TrayItem>,
    tray_icons: Vec<Arc<image::RgbaImage>>,
    panel_icon: Arc<image::RgbaImage>,
    codex_icon: Arc<image::RgbaImage>,
    task_icons: Vec<Option<(u16, Arc<image::RgbaImage>)>>,
    palette: ThemePalette,
    panel_hover: Option<PanelHover>,
    launcher_visible: bool,
    codex_project_menu_visible: bool,
    control_visible: bool,
    clock: String,
    date: String,
    effects: Vec<PanelAction>,
    task_drag: Option<(usize, isize)>,
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
                            .launcher
                            .taskbar_applications(&self.windows)
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
        let tasks = self.launcher.taskbar_applications(&self.windows);
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
impl DesktopApplication {
    #[allow(dead_code)] // Binary and fixture library compile this shared module separately.
    pub fn fixture(wallpaper: Option<Arc<image::RgbaImage>>, palette: ThemePalette) -> Self {
        let (operation_tx, operation_rx) = std::sync::mpsc::channel();
        Self {
            wallpaper,
            palette,
            browser: None,
            watch: None,
            layout: DesktopLayout::new(vec![DesktopOutput {
                id: "primary".into(),
                work_area: DesktopRect {
                    x: 0.0,
                    y: 0.0,
                    width: 1920.0,
                    height: 1024.0,
                },
                scale: 1.0,
            }]),
            active_output: "primary".into(),
            output_origin: DesktopPoint::default(),
            active_scale: 1.0,
            icon_cache: HashMap::new(),
            pointer_down: None,
            drag_commit_position: None,
            selection_start: None,
            pointer_position: DesktopPoint::default(),
            pointer_seen: false,
            pointer_dragged: false,
            last_click: None,
            modifiers: SelectionModifiers::default(),
            context_menu: None,
            topology_generation: 0,
            directory_generation: 0,
            outputs: Vec::new(),
            workspace: None,
            persist_layout: false,
            operation_tx,
            operation_rx,
            paste_in_progress: false,
            error: None,
        }
    }
}

#[cfg(any(test, feature = "workbench-fixtures"))]
impl LockApplication {
    #[allow(dead_code)] // Binary and fixture library compile this shared module separately.
    pub fn fixture(password: &str, status: Option<String>) -> Self {
        Self {
            password: Zeroizing::new(password.to_owned()),
            status,
            effects: Vec::new(),
        }
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
            launcher,
            windows: Vec::new(),
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
        let mut application = Self::fixture(launcher, palette);
        application.windows = vec![
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

fn panel_clock_text() -> (String, String) {
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

impl nickel_ui::Application for LockApplication {
    type Message = LockMessage;

    fn update(&mut self, message: Self::Message) {
        match message {
            LockMessage::Password(value) if value.len() <= 1024 => {
                self.password = Zeroizing::new(value);
                self.status = None;
            }
            LockMessage::Password(_) => {}
        }
    }

    fn shortcut(&mut self, shortcut: Shortcut) -> bool {
        if shortcut != Shortcut::Submit {
            return false;
        }
        self.effects
            .push(LockEffect::Authenticate(std::mem::take(&mut self.password)));
        true
    }

    fn view(&self, context: ViewContext) -> impl nickel_ui::View<Self::Message> {
        let width = context.viewport.size.width;
        let height = context.viewport.size.height;
        let username = std::env::var("USER").unwrap_or_else(|_| "Session locked".into());
        let password_color = if self.password.is_empty() {
            0x8992a6
        } else {
            0xffffff
        };
        let mut content = Column::new()
            .width(width)
            .height(height)
            .child(Spacer::vertical(height * 0.38))
            .child(
                Text::new("Nickel")
                    .height(48.0)
                    .scale(30.0)
                    .color(0xffffff)
                    .align(TextAlign::Center)
                    .bold(true),
            )
            .child(Spacer::vertical(8.0))
            .child(
                Text::new(username)
                    .height(32.0)
                    .scale(18.0)
                    .color(0xb8c0d4)
                    .align(TextAlign::Center),
            )
            .child(Spacer::vertical(16.0))
            .child(
                Container::new()
                    .id("lock-password")
                    .accessibility_label("Password")
                    .width(340.0)
                    .height(46.0)
                    .align_self(nickel_ui::Align::Center)
                    .background(0x20283a)
                    .radius(10.0)
                    .padding(Insets {
                        top: 9.0,
                        right: 16.0,
                        bottom: 9.0,
                        left: 16.0,
                    })
                    .child(
                        TextField::on_change_masked_with_placeholder(
                            &self.password,
                            "Password",
                            '•',
                            LockMessage::Password,
                        )
                        .id("lock-password-input")
                        .accessibility_label("Password")
                        .scale(18.0)
                        .single_line_height(28.0)
                        .color(password_color),
                    ),
            );
        if let Some(status) = &self.status {
            content = content.child(Spacer::vertical(14.0)).child(
                Text::new(status)
                    .height(28.0)
                    .scale(15.0)
                    .color(0xff9a9a)
                    .align(TextAlign::Center),
            );
        }
        Container::new()
            .id("lock-screen")
            .accessibility_label("Session locked")
            .background(0x080b12)
            .width(width)
            .height(height)
            .child(content)
    }

    fn title(&self) -> &str {
        "Nickel Lock Screen"
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ShellImageCacheDiagnostics {
    pub launcher_icon_entries: usize,
    pub launcher_icon_bytes: usize,
    pub wallpaper_entries: usize,
    pub wallpaper_bytes: usize,
    pub tray_entries: usize,
    pub tray_bytes: usize,
    pub preview_entries: usize,
    pub preview_bytes: usize,
}

#[derive(Debug, Default, Eq, PartialEq)]
pub struct ShellDeadlineOutcome {
    pub redraw: Vec<SurfaceRole>,
    pub capture_screenshot: bool,
    pub visibility_changed: bool,
}

pub struct LiveShell {
    host_runtime_samples: HostRuntimeSamples,
    launcher: Launcher,
    window_feed: WindowFeed,
    tray_feed: TrayFeed,
    notification_feed: NotificationFeed,
    windows: Vec<OpenWindow>,
    window_icons: HashMap<crate::model::WindowId, Arc<image::RgbaImage>>,
    task_switcher: TaskSwitcher<crate::model::WindowId>,
    task_switcher_group: Option<WindowGroup>,
    workspaces: Vec<platform::WorkspaceSummary>,
    window_feed_status: FeedStatus,
    workspace_feed_status: FeedStatus,
    tray: Vec<TrayItem>,
    tray_icons: Vec<Arc<image::RgbaImage>>,
    notification: Option<DesktopNotification>,
    notification_history_visible: bool,
    wallpaper_path: Option<std::path::PathBuf>,
    wallpaper: Option<Arc<image::RgbaImage>>,
    wallpaper_size: (u32, u32),
    desktop_host: nickel_ui::UiHost<DesktopApplication>,
    desktop_change_token: HostChangeToken,
    desktop_deadline: Option<Instant>,
    panel_icon: Arc<image::RgbaImage>,
    codex_icon: Arc<image::RgbaImage>,
    palette: ThemePalette,
    network: NetworkStatus,
    bluetooth: BluetoothStatus,
    audio: AudioStatus,
    volume_osd_until: Option<Instant>,
    volume_osd_host: nickel_ui::UiHost<VolumeOsdApplication>,
    launcher_visible: bool,
    locked: bool,
    lock_host: nickel_ui::UiHost<LockApplication>,
    lock_change_token: HostChangeToken,
    lock_deadline: Option<Instant>,
    control_visible: bool,
    codex_project_menu_visible: bool,
    panel_hover: Option<PanelHover>,
    panel_hover_output: Option<String>,
    panel_host: nickel_ui::UiHost<PanelApplication>,
    panel_change_token: HostChangeToken,
    panel_deadline: Option<Instant>,
    panel_output: Option<String>,
    pending_popover_anchor: Option<PendingPopoverAnchor>,
    all_windows_on_every_bar: bool,
    preview_group: Option<usize>,
    preview_pending: Option<(usize, Instant)>,
    preview_focus_requested: bool,
    preview_pointer_inside: bool,
    preview_leave_deadline: Option<Instant>,
    preview_hovered: Option<crate::model::WindowId>,
    preview_images: HashMap<crate::model::WindowId, Arc<image::RgbaImage>>,
    preview_refresh_deadline: Option<Instant>,
    preview_frame: Option<WindowPreviewFrame>,
    window_menu: Option<crate::model::WindowId>,
    window_menu_snapshot: Option<OpenWindow>,
    window_menu_anchor_x: Option<i32>,
    window_menu_host: Option<nickel_ui::UiHost<WindowMenuApp>>,
    notification_host: NotificationHost,
    panel_origin_x: i32,
    control_host: ControlCenterHost,
    control_change_token: HostChangeToken,
    control_deadline: Option<Instant>,
    projection_chooser: nickel_core::display_projection::ProjectionChooser,
    projection_rollback_deadline: Option<Instant>,
    launcher_view: LauncherViewState,
    launcher_icons: LauncherIconCache,
    launcher_host: nickel_ui::UiHost<LauncherApplication>,
    launcher_status: Option<String>,
    shortcut_action_status: Option<String>,
    shortcut_capability_status: Option<String>,
    #[cfg(test)]
    launcher_preferences_path: Option<std::path::PathBuf>,
    #[cfg(test)]
    launcher_persistence_attempts: usize,
    secure_storage_override: Option<String>,
    secure_storage_state: platform::SecureStorageState,
    #[cfg(target_os = "linux")]
    secure_storage_query_error: Option<(platform::SessionRequestError, Instant)>,
    requested_codex_project: Option<String>,
    screenshot: ScreenshotTool,
}

#[derive(Default)]
struct HostRuntimeSamples {
    input_to_message_us: VecDeque<u64>,
    input_to_frame_us: VecDeque<u64>,
    layout_us: VecDeque<u64>,
    paint_list_us: VecDeque<u64>,
    scheduled_wakeups: u64,
}

impl HostRuntimeSamples {
    fn record(&mut self, telemetry: nickel_ui::HostTelemetry) {
        for (samples, value) in [
            (&mut self.input_to_message_us, telemetry.input_to_message_us),
            (&mut self.input_to_frame_us, telemetry.input_to_frame_us),
            (&mut self.layout_us, telemetry.layout_us),
            (&mut self.paint_list_us, telemetry.paint_list_us),
        ] {
            if samples.len() == nickel_session_protocol::MAX_RUNTIME_PERFORMANCE_SAMPLES {
                samples.pop_front();
            }
            samples.push_back(value);
        }
        self.scheduled_wakeups = self
            .scheduled_wakeups
            .saturating_add(telemetry.scheduled_wakeups as u64);
    }
}

fn preview_refresh_due(deadline: Option<Instant>, now: Instant) -> bool {
    deadline.is_none_or(|deadline| now >= deadline)
}

fn shortcut_capability_status(
    capability: &nickel_input::global::ShortcutCapability,
) -> Option<String> {
    use nickel_input::global::{ShortcutCapability, UnavailableReason};

    let reason = match capability {
        ShortcutCapability::Available => return None,
        ShortcutCapability::Unavailable(UnavailableReason::UnsupportedPlatform) => {
            "this platform is unsupported".to_owned()
        }
        ShortcutCapability::Unavailable(UnavailableReason::MissingRuntime) => {
            "the Nickel session runtime is missing".to_owned()
        }
        ShortcutCapability::Unavailable(UnavailableReason::PermissionDenied) => {
            "the session denied permission".to_owned()
        }
        ShortcutCapability::Unavailable(UnavailableReason::SessionLocked) => {
            "the session is locked".to_owned()
        }
        ShortcutCapability::Unavailable(UnavailableReason::Backend(reason)) => reason.clone(),
    };
    Some(format!("Global shortcuts unavailable: {reason}."))
}

impl LiveShell {
    #[cfg(target_os = "linux")]
    pub fn host_runtime_samples(&self) -> (Vec<u64>, Vec<u64>, Vec<u64>, Vec<u64>, u64) {
        (
            self.host_runtime_samples
                .input_to_message_us
                .iter()
                .copied()
                .collect(),
            self.host_runtime_samples
                .input_to_frame_us
                .iter()
                .copied()
                .collect(),
            self.host_runtime_samples
                .layout_us
                .iter()
                .copied()
                .collect(),
            self.host_runtime_samples
                .paint_list_us
                .iter()
                .copied()
                .collect(),
            self.host_runtime_samples.scheduled_wakeups,
        )
    }
    pub fn set_dashboard_projects(
        &mut self,
        projects: DashboardSection<Vec<DashboardProject>>,
    ) -> bool {
        self.launcher.set_dashboard_projects(projects)
    }

    pub fn apply_codex_projection(
        &mut self,
        projection: nickel_core::optional_features::CodexAvailabilityProjection,
    ) -> bool {
        if projection.presentation() == nickel_core::optional_features::CodexPresentation::Hidden {
            self.codex_project_menu_visible = false;
            self.requested_codex_project = None;
        }
        self.launcher.apply_codex_projection(projection)
    }

    pub fn take_requested_codex_project(&mut self) -> Option<String> {
        if self.launcher.codex_available() {
            self.requested_codex_project.take()
        } else {
            self.requested_codex_project = None;
            None
        }
    }
    pub fn new() -> Result<Self, String> {
        let shell_settings = ShellSettings::load_default();
        let application_discovery = platform::application_discovery();
        let application_status = application_discovery_status_label(application_discovery.status());
        let mut launcher = Launcher::new(application_discovery.into_applications());
        launcher.set_places(crate::places::applications(
            shell_settings.preferred_file_manager.as_deref(),
        ));
        let launcher_preferences = match LauncherPreferences::load_default() {
            Ok(preferences) => preferences,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                LauncherPreferences::default()
            }
            Err(error) => {
                tracing::warn!(%error, "launcher preferences could not be loaded");
                LauncherPreferences::default()
            }
        };
        launcher.set_preferences(launcher_preferences);
        let _ = launcher.set_dashboard_account(
            std::env::var("USER")
                .or_else(|_| std::env::var("USERNAME"))
                .ok()
                .filter(|name| !name.trim().is_empty())
                .map_or_else(
                    || DashboardSection::Unavailable("Local identity unavailable".into()),
                    |display_name| {
                        DashboardSection::Ready(DashboardAccount {
                            display_name,
                            supporting_text: "Local session".into(),
                        })
                    },
                ),
        );
        let wallpaper_settings = WallpaperSettings::load_default();
        let (wallpaper_path, wallpaper, wallpaper_size) =
            initial_wallpaper(wallpaper_settings.image, || {
                #[cfg(target_os = "windows")]
                return platform::wallpaper().image;
                #[cfg(not(target_os = "windows"))]
                None
            });
        let palette =
            ThemePalette::from_appearance(shell_settings.resolve_appearance(Appearance::default()));
        let panel_icon = crate::icons::load_svg_bytes(
            include_bytes!("../../../assets/icons/nickel-start.svg"),
            96,
        )
        .map(|icon| tint_panel_icon(icon, palette.text))
        .map(Arc::new)
        .expect("embedded Nickel start icon remains valid");
        let codex_icon = crate::icons::load_svg_bytes(
            include_bytes!("../../../assets/icons/nickel-chat.svg"),
            96,
        )
        .map(|icon| tint_panel_icon(icon, palette.text))
        .map(Arc::new)
        .expect("embedded Nickel chat icon remains valid");
        let window_feed = WindowFeed::new();
        let tray_feed = TrayFeed::new();
        let notification_feed = NotificationFeed::new()?;
        let windows = Vec::new();
        let workspaces = Vec::new();
        let tray = normalize_tray_items(tray_feed.snapshot());
        let tray_icons = panel_tray_icons(&tray);
        let network = platform::network_status();
        let bluetooth = platform::bluetooth_status();
        let audio = platform::audio_status();
        let volume_osd_host = nickel_ui::UiHost::new(
            VolumeOsdApplication {
                label: String::new(),
                percent: audio.volume_percent.min(100),
                palette,
            },
            420,
            96,
        );
        #[cfg(target_os = "linux")]
        let (secure_storage_state, secure_storage_query_error) =
            match platform::secure_storage_state() {
                Ok(state) => (state, None),
                Err(error) => {
                    tracing::warn!(%error, "secure-storage query failed during shell startup");
                    (
                        platform::SecureStorageState::ControlUnavailable,
                        Some((error, Instant::now())),
                    )
                }
            };
        #[cfg(not(target_os = "linux"))]
        let secure_storage_state = platform::SecureStorageState::Ready;
        let control_host = ControlCenterHost::new(
            ControlCenterApp::new(
                network.clone(),
                bluetooth.clone(),
                audio.clone(),
                workspaces.clone(),
            ),
            380,
            650,
        );
        let notification_host = NotificationHost::new(NotificationApp::new(palette), 420, 180);
        let desktop_host = nickel_ui::UiHost::new(
            DesktopApplication::new(wallpaper.clone(), palette),
            1920,
            1080,
        );
        let lock_host = nickel_ui::UiHost::new(
            LockApplication {
                password: Zeroizing::new(String::new()),
                status: None,
                effects: Vec::new(),
            },
            1920,
            1080,
        );
        let launcher_view = LauncherViewState::default();
        let launcher_icons = LauncherIconCache::new();
        let launcher_host = nickel_ui::UiHost::new(
            LauncherApplication::new(
                launcher.clone(),
                launcher_view.clone(),
                launcher_icons.clone(),
                palette,
            ),
            920,
            680,
        );
        let (clock, date) = panel_clock_text();
        let panel_host = nickel_ui::UiHost::new(
            PanelApplication {
                launcher: launcher.clone(),
                windows: windows.clone(),
                tray: tray.clone(),
                tray_icons: tray_icons.clone(),
                panel_icon: Arc::clone(&panel_icon),
                codex_icon: Arc::clone(&codex_icon),
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
            },
            1920,
            56,
        );
        Ok(Self {
            host_runtime_samples: HostRuntimeSamples::default(),
            launcher,
            window_feed,
            tray_feed,
            notification_feed,
            windows,
            window_icons: HashMap::new(),
            task_switcher: TaskSwitcher::default(),
            task_switcher_group: None,
            workspaces,
            window_feed_status: FeedStatus::Loading,
            workspace_feed_status: FeedStatus::Loading,
            tray,
            tray_icons,
            notification: None,
            notification_history_visible: false,
            wallpaper_path,
            wallpaper,
            wallpaper_size,
            desktop_host,
            desktop_change_token: HostChangeToken::default(),
            desktop_deadline: None,
            panel_icon,
            codex_icon,
            palette,
            network,
            bluetooth,
            audio,
            volume_osd_until: None,
            volume_osd_host,
            launcher_visible: false,
            locked: false,
            lock_host,
            lock_change_token: HostChangeToken::default(),
            lock_deadline: None,
            control_visible: false,
            codex_project_menu_visible: false,
            panel_hover: None,
            panel_hover_output: None,
            panel_host,
            panel_change_token: HostChangeToken::default(),
            panel_deadline: None,
            panel_output: None,
            pending_popover_anchor: None,
            all_windows_on_every_bar: shell_settings.all_windows_on_every_bar,
            preview_group: None,
            preview_pending: None,
            preview_focus_requested: false,
            preview_pointer_inside: false,
            preview_leave_deadline: None,
            preview_hovered: None,
            preview_images: HashMap::new(),
            preview_refresh_deadline: None,
            preview_frame: None,
            window_menu: None,
            window_menu_snapshot: None,
            window_menu_anchor_x: None,
            window_menu_host: None,
            notification_host,
            panel_origin_x: 0,
            control_host,
            control_change_token: HostChangeToken::default(),
            control_deadline: Some(Instant::now()),
            projection_chooser: Default::default(),
            projection_rollback_deadline: None,
            launcher_view,
            launcher_icons,
            launcher_host,
            launcher_status: application_status.map(str::to_owned),
            shortcut_action_status: None,
            shortcut_capability_status: None,
            #[cfg(test)]
            launcher_preferences_path: None,
            #[cfg(test)]
            launcher_persistence_attempts: 0,
            secure_storage_override: None,
            secure_storage_state,
            #[cfg(target_os = "linux")]
            secure_storage_query_error,
            requested_codex_project: None,
            screenshot: ScreenshotTool::default(),
        })
    }

    pub fn refresh(&mut self) -> bool {
        let fast = self.refresh_fast();
        let system = self.refresh_system();
        fast || system
    }

    pub fn image_cache_diagnostics(&self) -> ShellImageCacheDiagnostics {
        let launcher = self.launcher_icons.diagnostics();
        let wallpaper_bytes = self
            .wallpaper
            .as_ref()
            .map_or(0, |image| image.as_raw().len());
        ShellImageCacheDiagnostics {
            launcher_icon_entries: launcher.entries,
            launcher_icon_bytes: launcher.retained_pixel_bytes,
            wallpaper_entries: usize::from(self.wallpaper.is_some()),
            wallpaper_bytes,
            tray_entries: self.tray.len().saturating_add(self.tray_icons.len()),
            tray_bytes: self
                .tray
                .iter()
                .map(|item| item.icon.as_raw().len())
                .chain(self.tray_icons.iter().map(|image| image.as_raw().len()))
                .sum(),
            preview_entries: self.preview_images.len(),
            preview_bytes: self
                .preview_images
                .values()
                .map(|image| image.as_raw().len())
                .sum(),
        }
    }

    pub fn refresh_fast(&mut self) -> bool {
        let mut changed = false;
        let windows = self.window_feed.snapshot(&self.launcher);
        if update_feed_status(&mut self.window_feed_status, windows.status(), "windows") {
            changed = true;
        }
        if let FeedState::Ready(windows) = windows {
            if windows != self.windows {
                self.windows = windows;
                changed = true;
            }
            let previous_icon_count = self.window_icons.len();
            self.window_icons
                .retain(|window, _| self.windows.iter().any(|item| item.id == *window));
            for window in &self.windows {
                if !self.window_icons.contains_key(&window.id)
                    && let Some(icon) = self.window_feed.icon(window.id)
                {
                    self.window_icons.insert(window.id, Arc::new(icon));
                }
            }
            changed |= self.window_icons.len() != previous_icon_count;
            if self
                .window_menu
                .is_some_and(|target| !self.windows.iter().any(|window| window.id == target))
            {
                self.close_window_preview();
                changed = true;
            }
            if let Some(snapshot) = self.window_menu_snapshot.as_mut()
                && let Some(window) = self.windows.iter().find(|window| window.id == snapshot.id)
                && window != snapshot
            {
                snapshot.clone_from(window);
                changed = true;
            }
        }
        let workspaces = self.window_feed.workspaces();
        if update_feed_status(
            &mut self.workspace_feed_status,
            workspaces.status(),
            "workspaces",
        ) {
            changed = true;
        }
        if let FeedState::Ready(workspaces) = workspaces
            && workspaces != self.workspaces
        {
            self.workspaces = workspaces;
            self.desktop_host.application_mut().set_workspace(
                self.workspaces
                    .iter()
                    .find(|workspace| workspace.active)
                    .map(|workspace| workspace.id),
            );
            if self.window_menu.is_none() {
                self.close_window_preview();
            }
            changed = true;
        }
        if self
            .preview_leave_deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            self.close_window_preview();
            changed = true;
        }
        if let Some((index, deadline)) = self.preview_pending
            && Instant::now() >= deadline
        {
            self.preview_pending = None;
            self.open_window_preview(index);
            changed = true;
        }
        let preview_group = self.preview_group.and_then(|index| {
            let panel_windows = self.panel_windows();
            self.launcher
                .taskbar_applications(&panel_windows)
                .get(index)
                .map(|task| task.window_group())
        });
        let preview_refresh_now = Instant::now();
        if let Some(group) = preview_group
            && preview_refresh_due(self.preview_refresh_deadline, preview_refresh_now)
        {
            retain_preview_generation(&mut self.preview_images, &group.windows);
            for window in group.windows.iter().take(PREVIEW_CACHE_CAPACITY) {
                if let Some(preview) = self.window_feed.preview(window.id) {
                    let image = Arc::new(normalize_preview_image(&preview.image));
                    if self
                        .preview_images
                        .get(&window.id)
                        .is_none_or(|current| **current != *image)
                    {
                        self.preview_images.insert(window.id, image);
                        changed = true;
                    }
                }
            }
            self.preview_refresh_deadline = Some(preview_refresh_now + PREVIEW_REFRESH_INTERVAL);
        }
        let tray = normalize_tray_items(self.tray_feed.snapshot());
        if tray != self.tray {
            self.tray = tray;
            self.tray_icons = panel_tray_icons(&self.tray);
            changed = true;
        }
        let notification = self.notification_feed.snapshot();
        if !self.notification_history_visible && notification != self.notification {
            self.notification = notification;
            self.notification_host
                .application_mut()
                .sync(self.notification.as_ref(), self.palette);
            self.notification_host.step(HostBatch {
                events: vec![HostEvent::Poll],
                ..HostBatch::default()
            });
            changed = true;
        }
        changed
    }

    pub fn refresh_system(&mut self) -> bool {
        let mut changed = false;
        #[cfg(target_os = "linux")]
        {
            let secure_storage_state = match platform::secure_storage_state() {
                Ok(state) => {
                    if self.secure_storage_query_error.take().is_some() {
                        tracing::info!("secure-storage session query recovered");
                    }
                    state
                }
                Err(error) => {
                    let now = Instant::now();
                    let should_log = self.secure_storage_query_error.as_ref().is_none_or(
                        |(previous, logged)| {
                            previous != &error
                                || now.duration_since(*logged) >= RECURRING_DIAGNOSTIC_INTERVAL
                        },
                    );
                    if should_log {
                        tracing::warn!(%error, "secure-storage query failed during shell refresh");
                        self.secure_storage_query_error = Some((error, now));
                    }
                    platform::SecureStorageState::ControlUnavailable
                }
            };
            if secure_storage_state != self.secure_storage_state {
                self.secure_storage_state = secure_storage_state;
                changed = true;
            }
            if self.launcher_status.is_some()
                && secure_storage_state == platform::SecureStorageState::Ready
            {
                self.launcher_status = None;
                self.secure_storage_override = None;
                changed = true;
            }
        }
        let shell_settings = ShellSettings::load_default();
        self.launcher.set_places(crate::places::applications(
            shell_settings.preferred_file_manager.as_deref(),
        ));
        if self.all_windows_on_every_bar != shell_settings.all_windows_on_every_bar {
            self.all_windows_on_every_bar = shell_settings.all_windows_on_every_bar;
            self.close_window_preview();
            changed = true;
        }
        let palette =
            ThemePalette::from_appearance(shell_settings.resolve_appearance(Appearance::default()));
        if palette != self.palette {
            self.palette = palette;
            self.launcher_icons.begin_visual_generation();
            if let Some(icon) = crate::icons::load_svg_bytes(
                include_bytes!("../../../assets/icons/nickel-start.svg"),
                96,
            ) {
                self.panel_icon = Arc::new(tint_panel_icon(icon, palette.text));
            }
            if let Some(icon) = crate::icons::load_svg_bytes(
                include_bytes!("../../../assets/icons/nickel-chat.svg"),
                96,
            ) {
                self.codex_icon = Arc::new(tint_panel_icon(icon, palette.text));
            }
            changed = true;
        }
        let network = platform::network_status();
        if network != self.network {
            self.network = network;
            changed = true;
        }
        let bluetooth = platform::bluetooth_status();
        if bluetooth != self.bluetooth {
            self.bluetooth = bluetooth;
            changed = true;
        }
        let audio = platform::audio_status();
        if audio != self.audio {
            self.audio = audio;
            changed = true;
        }
        changed
    }

    pub fn semantic_theme(&self) -> nickel_ui::SemanticTheme {
        semantic_theme_from_palette(self.palette)
    }

    pub fn scene(&mut self, role: SurfaceRole, width: u32, height: u32) -> Vec<PaintCommand> {
        match role {
            SurfaceRole::Desktop => self.desktop_scene(width, height),
            SurfaceRole::Panel => self.panel_scene(width, height),
            SurfaceRole::Launcher => self.launcher_scene(width, height),
            SurfaceRole::ControlCenter => {
                self.sync_control_host(width, height);
                self.control_host.commands().to_vec()
            }
            SurfaceRole::Notification => {
                self.sync_notification_host(width, height);
                self.notification_host.commands().to_vec()
            }
            SurfaceRole::VolumeOsd => self.volume_osd_scene(width, height),
            SurfaceRole::WindowPreview => self.window_preview_scene(),
            SurfaceRole::WindowContextMenu => self.window_menu_scene(),
            SurfaceRole::Lock => self.lock_scene(width, height),
            SurfaceRole::Screenshot => self.screenshot.scene(width, height, self.palette),
            SurfaceRole::CodexProjectMenu | SurfaceRole::CodexChat => Vec::new(),
        }
    }

    pub fn set_desktop_outputs(&mut self, outputs: Vec<DesktopOutput>) {
        self.desktop_host.application_mut().set_outputs(outputs);
    }

    pub fn set_desktop_output(&mut self, output: String, x: f32, y: f32, scale: f32) {
        self.desktop_host
            .application_mut()
            .set_active_output(output, DesktopPoint { x, y }, scale);
    }

    pub fn desktop_input(&mut self, event: nickel_input::InputEvent) -> bool {
        if matches!(event, nickel_input::InputEvent::FocusLost { .. }) {
            self.desktop_host.application_mut().context_menu = None;
        }
        if self.desktop_host.application().context_menu.is_none()
            && let nickel_input::InputEvent::Pointer(nickel_input::PointerEvent::Button {
                button: nickel_input::PointerButton::Secondary,
                edge: nickel_input::KeyEdge::Pressed,
                position: Some(position),
                ..
            }) = &event
        {
            let application = self.desktop_host.application_mut();
            application.pointer_press(
                DesktopPoint {
                    x: position.x as f32,
                    y: position.y as f32,
                },
                true,
                application.modifiers,
            );
            let outcome = self.desktop_host.step(HostBatch {
                application_changed: true,
                ..HostBatch::default()
            });
            self.desktop_change_token = outcome.change_token;
            self.desktop_deadline = outcome.next_deadline;
        }
        if self.desktop_host.application().context_menu.is_some()
            || matches!(event, nickel_input::InputEvent::Touch(_))
            || matches!(
                event,
                nickel_input::InputEvent::Pointer(nickel_input::PointerEvent::Button {
                    button: nickel_input::PointerButton::Secondary,
                    ..
                })
            )
        {
            let outcome = self.desktop_host.step(HostBatch {
                events: vec![HostEvent::Normalized {
                    input: event,
                    clipboard_text: None,
                }],
                ..HostBatch::default()
            });
            self.desktop_change_token = outcome.change_token;
            self.desktop_deadline = outcome.next_deadline;
            return outcome.changed;
        }
        let application = self.desktop_host.application_mut();
        let changed = match event {
            nickel_input::InputEvent::Key(key) => application.key(&key),
            nickel_input::InputEvent::Pointer(nickel_input::PointerEvent::Button {
                button,
                edge,
                position: Some(position),
                ..
            }) => {
                let point = DesktopPoint {
                    x: position.x as f32,
                    y: position.y as f32,
                };
                if edge == nickel_input::KeyEdge::Pressed {
                    application.pointer_press(point, false, application.modifiers)
                } else if button == nickel_input::PointerButton::Primary {
                    application.pointer_release(point, Instant::now())
                } else {
                    false
                }
            }
            nickel_input::InputEvent::Pointer(nickel_input::PointerEvent::Motion {
                position,
                ..
            }) => application.pointer_motion(DesktopPoint {
                x: position.x as f32,
                y: position.y as f32,
            }),
            _ => false,
        };
        if changed {
            let outcome = self.desktop_host.step(HostBatch {
                application_changed: true,
                ..HostBatch::default()
            });
            self.desktop_change_token = outcome.change_token;
            self.desktop_deadline = outcome.next_deadline;
        }
        changed
    }

    pub fn desktop_controller(&mut self, action: ControllerAction) -> bool {
        let application = self.desktop_host.application_mut();
        let changed = match action {
            ControllerAction::Left => {
                application.layout.select_direction(-1, 0, false);
                true
            }
            ControllerAction::Right => {
                application.layout.select_direction(1, 0, false);
                true
            }
            ControllerAction::Up => {
                application.layout.select_direction(0, -1, false);
                true
            }
            ControllerAction::Down => {
                application.layout.select_direction(0, 1, false);
                true
            }
            ControllerAction::Confirm => {
                if let Some(id) = application.layout.active() {
                    application.activate(id);
                }
                true
            }
            ControllerAction::ContextMenu => {
                application.open_keyboard_context();
                true
            }
            ControllerAction::Cancel => {
                application.context_menu = None;
                application.layout.clear_selection();
                true
            }
            ControllerAction::Launcher
            | ControllerAction::PreviousPane
            | ControllerAction::NextPane => false,
        };
        if !changed {
            return false;
        }
        let outcome = self.desktop_host.step(HostBatch {
            application_changed: true,
            ..HostBatch::default()
        });
        self.desktop_change_token = outcome.change_token;
        self.desktop_deadline = outcome.next_deadline;
        true
    }

    pub fn desktop_file_drop(&mut self, source: &std::path::Path) -> bool {
        let application = self.desktop_host.application_mut();
        let target = application
            .hit(application.pointer_position)
            .and_then(|id| application.layout.items().iter().find(|item| item.id == id));
        if let Some(launcher) = target.filter(|item| {
            !item.entry.is_directory && nickel_file::is_application_launcher(&item.entry.path)
        }) {
            return match nickel_file::open_with_launcher(&launcher.entry.path, source) {
                Ok(()) => true,
                Err(error) => {
                    application.error = Some(format!(
                        "Could not open {} with {}: {error}",
                        source.display(),
                        launcher.entry.display_name()
                    ));
                    true
                }
            };
        }
        let destination = target
            .filter(|item| item.entry.is_directory)
            .map(|item| item.entry.path.clone())
            .unwrap_or_else(nickel_file::desktop_directory);
        match nickel_file::copy_into(source, &destination) {
            Ok(_) => application.refresh_directory(true),
            Err(error) => {
                application.error = Some(format!("Could not drop {}: {error}", source.display()));
                true
            }
        }
    }

    pub fn surface_visible(&self, role: SurfaceRole) -> bool {
        match role {
            SurfaceRole::Desktop | SurfaceRole::Panel => true,
            SurfaceRole::Launcher => self.launcher_visible,
            SurfaceRole::ControlCenter => self.control_visible,
            SurfaceRole::Notification => {
                self.notification.is_some() || self.notification_history_visible
            }
            SurfaceRole::VolumeOsd => self.volume_osd_until.is_some(),
            SurfaceRole::WindowPreview => {
                self.preview_group.is_some() || self.task_switcher_group.is_some()
            }
            SurfaceRole::WindowContextMenu => self.window_menu.is_some(),
            SurfaceRole::CodexProjectMenu => self.codex_project_menu_visible,
            SurfaceRole::Lock => self.locked,
            SurfaceRole::Screenshot => self.screenshot.visible(),
            SurfaceRole::CodexChat => true,
        }
    }

    pub fn next_host_deadline(&self) -> Option<Instant> {
        self.host_deadline_sources()
            .into_iter()
            .map(|(_, deadline)| deadline)
            .min()
    }

    pub fn host_deadline_sources(&self) -> Vec<(&'static str, Instant)> {
        let mut sources = Vec::new();
        let mut push = |name, deadline| {
            if let Some(deadline) = deadline {
                sources.push((name, deadline));
            }
        };
        push("desktop", self.desktop_deadline);
        push("panel", self.panel_deadline);
        push("lock", self.lock_deadline);
        push("control", self.control_deadline);
        push("screenshot", self.screenshot.next_deadline());
        push(
            "window-preview-host",
            self.preview_frame
                .as_ref()
                .and_then(WindowPreviewFrame::next_deadline),
        );
        push(
            "window-preview-open",
            self.preview_pending.map(|(_, deadline)| deadline),
        );
        push("window-preview-close", self.preview_leave_deadline);
        push("volume-osd", self.volume_osd_until);
        sources
    }

    pub fn scene_change_token(&self, role: SurfaceRole) -> Option<HostChangeToken> {
        let host_token = |inspection: nickel_ui::HostInspection| HostChangeToken {
            frame_generation: inspection.frame_generation,
            semantic_generation: inspection.semantic_generation,
        };
        match role {
            SurfaceRole::Desktop => Some(self.desktop_change_token),
            SurfaceRole::Panel => Some(self.panel_change_token),
            SurfaceRole::Lock => Some(self.lock_change_token),
            SurfaceRole::Launcher => Some(host_token(self.launcher_host.inspect())),
            SurfaceRole::ControlCenter => Some(self.control_change_token),
            SurfaceRole::Notification => Some(host_token(self.notification_host.inspect())),
            SurfaceRole::VolumeOsd => None,
            SurfaceRole::WindowPreview => {
                self.preview_frame.as_ref().map(|host| host.change_token())
            }
            SurfaceRole::WindowContextMenu => self
                .window_menu_host
                .as_ref()
                .map(|host| host_token(host.inspect())),
            SurfaceRole::Screenshot => Some(self.screenshot.change_token()),
            SurfaceRole::CodexProjectMenu | SurfaceRole::CodexChat => None,
        }
    }

    pub fn launcher_host_input(
        &mut self,
        input: nickel_input::InputEvent,
        clipboard_text: Option<String>,
        width: u32,
        height: u32,
    ) -> nickel_ui::HostEventOutcome {
        let status = self.launcher_status_text();
        self.launcher_host
            .application_mut()
            .sync(&self.launcher, self.palette, status);
        let outcome = self.launcher_host.step(HostBatch {
            surface_size: Some((width, height)),
            events: vec![HostEvent::Normalized {
                input,
                clipboard_text,
            }],
            ..HostBatch::default()
        });
        let actions = self.launcher_host.application_mut().take_effects();
        for action in actions {
            self.apply_launcher_action(action);
        }
        self.host_runtime_samples.record(outcome.telemetry);
        outcome
    }

    pub fn launcher_host_controller(
        &mut self,
        action: ControllerAction,
        family: nickel_ui::ControllerFamily,
    ) -> bool {
        let status = self.launcher_status_text();
        self.launcher_host
            .application_mut()
            .sync(&self.launcher, self.palette, status);
        self.launcher_host
            .application_mut()
            .set_controller_family(family);
        let event = launcher_controller_host_event(
            action,
            self.launcher_host.inspect().open_overlay.is_some(),
        );
        let outcome = self.launcher_host.step(HostBatch {
            events: vec![event],
            ..HostBatch::default()
        });
        let actions = self.launcher_host.application_mut().take_effects();
        for action in actions {
            self.apply_launcher_action(action);
        }
        self.host_runtime_samples.record(outcome.telemetry);
        outcome.changed
    }

    pub fn set_launcher_controller_family(&mut self, family: nickel_ui::ControllerFamily) {
        self.launcher_host
            .application_mut()
            .set_controller_family(family);
        self.launcher_host.step(HostBatch {
            application_changed: true,
            ..HostBatch::default()
        });
    }

    pub fn poll_host_deadlines(&mut self, now: Instant) -> Vec<SurfaceRole> {
        let mut changed = Vec::new();

        if self
            .desktop_deadline
            .is_some_and(|deadline| now >= deadline)
        {
            let outcome = self.desktop_host.step(HostBatch {
                now: Some(now),
                events: vec![HostEvent::Poll],
                ..HostBatch::default()
            });
            self.desktop_change_token = outcome.change_token;
            self.desktop_deadline = outcome.next_deadline;
            if outcome.changed {
                changed.push(SurfaceRole::Desktop);
            }
        }
        if self.panel_deadline.is_some_and(|deadline| now >= deadline) {
            let outcome = self.panel_host.step(HostBatch {
                now: Some(now),
                events: vec![HostEvent::Poll],
                ..HostBatch::default()
            });
            self.panel_change_token = outcome.change_token;
            self.panel_deadline = outcome.next_deadline;
            if outcome.changed {
                changed.push(SurfaceRole::Panel);
            }
        }
        if self.lock_deadline.is_some_and(|deadline| now >= deadline) {
            let outcome = self.lock_host.step(HostBatch {
                now: Some(now),
                events: vec![HostEvent::Poll],
                ..HostBatch::default()
            });
            self.lock_change_token = outcome.change_token;
            self.lock_deadline = outcome.next_deadline;
            if outcome.changed {
                changed.push(SurfaceRole::Lock);
            }
        }
        if self
            .control_deadline
            .is_some_and(|deadline| now >= deadline)
        {
            let control_changed = self.step_control_host(HostBatch {
                now: Some(now),
                events: vec![HostEvent::Poll],
                ..HostBatch::default()
            });
            self.apply_control_effects();
            if control_changed {
                changed.push(SurfaceRole::ControlCenter);
            }
        }
        if self
            .preview_frame
            .as_ref()
            .and_then(WindowPreviewFrame::next_deadline)
            .is_some_and(|deadline| now >= deadline)
            && let Some(frame) = self.preview_frame.as_mut()
            && frame
                .step(HostBatch {
                    now: Some(now),
                    events: vec![HostEvent::Poll],
                    ..HostBatch::default()
                })
                .changed
        {
            changed.push(SurfaceRole::WindowPreview);
        }
        changed
    }

    pub fn poll_deadlines(&mut self, now: Instant) -> ShellDeadlineOutcome {
        let mut outcome = ShellDeadlineOutcome {
            redraw: self.poll_host_deadlines(now),
            ..ShellDeadlineOutcome::default()
        };
        if let Some((index, deadline)) = self.preview_pending
            && now >= deadline
        {
            self.preview_pending = None;
            let previous = self.preview_group;
            self.open_window_preview(index);
            outcome.visibility_changed |= self.preview_group != previous;
            if self.preview_group.is_some() {
                outcome.redraw.push(SurfaceRole::WindowPreview);
            }
        }
        if self
            .preview_leave_deadline
            .is_some_and(|deadline| now >= deadline)
        {
            self.preview_leave_deadline = None;
            let was_open = self.preview_group.is_some();
            self.close_window_preview();
            outcome.visibility_changed |= was_open;
        }
        outcome.capture_screenshot = self.screenshot.capture_ready_at(now);
        if self.screenshot.poll_pointer_deadline(now) {
            outcome.redraw.push(SurfaceRole::Screenshot);
        }
        if self
            .volume_osd_until
            .is_some_and(|deadline| now >= deadline)
        {
            self.volume_osd_until = None;
            outcome.visibility_changed = true;
        }
        if self
            .projection_rollback_deadline
            .is_some_and(|deadline| now >= deadline)
        {
            self.rollback_projection();
            outcome.visibility_changed = true;
        }
        outcome
    }

    pub fn notification_click(&mut self, x: f32, y: f32, width: u32, height: u32) -> bool {
        if self.notification.is_none() && !self.notification_history_visible {
            return false;
        }
        self.sync_notification_host(width, height);
        let point = Point { x, y };
        let outcome = self.notification_host.step(HostBatch {
            events: vec![
                HostEvent::Ui(UiEvent::PointerPressed(point)),
                HostEvent::Ui(UiEvent::PointerReleased(point)),
            ],
            ..HostBatch::default()
        });
        if outcome.effects.is_empty() {
            self.notification_host.application_mut().request_dismiss();
        }
        self.apply_notification_effects()
    }

    pub fn notification_key(&mut self, key: Option<KeyCode>) -> bool {
        if self.notification.is_none() && !self.notification_history_visible {
            return false;
        }
        self.sync_notification_host(420, 180);
        let event = match key {
            Some(KeyCode::Escape) => HostEvent::Shortcut(Shortcut::Escape),
            Some(KeyCode::ArrowLeft | KeyCode::ArrowUp) => {
                HostEvent::Controller(ControllerAction::Left)
            }
            Some(KeyCode::ArrowRight | KeyCode::ArrowDown | KeyCode::Tab) => {
                HostEvent::Controller(ControllerAction::Right)
            }
            Some(KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space) => {
                HostEvent::Controller(ControllerAction::Confirm)
            }
            _ => return false,
        };
        self.notification_host.step(HostBatch {
            events: vec![event],
            ..HostBatch::default()
        });
        self.apply_notification_effects();
        true
    }

    pub fn notification_controller(&mut self, action: ControllerAction) -> bool {
        if self.notification.is_none() && !self.notification_history_visible {
            return false;
        }
        self.sync_notification_host(420, 180);
        let event = if action == ControllerAction::Cancel {
            HostEvent::Shortcut(Shortcut::Escape)
        } else {
            HostEvent::Controller(action)
        };
        let outcome = self.notification_host.step(HostBatch {
            events: vec![event],
            ..HostBatch::default()
        });
        self.apply_notification_effects();
        outcome.changed
    }

    pub fn panel_click(&mut self, x: f32, width: u32, secondary: bool) -> bool {
        self.sync_panel_host();
        let events = if secondary {
            vec![HostEvent::Ui(UiEvent::PointerContext(Point { x, y: 28.0 }))]
        } else {
            vec![
                HostEvent::Ui(UiEvent::PointerPressed(Point { x, y: 28.0 })),
                HostEvent::Ui(UiEvent::PointerReleased(Point { x, y: 28.0 })),
            ]
        };
        let outcome = self.panel_host.step(HostBatch {
            surface_size: Some((width, 56)),
            events,
            ..HostBatch::default()
        });
        self.panel_change_token = outcome.change_token;
        self.panel_deadline = outcome.next_deadline;
        self.apply_panel_effects()
    }

    pub fn panel_controller(&mut self, action: ControllerAction, width: u32) -> bool {
        self.sync_panel_host();
        let outcome = self.panel_host.step(HostBatch {
            surface_size: Some((width, 56)),
            events: vec![HostEvent::Controller(action)],
            ..HostBatch::default()
        });
        self.panel_change_token = outcome.change_token;
        self.panel_deadline = outcome.next_deadline;
        self.apply_panel_effects();
        outcome.changed
    }

    fn apply_panel_action(&mut self, action: PanelAction) {
        let anchored_role = match &action {
            PanelAction::Codex => Some((ShellRole::ProjectMenu, "panel-codex")),
            PanelAction::Control => Some((ShellRole::ControlCenter, "panel-control")),
            _ => None,
        };
        if anchored_role.is_some() {
            self.pending_popover_anchor = None;
        }
        if let Some((role, control)) = anchored_role
            && let (Some(output), Some(target)) = (
                self.panel_output.clone(),
                self.panel_host
                    .semantic_targets_for_message(&action)
                    .into_iter()
                    .next(),
            )
        {
            self.pending_popover_anchor = Some(PendingPopoverAnchor {
                role,
                control: control.to_owned(),
                output,
                bounds: target.bounds,
            });
        }
        match action {
            PanelAction::Launcher => self.set_launcher_visible(!self.launcher_visible),
            PanelAction::Task(index) => {
                let panel_windows = self.panel_windows();
                let groups = self.launcher.taskbar_applications(&panel_windows);
                if groups
                    .get(index)
                    .is_some_and(|group| group.windows.len() > 1)
                {
                    self.open_window_preview(index);
                    self.preview_focus_requested = true;
                } else if let Some(window) =
                    groups.get(index).and_then(|group| group.windows.first())
                {
                    let _ = send_session_command(
                        "activate-window",
                        ShellCommand::WindowAction {
                            window: window.id,
                            action: WindowAction::Activate,
                        },
                    );
                    self.close_window_preview();
                } else if let Some(application_id) = groups
                    .get(index)
                    .filter(|group| group.available)
                    .and_then(|group| group.application_id.as_ref())
                {
                    self.launch_application_by_id(application_id.as_str());
                }
            }
            PanelAction::TaskContext(index) => {
                let panel_windows = self.panel_windows();
                let groups = self.launcher.taskbar_applications(&panel_windows);
                let Some(group) = groups.get(index) else {
                    return;
                };
                let window_snapshot = group
                    .windows
                    .iter()
                    .find(|window| window.active)
                    .or_else(|| group.windows.first())
                    .cloned()
                    .or_else(|| {
                        let application_id = group.application_id.clone()?;
                        Some(OpenWindow {
                            // Application-only menus never dispatch a window action;
                            // capabilities are deliberately empty. The stable target
                            // is the canonical application identity below.
                            id: crate::model::WindowId(u64::MAX),
                            application_id: Some(application_id),
                            active: false,
                            title: group.application_name.clone(),
                            state: crate::model::WindowState {
                                capabilities: crate::model::WindowCapabilities {
                                    activate: false,
                                    close: false,
                                    minimize: false,
                                    maximize: false,
                                    fullscreen: false,
                                    move_workspace: false,
                                    move_display: false,
                                },
                                ..crate::model::WindowState::default()
                            },
                        })
                    });
                let Some(window_snapshot) = window_snapshot else {
                    return;
                };
                let window = group
                    .windows
                    .iter()
                    .any(|candidate| candidate.id == window_snapshot.id)
                    .then_some(window_snapshot.id);
                self.close_window_preview();
                self.window_menu = window;
                self.window_menu_snapshot = Some(window_snapshot);
                self.window_menu_host = None;
                let x = self
                    .panel_host
                    .semantic_targets_for_message(&PanelAction::Task(index))
                    .into_iter()
                    .next()
                    .map(|target| target.bounds.origin.x.round() as i32)
                    .unwrap_or((PANEL_ITEM_WIDTH * (index + 1) as f32).round() as i32);
                self.window_menu_anchor_x = Some(self.panel_origin_x + x);
                let _ = send_session_command(
                    "show-context-menu",
                    ShellCommand::ShowContextMenu {
                        x: self.panel_origin_x + x,
                        width: MENU_WIDTH as i32,
                        height: self.window_context_menu_height(),
                    },
                );
                #[cfg(target_os = "linux")]
                let _ = send_session_command("focus-context-menu", ShellCommand::FocusContextMenu);
            }
            PanelAction::ToggleTaskPin(id) => {
                self.launcher.toggle_pin(&id);
                self.persist_launcher_preferences();
            }
            PanelAction::MoveTaskPinLeft(id) => {
                if self.launcher.move_pin(&id, -1) {
                    self.persist_launcher_preferences();
                }
            }
            PanelAction::MoveTaskPinRight(id) => {
                if self.launcher.move_pin(&id, 1) {
                    self.persist_launcher_preferences();
                }
            }
            // Drag gestures are reduced by `PanelApplication` into a typed move action.
            PanelAction::TaskDrag(_, _) => {}
            PanelAction::Codex => {
                if !self.launcher.codex_available() {
                    self.codex_project_menu_visible = false;
                    return;
                }
                if self.launcher_visible {
                    self.set_launcher_visible(false);
                }
                self.codex_project_menu_visible = !self.codex_project_menu_visible;
            }
            PanelAction::Tray(id) => self.tray_feed.activate(&id),
            PanelAction::TrayContext(id) => self.tray_feed.context_menu(&id),
            PanelAction::Control => {
                if self.launcher_visible {
                    self.set_launcher_visible(false);
                }
                self.set_control_visible(!self.control_visible);
            }
        }
    }

    /// Resolves a test/accessibility semantic target from the same live group
    /// and renderer frame records used by pointer hit testing. The caller is
    /// responsible for dispatching the returned point as ordinary input.
    #[cfg(any(target_os = "linux", test))]
    pub fn resolve_semantic_target(
        &self,
        target: &ShellSemanticTarget,
    ) -> Option<ResolvedShellTarget> {
        match target {
            ShellSemanticTarget::PanelApplication {
                application_id,
                output,
                interaction,
            } => {
                let groups = self.launcher.taskbar_applications(&self.windows);
                let index = groups.iter().take(12).position(|group| {
                    group
                        .application_id
                        .as_ref()
                        .is_some_and(|id| id.as_str() == application_id)
                })?;
                let bounds = self
                    .panel_host
                    .semantic_targets_for_message(&PanelAction::Task(index))
                    .into_iter()
                    .next()?
                    .bounds;
                Some(ResolvedShellTarget {
                    role: ShellRole::Panel,
                    output: output.clone(),
                    x: (bounds.origin.x + bounds.size.width / 2.0).round() as i32,
                    y: (bounds.origin.y + bounds.size.height / 2.0).round() as i32,
                    interaction: *interaction,
                })
            }
            ShellSemanticTarget::PreviewWindow { window, action } => {
                let window = crate::model::WindowId(window.0);
                let preview_action = match action {
                    PreviewTargetAction::Hover | PreviewTargetAction::Activate => {
                        PreviewAction::Activate(window)
                    }
                    PreviewTargetAction::Close => PreviewAction::Close(window),
                    PreviewTargetAction::OpenMenu => PreviewAction::OpenMenu(window),
                };
                let bounds = self
                    .preview_frame
                    .as_ref()?
                    .semantic_bounds(preview_action)?;
                let point = Point {
                    x: bounds.origin.x + bounds.size.width / 2.0,
                    y: bounds.origin.y + bounds.size.height / 2.0,
                };
                Some(ResolvedShellTarget {
                    role: ShellRole::Preview,
                    output: None,
                    x: point.x.round() as i32,
                    y: point.y.round() as i32,
                    interaction: match action {
                        PreviewTargetAction::Hover => PointerInteraction::Hover,
                        PreviewTargetAction::OpenMenu => PointerInteraction::RightClick,
                        PreviewTargetAction::Activate | PreviewTargetAction::Close => {
                            PointerInteraction::LeftClick
                        }
                    },
                })
            }
            ShellSemanticTarget::WindowMenu { window, action } => {
                let window = crate::model::WindowId(window.0);
                let menu_action = match action {
                    WindowMenuTargetAction::Close => MenuAction::Close(window),
                    WindowMenuTargetAction::MaximizeRestore => MenuAction::MaximizeRestore(window),
                    WindowMenuTargetAction::Minimize => MenuAction::Minimize(window),
                };
                let bounds = self
                    .window_menu_host
                    .as_ref()?
                    .semantic_targets_for_message(&menu_action)
                    .into_iter()
                    .next()?
                    .bounds;
                let point = Point {
                    x: bounds.origin.x + bounds.size.width / 2.0,
                    y: bounds.origin.y + bounds.size.height / 2.0,
                };
                Some(ResolvedShellTarget {
                    role: ShellRole::ContextMenu,
                    output: None,
                    x: point.x.round() as i32,
                    y: point.y.round() as i32,
                    interaction: PointerInteraction::LeftClick,
                })
            }
            ShellSemanticTarget::Screenshot { .. } => None,
        }
    }

    #[cfg(any(target_os = "linux", test))]
    pub fn perform_screenshot_semantic_action(
        &mut self,
        action: nickel_session_protocol::ScreenshotTargetAction,
    ) -> bool {
        self.screenshot.perform_semantic_action(action)
    }

    pub fn panel_pointer_moved(&mut self, x: f32, width: u32) -> bool {
        self.sync_panel_host();
        self.panel_host.step(HostBatch {
            surface_size: Some((width, 56)),
            events: vec![HostEvent::Ui(UiEvent::PointerMoved(Point { x, y: 28.0 }))],
            ..HostBatch::default()
        });
        let hovered_action = self
            .panel_host
            .inspect()
            .pointer_hover
            .as_ref()
            .and_then(|target| self.panel_host.message_for_semantic_target(target))
            .cloned();
        let hovered = hovered_action
            .as_ref()
            .and_then(|action| self.panel_hover_for_action(action));
        let changed = hovered != self.panel_hover;
        self.panel_hover = hovered;
        self.panel_hover_output.clone_from(&self.panel_output);
        if let Some(PanelHover::Task(index)) = hovered {
            if self.preview_group != Some(index)
                && self.preview_pending.map(|(pending, _)| pending) != Some(index)
            {
                self.preview_pending = Some((index, Instant::now() + PREVIEW_HOVER_DELAY));
            }
            self.preview_leave_deadline = None;
        } else {
            self.preview_pending = None;
            if self.preview_group.is_some() && !self.preview_pointer_inside {
                self.preview_leave_deadline = Some(Instant::now() + PREVIEW_LEAVE_DELAY);
            }
        }
        changed
    }

    fn panel_hover_for_action(&self, action: &PanelAction) -> Option<PanelHover> {
        Some(match action {
            PanelAction::Launcher => PanelHover::Launcher,
            PanelAction::Task(index)
            | PanelAction::TaskContext(index)
            | PanelAction::TaskDrag(index, _) => PanelHover::Task(*index),
            PanelAction::ToggleTaskPin(_)
            | PanelAction::MoveTaskPinLeft(_)
            | PanelAction::MoveTaskPinRight(_) => return None,
            PanelAction::Codex => PanelHover::Codex,
            PanelAction::Tray(id) | PanelAction::TrayContext(id) => self
                .tray
                .iter()
                .rev()
                .take(4)
                .rev()
                .position(|item| item.id == id.as_str())
                .map(PanelHover::Tray)
                .unwrap_or(PanelHover::Tray(0)),
            PanelAction::Control => PanelHover::Control,
        })
    }

    pub fn set_panel_origin_x(&mut self, origin_x: i32) {
        self.panel_origin_x = origin_x;
    }

    pub fn set_panel_output(&mut self, output: impl Into<String>) {
        self.panel_output = Some(output.into());
    }

    fn visible_panel_hover(&self) -> Option<PanelHover> {
        (self.panel_hover_output == self.panel_output)
            .then_some(self.panel_hover)
            .flatten()
    }

    #[cfg(any(target_os = "linux", test))]
    pub fn popover_anchor(&self, preferred: AnchorSide) -> Option<(ShellRole, ShellPopoverAnchor)> {
        let anchor = self.pending_popover_anchor.as_ref()?;
        let visible = match anchor.role {
            ShellRole::ControlCenter => self.control_visible,
            ShellRole::ProjectMenu => self.codex_project_menu_visible,
            _ => false,
        };
        if !visible {
            return None;
        }
        let bounds = Geometry {
            x: anchor.bounds.origin.x.floor() as i32,
            y: anchor.bounds.origin.y.floor() as i32,
            width: anchor.bounds.size.width.ceil().max(1.0) as i32,
            height: anchor.bounds.size.height.ceil().max(1.0) as i32,
        };
        Some((
            anchor.role,
            ShellPopoverAnchor {
                control: anchor.control.clone(),
                output: anchor.output.clone(),
                bounds,
                preferred,
            },
        ))
    }

    fn panel_windows(&self) -> Vec<OpenWindow> {
        if self.all_windows_on_every_bar {
            return self.windows.clone();
        }
        let Some(output) = self.panel_output.as_deref() else {
            return self.windows.clone();
        };
        self.windows
            .iter()
            .filter(|window| {
                window_belongs_to_panel(
                    false,
                    Some(output),
                    self.window_feed.window_output(window.id).as_deref(),
                )
            })
            .cloned()
            .collect()
    }

    pub fn primary_output_name(&self) -> Option<String> {
        self.window_feed.primary_output()
    }

    pub fn panel_pointer_left(&mut self) -> bool {
        if self.panel_hover.is_none() || self.panel_hover_output != self.panel_output {
            return false;
        }
        self.panel_hover = None;
        self.panel_hover_output = None;
        self.preview_pending = None;
        if self.preview_group.is_some() && !self.preview_pointer_inside {
            self.preview_leave_deadline = Some(Instant::now() + PREVIEW_LEAVE_DELAY);
        }
        true
    }

    pub fn preview_controller(&mut self, action: ControllerAction) -> bool {
        if self.window_menu.is_some() {
            return self.window_menu_host_controller(action);
        }
        let Some(frame) = self.preview_frame.as_mut() else {
            return false;
        };
        let primed = action != ControllerAction::Cancel && frame.ensure_controller_selection();
        let outcome = frame.step(HostBatch {
            events: vec![HostEvent::Controller(action)],
            ..HostBatch::default()
        });
        let actions = frame.take_actions();
        for action in actions {
            self.apply_preview_action(action);
        }
        outcome.changed || primed
    }

    pub fn panel_pointer_entered(&mut self) -> bool {
        self.preview_leave_deadline.take().is_some()
    }

    pub fn preview_pointer_entered(&mut self, entered: bool) -> bool {
        if self.preview_pointer_inside == entered {
            return false;
        }
        self.preview_pointer_inside = entered;
        if entered {
            self.preview_leave_deadline = None;
        } else {
            self.preview_leave_deadline = Some(Instant::now() + PREVIEW_LEAVE_DELAY);
            if self.preview_hovered.take().is_some() {
                let _ = send_session_command(
                    "clear-window-highlight",
                    ShellCommand::ClearWindowHighlight,
                );
            }
        }
        true
    }

    pub fn preview_pointer_moved(&mut self, x: f32, y: f32) -> bool {
        let hovered = self
            .preview_frame
            .as_mut()
            .and_then(|frame| frame.transition_pointer_hover(Point { x, y }));
        if hovered == self.preview_hovered {
            return false;
        }
        self.preview_hovered = hovered;
        let command = hovered.map_or(
            ShellCommand::ClearWindowHighlight,
            ShellCommand::HighlightWindow,
        );
        let _ = send_session_command("highlight-preview-window", command);
        true
    }

    pub fn preview_click(&mut self, x: f32, y: f32, right_click: bool) -> bool {
        let Some(action) = self
            .preview_frame
            .as_mut()
            .and_then(|frame| frame.transition_pointer(Point { x, y }, right_click))
        else {
            return false;
        };
        self.apply_preview_action(action);
        true
    }

    pub fn preview_host_input(
        &mut self,
        input: nickel_input::InputEvent,
    ) -> nickel_ui::HostEventOutcome {
        let Some(frame) = self.preview_frame.as_mut() else {
            return nickel_ui::HostEventOutcome::default();
        };
        let outcome = frame.step(HostBatch {
            events: vec![HostEvent::Normalized {
                input,
                clipboard_text: None,
            }],
            ..HostBatch::default()
        });
        let actions = frame.take_actions();
        for action in actions {
            self.apply_preview_action(action);
        }
        outcome
    }

    fn apply_preview_action(&mut self, action: PreviewAction) {
        match action {
            PreviewAction::Activate(window) => {
                self.send_window_action(window, WindowAction::Activate);
                self.close_window_preview();
            }
            PreviewAction::Close(window) => {
                self.send_window_action(window, WindowAction::Close);
            }
            PreviewAction::OpenMenu(window) => {
                self.window_menu = Some(window);
                self.window_menu_snapshot = self
                    .windows
                    .iter()
                    .find(|candidate| candidate.id == window)
                    .cloned();
                self.window_menu_host = None;
                let x = self.panel_origin_x
                    + self.preview_group.map_or(0, |index| {
                        (PANEL_ITEM_WIDTH + index as f32 * PANEL_ITEM_WIDTH) as i32
                    });
                self.window_menu_anchor_x = Some(x);
                let _ = send_session_command(
                    "show-context-menu",
                    ShellCommand::ShowContextMenu {
                        x,
                        width: MENU_WIDTH as i32,
                        height: self.window_context_menu_height(),
                    },
                );
                #[cfg(target_os = "linux")]
                let _ = send_session_command("focus-context-menu", ShellCommand::FocusContextMenu);
            }
            PreviewAction::Dismiss => self.close_window_preview(),
        }
    }

    pub fn preview_key(&mut self, key: Option<KeyCode>) -> bool {
        if self.window_menu.is_some() {
            return self.window_menu_host_key(key);
        }
        let Some(frame) = self.preview_frame.as_mut() else {
            return false;
        };
        if !matches!(key, Some(KeyCode::Escape) | None) {
            let _ = frame.ensure_controller_selection();
        }
        match key {
            Some(KeyCode::Escape) => {
                frame.step(HostBatch {
                    events: vec![HostEvent::Shortcut(Shortcut::Escape)],
                    ..HostBatch::default()
                });
                let dismissed = frame.take_actions().contains(&PreviewAction::Dismiss);
                if dismissed {
                    self.close_window_preview();
                }
                #[cfg(target_os = "linux")]
                let _ = send_session_command(
                    "restore-application-focus",
                    ShellCommand::RestoreApplicationFocus,
                );
            }
            Some(KeyCode::ArrowLeft | KeyCode::ArrowUp) => {
                frame.step(HostBatch {
                    events: vec![HostEvent::Controller(ControllerAction::Left)],
                    ..HostBatch::default()
                });
                self.preview_hovered = frame.controller_selected_window();
                if let Some(window) = self.preview_hovered {
                    let _ = send_session_command(
                        "highlight-preview-window",
                        ShellCommand::HighlightWindow(window),
                    );
                }
            }
            Some(KeyCode::ArrowRight | KeyCode::ArrowDown | KeyCode::Tab) => {
                frame.step(HostBatch {
                    events: vec![HostEvent::Controller(ControllerAction::Right)],
                    ..HostBatch::default()
                });
                self.preview_hovered = frame.controller_selected_window();
                if let Some(window) = self.preview_hovered {
                    let _ = send_session_command(
                        "highlight-preview-window",
                        ShellCommand::HighlightWindow(window),
                    );
                }
            }
            Some(KeyCode::Delete) => {
                if !frame.close_controller_selected() {
                    return false;
                }
                for action in frame.take_actions() {
                    self.apply_preview_action(action);
                }
            }
            Some(KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space) => {
                frame.step(HostBatch {
                    events: vec![HostEvent::Controller(ControllerAction::Confirm)],
                    ..HostBatch::default()
                });
                let actions = frame.take_actions();
                for action in actions {
                    if let PreviewAction::Activate(window) = action {
                        self.send_window_action(window, WindowAction::Activate);
                        self.close_window_preview();
                    }
                }
            }
            _ => return false,
        }
        true
    }

    fn apply_window_menu_action(&mut self, action: MenuAction) {
        match action {
            MenuAction::Dismiss => self.dismiss_window_menu(),
            MenuAction::ShowWorkspaces | MenuAction::ShowDisplays | MenuAction::Back => {}
            MenuAction::Activate(window) => self.send_window_action(window, WindowAction::Activate),
            MenuAction::Close(window) => self.send_window_action(window, WindowAction::Close),
            MenuAction::MaximizeRestore(window) => {
                self.send_window_action(window, WindowAction::Maximize)
            }
            MenuAction::Minimize(window) => self.send_window_action(window, WindowAction::Minimize),
            MenuAction::FullscreenRestore(window) => {
                self.send_window_action(window, WindowAction::Fullscreen)
            }
            MenuAction::SnapLeading(window) => {
                self.send_window_action(window, WindowAction::SnapLeading)
            }
            MenuAction::SnapTrailing(window) => {
                self.send_window_action(window, WindowAction::SnapTrailing)
            }
            MenuAction::MoveToWorkspace(window, workspace) => {
                let _ = send_session_command(
                    "move-window-to-workspace",
                    ShellCommand::MoveWindowToWorkspace { window, workspace },
                );
            }
            MenuAction::MoveToDisplay(window, output) => {
                let _ = send_session_command(
                    "move-window-to-display",
                    ShellCommand::MoveWindowToDisplay { window, output },
                );
            }
            MenuAction::NewWindow(application) => self.apply_launcher_action(
                LauncherAction::LaunchApplication(application.as_str().to_owned()),
            ),
            MenuAction::TogglePin(application) => self
                .apply_launcher_action(LauncherAction::TogglePin(application.as_str().to_owned())),
            MenuAction::MovePinLeft(application) => {
                if self.launcher.move_pin(application.as_str(), -1) {
                    self.persist_launcher_preferences();
                }
            }
            MenuAction::MovePinRight(application) => {
                if self.launcher.move_pin(application.as_str(), 1) {
                    self.persist_launcher_preferences();
                }
            }
        }
    }

    pub fn window_menu_host_input(
        &mut self,
        input: nickel_input::InputEvent,
        width: u32,
        height: u32,
    ) -> bool {
        if self.window_menu_host.is_none() {
            let _ = self.window_menu_scene();
        }
        let Some(host) = self.window_menu_host.as_mut() else {
            return false;
        };
        let outcome = host.step(HostBatch {
            surface_size: Some((width, height)),
            events: vec![HostEvent::Normalized {
                input,
                clipboard_text: None,
            }],
            ..HostBatch::default()
        });
        for failure in &outcome.failures {
            tracing::warn!(
                ?failure,
                "window context menu host reported recoverable failure"
            );
        }
        let actions = host.application_mut().take_effects();
        for action in actions {
            self.apply_window_menu_action(action);
            self.close_window_preview();
        }
        outcome.changed
    }

    pub fn window_menu_host_key(&mut self, key: Option<KeyCode>) -> bool {
        if self.window_menu_host.is_none() {
            let _ = self.window_menu_scene();
        }
        let event = match key {
            Some(KeyCode::Escape) => HostEvent::Shortcut(Shortcut::Escape),
            Some(KeyCode::ArrowUp | KeyCode::ArrowLeft) => {
                HostEvent::Controller(ControllerAction::Up)
            }
            Some(KeyCode::ArrowDown | KeyCode::ArrowRight | KeyCode::Tab) => {
                HostEvent::Controller(ControllerAction::Down)
            }
            Some(KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space) => {
                HostEvent::Controller(ControllerAction::Confirm)
            }
            _ => return false,
        };
        let Some(host) = self.window_menu_host.as_mut() else {
            return false;
        };
        let outcome = host.step(HostBatch {
            events: vec![event],
            ..HostBatch::default()
        });
        for failure in &outcome.failures {
            tracing::warn!(
                ?failure,
                "window context menu host reported recoverable failure"
            );
        }
        let actions = host.application_mut().take_effects();
        for action in actions {
            self.apply_window_menu_action(action);
            self.close_window_preview();
        }
        if key == Some(KeyCode::Escape) {
            self.dismiss_window_menu();
        }
        outcome.changed
    }

    pub fn window_menu_host_controller(&mut self, action: ControllerAction) -> bool {
        if self.window_menu_host.is_none() {
            let _ = self.window_menu_scene();
        }
        let Some(host) = self.window_menu_host.as_mut() else {
            return false;
        };
        let outcome = host.step(HostBatch {
            events: vec![HostEvent::Controller(action)],
            ..HostBatch::default()
        });
        let effects = host.application_mut().take_effects();
        for effect in effects {
            self.apply_window_menu_action(effect);
            self.close_window_preview();
        }
        if action == ControllerAction::Cancel {
            self.dismiss_window_menu();
        }
        outcome.changed
    }

    pub fn sync_transient_overlays(&mut self) {
        if let Some(group) = &self.task_switcher_group {
            let windows = group
                .windows
                .iter()
                .map(|window| window.id)
                .collect::<Vec<_>>();
            let (width, height) = preview_dimensions(windows.len());
            let _ = send_session_command(
                "show-task-switcher",
                ShellCommand::ShowTaskSwitcher {
                    width: width as i32,
                    height: height as i32,
                    windows,
                },
            );
        } else if let Some(index) = self.preview_group {
            let panel_windows = self.panel_windows();
            let groups = self.launcher.taskbar_applications(&panel_windows);
            if let Some(group) = groups.get(index) {
                let windows = group
                    .windows
                    .iter()
                    .map(|window| window.id)
                    .collect::<Vec<_>>();
                let (width, height) = preview_dimensions(windows.len());
                let x = self.preview_origin_x(index, width);
                let _ = send_session_command(
                    "show-preview",
                    ShellCommand::ShowPreview {
                        x,
                        width: width as i32,
                        height: height as i32,
                        windows,
                    },
                );
                if self.preview_focus_requested {
                    #[cfg(target_os = "linux")]
                    let _ = send_session_command("focus-preview", ShellCommand::FocusPreview);
                    self.preview_focus_requested = false;
                }
            }
        }
        if self.window_menu.is_some() {
            let x = self.window_menu_anchor_x.unwrap_or(self.panel_origin_x);
            let _ = send_session_command(
                "show-context-menu",
                ShellCommand::ShowContextMenu {
                    x,
                    width: MENU_WIDTH as i32,
                    height: self.window_context_menu_height(),
                },
            );
        }
    }

    fn preview_origin_x(&self, index: usize, width: u32) -> i32 {
        let icon_center = self
            .panel_host
            .semantic_targets_for_message(&PanelAction::Task(index))
            .into_iter()
            .next()
            .map(|target| target.bounds.origin.x + target.bounds.size.width / 2.0)
            .unwrap_or(PANEL_ITEM_WIDTH + index as f32 * PANEL_ITEM_WIDTH + PANEL_ITEM_WIDTH / 2.0);
        self.panel_origin_x + (icon_center - width as f32 / 2.0).round() as i32
    }

    fn window_context_menu_height(&self) -> i32 {
        let Some(window) = self.window_menu_snapshot.as_ref().or_else(|| {
            self.window_menu
                .and_then(|id| self.windows.iter().find(|candidate| candidate.id == id))
        }) else {
            return menu_height(&self.workspaces) as i32;
        };
        let outputs = self.window_feed.outputs();
        let application_id = window.application_id.as_ref();
        let application_launch_available = application_id.is_some_and(|id| {
            self.launcher
                .application(id)
                .is_some_and(|application| application.launch_command().is_some())
        });
        let pinned = application_id.is_some_and(|id| self.launcher.is_pinned(id.as_str()));
        menu_height_for_rows(window_menu_max_rows(
            window,
            &self.workspaces,
            &outputs,
            application_id,
            application_launch_available,
            pinned,
        )) as i32
    }

    fn send_window_action(&self, window: crate::model::WindowId, action: WindowAction) {
        let _ = send_session_command(
            "window-action",
            ShellCommand::WindowAction { window, action },
        );
    }

    fn open_window_preview(&mut self, index: usize) {
        if self.preview_group == Some(index) {
            self.preview_pending = None;
            return;
        }
        let panel_windows = self.panel_windows();
        let groups = self.launcher.taskbar_applications(&panel_windows);
        if groups
            .get(index)
            .is_none_or(|group| group.windows.is_empty())
        {
            return;
        }
        self.preview_pending = None;
        self.preview_group = Some(index);
        self.preview_images.clear();
        self.preview_refresh_deadline = None;
        self.preview_hovered = None;
        self.window_menu = None;
        self.window_menu_snapshot = None;
        self.window_menu_anchor_x = None;
        self.window_menu_host = None;
    }

    fn close_window_preview(&mut self) {
        self.preview_group = None;
        self.preview_pending = None;
        self.preview_focus_requested = false;
        self.preview_pointer_inside = false;
        self.preview_leave_deadline = None;
        self.preview_hovered = None;
        self.preview_images.clear();
        self.preview_refresh_deadline = None;
        self.preview_frame = None;
        self.window_menu = None;
        self.window_menu_snapshot = None;
        self.window_menu_anchor_x = None;
        self.window_menu_host = None;
        let _ = send_session_command("clear-window-highlight", ShellCommand::ClearWindowHighlight);
        let _ = send_session_command("hide-context-menu", ShellCommand::HideContextMenu);
    }

    fn dismiss_window_menu(&mut self) {
        let focused_menu = self.window_menu.is_some();
        self.close_window_preview();
        if focused_menu {
            #[cfg(target_os = "linux")]
            let _ = send_session_command(
                "restore-window-menu-focus",
                ShellCommand::RestoreApplicationFocus,
            );
        }
    }

    pub fn global_shortcut(&mut self, shortcut: platform::GlobalShortcut) -> bool {
        match shortcut {
            platform::GlobalShortcut::ReloadShellSettings => self.refresh_system(),
            platform::GlobalShortcut::ToggleLauncher => {
                self.apply_launcher_signal(!self.launcher_visible);
                true
            }
            platform::GlobalShortcut::ShowLauncher => {
                self.apply_launcher_signal(true);
                true
            }
            platform::GlobalShortcut::HideLauncher => {
                self.apply_launcher_signal(false);
                true
            }
            platform::GlobalShortcut::SwitchNext => {
                self.apply_task_switch_action(nickel_core::hotkeys::HotkeyAction::SwitchNext)
            }
            platform::GlobalShortcut::SwitchPrevious => {
                self.apply_task_switch_action(nickel_core::hotkeys::HotkeyAction::SwitchPrevious)
            }
            platform::GlobalShortcut::SwitchGroupNext => {
                self.apply_task_switch_action(nickel_core::hotkeys::HotkeyAction::SwitchGroupNext)
            }
            platform::GlobalShortcut::SwitchGroupPrevious => self
                .apply_task_switch_action(nickel_core::hotkeys::HotkeyAction::SwitchGroupPrevious),
            platform::GlobalShortcut::CommitSwitch => {
                self.apply_task_switch_action(nickel_core::hotkeys::HotkeyAction::CommitSwitch)
            }
            platform::GlobalShortcut::LockState { locked } => {
                self.locked = locked;
                let application = self.lock_host.application_mut();
                application.password.zeroize();
                application.status = None;
                if locked {
                    self.desktop_host.application_mut().context_menu = None;
                    self.launcher_visible = false;
                    self.control_visible = false;
                    self.codex_project_menu_visible = false;
                    self.close_window_preview();
                }
                true
            }
            platform::GlobalShortcut::ShowRun => {
                tracing::warn!("Nickel Run is not implemented in the shell yet");
                false
            }
            platform::GlobalShortcut::OpenFiles => self.launch_named_application("Nickel File"),
            platform::GlobalShortcut::OpenSettings => {
                self.launch_named_application("Nickel Settings")
            }
            platform::GlobalShortcut::ShowControlCenter => {
                self.set_control_visible(true);
                true
            }
            platform::GlobalShortcut::ShowNotifications => {
                let history = self.notification_feed.history();
                self.notification_host
                    .application_mut()
                    .sync_history(&history, self.palette);
                self.notification = history.first().cloned();
                self.notification_history_visible = true;
                #[cfg(target_os = "linux")]
                let _ = send_session_command(
                    "focus-notifications",
                    ShellCommand::SetShellRoleVisible {
                        role: nickel_session_protocol::ShellRole::Notification,
                        visible: true,
                    },
                );
                true
            }
            platform::GlobalShortcut::ShowDesktop => {
                send_session_command("toggle-show-desktop", ShellCommand::ToggleShowDesktop)
            }
            platform::GlobalShortcut::ProjectDisplays => {
                self.set_control_visible(true);
                true
            }
            platform::GlobalShortcut::ShowWindowMenu => self.open_active_window_menu(),
            platform::GlobalShortcut::ConsumerControl(control) => {
                platform::handle_consumer_control(control);
                true
            }
            platform::GlobalShortcut::AudioChanged {
                available,
                volume_percent,
                muted,
                output_name,
            } => {
                self.audio.available = available;
                self.audio.volume_percent = volume_percent;
                self.audio.muted = muted;
                if let Some(name) = output_name {
                    for device in &mut self.audio.devices {
                        device.is_default = device.name == name;
                    }
                    if !self.audio.devices.iter().any(|device| device.name == name) {
                        self.audio.devices.push(platform::AudioDeviceStatus {
                            id: name.clone(),
                            name,
                            is_default: true,
                        });
                    }
                }
                self.volume_osd_until =
                    available.then(|| Instant::now() + Duration::from_millis(1500));
                true
            }
            platform::GlobalShortcut::Screenshot(platform::ScreenshotAction::ActiveWindow) => {
                if let Err(error) = platform::capture_active_window() {
                    tracing::warn!(%error, "failed to copy active window screenshot");
                    self.screenshot.show_error(error);
                    self.set_screenshot_focus(true);
                    return true;
                }
                false
            }
            platform::GlobalShortcut::Screenshot(
                platform::ScreenshotAction::ActiveWindowToFile,
            ) => {
                if let Err(error) = platform::capture_active_window_to_file() {
                    tracing::warn!(%error, "failed to capture active window to a temporary file");
                    self.screenshot.show_error(error);
                    self.set_screenshot_focus(true);
                    return true;
                }
                false
            }
            platform::GlobalShortcut::Screenshot(platform::ScreenshotAction::InteractiveRegion) => {
                let was_visible = self.screenshot.visible();
                self.screenshot.request_capture();
                if was_visible {
                    self.set_screenshot_focus(false);
                }
                true
            }
            platform::GlobalShortcut::Screenshot(
                platform::ScreenshotAction::InteractiveRegionToFile,
            ) => {
                let was_visible = self.screenshot.visible();
                self.screenshot.request_capture_to_file();
                if was_visible {
                    self.set_screenshot_focus(false);
                }
                true
            }
        }
    }

    fn apply_task_switch_action(&mut self, action: nickel_core::hotkeys::HotkeyAction) -> bool {
        if self.task_switcher.session().is_none()
            && matches!(
                action,
                nickel_core::hotkeys::HotkeyAction::SwitchNext
                    | nickel_core::hotkeys::HotkeyAction::SwitchPrevious
                    | nickel_core::hotkeys::HotkeyAction::SwitchGroupNext
                    | nickel_core::hotkeys::HotkeyAction::SwitchGroupPrevious
            )
            && let FeedState::Ready(windows) = self.window_feed.snapshot(&self.launcher)
        {
            // Shortcut dispatch can run between periodic feed refreshes. Starting a switch from a
            // fresh snapshot keeps the active application and MRU order aligned with Win32 now.
            self.windows = windows;
        }
        let windows = self
            .windows
            .iter()
            .map(|window| SwitchWindow {
                id: window.id,
                application_id: window
                    .application_id
                    .as_ref()
                    .map(|id| id.as_str().to_owned())
                    .unwrap_or_else(|| window.title.clone()),
                active: window.active,
            })
            .collect::<Vec<_>>();
        let effects = self.task_switcher.apply(action, &windows);
        let changed = !effects.is_empty();
        for effect in effects {
            match effect {
                TaskSwitchEffect::ActivateWindow(window) => {
                    let _ = send_session_command(
                        "task-switcher-activate",
                        ShellCommand::WindowAction {
                            window,
                            action: WindowAction::Activate,
                        },
                    );
                }
                TaskSwitchEffect::HideFlip { .. } => {
                    self.task_switcher_group = None;
                    self.preview_frame = None;
                    self.preview_images.clear();
                }
                TaskSwitchEffect::ShowFlip { .. }
                | TaskSwitchEffect::RequestPreviews(_)
                | TaskSwitchEffect::SelectPreview(_) => {}
            }
        }
        if self.task_switcher.session().is_some() {
            self.rebuild_task_switcher_preview();
        }
        changed
    }

    fn rebuild_task_switcher_preview(&mut self) {
        let visible = self.task_switcher.visible_range(5);
        let ids = self.task_switcher.candidates()[visible].to_vec();
        let windows = ids
            .iter()
            .filter_map(|id| self.windows.iter().find(|window| window.id == *id).cloned())
            .collect::<Vec<_>>();
        self.preview_images.clear();
        for window in &windows {
            if let Some(preview) = self.window_feed.preview(window.id) {
                self.preview_images
                    .insert(window.id, Arc::new(preview.image));
            }
        }
        self.preview_hovered = self.task_switcher.selected().copied();
        self.task_switcher_group = Some(WindowGroup {
            application_id: None,
            application_name: "Open windows".into(),
            windows,
        });
        self.preview_frame = None;
    }

    /// Requests a launcher toggle initiated by shell-owned input such as a controller.
    ///
    /// Linux compositor shortcut notifications use [`Self::global_shortcut`] after the
    /// compositor has already changed visibility. Shell-originated input must instead send the
    /// visibility request to the compositor before mirroring the resulting state.
    pub fn request_launcher_toggle(&mut self) -> bool {
        let visible = !self.launcher_visible;
        let command = if visible {
            ShellCommand::ShowFromController
        } else {
            ShellCommand::Hide
        };
        if !send_session_command("controller-launcher-visibility", command) {
            self.launcher_status = Some("Nickel could not update the launcher.".to_owned());
            return false;
        }
        self.apply_session_launcher_visibility(visible);
        platform::launcher_visibility_applied(visible);
        self.launcher_visible == visible
    }

    pub fn capture_screenshot(&mut self) -> bool {
        match platform::capture_desktop() {
            Ok(capture) => {
                self.screenshot.show(capture.image);
                self.set_screenshot_focus(true);
                true
            }
            Err(error) => {
                tracing::warn!(%error, "failed to capture desktop");
                self.screenshot.show_error(error);
                self.set_screenshot_focus(true);
                true
            }
        }
    }

    pub fn screenshot_pointer_moved(&mut self, x: f32, y: f32, width: u32, height: u32) -> bool {
        self.screenshot.queue_pointer_moved(x, y, width, height);
        false
    }

    pub fn screenshot_pointer_pressed(&mut self, x: f32, y: f32, width: u32, height: u32) -> bool {
        let was_visible = self.screenshot.visible();
        let handled = self.screenshot.pointer_pressed(x, y, width, height);
        if was_visible && !self.screenshot.visible() {
            self.set_screenshot_focus(false);
        }
        handled
    }

    pub fn screenshot_pointer_released(&mut self) -> bool {
        self.screenshot.pointer_released()
    }

    pub fn screenshot_key(&mut self, key: Option<KeyCode>) -> bool {
        if key == Some(KeyCode::Escape) && self.screenshot.visible() {
            let _ = self.screenshot.escape();
            self.set_screenshot_focus(false);
            true
        } else {
            false
        }
    }

    pub fn screenshot_controller(&mut self, action: ControllerAction) -> bool {
        if !self.screenshot.visible() {
            return false;
        }
        let changed = self.screenshot.controller_action(action);
        if !self.screenshot.visible() {
            self.set_screenshot_focus(false);
        }
        changed
    }

    fn set_screenshot_focus(&self, visible: bool) {
        #[cfg(target_os = "linux")]
        let _ = send_session_command(
            "screenshot-focus",
            if visible {
                ShellCommand::FocusScreenshot
            } else {
                ShellCommand::RestoreApplicationFocus
            },
        );
        #[cfg(not(target_os = "linux"))]
        let _ = visible;
    }

    fn set_launcher_visible(&mut self, visible: bool) {
        if !send_session_command(
            "launcher-visibility",
            if visible {
                ShellCommand::Show
            } else {
                ShellCommand::Hide
            },
        ) {
            self.launcher_status = Some("Nickel could not update the launcher.".to_owned());
            return;
        }
        self.apply_session_launcher_visibility(visible);
        platform::launcher_visibility_applied(visible);
    }

    fn set_control_visible(&mut self, visible: bool) {
        #[cfg(target_os = "linux")]
        if !send_session_command(
            "control-center-focus",
            if visible {
                ShellCommand::FocusControlCenter
            } else {
                ShellCommand::RestoreApplicationFocus
            },
        ) {
            self.launcher_status = Some("Nickel could not update Quick Settings.".to_owned());
            return;
        }
        self.control_visible = visible;
    }

    fn apply_launcher_signal(&mut self, visible: bool) {
        #[cfg(target_os = "linux")]
        self.apply_session_launcher_visibility(visible);
        #[cfg(not(target_os = "linux"))]
        self.set_launcher_visible(visible);
    }

    fn apply_session_launcher_visibility(&mut self, visible: bool) {
        self.launcher_visible = visible;
        if visible {
            self.control_visible = false;
            self.focus_launcher_search();
        } else {
            self.launcher.clear();
        }
    }

    pub fn focus_launcher_search(&mut self) -> bool {
        let status = self.launcher_status_text();
        self.launcher_host
            .application_mut()
            .sync(&self.launcher, self.palette, status);
        self.launcher_host.step(HostBatch::default());
        let Ok(search) = self
            .launcher_host
            .query_unique(&nickel_ui::SemanticSelector::Role(SemanticRole::TextField))
        else {
            return false;
        };
        let outcome = self.launcher_host.request_focus(search.id);
        outcome.changed && outcome.failures.is_empty()
    }

    pub fn control_click(&mut self, x: f32, y: f32, width: u32, height: u32) -> bool {
        self.sync_control_host(width, height);
        let point = Point { x, y };
        self.step_control_host(HostBatch {
            events: vec![
                HostEvent::Ui(UiEvent::PointerPressed(point)),
                HostEvent::Ui(UiEvent::PointerReleased(point)),
            ],
            ..HostBatch::default()
        });
        self.apply_control_effects();
        true
    }

    pub fn control_key(&mut self, key: Option<KeyCode>, width: u32, height: u32) -> bool {
        if !self.control_visible {
            return false;
        }
        self.sync_control_host(width, height);
        let action = match key {
            Some(KeyCode::Escape) => {
                self.set_control_visible(false);
                return true;
            }
            Some(KeyCode::ArrowDown | KeyCode::Tab) => ControllerAction::Down,
            Some(KeyCode::ArrowRight) => ControllerAction::Right,
            Some(KeyCode::ArrowUp) => ControllerAction::Up,
            Some(KeyCode::ArrowLeft) => ControllerAction::Left,
            Some(KeyCode::Enter | KeyCode::NumpadEnter) => ControllerAction::Confirm,
            _ => return false,
        };
        self.step_control_host(HostBatch {
            events: vec![HostEvent::Controller(action)],
            ..HostBatch::default()
        });
        self.apply_control_effects();
        true
    }

    pub fn control_controller(
        &mut self,
        action: ControllerAction,
        width: u32,
        height: u32,
    ) -> bool {
        if !self.control_visible {
            return false;
        }
        self.sync_control_host(width, height);
        let changed = self.step_control_host(HostBatch {
            events: vec![HostEvent::Controller(action)],
            ..HostBatch::default()
        });
        self.apply_control_effects();
        let dismissed = action == ControllerAction::Cancel && self.control_visible;
        if dismissed {
            self.set_control_visible(false);
        }
        changed || dismissed
    }

    #[allow(dead_code)]
    pub fn hide_overlay(&mut self, role: SurfaceRole) -> bool {
        match role {
            SurfaceRole::Launcher if self.launcher_visible => {
                self.apply_session_launcher_visibility(false);
                let _ = send_session_command("hide-launcher", ShellCommand::Hide);
                true
            }
            SurfaceRole::ControlCenter if self.control_visible => {
                self.set_control_visible(false);
                true
            }
            SurfaceRole::CodexProjectMenu if self.codex_project_menu_visible => {
                self.codex_project_menu_visible = false;
                true
            }
            SurfaceRole::Screenshot if self.screenshot.visible() => {
                self.screenshot.hide();
                self.set_screenshot_focus(false);
                true
            }
            _ => false,
        }
    }

    pub fn scroll(&mut self, delta: f32) -> bool {
        if self.control_visible {
            self.sync_control_host(800, 600);
            self.step_control_host(HostBatch {
                events: vec![HostEvent::Ui(UiEvent::Scroll {
                    point: Point { x: 400.0, y: 300.0 },
                    delta_y: delta * 36.0,
                })],
                ..HostBatch::default()
            });
            self.apply_control_effects();
            return true;
        }
        false
    }

    fn launch_result(&mut self, index: usize) {
        let Some(application) = self.launcher.result_at(index).cloned() else {
            return;
        };
        self.launch_application(application);
    }

    fn launch_application_by_id(&mut self, id: &str) {
        let Some(application) = self
            .launcher
            .applications()
            .find(|application| application.id() == id)
            .cloned()
        else {
            return;
        };
        self.launch_application(application);
    }

    fn launch_named_application(&mut self, name: &str) -> bool {
        let Some(application) = self
            .launcher
            .applications()
            .find(|application| application.name() == name)
            .cloned()
        else {
            tracing::warn!(application = name, "shortcut application is unavailable");
            self.shortcut_action_status = Some(format!("{name} is unavailable."));
            self.set_launcher_visible(true);
            return false;
        };
        self.shortcut_action_status = None;
        self.launch_application(application);
        true
    }

    fn open_active_window_menu(&mut self) -> bool {
        let Some(snapshot) = self.windows.iter().find(|window| window.active).cloned() else {
            return false;
        };
        self.window_menu = Some(snapshot.id);
        self.window_menu_snapshot = Some(snapshot);
        self.window_menu_host = None;
        self.window_menu_anchor_x = Some(self.panel_origin_x);
        let sent = send_session_command(
            "show-context-menu",
            ShellCommand::ShowContextMenu {
                x: self.panel_origin_x,
                width: MENU_WIDTH as i32,
                height: self.window_context_menu_height(),
            },
        );
        #[cfg(target_os = "linux")]
        let _ = send_session_command("focus-context-menu", ShellCommand::FocusContextMenu);
        sent
    }

    fn launch_application(&mut self, application: Application) {
        #[cfg(target_os = "linux")]
        if platform::application_requires_secure_storage(&application)
            && platform::secure_storage_state().unwrap_or_else(|error| {
                tracing::warn!(%error, "secure-storage query failed before application launch");
                platform::SecureStorageState::ControlUnavailable
            }) != platform::SecureStorageState::Ready
            && self.secure_storage_override.as_deref() != Some(application.id())
        {
            if let Err(error) = platform::request_secure_storage_retry() {
                tracing::warn!(%error, "secure-storage retry command failed");
            }
            self.secure_storage_override = Some(application.id().to_owned());
            self.launcher_status = Some(format!(
                "Secure storage is not ready. Activate {} again to launch without credentials.",
                application.name()
            ));
            return;
        }
        self.secure_storage_override = None;
        self.launcher_status = None;
        #[cfg(target_os = "linux")]
        let result = if application.name() == "Nickel Settings" {
            platform::launch_session_application(&application)
        } else {
            platform::launch_application(&application)
        };
        #[cfg(not(target_os = "linux"))]
        let result = platform::launch_application(&application);
        match result {
            Ok(_) => {
                self.launcher.record_launch(application.id());
                self.persist_launcher_preferences();
                self.set_launcher_visible(false);
            }
            Err(error) => {
                tracing::warn!(
                    application = application.name(),
                    ?error,
                    "failed to launch application from launcher"
                );
                self.launcher_status = Some(format!(
                    "Could not launch {}: {}",
                    application.name(),
                    launch_error_summary(&error)
                ));
            }
        }
    }

    fn desktop_scene(&mut self, width: u32, height: u32) -> Vec<PaintCommand> {
        self.load_wallpaper_for(width, height);
        let application = self.desktop_host.application_mut();
        let wallpaper_changed = match (&application.wallpaper, &self.wallpaper) {
            (Some(current), Some(next)) => !Arc::ptr_eq(current, next),
            (None, None) => false,
            _ => true,
        };
        let palette_changed = application.palette != self.palette;
        if palette_changed {
            application.icon_cache.clear();
        }
        application.wallpaper.clone_from(&self.wallpaper);
        application.palette = self.palette;
        let icons_changed = application.prepare_icons();
        let outcome = self.desktop_host.step(HostBatch {
            application_changed: wallpaper_changed || palette_changed || icons_changed,
            surface_size: Some((width, height)),
            ..HostBatch::default()
        });
        self.desktop_change_token = outcome.change_token;
        self.desktop_deadline = outcome.next_deadline;
        self.desktop_host.commands().to_vec()
    }

    fn volume_osd_scene(&mut self, width: u32, height: u32) -> Vec<PaintCommand> {
        let percent = self.audio.volume_percent.min(100);
        let mut label = if self.audio.muted {
            "Muted".to_owned()
        } else {
            format!("Volume {percent}%")
        };
        let output = if self.locked {
            Some("Audio output")
        } else {
            self.audio
                .devices
                .iter()
                .find(|device| device.is_default)
                .map(|device| device.name.as_str())
        };
        if let Some(output) = output {
            label.push_str(" · ");
            label.push_str(output);
        }
        let application = self.volume_osd_host.application_mut();
        let changed = application.label != label
            || application.percent != percent
            || application.palette != self.palette;
        application.label = label;
        application.percent = percent;
        application.palette = self.palette;
        self.volume_osd_host.step(HostBatch {
            application_changed: changed,
            surface_size: Some((width, height)),
            ..HostBatch::default()
        });
        self.volume_osd_host.commands().to_vec()
    }

    fn lock_scene(&mut self, width: u32, height: u32) -> Vec<PaintCommand> {
        let outcome = self.lock_host.step(HostBatch {
            surface_size: Some((width, height)),
            ..HostBatch::default()
        });
        self.lock_change_token = outcome.change_token;
        self.lock_deadline = outcome.next_deadline;
        self.lock_host.commands().to_vec()
    }

    pub fn lock_host_input(
        &mut self,
        input: nickel_input::InputEvent,
        width: u32,
        height: u32,
    ) -> bool {
        if !self.locked {
            return false;
        }
        self.lock_host.step(HostBatch {
            surface_size: Some((width, height)),
            ..HostBatch::default()
        });
        if !self.lock_host.input_context().text_focused
            && let Ok(target) = self
                .lock_host
                .query_unique(&nickel_ui::SemanticSelector::Role(SemanticRole::TextField))
        {
            let point = Point {
                x: target.bounds.origin.x + target.bounds.size.width / 2.0,
                y: target.bounds.origin.y + target.bounds.size.height / 2.0,
            };
            self.lock_host.step(HostBatch {
                events: vec![
                    HostEvent::Ui(UiEvent::PointerPressed(point)),
                    HostEvent::Ui(UiEvent::PointerReleased(point)),
                ],
                ..HostBatch::default()
            });
        }
        let outcome = self.lock_host.step(HostBatch {
            events: vec![HostEvent::Normalized {
                input,
                clipboard_text: None,
            }],
            ..HostBatch::default()
        });
        let changed = outcome.changed;
        changed | self.apply_lock_effects()
    }

    pub fn lock_host_controller(&mut self, action: ControllerAction) -> bool {
        if !self.locked {
            return false;
        }
        let event = if action == ControllerAction::Confirm {
            HostEvent::Shortcut(Shortcut::Submit)
        } else {
            HostEvent::Controller(action)
        };
        let outcome = self.lock_host.step(HostBatch {
            events: vec![event],
            ..HostBatch::default()
        });
        outcome.changed | self.apply_lock_effects()
    }

    fn apply_lock_effects(&mut self) -> bool {
        let effects = std::mem::take(&mut self.lock_host.application_mut().effects);
        let changed = !effects.is_empty();
        for effect in effects {
            match effect {
                LockEffect::Authenticate(password) => {
                    let username = std::env::var("USER").unwrap_or_default();
                    let authenticated: Result<bool, String> = {
                        #[cfg(target_os = "linux")]
                        {
                            crate::lock_auth::authenticate(&username, &password)
                        }
                        #[cfg(not(target_os = "linux"))]
                        {
                            let _ = (&username, &password);
                            Err("system authentication is unavailable on this platform".into())
                        }
                    };
                    let application = self.lock_host.application_mut();
                    match authenticated {
                        Ok(true) => {
                            #[cfg(target_os = "linux")]
                            if let Err(error) = platform::send_shell_command(ShellCommand::Unlock) {
                                tracing::warn!(%error, "session unlock command failed");
                                application.status = Some("Could not contact the session".into());
                            }
                        }
                        Ok(false) => application.status = Some("Authentication failed".into()),
                        Err(error) => {
                            tracing::error!(%error, "lock authentication failed");
                            application.status = Some("Authentication service unavailable".into());
                        }
                    }
                }
            }
        }
        changed
    }

    fn load_wallpaper_for(&mut self, width: u32, height: u32) {
        let requested = (width.max(1), height.max(1));
        let Some(target) = wallpaper_cache_target(self.wallpaper_size, requested) else {
            return;
        };
        let Some(path) = self.wallpaper_path.as_deref() else {
            return;
        };
        let Ok(image) = image::open(path) else {
            return;
        };
        self.wallpaper = Some(Arc::new(image.thumbnail(target.0, target.1).into_rgba8()));
        self.wallpaper_size = target;
    }

    fn sync_notification_host(&mut self, width: u32, height: u32) {
        if self.notification_history_visible {
            let history = self.notification_feed.history();
            self.notification_host
                .application_mut()
                .sync_history(&history, self.palette);
        } else {
            self.notification_host
                .application_mut()
                .sync(self.notification.as_ref(), self.palette);
        }
        self.notification_host.step(HostBatch {
            surface_size: Some((width, height)),
            events: vec![HostEvent::Poll],
            ..HostBatch::default()
        });
    }

    fn apply_notification_effects(&mut self) -> bool {
        for failure in self.notification_host.application_mut().take_failures() {
            tracing::warn!(?failure, "notification application rejected an action");
        }
        let effects = self.notification_host.application_mut().take_effects();
        let handled = !effects.is_empty();
        for effect in effects {
            match effect {
                NotificationEffect::Invoke {
                    notification_id,
                    key,
                } => {
                    self.notification_feed.invoke(notification_id, &key);
                    self.dismiss_notification_transport(notification_id);
                }
                NotificationEffect::Dismiss { notification_id } => {
                    self.dismiss_notification_transport(notification_id);
                }
                NotificationEffect::CloseHistory => {
                    self.notification_history_visible = false;
                    self.notification = None;
                    #[cfg(target_os = "linux")]
                    let _ = send_session_command(
                        "hide-notification-history",
                        ShellCommand::SetShellRoleVisible {
                            role: nickel_session_protocol::ShellRole::Notification,
                            visible: false,
                        },
                    );
                    #[cfg(target_os = "linux")]
                    let _ = send_session_command(
                        "restore-notification-focus",
                        ShellCommand::RestoreApplicationFocus,
                    );
                }
            }
        }
        handled
    }

    fn dismiss_notification_transport(&mut self, notification_id: u32) {
        if self.notification.as_ref().map(|item| item.id) != Some(notification_id) {
            return;
        }
        self.notification = None;
        self.notification_feed.dismiss(notification_id);
        self.notification_host
            .application_mut()
            .sync(None, self.palette);
        self.notification_host.step(HostBatch {
            events: vec![HostEvent::Poll],
            ..HostBatch::default()
        });
    }

    fn window_preview_scene(&mut self) -> Vec<PaintCommand> {
        let group = self.task_switcher_group.clone().or_else(|| {
            self.preview_group.and_then(|index| {
                let panel_windows = self.panel_windows();
                self.launcher
                    .taskbar_applications(&panel_windows)
                    .get(index)
                    .map(|task| task.window_group())
            })
        });
        let Some(group) = group else {
            self.preview_frame = None;
            return Vec::new();
        };
        let theme = self.semantic_theme();
        if let Some(frame) = self.preview_frame.as_mut() {
            frame.sync(&group, &self.preview_images, self.preview_hovered, theme);
        } else {
            self.preview_frame = Some(build_preview_frame(
                &group,
                &self.preview_images,
                self.preview_hovered,
                theme,
            ));
        }
        self.preview_frame.as_ref().map_or_else(Vec::new, |frame| {
            let _change_token = frame.change_token();
            frame.commands().to_vec()
        })
    }

    fn window_menu_scene(&mut self) -> Vec<PaintCommand> {
        if self.window_menu.is_none() && self.window_menu_snapshot.is_none() {
            self.window_menu_host = None;
            return Vec::new();
        }
        let Some(snapshot) = self.window_menu_snapshot.clone().or_else(|| {
            self.window_menu.and_then(|window| {
                self.windows
                    .iter()
                    .find(|candidate| candidate.id == window)
                    .cloned()
            })
        }) else {
            self.close_window_preview();
            return Vec::new();
        };
        self.window_menu_snapshot
            .get_or_insert_with(|| snapshot.clone());
        let outputs = self.window_feed.outputs();
        let application_id = snapshot.application_id.clone();
        let application_launch_available = application_id.as_ref().is_some_and(|id| {
            self.launcher
                .application(id)
                .is_some_and(|application| application.launch_command().is_some())
        });
        let pinned = application_id
            .as_ref()
            .is_some_and(|id| self.launcher.is_pinned(id.as_str()));
        let height = menu_height_for_rows(window_menu_max_rows(
            &snapshot,
            &self.workspaces,
            &outputs,
            application_id.as_ref(),
            application_launch_available,
            pinned,
        ))
        .ceil()
        .max(1.0) as u32;
        let host = self.window_menu_host.get_or_insert_with(|| {
            nickel_ui::UiHost::new(
                WindowMenuApp::new(
                    snapshot.clone(),
                    self.workspaces.clone(),
                    outputs.clone(),
                    application_id.clone(),
                    application_launch_available,
                    pinned,
                    self.palette,
                ),
                MENU_WIDTH.ceil() as u32,
                height,
            )
        });
        host.application_mut()
            .sync(&snapshot, &self.workspaces, &outputs, self.palette);
        host.step(HostBatch {
            surface_size: Some((MENU_WIDTH.ceil() as u32, height)),
            events: vec![HostEvent::Poll],
            ..HostBatch::default()
        });
        host.commands().to_vec()
    }

    fn launcher_scene(&mut self, width: u32, height: u32) -> Vec<PaintCommand> {
        let status = self.launcher_status_text();
        self.launcher_host
            .application_mut()
            .sync(&self.launcher, self.palette, status);
        self.launcher_host.step(HostBatch {
            surface_size: Some((width, height)),
            events: vec![HostEvent::Poll],
            ..HostBatch::default()
        });
        for action in self.launcher_host.application_mut().take_effects() {
            self.apply_launcher_action(action);
        }
        self.launcher_host.commands().to_vec()
    }

    fn launcher_status_text(&self) -> Option<String> {
        self.launcher_status
            .as_deref()
            .or(self.shortcut_action_status.as_deref())
            .or(self.shortcut_capability_status.as_deref())
            .or_else(|| secure_storage_status_label(self.secure_storage_state))
            .or_else(|| {
                session_feed_status_label(self.window_feed_status, self.workspace_feed_status)
            })
            .map(str::to_owned)
    }

    pub fn set_global_shortcut_capability(
        &mut self,
        capability: &nickel_input::global::ShortcutCapability,
    ) {
        self.shortcut_capability_status = shortcut_capability_status(capability);
    }

    fn panel_scene(&mut self, width: u32, height: u32) -> Vec<PaintCommand> {
        let application_changed = self.sync_panel_host();
        let outcome = self.panel_host.step(HostBatch {
            application_changed,
            surface_size: Some((width, height)),
            ..HostBatch::default()
        });
        self.panel_change_token = outcome.change_token;
        self.panel_deadline = outcome.next_deadline;
        self.apply_panel_effects();
        self.panel_host.commands().to_vec()
    }

    fn sync_panel_host(&mut self) -> bool {
        let panel_windows = self.panel_windows();
        let groups = self.launcher.taskbar_applications(&panel_windows);
        let task_icons: Vec<Option<(u16, Arc<image::RgbaImage>)>> = groups
            .iter()
            .take(12)
            .map(|group| {
                group
                    .application_id
                    .as_ref()
                    .and_then(|id| self.launcher.application(id))
                    .and_then(|application| self.launcher_icons.resolve(application))
                    .or_else(|| {
                        group
                            .application_id
                            .as_ref()
                            .is_some_and(|id| id.as_str().starts_with("io.nickel.codex.project."))
                            .then(|| (0x3002, Arc::clone(&self.codex_icon)))
                    })
                    .or_else(|| {
                        crate::icons::nickel_application(&group.application_name)
                            .map(|(id, image)| (id, Arc::new(image)))
                    })
                    .or_else(|| {
                        group.windows.first().and_then(|window| {
                            self.window_icons.get(&window.id).cloned().map(|icon| {
                                self.launcher_icons.resolve_window_icon(window.id, icon)
                            })
                        })
                    })
            })
            .collect();
        let visible_panel_hover = self.visible_panel_hover();
        let application = self.panel_host.application_mut();
        let task_icons_changed = application.task_icons.len() != task_icons.len()
            || application
                .task_icons
                .iter()
                .zip(&task_icons)
                .any(|(current, next)| match (current, next) {
                    (Some((current_id, current)), Some((next_id, next))) => {
                        current_id != next_id || !Arc::ptr_eq(current, next)
                    }
                    (None, None) => false,
                    _ => true,
                });
        let application_changed = application.palette != self.palette
            || application.windows != panel_windows
            || application.tray != self.tray
            || application.panel_hover != visible_panel_hover
            || application.launcher_visible != self.launcher_visible
            || application.codex_project_menu_visible != self.codex_project_menu_visible
            || application.control_visible != self.control_visible
            || application.launcher.codex_available() != self.launcher.codex_available()
            || application.launcher.preferences() != self.launcher.preferences()
            || task_icons_changed;
        application.launcher.clone_from(&self.launcher);
        application.windows = panel_windows;
        application.tray.clone_from(&self.tray);
        application.tray_icons.clone_from(&self.tray_icons);
        application.panel_icon = Arc::clone(&self.panel_icon);
        application.codex_icon = Arc::clone(&self.codex_icon);
        application.task_icons = task_icons;
        application.palette = self.palette;
        application.panel_hover = visible_panel_hover;
        application.launcher_visible = self.launcher_visible;
        application.codex_project_menu_visible = self.codex_project_menu_visible;
        application.control_visible = self.control_visible;
        application_changed
    }

    fn apply_panel_effects(&mut self) -> bool {
        let effects = std::mem::take(&mut self.panel_host.application_mut().effects);
        let changed = !effects.is_empty();
        for action in effects {
            self.apply_panel_action(action);
        }
        changed
    }
}

fn window_belongs_to_panel(
    all_windows: bool,
    panel_output: Option<&str>,
    window_output: Option<&str>,
) -> bool {
    all_windows
        || panel_output.is_none()
        || window_output.is_none()
        || panel_output == window_output
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
        let groups = self.launcher.taskbar_applications(&self.windows);
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
        if self.launcher.codex_available() {
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

impl LiveShell {
    fn sync_control_host(&mut self, width: u32, height: u32) {
        self.control_host.application_mut().sync(
            &self.network,
            &self.bluetooth,
            &self.audio,
            &self.workspaces,
        );
        self.step_control_host(HostBatch {
            surface_size: Some((width, height)),
            events: vec![HostEvent::Poll],
            ..HostBatch::default()
        });
    }

    fn step_control_host(&mut self, batch: HostBatch) -> bool {
        let outcome = self.control_host.step(batch);
        self.host_runtime_samples.record(outcome.telemetry);
        let changed = outcome.change_token != self.control_change_token;
        self.control_change_token = outcome.change_token;
        self.control_deadline = outcome.next_deadline;
        changed
    }

    fn apply_control_effects(&mut self) {
        let effects = self.control_host.application_mut().take_effects();
        for action in effects {
            self.apply_control_action(action);
        }
    }

    fn apply_launcher_action(&mut self, action: LauncherAction) {
        let Some(effect) =
            reduce_launcher_action(&mut self.launcher, &mut self.launcher_view, action)
        else {
            return;
        };
        self.apply_launcher_effect(effect);
    }

    fn apply_launcher_effect(&mut self, effect: LauncherShellEffect) {
        match effect {
            LauncherShellEffect::ActivateResult(index) => self.launch_result(index),
            LauncherShellEffect::TogglePin(id) => {
                self.launcher.toggle_pin(&id);
                self.persist_launcher_preferences();
            }
            LauncherShellEffect::RetryPreferencePersistence => {
                self.persist_launcher_preferences();
            }
            LauncherShellEffect::LaunchApplication(id) => self.launch_application_by_id(&id),
            LauncherShellEffect::OpenProject(id) => {
                self.set_launcher_visible(false);
                self.requested_codex_project = Some(id);
            }
            LauncherShellEffect::SeeAllProjects => {
                self.set_launcher_visible(false);
                self.codex_project_menu_visible = true;
            }
            LauncherShellEffect::OpenSettings(destination) => {
                let preferred = match destination {
                    crate::launcher::SettingsDestination::Nickel
                    | crate::launcher::SettingsDestination::KeyboardShortcuts
                    | crate::launcher::SettingsDestination::About => "Nickel Settings",
                };
                let id = self
                    .launcher
                    .applications()
                    .find(|application| application.name() == preferred)
                    .map(|application| application.id().to_owned());
                if let Some(id) = id {
                    let application = self
                        .launcher
                        .applications()
                        .find(|application| application.id() == id)
                        .cloned();
                    if let Some(application) = application {
                        let screen = match destination {
                            crate::launcher::SettingsDestination::Nickel => Some("appearance"),
                            crate::launcher::SettingsDestination::KeyboardShortcuts => {
                                Some("keyboard-shortcuts")
                            }
                            crate::launcher::SettingsDestination::About => Some("about"),
                        };
                        let application = match (screen, application.launch_command()) {
                            (Some(screen), Some(command)) => {
                                let mut command = command.to_vec();
                                command.extend(["--screen".into(), screen.into()]);
                                Application::new(
                                    application.id().to_owned(),
                                    application.name().to_owned(),
                                    application.icon().map(str::to_owned),
                                    application.icon_path().map(std::path::Path::to_owned),
                                    Some(command),
                                )
                            }
                            _ => application,
                        };
                        self.launch_application(application);
                    }
                }
            }
            LauncherShellEffect::OpenAccount => {
                self.set_control_visible(true);
                if self.control_visible {
                    self.set_launcher_visible(false);
                }
            }
            LauncherShellEffect::RequestLogout => {
                self.set_control_visible(true);
                if self.control_visible {
                    self.set_launcher_visible(false);
                    self.control_host
                        .application_mut()
                        .request_session_action(platform::SessionAction::LogOut);
                    self.step_control_host(HostBatch {
                        events: vec![HostEvent::Poll],
                        ..HostBatch::default()
                    });
                }
            }
            LauncherShellEffect::Dismiss => self.set_launcher_visible(false),
        }
    }

    fn persist_launcher_preferences(&mut self) {
        #[cfg(test)]
        {
            self.launcher_persistence_attempts += 1;
        }
        #[cfg(test)]
        let result = self.launcher_preferences_path.as_ref().map_or_else(
            || self.launcher.preferences().save_default(),
            |path| self.launcher.preferences().save(path),
        );
        #[cfg(not(test))]
        let result = self.launcher.preferences().save_default();
        match result {
            Err(error) => {
                tracing::warn!(%error, "launcher preferences could not be saved");
                self.launcher_status = Some(format!(
                    "Launcher preferences could not be saved: {}",
                    error
                ));
            }
            Ok(())
                if self.launcher_status.as_deref().is_some_and(|status| {
                    status.starts_with("Launcher preferences could not be saved:")
                }) =>
            {
                self.launcher_status = None;
            }
            Ok(()) => {}
        }
    }

    fn apply_control_action(&mut self, action: ControlAction) {
        match action {
            ControlAction::ToggleWifiSection => {}
            ControlAction::SetWifiEnabled(enabled) => {
                log_control_result("set-wifi-enabled", platform::set_wifi_enabled(enabled));
            }
            ControlAction::ActivateWifi { id } => {
                log_control_result(
                    "activate-wifi-network",
                    platform::activate_wifi_network(&id),
                );
            }
            ControlAction::ToggleBluetoothSection => {}
            ControlAction::SetBluetoothPowered(powered) => {
                log_control_result(
                    "set-bluetooth-powered",
                    platform::set_bluetooth_powered(powered),
                );
            }
            ControlAction::SetBluetoothDiscovery(discovering) => {
                log_control_result(
                    "set-bluetooth-discovery",
                    platform::set_bluetooth_discovery(discovering),
                );
            }
            ControlAction::ToggleBluetoothDevice { id } => {
                log_control_result(
                    "toggle-bluetooth-device",
                    platform::toggle_bluetooth_device(&id),
                );
            }
            ControlAction::ToggleAudioSection => {}
            ControlAction::SetAudioVolume(volume) => {
                log_control_result("set-audio-volume", platform::set_audio_volume(volume));
            }
            ControlAction::SelectAudioDevice { id } => {
                log_control_result("select-audio-device", platform::select_audio_device(&id));
            }
            ControlAction::SwitchWorkspace(workspace) => {
                let _ = send_session_command(
                    "switch-workspace",
                    ShellCommand::SwitchWorkspace(workspace),
                );
            }
            ControlAction::CreateWorkspace => {
                let _ = send_session_command("create-workspace", ShellCommand::CreateWorkspace);
            }
            ControlAction::ToggleShowDesktop => {
                let _ =
                    send_session_command("toggle-show-desktop", ShellCommand::ToggleShowDesktop);
            }
            ControlAction::ShowNotifications => {
                self.global_shortcut(platform::GlobalShortcut::ShowNotifications);
            }
            ControlAction::RemoveWorkspace(workspace) => {
                let _ = send_session_command(
                    "remove-workspace",
                    ShellCommand::RemoveWorkspace(workspace),
                );
            }
            ControlAction::PreviewProjection(mode) => self.preview_projection(mode),
            ControlAction::ConfirmProjection => {
                self.projection_chooser.confirm();
                self.projection_rollback_deadline = None;
            }
            ControlAction::CancelProjection => self.rollback_projection(),
            ControlAction::RequestSessionAction(_)
            | ControlAction::CancelSessionAction
            | ControlAction::ConfirmSessionAction => {}
            ControlAction::SessionAction(action) => {
                let _ = send_session_command("session-action", ShellCommand::SessionAction(action));
            }
        }
        let _ = self.refresh();
    }

    fn preview_projection(&mut self, mode: nickel_core::display_projection::ProjectionMode) {
        #[cfg(target_os = "linux")]
        {
            use nickel_core::display_projection::{
                ProjectionChooser, ProjectionOutput, ProjectionPlacement,
            };
            let Ok(outputs) = platform::projection_outputs() else {
                return;
            };
            let topology = outputs
                .iter()
                .map(|output| ProjectionOutput {
                    name: output.name.clone(),
                    internal: output.name.starts_with("eDP") || output.name.starts_with("LVDS"),
                    width: output.geometry.width,
                    height: output.geometry.height,
                    scale: nickel_core::dpi::Scale120::new(output.scale_120).unwrap_or_default(),
                })
                .collect::<Vec<_>>();
            let Some(plan) = ProjectionChooser::plan(mode, &topology) else {
                return;
            };
            let previous = outputs
                .iter()
                .map(|output| ProjectionPlacement {
                    name: output.name.clone(),
                    x: output.geometry.x,
                    y: output.geometry.y,
                    enabled: output.enabled,
                    scale: nickel_core::dpi::Scale120::new(output.scale_120).unwrap_or_default(),
                })
                .collect();
            let primary = outputs
                .iter()
                .find(|output| {
                    output.primary
                        && plan
                            .placements
                            .iter()
                            .any(|entry| entry.name == output.name && entry.enabled)
                })
                .or_else(|| {
                    outputs.iter().find(|output| {
                        plan.placements
                            .iter()
                            .any(|entry| entry.name == output.name && entry.enabled)
                    })
                })
                .map(|output| output.name.clone())
                .unwrap_or_default();
            let layout = nickel_session_protocol::OutputLayout {
                primary,
                placements: plan
                    .placements
                    .iter()
                    .map(|entry| nickel_session_protocol::OutputPlacement {
                        name: entry.name.clone(),
                        x: entry.x,
                        y: entry.y,
                        enabled: entry.enabled,
                        scale_120: entry.scale.units(),
                    })
                    .collect(),
            };
            if send_session_command("preview-projection", ShellCommand::ApplyOutputs(layout)) {
                self.projection_chooser.preview(previous, plan);
                self.projection_rollback_deadline = Some(Instant::now() + Duration::from_secs(15));
            }
        }
        #[cfg(not(target_os = "linux"))]
        let _ = mode;
    }

    fn rollback_projection(&mut self) {
        self.projection_rollback_deadline = None;
        #[cfg(target_os = "linux")]
        if let Some(mut previous) = self.projection_chooser.rollback() {
            if let Ok(outputs) = platform::projection_outputs() {
                previous.retain(|entry| outputs.iter().any(|output| output.name == entry.name));
                if !previous.iter().any(|entry| entry.enabled)
                    && let Some(first) = previous.first_mut()
                {
                    first.enabled = true;
                }
            }
            if previous.is_empty() {
                return;
            }
            let primary = previous
                .iter()
                .find(|entry| entry.enabled)
                .map(|entry| entry.name.clone())
                .unwrap_or_default();
            let layout = nickel_session_protocol::OutputLayout {
                primary,
                placements: previous
                    .into_iter()
                    .map(|entry| nickel_session_protocol::OutputPlacement {
                        name: entry.name,
                        x: entry.x,
                        y: entry.y,
                        enabled: entry.enabled,
                        scale_120: entry.scale.units(),
                    })
                    .collect(),
            };
            let _ = send_session_command("rollback-projection", ShellCommand::ApplyOutputs(layout));
        }
    }
}

fn log_control_result(operation: &'static str, succeeded: bool) {
    if !succeeded {
        tracing::warn!(operation, "control action failed");
    }
}

fn secure_storage_status_label(state: platform::SecureStorageState) -> Option<&'static str> {
    match state {
        platform::SecureStorageState::Starting => Some("Secure storage is starting…"),
        platform::SecureStorageState::Locked => Some("Secure storage is locked."),
        platform::SecureStorageState::PromptRequired => {
            Some("Secure storage is waiting for its unlock prompt.")
        }
        platform::SecureStorageState::Unavailable => Some("Secure storage is unavailable."),
        platform::SecureStorageState::UnavailableReason(reason) => Some(match reason {
            nickel_session_protocol::SecureStorageUnavailableReason::Connection => {
                "Secure storage cannot connect to the session bus."
            }
            nickel_session_protocol::SecureStorageUnavailableReason::MissingDefaultCollection => {
                "Secure storage has no default collection."
            }
            nickel_session_protocol::SecureStorageUnavailableReason::PromptTimedOut => {
                "The secure-storage unlock prompt timed out."
            }
            nickel_session_protocol::SecureStorageUnavailableReason::ProviderDisappeared => {
                "The secure-storage provider disappeared."
            }
            nickel_session_protocol::SecureStorageUnavailableReason::ProviderConfiguration
            | nickel_session_protocol::SecureStorageUnavailableReason::UnexpectedProvider => {
                "The secure-storage provider configuration is invalid."
            }
            nickel_session_protocol::SecureStorageUnavailableReason::Protocol
            | nickel_session_protocol::SecureStorageUnavailableReason::ReadinessCheck => {
                "Secure storage failed its readiness check."
            }
        }),
        platform::SecureStorageState::ControlUnavailable => {
            Some("Nickel cannot reach the session service.")
        }
        platform::SecureStorageState::Ready => None,
    }
}

fn send_session_command(operation: &'static str, command: ShellCommand) -> bool {
    #[cfg(test)]
    {
        let _ = (operation, command);
        true
    }
    #[cfg(all(target_os = "linux", not(test)))]
    {
        match platform::send_shell_command(command) {
            Ok(()) => true,
            Err(error) => {
                tracing::warn!(operation, %error, "session command failed");
                false
            }
        }
    }
    #[cfg(all(not(target_os = "linux"), not(test)))]
    {
        let _ = operation;
        platform::send_shell_command(command)
    }
}

fn update_feed_status(current: &mut FeedStatus, next: FeedStatus, feed: &'static str) -> bool {
    if *current == next {
        return false;
    }
    tracing::info!(feed, status = ?next, "shell feed state changed");
    *current = next;
    true
}

fn session_feed_status_label(
    window_status: FeedStatus,
    workspace_status: FeedStatus,
) -> Option<&'static str> {
    match (window_status, workspace_status) {
        (FeedStatus::Loading, FeedStatus::Loading) => Some("Loading session data…"),
        (FeedStatus::Loading, _) => Some("Loading session windows…"),
        (_, FeedStatus::Loading) => Some("Loading session workspaces…"),
        (FeedStatus::Disconnected, _) => Some("Session window data is disconnected."),
        (_, FeedStatus::Disconnected) => Some("Session workspace data is disconnected."),
        (FeedStatus::Failed, _) => Some("Session window data failed to load."),
        (_, FeedStatus::Failed) => Some("Session workspace data failed to load."),
        (FeedStatus::Ready, FeedStatus::Ready) => None,
    }
}

fn application_discovery_status_label(
    status: crate::model::ApplicationDiscoveryStatus,
) -> Option<&'static str> {
    match status {
        crate::model::ApplicationDiscoveryStatus::ReadyEmpty => Some("No applications found."),
        crate::model::ApplicationDiscoveryStatus::Ready => None,
        crate::model::ApplicationDiscoveryStatus::PartialFailure => {
            Some("Some applications could not be loaded.")
        }
    }
}

fn launch_error_summary(error: &platform::LaunchError) -> String {
    match error {
        platform::LaunchError::EmptyCommand => "no launch command".into(),
        platform::LaunchError::InvalidQuotes => "invalid launch command quoting".into(),
        platform::LaunchError::MissingTarget(target) => format!("missing target {target}"),
        platform::LaunchError::NotFound(target) => format!("target not found: {target}"),
        platform::LaunchError::PathNotFound(path) => format!("path not found: {path}"),
        platform::LaunchError::AccessDenied(target) => format!("access denied: {target}"),
        platform::LaunchError::NoAssociation(target) => {
            format!("no application association for {target}")
        }
        platform::LaunchError::Platform(message) => message.clone(),
    }
}

fn wallpaper_cache_target(current: (u32, u32), requested: (u32, u32)) -> Option<(u32, u32)> {
    let bounded = (
        requested.0.clamp(1, WALLPAPER_MAX_WIDTH),
        requested.1.clamp(1, WALLPAPER_MAX_HEIGHT),
    );
    if current == (0, 0) || bounded.0 > current.0 || bounded.1 > current.1 {
        return Some(bounded);
    }
    let current_pixels = u64::from(current.0) * u64::from(current.1);
    let requested_pixels = u64::from(bounded.0) * u64::from(bounded.1);
    (requested_pixels.saturating_mul(4) <= current_pixels).then_some(bounded)
}

fn initial_wallpaper(
    configured_path: Option<std::path::PathBuf>,
    system_image: impl FnOnce() -> Option<image::RgbaImage>,
) -> (
    Option<std::path::PathBuf>,
    Option<Arc<image::RgbaImage>>,
    (u32, u32),
) {
    if configured_path.is_some() {
        return (configured_path, None, (0, 0));
    }
    let wallpaper = system_image().map(Arc::new);
    let size = wallpaper
        .as_deref()
        .map_or((0, 0), |image| image.dimensions());
    (None, wallpaper, size)
}

#[cfg(test)]
fn panel_control_start(width: u32) -> f32 {
    width as f32 - PANEL_CLOCK_WIDTH - PANEL_CONTROL_GAP
}

fn panel_tray_icons(items: &[TrayItem]) -> Vec<Arc<image::RgbaImage>> {
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

fn normalize_tray_items(items: Vec<TrayItem>) -> Vec<TrayItem> {
    let keep_from = items.len().saturating_sub(4);
    items.into_iter().skip(keep_from).collect()
}

fn normalize_preview_image(image: &image::RgbaImage) -> image::RgbaImage {
    image.clone()
}

fn retain_preview_generation(
    images: &mut HashMap<crate::model::WindowId, Arc<image::RgbaImage>>,
    windows: &[OpenWindow],
) {
    images.retain(|window, _| {
        windows
            .iter()
            .take(PREVIEW_CACHE_CAPACITY)
            .any(|candidate| candidate.id == *window)
    });
}

#[cfg(test)]
fn visible_tray_item(items: &[TrayItem], visual_index: usize) -> Option<&TrayItem> {
    items.get(items.len().saturating_sub(4).checked_add(visual_index)?)
}

fn tint_panel_icon(mut icon: image::RgbaImage, color: u32) -> image::RgbaImage {
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

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        sync::Arc,
        time::{Duration, Instant},
    };

    use image::{Rgba, RgbaImage};
    use nickel_input::KeyCode;
    use nickel_session_protocol::{
        AnchorSide, PointerInteraction, PreviewTargetAction, ScreenshotTargetAction, ShellRole,
        ShellSemanticTarget, WindowMenuTargetAction,
    };
    use nickel_ui::{
        ActionKind, Application as _, ControllerAction, FrameOverlay, HostBatch, HostEvent,
        HostTelemetry, InputModality, OverlayAnchor, Point, Rect, SemanticAction, SemanticRole,
        SemanticSelector, SemanticValueInput, SemanticValueSnapshot, Shortcut, UiEvent, UiHost,
        ViewContext,
    };
    use nickel_ui_testkit::{Scenario, Selector};

    use super::{
        HostRuntimeSamples, LiveShell, initial_wallpaper, panel_status_layout, panel_tray_icons,
        platform::{AudioStatus, FeedState, FeedStatus, GlobalShortcut, SecureStorageState},
        preview_refresh_due, secure_storage_status_label, semantic_theme_from_palette,
        session_feed_status_label, shortcut_capability_status, visible_tray_item,
        window_belongs_to_panel,
    };

    #[test]
    fn explicit_wallpaper_path_wins_without_loading_the_system_wallpaper() {
        let path = std::path::PathBuf::from("configured-wallpaper.png");
        let (resolved_path, wallpaper, size) = initial_wallpaper(Some(path.clone()), || {
            panic!("an explicit Nickel wallpaper must suppress the system fallback")
        });

        assert_eq!(resolved_path, Some(path));
        assert!(wallpaper.is_none());
        assert_eq!(size, (0, 0));
    }

    #[test]
    fn system_wallpaper_is_used_when_nickel_has_no_explicit_path() {
        let image = RgbaImage::from_pixel(3, 2, Rgba([10, 20, 30, 255]));
        let (resolved_path, wallpaper, size) = initial_wallpaper(None, || Some(image.clone()));

        assert!(resolved_path.is_none());
        assert_eq!(wallpaper.as_deref(), Some(&image));
        assert_eq!(size, (3, 2));
    }

    #[test]
    fn missing_system_wallpaper_degrades_to_the_themed_desktop_base() {
        let (resolved_path, wallpaper, size) = initial_wallpaper(None, || None);

        assert!(resolved_path.is_none());
        assert!(wallpaper.is_none());
        assert_eq!(size, (0, 0));
    }

    #[test]
    fn shortcut_capability_failures_have_visible_classified_status() {
        use nickel_input::global::{ShortcutCapability, UnavailableReason};

        assert_eq!(
            shortcut_capability_status(&ShortcutCapability::Available),
            None
        );
        for (reason, detail) in [
            (UnavailableReason::UnsupportedPlatform, "unsupported"),
            (UnavailableReason::MissingRuntime, "runtime"),
            (UnavailableReason::PermissionDenied, "permission"),
            (UnavailableReason::SessionLocked, "locked"),
            (
                UnavailableReason::Backend("registration conflict".into()),
                "registration conflict",
            ),
        ] {
            let status = shortcut_capability_status(&ShortcutCapability::Unavailable(reason))
                .expect("an unavailable shortcut adapter must remain visible");
            assert!(status.starts_with("Global shortcuts unavailable:"));
            assert!(status.contains(detail), "{status:?}");
        }
    }

    #[test]
    fn per_display_panel_projection_keeps_owned_and_unresolved_windows_only() {
        assert!(window_belongs_to_panel(false, Some("DP-1"), Some("DP-1")));
        assert!(!window_belongs_to_panel(
            false,
            Some("DP-1"),
            Some("HDMI-A-1")
        ));
        assert!(window_belongs_to_panel(false, Some("DP-1"), None));
        assert!(window_belongs_to_panel(
            true,
            Some("DP-1"),
            Some("HDMI-A-1")
        ));
    }

    #[test]
    fn host_runtime_phase_samples_are_bounded() {
        let mut samples = HostRuntimeSamples::default();
        for value in 0..70 {
            samples.record(HostTelemetry {
                input_to_message_us: value,
                input_to_frame_us: value,
                layout_us: value,
                paint_list_us: value,
                scheduled_wakeups: 1,
                ..HostTelemetry::default()
            });
        }
        assert_eq!(samples.input_to_frame_us.len(), 64);
        assert_eq!(samples.input_to_frame_us.front(), Some(&6));
        assert_eq!(samples.scheduled_wakeups, 70);
    }
    use crate::{
        launcher_view::{LauncherAction, LauncherApplication},
        model::{ApplicationId, OpenWindow, TrayItem, WindowGroup, WindowId},
        notification::{NotificationAction, NotificationRequest, NotificationStore},
        window_preview::{MenuAction, build_preview_frame},
        winit_shell::SurfaceRole,
    };
    use nickel_core::launcher_preferences::LauncherPreferences;
    use nickel_core::theme::{Appearance, ThemeMode, ThemePalette};

    #[test]
    fn launcher_open_focuses_search_and_sequential_input_survives_mode_change() {
        let mut shell = LiveShell::new().unwrap();
        shell.apply_session_launcher_visibility(true);
        shell.launcher_host.step(HostBatch {
            surface_size: Some((920, 680)),
            ..HostBatch::default()
        });
        assert!(shell.launcher_host.inspect().keyboard_focus.is_some());
        shell.launcher_host.step(HostBatch {
            events: vec![HostEvent::Ui(UiEvent::TextInput("a".into()))],
            ..HostBatch::default()
        });
        for action in shell.launcher_host.application_mut().take_effects() {
            shell.apply_launcher_action(action);
        }
        assert_eq!(shell.launcher.query(), "a");
        let status = shell.launcher_status_text();
        shell
            .launcher_host
            .application_mut()
            .sync(&shell.launcher, shell.palette, status);
        shell.launcher_host.step(HostBatch {
            events: vec![HostEvent::Ui(UiEvent::TextInput("b".into()))],
            ..HostBatch::default()
        });
        for action in shell.launcher_host.application_mut().take_effects() {
            shell.apply_launcher_action(action);
        }
        assert_eq!(shell.launcher.query(), "ab");
    }

    #[test]
    fn controller_cancel_closes_nested_overlay_before_requesting_launcher_dismissal() {
        assert!(matches!(
            super::launcher_controller_host_event(ControllerAction::Cancel, true),
            HostEvent::Controller(ControllerAction::Cancel)
        ));
        assert!(matches!(
            super::launcher_controller_host_event(ControllerAction::Cancel, false),
            HostEvent::Shortcut(Shortcut::Escape)
        ));
        assert!(matches!(
            super::launcher_controller_host_event(ControllerAction::Down, false),
            HostEvent::Controller(ControllerAction::Down)
        ));
    }

    #[test]
    fn failed_application_launch_keeps_launcher_open_and_reports_error() {
        let mut shell = LiveShell::new().unwrap();
        shell.launcher_visible = true;
        let application = crate::model::Application::new(
            "org.example.missing".into(),
            "Missing application".into(),
            None,
            None,
            Some(vec!["nickel-test-command-that-does-not-exist".into()]),
        );

        shell.launch_application(application);

        assert!(shell.launcher_visible);
        let status = shell.launcher_status.as_deref().unwrap_or_default();
        assert!(status.starts_with("Could not launch Missing application: "));
        assert!(status.contains("No such file") || status.contains("not found"));
    }

    #[test]
    fn unavailable_shortcut_application_is_a_visible_typed_failure() {
        let mut shell = LiveShell::new().unwrap();

        assert!(!shell.launch_named_application("Missing Nickel Tool"));
        assert_eq!(
            shell.shortcut_action_status.as_deref(),
            Some("Missing Nickel Tool is unavailable.")
        );
    }

    fn launcher_application_menu_has_label(
        launcher: &crate::launcher::Launcher,
        palette: nickel_core::theme::ThemePalette,
        application_id: &str,
        expected_label: &str,
    ) -> bool {
        let mut host = UiHost::new(
            LauncherApplication::new(
                launcher.clone(),
                crate::launcher_view::LauncherViewState::default(),
                crate::launcher_view::LauncherIconCache::new(),
                palette,
            ),
            920,
            680,
        );
        let target = host
            .unique_semantic_target_for_message(&LauncherAction::LaunchApplication(
                application_id.to_owned(),
            ))
            .expect("application semantic target");
        let outcome = host.perform_accessibility_action(
            target.id.clone(),
            SemanticAction::Invoke(ActionKind::ContextMenu),
        );
        assert!(outcome.failures.is_empty(), "{:#?}", outcome.failures);
        host.accessibility_nodes()
            .iter()
            .any(|node| node.label.as_deref() == Some(expected_label))
    }

    #[test]
    fn launcher_pin_persists_once_reopens_and_recovers_after_save_failure() {
        let directory = tempfile::tempdir().expect("temporary preferences directory");
        let preferences_path = directory.path().join("launcher-preferences");
        let mut shell = LiveShell::new().unwrap();
        let application_id = "org.nickel.Files".to_owned();
        shell.launcher = crate::launcher::Launcher::new(vec![crate::model::Application::new(
            application_id.clone(),
            "Files".into(),
            None,
            None,
            None,
        )]);
        shell.launcher_preferences_path = Some(preferences_path.clone());

        shell.apply_launcher_action(crate::launcher_view::LauncherAction::TogglePin(
            application_id.clone(),
        ));
        assert_eq!(shell.launcher_persistence_attempts, 1);
        assert!(shell.launcher.is_pinned(&application_id));
        let persisted = LauncherPreferences::load(&preferences_path).expect("persisted favorite");
        assert_eq!(persisted.favorites(), [application_id.as_str()]);

        let mut reopened =
            crate::launcher::Launcher::new(shell.launcher.applications().cloned().collect());
        reopened.set_preferences(persisted);
        assert!(reopened.is_pinned(&application_id));
        assert!(launcher_application_menu_has_label(
            &reopened,
            shell.palette,
            &application_id,
            "Unpin from Nickel Bar",
        ));

        shell.launcher_preferences_path = Some(directory.path().to_path_buf());
        shell.apply_launcher_action(crate::launcher_view::LauncherAction::TogglePin(
            application_id.clone(),
        ));
        assert_eq!(shell.launcher_persistence_attempts, 2);
        assert!(!shell.launcher.is_pinned(&application_id));
        assert!(
            shell.launcher_status.as_deref().is_some_and(
                |status| status.starts_with("Launcher preferences could not be saved:")
            )
        );
        assert!(launcher_application_menu_has_label(
            &shell.launcher,
            shell.palette,
            &application_id,
            "Pin to Nickel Bar",
        ));

        shell.launcher_preferences_path = Some(preferences_path.clone());
        shell.apply_launcher_action(crate::launcher_view::LauncherAction::TogglePin(
            application_id.clone(),
        ));
        assert_eq!(shell.launcher_persistence_attempts, 3);
        assert!(shell.launcher.is_pinned(&application_id));
        assert!(shell.launcher_status.is_none());
        assert_eq!(
            LauncherPreferences::load(preferences_path)
                .expect("recovered preferences")
                .favorites(),
            [application_id]
        );
    }

    fn launcher_scenario(
        launcher: &crate::launcher::Launcher,
        palette: nickel_core::theme::ThemePalette,
        status: Option<String>,
    ) -> Scenario<LauncherApplication> {
        let mut application = LauncherApplication::new(
            launcher.clone(),
            crate::launcher_view::LauncherViewState::default(),
            crate::launcher_view::LauncherIconCache::new(),
            palette,
        );
        application.sync(launcher, palette, status);
        Scenario::new(application, 920, 680)
    }

    fn application_context_target(
        scenario: &Scenario<LauncherApplication>,
        application_id: &str,
    ) -> Selector {
        let target = scenario
            .host()
            .unique_semantic_target_for_message(&LauncherAction::LaunchApplication(
                application_id.to_owned(),
            ))
            .expect("launcher application semantic target");
        Selector::id(target.id.as_str())
    }

    #[test]
    fn controller_scenario_pins_reopens_unpins_and_persists_each_action_once() {
        let directory = tempfile::tempdir().expect("temporary preferences directory");
        let preferences_path = directory.path().join("launcher-preferences");
        let mut shell = LiveShell::new().unwrap();
        shell.launcher = crate::launcher::Launcher::default();
        shell.launcher_preferences_path = Some(preferences_path.clone());
        let application_id = "firefox";

        let mut pin = launcher_scenario(&shell.launcher, shell.palette, None);
        let origin = application_context_target(&pin, application_id);
        pin.controller_semantic_action(&origin, ActionKind::ContextMenu)
            .expect("production controller context action opens application menu");
        pin.controller_activate(&Selector::role_name(
            SemanticRole::MenuItem,
            "Pin to Nickel Bar",
        ))
        .expect("controller reaches and confirms Pin");
        assert!(pin.host().inspect().open_overlay.is_none());
        assert_eq!(
            pin.host().inspect().controller_target.as_ref(),
            Some(
                &pin.host()
                    .query_unique(&SemanticSelector::Id(match &origin {
                        Selector::Id(id) => id.clone(),
                        _ => unreachable!(),
                    }))
                    .expect("origin remains present")
                    .id
            )
        );
        let effects = pin.host_mut().application_mut().take_effects();
        assert_eq!(effects, [LauncherAction::TogglePin(application_id.into())]);
        for effect in effects {
            shell.apply_launcher_action(effect);
        }
        assert_eq!(shell.launcher_persistence_attempts, 1);
        assert!(shell.launcher.is_pinned(application_id));
        assert_eq!(
            LauncherPreferences::load(&preferences_path)
                .expect("pin persisted")
                .favorites(),
            [application_id]
        );

        let mut unpin = launcher_scenario(&shell.launcher, shell.palette, None);
        let origin = application_context_target(&unpin, application_id);
        unpin
            .controller_semantic_action(&origin, ActionKind::ContextMenu)
            .expect("reopened menu uses authoritative favorite state");
        unpin
            .controller_activate(&Selector::role_name(
                SemanticRole::MenuItem,
                "Unpin from Nickel Bar",
            ))
            .expect("controller reaches and confirms Unpin");
        let effects = unpin.host_mut().application_mut().take_effects();
        assert_eq!(effects, [LauncherAction::TogglePin(application_id.into())]);
        for effect in effects {
            shell.apply_launcher_action(effect);
        }
        assert_eq!(shell.launcher_persistence_attempts, 2);
        assert!(!shell.launcher.is_pinned(application_id));
        assert!(
            LauncherPreferences::load(preferences_path)
                .expect("unpin persisted")
                .favorites()
                .is_empty()
        );
    }

    #[test]
    fn controller_scenario_logout_emits_only_the_typed_request() {
        let shell = LiveShell::new().unwrap();
        let mut scenario = launcher_scenario(&shell.launcher, shell.palette, None);
        let account = scenario
            .host()
            .unique_semantic_target_for_message(&LauncherAction::OpenAccount)
            .expect("account presentation semantic target");
        scenario
            .controller_semantic_action(&Selector::id(account.id.as_str()), ActionKind::ContextMenu)
            .expect("controller opens the shared account menu");
        scenario
            .controller_activate(&Selector::role_name(SemanticRole::MenuItem, "Log out"))
            .expect("controller reaches Logout");
        assert_eq!(
            scenario.host_mut().application_mut().take_effects(),
            [LauncherAction::RequestLogout]
        );
        assert!(scenario.host().inspect().open_overlay.is_none());
    }

    #[test]
    fn failed_pin_retry_is_idempotent_and_restores_origin_focus() {
        let directory = tempfile::tempdir().expect("temporary preferences directory");
        let valid_path = directory.path().join("launcher-preferences");
        let mut shell = LiveShell::new().unwrap();
        shell.launcher = crate::launcher::Launcher::default();
        let application_id = "firefox";
        shell.launcher_preferences_path = Some(directory.path().to_path_buf());

        shell.apply_launcher_action(LauncherAction::TogglePin(application_id.into()));
        assert_eq!(shell.launcher_persistence_attempts, 1);
        assert!(shell.launcher.is_pinned(application_id));
        let failure = shell
            .launcher_status
            .clone()
            .expect("truthful save failure");

        shell.launcher_preferences_path = Some(valid_path.clone());
        let mut retry = launcher_scenario(&shell.launcher, shell.palette, Some(failure));
        let origin = application_context_target(&retry, application_id);
        let origin_id = match &origin {
            Selector::Id(id) => id.clone(),
            _ => unreachable!(),
        };
        retry
            .controller_semantic_action(&origin, ActionKind::ContextMenu)
            .expect("failed menu remains controller-usable");
        retry
            .controller_activate(&Selector::role_name(
                SemanticRole::MenuItem,
                "Retry saving favorites",
            ))
            .expect("controller retries persistence without toggling state");
        assert!(retry.host().inspect().open_overlay.is_none());
        assert_eq!(
            retry.host().inspect().controller_target.as_ref(),
            Some(&origin_id),
            "closing the retry menu restores the originating application"
        );
        let effects = retry.host_mut().application_mut().take_effects();
        assert_eq!(effects, [LauncherAction::RetryPreferencePersistence]);
        for effect in effects {
            shell.apply_launcher_action(effect);
        }
        assert_eq!(shell.launcher_persistence_attempts, 2);
        assert!(shell.launcher.is_pinned(application_id));
        assert!(shell.launcher_status.is_none());
        assert_eq!(
            LauncherPreferences::load(valid_path)
                .expect("retry persisted unchanged authoritative state")
                .favorites(),
            [application_id]
        );
    }

    #[test]
    fn launcher_exposes_every_non_ready_secure_storage_state() {
        for (state, expected) in [
            (SecureStorageState::Starting, "Secure storage is starting…"),
            (SecureStorageState::Locked, "Secure storage is locked."),
            (
                SecureStorageState::PromptRequired,
                "Secure storage is waiting for its unlock prompt.",
            ),
            (
                SecureStorageState::Unavailable,
                "Secure storage is unavailable.",
            ),
            (
                SecureStorageState::UnavailableReason(
                    nickel_session_protocol::SecureStorageUnavailableReason::ProviderDisappeared,
                ),
                "The secure-storage provider disappeared.",
            ),
            (
                SecureStorageState::ControlUnavailable,
                "Nickel cannot reach the session service.",
            ),
        ] {
            assert_eq!(secure_storage_status_label(state), Some(expected));
        }
        assert_eq!(secure_storage_status_label(SecureStorageState::Ready), None);
    }

    #[test]
    fn session_feeds_start_loading_and_keep_failure_distinct_from_empty_ready() {
        let shell = LiveShell::new().unwrap();
        assert_eq!(shell.window_feed_status, FeedStatus::Loading);
        assert_eq!(shell.workspace_feed_status, FeedStatus::Loading);
        assert_eq!(
            session_feed_status_label(shell.window_feed_status, shell.workspace_feed_status),
            Some("Loading session data…")
        );

        assert_eq!(
            FeedState::<Vec<OpenWindow>>::Ready(Vec::new()).status(),
            FeedStatus::Ready
        );
        assert_eq!(
            FeedState::<Vec<OpenWindow>>::Disconnected.status(),
            FeedStatus::Disconnected
        );
        assert_eq!(
            FeedState::<Vec<OpenWindow>>::Failed.status(),
            FeedStatus::Failed
        );
        assert_eq!(
            session_feed_status_label(FeedStatus::Ready, FeedStatus::Ready),
            None
        );
        assert_eq!(
            session_feed_status_label(FeedStatus::Disconnected, FeedStatus::Ready),
            Some("Session window data is disconnected.")
        );
        assert_eq!(
            session_feed_status_label(FeedStatus::Failed, FeedStatus::Ready),
            Some("Session window data failed to load.")
        );
    }

    #[test]
    fn semantic_shell_targets_come_from_live_group_preview_and_menu_records() {
        let mut shell = LiveShell::new().unwrap();
        shell
            .launcher
            .set_preferences(LauncherPreferences::default());
        let application_id = ApplicationId::new("org.kde.konsole");
        shell.windows = vec![
            OpenWindow {
                id: WindowId(4),
                application_id: Some(application_id.clone()),
                active: true,
                title: "one".into(),
                state: crate::model::WindowState::default(),
            },
            OpenWindow {
                id: WindowId(9),
                application_id: Some(application_id),
                active: false,
                title: "two".into(),
                state: crate::model::WindowState::default(),
            },
        ];
        let _ = shell.scene(SurfaceRole::Panel, 1280, 56);
        let panel = shell
            .resolve_semantic_target(&ShellSemanticTarget::PanelApplication {
                application_id: "org.kde.konsole".into(),
                output: Some("DP-1".into()),
                interaction: PointerInteraction::Hover,
            })
            .expect("live panel group resolves");
        assert_eq!(panel.role, ShellRole::Panel);
        assert_eq!(panel.output.as_deref(), Some("DP-1"));
        assert!(shell.panel_pointer_moved(panel.x as f32, 1280));
        assert_eq!(shell.panel_hover, Some(super::PanelHover::Task(0)));
        assert!(shell.preview_group.is_none());
        assert_eq!(shell.preview_pending.map(|(index, _)| index), Some(0));
        assert!(shell.preview_pending.unwrap().1 > Instant::now());

        let (preview_width, _) = super::preview_dimensions(2);
        assert_eq!(
            shell.preview_origin_x(0, preview_width),
            panel.x - i32::try_from(preview_width / 2).unwrap()
        );

        let group = shell.launcher.group_windows(&shell.windows).remove(0);
        shell.preview_frame = Some(build_preview_frame(
            &group,
            &HashMap::new(),
            None,
            shell.semantic_theme(),
        ));
        let preview = shell
            .resolve_semantic_target(&ShellSemanticTarget::PreviewWindow {
                window: nickel_session_protocol::WindowId(9),
                action: PreviewTargetAction::Close,
            })
            .expect("live preview close target resolves");
        assert_eq!(preview.role, ShellRole::Preview);
        assert_eq!(preview.interaction, PointerInteraction::LeftClick);
        assert_eq!(
            shell.preview_frame.as_mut().unwrap().transition_pointer(
                Point {
                    x: preview.x as f32,
                    y: preview.y as f32,
                },
                false,
            ),
            Some(crate::window_preview::PreviewAction::Close(WindowId(9)))
        );

        shell.window_menu = Some(WindowId(9));
        let _ = shell.window_menu_scene();
        let menu = shell
            .resolve_semantic_target(&ShellSemanticTarget::WindowMenu {
                window: nickel_session_protocol::WindowId(9),
                action: WindowMenuTargetAction::Minimize,
            })
            .expect("live context-menu row resolves");
        assert_eq!(menu.role, ShellRole::ContextMenu);
        assert!(
            shell
                .window_menu_host
                .as_ref()
                .unwrap()
                .semantic_targets_for_message(&MenuAction::Minimize(WindowId(9)))
                .into_iter()
                .next()
                .is_some()
        );

        shell.screenshot.show(image::RgbaImage::new(400, 200));
        let _ = shell.scene(SurfaceRole::Screenshot, 800, 600);
        assert!(shell.perform_screenshot_semantic_action(ScreenshotTargetAction::SelectionStart));
        assert!(shell.perform_screenshot_semantic_action(ScreenshotTargetAction::SelectionEnd));
        assert!(shell.perform_screenshot_semantic_action(ScreenshotTargetAction::Confirm));
        assert!(shell.screenshot.confirmed());
    }

    #[test]
    fn taskbar_secondary_click_opens_menu_for_active_group_member_at_item_anchor() {
        let mut shell = LiveShell::new().unwrap();
        shell
            .launcher
            .set_preferences(LauncherPreferences::default());
        let application_id = ApplicationId::new("org.kde.dolphin");
        shell.windows = vec![
            OpenWindow {
                id: WindowId(41),
                application_id: Some(application_id.clone()),
                active: false,
                title: "Files".into(),
                state: crate::model::WindowState::default(),
            },
            OpenWindow {
                id: WindowId(42),
                application_id: Some(application_id),
                active: true,
                title: "Downloads".into(),
                state: crate::model::WindowState::default(),
            },
        ];
        shell.panel_origin_x = 1_920;
        let _ = shell.scene(SurfaceRole::Panel, 1_280, 56);
        let target = shell
            .panel_host
            .unique_semantic_target_for_message(&super::PanelAction::Task(0))
            .expect("taskbar item");
        let expected_anchor = shell.panel_origin_x + target.bounds.origin.x.round() as i32;
        let center = target.bounds.origin.x + target.bounds.size.width / 2.0;

        assert!(shell.panel_click(center, 1_280, true));
        assert_eq!(shell.window_menu, Some(WindowId(42)));
        assert_eq!(shell.window_menu_anchor_x, Some(expected_anchor));
        assert!(shell.preview_group.is_none());
        assert_eq!(
            shell.window_menu_snapshot.as_ref().map(|window| window.id),
            Some(WindowId(42))
        );
        shell.windows[0].active = true;
        shell.windows[1].active = false;
        assert_eq!(
            shell.window_menu_snapshot.as_ref().map(|window| window.id),
            Some(WindowId(42)),
            "an open menu must not retarget when group activity changes"
        );
        shell.windows[0].active = false;
        shell.windows[1].active = true;

        shell.sync_transient_overlays();
        assert_eq!(shell.window_menu_anchor_x, Some(expected_anchor));

        shell.close_window_preview();
        let outcome = shell.panel_host.perform_accessibility_action(
            target.id.clone(),
            SemanticAction::Invoke(ActionKind::ContextMenu),
        );
        assert!(outcome.failures.is_empty(), "{:#?}", outcome.failures);
        assert!(shell.apply_panel_effects());
        assert_eq!(shell.window_menu, Some(WindowId(42)));
        assert_eq!(shell.window_menu_anchor_x, Some(expected_anchor));

        for event in [
            HostEvent::Ui(UiEvent::KeyboardContextMenu),
            HostEvent::Controller(ControllerAction::ContextMenu),
        ] {
            shell.close_window_preview();
            shell.panel_host.step(HostBatch {
                events: vec![
                    HostEvent::Ui(UiEvent::AccessibilityFocus(target.id.clone())),
                    event,
                ],
                ..HostBatch::default()
            });
            assert!(shell.apply_panel_effects());
            assert_eq!(shell.window_menu, Some(WindowId(42)));
            assert_eq!(shell.window_menu_anchor_x, Some(expected_anchor));
        }
    }

    #[test]
    fn panel_popover_anchor_is_semantic_and_scoped_to_the_invoking_output() {
        let mut shell = LiveShell::new().unwrap();
        let _ = shell.scene(SurfaceRole::Panel, 1_280, 56);
        shell.set_panel_output("left");
        let target = shell
            .panel_host
            .unique_semantic_target_for_message(&super::PanelAction::Control)
            .expect("control button");
        let expected = target.bounds;
        let outcome = shell
            .panel_host
            .perform_accessibility_action(target.id, SemanticAction::Invoke(ActionKind::Activate));
        assert!(outcome.failures.is_empty(), "{:#?}", outcome.failures);
        assert!(shell.apply_panel_effects());
        let (role, first) = shell.popover_anchor(AnchorSide::Above).unwrap();
        assert_eq!(role, ShellRole::ControlCenter);
        assert_eq!(first.control, "panel-control");
        assert_eq!(first.output, "left");
        assert_eq!(first.bounds.x, expected.origin.x.floor() as i32);

        shell.set_panel_output("right");
        for _ in 0..2 {
            let target = shell
                .panel_host
                .unique_semantic_target_for_message(&super::PanelAction::Control)
                .unwrap();
            let outcome = shell.panel_host.perform_accessibility_action(
                target.id,
                SemanticAction::Invoke(ActionKind::Activate),
            );
            assert!(outcome.failures.is_empty(), "{:#?}", outcome.failures);
            assert!(shell.apply_panel_effects());
        }
        let (_, reopened) = shell.popover_anchor(AnchorSide::Below).unwrap();
        assert_eq!(reopened.output, "right");
        assert_eq!(reopened.preferred, AnchorSide::Below);
        assert_eq!(reopened.bounds, first.bounds);
    }

    #[test]
    fn lock_application_transfers_password_to_a_typed_authentication_effect() {
        let mut application = super::LockApplication {
            password: zeroize::Zeroizing::new(String::new()),
            status: None,
            effects: Vec::new(),
        };
        nickel_ui::Application::update(
            &mut application,
            super::LockMessage::Password("secret".into()),
        );
        assert!(nickel_ui::Application::shortcut(
            &mut application,
            nickel_ui::Shortcut::Submit
        ));
        assert!(application.password.is_empty());
        let super::LockEffect::Authenticate(password) = application.effects.pop().unwrap();
        assert_eq!(&**password, "secret");
    }

    #[test]
    fn lock_password_is_a_named_protected_textbox_through_the_production_host() {
        let mut scenario = Scenario::new(super::LockApplication::fixture("nickel", None), 960, 540);
        let selector = Selector::RoleAndName {
            role: SemanticRole::TextField,
            name: "Password".into(),
        };
        let target = scenario
            .host()
            .query_unique(&SemanticSelector::RoleAndName {
                role: SemanticRole::TextField,
                name: "Password".into(),
            })
            .expect("lock password semantic target");

        assert_eq!(target.role, Some(SemanticRole::TextField));
        assert_eq!(target.name.as_deref(), Some("Password"));
        assert_eq!(target.actions, vec![ActionKind::SetValue]);
        scenario
            .assert_value(
                &selector,
                &SemanticValueSnapshot::ProtectedText { character_count: 6 },
            )
            .unwrap();
        scenario
            .set_value(&selector, SemanticValueInput::Text("new secret".into()))
            .unwrap()
            .assert_value(
                &selector,
                &SemanticValueSnapshot::ProtectedText {
                    character_count: 10,
                },
            )
            .unwrap();
    }

    #[test]
    fn control_center_keyboard_navigation_uses_host_semantic_order() {
        let mut shell = LiveShell::new().unwrap();
        shell.control_visible = true;

        assert!(shell.control_key(Some(KeyCode::ArrowDown), 420, 600));
        assert!(shell.control_host.inspect().controller_target.is_some());
        assert!(shell.control_key(Some(KeyCode::ArrowUp), 420, 600));
        assert!(shell.control_host.inspect().controller_target.is_some());
        assert!(shell.control_key(Some(KeyCode::Escape), 420, 600));
        assert!(!shell.control_visible);
    }

    #[test]
    fn control_center_controller_dispatch_matches_keyboard_adapter() {
        let mut keyboard = LiveShell::new().unwrap();
        let mut controller = LiveShell::new().unwrap();
        keyboard.control_visible = true;
        controller.control_visible = true;

        assert!(controller.control_controller(nickel_ui::ControllerAction::Down, 420, 600));
        assert!(
            controller
                .control_host
                .inspect()
                .controller_target
                .is_some()
        );

        assert!(keyboard.control_key(Some(KeyCode::Escape), 420, 600));
        assert!(controller.control_controller(nickel_ui::ControllerAction::Cancel, 420, 600));
        assert_eq!(controller.control_visible, keyboard.control_visible);
    }

    #[test]
    fn transient_keyboard_navigation_uses_production_frame_order() {
        let mut shell = LiveShell::new().unwrap();
        let palette = nickel_core::theme::ThemePalette::from_appearance(Appearance::default());
        let group = WindowGroup {
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
                    title: "two".into(),
                    state: crate::model::WindowState::default(),
                },
            ],
        };
        shell.preview_group = Some(0);
        shell.preview_frame = Some(build_preview_frame(
            &group,
            &HashMap::new(),
            None,
            semantic_theme_from_palette(palette),
        ));

        assert!(shell.preview_key(Some(KeyCode::ArrowRight)));
        assert_eq!(shell.preview_hovered, Some(WindowId(9)));
        assert!(shell.preview_key(Some(KeyCode::ArrowLeft)));
        assert_eq!(shell.preview_hovered, Some(WindowId(4)));

        shell.window_menu = Some(WindowId(4));
        shell.window_menu_snapshot = Some(group.windows[0].clone());
        let _ = shell.window_menu_scene();
        assert!(shell.preview_key(Some(KeyCode::ArrowDown)));
        assert!(
            shell
                .window_menu_host
                .as_ref()
                .unwrap()
                .inspect()
                .controller_target
                .is_some()
        );
        let first_target = shell
            .window_menu_host
            .as_ref()
            .unwrap()
            .inspect()
            .controller_target
            .clone();
        assert!(!shell.window_menu_host_key(Some(KeyCode::ArrowUp)));
        assert_eq!(
            shell
                .window_menu_host
                .as_ref()
                .unwrap()
                .inspect()
                .controller_target,
            first_target
        );
        assert!(shell.window_menu_host_key(Some(KeyCode::ArrowDown)));
        assert_ne!(
            shell
                .window_menu_host
                .as_ref()
                .unwrap()
                .inspect()
                .controller_target,
            first_target
        );
    }

    #[test]
    fn notification_host_effects_stay_at_the_transport_boundary() {
        let mut shell = LiveShell::new().unwrap();
        let mut store = NotificationStore::default();
        store.notify(
            0,
            NotificationRequest {
                app_name: "Test".into(),
                summary: "Ready".into(),
                body: "Choose".into(),
                actions: vec![NotificationAction {
                    key: "open".into(),
                    label: "Open".into(),
                }],
                expire_timeout_ms: 0,
            },
            Instant::now(),
        );
        shell.notification = store.newest();
        let _ = shell.scene(SurfaceRole::Notification, 420, 180);
        let target = shell
            .notification_host
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: nickel_ui::SemanticRole::Button,
                name: "Open".into(),
            })
            .unwrap();
        let point = Point {
            x: target.bounds.origin.x + target.bounds.size.width / 2.0,
            y: target.bounds.origin.y + target.bounds.size.height / 2.0,
        };

        assert!(shell.notification_click(point.x, point.y, 420, 180));
        assert!(shell.notification.is_none());
        assert!(
            shell
                .notification_host
                .query(&nickel_ui::SemanticSelector::Role(
                    nickel_ui::SemanticRole::Dialog
                ))
                .is_empty()
        );
    }

    #[test]
    fn notification_controller_cancel_uses_the_typed_host_effect() {
        let mut shell = LiveShell::new().unwrap();
        let mut store = NotificationStore::default();
        store.notify(
            0,
            NotificationRequest {
                app_name: "Test".into(),
                summary: "Ready".into(),
                body: "Choose".into(),
                actions: vec![],
                expire_timeout_ms: 0,
            },
            Instant::now(),
        );
        shell.notification = store.newest();
        let _ = shell.scene(SurfaceRole::Notification, 420, 180);

        assert!(shell.notification_controller(ControllerAction::Cancel));
        assert!(shell.notification.is_none());
        assert!(
            shell
                .notification_host
                .query(&nickel_ui::SemanticSelector::Role(
                    nickel_ui::SemanticRole::Dialog
                ))
                .is_empty()
        );
    }

    #[test]
    fn right_panel_cluster_is_compact_and_grouped() {
        let layout = panel_status_layout(1920, 3, true);
        assert_eq!(layout.control_start, 1816.0);
        assert_eq!(layout.tray_start, 1732.0);
        assert_eq!(layout.codex_start, 1696.0);
        assert_eq!(
            layout.codex_icon_bounds(),
            Rect::new(1700.0, 14.0, 28.0, 28.0)
        );
    }

    #[test]
    fn panel_host_owns_pointer_and_accessibility_targets() {
        let mut shell = LiveShell::new().unwrap();
        let commands = shell.scene(SurfaceRole::Panel, 1280, 56);
        assert!(!commands.is_empty());

        let launcher = shell
            .panel_host
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: nickel_ui::SemanticRole::Button,
                name: "Open Nickel Start".into(),
            })
            .unwrap();
        let center = Point {
            x: launcher.bounds.origin.x + launcher.bounds.size.width / 2.0,
            y: launcher.bounds.origin.y + launcher.bounds.size.height / 2.0,
        };
        assert!(shell.panel_pointer_moved(center.x, 1280));
        assert_eq!(shell.panel_hover, Some(super::PanelHover::Launcher));
        assert!(
            shell
                .panel_host
                .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                    role: nickel_ui::SemanticRole::Button,
                    name: "Open Quick Settings".into(),
                })
                .is_ok()
        );
    }

    #[test]
    fn panel_scene_rebuilds_when_persisted_appearance_changes() {
        let mut shell = LiveShell::new().unwrap();
        let before_commands = shell.scene(SurfaceRole::Panel, 1280, 56);
        let before = shell.panel_change_token;
        let light = ThemePalette::from_appearance(Appearance {
            mode: ThemeMode::Light,
            accent: nickel_core::theme::accent_from_hue(167),
            intensity: 100,
        });
        let dark = ThemePalette::from_appearance(Appearance {
            mode: ThemeMode::Dark,
            accent: nickel_core::theme::accent_from_hue(167),
            intensity: 100,
        });
        shell.palette = if shell.palette == light { dark } else { light };

        let commands = shell.scene(SurfaceRole::Panel, 1280, 56);

        assert_ne!(shell.panel_change_token, before);
        assert_ne!(commands, before_commands);
    }

    #[test]
    fn panel_scene_rebuilds_when_a_window_feed_adds_an_application() {
        let mut shell = LiveShell::new().unwrap();
        let _ = shell.scene(SurfaceRole::Panel, 1280, 56);
        let before = shell.panel_change_token;
        shell.windows.push(OpenWindow {
            id: WindowId(77),
            application_id: Some(ApplicationId::new("google-chrome")),
            active: true,
            title: "Chrome".into(),
            state: crate::model::WindowState::default(),
        });

        let _ = shell.scene(SurfaceRole::Panel, 1280, 56);

        assert_ne!(shell.panel_change_token, before);
        assert_eq!(
            shell
                .panel_host
                .semantic_targets_for_message(&super::PanelAction::Task(0))
                .len(),
            1
        );
    }

    #[test]
    fn taskbar_drag_reorders_a_pin_without_emitting_activation() {
        let mut launcher = crate::launcher::Launcher::new(vec![
            crate::model::Application::new(
                "first".into(),
                "First".into(),
                None,
                None,
                Some(vec!["first".into()]),
            ),
            crate::model::Application::new(
                "second".into(),
                "Second".into(),
                None,
                None,
                Some(vec!["second".into()]),
            ),
        ]);
        launcher.set_pins(vec![("first".into(), 0), ("second".into(), 1)]);
        let mut panel = super::PanelApplication::fixture(
            launcher,
            ThemePalette::from_appearance(Appearance::default()),
        );
        let bounds = Rect::new(100.0, 0.0, 48.0, 48.0);

        nickel_ui::Application::update(
            &mut panel,
            super::PanelAction::TaskDrag(
                0,
                nickel_ui::DragGesture {
                    phase: nickel_ui::DragPhase::Moved,
                    position: Point { x: 170.0, y: 20.0 },
                    bounds,
                },
            ),
        );
        nickel_ui::Application::update(
            &mut panel,
            super::PanelAction::TaskDrag(
                0,
                nickel_ui::DragGesture {
                    phase: nickel_ui::DragPhase::Ended,
                    position: Point { x: 170.0, y: 20.0 },
                    bounds,
                },
            ),
        );

        assert_eq!(
            panel.effects,
            [super::PanelAction::MoveTaskPinRight("first".into())]
        );
    }

    #[test]
    fn closed_taskbar_pin_opens_application_actions_and_unpins_once() {
        let directory = tempfile::tempdir().expect("temporary preferences directory");
        let mut shell = LiveShell::new().unwrap();
        shell.launcher = crate::launcher::Launcher::new(vec![crate::model::Application::new(
            "org.example.pinned".into(),
            "Pinned Example".into(),
            None,
            None,
            Some(vec!["example".into()]),
        )]);
        shell
            .launcher
            .set_pins(vec![("org.example.pinned".into(), 0)]);
        shell.launcher_preferences_path = Some(directory.path().join("launcher-preferences"));
        shell.windows.clear();
        shell.sync_panel_host();

        shell.apply_panel_action(super::PanelAction::TaskContext(0));

        assert!(
            shell.window_menu.is_none(),
            "closed pins have no WindowId target"
        );
        assert_eq!(
            shell
                .window_menu_snapshot
                .as_ref()
                .and_then(|target| target.application_id.as_ref())
                .map(crate::model::ApplicationId::as_str),
            Some("org.example.pinned")
        );
        let _ = shell.window_menu_scene();
        let host = shell
            .window_menu_host
            .as_ref()
            .expect("application-only task menu host");
        assert!(host.accessibility_nodes().iter().any(|node| {
            node.label.as_deref() == Some("New Window")
                && node.semantic_role == Some(SemanticRole::Button)
        }));
        assert!(host.accessibility_nodes().iter().any(|node| {
            node.label.as_deref() == Some("Unpin from Nickel Bar")
                && node.semantic_role == Some(SemanticRole::Button)
        }));

        shell.apply_window_menu_action(crate::window_preview::MenuAction::TogglePin(
            crate::model::ApplicationId::new("org.example.pinned"),
        ));
        assert!(!shell.launcher.is_pinned("org.example.pinned"));
    }

    #[test]
    fn due_panel_clock_deadline_rebuilds_only_when_the_minute_changes() {
        let mut shell = LiveShell::new().unwrap();
        let _ = shell.scene(SurfaceRole::Panel, 1280, 56);
        shell.panel_host.application_mut().clock = "stale".into();
        shell.panel_host.application_mut().date = "stale".into();
        let now = Instant::now();
        shell.panel_deadline = Some(now);

        assert!(shell.poll_host_deadlines(now).contains(&SurfaceRole::Panel));
        assert_ne!(shell.panel_host.application().clock, "stale");
        assert!(shell.panel_deadline.is_some_and(|deadline| deadline > now));
    }

    #[test]
    fn every_advertised_shell_deadline_is_consumed_when_due() {
        let mut shell = LiveShell::new().unwrap();
        let _ = shell.scene(SurfaceRole::Desktop, 1280, 720);
        let _ = shell.scene(SurfaceRole::Panel, 1280, 56);
        let _ = shell.scene(SurfaceRole::Lock, 1280, 720);
        let _ = shell.scene(SurfaceRole::ControlCenter, 420, 640);
        let now = Instant::now();
        shell.desktop_deadline = Some(now);
        shell.panel_deadline = Some(now);
        shell.lock_deadline = Some(now);
        shell.control_deadline = Some(now);
        shell.preview_pending = Some((usize::MAX, now));
        shell.preview_leave_deadline = Some(now);
        shell.screenshot.request_capture();
        shell.screenshot.queue_pointer_moved(4.0, 5.0, 800, 600);
        let due = shell
            .next_host_deadline()
            .expect("the shell advertises its earliest wakeup")
            .max(now + Duration::from_millis(100));

        let outcome = shell.poll_deadlines(due);

        assert!(outcome.capture_screenshot);
        assert!(shell.desktop_deadline.is_none_or(|deadline| deadline > due));
        assert!(shell.panel_deadline.is_none_or(|deadline| deadline > due));
        assert!(shell.lock_deadline.is_none_or(|deadline| deadline > due));
        assert!(shell.control_deadline.is_none_or(|deadline| deadline > due));
        assert!(
            shell
                .next_host_deadline()
                .is_none_or(|deadline| deadline > due)
        );
    }

    #[test]
    fn panel_hover_treats_semantic_ids_as_opaque() {
        let mut shell = LiveShell::new().unwrap();
        shell.tray = vec![TrayItem {
            id: "opaque/panel-task-999".into(),
            title: "Opaque tray target".into(),
            icon: RgbaImage::new(18, 18),
        }];
        shell.tray_icons = panel_tray_icons(&shell.tray);
        let _ = shell.scene(SurfaceRole::Panel, 1280, 56);

        let tray = shell
            .panel_host
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: nickel_ui::SemanticRole::Button,
                name: "Opaque tray target".into(),
            })
            .unwrap();
        let center = Point {
            x: tray.bounds.origin.x + tray.bounds.size.width / 2.0,
            y: tray.bounds.origin.y + tray.bounds.size.height / 2.0,
        };

        assert!(shell.panel_pointer_moved(center.x, 1280));
        assert_eq!(shell.panel_hover, Some(super::PanelHover::Tray(0)));
    }

    #[test]
    fn panel_hover_is_projected_only_on_the_output_that_received_pointer_input() {
        let mut shell = LiveShell::new().unwrap();
        shell.panel_hover = Some(super::PanelHover::Launcher);
        shell.panel_hover_output = Some("DP-1".into());

        shell.set_panel_output("DP-1");
        assert_eq!(
            shell.visible_panel_hover(),
            Some(super::PanelHover::Launcher)
        );
        shell.set_panel_output("HDMI-A-1");
        assert_eq!(shell.visible_panel_hover(), None);
        assert!(!shell.panel_pointer_left());
        assert_eq!(shell.panel_hover, Some(super::PanelHover::Launcher));

        shell.set_panel_output("DP-1");
        assert!(shell.panel_pointer_left());
        assert_eq!(shell.panel_hover, None);
    }

    #[test]
    fn right_panel_omits_codex_space_until_codex_is_available() {
        let layout = panel_status_layout(1920, 3, false);
        assert_eq!(layout.codex_start, layout.tray_start);
    }

    #[test]
    fn stale_codex_actions_cannot_restore_hidden_chrome_or_project_requests() {
        use nickel_core::optional_features::{
            CodexAvailabilityProjection, FeatureHealth, FeatureInstallation, FeatureSupport,
        };
        let mut shell = LiveShell::new().unwrap();
        shell.requested_codex_project = Some("stale-private-project".into());
        shell.codex_project_menu_visible = true;
        shell.apply_codex_projection(CodexAvailabilityProjection::new(
            FeatureSupport::Supported,
            FeatureInstallation::Installed,
            false,
            FeatureHealth::Unknown,
            9,
            None,
        ));
        assert_eq!(shell.take_requested_codex_project(), None);
        shell.apply_panel_action(super::PanelAction::Codex);
        assert!(!shell.codex_project_menu_visible);
    }

    #[test]
    fn panel_reentry_cancels_a_stale_preview_leave_deadline() {
        let mut shell = LiveShell::new().unwrap();
        shell.preview_leave_deadline = Some(Instant::now());

        assert!(shell.panel_pointer_entered());
        assert!(shell.preview_leave_deadline.is_none());
        assert!(!shell.panel_pointer_entered());
    }

    #[test]
    fn preview_refresh_has_a_hard_temporal_bound() {
        let now = Instant::now();
        assert!(preview_refresh_due(None, now));
        assert!(!preview_refresh_due(
            Some(now + super::PREVIEW_REFRESH_INTERVAL),
            now
        ));
        assert!(preview_refresh_due(Some(now), now));
    }

    #[test]
    fn wallpaper_cache_caps_growth_and_reclaims_four_x_area_reductions() {
        assert_eq!(
            super::wallpaper_cache_target((0, 0), (16_000, 9_000)),
            Some((7680, 4320))
        );
        assert_eq!(
            super::wallpaper_cache_target((3840, 2160), (2560, 1440)),
            None,
            "minor topology changes reuse the existing thumbnail"
        );
        assert_eq!(
            super::wallpaper_cache_target((3840, 2160), (1920, 1080)),
            Some((1920, 1080)),
            "4K to FHD releases three quarters of retained pixels"
        );
    }

    #[test]
    fn desktop_scene_rebuilds_when_wallpaper_arrives_at_the_initial_host_size() {
        let mut shell = LiveShell::new().unwrap();
        shell.wallpaper = None;
        shell.wallpaper_size = (0, 0);
        shell.desktop_host.application_mut().wallpaper = None;
        shell.desktop_host.step(HostBatch {
            application_changed: true,
            surface_size: Some((1920, 1080)),
            ..HostBatch::default()
        });
        shell.wallpaper = Some(Arc::new(RgbaImage::from_pixel(
            1920,
            1080,
            Rgba([10, 20, 30, 255]),
        )));
        shell.wallpaper_size = (1920, 1080);
        let initial = shell.desktop_host.inspect();

        shell.desktop_scene(1920, 1080);
        let rebuilt = shell.desktop_host.inspect();

        assert_eq!(rebuilt.frame_generation, initial.frame_generation + 1);
        assert!(
            rebuilt.resources.paint_primitive_count > initial.resources.paint_primitive_count,
            "the declarative wallpaper image enters the rebuilt desktop frame"
        );
    }

    #[test]
    fn preview_cache_retains_authoritative_source_aspect_for_ui_containment() {
        let source = RgbaImage::from_pixel(240, 135, Rgba([10, 20, 30, 255]));
        let normalized = super::normalize_preview_image(&source);

        assert_eq!(normalized.dimensions(), source.dimensions());
        assert_eq!(normalized.as_raw(), source.as_raw());
        assert_eq!(
            super::PREVIEW_CACHE_CAPACITY * normalized.as_raw().len(),
            4_147_200
        );
    }

    #[test]
    fn shell_image_diagnostics_account_owned_caches_without_process_rss() {
        let mut shell = LiveShell::new().unwrap();
        shell.wallpaper = Some(Arc::new(RgbaImage::new(10, 10)));
        shell.tray = vec![TrayItem {
            id: "fixture".into(),
            title: "Fixture".into(),
            icon: RgbaImage::new(18, 18),
        }];
        shell.tray_icons = panel_tray_icons(&shell.tray);
        shell
            .preview_images
            .insert(WindowId(1), Arc::new(RgbaImage::new(240, 135)));

        let diagnostics = shell.image_cache_diagnostics();
        assert_eq!(diagnostics.wallpaper_entries, 1);
        assert_eq!(diagnostics.wallpaper_bytes, 400);
        assert_eq!(diagnostics.tray_entries, 2);
        assert_eq!(diagnostics.tray_bytes, 18 * 18 * 4 * 2);
        assert_eq!(diagnostics.preview_entries, 1);
        assert_eq!(diagnostics.preview_bytes, 240 * 135 * 4);
    }

    #[test]
    fn preview_cache_churn_releases_previous_group_pixels_and_stays_bounded() {
        let normalized = Arc::new(super::normalize_preview_image(&RgbaImage::from_pixel(
            240,
            135,
            Rgba([10, 20, 30, 255]),
        )));
        let mut cache = HashMap::new();
        for generation in 0..20_u64 {
            let windows = (0..64_u64)
                .map(|offset| OpenWindow {
                    id: WindowId(generation * 100 + offset),
                    application_id: None,
                    active: false,
                    title: String::new(),
                    state: crate::model::WindowState::default(),
                })
                .collect::<Vec<_>>();
            super::retain_preview_generation(&mut cache, &windows);
            for window in windows.iter().take(super::PREVIEW_CACHE_CAPACITY) {
                cache.insert(window.id, Arc::new((*normalized).clone()));
            }
            assert_eq!(cache.len(), super::PREVIEW_CACHE_CAPACITY);
            assert!(cache.keys().all(|id| id.0 / 100 == generation));
            assert_eq!(
                cache
                    .values()
                    .map(|image| image.as_raw().len())
                    .sum::<usize>(),
                super::PREVIEW_CACHE_CAPACITY * normalized.as_raw().len()
            );
        }
        cache.clear();
        assert!(cache.is_empty());
        assert_eq!(Arc::strong_count(&normalized), 1);
    }

    #[test]
    fn tray_icons_are_normalized_without_letter_fallbacks() {
        let item = TrayItem {
            id: "status".into(),
            title: "Status Application".into(),
            icon: RgbaImage::from_pixel(32, 16, Rgba([10, 20, 30, 255])),
        };
        let icons = panel_tray_icons(&[item]);

        assert_eq!(icons.len(), 1);
        assert_eq!(icons[0].dimensions(), (18, 18));
        assert!(icons[0].pixels().any(|pixel| pixel.0[3] != 0));
    }

    #[test]
    fn tray_source_keeps_only_four_items_without_discarding_source_quality() {
        let items = (0..7)
            .map(|index| TrayItem {
                id: index.to_string(),
                title: format!("Item {index}"),
                icon: RgbaImage::from_pixel(128, 64, Rgba([index, 0, 0, 255])),
            })
            .collect();
        let normalized = super::normalize_tray_items(items);

        assert_eq!(
            normalized
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["3", "4", "5", "6"]
        );
        assert!(
            normalized
                .iter()
                .all(|item| item.icon.dimensions() == (128, 64))
        );
        assert_eq!(
            normalized
                .iter()
                .map(|item| item.icon.as_raw().len())
                .sum::<usize>(),
            131_072
        );
    }

    #[test]
    #[ignore = "release-only cache timing evidence; debug resampling is intentionally slow"]
    fn shell_image_cache_warm_and_churn_costs_are_measured() {
        use std::time::Duration;

        fn p95(mut samples: Vec<Duration>) -> Duration {
            samples.sort_unstable();
            samples[samples.len() * 95 / 100]
        }

        let wallpaper_warm = p95((0..31)
            .map(|_| {
                let started = Instant::now();
                assert_eq!(
                    super::wallpaper_cache_target((3840, 2160), (2560, 1440)),
                    None
                );
                started.elapsed()
            })
            .collect());
        let wallpaper_source = RgbaImage::from_pixel(2560, 1440, Rgba([10, 20, 30, 255]));
        let wallpaper_churn = p95((0..7)
            .map(|_| {
                let started = Instant::now();
                let _ = crate::icons::resized(&wallpaper_source, 1920, 1080);
                started.elapsed()
            })
            .collect());
        let preview_source = RgbaImage::from_pixel(1280, 720, Rgba([10, 20, 30, 255]));
        let preview_churn = p95((0..11)
            .map(|_| {
                let started = Instant::now();
                let _ = super::normalize_preview_image(&preview_source);
                started.elapsed()
            })
            .collect());
        let tray_source = (0..7)
            .map(|index| TrayItem {
                id: index.to_string(),
                title: String::new(),
                icon: RgbaImage::from_pixel(512, 512, Rgba([index, 0, 0, 255])),
            })
            .collect::<Vec<_>>();
        let tray_churn = p95((0..11)
            .map(|_| {
                let started = Instant::now();
                let _ = super::normalize_tray_items(tray_source.clone());
                started.elapsed()
            })
            .collect());

        println!(
            "shell image caches wallpaper-warm-p95={}ns wallpaper-rebuild-p95={}us preview-normalize-p95={}us tray-generation-p95={}us",
            wallpaper_warm.as_nanos(),
            wallpaper_churn.as_micros(),
            preview_churn.as_micros(),
            tray_churn.as_micros()
        );
        assert!(wallpaper_churn > wallpaper_warm);
        assert!(preview_churn > wallpaper_warm);
        assert!(tray_churn > wallpaper_warm);
    }

    #[test]
    #[ignore = "release-only cache timing evidence; debug resampling is intentionally slow"]
    fn preview_image_cache_cold_warm_churn_and_low_reuse_are_measured() {
        use std::{hint::black_box, time::Duration};

        const SAMPLES: usize = 51;
        const CHURN_REUSES: usize = 8;
        const LOW_REUSE_P95_ADDITION: Duration = Duration::from_micros(500);

        fn percentile(samples: &[Duration], percentile: usize) -> Duration {
            assert!(
                samples.len() >= 20,
                "tail latency needs a useful sample set"
            );
            let mut sorted = samples.to_vec();
            sorted.sort_unstable();
            let rank = (sorted.len() * percentile).div_ceil(100);
            sorted[rank.saturating_sub(1)]
        }

        fn cached_preview(
            cache: &mut HashMap<WindowId, Arc<RgbaImage>>,
            id: WindowId,
            source: &RgbaImage,
        ) -> Arc<RgbaImage> {
            if let Some(image) = cache.get(&id) {
                return Arc::clone(image);
            }
            let image = Arc::new(super::normalize_preview_image(source));
            cache.insert(id, Arc::clone(&image));
            image
        }

        let sources = (0..SAMPLES)
            .map(|index| {
                RgbaImage::from_pixel(
                    640 + (index % 3) as u32,
                    360 + (index % 5) as u32,
                    Rgba([index as u8, 20, 30, 255]),
                )
            })
            .collect::<Vec<_>>();
        let expected = sources
            .iter()
            .map(super::normalize_preview_image)
            .collect::<Vec<_>>();

        let mut cold_cached = Vec::with_capacity(SAMPLES);
        let mut cold_bypass = Vec::with_capacity(SAMPLES);
        let mut warm_cached = Vec::with_capacity(SAMPLES);
        let mut warm_bypass = Vec::with_capacity(SAMPLES);
        let mut churn_cached = Vec::with_capacity(SAMPLES);
        let mut churn_bypass = Vec::with_capacity(SAMPLES);
        let mut low_reuse_cached = Vec::with_capacity(SAMPLES);
        let mut low_reuse_bypass = Vec::with_capacity(SAMPLES);

        for (index, source) in sources.iter().enumerate() {
            let id = WindowId(index as u64);
            let mut cache = HashMap::new();
            let started = Instant::now();
            let cached = cached_preview(&mut cache, id, black_box(source));
            cold_cached.push(started.elapsed());
            let started = Instant::now();
            let bypass = super::normalize_preview_image(black_box(source));
            cold_bypass.push(started.elapsed());
            assert_eq!(&*cached, &bypass, "cold cache changed preview pixels");

            let started = Instant::now();
            let cached = cached_preview(&mut cache, id, black_box(source));
            warm_cached.push(started.elapsed());
            let started = Instant::now();
            let bypass = super::normalize_preview_image(black_box(source));
            warm_bypass.push(started.elapsed());
            assert_eq!(&*cached, &bypass, "warm cache changed preview pixels");

            let churn_id = WindowId(10_000 + index as u64);
            cache.clear();
            let started = Instant::now();
            let mut cached = cached_preview(&mut cache, churn_id, black_box(source));
            for _ in 1..CHURN_REUSES {
                cached = cached_preview(&mut cache, churn_id, black_box(source));
            }
            churn_cached.push(started.elapsed());
            let started = Instant::now();
            let mut bypass = super::normalize_preview_image(black_box(source));
            for _ in 1..CHURN_REUSES {
                bypass = super::normalize_preview_image(black_box(source));
            }
            churn_bypass.push(started.elapsed());
            assert_eq!(&*cached, &bypass, "generation churn changed preview pixels");

            let unique_id = WindowId(20_000 + index as u64);
            let started = Instant::now();
            let cached = cached_preview(&mut cache, unique_id, black_box(source));
            low_reuse_cached.push(started.elapsed());
            let started = Instant::now();
            let bypass = super::normalize_preview_image(black_box(source));
            low_reuse_bypass.push(started.elapsed());
            assert_eq!(&*cached, &bypass, "low-reuse cache changed preview pixels");
            assert_eq!(&*cached, &expected[index]);
        }

        let cold_cached_p95 = percentile(&cold_cached, 95);
        let cold_bypass_p95 = percentile(&cold_bypass, 95);
        let warm_cached_p95 = percentile(&warm_cached, 95);
        let warm_bypass_p95 = percentile(&warm_bypass, 95);
        let churn_cached_p95 = percentile(&churn_cached, 95);
        let churn_bypass_p95 = percentile(&churn_bypass, 95);
        let low_reuse_cached_p95 = percentile(&low_reuse_cached, 95);
        let low_reuse_bypass_p95 = percentile(&low_reuse_bypass, 95);

        println!(
            "preview image cache samples={SAMPLES} cold-median={:?}/{:?} cold-p95={cold_cached_p95:?}/{cold_bypass_p95:?} warm-median={:?}/{:?} warm-p95={warm_cached_p95:?}/{warm_bypass_p95:?} churn-reuses={CHURN_REUSES} churn-median={:?}/{:?} churn-p95={churn_cached_p95:?}/{churn_bypass_p95:?} low-reuse-median={:?}/{:?} low-reuse-p95={low_reuse_cached_p95:?}/{low_reuse_bypass_p95:?}",
            percentile(&cold_cached, 50),
            percentile(&cold_bypass, 50),
            percentile(&warm_cached, 50),
            percentile(&warm_bypass, 50),
            percentile(&churn_cached, 50),
            percentile(&churn_bypass, 50),
            percentile(&low_reuse_cached, 50),
            percentile(&low_reuse_bypass, 50),
        );

        assert!(warm_cached_p95 < warm_bypass_p95);
        assert!(churn_cached_p95 < churn_bypass_p95);
        assert!(
            cold_cached_p95 <= cold_bypass_p95 + LOW_REUSE_P95_ADDITION,
            "cold insertion exceeds the predeclared 0.5 ms frame-work allowance"
        );
        assert!(
            low_reuse_cached_p95 <= low_reuse_bypass_p95 + LOW_REUSE_P95_ADDITION,
            "low-reuse insertion exceeds the predeclared 0.5 ms frame-work allowance"
        );
    }

    #[test]
    fn visible_tray_indices_address_the_last_four_items() {
        let items = (0..5)
            .map(|index| TrayItem {
                id: index.to_string(),
                title: String::new(),
                icon: RgbaImage::new(1, 1),
            })
            .collect::<Vec<_>>();

        assert_eq!(visible_tray_item(&items, 0).unwrap().id, "1");
        assert_eq!(visible_tray_item(&items, 3).unwrap().id, "4");
        assert!(visible_tray_item(&items, 4).is_none());
    }

    #[test]
    fn confirmed_audio_changes_coalesce_one_bounded_volume_osd() {
        let mut shell = LiveShell::new().unwrap();
        // The production constructor may discover the developer machine's live
        // default output. Keep this state-machine test independent of that
        // ambient device while the explicit-output case below covers labeling.
        shell.audio = AudioStatus::default();
        assert!(!shell.surface_visible(SurfaceRole::VolumeOsd));

        shell.global_shortcut(GlobalShortcut::AudioChanged {
            available: true,
            volume_percent: 47,
            muted: false,
            output_name: None,
        });
        let first_deadline = shell.volume_osd_until.unwrap();
        assert!(shell.surface_visible(SurfaceRole::VolumeOsd));
        shell.volume_osd_scene(320, 88);
        assert!(
            shell
                .volume_osd_host
                .application()
                .label
                .starts_with("Volume 47%")
        );
        assert!(
            shell
                .volume_osd_host
                .accessibility_nodes()
                .iter()
                .any(|node| {
                    node.semantic_role == Some(SemanticRole::Status)
                        && node
                            .label
                            .as_deref()
                            .is_some_and(|label| label.starts_with("Volume 47%"))
                })
        );

        shell.global_shortcut(GlobalShortcut::AudioChanged {
            available: true,
            volume_percent: 47,
            muted: true,
            output_name: None,
        });
        assert!(shell.volume_osd_until.unwrap() >= first_deadline);
        shell.volume_osd_scene(320, 88);
        assert!(
            shell
                .volume_osd_host
                .application()
                .label
                .starts_with("Muted")
        );

        let outcome = shell.poll_deadlines(Instant::now() + Duration::from_secs(2));
        assert!(outcome.visibility_changed);
        assert!(!shell.surface_visible(SurfaceRole::VolumeOsd));

        shell.global_shortcut(GlobalShortcut::AudioChanged {
            available: false,
            volume_percent: 0,
            muted: false,
            output_name: None,
        });
        assert!(!shell.surface_visible(SurfaceRole::VolumeOsd));

        shell.global_shortcut(GlobalShortcut::LockState { locked: true });
        shell.global_shortcut(GlobalShortcut::AudioChanged {
            available: true,
            volume_percent: 47,
            muted: false,
            output_name: Some("Private Bluetooth Headset".into()),
        });
        shell.volume_osd_scene(320, 88);
        assert_eq!(
            shell.volume_osd_host.application().label,
            "Volume 47% · Audio output"
        );
        assert!(
            !shell
                .volume_osd_host
                .application()
                .label
                .contains("Private")
        );
    }

    #[test]
    fn desktop_surface_projects_only_its_output_and_has_no_idle_tile_backgrounds() {
        use std::{ffi::OsString, path::PathBuf};
        let palette = nickel_core::theme::ThemePalette::from_appearance(Default::default());
        let mut desktop = super::DesktopApplication::fixture(None, palette);
        desktop.set_outputs(vec![
            nickel_file::desktop::DesktopOutput {
                id: "left".into(),
                work_area: nickel_file::desktop::Rect {
                    x: -400.0,
                    y: 0.0,
                    width: 400.0,
                    height: 600.0,
                },
                scale: 1.0,
            },
            nickel_file::desktop::DesktopOutput {
                id: "right".into(),
                work_area: nickel_file::desktop::Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 400.0,
                    height: 600.0,
                },
                scale: 1.5,
            },
        ]);
        desktop.layout.reconcile(vec![(
            nickel_file::FileIdentity(1, 2),
            nickel_file::FileEntry {
                name: OsString::from("document.txt"),
                path: PathBuf::from("/desktop/document.txt"),
                is_directory: false,
                size: Some(12),
                modified: None,
            },
        )]);
        desktop.set_active_output(
            "left".into(),
            nickel_file::desktop::Point { x: -400.0, y: 0.0 },
            1.0,
        );
        let _ = desktop.prepare_icons();
        let mut host = UiHost::new(desktop, 400, 600);
        let entry = host
            .accessibility_nodes()
            .iter()
            .find(|node| node.semantic_role == Some(SemanticRole::GridCell))
            .expect("desktop entry belongs to the active output");
        assert_eq!(entry.label.as_deref(), Some("document.txt"));
        host.application_mut().set_active_output(
            "right".into(),
            nickel_file::desktop::Point { x: 0.0, y: 0.0 },
            1.5,
        );
        host.step(HostBatch {
            application_changed: true,
            ..HostBatch::default()
        });
        assert!(
            host.accessibility_nodes()
                .iter()
                .all(|node| node.semantic_role != Some(SemanticRole::GridCell))
        );
        host.application_mut().set_active_output(
            "left".into(),
            nickel_file::desktop::Point { x: -400.0, y: 0.0 },
            1.0,
        );
        let _ = host.application_mut().prepare_icons();
        host.step(HostBatch {
            application_changed: true,
            ..HostBatch::default()
        });
        let entry = host
            .accessibility_nodes()
            .iter()
            .find(|node| node.semantic_role == Some(SemanticRole::GridCell))
            .expect("desktop entry belongs to Nickel UI semantic authority");
        assert_eq!(entry.label.as_deref(), Some("document.txt"));
        assert_eq!(entry.rect.size.width, 96.0);
        assert!(entry.actions.contains(&ActionKind::Activate));
        assert!(entry.actions.contains(&ActionKind::ContextMenu));
    }

    #[test]
    fn desktop_live_input_rebuilds_selection_and_keyboard_navigation() {
        use std::{ffi::OsString, path::PathBuf};
        let mut shell = LiveShell::new().unwrap();
        let application = shell.desktop_host.application_mut();
        application.set_outputs(vec![nickel_file::desktop::DesktopOutput {
            id: "primary".into(),
            work_area: nickel_file::desktop::Rect {
                x: 0.0,
                y: 0.0,
                width: 400.0,
                height: 600.0,
            },
            scale: 1.0,
        }]);
        application.set_active_output(
            "primary".into(),
            nickel_file::desktop::Point::default(),
            1.0,
        );
        application.layout.reconcile(
            ["first.txt", "second.txt"]
                .into_iter()
                .enumerate()
                .map(|(index, name)| {
                    (
                        nickel_file::FileIdentity(41, index as u64 + 1),
                        nickel_file::FileEntry {
                            name: OsString::from(name),
                            path: PathBuf::from("/desktop").join(name),
                            is_directory: false,
                            size: Some(1),
                            modified: None,
                        },
                    )
                })
                .collect(),
        );
        let before_commands = shell.scene(SurfaceRole::Desktop, 400, 600);
        let before_token = shell.desktop_change_token;

        assert!(shell.desktop_input(nickel_input::InputEvent::Pointer(
            nickel_input::PointerEvent::Button {
                device: nickel_input::DeviceId(1),
                order: nickel_input::EventOrder(1),
                button: nickel_input::PointerButton::Primary,
                edge: nickel_input::KeyEdge::Pressed,
                position: Some(nickel_input::Point { x: 4.0, y: 4.0 }),
            },
        )));
        assert_eq!(
            shell.desktop_host.application().layout.active(),
            Some(nickel_file::desktop::DesktopEntryId(
                nickel_file::FileIdentity(41, 1)
            ))
        );
        assert_ne!(shell.desktop_change_token, before_token);
        assert_ne!(shell.desktop_host.commands(), before_commands);

        let pointer_token = shell.desktop_change_token;
        assert!(
            shell.desktop_input(nickel_input::InputEvent::Key(nickel_input::KeyEvent {
                device: nickel_input::DeviceId(1),
                order: nickel_input::EventOrder(2),
                physical: nickel_input::PhysicalKey::Code(KeyCode::ArrowDown),
                logical: nickel_input::LogicalKey::Named(nickel_input::NamedKey::ArrowDown),
                location: nickel_input::KeyLocation::Standard,
                edge: nickel_input::KeyEdge::Pressed,
                repeat: false,
                modifiers: nickel_input::ModifierState::default(),
            },))
        );
        assert_eq!(
            shell.desktop_host.application().layout.active(),
            Some(nickel_file::desktop::DesktopEntryId(
                nickel_file::FileIdentity(41, 2)
            ))
        );
        assert_ne!(shell.desktop_change_token, pointer_token);
    }

    #[test]
    fn desktop_secondary_press_opens_overlay_without_hiding_items_on_release_or_motion() {
        let mut shell = LiveShell::new().unwrap();
        let point = nickel_input::Point { x: 300.0, y: 300.0 };
        let button = |edge, order| {
            nickel_input::InputEvent::Pointer(nickel_input::PointerEvent::Button {
                device: nickel_input::DeviceId(1),
                order: nickel_input::EventOrder(order),
                button: nickel_input::PointerButton::Secondary,
                edge,
                position: Some(point),
            })
        };

        assert!(shell.desktop_host.application().layout.icons_visible());
        assert!(shell.desktop_input(button(nickel_input::KeyEdge::Pressed, 1)));
        assert!(shell.desktop_host.application().context_menu.is_some());
        assert!(shell.desktop_host.inspect().open_overlay.is_some());

        let _ = shell.desktop_input(button(nickel_input::KeyEdge::Released, 2));
        let _ = shell.desktop_input(nickel_input::InputEvent::Pointer(
            nickel_input::PointerEvent::Motion {
                device: nickel_input::DeviceId(1),
                order: nickel_input::EventOrder(3),
                position: nickel_input::Point { x: 300.0, y: 260.0 },
                delta: Some(nickel_input::Vector { x: 0.0, y: -40.0 }),
            },
        ));

        assert!(shell.desktop_host.application().layout.icons_visible());
        assert!(shell.desktop_host.application().context_menu.is_some());
        assert!(shell.desktop_host.inspect().open_overlay.is_some());
    }

    #[test]
    fn file_like_surfaces_share_the_file_plane_item_authority() {
        let file = include_str!("../../nickel-file/src/components.rs");
        let launcher = include_str!("launcher_view.rs");
        let desktop = include_str!("live_shell.rs");
        let desktop_production = desktop
            .split("\nmod tests {")
            .next()
            .expect("production source precedes tests");
        assert!(file.contains("FileGridItem::new_with_generation"));
        assert!(launcher.matches("FilePlaneItem::new").count() >= 2);
        assert!(desktop_production.contains("FilePlaneItem::new_with_generation"));
        let shared = include_str!("../../nickel-ui/src/ui/components.rs");
        assert!(shared.contains("pub struct FilePlaneItem"));
        assert!(shared.contains("fn from_image(message: Message"));
        assert!(shared.contains("Self::from_image(message, label"));
        assert!(shared.contains(".message(message)"));
        assert!(shared.contains("pub fn context_message"));
        assert!(shared.contains(".semantic_role(SemanticRole::Button)"));
    }

    #[test]
    fn desktop_icon_secondary_hit_takes_precedence_and_background_preserves_selection() {
        use std::{ffi::OsString, path::PathBuf};
        let palette = nickel_core::theme::ThemePalette::from_appearance(Default::default());
        let mut desktop = super::DesktopApplication::fixture(None, palette);
        desktop.layout.reconcile(vec![(
            nickel_file::FileIdentity(9, 2),
            nickel_file::FileEntry {
                name: OsString::from("entry.txt"),
                path: PathBuf::from("/desktop/entry.txt"),
                is_directory: false,
                size: Some(2),
                modified: None,
            },
        )]);
        let item = desktop.layout.items()[0].clone();
        let local = nickel_file::desktop::Point {
            x: item.position.x + 4.0,
            y: item.position.y + 4.0,
        };
        desktop.pointer_press(local, true, Default::default());
        assert_eq!(desktop.context_menu.as_ref().unwrap().entry, Some(item.id));
        assert!(desktop.layout.selected().contains(&item.id));

        desktop.pointer_press(
            nickel_file::desktop::Point { x: 700.0, y: 500.0 },
            true,
            Default::default(),
        );
        assert_eq!(desktop.context_menu.as_ref().unwrap().entry, None);
        assert!(desktop.layout.selected().contains(&item.id));
    }

    #[test]
    fn desktop_drag_accumulates_small_motion_until_crossing_the_snap_threshold() {
        use std::{ffi::OsString, path::PathBuf};
        let palette = nickel_core::theme::ThemePalette::from_appearance(Default::default());
        let mut desktop = super::DesktopApplication::fixture(None, palette);
        desktop.layout.reconcile(vec![(
            nickel_file::FileIdentity(12, 3),
            nickel_file::FileEntry {
                name: OsString::from("drag-me.txt"),
                path: PathBuf::from("/desktop/drag-me.txt"),
                is_directory: false,
                size: Some(3),
                modified: None,
            },
        )]);
        let item = desktop.layout.items()[0].clone();
        let start = nickel_file::desktop::Point {
            x: item.position.x + 4.0,
            y: item.position.y + 4.0,
        };
        assert!(desktop.pointer_press(start, false, Default::default()));

        for offset in [10.0, 20.0, 30.0, 40.0, 49.0] {
            let _ = desktop.pointer_motion(nickel_file::desktop::Point {
                x: start.x + offset,
                y: start.y,
            });
        }

        assert_eq!(
            desktop.layout.items()[0].position.x,
            item.position.x + desktop.layout.grid().0,
            "sub-threshold motion events must not be discarded individually"
        );
        let _ = desktop.pointer_motion(nickel_file::desktop::Point {
            x: start.x + 97.0,
            y: start.y,
        });
        assert_eq!(
            desktop.layout.items()[0].position.x,
            item.position.x + desktop.layout.grid().0,
            "crossing one cell must not double-count the next half-cell"
        );
        let _ = desktop.pointer_motion(nickel_file::desktop::Point {
            x: start.x + 145.0,
            y: start.y,
        });
        assert_eq!(
            desktop.layout.items()[0].position.x,
            item.position.x + desktop.layout.grid().0 * 2.0,
        );
        assert!(desktop.pointer_release(
            nickel_file::desktop::Point {
                x: start.x + 145.0,
                y: start.y,
            },
            Instant::now(),
        ));
    }

    #[test]
    fn desktop_background_menu_captures_pointer_and_uses_shared_accessible_overlay() {
        let palette = nickel_core::theme::ThemePalette::from_appearance(Default::default());
        let mut desktop = super::DesktopApplication::fixture(None, palette);
        let anchor = nickel_file::desktop::Point { x: 372.0, y: 214.0 };
        assert!(desktop.pointer_press(anchor, true, Default::default()));
        desktop.pointer_motion(nickel_file::desktop::Point { x: 40.0, y: 60.0 });

        let overlays = desktop.frame_overlays(ViewContext::new(
            Rect::new(0.0, 0.0, 800.0, 600.0),
            InputModality::Pointer,
        ));
        let menu = overlays
            .into_iter()
            .find_map(|overlay| match overlay {
                FrameOverlay::Menu(menu) => Some(menu),
                _ => None,
            })
            .expect("background context menu");
        assert!(
            matches!(menu.anchor, OverlayAnchor::Point { point, .. } if point == Point { x: 372.0, y: 214.0 })
        );
        assert!(menu.items.iter().any(|item| item.label == "Personalize"));
        assert!(
            menu.items
                .iter()
                .any(|item| item.label == "Display Settings")
        );
        assert!(menu.items.iter().any(|item| item.label == "Refresh"));
        assert!(menu.row_height * menu.items.len() as f32 <= 600.0);
        assert_ne!(menu.background, 0x000000);
        assert_ne!(menu.border, menu.background);
        assert!(
            menu.items
                .iter()
                .all(|item| item.accessible_name.is_some() || !item.label.is_empty())
        );

        let host = UiHost::new(desktop, 800, 600);
        let root = host
            .accessibility_nodes()
            .iter()
            .find(|node| node.label.as_deref() == Some("Desktop"))
            .unwrap();
        assert!(root.actions.contains(&ActionKind::ContextMenu));
    }

    #[test]
    fn desktop_background_menu_uses_keyboard_controller_and_accessibility_routes() {
        let palette = nickel_core::theme::ThemePalette::from_appearance(Default::default());
        let mut desktop = super::DesktopApplication::fixture(None, palette);
        assert!(desktop.key(&nickel_input::KeyEvent {
            device: nickel_input::DeviceId(1),
            order: nickel_input::EventOrder(1),
            physical: nickel_input::PhysicalKey::Code(KeyCode::ContextMenu),
            logical: nickel_input::LogicalKey::Named(nickel_input::NamedKey::ContextMenu),
            location: nickel_input::KeyLocation::Standard,
            edge: nickel_input::KeyEdge::Pressed,
            repeat: false,
            modifiers: nickel_input::ModifierState::default(),
        }));
        assert_eq!(desktop.context_menu.as_ref().unwrap().output, "primary");

        let desktop = super::DesktopApplication::fixture(None, palette);
        let mut host = UiHost::new(desktop, 800, 600);
        let root = host
            .accessibility_nodes()
            .iter()
            .find(|node| node.label.as_deref() == Some("Desktop"))
            .unwrap()
            .id
            .clone();
        let outcome = host
            .perform_accessibility_action(root, SemanticAction::Invoke(ActionKind::ContextMenu));
        assert!(outcome.failures.is_empty());
        assert!(host.application().context_menu.is_some());

        let mut desktop = super::DesktopApplication::fixture(None, palette);
        desktop.open_keyboard_context();
        assert!(
            desktop.context_menu.is_some(),
            "controller uses this command model"
        );
    }

    #[test]
    fn stale_desktop_menu_command_is_rejected_after_topology_change() {
        let palette = nickel_core::theme::ThemePalette::from_appearance(Default::default());
        let mut desktop = super::DesktopApplication::fixture(None, palette);
        desktop.open_background_context(None);
        desktop.set_outputs(vec![nickel_file::desktop::DesktopOutput {
            id: "new-output".into(),
            work_area: nickel_file::desktop::Rect {
                x: -500.0,
                y: 0.0,
                width: 500.0,
                height: 700.0,
            },
            scale: 1.5,
        }]);
        desktop.apply_desktop_command(super::DesktopCommand::IconsVisible(false));
        assert!(desktop.layout.icons_visible());
        assert!(desktop.context_menu.is_none());
    }

    #[test]
    fn desktop_presentation_commands_persist_authoritative_live_state() {
        let palette = nickel_core::theme::ThemePalette::from_appearance(Default::default());
        let mut desktop = super::DesktopApplication::fixture(None, palette);

        desktop.open_background_context(None);
        desktop.apply_desktop_command(super::DesktopCommand::IconsVisible(false));
        assert!(!desktop.layout.icons_visible());

        desktop.open_background_context(None);
        desktop.apply_desktop_command(super::DesktopCommand::IconSize(128.0, 144.0));
        assert_eq!(desktop.layout.grid(), (128.0, 144.0));

        desktop.open_background_context(None);
        desktop.apply_desktop_command(super::DesktopCommand::Sort(
            nickel_file::desktop::SortKey::Size,
            nickel_file::desktop::SortDirection::Descending,
        ));
        assert_eq!(
            desktop.layout.arrangement(),
            nickel_file::desktop::Arrangement::Sorted {
                key: nickel_file::desktop::SortKey::Size,
                direction: nickel_file::desktop::SortDirection::Descending,
            }
        );

        desktop.open_background_context(None);
        desktop.apply_desktop_command(super::DesktopCommand::Manual);
        assert_eq!(
            desktop.layout.arrangement(),
            nickel_file::desktop::Arrangement::Manual
        );

        desktop.open_background_context(None);
        desktop.apply_desktop_command(super::DesktopCommand::FolderGrouping(
            nickel_file::desktop::FolderGrouping::Mixed,
        ));
        assert_eq!(
            desktop.layout.folder_grouping(),
            nickel_file::desktop::FolderGrouping::Mixed
        );
    }

    #[test]
    fn desktop_menu_rejects_selection_workspace_and_output_staleness() {
        use std::{ffi::OsString, path::PathBuf};
        let palette = nickel_core::theme::ThemePalette::from_appearance(Default::default());
        let mut desktop = super::DesktopApplication::fixture(None, palette);
        desktop.layout.reconcile(vec![(
            nickel_file::FileIdentity(44, 1),
            nickel_file::FileEntry {
                name: OsString::from("selected.txt"),
                path: PathBuf::from("/desktop/selected.txt"),
                is_directory: false,
                size: Some(1),
                modified: None,
            },
        )]);
        desktop.open_background_context(None);
        desktop.layout.select(
            nickel_file::desktop::DesktopEntryId(nickel_file::FileIdentity(44, 1)),
            Default::default(),
        );
        desktop.apply_desktop_command(super::DesktopCommand::IconsVisible(false));
        assert!(desktop.layout.icons_visible());
        assert!(desktop.context_menu.is_none());

        desktop.open_background_context(None);
        desktop.set_workspace(Some(2));
        assert!(desktop.context_menu.is_none());

        desktop.open_background_context(None);
        desktop.set_active_output("other".into(), nickel_file::desktop::Point::default(), 1.0);
        assert!(desktop.context_menu.is_none());
    }

    #[test]
    fn desktop_settings_destinations_are_typed_and_keep_the_invoking_output() {
        assert_eq!(
            super::SettingsDestination::Appearance.arguments(),
            ["--screen", "appearance"]
        );
        assert_eq!(
            super::SettingsDestination::Display {
                output: "DP-2".into()
            }
            .arguments(),
            ["--screen", "display", "--output", "DP-2"]
        );
    }
}
