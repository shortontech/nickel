use std::{collections::HashMap, error::Error, io};

use wayland_client::{
    Connection, Dispatch, QueueHandle,
    globals::{GlobalListContents, registry_queue_init},
    protocol::wl_registry,
};
use wayland_protocols_plasma::plasma_window_management::client::{
    org_kde_plasma_window,
    org_kde_plasma_window_management::{self, OrgKdePlasmaWindowManagement},
};

const ACTIVE_STATE: u32 = 1;

#[derive(Debug, Default)]
pub struct WindowInfo {
    pub title: String,
    pub app_id: String,
    pub active: bool,
}

#[derive(Default)]
struct State {
    windows: HashMap<String, WindowInfo>,
}

pub fn window_list() -> Result<Vec<WindowInfo>, Box<dyn Error>> {
    let connection = Connection::connect_to_env()?;
    let (globals, mut event_queue) = registry_queue_init::<State>(&connection)?;
    let queue_handle = event_queue.handle();
    let _manager = globals
        .bind::<OrgKdePlasmaWindowManagement, _, _>(&queue_handle, 12..=18, ())
        .map_err(|error| {
            io::Error::other(format!(
                "KWin did not expose org_kde_plasma_window_management: {error}"
            ))
        })?;
    let mut state = State::default();

    // The first roundtrip receives the initial window handles; the second receives
    // their properties.
    event_queue.roundtrip(&mut state)?;
    event_queue.roundtrip(&mut state)?;

    let mut windows: Vec<_> = state.windows.into_values().collect();
    windows.sort_by(|left, right| left.title.cmp(&right.title));
    Ok(windows)
}

impl Dispatch<OrgKdePlasmaWindowManagement, ()> for State {
    fn event(
        state: &mut Self,
        manager: &OrgKdePlasmaWindowManagement,
        event: org_kde_plasma_window_management::Event,
        _: &(),
        _: &Connection,
        queue_handle: &QueueHandle<Self>,
    ) {
        if let org_kde_plasma_window_management::Event::WindowWithUuid { uuid, .. } = event {
            state.windows.entry(uuid.clone()).or_default();
            manager.get_window_by_uuid(uuid.clone(), queue_handle, uuid);
        }
    }
}

impl Dispatch<org_kde_plasma_window::OrgKdePlasmaWindow, String> for State {
    fn event(
        state: &mut Self,
        _: &org_kde_plasma_window::OrgKdePlasmaWindow,
        event: org_kde_plasma_window::Event,
        uuid: &String,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(window) = state.windows.get_mut(uuid) else {
            return;
        };
        match event {
            org_kde_plasma_window::Event::TitleChanged { title } => window.title = title,
            org_kde_plasma_window::Event::AppIdChanged { app_id } => window.app_id = app_id,
            org_kde_plasma_window::Event::StateChanged { flags } => {
                window.active = flags & ACTIVE_STATE != 0;
            }
            org_kde_plasma_window::Event::Unmapped => {
                state.windows.remove(uuid);
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
