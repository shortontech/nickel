use std::time::Instant;

use nickel_input::{
    AggregateModifier, InputEvent, KeyCode, KeyEdge, PhysicalKey, PointerButton, PointerEvent,
};
use nickel_ui::{
    AdapterOutcome, Application, HostAdapter, HostServices, Point, ReadingDirection, UiHost,
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

pub(crate) struct FileHostAdapter {
    sync_requested: bool,
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

fn selection_command_modifier(modifiers: &nickel_input::ModifierState, macos: bool) -> bool {
    modifiers.aggregate(if macos {
        AggregateModifier::Super
    } else {
        AggregateModifier::Control
    })
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

impl Default for FileHostAdapter {
    fn default() -> Self {
        Self {
            sync_requested: true,
        }
    }
}

impl HostAdapter<FileApp> for FileHostAdapter {
    fn next_deadline(&self, now: Instant) -> Option<Instant> {
        self.sync_requested.then_some(now)
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

    fn normalized_input(
        &mut self,
        host: &mut UiHost<FileApp>,
        event: &InputEvent,
        _services: HostServices<'_>,
    ) -> Result<AdapterOutcome, Box<dyn std::error::Error>> {
        let mut changed = false;
        let mut consume = false;
        match event.clone() {
            InputEvent::Key(key) => {
                let app = host.application_mut();
                app.control_down =
                    selection_command_modifier(&key.modifiers, cfg!(target_os = "macos"));
                app.shift_down = key.modifiers.aggregate(AggregateModifier::Shift);
                let alt_down = key.modifiers.aggregate(AggregateModifier::Alt);
                if key.edge != KeyEdge::Pressed || key.repeat {
                    return Ok(AdapterOutcome::default());
                }
                let PhysicalKey::Code(key) = key.physical else {
                    return Ok(AdapterOutcome::default());
                };
                if let Some(shortcut) = navigation_shortcut(key, alt_down) {
                    match shortcut {
                        NavigationShortcut::Back => app.go_back(),
                        NavigationShortcut::Forward => app.go_forward(),
                        NavigationShortcut::Up => app.go_up(),
                    }
                    return Ok(AdapterOutcome {
                        changed: true,
                        ..AdapterOutcome::default()
                    });
                }
                if key == KeyCode::Tab
                    && app.control_down
                    && let Some(index) =
                        adjacent_tab_index(app.active_tab, app.tabs.len(), app.shift_down)
                {
                    app.switch_tab(index);
                    return Ok(AdapterOutcome {
                        changed: true,
                        ..AdapterOutcome::default()
                    });
                }
                match key {
                    KeyCode::KeyP if app.control_down => {
                        app.update(FileMessage::ToggleCommandSurface);
                    }
                    KeyCode::KeyT if app.control_down => app.update(FileMessage::NewTab),
                    KeyCode::KeyW if app.control_down => {
                        app.update(FileMessage::CloseTab(app.active_tab));
                    }
                    KeyCode::KeyL if app.control_down => {
                        if !app.address_editing {
                            app.update(FileMessage::ToggleAddressEditing);
                        }
                    }
                    KeyCode::KeyH if app.control_down => {
                        app.update(FileMessage::ToggleHiddenFiles);
                    }
                    KeyCode::Equal if app.control_down => {
                        app.update(FileMessage::AdjustTileWidth(1));
                    }
                    KeyCode::Minus if app.control_down => {
                        app.update(FileMessage::AdjustTileWidth(-1));
                    }
                    KeyCode::ArrowDown => app.select_relative(app.resolved_grid_columns() as isize),
                    KeyCode::ArrowUp => {
                        app.select_relative(-(app.resolved_grid_columns() as isize))
                    }
                    KeyCode::ArrowRight => app.select_relative(
                        if app.reading_direction == ReadingDirection::RightToLeft {
                            -1
                        } else {
                            1
                        },
                    ),
                    KeyCode::ArrowLeft => app.select_relative(
                        if app.reading_direction == ReadingDirection::RightToLeft {
                            1
                        } else {
                            -1
                        },
                    ),
                    KeyCode::Backspace => app.go_back(),
                    KeyCode::Escape => {
                        if app.command_surface_open {
                            app.update(FileMessage::ToggleCommandSurface);
                        } else if app.address_editing {
                            app.update(FileMessage::ToggleAddressEditing);
                        } else {
                            app.clear_selection();
                        }
                    }
                    KeyCode::Enter if app.address_editing => app.submit_address(),
                    KeyCode::Enter => app.activate_selected(),
                    KeyCode::Space => app.toggle_active_selection(),
                    KeyCode::KeyA if app.control_down => {
                        app.select_all();
                    }
                    KeyCode::F5 => {
                        app.update(FileMessage::Refresh);
                    }
                    _ => {}
                }
                consume = matches!(
                    key,
                    KeyCode::ArrowDown
                        | KeyCode::ArrowUp
                        | KeyCode::ArrowRight
                        | KeyCode::ArrowLeft
                        | KeyCode::Escape
                        | KeyCode::Enter
                        | KeyCode::Space
                ) || (app.control_down && key == KeyCode::KeyA);
                changed = true;
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
                let app = host.application_mut();
                app.cursor = cursor;
                if let Some(entries) = selected_entries {
                    app.selected_entries = entries;
                    app.selected = app.selected_entries.iter().copied().min();
                }
                if resizing {
                    app.sidebar_width = cursor.x.clamp(MIN_SIDEBAR_WIDTH, MAX_SIDEBAR_WIDTH);
                    app.ensure_selection_visible();
                }
                if resizing_details {
                    app.resize_details_column_to(cursor.x);
                }
                changed = selection_drag.is_some() || resizing || resizing_details;
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
                edge: KeyEdge::Released,
                ..
            }) => {
                let app = host.application_mut();
                app.resizing_sidebar = false;
                app.resizing_details_column = None;
                app.selection_drag = None;
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
            InputEvent::FocusLost { .. } | InputEvent::DeviceRemoved { .. } => {
                let app = host.application_mut();
                app.control_down = false;
                app.shift_down = false;
                app.resizing_sidebar = false;
                app.resizing_details_column = None;
                app.selection_drag = None;
            }
            InputEvent::FocusGained { .. } => {
                host.application_mut().refresh_icons();
                changed = true;
            }
            _ => {}
        }
        self.sync_requested |= changed;
        Ok(AdapterOutcome {
            changed,
            consume,
            exit: host.application().exit_requested,
        })
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
        let selected = host.application().selected;
        let scroll_offset = host.application().file_scroll_offset;
        let mut changed = false;
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
        let title = format!(
            "Nickel File — {}",
            host.application().browser.current().display()
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

#[cfg(test)]
mod tests {
    use super::{
        NavigationShortcut, adjacent_tab_index, navigation_shortcut, selection_command_modifier,
    };
    use nickel_input::{KeyCode, Modifier, ModifierState};

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
    fn selection_command_modifier_maps_both_physical_sides_and_platforms() {
        for modifier in [Modifier::ControlLeft, Modifier::ControlRight] {
            let state = ModifierState::from_sides([modifier]);
            assert!(selection_command_modifier(&state, false));
            assert!(!selection_command_modifier(&state, true));
        }
        for modifier in [Modifier::SuperLeft, Modifier::SuperRight] {
            let state = ModifierState::from_sides([modifier]);
            assert!(selection_command_modifier(&state, true));
            assert!(!selection_command_modifier(&state, false));
        }
    }
}
