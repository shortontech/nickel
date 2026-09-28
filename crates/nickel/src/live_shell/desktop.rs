use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use nickel_core::{shell_settings::ShellSettings, theme::ThemePalette};
use nickel_file::{
    DirectoryBrowser, DirectoryWatch,
    desktop::{
        Arrangement as DesktopArrangement, DesktopContextAction, DesktopEntryId, DesktopFileAction,
        DesktopLayout, DesktopOutput, FolderGrouping, Point as DesktopPoint, Rect as DesktopRect,
        SelectionModifiers, SortDirection as DesktopSortDirection, SortKey as DesktopSortKey,
    },
};
use nickel_input::KeyCode;
use nickel_ui::{Container, FrameOverlay, Layer, Point, Rect, SemanticRole, Size, ViewContext};

use super::desktop_label_foreground;
use crate::file_window_host::FileWindowHost;
use crate::plugin_panel::DesktopBackgroundAction;

pub struct DesktopApplication {
    pub(super) wallpaper: Option<Arc<image::RgbaImage>>,
    pub(super) plugin_background: bool,
    pub(super) wallpaper_generation: u64,
    pub(super) palette: ThemePalette,
    pub(super) browser: Option<DirectoryBrowser>,
    pub(super) watch: Option<DirectoryWatch>,
    pub(super) layout: DesktopLayout,
    pub(super) active_output: String,
    pub(super) output_origin: DesktopPoint,
    /// Horizontal projection into uniquely placed cells beyond the visible work area.
    /// This is transient viewport state and is never persisted as item geometry.
    pub(super) overflow_offsets: HashMap<String, f32>,
    pub(super) active_scale: f32,
    pub(super) icon_cache: HashMap<std::path::PathBuf, Arc<image::RgbaImage>>,
    pub(super) pointer_down: Option<(DesktopEntryId, DesktopPoint)>,
    pub(super) selection_start: Option<DesktopPoint>,
    pub(super) pointer_position: DesktopPoint,
    pub(super) pointer_seen: bool,
    pub(super) pointer_dragged: bool,
    pub(super) last_click: Option<(DesktopEntryId, Instant)>,
    pub(super) pending_plugin_open: Option<DesktopEntryId>,
    pub(super) pending_plugin_select: Option<(DesktopEntryId, SelectionModifiers)>,
    pub(super) pending_plugin_move: Option<(DesktopEntryId, DesktopPoint)>,
    pub(super) pending_plugin_file_action: Option<(DesktopEntryId, DesktopContextAction)>,
    pub(super) modifiers: SelectionModifiers,
    pub(super) context_menu: Option<DesktopMenuContext>,
    pub(super) last_menu_dismissal: Option<DesktopMenuDismissal>,
    pub(super) topology_generation: u64,
    pub(super) directory_generation: u64,
    pub(super) outputs: Vec<DesktopOutput>,
    pub(super) workspace: Option<u64>,
    pub(super) persist_layout: bool,
    pub(super) operation_tx: std::sync::mpsc::Sender<Result<String, String>>,
    pub(super) operation_rx: std::sync::mpsc::Receiver<Result<String, String>>,
    pub(super) paste_in_progress: bool,
    pub(super) file_clipboard_available: bool,
    #[cfg(target_os = "linux")]
    pending_launches: nickel_file::FileLaunches<()>,
    pub(super) error: Option<String>,
    pub(super) file_window_host: Arc<dyn FileWindowHost>,
}

pub(super) struct DesktopViewportState {
    active_output: String,
    output_origin: DesktopPoint,
    active_scale: f32,
    pointer_down: Option<(DesktopEntryId, DesktopPoint)>,
    selection_start: Option<DesktopPoint>,
    pointer_position: DesktopPoint,
    pointer_seen: bool,
    pointer_dragged: bool,
}

impl DesktopViewportState {
    pub(super) fn new(
        active_output: String,
        output_origin: DesktopPoint,
        active_scale: f32,
    ) -> Self {
        Self {
            active_output,
            output_origin,
            active_scale: active_scale.max(1.0),
            pointer_down: None,
            selection_start: None,
            pointer_position: DesktopPoint::default(),
            pointer_seen: false,
            pointer_dragged: false,
        }
    }

    pub(super) fn set_projection(
        &mut self,
        active_output: String,
        output_origin: DesktopPoint,
        active_scale: f32,
    ) {
        self.active_output = active_output;
        self.output_origin = output_origin;
        self.active_scale = active_scale.max(1.0);
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct DesktopMenuContext {
    pub(super) anchor: Option<DesktopPoint>,
    pub(super) entry: Option<DesktopEntryId>,
    pub(super) output: String,
    pub(super) topology_generation: u64,
    pub(super) directory_generation: u64,
    pub(super) selection: std::collections::HashSet<DesktopEntryId>,
    pub(super) workspace: Option<u64>,
    pub(super) paste_available: bool,
    pub(super) desktop_writable: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DesktopMenuDismissReason {
    Action,
    OutsidePress,
    Cancel,
    Replacement,
    TargetInvalidated,
    OutputRemoved,
    FocusDeparted,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct DesktopMenuDismissal {
    pub(super) output: String,
    pub(super) topology_generation: u64,
    pub(super) directory_generation: u64,
    pub(super) reason: DesktopMenuDismissReason,
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
pub(super) enum SettingsDestination {
    Appearance,
    Display { output: String },
}

impl SettingsDestination {
    pub(super) fn arguments(&self) -> Vec<String> {
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
    OpenTerminal(DesktopEntryId),
    BackgroundContext,
    Command(DesktopCommand),
}

impl DesktopApplication {
    pub(super) fn new(
        wallpaper: Option<Arc<image::RgbaImage>>,
        palette: ThemePalette,
        file_window_host: Arc<dyn FileWindowHost>,
    ) -> Self {
        let (operation_tx, operation_rx) = std::sync::mpsc::channel();
        let path = nickel_file::desktop_directory();
        let browser = DirectoryBrowser::open(&path).ok();
        let mut layout = DesktopLayout::new(Vec::new());
        if let Some(browser) = &browser {
            layout.reconcile(desktop_snapshot(browser));
            let _ = layout.restore(desktop_layout_path());
        }
        Self {
            wallpaper,
            plugin_background: false,
            wallpaper_generation: 0,
            palette,
            browser,
            watch: None,
            layout,
            active_output: "primary".into(),
            output_origin: DesktopPoint::default(),
            overflow_offsets: HashMap::new(),
            active_scale: 1.0,
            icon_cache: HashMap::new(),
            pointer_down: None,
            selection_start: None,
            pointer_position: DesktopPoint::default(),
            pointer_seen: false,
            pointer_dragged: false,
            last_click: None,
            pending_plugin_open: None,
            pending_plugin_select: None,
            pending_plugin_move: None,
            pending_plugin_file_action: None,
            modifiers: SelectionModifiers::default(),
            context_menu: None,
            last_menu_dismissal: None,
            topology_generation: 0,
            directory_generation: 0,
            outputs: Vec::new(),
            workspace: None,
            persist_layout: true,
            operation_tx,
            operation_rx,
            paste_in_progress: false,
            file_clipboard_available: false,
            #[cfg(target_os = "linux")]
            pending_launches: Default::default(),
            error: None,
            file_window_host,
        }
    }

    pub(super) fn refresh_directory(&mut self, force: bool) -> bool {
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
        let previous_entries = self
            .layout
            .items()
            .iter()
            .map(|item| {
                (
                    item.entry.path.clone(),
                    desktop_entry_fingerprint(&item.entry),
                )
            })
            .collect::<HashMap<_, _>>();
        match browser.refresh() {
            Ok(()) => {
                let snapshot = desktop_snapshot(browser);
                let current_entries = snapshot
                    .iter()
                    .map(|(_, entry)| (entry.path.clone(), desktop_entry_fingerprint(entry)))
                    .collect::<HashMap<_, _>>();
                self.layout.reconcile(snapshot);
                self.directory_generation = self.directory_generation.wrapping_add(1);
                if let Some(menu) = &mut self.context_menu {
                    let target_survives = menu
                        .entry
                        .is_none_or(|id| self.layout.items().iter().any(|item| item.id == id));
                    if target_survives {
                        menu.directory_generation = self.directory_generation;
                    } else {
                        self.dismiss_context_menu(DesktopMenuDismissReason::TargetInvalidated);
                    }
                }
                retain_unchanged_desktop_icons(
                    &mut self.icon_cache,
                    &previous_entries,
                    &current_entries,
                );
                self.error = None;
                true
            }
            Err(error) => {
                self.error = Some(format!("Desktop could not be refreshed: {error}"));
                true
            }
        }
    }

    pub(super) fn set_outputs(&mut self, outputs: Vec<DesktopOutput>) {
        if !self.layout.set_outputs(outputs) {
            return;
        }
        let outputs = self.layout.outputs().to_vec();
        if self.outputs == outputs {
            return;
        }
        self.topology_generation = self.topology_generation.wrapping_add(1);
        let menu_output_exists = self
            .context_menu
            .as_ref()
            .is_none_or(|menu| outputs.iter().any(|output| output.id == menu.output));
        self.outputs.clone_from(&outputs);
        self.overflow_offsets
            .retain(|output, _| self.outputs.iter().any(|candidate| &candidate.id == output));
        for output in self
            .outputs
            .iter()
            .map(|output| output.id.clone())
            .collect::<Vec<_>>()
        {
            self.clamp_overflow_offset(&output);
        }
        if menu_output_exists {
            if let Some(menu) = &mut self.context_menu {
                menu.topology_generation = self.topology_generation;
            }
        } else {
            self.dismiss_context_menu(DesktopMenuDismissReason::OutputRemoved);
        }
    }

    pub(super) fn set_active_output(&mut self, id: String, origin: DesktopPoint, scale: f32) {
        // This is a viewport projection used while rendering or dispatching an
        // event for one native desktop surface. It is not a user interaction
        // and must never change the lifetime of a menu owned by another output.
        self.active_output = id;
        self.output_origin = origin;
        self.active_scale = scale.max(1.0);
    }

    pub(super) fn replace_viewport_state(
        &mut self,
        viewport: DesktopViewportState,
    ) -> DesktopViewportState {
        DesktopViewportState {
            active_output: std::mem::replace(&mut self.active_output, viewport.active_output),
            output_origin: std::mem::replace(&mut self.output_origin, viewport.output_origin),
            active_scale: std::mem::replace(&mut self.active_scale, viewport.active_scale),
            pointer_down: std::mem::replace(&mut self.pointer_down, viewport.pointer_down),
            selection_start: std::mem::replace(&mut self.selection_start, viewport.selection_start),
            pointer_position: std::mem::replace(
                &mut self.pointer_position,
                viewport.pointer_position,
            ),
            pointer_seen: std::mem::replace(&mut self.pointer_seen, viewport.pointer_seen),
            pointer_dragged: std::mem::replace(&mut self.pointer_dragged, viewport.pointer_dragged),
        }
    }

    fn projection_origin(&self) -> DesktopPoint {
        DesktopPoint {
            x: self.output_origin.x
                + self
                    .overflow_offsets
                    .get(&self.active_output)
                    .copied()
                    .unwrap_or(0.0),
            y: self.output_origin.y,
        }
    }

    fn maximum_overflow_offset(&self, output_id: &str) -> f32 {
        let Some(output) = self.outputs.iter().find(|output| output.id == output_id) else {
            return 0.0;
        };
        let (cell_width, _) = self.layout.grid();
        (self
            .layout
            .items()
            .iter()
            .filter(|item| item.output == output_id)
            .map(|item| item.position.x + cell_width)
            .fold(output.work_area.x + output.work_area.width, f32::max)
            - (output.work_area.x + output.work_area.width))
            .max(0.0)
    }

    fn clamp_overflow_offset(&mut self, output_id: &str) {
        let maximum = self.maximum_overflow_offset(output_id);
        let offset = self
            .overflow_offsets
            .entry(output_id.to_owned())
            .or_default();
        *offset = offset.clamp(0.0, maximum);
    }

    pub(super) fn scroll_overflow(&mut self, delta: f32) -> bool {
        let maximum = self.maximum_overflow_offset(&self.active_output);
        if maximum <= 0.0 || !delta.is_finite() {
            return false;
        }
        let offset = self
            .overflow_offsets
            .entry(self.active_output.clone())
            .or_default();
        let previous = *offset;
        *offset = (*offset + delta).clamp(0.0, maximum);
        *offset != previous
    }

    pub(super) fn reveal_active(&mut self) -> bool {
        let Some(id) = self.layout.active() else {
            return false;
        };
        let Some(item) = self.layout.items().iter().find(|item| item.id == id) else {
            return false;
        };
        if item.output != self.active_output {
            return false;
        }
        let Some(output) = self
            .outputs
            .iter()
            .find(|output| output.id == self.active_output)
        else {
            return false;
        };
        let (cell_width, _) = self.layout.grid();
        let current = self
            .overflow_offsets
            .get(&self.active_output)
            .copied()
            .unwrap_or(0.0);
        let left = output.work_area.x + current;
        let right = left + output.work_area.width;
        let desired = if item.position.x < left {
            item.position.x - output.work_area.x
        } else if item.position.x + cell_width > right {
            item.position.x + cell_width - output.work_area.x - output.work_area.width
        } else {
            current
        };
        let maximum = self.maximum_overflow_offset(&self.active_output);
        let desired = desired.clamp(0.0, maximum);
        if desired == current {
            false
        } else {
            self.overflow_offsets
                .insert(self.active_output.clone(), desired);
            true
        }
    }

    pub(super) fn set_workspace(&mut self, workspace: Option<u64>) {
        if self.workspace != workspace {
            self.workspace = workspace;
            self.dismiss_context_menu(DesktopMenuDismissReason::TargetInvalidated);
        }
    }

    pub(super) fn save_layout(&mut self) {
        if !self.persist_layout {
            return;
        }
        if let Err(error) = self.layout.save(desktop_layout_path()) {
            self.error = Some(format!("Desktop arrangement could not be saved: {error}"));
        }
    }

    pub(super) fn hit(&self, local: DesktopPoint) -> Option<DesktopEntryId> {
        if !self.layout.icons_visible() {
            return None;
        }
        let (cell_width, cell_height) = self.layout.grid();
        let origin = self.projection_origin();
        let global = DesktopPoint {
            x: local.x + origin.x,
            y: local.y + origin.y,
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

    pub(super) fn pointer_press(
        &mut self,
        local: DesktopPoint,
        secondary: bool,
        modifiers: SelectionModifiers,
    ) -> bool {
        self.pointer_press_with_selection(local, secondary, modifiers, false)
    }

    pub(super) fn pointer_press_for_plugin(
        &mut self,
        local: DesktopPoint,
        modifiers: SelectionModifiers,
    ) -> bool {
        self.pointer_press_with_selection(local, false, modifiers, true)
    }

    fn pointer_press_with_selection(
        &mut self,
        local: DesktopPoint,
        secondary: bool,
        modifiers: SelectionModifiers,
        defer_selection: bool,
    ) -> bool {
        self.pointer_position = local;
        self.pointer_seen = true;
        self.pending_plugin_select = None;
        self.pending_plugin_move = None;
        self.pending_plugin_file_action = None;
        let hit = self.hit(local);
        if secondary {
            self.replacing_context_menu();
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
                paste_available: hit.is_none() && self.file_clipboard_available,
                desktop_writable: hit.is_none()
                    && nickel_file::directory_is_writable(&nickel_file::desktop_directory()),
            });
            return true;
        }
        self.dismiss_context_menu(DesktopMenuDismissReason::OutsidePress);
        if let Some(id) = hit {
            if defer_selection {
                self.pending_plugin_select = Some((id, modifiers));
            } else {
                self.layout.select(id, modifiers);
            }
            self.pointer_down = Some((id, local));
            self.selection_start = None;
            self.pointer_dragged = false;
        } else {
            if !modifiers.toggle {
                self.layout.clear_selection();
            }
            self.pointer_down = None;
            self.selection_start = Some(local);
        }
        true
    }

    pub(super) fn pointer_motion(&mut self, local: DesktopPoint) -> bool {
        self.pointer_position = local;
        self.pointer_seen = true;
        let Some((_, pressed)) = self.pointer_down else {
            let Some(start) = self.selection_start else {
                return false;
            };
            let origin = self.projection_origin();
            let x = start.x.min(local.x) + origin.x;
            let y = start.y.min(local.y) + origin.y;
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
        if !self.pointer_dragged
            && nickel_file::file_drag_offset(
                Point {
                    x: pressed.x,
                    y: pressed.y,
                },
                Point {
                    x: local.x,
                    y: local.y,
                },
            )
            .is_none()
        {
            return false;
        }
        // Motion is a transient preview, not a layout transaction. Snapping,
        // collision resolution and persistence happen once, on release.
        self.pointer_dragged = true;
        true
    }

    pub(super) fn pointer_release(&mut self, local: DesktopPoint, now: Instant) -> bool {
        self.pointer_release_with_move(local, now, false)
    }

    pub(super) fn pointer_release_for_plugin(&mut self, local: DesktopPoint, now: Instant) -> bool {
        self.pointer_release_with_move(local, now, true)
    }

    fn pointer_release_with_move(
        &mut self,
        local: DesktopPoint,
        now: Instant,
        defer_move: bool,
    ) -> bool {
        self.pending_plugin_move = None;
        if self.selection_start.take().is_some() {
            return true;
        }
        let Some((id, pressed)) = self.pointer_down.take() else {
            return false;
        };
        let moved = self.pointer_dragged
            || nickel_file::file_drag_offset(
                Point {
                    x: pressed.x,
                    y: pressed.y,
                },
                Point {
                    x: local.x,
                    y: local.y,
                },
            )
            .is_some();
        self.pointer_dragged = false;
        if moved {
            let delta = DesktopPoint {
                x: local.x - pressed.x,
                y: local.y - pressed.y,
            };
            if defer_move {
                self.pending_plugin_move = Some((id, delta));
            } else {
                self.commit_move(id, delta);
            }
            self.last_click = None;
        } else if self.last_click.is_some_and(|(last, at)| {
            last == id && now.duration_since(at) <= Duration::from_millis(500)
        }) {
            self.last_click = None;
            self.request_activate(id);
        } else {
            self.last_click = Some((id, now));
        }
        true
    }

    pub(super) fn commit_move(&mut self, id: DesktopEntryId, delta: DesktopPoint) {
        self.layout.move_group(id, delta, &self.active_output);
        self.save_layout();
    }

    pub(super) fn cancel_pointer_transaction(&mut self) -> bool {
        let changed = self.pointer_down.take().is_some() || self.selection_start.take().is_some();
        self.pointer_dragged = false;
        self.pending_plugin_select = None;
        self.pending_plugin_move = None;
        self.pending_plugin_file_action = None;
        changed
    }

    pub(super) fn activate(&mut self, id: DesktopEntryId) {
        if let Some(action) = self.layout.activate(id) {
            let result = match action {
                DesktopFileAction::Browse(path) => {
                    self.file_window_host
                        .dispatch(nickel_file::FileWindowRequest::OpenOrFocus(
                            nickel_file::FileLaunch::Browse(path),
                        ))
                }
                DesktopFileAction::Open(path) => {
                    #[cfg(target_os = "linux")]
                    {
                        self.start_document_launch(path, nickel_file::spawn_open_path)
                    }
                    #[cfg(not(target_os = "linux"))]
                    {
                        nickel_file::open_path(&path)
                    }
                }
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

    pub(super) fn request_activate(&mut self, id: DesktopEntryId) {
        if self.plugin_background {
            self.pending_plugin_open = Some(id);
        } else {
            self.activate(id);
        }
    }

    #[cfg(target_os = "linux")]
    fn start_document_launch(
        &mut self,
        path: std::path::PathBuf,
        spawn: impl FnOnce(&std::path::Path) -> Result<std::process::Child, String>,
    ) -> Result<(), String> {
        self.pending_launches.start_with(path, (), spawn)
    }

    #[cfg(target_os = "linux")]
    fn poll_document_launches(&mut self) -> bool {
        let mut changed = false;
        for (path, (), result) in self.pending_launches.poll() {
            if let Err(error) = result {
                self.error = Some(format!("Could not open {}: {error}", path.display()));
                changed = true;
            }
        }
        changed
    }

    pub(super) fn open_background_context(&mut self, anchor: Option<DesktopPoint>) {
        self.replacing_context_menu();
        self.context_menu = Some(DesktopMenuContext {
            anchor,
            entry: None,
            output: self.active_output.clone(),
            topology_generation: self.topology_generation,
            directory_generation: self.directory_generation,
            selection: self.layout.selected().clone(),
            workspace: self.workspace,
            paste_available: self.file_clipboard_available,
            desktop_writable: nickel_file::directory_is_writable(&nickel_file::desktop_directory()),
        });
    }

    pub(super) fn open_keyboard_context(&mut self) {
        self.replacing_context_menu();
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
            // Replacement was already recorded above.
            self.context_menu = Some(DesktopMenuContext {
                anchor: None,
                entry: None,
                output: self.active_output.clone(),
                topology_generation: self.topology_generation,
                directory_generation: self.directory_generation,
                selection: self.layout.selected().clone(),
                workspace: self.workspace,
                paste_available: self.file_clipboard_available,
                desktop_writable: nickel_file::directory_is_writable(
                    &nickel_file::desktop_directory(),
                ),
            });
        }
    }

    pub(super) fn apply_desktop_command(&mut self, command: DesktopCommand) {
        let Some(context) = self.context_menu.clone() else {
            return;
        };
        if context.output != self.active_output
            || context.topology_generation != self.topology_generation
            || context.directory_generation != self.directory_generation
            || context.selection != *self.layout.selected()
            || context.workspace != self.workspace
        {
            self.dismiss_context_menu(DesktopMenuDismissReason::TargetInvalidated);
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
        self.dismiss_context_menu(DesktopMenuDismissReason::Action);
    }

    pub(super) fn apply_background_plugin_action(
        &mut self,
        action: DesktopBackgroundAction,
    ) -> bool {
        let Some(context) = self.context_menu.as_ref() else {
            return false;
        };
        if context.entry.is_some() || context.output != self.active_output {
            return false;
        }
        if matches!(action, DesktopBackgroundAction::Paste)
            && (!context.paste_available || self.paste_in_progress)
        {
            return false;
        }
        if matches!(action, DesktopBackgroundAction::NewFolder) && !context.desktop_writable {
            return false;
        }
        let command = match action {
            DesktopBackgroundAction::ToggleIcons => {
                DesktopCommand::IconsVisible(!self.layout.icons_visible())
            }
            DesktopBackgroundAction::SmallIcons => DesktopCommand::IconSize(72.0, 88.0),
            DesktopBackgroundAction::MediumIcons => DesktopCommand::IconSize(96.0, 112.0),
            DesktopBackgroundAction::LargeIcons => DesktopCommand::IconSize(128.0, 144.0),
            DesktopBackgroundAction::SortName => {
                DesktopCommand::Sort(DesktopSortKey::Name, DesktopSortDirection::Ascending)
            }
            DesktopBackgroundAction::SortNameDescending => {
                DesktopCommand::Sort(DesktopSortKey::Name, DesktopSortDirection::Descending)
            }
            DesktopBackgroundAction::SortKind => {
                DesktopCommand::Sort(DesktopSortKey::Kind, DesktopSortDirection::Ascending)
            }
            DesktopBackgroundAction::SortKindDescending => {
                DesktopCommand::Sort(DesktopSortKey::Kind, DesktopSortDirection::Descending)
            }
            DesktopBackgroundAction::SortSize => {
                DesktopCommand::Sort(DesktopSortKey::Size, DesktopSortDirection::Ascending)
            }
            DesktopBackgroundAction::SortSizeDescending => {
                DesktopCommand::Sort(DesktopSortKey::Size, DesktopSortDirection::Descending)
            }
            DesktopBackgroundAction::SortModified => {
                DesktopCommand::Sort(DesktopSortKey::Modified, DesktopSortDirection::Descending)
            }
            DesktopBackgroundAction::SortModifiedAscending => {
                DesktopCommand::Sort(DesktopSortKey::Modified, DesktopSortDirection::Ascending)
            }
            DesktopBackgroundAction::Manual => DesktopCommand::Manual,
            DesktopBackgroundAction::AlignGrid => DesktopCommand::AlignGrid,
            DesktopBackgroundAction::AutoArrange => DesktopCommand::AutoArrange,
            DesktopBackgroundAction::FoldersFirst => {
                DesktopCommand::FolderGrouping(FolderGrouping::FoldersFirst)
            }
            DesktopBackgroundAction::FoldersMixed => {
                DesktopCommand::FolderGrouping(FolderGrouping::Mixed)
            }
            DesktopBackgroundAction::Refresh => DesktopCommand::Refresh,
            DesktopBackgroundAction::Paste => DesktopCommand::Paste,
            DesktopBackgroundAction::NewFolder => DesktopCommand::NewFolder,
            DesktopBackgroundAction::DisplaySettings => DesktopCommand::DisplaySettings,
            DesktopBackgroundAction::Personalize => DesktopCommand::Personalize,
        };
        self.apply_desktop_command(command);
        self.context_menu.is_none()
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
                #[cfg(target_os = "linux")]
                command.env_remove("__EGL_VENDOR_LIBRARY_FILENAMES");
                command.args(destination.arguments());
                #[cfg(target_os = "linux")]
                crate::model::authorize_trusted_session_client(&mut command);
                command
                    .spawn()
                    .map(|_| ())
                    .map_err(|error| error.to_string())
            });
        if let Err(error) = result {
            self.error = Some(format!("Could not open Settings: {error}"));
        }
    }

    // Keyboard edges and native pointer snapshots share selection policy; neither
    // path should independently reinterpret Ctrl/Shift additive selection.
    pub(super) fn set_input_modifiers(&mut self, modifiers: &nickel_input::ModifierState) {
        use nickel_input::AggregateModifier;
        self.modifiers = SelectionModifiers {
            toggle: modifiers.aggregate(AggregateModifier::Control),
            range: modifiers.aggregate(AggregateModifier::Shift),
            additive_range: modifiers.aggregate(AggregateModifier::Control)
                && modifiers.aggregate(AggregateModifier::Shift),
        };
    }

    pub(super) fn key(&mut self, key: &nickel_input::KeyEvent) -> bool {
        use nickel_input::{AggregateModifier, PhysicalKey};
        self.set_input_modifiers(&key.modifiers);
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
                    self.request_activate(id);
                }
            }
            KeyCode::Escape => {
                self.dismiss_context_menu(DesktopMenuDismissReason::Cancel);
                self.layout.clear_selection();
            }
            _ => return false,
        }
        true
    }

    pub(super) fn move_selected_by(&mut self, x: f32, y: f32) {
        if let Some(id) = self.layout.active() {
            self.layout
                .move_group(id, DesktopPoint { x, y }, &self.active_output);
            self.save_layout();
        }
    }

    pub(super) fn prepare_icons(&mut self) -> bool {
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

    pub(super) fn plugin_tiles(
        &self,
        width: u32,
        height: u32,
    ) -> (Vec<serde_json::Value>, crate::plugin_panel::PluginImages) {
        let mut tiles = Vec::new();
        let mut images = crate::plugin_panel::PluginImages::new();
        if !self.layout.icons_visible() {
            return (tiles, images);
        }
        let origin = self.projection_origin();
        let hovered = self
            .pointer_seen
            .then(|| self.hit(self.pointer_position))
            .flatten();
        let (cell_width, cell_height) = self.layout.grid();
        for (index, item) in self
            .layout
            .items()
            .iter()
            .filter(|item| item.output == self.active_output)
            .enumerate()
        {
            let mut x = item.position.x - origin.x;
            let mut y = item.position.y - origin.y;
            let selected = self.layout.selected().contains(&item.id);
            if selected
                && self.pointer_dragged
                && let Some((_, pressed)) = self.pointer_down
            {
                x += self.pointer_position.x - pressed.x;
                y += self.pointer_position.y - pressed.y;
            }
            if x + cell_width < 0.0
                || y + cell_height < 0.0
                || x > width as f32
                || y > height as f32
            {
                continue;
            }
            let focused = self.layout.active() == Some(item.id);
            let active = hovered == Some(item.id) || focused;
            let interaction_surface = if selected {
                Some(self.palette.accent_soft)
            } else if active {
                Some(self.palette.surface_hover)
            } else {
                None
            };
            let foreground = desktop_label_foreground(
                self.wallpaper.as_deref(),
                Size {
                    width: width as f32,
                    height: height as f32,
                },
                Rect::new(
                    x + 3.0,
                    y + 62.0,
                    (cell_width - 6.0).max(1.0),
                    (cell_height - 74.0).max(1.0),
                ),
                self.palette.background,
                interaction_surface,
            );
            let asset = format!("desktop-icon-{index}");
            if let Some(icon) = self.icon_cache.get(&item.entry.path) {
                images.insert(
                    asset.clone(),
                    (
                        0x8000_u16.saturating_add(u16::try_from(tiles.len()).unwrap_or(u16::MAX)),
                        Arc::clone(icon),
                    ),
                );
            }
            tiles.push(serde_json::json!({
                "id": format!("{}:{}", item.id.0.0, item.id.0.1),
                "asset": asset,
                "label": item.entry.display_name(),
                "x": x.clamp(-8192.0, 8192.0),
                "y": y.clamp(-8192.0, 8192.0),
                "width": cell_width,
                "height": cell_height - 4.0,
                "selected": selected,
                "hovered": active,
                "dragging": self.pointer_dragged && selected,
                "color": foreground,
                "outline": if foreground == 0x111111 { 0xccffffff_u32 } else { 0xcc111111_u32 },
                "hoverBackground": self.palette.surface_hover,
                "selectedBackground": self.palette.accent_soft,
                "accent": self.palette.accent,
                "complement": self.palette.complement,
            }));
        }
        (tiles, images)
    }
}

type DesktopEntryFingerprint = (bool, Option<u64>, Option<std::time::SystemTime>);
fn desktop_entry_fingerprint(entry: &nickel_file::FileEntry) -> DesktopEntryFingerprint {
    (entry.is_directory, entry.size, entry.modified)
}

pub(super) fn retain_unchanged_desktop_icons(
    cache: &mut HashMap<std::path::PathBuf, Arc<image::RgbaImage>>,
    previous: &HashMap<std::path::PathBuf, DesktopEntryFingerprint>,
    current: &HashMap<std::path::PathBuf, DesktopEntryFingerprint>,
) {
    cache.retain(|path, _| {
        current
            .get(path)
            .is_some_and(|fingerprint| previous.get(path) == Some(fingerprint))
    });
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

impl DesktopApplication {
    pub(super) fn execute_context_action(
        &mut self,
        id: DesktopEntryId,
        action: DesktopContextAction,
    ) {
        match action {
            DesktopContextAction::Cut | DesktopContextAction::Copy => {
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
                if let Err(error) =
                    nickel_file::publish_file_clipboard(&paths, action == DesktopContextAction::Cut)
                {
                    self.error = Some(format!("Could not update file clipboard: {error}"));
                }
            }
            DesktopContextAction::Rename | DesktopContextAction::Properties => {
                if let Some(path) = self
                    .layout
                    .items()
                    .iter()
                    .find(|item| item.id == id)
                    .map(|item| item.entry.path.clone())
                {
                    let result = if action == DesktopContextAction::Rename {
                        self.file_window_host
                            .dispatch(nickel_file::FileWindowRequest::Open(
                                nickel_file::FileLaunch::Rename(path),
                            ))
                    } else {
                        self.file_window_host
                            .dispatch(nickel_file::FileWindowRequest::OpenOrFocus(
                                nickel_file::FileLaunch::Properties(path),
                            ))
                    };
                    if let Err(error) = result {
                        self.error = Some(error);
                    }
                }
            }
            DesktopContextAction::OpenTerminal => {
                if let Some(path) = self
                    .layout
                    .items()
                    .iter()
                    .find(|item| item.id == id)
                    .map(|item| item.entry.path.as_path())
                {
                    let directory = if path.is_dir() {
                        path
                    } else {
                        path.parent().unwrap_or(path)
                    };
                    if let Err(error) = nickel_platform::open_terminal(directory) {
                        self.error = Some(error);
                    }
                }
            }
        }
        self.dismiss_context_menu(DesktopMenuDismissReason::Action);
    }
}

impl nickel_ui::Application for DesktopApplication {
    type Message = DesktopMessage;

    fn update(&mut self, message: Self::Message) {
        match message {
            DesktopMessage::Activate(id) => self.request_activate(id),
            DesktopMessage::Context(id) => {
                if let Some(position) = self
                    .layout
                    .items()
                    .iter()
                    .find(|item| item.id == id)
                    .map(|item| item.position)
                {
                    let origin = self.projection_origin();
                    self.replacing_context_menu();
                    self.context_menu = Some(DesktopMenuContext {
                        anchor: Some(DesktopPoint {
                            x: position.x - origin.x,
                            y: position.y - origin.y,
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
            DesktopMessage::Cut(id)
            | DesktopMessage::Copy(id)
            | DesktopMessage::Rename(id)
            | DesktopMessage::Properties(id)
            | DesktopMessage::OpenTerminal(id) => {
                let action = match message {
                    DesktopMessage::Cut(_) => DesktopContextAction::Cut,
                    DesktopMessage::Copy(_) => DesktopContextAction::Copy,
                    DesktopMessage::Rename(_) => DesktopContextAction::Rename,
                    DesktopMessage::Properties(_) => DesktopContextAction::Properties,
                    DesktopMessage::OpenTerminal(_) => DesktopContextAction::OpenTerminal,
                    _ => unreachable!(),
                };
                if self.plugin_background {
                    self.pending_plugin_file_action = Some((id, action));
                    self.dismiss_context_menu(DesktopMenuDismissReason::Action);
                } else {
                    self.execute_context_action(id, action);
                }
            }
            DesktopMessage::BackgroundContext => {
                // The UI host emits this from the same secondary press that updated
                // pointer_position. Keep that anchor when replacing the menu so the
                // detached overlay opens at the actual invocation point.
                self.open_background_context(self.pointer_seen.then_some(self.pointer_position))
            }
            DesktopMessage::Command(command) => self.apply_desktop_command(command),
        }
    }

    fn frame_overlays(&self, _view_context: ViewContext) -> Vec<FrameOverlay<Self::Message>> {
        if !self.plugin_background {
            return Vec::new();
        }
        self.selection_start
            .map(|start| {
                let current = self.pointer_position;
                FrameOverlay::SelectionMarquee {
                    rect: Rect::new(
                        start.x.min(current.x),
                        start.y.min(current.y),
                        (current.x - start.x).abs(),
                        (current.y - start.y).abs(),
                    ),
                    fill: Some((0x38_u32 << 24) | (self.palette.accent_soft & 0x00ff_ffff)),
                    stroke: self.palette.accent,
                    width: 1.0,
                }
            })
            .into_iter()
            .collect()
    }

    fn view(&self, context: ViewContext) -> impl nickel_ui::View<Self::Message> {
        let width = context.viewport.size.width;
        let height = context.viewport.size.height;
        let mut layer = Layer::new().width(width).height(height);
        if self.plugin_background {
            let (cell_width, cell_height) = self.layout.grid();
            for item in self
                .layout
                .items()
                .iter()
                .filter(|item| self.layout.icons_visible() && item.output == self.active_output)
            {
                let origin = self.projection_origin();
                let mut position = Point {
                    x: item.position.x - origin.x,
                    y: item.position.y - origin.y,
                };
                let selected = self.layout.selected().contains(&item.id);
                if selected && self.pointer_dragged {
                    if let Some((_, pressed)) = self.pointer_down {
                        position.x += self.pointer_position.x - pressed.x;
                        position.y += self.pointer_position.y - pressed.y;
                    }
                }
                // Rust retains stable hit and menu-anchor IDs. JSX paints the tile.
                layer = layer.child(
                    Container::new()
                        .id(format!("desktop-entry-{}-{}", item.id.0.0, item.id.0.1))
                        .position(position)
                        .width(cell_width)
                        .height(cell_height - 4.0)
                        .message(DesktopMessage::Activate(item.id))
                        .context_message(DesktopMessage::Context(item.id))
                        .semantic_role(SemanticRole::GridCell)
                        .accessibility_label(item.entry.display_name()),
                );
            }
        }
        let root = Container::new()
            .id("desktop")
            .semantic_role(SemanticRole::ApplicationPresentation)
            .accessibility_label("Desktop")
            .width(width)
            .height(height);
        let root = if self.plugin_background {
            root.context_message(DesktopMessage::BackgroundContext)
        } else {
            root
        };
        root.child(layer)
    }

    fn title(&self) -> &str {
        "Nickel Desktop"
    }

    fn poll(&mut self) -> bool {
        if !self.plugin_background {
            return false;
        }
        let mut changed = self.refresh_directory(false);
        #[cfg(target_os = "linux")]
        {
            changed |= self.poll_document_launches();
        }
        while let Ok(result) = self.operation_rx.try_recv() {
            self.paste_in_progress = false;
            self.error = Some(result.unwrap_or_else(|error| error));
            self.refresh_directory(true);
            changed = true;
        }
        if self.plugin_background {
            changed |= self.prepare_icons();
        }
        changed
    }

    fn poll_interval(&self) -> Option<Duration> {
        self.plugin_background.then_some(Duration::from_millis(250))
    }
}
impl DesktopApplication {
    pub(super) fn dismiss_context_menu(&mut self, reason: DesktopMenuDismissReason) -> bool {
        let Some(menu) = self.context_menu.take() else {
            return false;
        };
        self.last_menu_dismissal = Some(DesktopMenuDismissal {
            output: menu.output,
            topology_generation: menu.topology_generation,
            directory_generation: menu.directory_generation,
            reason,
        });
        true
    }

    fn replacing_context_menu(&mut self) {
        self.dismiss_context_menu(DesktopMenuDismissReason::Replacement);
    }
}

#[cfg(any(test, feature = "workbench-fixtures"))]
impl DesktopApplication {
    #[allow(dead_code)] // Binary and fixture library compile this shared module separately.
    pub fn fixture(wallpaper: Option<Arc<image::RgbaImage>>, palette: ThemePalette) -> Self {
        let (operation_tx, operation_rx) = std::sync::mpsc::channel();
        Self {
            wallpaper,
            plugin_background: false,
            wallpaper_generation: 0,
            palette,
            file_window_host: crate::file_window_host::default_file_window_host(),
            browser: None,
            watch: None,
            layout: DesktopLayout::new(vec![DesktopOutput {
                id: "primary".into(),
                primary: true,
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
            overflow_offsets: HashMap::new(),
            active_scale: 1.0,
            icon_cache: HashMap::new(),
            pointer_down: None,
            selection_start: None,
            pointer_position: DesktopPoint::default(),
            pointer_seen: false,
            pointer_dragged: false,
            last_click: None,
            pending_plugin_open: None,
            pending_plugin_select: None,
            pending_plugin_move: None,
            pending_plugin_file_action: None,
            modifiers: SelectionModifiers::default(),
            context_menu: None,
            last_menu_dismissal: None,
            topology_generation: 0,
            directory_generation: 0,
            outputs: Vec::new(),
            workspace: None,
            persist_layout: false,
            operation_tx,
            operation_rx,
            paste_in_progress: false,
            file_clipboard_available: false,
            #[cfg(target_os = "linux")]
            pending_launches: Default::default(),
            error: None,
        }
    }
}
