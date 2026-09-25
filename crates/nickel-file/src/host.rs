use std::time::{Duration, Instant};

use nickel_input::{
    AggregateModifier, InputEvent, KeyCode, KeyEdge, PhysicalKey, PointerButton, PointerEvent,
};
use nickel_ui::{
    AdapterOutcome, Application, ControllerFence, EventDisposition, HostAdapter, HostServices,
    Point, ReadingDirection, SemanticNodeSnapshot, ShortcutOutcome, UiHost,
};
use winit::{
    dpi::LogicalSize,
    window::{Icon, Window},
};

use crate::{
    app::{FileApp, FileMessage, MAX_SIDEBAR_WIDTH, MIN_SIDEBAR_WIDTH},
    layout::{entries_in_selection, rect_between},
};

fn set_nickel_file_icon(window: &Window) {
    let Ok(image) =
        image::load_from_memory(include_bytes!("../../../assets/icons/nickel-file.png"))
    else {
        return;
    };
    let image = image.into_rgba8();
    let (width, height) = image.dimensions();
    if let Ok(icon) = Icon::from_rgba(image.into_raw(), width, height) {
        window.set_window_icon(Some(icon));
    }
}

/// Native-window integration for the standalone Nickel File executable.
///
/// Embedded hosts normally drive [`FileApp`] directly through the
/// [`nickel_ui::Application`] contract. This adapter remains public so a host
/// which already uses Nickel UI's native runtime can preserve the standalone
/// window behavior without asking this crate to create an event loop.
pub struct FileHostAdapter {
    sync_requested: bool,
    drop_hover_deadline: Option<Instant>,
    #[cfg(target_os = "windows")]
    context_popup_rx: Option<crate::windows_popup_menu::PopupSession<FileMessage>>,
    #[cfg(target_os = "windows")]
    context_popup_poll_at: Option<Instant>,
    #[cfg(target_os = "windows")]
    focused_shortcut: Option<std::sync::Arc<dyn Fn(KeyCode, KeyEdge) + Send + Sync>>,
}

#[cfg(target_os = "windows")]
impl FileHostAdapter {
    pub fn with_focused_shortcut_handler(
        mut self,
        shortcut: impl Fn(KeyCode, KeyEdge) + Send + Sync + 'static,
    ) -> Self {
        self.focused_shortcut = Some(std::sync::Arc::new(shortcut));
        self
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NavigationShortcut {
    Back,
    Forward,
    Up,
}

fn navigation_shortcut(key: KeyCode, alt_down: bool) -> Option<NavigationShortcut> {
    if !alt_down {
        return None;
    }
    match key {
        KeyCode::ArrowLeft => Some(NavigationShortcut::Back),
        KeyCode::ArrowRight => Some(NavigationShortcut::Forward),
        KeyCode::ArrowUp => Some(NavigationShortcut::Up),
        _ => None,
    }
}

fn perform_navigation_shortcut(app: &mut FileApp, shortcut: NavigationShortcut) -> ShortcutOutcome {
    let available = !app.navigation_pending()
        && match shortcut {
            NavigationShortcut::Back => app.browser.can_go_back(),
            NavigationShortcut::Forward => app.browser.can_go_forward(),
            NavigationShortcut::Up => app.browser.can_go_up(),
        };
    if !available {
        return ShortcutOutcome::rejected("navigation unavailable");
    }
    match shortcut {
        NavigationShortcut::Back => app.go_back(),
        NavigationShortcut::Forward => app.go_forward(),
        NavigationShortcut::Up => app.go_up(),
    }
    ShortcutOutcome::handled(true)
}

fn perform_tab_cycle(app: &mut FileApp, reverse: bool) -> ShortcutOutcome {
    let Some(index) = adjacent_tab_index(app.active_tab, app.tabs.len(), reverse) else {
        return ShortcutOutcome::rejected("tab cycle unavailable");
    };
    app.switch_tab(index);
    ShortcutOutcome::handled(true)
}

fn selection_command_modifier(modifiers: &nickel_input::ModifierState) -> bool {
    modifiers.aggregate(AggregateModifier::Control)
}

fn adjacent_tab_index(active: usize, count: usize, reverse: bool) -> Option<usize> {
    if count < 2 || active >= count {
        return None;
    }
    Some(if reverse {
        active.checked_sub(1).unwrap_or(count - 1)
    } else {
        (active + 1) % count
    })
}

fn cancel_transient_input_on_focus_loss(app: &mut FileApp) -> bool {
    let changed = app.rename_editor.is_some()
        || app.control_down
        || app.shift_down
        || app.resizing_sidebar
        || app.resizing_details_column.is_some()
        || app.selection_drag.is_some()
        || app.primary_down;
    // Inline rename never commits implicitly. Losing the parent window cancels
    // the edit, avoiding a partial or ambiguous filesystem write.
    app.rename_editor = None;
    app.control_down = false;
    app.shift_down = false;
    app.resizing_sidebar = false;
    app.resizing_details_column = None;
    app.selection_drag = None;
    app.primary_down = false;
    app.file_drag_origin = None;
    app.file_drag_started = false;
    app.outbound_drag = None;
    changed
}

fn point_in_node(point: Point, node: &SemanticNodeSnapshot) -> bool {
    point.x >= node.bounds.origin.x
        && point.y >= node.bounds.origin.y
        && point.x < node.bounds.origin.x + node.bounds.size.width
        && point.y < node.bounds.origin.y + node.bounds.size.height
}

/// Resolves native drops through production semantic geometry. The returned
/// provider path is snapshotted while pointer motion is available because the
/// native `DroppedFile` event itself carries source paths but no destination.
fn drop_destination_at(
    nodes: &[SemanticNodeSnapshot],
    point: Point,
    app: &FileApp,
) -> Option<std::path::PathBuf> {
    let hit_ids = nodes
        .iter()
        .filter(|node| point_in_node(point, node))
        .map(|node| node.id.as_str())
        .collect::<Vec<_>>();
    for id in &hit_ids {
        if let Some(index) = id
            .rsplit("/file-entry-")
            .next()
            .filter(|_| id.contains("/file-entry-"))
            .and_then(|index| index.parse::<usize>().ok())
            && let Some(entry) = app.browser.entries().get(index)
            && entry.is_directory
        {
            return Some(entry.path.clone());
        }
    }
    for index in 0..app.tabs.len() {
        if hit_ids
            .iter()
            .any(|id| id.ends_with(&format!("/file-drop-tab-{index}")))
        {
            return if index == app.active_tab {
                Some(app.browser.current().to_path_buf())
            } else {
                app.inactive_tab(index)
                    .map(|tab| tab.browser.current().to_path_buf())
            };
        }
    }
    let mut destinations = app
        .location_groups
        .iter()
        .flat_map(|group| group.entries.iter().map(|(_, path)| path.clone()))
        .chain(
            app.sidebar
                .children
                .values()
                .flat_map(|children| children.iter().map(|(_, path)| path.clone())),
        )
        .collect::<Vec<_>>();
    let mut ancestor = Some(app.browser.current());
    while let Some(path) = ancestor {
        destinations.push(path.to_path_buf());
        ancestor = path.parent();
    }
    for destination in destinations {
        for prefix in ["sidebar", "breadcrumb"] {
            let candidate = crate::app::drop_target_id(prefix, &destination);
            if hit_ids
                .iter()
                .any(|id| id.ends_with(&format!("/{candidate}")))
            {
                return Some(destination);
            }
        }
    }
    hit_ids
        .iter()
        .any(|id| id.ends_with("/file-content"))
        .then(|| app.browser.current().to_path_buf())
}

const DROP_HOVER_OPEN_DELAY: std::time::Duration = std::time::Duration::from_millis(700);

fn update_drop_hover(
    app: &mut FileApp,
    destination: Option<std::path::PathBuf>,
    now: Instant,
) -> bool {
    let changed = app.native_drop_destination != destination;
    app.native_drop_destination = destination.clone();
    if app.drag_hover.is_none() || destination.as_deref() == Some(app.browser.current()) {
        app.native_drop_hover_started = None;
        return changed;
    }
    if app
        .native_drop_hover_started
        .as_ref()
        .is_none_or(|(path, _)| Some(path) != destination.as_ref())
    {
        app.native_drop_hover_started = destination.map(|path| (path, now));
    }
    changed
}

fn open_drop_hover_target(app: &mut FileApp, now: Instant) -> bool {
    let Some((path, started)) = app.native_drop_hover_started.clone() else {
        return false;
    };
    if now.duration_since(started) < DROP_HOVER_OPEN_DELAY
        || app.native_drop_destination.as_ref() != Some(&path)
        || app.drag_hover.is_none()
    {
        return false;
    }
    app.native_drop_hover_started = None;
    if let Some(index) = (0..app.tabs.len()).find(|index| {
        if *index == app.active_tab {
            false
        } else {
            app.inactive_tab(*index)
                .is_some_and(|tab| tab.browser.current() == path)
        }
    }) {
        app.switch_tab(index);
    } else if path != app.browser.current() {
        app.navigate_to(path);
    }
    true
}

impl FileApp {
    pub(crate) fn application_input(
        host: &mut UiHost<FileApp>,
        event: &InputEvent,
    ) -> AdapterOutcome {
        host.application_mut().resolved_grid_columns =
            host.resolved_grid_columns().unwrap_or(1).max(1);
        let mut changed = false;
        let disposition = EventDisposition::Unhandled;
        match event.clone() {
            InputEvent::Key(key) => {
                let app = host.application_mut();
                app.control_down = selection_command_modifier(&key.modifiers);
                app.shift_down = key.modifiers.aggregate(AggregateModifier::Shift);
                let alt_down = key.modifiers.aggregate(AggregateModifier::Alt);
                if key.edge != KeyEdge::Pressed || key.repeat {
                    return AdapterOutcome::default();
                }
                let PhysicalKey::Code(key) = key.physical else {
                    return AdapterOutcome::default();
                };
                if let Some(shortcut) = navigation_shortcut(key, alt_down) {
                    let outcome = perform_navigation_shortcut(app, shortcut);
                    return AdapterOutcome {
                        changed: outcome.changed,
                        disposition: outcome.disposition,
                        ..AdapterOutcome::default()
                    };
                }
                if key == KeyCode::Tab && app.control_down {
                    let outcome = perform_tab_cycle(app, app.shift_down);
                    return AdapterOutcome {
                        changed: outcome.changed,
                        disposition: outcome.disposition,
                        ..AdapterOutcome::default()
                    };
                }
                if key == KeyCode::Backspace {
                    let outcome = perform_navigation_shortcut(app, NavigationShortcut::Back);
                    return AdapterOutcome {
                        changed: outcome.changed,
                        disposition: outcome.disposition,
                        ..AdapterOutcome::default()
                    };
                }
                let outcome = match key {
                    KeyCode::KeyP if app.control_down => {
                        app.update(FileMessage::ToggleCommandSurface);
                        ShortcutOutcome::handled(true)
                    }
                    KeyCode::KeyT if app.control_down => {
                        app.update(FileMessage::NewTab);
                        ShortcutOutcome::handled(true)
                    }
                    KeyCode::KeyW if app.control_down => {
                        app.update(FileMessage::CloseTab(app.active_tab));
                        ShortcutOutcome::handled(true)
                    }
                    KeyCode::KeyL if app.control_down => {
                        if !app.address_editing {
                            app.update(FileMessage::ToggleAddressEditing);
                            ShortcutOutcome::handled(true)
                        } else {
                            ShortcutOutcome::handled(false)
                        }
                    }
                    KeyCode::KeyH if app.control_down => {
                        app.update(FileMessage::ToggleHiddenFiles);
                        ShortcutOutcome::handled(true)
                    }
                    KeyCode::Equal if app.control_down => {
                        app.update(FileMessage::AdjustTileWidth(1));
                        ShortcutOutcome::handled(true)
                    }
                    KeyCode::Minus if app.control_down => {
                        app.update(FileMessage::AdjustTileWidth(-1));
                        ShortcutOutcome::handled(true)
                    }
                    KeyCode::ArrowDown => {
                        app.select_relative(app.resolved_grid_columns() as isize);
                        ShortcutOutcome::handled(true)
                    }
                    KeyCode::ArrowUp => {
                        app.select_relative(-(app.resolved_grid_columns() as isize));
                        ShortcutOutcome::handled(true)
                    }
                    KeyCode::ArrowRight => {
                        app.select_relative(
                            if app.reading_direction == ReadingDirection::RightToLeft {
                                -1
                            } else {
                                1
                            },
                        );
                        ShortcutOutcome::handled(true)
                    }
                    KeyCode::ArrowLeft => {
                        app.select_relative(
                            if app.reading_direction == ReadingDirection::RightToLeft {
                                1
                            } else {
                                -1
                            },
                        );
                        ShortcutOutcome::handled(true)
                    }
                    KeyCode::Escape => {
                        if app.pending_transfer_conflict.is_some() {
                            app.update(FileMessage::TransferCancelConflicts);
                        } else if app.rename_editor.is_some() {
                            app.update(FileMessage::CancelRename);
                        } else if app.transfer_rx.is_some() {
                            app.update(FileMessage::CancelTransfer);
                        } else if app.command_surface_open {
                            app.update(FileMessage::ToggleCommandSurface);
                        } else if app.address_editing {
                            app.update(FileMessage::ToggleAddressEditing);
                        } else {
                            app.clear_selection();
                        }
                        ShortcutOutcome::handled(true)
                    }
                    KeyCode::Enter if app.rename_editor.is_some() => {
                        app.update(FileMessage::CommitRename);
                        ShortcutOutcome::handled(true)
                    }
                    KeyCode::Enter if alt_down => {
                        app.update(FileMessage::ContextProperties);
                        ShortcutOutcome::handled(true)
                    }
                    KeyCode::Enter if app.address_editing => {
                        app.submit_address();
                        ShortcutOutcome::handled(true)
                    }
                    KeyCode::Enter => {
                        app.activate_selected();
                        ShortcutOutcome::handled(true)
                    }
                    KeyCode::Space => {
                        app.toggle_active_selection();
                        ShortcutOutcome::handled(true)
                    }
                    KeyCode::KeyA if app.control_down => {
                        app.select_all();
                        ShortcutOutcome::handled(true)
                    }
                    KeyCode::KeyC if app.control_down => {
                        app.update(FileMessage::CopySelection);
                        ShortcutOutcome::handled(true)
                    }
                    KeyCode::KeyX if app.control_down => {
                        app.update(FileMessage::CutSelection);
                        ShortcutOutcome::handled(true)
                    }
                    KeyCode::KeyV if app.control_down => {
                        app.update(FileMessage::Paste);
                        ShortcutOutcome::handled(true)
                    }
                    KeyCode::F2 => {
                        app.update(FileMessage::BeginRename);
                        ShortcutOutcome::handled(true)
                    }
                    KeyCode::F5 => {
                        app.update(FileMessage::Refresh);
                        ShortcutOutcome::handled(true)
                    }
                    _ => ShortcutOutcome::from_changed(false),
                };
                return AdapterOutcome {
                    changed: outcome.changed,
                    disposition: outcome.disposition,
                    ..AdapterOutcome::default()
                };
            }
            InputEvent::Pointer(PointerEvent::Motion { position, .. }) => {
                let cursor = Point {
                    x: position.x as f32,
                    y: position.y as f32,
                };
                let selection_drag = host.application().selection_drag;
                let resizing = host.application().is_resizing_sidebar();
                let resizing_details = host.application().is_resizing_details_column();
                let selected_entries = selection_drag.map(|start| {
                    let selection = rect_between(start, cursor);
                    entries_in_selection(
                        &host.semantic_nodes(),
                        selection,
                        host.application().browser.entries().len(),
                    )
                });
                // Incoming native drops need semantic destination geometry.
                // Ordinary hover/outbound motion must not clone the whole tree.
                let drop_destination = (host.application().drag_hover.is_some()
                    || host.application().file_drag_started)
                    .then(|| {
                        drop_destination_at(&host.semantic_nodes(), cursor, host.application())
                    })
                    .flatten();
                let app = host.application_mut();
                app.cursor = cursor;
                changed |= update_drop_hover(app, drop_destination, Instant::now());
                if let Some(entries) = selected_entries {
                    app.selected_entries = entries
                        .into_iter()
                        .filter_map(|index| app.browser.identity_at(index))
                        .collect();
                    app.selected = app.selected_entries.iter().copied().min_by_key(|identity| {
                        app.browser
                            .index_of_identity(*identity)
                            .unwrap_or(usize::MAX)
                    });
                }
                if resizing {
                    app.sidebar_width = cursor.x.clamp(MIN_SIDEBAR_WIDTH, MAX_SIDEBAR_WIDTH);
                    app.ensure_selection_visible();
                }
                if resizing_details {
                    app.resize_details_column_to(cursor.x);
                }
                changed |= selection_drag.is_some() || resizing || resizing_details;
            }
            InputEvent::Pointer(PointerEvent::Button {
                button: PointerButton::Secondary,
                edge: KeyEdge::Pressed,
                position: Some(position),
                ..
            }) => {
                host.application_mut().cursor = Point {
                    x: position.x as f32,
                    y: position.y as f32,
                };
                changed = true;
            }
            InputEvent::Pointer(PointerEvent::Button {
                button: PointerButton::Primary,
                edge: KeyEdge::Pressed,
                position,
                ..
            }) => {
                let app = host.application_mut();
                app.primary_down = true;
                if let Some(position) = position {
                    app.cursor = Point {
                        x: position.x as f32,
                        y: position.y as f32,
                    };
                }
            }
            InputEvent::Pointer(PointerEvent::Button {
                button: PointerButton::Primary,
                edge: KeyEdge::Released,
                position,
                ..
            }) => {
                if host.application().file_drag_started
                    && let Some(position) = position
                {
                    let destination = drop_destination_at(
                        &host.semantic_nodes(),
                        Point {
                            x: position.x as f32,
                            y: position.y as f32,
                        },
                        host.application(),
                    );
                    host.application_mut().native_drop_destination = destination;
                }
                let app = host.application_mut();
                app.resizing_sidebar = false;
                app.resizing_details_column = None;
                app.selection_drag = None;
                app.primary_down = false;
                changed = true;
            }
            InputEvent::Pointer(PointerEvent::Axis { delta, .. }) => {
                let y = delta.y as f32;
                let app = host.application_mut();
                if app.control_down {
                    app.update(FileMessage::AdjustTileWidth(y.signum() as i8));
                    changed = true;
                }
            }
            InputEvent::FocusLost { .. } => {
                let app = host.application_mut();
                changed |= cancel_transient_input_on_focus_loss(app);
            }
            InputEvent::DeviceRemoved { .. } => {
                let app = host.application_mut();
                let had_input_state = app.control_down
                    || app.shift_down
                    || app.resizing_sidebar
                    || app.resizing_details_column.is_some()
                    || app.selection_drag.is_some()
                    || app.primary_down;
                app.control_down = false;
                app.shift_down = false;
                app.resizing_sidebar = false;
                app.resizing_details_column = None;
                app.selection_drag = None;
                app.primary_down = false;
                app.file_drag_origin = None;
                app.file_drag_started = false;
                app.outbound_drag = None;
                changed |= had_input_state;
            }
            InputEvent::FocusGained { .. } => {
                host.application_mut().refresh_icons();
                changed = true;
            }
            _ => {}
        }
        AdapterOutcome {
            changed,
            disposition,
            exit: host.application().exit_requested,
        }
    }
}

impl Default for FileHostAdapter {
    fn default() -> Self {
        Self {
            sync_requested: true,
            drop_hover_deadline: None,
            #[cfg(target_os = "windows")]
            context_popup_rx: None,
            #[cfg(target_os = "windows")]
            context_popup_poll_at: None,
            #[cfg(target_os = "windows")]
            focused_shortcut: None,
        }
    }
}

impl HostAdapter<FileApp> for FileHostAdapter {
    #[cfg(target_os = "windows")]
    fn normalized_input(
        &mut self,
        _host: &mut UiHost<FileApp>,
        input: &InputEvent,
        _services: HostServices<'_>,
    ) -> Result<AdapterOutcome, Box<dyn std::error::Error>> {
        if matches!(
            input,
            InputEvent::Pointer(PointerEvent::Button {
                button: PointerButton::Secondary,
                edge: KeyEdge::Pressed,
                ..
            })
        ) {
            // Poll after the UI host dispatches this press, when FileApp has
            // produced its context menu request and target snapshot.
            self.sync_requested = true;
        }
        Ok(AdapterOutcome::default())
    }

    #[cfg(target_os = "windows")]
    fn event(
        &mut self,
        _host: &mut UiHost<FileApp>,
        event: &winit::event::WindowEvent,
        _services: HostServices<'_>,
    ) -> Result<AdapterOutcome, Box<dyn std::error::Error>> {
        if let winit::event::WindowEvent::KeyboardInput { event: key, .. } = event
            && let winit::keyboard::PhysicalKey::Code(physical_key) = key.physical_key
            && let Some(key_code) = match physical_key {
                winit::keyboard::KeyCode::PrintScreen => Some(KeyCode::PrintScreen),
                winit::keyboard::KeyCode::SuperLeft => Some(KeyCode::SuperLeft),
                winit::keyboard::KeyCode::SuperRight => Some(KeyCode::SuperRight),
                _ => None,
            }
            && !key.repeat
            && let Some(shortcut) = &self.focused_shortcut
        {
            shortcut(
                key_code,
                if key.state == winit::event::ElementState::Pressed {
                    KeyEdge::Pressed
                } else {
                    KeyEdge::Released
                },
            );
        }
        Ok(AdapterOutcome::default())
    }

    fn controller_fence(&mut self, _services: HostServices<'_>) -> ControllerFence {
        session_controller_fence()
    }

    fn request_text_entry(&mut self, _services: HostServices<'_>) {
        request_session_text_entry();
    }

    fn next_deadline(&self, now: Instant) -> Option<Instant> {
        self.sync_requested
            .then_some(now)
            .into_iter()
            .chain(self.drop_hover_deadline)
            .chain({
                #[cfg(target_os = "windows")]
                {
                    self.context_popup_poll_at
                }
                #[cfg(not(target_os = "windows"))]
                {
                    None
                }
            })
            .min()
    }

    fn started(
        &mut self,
        _host: &mut UiHost<FileApp>,
        services: HostServices<'_>,
    ) -> Result<AdapterOutcome, Box<dyn std::error::Error>> {
        services
            .window()
            .set_min_inner_size(Some(LogicalSize::new(560, 360)));
        set_nickel_file_icon(services.window());
        Ok(AdapterOutcome::default())
    }

    fn poll(
        &mut self,
        host: &mut UiHost<FileApp>,
        services: HostServices<'_>,
    ) -> Result<AdapterOutcome, Box<dyn std::error::Error>> {
        self.sync_requested = false;
        host.application_mut().resolved_grid_columns =
            host.resolved_grid_columns().unwrap_or(1).max(1);
        let pending_ensure = std::mem::take(&mut host.application_mut().pending_ensure_visible);
        let selected = host.application().selected_index();
        let scroll_offset = host.application().file_scroll_offset;
        let mut changed = false;
        #[cfg(target_os = "windows")]
        {
            if let Some(receiver) = &self.context_popup_rx {
                match receiver.try_recv() {
                    Ok(action) => {
                        if let Some(action) = action {
                            host.application_mut().apply_context_popup_action(action);
                        } else {
                            host.application_mut().close_context_popup();
                        }
                        self.context_popup_rx = None;
                        self.context_popup_poll_at = None;
                        changed = true;
                    }
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        host.application_mut().close_context_popup();
                        self.context_popup_rx = None;
                        self.context_popup_poll_at = None;
                        changed = true;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        self.context_popup_poll_at =
                            Some(Instant::now() + Duration::from_millis(16));
                    }
                }
            }
            if self.context_popup_rx.is_none() {
                let size = services.window().inner_size();
                if let Some(spec) = host
                    .application_mut()
                    .take_context_popup_request(size.width, size.height)
                {
                    self.context_popup_rx =
                        crate::windows_popup_menu::start(spec.menu, services.window());
                    self.context_popup_poll_at = self
                        .context_popup_rx
                        .as_ref()
                        .map(|_| Instant::now() + Duration::from_millis(16));
                    if self.context_popup_rx.is_none() {
                        host.application_mut().close_context_popup();
                    }
                    changed = true;
                }
            }
        }
        changed |= open_drop_hover_target(host.application_mut(), Instant::now());
        self.drop_hover_deadline = host
            .application()
            .native_drop_hover_started
            .as_ref()
            .map(|(_, started)| *started + DROP_HOVER_OPEN_DELAY);
        if pending_ensure && let Some(selected) = selected {
            let columns = host.application().resolved_grid_columns();
            let row_height = 54.0 + (host.application().tile_width * 0.42).clamp(42.0, 96.0);
            let row_top = (selected / columns) as f32 * (row_height + 10.0);
            let target_offset = host
                .semantic_nodes()
                .into_iter()
                .find(|node| node.id.as_str().ends_with("/file-list"))
                .map(|scroll| {
                    if row_top < scroll_offset {
                        row_top
                    } else if row_top + row_height > scroll_offset + scroll.bounds.size.height {
                        row_top + row_height - scroll.bounds.size.height
                    } else {
                        scroll_offset
                    }
                })
                .unwrap_or(scroll_offset)
                .max(0.0);
            if (target_offset - scroll_offset).abs() > f32::EPSILON {
                let app = host.application_mut();
                app.file_scroll_offset = target_offset;
                app.pending_ensure_visible = true;
                changed = true;
            } else {
                host.ensure_message_visible(
                    &FileMessage::Entry(selected),
                    &FileMessage::FileScroll(scroll_offset),
                );
            }
        }
        let title = host.application().localizer.value(
            "ui-file-window-title",
            "path",
            &host.application().browser.current().display().to_string(),
        );
        services.window().set_title(&title);
        Ok(if host.application().exit_requested {
            AdapterOutcome::exit()
        } else {
            AdapterOutcome {
                changed,
                ..AdapterOutcome::default()
            }
        })
    }
}

fn session_controller_fence() -> ControllerFence {
    #[cfg(target_os = "linux")]
    {
        use nickel_session_protocol::{Query, Request, ServerMessage};
        match nickel_session_protocol::client::request_from_environment(
            Request::Query(Query::OnScreenKeyboard),
            std::time::Duration::from_millis(25),
        ) {
            Ok(None) => ControllerFence::default(),
            Ok(Some(ServerMessage::OnScreenKeyboard(snapshot))) => ControllerFence {
                blocked: snapshot.visible,
                barrier_unix_ms: snapshot.controller_barrier_unix_ms,
            },
            _ => ControllerFence {
                blocked: true,
                ..ControllerFence::default()
            },
        }
    }
    #[cfg(not(target_os = "linux"))]
    ControllerFence::default()
}

fn request_session_text_entry() {
    #[cfg(target_os = "linux")]
    {
        use nickel_session_protocol::{Command, Request};
        let _ = nickel_session_protocol::client::request_from_environment(
            Request::Command(Command::RequestOnScreenKeyboard),
            std::time::Duration::from_millis(25),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DROP_HOVER_OPEN_DELAY, NavigationShortcut, adjacent_tab_index,
        cancel_transient_input_on_focus_loss, drop_destination_at, navigation_shortcut,
        open_drop_hover_target, perform_navigation_shortcut, perform_tab_cycle,
        selection_command_modifier, update_drop_hover,
    };
    use crate::{FileApp, FileMessage};
    use nickel_input::{
        DeviceId, EventOrder, InputEvent, KeyCode, KeyEdge, KeyEvent, KeyLocation, LogicalKey,
        Modifier, ModifierState, PhysicalKey,
    };
    use nickel_ui::{Application, EventDisposition};

    fn key_event(key: KeyCode, modifiers: ModifierState) -> InputEvent {
        InputEvent::Key(KeyEvent {
            device: DeviceId(1),
            order: EventOrder(1),
            physical: PhysicalKey::Code(key),
            logical: LogicalKey::Character(String::new()),
            location: KeyLocation::Standard,
            edge: KeyEdge::Pressed,
            repeat: false,
            modifiers,
        })
    }

    #[test]
    fn conventional_alt_navigation_shortcuts_precede_item_direction() {
        assert_eq!(
            navigation_shortcut(KeyCode::ArrowLeft, true),
            Some(NavigationShortcut::Back)
        );
        assert_eq!(
            navigation_shortcut(KeyCode::ArrowRight, true),
            Some(NavigationShortcut::Forward)
        );
        assert_eq!(
            navigation_shortcut(KeyCode::ArrowUp, true),
            Some(NavigationShortcut::Up)
        );
        assert_eq!(navigation_shortcut(KeyCode::ArrowLeft, false), None);
        assert_eq!(navigation_shortcut(KeyCode::ArrowDown, true), None);
    }

    #[test]
    fn conventional_tab_cycle_wraps_in_both_directions() {
        assert_eq!(adjacent_tab_index(0, 3, false), Some(1));
        assert_eq!(adjacent_tab_index(2, 3, false), Some(0));
        assert_eq!(adjacent_tab_index(2, 3, true), Some(1));
        assert_eq!(adjacent_tab_index(0, 3, true), Some(2));
        assert_eq!(adjacent_tab_index(0, 1, false), None);
        assert_eq!(adjacent_tab_index(2, 2, false), None);
    }

    #[test]
    fn unavailable_navigation_and_tab_cycle_are_typed_rejections() {
        let directory = tempfile::tempdir().unwrap();
        let mut app = FileApp::new(directory.path().to_path_buf());

        let back = perform_navigation_shortcut(&mut app, NavigationShortcut::Back);
        assert!(!back.changed);
        assert_eq!(
            back.disposition,
            EventDisposition::Rejected("navigation unavailable")
        );

        let tab = perform_tab_cycle(&mut app, false);
        assert!(!tab.changed);
        assert_eq!(
            tab.disposition,
            EventDisposition::Rejected("tab cycle unavailable")
        );
    }

    #[test]
    fn available_navigation_returns_typed_handled_outcome() {
        let directory = tempfile::tempdir().unwrap();
        let child = directory.path().join("child");
        std::fs::create_dir(&child).unwrap();
        let mut app = FileApp::new(child);

        let up = perform_navigation_shortcut(&mut app, NavigationShortcut::Up);
        assert!(up.changed);
        assert_eq!(up.disposition, EventDisposition::Handled);
    }

    #[test]
    fn command_shortcut_disposition_comes_from_executed_branch() {
        let directory = tempfile::tempdir().unwrap();
        let mut host =
            nickel_ui::UiHost::new(FileApp::new(directory.path().to_path_buf()), 860, 620);
        let control = ModifierState::from_sides([Modifier::ControlLeft]);

        let command =
            FileApp::application_input(&mut host, &key_event(KeyCode::KeyP, control.clone()));
        assert!(command.changed);
        assert_eq!(command.disposition, EventDisposition::Handled);

        let refresh = FileApp::application_input(
            &mut host,
            &key_event(KeyCode::F5, ModifierState::default()),
        );
        assert!(refresh.changed);
        assert_eq!(refresh.disposition, EventDisposition::Handled);

        let unmatched = FileApp::application_input(
            &mut host,
            &key_event(KeyCode::KeyB, ModifierState::default()),
        );
        assert!(!unmatched.changed);
        assert_eq!(unmatched.disposition, EventDisposition::Unhandled);
    }

    #[test]
    fn selection_command_modifier_maps_both_control_sides() {
        for modifier in [Modifier::ControlLeft, Modifier::ControlRight] {
            let state = ModifierState::from_sides([modifier]);
            assert!(selection_command_modifier(&state));
        }
        for modifier in [Modifier::SuperLeft, Modifier::SuperRight] {
            let state = ModifierState::from_sides([modifier]);
            assert!(!selection_command_modifier(&state));
        }
    }

    #[test]
    fn focus_loss_cancels_rename_without_committing_it() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("report.txt");
        std::fs::write(&path, b"report").unwrap();
        let mut app = FileApp::new(directory.path().to_path_buf());
        let identity = app.browser.identity_at(0).unwrap();
        app.selected = Some(identity);
        app.selected_entries.insert(identity);
        app.update(FileMessage::BeginRename);
        app.update(FileMessage::RenameChanged("renamed.txt".into()));

        assert!(cancel_transient_input_on_focus_loss(&mut app));
        assert!(app.rename_editor.is_none());
        assert!(path.exists());
        assert!(!directory.path().join("renamed.txt").exists());
        assert!(!cancel_transient_input_on_focus_loss(&mut app));
    }

    #[test]
    fn native_drop_target_uses_semantic_folder_geometry() {
        let directory = tempfile::tempdir().unwrap();
        let folder = directory.path().join("destination");
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(directory.path().join("file.txt"), b"file").unwrap();
        let host = nickel_ui::UiHost::new(FileApp::new(directory.path().to_path_buf()), 860, 620);
        let node = host
            .semantic_nodes()
            .into_iter()
            .find(|node| node.name.as_deref() == Some("destination"))
            .unwrap();
        let point = nickel_ui::Point {
            x: node.bounds.origin.x + node.bounds.size.width / 2.0,
            y: node.bounds.origin.y + node.bounds.size.height / 2.0,
        };

        assert_eq!(
            drop_destination_at(&host.semantic_nodes(), point, host.application()),
            Some(folder)
        );
    }

    #[test]
    fn native_drop_hover_is_delayed_cancellable_and_opens_once() {
        let directory = tempfile::tempdir().unwrap();
        let folder = directory.path().join("destination");
        std::fs::create_dir(&folder).unwrap();
        let mut app = FileApp::new(directory.path().to_path_buf());
        app.drag_hover = Some(directory.path().join("source.txt"));
        let started = std::time::Instant::now();
        assert!(update_drop_hover(&mut app, Some(folder.clone()), started));
        assert!(!open_drop_hover_target(
            &mut app,
            started + DROP_HOVER_OPEN_DELAY - std::time::Duration::from_millis(1)
        ));
        assert!(open_drop_hover_target(
            &mut app,
            started + DROP_HOVER_OPEN_DELAY
        ));
        assert!(app.navigation_pending());
        assert!(app.native_drop_hover_started.is_none());

        app.native_drop_hover_started = Some((folder.clone(), started));
        app.drag_hover = None;
        assert!(!open_drop_hover_target(
            &mut app,
            started + DROP_HOVER_OPEN_DELAY
        ));
    }
}
