#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

macro_rules! eprintln {
    ($($argument:tt)*) => {
        tracing::warn!($($argument)*)
    };
}

macro_rules! println {
    ($($argument:tt)*) => {
        tracing::info!($($argument)*)
    };
}

use std::{
    collections::{HashMap, HashSet},
    env,
    path::PathBuf,
    sync::{Arc, mpsc},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use glyphon::{
    Attrs, Buffer, Cache, Color, ContentType, CustomGlyph, Family, FontSystem, Metrics,
    RasterizeCustomGlyphRequest, RasterizedCustomGlyph, Resolution, Shaping, SwashCache, TextArea,
    TextAtlas, TextBounds, TextRenderer, Viewport, cosmic_text::Align,
};
use nickel_components::{TextEditor, TextField};
use nickel_core::hotkeys::{Hotkey, KeyEdge};
use nickel_core::launcher::LauncherVisibility;
use nickel_core::run::RunPrompt;
use nickel_core::shell_settings::ShellSettings;
use nickel_core::theme::{Appearance, ThemePalette};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, Ime, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::monitor::MonitorHandle;
use winit::window::{CursorIcon, Window, WindowAttributes, WindowId, WindowLevel};

mod context_menu;
mod control_center;
mod desktop;
mod graphics;
mod icons;
mod launcher;
mod layout;
mod model;
mod panel;
mod platform;
mod rectangles;
mod run_dialog;
mod screenshot;
mod storage;
mod volume_osd;

use launcher::Launcher;
use model::{OpenWindow, TrayItem, WindowGroup, WindowId as ShellWindowId};
use platform::{GlobalShortcut, ShellCommand, TrayFeed, TraySource, WindowAction, WindowFeed};

const PANEL_HEIGHT: f64 = 56.0;

#[derive(Clone, Copy)]
enum ContextAction {
    Activate(ShellWindowId),
    Close(ShellWindowId),
    Maximize(ShellWindowId),
    Minimize(ShellWindowId),
}

struct Nickel {
    desktop_surfaces: Vec<(Arc<Window>, desktop::DesktopGpu)>,
    window: Option<Arc<Window>>,
    gpu: Option<Gpu>,
    panel_surfaces: Vec<(Arc<Window>, panel::PanelGpu)>,
    panel_primary: Vec<bool>,
    shell_settings: ShellSettings,
    settings_deadline: Instant,
    display_deadline: Instant,
    display_topology: Vec<(String, i32, i32, u32, u32, u64)>,
    panel_hovered: bool,
    panel_task_hovered: Option<usize>,
    panel_tray_hovered: Option<usize>,
    panel_desktop_hovered: Option<u8>,
    panel_control_hovered: bool,
    control_center_window: Option<Arc<Window>>,
    control_center_gpu: Option<control_center::ControlCenterGpu>,
    control_center_visible: bool,
    volume_osd_window: Option<Arc<Window>>,
    volume_osd_gpu: Option<volume_osd::VolumeOsdGpu>,
    volume_osd_deadline: Option<Instant>,
    volume_osd_state: (u8, bool),
    screenshot_tool: Option<screenshot::ScreenshotTool>,
    screenshot_capture_deadline: Option<Instant>,
    context_menu_window: Option<Arc<Window>>,
    context_menu_gpu: Option<context_menu::ContextMenuGpu>,
    context_menu_hovered: Option<usize>,
    context_close_hovered: Option<usize>,
    context_menu_target: Option<ShellWindowId>,
    context_menu_actions: Vec<ContextAction>,
    context_preview_mode: bool,
    run_window: Option<Arc<Window>>,
    run_gpu: Option<run_dialog::RunDialogGpu>,
    run_prompt: RunPrompt,
    run_editor: TextEditor,
    run_modifiers: ModifiersState,
    run_ignoring_trigger_r: bool,
    run_hovered: Option<run_dialog::Action>,
    preview_group: Option<usize>,
    preview_hide_deadline: Option<Instant>,
    task_windows: Vec<OpenWindow>,
    window_groups: Vec<WindowGroup>,
    panel_glyph_ids: HashMap<String, u16>,
    next_panel_glyph_id: u16,
    unresolved_panel_icons: HashSet<String>,
    tray_items: Vec<TrayItem>,
    launcher_visibility: LauncherVisibility,
    launcher_focus_loss_deadline: Option<Instant>,
    launcher_hotkey_rx: mpsc::Receiver<GlobalShortcut>,
    task_switcher_active: bool,
    task_switcher_windows: Vec<OpenWindow>,
    clock_deadline: Instant,
    window_deadline: Instant,
    fullscreen_deadline: Instant,
    window_feed: WindowFeed,
    tray_feed: TrayFeed,
    launcher: Launcher,
    launcher_editor: TextEditor,
    hovered_result: Option<usize>,
    pin_store: Option<storage::PinStore>,
    scroll_offset: usize,
    cursor_position: Option<PhysicalPosition<f64>>,
    scrollbar_drag_offset: Option<f64>,
}

impl Default for Nickel {
    fn default() -> Self {
        let applications = platform::applications();

        let mut launcher = if applications.is_empty() {
            Launcher::default()
        } else {
            Launcher::new(applications)
        };
        let mut run_prompt = RunPrompt::default();
        let pin_store = match storage::PinStore::open_default() {
            Ok(store) => {
                match store.pins() {
                    Ok(pins) => launcher.set_pins(pins),
                    Err(error) => eprintln!("failed to read pins: {error}"),
                }
                match store.run_history() {
                    Ok(history) => run_prompt.set_history(history),
                    Err(error) => eprintln!("failed to read Run history: {error}"),
                }
                Some(store)
            }
            Err(error) => {
                eprintln!("persistent pin storage unavailable: {error}");
                None
            }
        };
        Self {
            desktop_surfaces: Vec::new(),
            window: None,
            gpu: None,
            panel_surfaces: Vec::new(),
            panel_primary: Vec::new(),
            shell_settings: ShellSettings::load_default(),
            settings_deadline: Instant::now(),
            display_deadline: Instant::now(),
            display_topology: Vec::new(),
            panel_hovered: false,
            panel_task_hovered: None,
            panel_tray_hovered: None,
            panel_desktop_hovered: None,
            panel_control_hovered: false,
            control_center_window: None,
            control_center_gpu: None,
            control_center_visible: false,
            volume_osd_window: None,
            volume_osd_gpu: None,
            volume_osd_deadline: None,
            volume_osd_state: (0, false),
            screenshot_tool: None,
            screenshot_capture_deadline: None,
            context_menu_window: None,
            context_menu_gpu: None,
            context_menu_hovered: None,
            context_close_hovered: None,
            context_menu_target: None,
            context_menu_actions: Vec::new(),
            context_preview_mode: false,
            run_window: None,
            run_gpu: None,
            run_prompt,
            run_editor: TextEditor::default(),
            run_modifiers: ModifiersState::empty(),
            run_ignoring_trigger_r: false,
            run_hovered: None,
            preview_group: None,
            preview_hide_deadline: None,
            task_windows: Vec::new(),
            window_groups: Vec::new(),
            panel_glyph_ids: HashMap::new(),
            next_panel_glyph_id: 1,
            unresolved_panel_icons: HashSet::new(),
            tray_items: Vec::new(),
            launcher_visibility: LauncherVisibility::default(),
            launcher_focus_loss_deadline: None,
            launcher_hotkey_rx: platform::launcher_hotkey_receiver(),
            task_switcher_active: false,
            task_switcher_windows: Vec::new(),
            clock_deadline: next_minute_deadline(Instant::now(), SystemTime::now()),
            window_deadline: Instant::now(),
            fullscreen_deadline: Instant::now(),
            window_feed: WindowFeed::new(),
            tray_feed: TrayFeed::new(),
            launcher,
            launcher_editor: TextEditor::default(),
            hovered_result: None,
            pin_store,
            scroll_offset: 0,
            cursor_position: None,
            scrollbar_drag_offset: None,
        }
    }
}

impl Nickel {
    fn set_control_center_visible(&mut self, visible: bool) {
        if visible && self.control_center_gpu.is_none() {
            let renderer = self
                .control_center_window
                .as_ref()
                .zip(self.gpu.as_ref())
                .and_then(|(window, gpu)| {
                    control_center::ControlCenterGpu::new(window.clone(), gpu.graphics.clone())
                        .map_err(|error| {
                            eprintln!(
                                "failed to initialize Nickel Control Center renderer: {error}"
                            );
                        })
                        .ok()
                });
            self.control_center_gpu = renderer;
        }
        if !visible
            && self
                .control_center_gpu
                .as_mut()
                .is_some_and(control_center::ControlCenterGpu::pointer_released)
        {
            platform::release_pointer();
        }
        self.control_center_visible = visible;
        if visible {
            self.set_launcher_visible(false);
            self.hide_run();
            if let Some(gpu) = &mut self.control_center_gpu {
                gpu.refresh();
            }
        }
        if let Some(window) = &self.control_center_window {
            window.set_visible(visible);
            if visible {
                window.focus_window();
                window.request_redraw();
            }
        }
        if !visible {
            self.control_center_gpu = None;
        }
    }

    fn toggle_control_center(&mut self) {
        self.set_control_center_visible(!self.control_center_visible);
    }

    fn show_volume_osd(&mut self, volume_percent: u8, muted: bool) {
        self.volume_osd_state = (volume_percent, muted);
        if self.volume_osd_gpu.is_none() {
            self.volume_osd_gpu = self
                .volume_osd_window
                .as_ref()
                .zip(self.gpu.as_ref())
                .and_then(|(window, gpu)| {
                    volume_osd::VolumeOsdGpu::new(window.clone(), gpu.graphics.clone())
                        .map_err(|error| {
                            eprintln!("failed to initialize Nickel volume indicator: {error}");
                        })
                        .ok()
                });
        }
        if let Some(window) = &self.volume_osd_window {
            window.set_visible(true);
            window.request_redraw();
        }
        self.volume_osd_deadline = Some(Instant::now() + Duration::from_millis(1_200));
    }

    fn show_run(&mut self) {
        self.set_launcher_visible(false);
        self.run_prompt.clear();
        self.run_editor.clear();
        self.run_ignoring_trigger_r = true;
        self.run_hovered = None;
        if let Some(window) = &self.run_window {
            window.set_visible(true);
            window.focus_window();
            window.request_redraw();
        }
    }

    fn hide_run(&mut self) {
        self.run_hovered = None;
        if let Some(window) = &self.run_window {
            window.set_visible(false);
        }
    }

    fn submit_run(&mut self) {
        let Some(command) = self
            .run_prompt
            .submission(self.run_editor.text())
            .map(str::to_owned)
        else {
            return;
        };
        if platform::execute_run_command(&command) {
            self.run_prompt.record(&command);
            if let Some(store) = &self.pin_store
                && let Err(error) = store.record_run(self.run_prompt.history())
            {
                eprintln!("failed to store Run history: {error}");
            }
            self.hide_run();
        }
    }

    fn ensure_context_menu_gpu(&mut self) -> bool {
        if self.context_menu_gpu.is_some() {
            return true;
        }
        let (Some(window), Some(launcher_gpu)) = (&self.context_menu_window, &self.gpu) else {
            return false;
        };
        match context_menu::ContextMenuGpu::new(window.clone(), launcher_gpu.graphics.clone()) {
            Ok(gpu) => {
                self.context_menu_gpu = Some(gpu);
                true
            }
            Err(error) => {
                eprintln!("failed to initialize Nickel context menu renderer: {error}");
                false
            }
        }
    }

    fn hide_context_menu(&mut self) {
        self.context_menu_target = None;
        self.context_menu_hovered = None;
        self.context_close_hovered = None;
        self.context_menu_actions.clear();
        self.context_preview_mode = false;
        self.preview_group = None;
        self.preview_hide_deadline = None;
        if !platform::send_shell_command(ShellCommand::HideContextMenu)
            && let Some(window) = &self.context_menu_window
        {
            window.set_visible(false);
        }
    }

    fn show_context_menu(&mut self, task_index: usize) {
        let Some(task) = self
            .window_groups
            .get(task_index)
            .and_then(|group| group.windows.last())
        else {
            return;
        };
        let task = task.id;
        self.context_menu_target = Some(task);
        self.context_preview_mode = false;
        self.preview_group = None;
        self.preview_hide_deadline = None;
        if !self.ensure_context_menu_gpu() {
            return;
        }
        self.context_menu_actions = vec![ContextAction::Close(task)];
        if let Some(gpu) = &mut self.context_menu_gpu {
            gpu.set_labels(&["Close window".into()]);
        }
        let x = panel::task_menu_x(task_index);
        if !platform::send_shell_command(ShellCommand::ShowContextMenu {
            x,
            width: context_menu::WIDTH as i32,
            height: context_menu::height_for(1) as i32,
        }) && let Some(window) = &self.context_menu_window
        {
            window.set_visible(true);
            window.focus_window();
        }
        if let Some(window) = &self.context_menu_window {
            window.request_redraw();
        }
    }

    fn show_window_group(&mut self, task_index: usize) {
        let preview_was_visible =
            self.context_preview_mode && self.preview_group == Some(task_index);
        let Some(group) = self.window_groups.get(task_index) else {
            return;
        };
        let windows = group.windows.clone();
        let application_name = group.application_name.clone();
        let previews: Vec<_> = windows
            .iter()
            .filter_map(|window| self.window_feed.preview(window.id))
            .collect();
        self.preview_group = Some(task_index);
        self.preview_hide_deadline = None;
        if previews.len() != windows.len() {
            self.context_menu_target = None;
            self.context_menu_hovered = None;
            self.context_close_hovered = None;
            self.context_menu_actions.clear();
            self.context_preview_mode = true;
            platform::send_shell_command(ShellCommand::HideContextMenu);
            return;
        }
        let labels: Vec<_> = previews
            .iter()
            .map(|preview| {
                let window = windows
                    .iter()
                    .find(|window| window.id == preview.window)
                    .expect("preview belongs to grouped window");
                if window.title.is_empty() {
                    application_name.clone()
                } else {
                    window.title.clone()
                }
            })
            .collect();
        let actions: Vec<_> = previews
            .iter()
            .map(|preview| ContextAction::Activate(preview.window))
            .collect();
        self.context_menu_target = previews.last().map(|preview| preview.window);
        if !self.ensure_context_menu_gpu() {
            return;
        }
        self.context_menu_actions = actions;
        self.context_preview_mode = true;
        if let Some(gpu) = &mut self.context_menu_gpu {
            gpu.set_previews(
                &labels,
                previews.into_iter().map(|preview| preview.image).collect(),
            );
        }
        let x = panel::task_menu_x(task_index);
        if !preview_was_visible {
            platform::send_shell_command(ShellCommand::ShowPreview {
                x,
                width: context_menu::preview_width(labels.len()) as i32,
                height: context_menu::PREVIEW_HEIGHT as i32,
                windows: windows.iter().map(|window| window.id).collect(),
            });
        }
        if let Some(window) = &self.context_menu_window {
            window.request_redraw();
        }
    }

    fn show_window_actions(&mut self, index: usize) {
        let x = self
            .preview_group
            .map(panel::task_menu_x)
            .unwrap_or_default()
            + index as i32 * context_menu::PREVIEW_CARD_WIDTH as i32;
        let Some(window) = self
            .context_menu_actions
            .get(index)
            .map(|action| match action {
                ContextAction::Activate(window)
                | ContextAction::Close(window)
                | ContextAction::Maximize(window)
                | ContextAction::Minimize(window) => *window,
            })
        else {
            return;
        };
        self.context_menu_target = Some(window);
        self.context_preview_mode = false;
        self.preview_group = None;
        self.preview_hide_deadline = None;
        self.context_menu_hovered = None;
        self.context_close_hovered = None;
        self.context_menu_actions = vec![
            ContextAction::Close(window),
            ContextAction::Maximize(window),
            ContextAction::Minimize(window),
        ];
        if let Some(gpu) = &mut self.context_menu_gpu {
            gpu.set_labels(&[
                "Close window".into(),
                "Maximize / Restore".into(),
                "Minimize window".into(),
            ]);
        }
        platform::send_shell_command(ShellCommand::ShowContextMenu {
            x,
            width: context_menu::WIDTH as i32,
            height: context_menu::height_for(3) as i32,
        });
        if let Some(window) = &self.context_menu_window {
            window.request_redraw();
        }
    }

    fn set_launcher_visible(&mut self, visible: bool) {
        self.hide_context_menu();
        self.launcher_visibility.set(visible);
        if visible {
            #[cfg(target_os = "windows")]
            {
                self.launcher_focus_loss_deadline =
                    Some(Instant::now() + Duration::from_millis(150));
            }
        } else {
            self.launcher_focus_loss_deadline = None;
            self.launcher.clear();
            self.launcher_editor.clear();
            self.hovered_result = None;
            self.scroll_offset = 0;
            self.scrollbar_drag_offset = None;
        }
        let handled = platform::send_shell_command(if visible {
            ShellCommand::Show
        } else {
            ShellCommand::Hide
        });
        if !handled && let Some(window) = &self.window {
            window.set_visible(visible);
            if visible {
                window.focus_window();
                window.request_redraw();
            }
        }
        platform::launcher_visibility_applied(visible);
    }

    fn toggle_launcher(&mut self) {
        self.set_launcher_visible(!self.launcher_visibility.is_visible());
    }

    fn advance_task_switcher(&mut self, forward: bool, same_group: bool) {
        if !self.task_switcher_active {
            self.refresh_task_windows();
            self.task_switcher_windows = if same_group {
                let active_application = self
                    .task_windows
                    .iter()
                    .find(|window| window.active)
                    .and_then(|window| window.application_id.clone());
                self.task_windows
                    .iter()
                    .filter(|window| {
                        active_application.is_some() && window.application_id == active_application
                    })
                    .cloned()
                    .collect()
            } else {
                self.task_windows.clone()
            };
            if self.task_switcher_windows.len() < 2 {
                self.task_switcher_windows.clear();
                return;
            }
            self.context_menu_actions = self
                .task_switcher_windows
                .iter()
                .map(|window| ContextAction::Activate(window.id))
                .collect();
            let labels: Vec<_> = self
                .task_switcher_windows
                .iter()
                .map(|window| {
                    if window.title.is_empty() {
                        "Untitled window".to_owned()
                    } else {
                        window.title.clone()
                    }
                })
                .collect();
            let active = self
                .task_switcher_windows
                .iter()
                .position(|window| window.active)
                .unwrap_or(0);
            self.context_menu_hovered = Some(active);
            self.context_menu_target = self
                .task_switcher_windows
                .get(active)
                .map(|window| window.id);
            self.context_preview_mode = false;
            self.preview_group = None;
            self.preview_hide_deadline = None;
            if !self.ensure_context_menu_gpu() {
                return;
            }
            if let Some(gpu) = &mut self.context_menu_gpu {
                gpu.set_previews(
                    &labels,
                    labels.iter().map(|_| image::RgbaImage::new(1, 1)).collect(),
                );
            }
            let width = context_menu::preview_width(labels.len());
            let height = context_menu::PREVIEW_HEIGHT;
            self.task_switcher_active = true;
            if let Some(window) = &self.context_menu_window {
                let monitor = window.current_monitor().or_else(|| {
                    self.panel_surfaces
                        .first()
                        .and_then(|(panel, _)| panel.current_monitor())
                });
                let x = monitor
                    .as_ref()
                    .map(|monitor| centered_position(monitor, (width, height)).x)
                    .unwrap_or_default();
                if !platform::send_shell_command(ShellCommand::ShowPreview {
                    x,
                    width: width as i32,
                    height: height as i32,
                    windows: self
                        .task_switcher_windows
                        .iter()
                        .map(|window| window.id)
                        .collect(),
                }) {
                    let _ = window.request_inner_size(PhysicalSize::new(width, height));
                    if let Some(monitor) = monitor {
                        window.set_outer_position(centered_position(&monitor, (width, height)));
                    }
                    window.set_visible(true);
                    #[cfg(not(target_os = "windows"))]
                    window.focus_window();
                }
                window.request_redraw();
            }
        }
        let count = self.context_menu_actions.len();
        if count == 0 {
            return;
        }
        let current = self.context_menu_hovered.unwrap_or(0);
        let selected = if forward {
            (current + 1) % count
        } else {
            (current + count - 1) % count
        };
        self.context_menu_hovered = Some(selected);
        self.context_menu_target = self
            .task_switcher_windows
            .get(selected)
            .map(|window| window.id);
        if let Some(window) = &self.context_menu_window {
            window.request_redraw();
        }
    }

    fn commit_task_switcher(&mut self) {
        if !self.task_switcher_active {
            return;
        }
        let target = self.context_menu_hovered.and_then(|index| {
            self.context_menu_actions.get(index).and_then(|action| {
                if let ContextAction::Activate(window) = action {
                    Some(*window)
                } else {
                    None
                }
            })
        });
        self.task_switcher_active = false;
        self.task_switcher_windows.clear();
        self.hide_context_menu();
        if let Some(window) = target {
            platform::send_shell_command(ShellCommand::WindowAction {
                window,
                action: WindowAction::Activate,
            });
        }
    }

    fn refresh_task_windows(&mut self) {
        let Some(windows) = self.window_feed.snapshot(&self.launcher) else {
            return;
        };
        if windows == self.task_windows {
            return;
        }
        let groups =
            stable_window_group_order(&self.window_groups, self.launcher.group_windows(&windows));
        let mut tasks = Vec::with_capacity(groups.len());
        for group in &groups {
            let key = window_group_key(group);
            let glyph_id = if let Some(id) = self.panel_glyph_ids.get(&key) {
                *id
            } else {
                let id = self.next_panel_glyph_id;
                self.next_panel_glyph_id += 1;
                self.panel_glyph_ids.insert(key, id);
                id
            };
            let resolved = group
                .application_id
                .as_ref()
                .and_then(|id| self.launcher.application(id))
                .and_then(|application| application.icon_path())
                .and_then(icons::load)
                .or_else(|| {
                    group
                        .windows
                        .last()
                        .and_then(|window| self.window_feed.icon(window.id))
                });
            if resolved.is_none()
                && self
                    .unresolved_panel_icons
                    .insert(group.application_name.clone())
            {
                eprintln!("no panel icon resolved for {}", group.application_name);
            }
            tasks.push(panel::PanelTask {
                glyph_id,
                active: group.active(),
                icon: resolved.unwrap_or_else(panel::fallback_icon),
            });
        }
        self.task_windows = windows;
        self.window_groups = groups;
        if self
            .context_menu_target
            .is_some_and(|target| !self.task_windows.iter().any(|window| window.id == target))
        {
            self.hide_context_menu();
        }
        for (_, gpu) in &mut self.panel_surfaces {
            gpu.set_tasks(tasks.clone());
        }
        for (window, _) in &self.panel_surfaces {
            window.request_redraw();
        }
    }

    fn refresh_tray_items(&mut self) {
        let items = self.tray_feed.snapshot();
        if items == self.tray_items {
            return;
        }
        eprintln!("nickel-ui: tray items updated: {}", items.len());
        let rendered: Vec<_> = items
            .iter()
            .map(|item| panel::PanelTrayItem {
                icon: item.icon.clone(),
            })
            .collect();
        self.tray_items = items;
        for (_, gpu) in &mut self.panel_surfaces {
            gpu.set_tray_items(rendered.clone());
        }
        for (window, _) in &self.panel_surfaces {
            window.request_redraw();
        }
    }

    fn refresh_shell_settings(&mut self) {
        let settings = ShellSettings::load_default();
        if settings == self.shell_settings {
            return;
        }
        let bar_visibility_changed =
            settings.bar_on_all_displays != self.shell_settings.bar_on_all_displays;
        let desktops_changed = settings.desktop_count != self.shell_settings.desktop_count
            || settings.active_desktop != self.shell_settings.active_desktop;
        let appearance_changed = settings.theme != self.shell_settings.theme
            || settings.accent_hue != self.shell_settings.accent_hue
            || settings.accent_intensity != self.shell_settings.accent_intensity;
        self.shell_settings = settings;
        if desktops_changed {
            for (window, gpu) in &mut self.panel_surfaces {
                gpu.set_desktops(settings.desktop_count, settings.active_desktop);
                window.request_redraw();
            }
        }
        if appearance_changed {
            let appearance = settings.resolve_appearance(nickel_platform::appearance());
            if let Some(gpu) = &mut self.gpu {
                gpu.set_appearance(appearance);
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            for (window, gpu) in &mut self.panel_surfaces {
                gpu.set_appearance(appearance);
                window.request_redraw();
            }
        }
        if !bar_visibility_changed {
            return;
        }
        for (index, (window, _)) in self.panel_surfaces.iter().enumerate() {
            let should_show = settings.bar_on_all_displays
                || self.panel_primary.get(index).copied().unwrap_or(index == 0);
            if window.is_visible() == Some(should_show) {
                continue;
            }
            if should_show {
                #[cfg(target_os = "windows")]
                if !platform::configure_panel_window(window) {
                    eprintln!("failed to restore a Nickel panel AppBar");
                }
                window.set_visible(true);
                window.request_redraw();
            } else {
                #[cfg(target_os = "windows")]
                platform::release_panel_window(window);
                window.set_visible(false);
            }
        }
    }

    fn reconcile_display_topology(&mut self, event_loop: &ActiveEventLoop) {
        let monitors = sorted_monitors(event_loop);
        let topology = display_topology(&monitors);
        if topology == self.display_topology {
            return;
        }
        if monitors.len() != self.desktop_surfaces.len()
            || monitors.len() != self.panel_surfaces.len()
        {
            eprintln!(
                "display count changed from {} to {}; surface recreation required",
                self.desktop_surfaces.len(),
                monitors.len()
            );
            self.display_topology = topology;
            return;
        }

        let primary = event_loop.primary_monitor();
        self.panel_primary.clear();
        for ((desktop_window, _), monitor) in self.desktop_surfaces.iter().zip(&monitors) {
            desktop_window.set_outer_position(monitor.position());
            let _ = desktop_window.request_inner_size(monitor.size());
            #[cfg(target_os = "windows")]
            if !platform::configure_desktop_window(desktop_window) {
                eprintln!("failed to reposition a Nickel desktop surface");
            }
        }
        for ((panel_window, _), monitor) in self.panel_surfaces.iter().zip(&monitors) {
            let is_primary = primary.as_ref() == Some(monitor);
            self.panel_primary.push(is_primary);
            #[cfg(target_os = "windows")]
            platform::release_panel_window(panel_window);
            let (size, position) =
                panel_layout(monitor.position(), monitor.size(), monitor.scale_factor());
            panel_window.set_outer_position(position);
            let _ = panel_window.request_inner_size(size);
            let should_show = self.shell_settings.bar_on_all_displays || is_primary;
            panel_window.set_visible(should_show);
            #[cfg(target_os = "windows")]
            if should_show && !platform::configure_panel_window(panel_window) {
                eprintln!("failed to reposition a Nickel panel AppBar");
            }
            panel_window.request_redraw();
        }
        self.display_topology = topology;
        tracing::info!("reconciled Nickel surfaces with changed display topology");
    }

    fn launch_result(&mut self, index: usize) {
        let Some(result) = self.launcher.result_at(index) else {
            return;
        };
        match result.launch() {
            Ok(child) => {
                println!(
                    "launched application: {} (pid {}, icon {})",
                    result.name(),
                    child.id(),
                    result.icon().unwrap_or("none")
                );
                self.set_launcher_visible(false);
            }
            Err(error) => eprintln!("failed to launch application {}: {error}", result.name()),
        }
    }

    fn viewport_metrics(&self) -> (u32, u32, usize) {
        let size = self.window.as_ref().expect("window exists").inner_size();
        (
            size.width,
            size.height,
            layout::visible_capacity(size.width, size.height),
        )
    }

    fn set_scroll_offset(&mut self, offset: usize) {
        let (_, _, capacity) = self.viewport_metrics();
        self.scroll_offset = offset.min(layout::max_scroll_offset(
            self.launcher.result_count(),
            capacity,
        ));
    }

    fn scroll_by(&mut self, rows: i32) {
        let offset = if rows.is_negative() {
            self.scroll_offset
                .saturating_sub(rows.unsigned_abs() as usize)
        } else {
            self.scroll_offset.saturating_add(rows as usize)
        };
        self.set_scroll_offset(offset);
    }

    fn ensure_selection_visible(&mut self) {
        let (_, _, capacity) = self.viewport_metrics();
        if capacity == 0 {
            return;
        }
        let selected = self.launcher.selected_index();
        if selected < self.scroll_offset {
            self.scroll_offset = selected;
        } else if selected >= self.scroll_offset + capacity {
            self.scroll_offset = selected + 1 - capacity;
        }
        self.set_scroll_offset(self.scroll_offset);
    }
}

struct Gpu {
    surface: wgpu::Surface<'static>,
    graphics: Arc<graphics::SharedGraphics>,
    config: wgpu::SurfaceConfiguration,
    font_system: FontSystem,
    swash_cache: SwashCache,
    viewport: Viewport,
    atlas: TextAtlas,
    text_renderer: TextRenderer,
    search_buffer: Buffer,
    chrome_buffers: Vec<Buffer>,
    result_buffers: Vec<Buffer>,
    icon_ids: HashMap<LauncherIconSource, u16>,
    icon_images: Vec<Option<image::RgbaImage>>,
    icon_requests: mpsc::Sender<(u16, LauncherIconSource)>,
    icon_results: mpsc::Receiver<(u16, Option<image::RgbaImage>)>,
    rectangle_renderer: rectangles::RectangleRenderer,
    palette: ThemePalette,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum LauncherIconSource {
    File(PathBuf),
    Platform(String),
}

impl Gpu {
    async fn new(window: Arc<Window>) -> Result<Self, String> {
        let (graphics, surface) = graphics::SharedGraphics::new(window.clone()).await?;
        let size = window.inner_size();
        let mut config = surface
            .get_default_config(&graphics.adapter, size.width.max(1), size.height.max(1))
            .ok_or_else(|| "graphics surface has no supported configuration".to_owned())?;
        config.desired_maximum_frame_latency = 1;
        surface.configure(&graphics.device, &config);

        let mut font_system = FontSystem::new();
        let swash_cache = SwashCache::new();
        let cache = Cache::new(&graphics.device);
        let viewport = Viewport::new(&graphics.device, &cache);
        let mut atlas = TextAtlas::new(&graphics.device, &graphics.queue, &cache, config.format);
        let text_renderer = TextRenderer::new(
            &mut atlas,
            &graphics.device,
            wgpu::MultisampleState::default(),
            None,
        );
        let rectangle_renderer =
            rectangles::RectangleRenderer::new(&graphics.device, config.format);
        let mut search_buffer = Buffer::new(&mut font_system, Metrics::new(20.0, 30.0));
        search_buffer.set_size(
            Some(
                config
                    .width
                    .saturating_sub(layout::CONTENT_LEFT as u32 + 56) as f32,
            ),
            Some(40.0),
        );
        let chrome_buffers = (0..8)
            .map(|_| Buffer::new(&mut font_system, Metrics::new(17.0, 30.0)))
            .collect();
        let result_buffers = (0..12)
            .map(|_| Buffer::new(&mut font_system, Metrics::new(17.0, 24.0)))
            .collect();
        let (icon_requests, worker_requests) = mpsc::channel::<(u16, LauncherIconSource)>();
        let (worker_results, icon_results) = mpsc::channel();
        let redraw_window = window.clone();
        thread::Builder::new()
            .name("nickel-icon-loader".into())
            .spawn(move || {
                while let Ok((id, source)) = worker_requests.recv() {
                    let requested_source = source.clone();
                    let image = match source {
                        LauncherIconSource::File(path) => icons::load(&path),
                        LauncherIconSource::Platform(reference) => {
                            platform::application_icon(&reference)
                        }
                    };
                    if !image
                        .as_ref()
                        .is_some_and(|image| image.pixels().any(|pixel| pixel.0[3] != 0))
                    {
                        tracing::warn!(
                            ?requested_source,
                            glyph_id = id,
                            dimensions = ?image.as_ref().map(image::RgbaImage::dimensions),
                            "launcher icon request returned no visible pixels"
                        );
                    }
                    if worker_results.send((id, image)).is_err() {
                        break;
                    }
                    redraw_window.request_redraw();
                }
            })
            .map_err(|error| format!("failed to start icon loader: {error}"))?;

        Ok(Self {
            surface,
            graphics,
            config,
            font_system,
            swash_cache,
            viewport,
            atlas,
            text_renderer,
            search_buffer,
            chrome_buffers,
            result_buffers,
            icon_ids: HashMap::new(),
            icon_images: Vec::new(),
            icon_requests,
            icon_results,
            rectangle_renderer,
            palette: ThemePalette::from_appearance(Appearance::default()),
        })
    }

    fn set_appearance(&mut self, appearance: Appearance) {
        self.palette = ThemePalette::from_appearance(appearance);
    }

    fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.graphics.device, &self.config);
        let text_width = width.saturating_sub(layout::CONTENT_LEFT as u32 + 56) as f32;
        self.search_buffer.set_size(Some(text_width), Some(40.0));
    }

    fn render(
        &mut self,
        launcher: &Launcher,
        editor: &TextEditor,
        hovered_result: Option<usize>,
        scroll_offset: usize,
    ) {
        while let Ok((id, image)) = self.icon_results.try_recv() {
            if let Some(slot) = self.icon_images.get_mut(id as usize) {
                *slot = image;
            }
        }
        let search_field = TextField::placeholder(editor, "Search applications…").scale(3.0);
        self.search_buffer.set_text(
            search_field.display_text(),
            &Attrs::new().family(Family::SansSerif),
            Shaping::Advanced,
            None,
        );
        self.search_buffer
            .shape_until_scroll(&mut self.font_system, false);
        let chrome = [
            "FAVORITES",
            "ALL APPLICATIONS",
            "RECENT",
            "PLACES",
            "SETTINGS",
            "NICKEL FILE",
            "NICKEL PLATING",
            "POWER",
        ];
        for (buffer, label) in self.chrome_buffers.iter_mut().zip(chrome) {
            buffer.set_size(Some(188.0), Some(34.0));
            buffer.set_text(
                label,
                &Attrs::new().family(Family::SansSerif),
                Shaping::Advanced,
                None,
            );
            buffer.shape_until_scroll(&mut self.font_system, false);
        }

        let capacity = layout::visible_capacity(self.config.width, self.config.height);
        let visible_count = launcher
            .result_count()
            .saturating_sub(scroll_offset)
            .min(capacity)
            .max(1);
        while self.result_buffers.len() < visible_count {
            self.result_buffers
                .push(Buffer::new(&mut self.font_system, Metrics::new(17.0, 24.0)));
        }
        let mut row_glyphs = vec![Vec::new(); visible_count];
        for (index, buffer) in self
            .result_buffers
            .iter_mut()
            .take(visible_count)
            .enumerate()
        {
            let result_index = scroll_offset + index;
            let row = layout::ResultRow::allocate(index, self.config.width);
            let text = launcher
                .result_at(result_index)
                .map_or("No applications found", |result| result.name());
            let text = compact_tile_label(text, 25);
            buffer.set_size(Some(row.label.width), Some(row.label.height));
            buffer.set_text(
                &text,
                &Attrs::new().family(Family::SansSerif),
                Shaping::Advanced,
                None,
            );
            for line in &mut buffer.lines {
                line.set_align(Some(Align::Center));
            }
            buffer.shape_until_scroll(&mut self.font_system, false);

            let Some(source) = launcher.result_at(result_index).and_then(|application| {
                application
                    .icon_path()
                    .map(|path| LauncherIconSource::File(path.to_owned()))
                    .or_else(|| {
                        application
                            .icon()
                            .map(|reference| LauncherIconSource::Platform(reference.to_owned()))
                    })
            }) else {
                continue;
            };
            let glyph_id = if let Some(id) = self.icon_ids.get(&source) {
                *id
            } else {
                let Ok(id) = u16::try_from(self.icon_images.len()) else {
                    continue;
                };
                self.icon_ids.insert(source.clone(), id);
                self.icon_images.push(None);
                if self.icon_requests.send((id, source)).is_err() {
                    eprintln!("launcher icon loader stopped");
                }
                id
            };
            if self.icon_images[glyph_id as usize].is_some() {
                row_glyphs[index].push(CustomGlyph {
                    id: glyph_id,
                    left: row.icon.x - row.label.x,
                    top: row.icon.y - row.label.y,
                    width: row.icon.width,
                    height: row.icon.height,
                    color: None,
                    snap_to_physical_pixel: true,
                    metadata: 0,
                });
            }
        }

        self.viewport.update(
            &self.graphics.queue,
            Resolution {
                width: self.config.width,
                height: self.config.height,
            },
        );
        let searching = !editor.text().is_empty() || !editor.preedit().is_empty();
        let scrollbar = searching
            .then(|| {
                layout::scrollbar(
                    self.config.width,
                    self.config.height,
                    launcher.result_count(),
                    capacity,
                    scroll_offset,
                )
            })
            .flatten();
        let hovered_row = hovered_result.and_then(|index| index.checked_sub(scroll_offset));
        let selected_row = launcher
            .selected_index()
            .checked_sub(scroll_offset)
            .filter(|index| *index < visible_count);
        self.rectangle_renderer.update(
            &self.graphics.queue,
            (self.config.width, self.config.height),
            hovered_row.filter(|index| *index < visible_count),
            selected_row,
            scrollbar,
            self.palette,
        );
        let icon_images = &self.icon_images;
        let mut text_areas = Vec::with_capacity(visible_count + self.chrome_buffers.len() + 1);
        text_areas.push(TextArea {
            buffer: &self.search_buffer,
            left: layout::CONTENT_LEFT + 14.0,
            top: 27.0,
            scale: 1.0,
            bounds: TextBounds {
                left: layout::CONTENT_LEFT as i32,
                top: 22,
                right: self.config.width.saturating_sub(28) as i32,
                bottom: 70,
            },
            default_color: glyphon_color(self.palette.text),
            custom_glyphs: &[],
        });
        let chrome_positions = [
            (18.0, 78.0),
            (18.0, 120.0),
            (18.0, 162.0),
            (18.0, 204.0),
            (18.0, 246.0),
            (layout::CONTENT_LEFT, self.config.height as f32 - 48.0),
            (
                layout::CONTENT_LEFT + 174.0,
                self.config.height as f32 - 48.0,
            ),
            (
                self.config.width as f32 - 112.0,
                self.config.height as f32 - 48.0,
            ),
        ];
        for (index, (buffer, (left, top))) in
            self.chrome_buffers.iter().zip(chrome_positions).enumerate()
        {
            text_areas.push(TextArea {
                buffer,
                left,
                top,
                scale: 1.0,
                bounds: TextBounds {
                    left: left as i32,
                    top: top as i32,
                    right: if index < 5 {
                        layout::SIDEBAR_WIDTH as i32
                    } else {
                        self.config.width as i32
                    },
                    bottom: (top + 34.0) as i32,
                },
                default_color: glyphon_color(if index == 1 {
                    self.palette.text
                } else {
                    self.palette.muted
                }),
                custom_glyphs: &[],
            });
        }
        for (index, buffer) in self.result_buffers.iter().take(visible_count).enumerate() {
            let row = layout::ResultRow::allocate(index, self.config.width);
            text_areas.push(TextArea {
                buffer,
                left: row.label.x,
                top: row.label.y,
                scale: 1.0,
                bounds: TextBounds {
                    left: row.outer.x as i32,
                    top: row.outer.y as i32,
                    right: row.outer.right() as i32,
                    bottom: row.outer.bottom() as i32,
                },
                default_color: glyphon_color(self.palette.muted),
                custom_glyphs: &row_glyphs[index],
            });
        }
        self.text_renderer
            .prepare_with_custom(
                &self.graphics.device,
                &self.graphics.queue,
                &mut self.font_system,
                &mut self.atlas,
                &self.viewport,
                text_areas,
                &mut self.swash_cache,
                &|request: RasterizeCustomGlyphRequest| {
                    let source = icon_images.get(request.id as usize)?.as_ref()?;
                    let image = icons::resized(source, request.width.into(), request.height.into());
                    Some(RasterizedCustomGlyph {
                        data: image.into_raw(),
                        content_type: ContentType::Color,
                    })
                },
            )
            .expect("text preparation should succeed");

        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.graphics.device, &self.config);
                return;
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => return,
            wgpu::CurrentSurfaceTexture::Validation => {
                eprintln!("skipped frame after surface validation error");
                return;
            }
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder =
            self.graphics
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("nickel frame encoder"),
                });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("nickel background pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu_color(self.palette.background)),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                ..Default::default()
            });
            self.rectangle_renderer.render(&mut pass);
            self.text_renderer
                .render(&self.atlas, &self.viewport, &mut pass)
                .expect("text rendering should succeed");
        }

        self.graphics.queue.submit([encoder.finish()]);
        self.graphics.queue.present(frame);
    }
}

fn glyphon_color(rgb: u32) -> Color {
    Color::rgb(
        ((rgb >> 16) & 0xff) as u8,
        ((rgb >> 8) & 0xff) as u8,
        (rgb & 0xff) as u8,
    )
}

fn compact_tile_label(label: &str, maximum_characters: usize) -> String {
    if label.chars().count() <= maximum_characters {
        return label.to_owned();
    }
    let mut compact = label
        .chars()
        .take(maximum_characters.saturating_sub(1))
        .collect::<String>();
    compact.push('…');
    compact
}

fn wgpu_color(rgb: u32) -> wgpu::Color {
    wgpu::Color {
        r: f64::from((rgb >> 16) & 0xff) / 255.0,
        g: f64::from((rgb >> 8) & 0xff) / 255.0,
        b: f64::from(rgb & 0xff) / 255.0,
        a: 1.0,
    }
}

impl ApplicationHandler for Nickel {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let monitors = sorted_monitors(event_loop);
        let primary = event_loop.primary_monitor();
        let target = primary.clone().or_else(|| monitors.first().cloned());

        let desktop_attributes = WindowAttributes::default()
            .with_title("Nickel Desktop")
            .with_inner_size(PhysicalSize::new(1280, 720))
            .with_decorations(false)
            .with_resizable(false)
            .with_window_level(WindowLevel::AlwaysOnBottom);
        let mut launcher_attributes = WindowAttributes::default()
            .with_title("Nickel Launcher Initializing")
            .with_inner_size(LogicalSize::new(960, 640))
            .with_min_inner_size(LogicalSize::new(480, 320))
            .with_visible(false)
            .with_decorations(false)
            .with_window_level(WindowLevel::AlwaysOnTop);
        let mut panel_attributes = WindowAttributes::default()
            .with_title("Nickel Panel")
            .with_inner_size(LogicalSize::new(960.0, PANEL_HEIGHT))
            .with_decorations(false)
            .with_transparent(true);
        #[cfg(target_os = "windows")]
        {
            use winit::platform::windows::WindowAttributesExtWindows;
            panel_attributes = panel_attributes.with_class_name("Shell_TrayWnd");
        }
        let context_menu_attributes = WindowAttributes::default()
            .with_title("Nickel Context Menu")
            .with_inner_size(LogicalSize::new(context_menu::WIDTH, context_menu::HEIGHT))
            .with_visible(false)
            .with_decorations(false)
            .with_window_level(WindowLevel::AlwaysOnTop);
        let mut control_center_attributes = WindowAttributes::default()
            .with_title("Nickel Control Center")
            .with_inner_size(LogicalSize::new(
                control_center::WIDTH,
                control_center::HEIGHT,
            ))
            .with_visible(false)
            .with_decorations(false)
            .with_resizable(false)
            .with_window_level(WindowLevel::AlwaysOnTop);
        let mut run_attributes = WindowAttributes::default()
            .with_title("Run")
            .with_inner_size(LogicalSize::new(run_dialog::WIDTH, run_dialog::HEIGHT))
            .with_visible(false)
            .with_resizable(false)
            .with_window_level(WindowLevel::AlwaysOnTop);
        let mut volume_osd_attributes = WindowAttributes::default()
            .with_title("Nickel Volume")
            .with_inner_size(LogicalSize::new(volume_osd::WIDTH, volume_osd::HEIGHT))
            .with_visible(false)
            .with_decorations(false)
            .with_resizable(false)
            .with_window_level(WindowLevel::AlwaysOnTop);
        let mut screenshot_attributes = WindowAttributes::default()
            .with_title("Nickel Screenshot")
            .with_inner_size(PhysicalSize::new(1200, 760))
            .with_min_inner_size(PhysicalSize::new(720, 480))
            .with_visible(false)
            .with_decorations(true)
            .with_resizable(true)
            .with_window_level(WindowLevel::Normal);
        if let Some(monitor) = target.as_ref() {
            launcher_attributes =
                launcher_attributes.with_position(launcher_position(monitor, (960, 640)));
            run_attributes = run_attributes.with_position(centered_position(
                monitor,
                (run_dialog::WIDTH, run_dialog::HEIGHT),
            ));
            let monitor_position = monitor.position();
            let monitor_size = monitor.size();
            volume_osd_attributes = volume_osd_attributes.with_position(PhysicalPosition::new(
                monitor_position.x
                    + (monitor_size.width.saturating_sub(volume_osd::WIDTH) / 2) as i32,
                monitor_position.y + monitor_size.height as i32
                    - volume_osd::HEIGHT as i32
                    - PANEL_HEIGHT as i32
                    - 52,
            ));
            control_center_attributes =
                control_center_attributes.with_position(PhysicalPosition::new(
                    monitor_position.x + monitor_size.width as i32
                        - control_center::WIDTH as i32
                        - 12,
                    monitor_position.y + monitor_size.height as i32
                        - control_center::HEIGHT as i32
                        - PANEL_HEIGHT as i32
                        - 12,
                ));
            let monitor_position = monitor.position();
            let monitor_size = monitor.size();
            screenshot_attributes = screenshot_attributes.with_position(PhysicalPosition::new(
                monitor_position.x + (monitor_size.width.saturating_sub(1200) / 2) as i32,
                monitor_position.y + (monitor_size.height.saturating_sub(760) / 2) as i32,
            ));
        }

        let mut desktop_windows = Vec::new();
        if cfg!(target_os = "windows") {
            for monitor in &monitors {
                let attributes = desktop_attributes
                    .clone()
                    .with_inner_size(monitor.size())
                    .with_position(monitor.position());
                let Ok(window) = event_loop.create_window(attributes) else {
                    eprintln!("failed to create Nickel desktop window");
                    event_loop.exit();
                    return;
                };
                let window = Arc::new(window);
                #[cfg(target_os = "windows")]
                {
                    use winit::platform::windows::WindowExtWindows;
                    window.set_skip_taskbar(true);
                    if !platform::configure_desktop_window(&window) {
                        eprintln!(
                            "failed to place Nickel desktop at the bottom of the Windows Z-order"
                        );
                    }
                }
                desktop_windows.push(window);
            }
        }
        let Ok(launcher_window) = event_loop.create_window(launcher_attributes) else {
            eprintln!("failed to create Nickel launcher window");
            event_loop.exit();
            return;
        };
        let launcher_window = Arc::new(launcher_window);
        launcher_window.set_ime_allowed(true);
        launcher_window
            .set_ime_cursor_area(PhysicalPosition::new(56, 96), PhysicalSize::new(2, 32));
        #[cfg(target_os = "windows")]
        if !platform::configure_launcher_window(&launcher_window) {
            eprintln!("failed to register Nickel's launcher window handle");
        }
        let mut panel_windows = Vec::new();
        let mut panel_primary = Vec::new();
        for monitor in &monitors {
            let (panel_size, panel_position) =
                panel_layout(monitor.position(), monitor.size(), monitor.scale_factor());
            let attributes = panel_attributes
                .clone()
                .with_inner_size(panel_size)
                .with_position(panel_position);
            let Ok(window) = event_loop.create_window(attributes) else {
                eprintln!("failed to create Nickel panel window");
                event_loop.exit();
                return;
            };
            let window = Arc::new(window);
            let is_primary = primary.as_ref() == Some(monitor);
            let should_show = self.shell_settings.bar_on_all_displays || is_primary;
            if should_show {
                #[cfg(target_os = "windows")]
                if !platform::configure_panel_window(&window) {
                    eprintln!("failed to reserve the Windows work area for a Nickel panel");
                }
            } else {
                window.set_visible(false);
            }
            panel_windows.push(window);
            panel_primary.push(is_primary);
        }
        let Ok(context_menu_window) = event_loop.create_window(context_menu_attributes) else {
            eprintln!("failed to create Nickel context menu window");
            event_loop.exit();
            return;
        };
        let context_menu_window = Arc::new(context_menu_window);
        let Ok(run_window) = event_loop.create_window(run_attributes) else {
            eprintln!("failed to create Nickel Run window");
            event_loop.exit();
            return;
        };
        let run_window = Arc::new(run_window);
        run_window.set_ime_allowed(true);
        run_window.set_ime_cursor_area(PhysicalPosition::new(42, 148), PhysicalSize::new(2, 28));
        let Ok(control_center_window) = event_loop.create_window(control_center_attributes) else {
            eprintln!("failed to create Nickel Control Center window");
            event_loop.exit();
            return;
        };
        let control_center_window = Arc::new(control_center_window);
        let Ok(volume_osd_window) = event_loop.create_window(volume_osd_attributes) else {
            eprintln!("failed to create Nickel volume indicator window");
            event_loop.exit();
            return;
        };
        let volume_osd_window = Arc::new(volume_osd_window);
        let Ok(screenshot_window) = event_loop.create_window(screenshot_attributes) else {
            eprintln!("failed to create Nickel screenshot surface");
            event_loop.exit();
            return;
        };
        let screenshot_window = Arc::new(screenshot_window);
        #[cfg(target_os = "windows")]
        {
            use winit::platform::windows::WindowExtWindows;
            context_menu_window.set_skip_taskbar(true);
            run_window.set_skip_taskbar(true);
            control_center_window.set_skip_taskbar(true);
            volume_osd_window.set_skip_taskbar(true);
            screenshot_window.set_skip_taskbar(true);
            if !platform::configure_context_menu_window(&context_menu_window) {
                eprintln!("failed to register Nickel's Windows preview window");
            }
            if !platform::configure_volume_osd_window(&volume_osd_window) {
                eprintln!("failed to configure Nickel's Windows volume indicator");
            }
        }
        platform::send_shell_command(ShellCommand::HideContextMenu);
        let Ok(mut launcher_gpu) = pollster::block_on(Gpu::new(launcher_window.clone())) else {
            eprintln!("failed to initialize Nickel launcher renderer");
            event_loop.exit();
            return;
        };
        launcher_gpu.set_appearance(
            self.shell_settings
                .resolve_appearance(nickel_platform::appearance()),
        );
        let shared_graphics = launcher_gpu.graphics.clone();
        let wallpaper = platform::wallpaper();
        let mut desktop_surfaces = Vec::new();
        for window in desktop_windows {
            match desktop::DesktopGpu::new(
                window.clone(),
                shared_graphics.clone(),
                wallpaper.clone(),
            ) {
                Ok(gpu) => desktop_surfaces.push((window, gpu)),
                Err(error) => eprintln!("failed to initialize Nickel desktop renderer: {error}"),
            }
        }
        let mut panel_surfaces = Vec::new();
        for window in panel_windows {
            let Ok(mut gpu) = panel::PanelGpu::new(window.clone(), shared_graphics.clone()) else {
                eprintln!("failed to initialize Nickel panel renderer");
                event_loop.exit();
                return;
            };
            gpu.set_desktops(
                self.shell_settings.desktop_count,
                self.shell_settings.active_desktop,
            );
            gpu.set_appearance(
                self.shell_settings
                    .resolve_appearance(nickel_platform::appearance()),
            );
            panel_surfaces.push((window, gpu));
        }
        let Ok(run_gpu) =
            run_dialog::RunDialogGpu::new(run_window.clone(), shared_graphics.clone())
        else {
            eprintln!("failed to initialize Nickel Run renderer");
            event_loop.exit();
            return;
        };
        let control_center_gpu = None;
        let screenshot_tool = screenshot::ScreenshotTool::new(
            screenshot_window,
            self.shell_settings
                .resolve_appearance(nickel_platform::appearance()),
        )
        .ok();
        launcher_window.set_title("Nickel Launcher");
        for (window, _) in &desktop_surfaces {
            window.request_redraw();
        }
        for (window, _) in &panel_surfaces {
            window.request_redraw();
        }
        self.desktop_surfaces = desktop_surfaces;
        self.window = Some(launcher_window);
        self.gpu = Some(launcher_gpu);
        self.panel_surfaces = panel_surfaces;
        self.panel_primary = panel_primary;
        self.display_topology = display_topology(&monitors);
        self.context_menu_window = Some(context_menu_window);
        self.context_menu_gpu = None;
        self.run_window = Some(run_window);
        self.run_gpu = Some(run_gpu);
        self.control_center_window = Some(control_center_window);
        self.control_center_gpu = control_center_gpu;
        self.volume_osd_window = Some(volume_osd_window);
        self.screenshot_tool = screenshot_tool;
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if let WindowEvent::KeyboardInput { event, .. } = &event
            && let Some((key, edge)) = focused_shortcut_edge(event)
        {
            platform::handle_focused_shortcut(key, edge);
        }
        if self.screenshot_tool.as_ref().map(|tool| tool.window.id()) == Some(window_id) {
            let tool = self
                .screenshot_tool
                .as_mut()
                .expect("screenshot tool exists");
            match event {
                WindowEvent::CloseRequested => tool.hide(),
                WindowEvent::RedrawRequested => tool.render(),
                WindowEvent::Resized(size) => tool.resize(size.width, size.height),
                WindowEvent::CursorMoved { position, .. } => tool.cursor_moved(position),
                WindowEvent::MouseInput {
                    state: ElementState::Pressed,
                    button: MouseButton::Left,
                    ..
                } => {
                    if tool.pointer_pressed() {
                        tool.hide();
                    } else {
                        platform::capture_pointer(&tool.window);
                    }
                }
                WindowEvent::MouseInput {
                    state: ElementState::Released,
                    button: MouseButton::Left,
                    ..
                } => {
                    tool.pointer_released();
                    platform::release_pointer();
                }
                WindowEvent::KeyboardInput { event, .. }
                    if event.state == ElementState::Pressed
                        && event.logical_key == Key::Named(NamedKey::Escape) =>
                {
                    tool.hide();
                }
                _ => {}
            }
            return;
        }
        if self.volume_osd_window.as_ref().map(|window| window.id()) == Some(window_id) {
            match event {
                WindowEvent::RedrawRequested => {
                    if let Some(gpu) = &mut self.volume_osd_gpu {
                        gpu.render(self.volume_osd_state.0, self.volume_osd_state.1);
                    }
                }
                WindowEvent::CloseRequested => {
                    if let Some(window) = &self.volume_osd_window {
                        window.set_visible(false);
                    }
                    self.volume_osd_deadline = None;
                    self.volume_osd_gpu = None;
                }
                _ => {}
            }
            return;
        }
        if self
            .control_center_window
            .as_ref()
            .map(|window| window.id())
            == Some(window_id)
        {
            match event {
                WindowEvent::CloseRequested | WindowEvent::Focused(false) => {
                    self.set_control_center_visible(false);
                }
                WindowEvent::Resized(size) => {
                    if let Some(gpu) = &mut self.control_center_gpu {
                        gpu.resize(size.width, size.height);
                    }
                    self.control_center_window
                        .as_ref()
                        .expect("control center window exists")
                        .request_redraw();
                }
                WindowEvent::RedrawRequested => {
                    if let Some(gpu) = &mut self.control_center_gpu {
                        gpu.render();
                    }
                }
                WindowEvent::CursorMoved { position, .. } => {
                    if let Some(gpu) = &mut self.control_center_gpu {
                        gpu.cursor_moved(position.x as f32, position.y as f32);
                        if gpu.is_volume_dragging() {
                            self.control_center_window
                                .as_ref()
                                .expect("control center window exists")
                                .request_redraw();
                        }
                    }
                }
                WindowEvent::MouseInput {
                    state: ElementState::Pressed,
                    button: MouseButton::Left,
                    ..
                } => {
                    if self
                        .control_center_gpu
                        .as_mut()
                        .is_some_and(control_center::ControlCenterGpu::pointer_pressed)
                    {
                        let window = self
                            .control_center_window
                            .as_ref()
                            .expect("control center window exists");
                        if self
                            .control_center_gpu
                            .as_ref()
                            .is_some_and(control_center::ControlCenterGpu::is_volume_dragging)
                        {
                            platform::capture_pointer(window);
                        }
                        window.request_redraw();
                    }
                }
                WindowEvent::MouseInput {
                    state: ElementState::Released,
                    button: MouseButton::Left,
                    ..
                } => {
                    if self
                        .control_center_gpu
                        .as_mut()
                        .is_some_and(control_center::ControlCenterGpu::pointer_released)
                    {
                        platform::release_pointer();
                    }
                }
                _ => {}
            }
            return;
        }

        if let Some((window, gpu)) = self
            .desktop_surfaces
            .iter_mut()
            .find(|(window, _)| window.id() == window_id)
        {
            match event {
                WindowEvent::Resized(size) => {
                    gpu.resize(size.width, size.height);
                }
                WindowEvent::RedrawRequested => {
                    gpu.render();
                }
                WindowEvent::CloseRequested => {}
                _ => {}
            }
            let _ = window;
            return;
        }
        if self.context_menu_window.as_ref().map(|window| window.id()) == Some(window_id) {
            match event {
                WindowEvent::CloseRequested => self.hide_context_menu(),
                WindowEvent::Focused(false)
                    if self.context_preview_mode && !self.task_switcher_active =>
                {
                    self.hide_context_menu();
                }
                WindowEvent::Focused(false)
                    if self.context_menu_target.is_some()
                        && !self.context_preview_mode
                        && !self.task_switcher_active =>
                {
                    self.hide_context_menu();
                }
                WindowEvent::Resized(size) => {
                    if self.context_menu_gpu.is_none() {
                        self.ensure_context_menu_gpu();
                    }
                    if let Some(gpu) = &mut self.context_menu_gpu {
                        gpu.resize(size.width, size.height);
                    }
                }
                WindowEvent::RedrawRequested => {
                    if self.context_menu_gpu.is_none() {
                        self.ensure_context_menu_gpu();
                    }
                    if let Some(gpu) = &mut self.context_menu_gpu {
                        gpu.render(self.context_menu_hovered);
                    }
                }
                WindowEvent::CursorMoved { position, .. } => {
                    self.preview_hide_deadline = None;
                    let hovered = if self.context_preview_mode {
                        context_menu::preview_at(position, self.context_menu_actions.len())
                    } else {
                        context_menu::item_at(position, self.context_menu_actions.len())
                    };
                    let close_hovered = self
                        .context_preview_mode
                        .then(|| {
                            context_menu::preview_close_at(
                                position,
                                self.context_menu_actions.len(),
                            )
                        })
                        .flatten();
                    if self.context_preview_mode && hovered != self.context_menu_hovered {
                        if let Some(window) = hovered.and_then(|index| {
                            self.context_menu_actions
                                .get(index)
                                .map(|action| match action {
                                    ContextAction::Activate(window)
                                    | ContextAction::Close(window)
                                    | ContextAction::Maximize(window)
                                    | ContextAction::Minimize(window) => *window,
                                })
                        }) {
                            platform::send_shell_command(ShellCommand::HighlightWindow(window));
                        } else {
                            platform::send_shell_command(ShellCommand::ClearWindowHighlight);
                        }
                    }
                    if hovered != self.context_menu_hovered
                        || close_hovered != self.context_close_hovered
                    {
                        self.context_menu_hovered = hovered;
                        self.context_close_hovered = close_hovered;
                        self.context_menu_window
                            .as_ref()
                            .expect("context menu window exists")
                            .request_redraw();
                    }
                }
                WindowEvent::CursorLeft { .. } => {
                    if self.context_preview_mode {
                        platform::send_shell_command(ShellCommand::ClearWindowHighlight);
                    }
                    self.context_menu_hovered = None;
                    self.context_close_hovered = None;
                    if self.context_preview_mode {
                        self.preview_hide_deadline =
                            Some(Instant::now() + Duration::from_millis(250));
                    }
                    self.context_menu_window
                        .as_ref()
                        .expect("context menu window exists")
                        .request_redraw();
                }
                WindowEvent::MouseInput {
                    state: ElementState::Pressed,
                    button: MouseButton::Left,
                    ..
                } if self.context_menu_hovered.is_some() => {
                    let index = self.context_menu_hovered.expect("hovered item exists");
                    let action = if self.context_close_hovered == Some(index) {
                        self.context_menu_actions
                            .get(index)
                            .map(|action| match action {
                                ContextAction::Activate(window) => ContextAction::Close(*window),
                                other => *other,
                            })
                    } else {
                        self.context_menu_actions.get(index).copied()
                    };
                    if let Some(action) = action {
                        let (window, action) = match action {
                            ContextAction::Activate(window) => (window, WindowAction::Activate),
                            ContextAction::Close(window) => (window, WindowAction::Close),
                            ContextAction::Maximize(window) => (window, WindowAction::Maximize),
                            ContextAction::Minimize(window) => (window, WindowAction::Minimize),
                        };
                        platform::send_shell_command(ShellCommand::WindowAction { window, action });
                    }
                    self.hide_context_menu();
                }
                WindowEvent::MouseInput {
                    state: ElementState::Pressed,
                    button: MouseButton::Right,
                    ..
                } if self.context_preview_mode && self.context_menu_hovered.is_some() => {
                    self.show_window_actions(
                        self.context_menu_hovered.expect("preview is hovered"),
                    );
                }
                _ => {}
            }
            return;
        }
        if self.run_window.as_ref().map(|window| window.id()) == Some(window_id) {
            match event {
                WindowEvent::CloseRequested => self.hide_run(),
                WindowEvent::Resized(size) => {
                    if let Some(gpu) = &mut self.run_gpu {
                        gpu.resize(size.width, size.height);
                    }
                }
                WindowEvent::RedrawRequested => {
                    let displayed_command = self.run_editor.display_text_with_caret("▏");
                    if let Some(gpu) = &mut self.run_gpu {
                        gpu.render(
                            &displayed_command,
                            self.run_prompt.history(),
                            self.run_prompt.history_open(),
                            self.run_prompt.history_selection(),
                            self.run_hovered,
                        );
                    }
                }
                WindowEvent::CursorMoved { position, .. } => {
                    let hovered = run_dialog::action_at(
                        position,
                        self.run_prompt.history_open(),
                        self.run_prompt.history().len(),
                    );
                    if hovered != self.run_hovered {
                        self.run_hovered = hovered;
                        self.run_window
                            .as_ref()
                            .expect("run window exists")
                            .request_redraw();
                    }
                }
                WindowEvent::CursorLeft { .. } => {
                    self.run_hovered = None;
                    self.run_window
                        .as_ref()
                        .expect("run window exists")
                        .request_redraw();
                }
                WindowEvent::MouseInput {
                    state: ElementState::Pressed,
                    button: MouseButton::Left,
                    ..
                } => match self.run_hovered {
                    Some(run_dialog::Action::Run) => self.submit_run(),
                    Some(run_dialog::Action::Cancel) => self.hide_run(),
                    Some(run_dialog::Action::HistoryToggle) => {
                        self.run_prompt.toggle_history();
                        self.run_window
                            .as_ref()
                            .expect("run window exists")
                            .request_redraw();
                    }
                    Some(run_dialog::Action::HistoryItem(index)) => {
                        self.run_prompt.choose_history(index);
                        if let Some(command) = self.run_prompt.selected_history_command() {
                            self.run_editor.set_text(command);
                        }
                        self.run_window
                            .as_ref()
                            .expect("run window exists")
                            .request_redraw();
                    }
                    Some(run_dialog::Action::Browse) | None => {}
                },
                WindowEvent::ModifiersChanged(modifiers) => {
                    self.run_modifiers = modifiers.state();
                }
                WindowEvent::Ime(Ime::Preedit(text, cursor)) => {
                    self.run_editor
                        .set_preedit(text, cursor.map(|(start, end)| start..end));
                    self.run_window
                        .as_ref()
                        .expect("run window exists")
                        .request_redraw();
                }
                WindowEvent::Ime(Ime::Commit(text)) => {
                    self.run_editor.commit_preedit(&text);
                    self.run_window
                        .as_ref()
                        .expect("run window exists")
                        .request_redraw();
                }
                WindowEvent::Ime(Ime::Disabled) => {
                    self.run_editor.cancel_preedit();
                    self.run_window
                        .as_ref()
                        .expect("run window exists")
                        .request_redraw();
                }
                WindowEvent::Ime(Ime::Enabled) => {}
                WindowEvent::KeyboardInput { event, .. }
                    if event.state == ElementState::Released
                        && matches!(
                            &event.logical_key,
                            Key::Character(text) if text.eq_ignore_ascii_case("r")
                        ) =>
                {
                    self.run_ignoring_trigger_r = false;
                }
                WindowEvent::KeyboardInput { event, .. }
                    if event.state == ElementState::Pressed =>
                {
                    let mut changed = true;
                    match event.logical_key {
                        Key::Named(NamedKey::Backspace) => self.run_editor.backspace(),
                        Key::Named(NamedKey::Delete) => self.run_editor.delete(),
                        Key::Named(NamedKey::ArrowLeft) => {
                            self.run_editor.move_left(self.run_modifiers.shift_key())
                        }
                        Key::Named(NamedKey::ArrowRight) => {
                            self.run_editor.move_right(self.run_modifiers.shift_key())
                        }
                        Key::Named(NamedKey::Home) => {
                            self.run_editor.move_home(self.run_modifiers.shift_key())
                        }
                        Key::Named(NamedKey::End) => {
                            self.run_editor.move_end(self.run_modifiers.shift_key())
                        }
                        Key::Named(NamedKey::Escape) => {
                            if self.run_editor.preedit().is_empty() {
                                self.hide_run();
                                changed = false;
                            } else {
                                self.run_editor.cancel_preedit();
                            }
                        }
                        Key::Named(NamedKey::Enter) => {
                            self.submit_run();
                            changed = false;
                        }
                        Key::Named(NamedKey::ArrowDown) => {
                            self.run_prompt.select_history_next();
                            if let Some(command) = self.run_prompt.selected_history_command() {
                                self.run_editor.set_text(command);
                            }
                        }
                        Key::Named(NamedKey::ArrowUp) => {
                            self.run_prompt.select_history_previous();
                            if let Some(command) = self.run_prompt.selected_history_command() {
                                self.run_editor.set_text(command);
                            }
                        }
                        Key::Named(NamedKey::Space) => self.run_editor.insert(" "),
                        Key::Character(text) => {
                            if self.run_ignoring_trigger_r && text.eq_ignore_ascii_case("r") {
                                changed = false;
                            } else if self.run_modifiers.control_key()
                                && text.eq_ignore_ascii_case("a")
                            {
                                self.run_editor.select_all();
                            } else if let Some(pasted) = platform::paste_text_if_requested(&text) {
                                self.run_editor.insert(&pasted);
                            } else if self.run_modifiers.control_key() {
                                changed = false;
                            } else {
                                self.run_ignoring_trigger_r = false;
                                self.run_editor.insert(&text);
                            }
                        }
                        _ => changed = false,
                    }
                    if changed {
                        self.run_window
                            .as_ref()
                            .expect("run window exists")
                            .request_redraw();
                    }
                }
                _ => {}
            }
            return;
        }

        if let Some(panel_index) = self
            .panel_surfaces
            .iter()
            .position(|(window, _)| window.id() == window_id)
        {
            match event {
                WindowEvent::CloseRequested => event_loop.exit(),
                WindowEvent::Resized(size) => {
                    self.panel_surfaces[panel_index]
                        .1
                        .resize(size.width, size.height);
                    self.panel_surfaces[panel_index].0.request_redraw();
                }
                WindowEvent::RedrawRequested => {
                    self.panel_surfaces[panel_index].1.render(
                        self.panel_hovered,
                        self.panel_task_hovered,
                        self.panel_desktop_hovered,
                    );
                }
                WindowEvent::CursorMoved { position, .. } => {
                    let hovered = panel::launcher_button_contains(position);
                    let task_hovered = panel::task_at(position, self.window_groups.len());
                    let panel_width = self.panel_surfaces[panel_index].0.inner_size().width;
                    let tray_hovered = panel::tray_at(position, panel_width, self.tray_items.len());
                    let control_hovered = panel::control_center_contains(position, panel_width);
                    let desktop_hovered = panel::desktop_at(
                        position,
                        panel_width,
                        self.tray_items.len(),
                        self.shell_settings.desktop_count,
                    );
                    if hovered == self.panel_hovered
                        && task_hovered == self.panel_task_hovered
                        && tray_hovered == self.panel_tray_hovered
                        && desktop_hovered == self.panel_desktop_hovered
                        && control_hovered == self.panel_control_hovered
                    {
                        return;
                    }
                    self.panel_hovered = hovered;
                    self.panel_task_hovered = task_hovered;
                    self.panel_tray_hovered = tray_hovered;
                    self.panel_desktop_hovered = desktop_hovered;
                    self.panel_control_hovered = control_hovered;
                    if let Some(index) =
                        task_hovered.filter(|_| self.window_feed.supports_previews())
                    {
                        self.show_window_group(index);
                    } else if self.context_preview_mode {
                        self.preview_hide_deadline =
                            Some(Instant::now() + Duration::from_millis(250));
                    }
                    let window = &self.panel_surfaces[panel_index].0;
                    window.set_cursor(
                        if hovered
                            || task_hovered.is_some()
                            || tray_hovered.is_some()
                            || desktop_hovered.is_some()
                            || control_hovered
                        {
                            CursorIcon::Pointer
                        } else {
                            CursorIcon::Default
                        },
                    );
                    window.request_redraw();
                }
                WindowEvent::CursorLeft { .. } => {
                    self.panel_hovered = false;
                    self.panel_task_hovered = None;
                    self.panel_tray_hovered = None;
                    self.panel_desktop_hovered = None;
                    self.panel_control_hovered = false;
                    if self.context_preview_mode {
                        self.preview_hide_deadline =
                            Some(Instant::now() + Duration::from_millis(250));
                    }
                    let window = &self.panel_surfaces[panel_index].0;
                    window.set_cursor(CursorIcon::Default);
                    window.request_redraw();
                }
                WindowEvent::Focused(false) if self.context_preview_mode => {
                    self.hide_context_menu();
                }
                WindowEvent::MouseInput {
                    state: ElementState::Pressed,
                    button: MouseButton::Left,
                    ..
                } if self.panel_hovered => {
                    if let Some(launcher) = &self.window
                        && let Some(monitor) = self.panel_surfaces[panel_index].0.current_monitor()
                    {
                        let size = launcher.inner_size();
                        launcher.set_outer_position(launcher_position(
                            &monitor,
                            (size.width, size.height),
                        ));
                    }
                    self.toggle_launcher();
                }
                WindowEvent::MouseInput {
                    state: ElementState::Pressed,
                    button: MouseButton::Left,
                    ..
                } if self.panel_control_hovered => self.toggle_control_center(),
                WindowEvent::MouseInput {
                    state: ElementState::Pressed,
                    button: MouseButton::Left,
                    ..
                } if self.panel_desktop_hovered.is_some() => {
                    self.shell_settings.active_desktop =
                        self.panel_desktop_hovered.expect("desktop is hovered");
                    let _ = self.shell_settings.save_default();
                    for (window, gpu) in &mut self.panel_surfaces {
                        gpu.set_desktops(
                            self.shell_settings.desktop_count,
                            self.shell_settings.active_desktop,
                        );
                        window.request_redraw();
                    }
                }
                WindowEvent::MouseInput {
                    state: ElementState::Pressed,
                    button: MouseButton::Left,
                    ..
                } if self.panel_tray_hovered.is_some() => {
                    let index = self.panel_tray_hovered.expect("tray item is hovered");
                    if let Some(item) = self.tray_items.get(index) {
                        self.tray_feed.activate(&item.id);
                    }
                }
                WindowEvent::MouseInput {
                    state: ElementState::Pressed,
                    button: MouseButton::Right,
                    ..
                } => {
                    if let Some(index) = self.panel_tray_hovered {
                        if let Some(item) = self.tray_items.get(index) {
                            self.tray_feed.context_menu(&item.id);
                        }
                    } else if let Some(index) = self.panel_task_hovered {
                        self.show_context_menu(index);
                    } else {
                        self.hide_context_menu();
                    }
                }
                WindowEvent::MouseInput {
                    state: ElementState::Pressed,
                    button: MouseButton::Left,
                    ..
                } => {
                    if let Some(index) = self.panel_task_hovered {
                        if self.window_feed.supports_previews()
                            && self
                                .window_groups
                                .get(index)
                                .is_some_and(|group| group.windows.len() > 1)
                        {
                            self.show_window_group(index);
                            if self.context_preview_mode {
                                return;
                            }
                        }
                        if let Some(window) = self
                            .window_groups
                            .get(index)
                            .and_then(group_activation_target)
                        {
                            platform::send_shell_command(ShellCommand::WindowAction {
                                window: window.id,
                                action: WindowAction::Activate,
                            });
                        }
                    }
                    self.hide_context_menu();
                }
                _ => {}
            }
            return;
        }

        if self.window.as_ref().map(|window| window.id()) != Some(window_id) {
            return;
        }

        match event {
            WindowEvent::CloseRequested => self.set_launcher_visible(false),
            WindowEvent::Focused(true) => {
                #[cfg(not(target_os = "windows"))]
                {
                    self.launcher_focus_loss_deadline = None;
                }
            }
            WindowEvent::Focused(false) if self.launcher_visibility.is_visible() => {
                self.launcher_focus_loss_deadline =
                    Some(Instant::now() + Duration::from_millis(100));
            }
            WindowEvent::Resized(size) => {
                if let Some(gpu) = &mut self.gpu {
                    gpu.resize(size.width, size.height);
                }
                self.set_scroll_offset(self.scroll_offset);
                self.window
                    .as_ref()
                    .expect("window exists")
                    .request_redraw();
            }
            WindowEvent::RedrawRequested => {
                if let Some(gpu) = &mut self.gpu {
                    gpu.render(
                        &self.launcher,
                        &self.launcher_editor,
                        self.hovered_result,
                        self.scroll_offset,
                    );
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor_position = Some(position);
                if let Some(grab_offset) = self.scrollbar_drag_offset {
                    let (width, height, capacity) = self.viewport_metrics();
                    if let Some(scrollbar) = layout::scrollbar(
                        width,
                        height,
                        self.launcher.result_count(),
                        capacity,
                        self.scroll_offset,
                    ) {
                        let offset = layout::offset_from_thumb_y(
                            position.y - grab_offset,
                            scrollbar,
                            self.launcher.result_count(),
                            capacity,
                        );
                        self.set_scroll_offset(offset);
                        self.hovered_result = None;
                        self.window
                            .as_ref()
                            .expect("window exists")
                            .request_redraw();
                    }
                    return;
                }
                let (width, _, capacity) = self.viewport_metrics();
                let local_count = self
                    .launcher
                    .result_count()
                    .saturating_sub(self.scroll_offset)
                    .min(capacity);
                let hovered = hit_test_result(position, width, local_count)
                    .map(|index| index + self.scroll_offset);
                if hovered != self.hovered_result {
                    self.hovered_result = hovered;
                    let window = self.window.as_ref().expect("window exists");
                    window.set_cursor(if hovered.is_some() {
                        CursorIcon::Pointer
                    } else {
                        CursorIcon::Default
                    });
                    window.request_redraw();
                }
            }
            WindowEvent::CursorLeft { .. } => {
                self.cursor_position = None;
                self.hovered_result = None;
                let window = self.window.as_ref().expect("window exists");
                window.set_cursor(CursorIcon::Default);
                window.request_redraw();
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let rows = match delta {
                    MouseScrollDelta::LineDelta(_, y) => -y.round() as i32,
                    MouseScrollDelta::PixelDelta(position) => (-position.y / 52.0).round() as i32,
                };
                if rows != 0 {
                    self.scroll_by(rows * layout::GRID_COLUMNS as i32);
                    self.hovered_result = None;
                    self.window
                        .as_ref()
                        .expect("window exists")
                        .request_redraw();
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => {
                let (width, height, capacity) = self.viewport_metrics();
                let searching = !self.launcher_editor.text().is_empty()
                    || !self.launcher_editor.preedit().is_empty();
                if let (Some(position), Some(scrollbar)) = (
                    self.cursor_position,
                    searching
                        .then(|| {
                            layout::scrollbar(
                                width,
                                height,
                                self.launcher.result_count(),
                                capacity,
                                self.scroll_offset,
                            )
                        })
                        .flatten(),
                ) {
                    if layout::rect_contains(scrollbar.thumb, position.x, position.y) {
                        self.scrollbar_drag_offset =
                            Some(position.y - f64::from(scrollbar.thumb.y));
                        return;
                    }
                    if layout::rect_contains(scrollbar.track, position.x, position.y) {
                        if position.y < f64::from(scrollbar.thumb.y) {
                            self.scroll_by(-(capacity as i32));
                        } else {
                            self.scroll_by(capacity as i32);
                        }
                        self.window
                            .as_ref()
                            .expect("window exists")
                            .request_redraw();
                        return;
                    }
                }
                if let Some(index) = self.hovered_result {
                    self.launcher.select(index);
                    self.launch_result(index);
                    self.window
                        .as_ref()
                        .expect("window exists")
                        .request_redraw();
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: MouseButton::Left,
                ..
            } => self.scrollbar_drag_offset = None,
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Right,
                ..
            } => {
                if let Some(index) = self.hovered_result
                    && let Some(application_id) = self
                        .launcher
                        .result_at(index)
                        .map(|application| application.id().to_owned())
                    && let Some(store) = &self.pin_store
                {
                    match store.toggle(&application_id).and_then(|_| store.pins()) {
                        Ok(pins) => {
                            self.launcher.set_pins(pins);
                            self.scroll_offset = 0;
                            self.hovered_result = None;
                            self.window
                                .as_ref()
                                .expect("window exists")
                                .request_redraw();
                        }
                        Err(error) => eprintln!("failed to update pin: {error}"),
                    }
                }
            }
            WindowEvent::Ime(Ime::Preedit(text, cursor)) => {
                self.launcher_editor
                    .set_preedit(text, cursor.map(|(start, end)| start..end));
                self.window
                    .as_ref()
                    .expect("window exists")
                    .request_redraw();
            }
            WindowEvent::Ime(Ime::Commit(text)) => {
                self.launcher_editor.commit_preedit(&text);
                self.launcher.set_query(self.launcher_editor.text());
                self.scroll_offset = 0;
                self.ensure_selection_visible();
                self.window
                    .as_ref()
                    .expect("window exists")
                    .request_redraw();
            }
            WindowEvent::Ime(Ime::Disabled) => {
                self.launcher_editor.cancel_preedit();
                self.window
                    .as_ref()
                    .expect("window exists")
                    .request_redraw();
            }
            WindowEvent::Ime(Ime::Enabled) => {}
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                let mut changed = true;
                let mut query_changed = false;
                match event.logical_key {
                    Key::Named(NamedKey::ArrowDown) => {
                        self.launcher.select_relative(layout::GRID_COLUMNS as isize)
                    }
                    Key::Named(NamedKey::ArrowUp) => self
                        .launcher
                        .select_relative(-(layout::GRID_COLUMNS as isize)),
                    Key::Named(NamedKey::ArrowRight) => self.launcher.select_relative(1),
                    Key::Named(NamedKey::ArrowLeft) => self.launcher.select_relative(-1),
                    Key::Named(NamedKey::Backspace) => {
                        self.launcher_editor.backspace();
                        query_changed = true;
                    }
                    Key::Named(NamedKey::Escape)
                        if self.launcher_editor.text().is_empty()
                            && self.launcher_editor.preedit().is_empty() =>
                    {
                        self.set_launcher_visible(false);
                    }
                    Key::Named(NamedKey::Escape) if !self.launcher_editor.preedit().is_empty() => {
                        self.launcher_editor.cancel_preedit();
                    }
                    Key::Named(NamedKey::Escape) => {
                        self.launcher_editor.clear();
                        query_changed = true;
                    }
                    Key::Named(NamedKey::Enter) => {
                        self.launch_result(self.launcher.selected_index());
                        changed = false;
                    }
                    Key::Character(text) if self.launcher_editor.preedit().is_empty() => {
                        self.launcher_editor.insert(&text);
                        query_changed = true;
                    }
                    _ => changed = false,
                }
                if query_changed {
                    self.launcher.set_query(self.launcher_editor.text());
                    self.scroll_offset = 0;
                }
                if changed {
                    self.ensure_selection_visible();
                    self.window
                        .as_ref()
                        .expect("window exists")
                        .request_redraw();
                }
            }
            _ => {}
        }
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        #[cfg(target_os = "windows")]
        for (window, _) in &self.panel_surfaces {
            platform::release_panel_window(window);
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        for (window, gpu) in &self.desktop_surfaces {
            if gpu.is_animated() {
                window.request_redraw();
            }
        }
        while let Ok(shortcut) = self.launcher_hotkey_rx.try_recv() {
            match shortcut {
                GlobalShortcut::ShowLauncher => self.set_launcher_visible(true),
                GlobalShortcut::HideLauncher => self.set_launcher_visible(false),
                GlobalShortcut::ShowRun => self.show_run(),
                GlobalShortcut::SwitchNext => self.advance_task_switcher(true, false),
                GlobalShortcut::SwitchPrevious => self.advance_task_switcher(false, false),
                GlobalShortcut::SwitchGroupNext => self.advance_task_switcher(true, true),
                GlobalShortcut::SwitchGroupPrevious => self.advance_task_switcher(false, true),
                GlobalShortcut::CommitSwitch => self.commit_task_switcher(),
                GlobalShortcut::CaptureActiveWindow => {
                    if let Err(error) = platform::capture_active_window() {
                        tracing::warn!(%error, "failed to copy active window screenshot");
                    }
                }
                GlobalShortcut::CaptureActiveWindowToFile => {
                    if let Err(error) = platform::capture_active_window_to_file() {
                        tracing::warn!(%error, "failed to capture active window to a temporary file");
                    }
                }
                GlobalShortcut::ShowScreenshotTool => {
                    if let Some(tool) = &mut self.screenshot_tool {
                        tool.hide();
                    }
                    self.screenshot_capture_deadline =
                        Some(Instant::now() + Duration::from_millis(75));
                }
                GlobalShortcut::AudioChanged {
                    volume_percent,
                    muted,
                } => self.show_volume_osd(volume_percent, muted),
            }
        }
        if self
            .screenshot_capture_deadline
            .is_some_and(|deadline| now >= deadline)
        {
            self.screenshot_capture_deadline = None;
            match platform::capture_desktop() {
                Ok(capture) => {
                    if let Some(tool) = &mut self.screenshot_tool {
                        tool.show(capture.image);
                    }
                }
                Err(error) => tracing::warn!(%error, "failed to capture desktop"),
            }
        }
        if self
            .volume_osd_deadline
            .is_some_and(|deadline| now >= deadline)
        {
            if let Some(window) = &self.volume_osd_window {
                window.set_visible(false);
            }
            self.volume_osd_deadline = None;
            self.volume_osd_gpu = None;
        }
        if self
            .launcher_focus_loss_deadline
            .is_some_and(|deadline| now >= deadline)
        {
            if platform::launcher_has_foreground_focus() {
                #[cfg(target_os = "windows")]
                {
                    self.launcher_focus_loss_deadline = Some(now + Duration::from_millis(100));
                }
                #[cfg(not(target_os = "windows"))]
                {
                    self.launcher_focus_loss_deadline = None;
                }
            } else {
                self.set_launcher_visible(false);
            }
        }
        if self
            .preview_hide_deadline
            .is_some_and(|deadline| now >= deadline)
        {
            self.hide_context_menu();
        }
        if now >= self.window_deadline {
            self.refresh_task_windows();
            self.refresh_tray_items();
            if let Some(group) = self.preview_group
                && (self.panel_task_hovered == Some(group) || self.context_menu_hovered.is_some())
            {
                self.show_window_group(group);
            }
            self.window_deadline = now + Duration::from_millis(250);
        }
        if now >= self.fullscreen_deadline {
            platform::update_panel_fullscreen_state();
            self.fullscreen_deadline = now + Duration::from_secs(1);
        }
        if now >= self.settings_deadline {
            self.refresh_shell_settings();
            self.settings_deadline = now + Duration::from_millis(500);
        }
        if now >= self.display_deadline {
            self.reconcile_display_topology(event_loop);
            self.display_deadline = now + Duration::from_secs(1);
        }
        if now >= self.clock_deadline {
            for (window, gpu) in &mut self.panel_surfaces {
                if gpu.update_clock() {
                    window.request_redraw();
                }
            }
            self.clock_deadline = next_minute_deadline(now, SystemTime::now());
        }
        let mut deadline = self
            .clock_deadline
            .min(self.window_deadline)
            .min(self.fullscreen_deadline)
            .min(self.settings_deadline)
            .min(self.display_deadline);
        if let Some(preview_deadline) = self.preview_hide_deadline {
            deadline = deadline.min(preview_deadline);
        }
        if let Some(focus_deadline) = self.launcher_focus_loss_deadline {
            deadline = deadline.min(focus_deadline);
        }
        if let Some(capture_deadline) = self.screenshot_capture_deadline {
            deadline = deadline.min(capture_deadline);
        }
        #[cfg(target_os = "windows")]
        let deadline = deadline.min(now + Duration::from_millis(25));
        event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
    }
}

fn focused_shortcut_edge(event: &winit::event::KeyEvent) -> Option<(Hotkey, KeyEdge)> {
    let key = match &event.logical_key {
        Key::Named(NamedKey::Alt) => Hotkey::Alt,
        Key::Named(NamedKey::Shift) => Hotkey::Shift,
        Key::Named(NamedKey::Tab) => Hotkey::Tab,
        Key::Named(NamedKey::PrintScreen) => Hotkey::PrintScreen,
        Key::Character(character) if character.as_str() == "`" => Hotkey::Grave,
        _ => return None,
    };
    let edge = match event.state {
        ElementState::Pressed => KeyEdge::Pressed,
        ElementState::Released => KeyEdge::Released,
    };
    Some((key, edge))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _log_path = nickel_logging::init("nickel-ui").ok();
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Wait);
    event_loop.run_app(&mut Nickel::default())?;
    Ok(())
}

fn env_flag(name: &str) -> bool {
    env::var(name).is_ok_and(|value| {
        matches!(
            value.to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

fn next_minute_deadline(now: Instant, wall_clock: SystemTime) -> Instant {
    const NANOS_PER_MINUTE: u128 = 60_000_000_000;
    let elapsed = wall_clock.duration_since(UNIX_EPOCH).unwrap_or_default();
    let into_minute = elapsed.as_nanos() % NANOS_PER_MINUTE;
    let remaining = (NANOS_PER_MINUTE - into_minute) as u64;
    now + Duration::from_nanos(remaining)
}

fn centered_position(monitor: &MonitorHandle, window_size: (u32, u32)) -> PhysicalPosition<i32> {
    let origin = monitor.position();
    let size = monitor.size();
    let x = origin.x + (size.width.saturating_sub(window_size.0) / 2) as i32;
    let y = origin.y + (size.height.saturating_sub(window_size.1) / 2) as i32;
    PhysicalPosition::new(x, y)
}

fn launcher_position(monitor: &MonitorHandle, window_size: (u32, u32)) -> PhysicalPosition<i32> {
    let origin = monitor.position();
    let size = monitor.size();
    PhysicalPosition::new(
        origin.x,
        origin.y
            + size
                .height
                .saturating_sub(PANEL_HEIGHT.round() as u32)
                .saturating_sub(window_size.1) as i32,
    )
}

fn sorted_monitors(event_loop: &ActiveEventLoop) -> Vec<MonitorHandle> {
    let mut monitors: Vec<_> = event_loop.available_monitors().collect();
    monitors.sort_by_key(|monitor| {
        (
            monitor.name().unwrap_or_default(),
            monitor.position().x,
            monitor.position().y,
        )
    });
    monitors
}

fn display_topology(monitors: &[MonitorHandle]) -> Vec<(String, i32, i32, u32, u32, u64)> {
    monitors
        .iter()
        .map(|monitor| {
            let position = monitor.position();
            let size = monitor.size();
            (
                monitor.name().unwrap_or_default(),
                position.x,
                position.y,
                size.width,
                size.height,
                monitor.scale_factor().to_bits(),
            )
        })
        .collect()
}

fn panel_layout(
    origin: PhysicalPosition<i32>,
    monitor_size: PhysicalSize<u32>,
    _scale_factor: f64,
) -> (PhysicalSize<u32>, PhysicalPosition<i32>) {
    let physical_height = PANEL_HEIGHT.round() as u32;
    let y = origin.y + monitor_size.height.saturating_sub(physical_height) as i32;
    (
        PhysicalSize::new(monitor_size.width, physical_height),
        PhysicalPosition::new(origin.x, y),
    )
}

fn stable_window_group_order(
    previous: &[WindowGroup],
    mut current: Vec<WindowGroup>,
) -> Vec<WindowGroup> {
    let mut ordered = Vec::with_capacity(current.len());
    for old in previous {
        if let Some(index) = current.iter().position(|new| same_window_group(old, new)) {
            ordered.push(current.remove(index));
        }
    }
    ordered.extend(current);
    ordered
}

fn same_window_group(left: &WindowGroup, right: &WindowGroup) -> bool {
    match (&left.application_id, &right.application_id) {
        (Some(left), Some(right)) => left == right,
        (None, None) => {
            left.windows.first().map(|window| window.id)
                == right.windows.first().map(|window| window.id)
        }
        _ => false,
    }
}

fn window_group_key(group: &WindowGroup) -> String {
    group.application_id.as_ref().map_or_else(
        || {
            format!(
                "window:{}",
                group.windows.first().map_or(0, |window| window.id.0)
            )
        },
        |application| format!("application:{}", application.as_str()),
    )
}

fn group_activation_target(group: &WindowGroup) -> Option<&OpenWindow> {
    let active = group.windows.iter().position(|window| window.active);
    match active {
        Some(index) if group.windows.len() > 1 => {
            group.windows.get((index + 1) % group.windows.len())
        }
        Some(index) => group.windows.get(index),
        None => group.windows.first(),
    }
}

fn hit_test_result(
    position: PhysicalPosition<f64>,
    window_width: u32,
    result_count: usize,
) -> Option<usize> {
    layout::hit_test_result(position.x, position.y, window_width, result_count)
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant, UNIX_EPOCH};

    use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};

    use crate::model::{ApplicationId, OpenWindow, WindowGroup, WindowId as ShellWindowId};

    use super::{
        LauncherVisibility, env_flag, group_activation_target, next_minute_deadline, panel_layout,
        stable_window_group_order,
    };

    fn window_group(application: &str, window: u64, active: bool) -> WindowGroup {
        WindowGroup {
            application_id: Some(ApplicationId::new(application)),
            application_name: application.into(),
            windows: vec![OpenWindow {
                id: ShellWindowId(window),
                application_id: Some(ApplicationId::new(application)),
                active,
                title: application.into(),
            }],
        }
    }

    #[test]
    fn window_groups_keep_panel_slots_when_z_order_changes() {
        let previous = vec![
            window_group("powershell", 1, true),
            window_group("chrome", 2, false),
        ];
        let refreshed = vec![
            window_group("chrome", 2, true),
            window_group("powershell", 1, false),
        ];
        let ordered = stable_window_group_order(&previous, refreshed);
        assert_eq!(
            ordered
                .iter()
                .map(|group| group.application_name.as_str())
                .collect::<Vec<_>>(),
            ["powershell", "chrome"]
        );
        assert!(!ordered[0].active());
        assert!(ordered[1].active());
    }

    #[test]
    fn grouped_task_activates_front_window_then_cycles_when_active() {
        let mut group = window_group("chrome", 10, false);
        group.windows.push(OpenWindow {
            id: ShellWindowId(11),
            application_id: Some(ApplicationId::new("chrome")),
            active: false,
            title: "other chrome".into(),
        });
        assert_eq!(
            group_activation_target(&group).map(|window| window.id),
            Some(ShellWindowId(10))
        );
        group.windows[0].active = true;
        assert_eq!(
            group_activation_target(&group).map(|window| window.id),
            Some(ShellWindowId(11))
        );
    }

    #[test]
    fn panel_keeps_consistent_physical_height_across_display_scales() {
        let (size, position) = panel_layout(
            PhysicalPosition::new(1920, 0),
            PhysicalSize::new(2560, 1440),
            1.25,
        );
        assert_eq!(size, PhysicalSize::new(2560, 56));
        assert_eq!(position, PhysicalPosition::new(1920, 1384));
    }

    #[test]
    fn clock_deadline_targets_the_next_minute_boundary() {
        let now = Instant::now();
        let wall_clock = UNIX_EPOCH + Duration::from_secs(125) + Duration::from_millis(250);
        assert_eq!(
            next_minute_deadline(now, wall_clock).duration_since(now),
            Duration::from_millis(54_750)
        );
    }

    #[test]
    fn launcher_tile_labels_are_bounded_without_splitting_unicode() {
        assert_eq!(
            super::compact_tile_label("Control Panel", 25),
            "Control Panel"
        );
        assert_eq!(
            super::compact_tile_label("Documentation for Desktop Applications", 25),
            "Documentation for Deskto…"
        );
    }

    #[test]
    fn launcher_visibility_toggles_without_recreation() {
        let mut visibility = LauncherVisibility::default();
        assert!(!visibility.is_visible());
        assert!(visibility.toggle());
        assert!(!visibility.toggle());
        visibility.set(true);
        assert!(visibility.is_visible());
    }

    #[test]
    fn missing_environment_flag_is_disabled() {
        let name = "NICKEL_TEST_MISSING_FLAG";
        // SAFETY: This test uses a unique variable name and no other thread accesses it.
        unsafe { std::env::remove_var(name) };
        assert!(!env_flag(name));
    }

    #[test]
    fn common_true_values_enable_environment_flag() {
        let name = "NICKEL_TEST_TRUE_FLAG";
        for value in ["1", "true", "TRUE", "yes", "on"] {
            // SAFETY: This test uses a unique variable name and no other thread accesses it.
            unsafe { std::env::set_var(name, value) };
            assert!(env_flag(name));
        }
        // SAFETY: This test uses a unique variable name and no other thread accesses it.
        unsafe { std::env::remove_var(name) };
    }
}
