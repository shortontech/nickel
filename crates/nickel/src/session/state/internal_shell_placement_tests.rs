use super::{
    avoid_trusted_control_collision, internal_codex_chat_placement,
    internal_codex_project_menu_placement, internal_shell_surface_placement,
};
use crate::{internal_shell::InternalOutput, winit_shell::SurfaceRole};
use nickel_session_protocol::{AnchorSide, Geometry, ShellPopoverAnchor};

fn outputs() -> Vec<(InternalOutput, i32, i32)> {
    vec![
        (
            InternalOutput {
                x: -1920,
                y: -120,
                name: "left".into(),
                width: 1920,
                height: 1080,
                scale: 1.0,
            },
            -1920,
            -120,
        ),
        (
            InternalOutput {
                x: 0,
                y: 240,
                name: "right".into(),
                width: 2560,
                height: 1440,
                scale: 1.0,
            },
            0,
            240,
        ),
    ]
}

pub(super) fn receive_x11_clipboard(
    display: &str,
    property_name: &str,
    acknowledgement_delay: std::time::Duration,
    target_name: &str,
    input_only: bool,
) -> Result<Vec<u8>, String> {
    receive_x11_selection(
        display,
        "CLIPBOARD",
        property_name,
        acknowledgement_delay,
        target_name,
        input_only,
    )
}

pub(super) fn receive_x11_selection(
    display: &str,
    selection_name: &str,
    property_name: &str,
    acknowledgement_delay: std::time::Duration,
    target_name: &str,
    input_only: bool,
) -> Result<Vec<u8>, String> {
    use smithay::reexports::x11rb::{
        connection::Connection,
        protocol::{
            Event,
            xproto::{AtomEnum, ConnectionExt, CreateWindowAux, EventMask, Property, WindowClass},
        },
    };
    use std::time::{Duration, Instant};

    let (connection, screen) =
        smithay::reexports::x11rb::connect(Some(display)).map_err(|e| e.to_string())?;
    let root = &connection.setup().roots[screen];
    let requestor = connection.generate_id().map_err(|e| e.to_string())?;
    let (depth, class, visual) = if input_only {
        (0, WindowClass::INPUT_ONLY, 0)
    } else {
        (root.root_depth, WindowClass::INPUT_OUTPUT, root.root_visual)
    };
    connection
        .create_window(
            depth,
            requestor,
            root.root,
            0,
            0,
            1,
            1,
            0,
            class,
            visual,
            &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE),
        )
        .map_err(|e| e.to_string())?;
    let atom = |name: &[u8]| {
        connection
            .intern_atom(false, name)
            .map_err(|e| e.to_string())?
            .reply()
            .map(|reply| reply.atom)
            .map_err(|e| e.to_string())
    };
    let selection = atom(selection_name.as_bytes())?;
    let target = atom(target_name.as_bytes())?;
    let property = atom(property_name.as_bytes())?;
    let incr = atom(b"INCR")?;
    connection
        .convert_selection(
            requestor,
            selection,
            target,
            property,
            smithay::reexports::x11rb::CURRENT_TIME,
        )
        .map_err(|e| e.to_string())?;
    connection.flush().map_err(|e| e.to_string())?;

    let mut bytes = Vec::new();
    let transfer_deadline = Instant::now() + Duration::from_secs(15);
    let mut incremental = false;
    loop {
        if Instant::now() >= transfer_deadline {
            return Err("X11 selection transfer timed out".into());
        }
        let Some(event) = connection.poll_for_event().map_err(|e| e.to_string())? else {
            std::thread::sleep(Duration::from_millis(1));
            continue;
        };
        match event {
            Event::SelectionNotify(event) if event.requestor == requestor => {
                if event.property == smithay::reexports::x11rb::NONE {
                    return Err(format!("selection owner rejected {target_name}"));
                }
                let reply = connection
                    .get_property(false, requestor, property, AtomEnum::ANY, 0, u32::MAX)
                    .map_err(|e| e.to_string())?
                    .reply()
                    .map_err(|e| e.to_string())?;
                if reply.type_ == incr {
                    incremental = true;
                    if !acknowledgement_delay.is_zero() {
                        std::thread::sleep(acknowledgement_delay);
                    }
                    connection
                        .delete_property(requestor, property)
                        .map_err(|e| e.to_string())?;
                    connection.flush().map_err(|e| e.to_string())?;
                } else {
                    return Ok(reply.value);
                }
            }
            Event::PropertyNotify(event)
                if incremental
                    && event.window == requestor
                    && event.atom == property
                    && event.state == Property::NEW_VALUE =>
            {
                let reply = connection
                    .get_property(true, requestor, property, AtomEnum::ANY, 0, u32::MAX)
                    .map_err(|e| e.to_string())?
                    .reply()
                    .map_err(|e| e.to_string())?;
                if reply.value.is_empty() {
                    return Ok(bytes);
                }
                bytes.extend_from_slice(&reply.value);
                connection.flush().map_err(|e| e.to_string())?;
            }
            _ => {}
        }
    }
}

pub(super) fn receive_concurrent_x11_text_mimes(display: &str) -> Result<[Vec<u8>; 2], String> {
    use smithay::reexports::x11rb::{
        connection::Connection,
        protocol::{
            Event,
            xproto::{
                Atom, AtomEnum, ConnectionExt, CreateWindowAux, EventMask, Property, WindowClass,
            },
        },
    };
    use std::time::{Duration, Instant};

    #[derive(Default)]
    struct Transfer {
        property: Atom,
        incremental: bool,
        complete: bool,
        bytes: Vec<u8>,
    }

    let (connection, screen) =
        smithay::reexports::x11rb::connect(Some(display)).map_err(|e| e.to_string())?;
    let root = &connection.setup().roots[screen];
    let requestor = connection.generate_id().map_err(|e| e.to_string())?;
    connection
        .create_window(
            0,
            requestor,
            root.root,
            0,
            0,
            1,
            1,
            0,
            WindowClass::INPUT_ONLY,
            0,
            &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE),
        )
        .map_err(|e| e.to_string())?;
    let atom = |name: &[u8]| {
        connection
            .intern_atom(false, name)
            .map_err(|e| e.to_string())?
            .reply()
            .map(|reply| reply.atom)
            .map_err(|e| e.to_string())
    };
    let clipboard = atom(b"CLIPBOARD")?;
    let incr = atom(b"INCR")?;
    let targets = [atom(b"text/plain;charset=utf-8")?, atom(b"text/plain")?];
    let mut transfers = [
        Transfer {
            property: atom(b"NICKEL_CONCURRENT_UTF8")?,
            ..Default::default()
        },
        Transfer {
            property: atom(b"NICKEL_CONCURRENT_PLAIN")?,
            ..Default::default()
        },
    ];
    for (target, transfer) in targets.into_iter().zip(&transfers) {
        connection
            .convert_selection(
                requestor,
                clipboard,
                target,
                transfer.property,
                smithay::reexports::x11rb::CURRENT_TIME,
            )
            .map_err(|e| e.to_string())?;
    }
    connection.flush().map_err(|e| e.to_string())?;

    let deadline = Instant::now() + Duration::from_secs(15);
    while transfers.iter().any(|transfer| !transfer.complete) {
        if Instant::now() >= deadline {
            return Err("concurrent X11 MIME requests timed out".into());
        }
        let Some(event) = connection.poll_for_event().map_err(|e| e.to_string())? else {
            std::thread::sleep(Duration::from_millis(1));
            continue;
        };
        match event {
            Event::SelectionNotify(event) if event.requestor == requestor => {
                if event.property == smithay::reexports::x11rb::NONE {
                    return Err("concurrent X11 MIME request was rejected".into());
                }
                let Some(transfer) = transfers
                    .iter_mut()
                    .find(|transfer| transfer.property == event.property)
                else {
                    continue;
                };
                let reply = connection
                    .get_property(
                        false,
                        requestor,
                        transfer.property,
                        AtomEnum::ANY,
                        0,
                        u32::MAX,
                    )
                    .map_err(|e| e.to_string())?
                    .reply()
                    .map_err(|e| e.to_string())?;
                if reply.type_ == incr {
                    transfer.incremental = true;
                    connection
                        .delete_property(requestor, transfer.property)
                        .map_err(|e| e.to_string())?;
                } else {
                    transfer.bytes = reply.value;
                    transfer.complete = true;
                }
                connection.flush().map_err(|e| e.to_string())?;
            }
            Event::PropertyNotify(event)
                if event.window == requestor && event.state == Property::NEW_VALUE =>
            {
                let Some(transfer) = transfers.iter_mut().find(|transfer| {
                    transfer.incremental && !transfer.complete && transfer.property == event.atom
                }) else {
                    continue;
                };
                let reply = connection
                    .get_property(
                        true,
                        requestor,
                        transfer.property,
                        AtomEnum::ANY,
                        0,
                        u32::MAX,
                    )
                    .map_err(|e| e.to_string())?
                    .reply()
                    .map_err(|e| e.to_string())?;
                if reply.value.is_empty() {
                    transfer.complete = true;
                } else {
                    transfer.bytes.extend_from_slice(&reply.value);
                }
                connection.flush().map_err(|e| e.to_string())?;
            }
            _ => {}
        }
    }
    Ok(transfers.map(|transfer| transfer.bytes))
}

pub(super) fn receive_x11_after_requestor_id_reuse(
    display: &str,
    old_ready: std::sync::mpsc::Sender<u32>,
    old_disconnected: std::sync::mpsc::Sender<()>,
    reconnect: std::sync::mpsc::Receiver<()>,
) -> Result<(u32, Vec<u8>), String> {
    use smithay::reexports::x11rb::{
        connection::Connection,
        protocol::{
            Event,
            xproto::{AtomEnum, ConnectionExt, CreateWindowAux, EventMask, Property, WindowClass},
        },
    };
    use std::time::{Duration, Instant};

    let old_requestor = {
        let (connection, screen) =
            smithay::reexports::x11rb::connect(Some(display)).map_err(|e| e.to_string())?;
        let root = &connection.setup().roots[screen];
        let requestor = connection.generate_id().map_err(|e| e.to_string())?;
        connection
            .create_window(
                0,
                requestor,
                root.root,
                0,
                0,
                1,
                1,
                0,
                WindowClass::INPUT_ONLY,
                0,
                &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE),
            )
            .map_err(|e| e.to_string())?;
        let atom = |name: &[u8]| {
            connection
                .intern_atom(false, name)
                .map_err(|e| e.to_string())?
                .reply()
                .map(|reply| reply.atom)
                .map_err(|e| e.to_string())
        };
        let clipboard = atom(b"CLIPBOARD")?;
        let target = atom(b"image/png")?;
        let property = atom(b"NICKEL_REUSED_REQUESTOR")?;
        let incr = atom(b"INCR")?;
        connection
            .convert_selection(
                requestor,
                clipboard,
                target,
                property,
                smithay::reexports::x11rb::CURRENT_TIME,
            )
            .map_err(|e| e.to_string())?;
        connection.flush().map_err(|e| e.to_string())?;
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if Instant::now() >= deadline {
                return Err("old X11 requestor did not enter INCR".into());
            }
            match connection.poll_for_event().map_err(|e| e.to_string())? {
                Some(Event::SelectionNotify(event)) if event.requestor == requestor => {
                    let reply = connection
                        .get_property(false, requestor, property, AtomEnum::ANY, 0, u32::MAX)
                        .map_err(|e| e.to_string())?
                        .reply()
                        .map_err(|e| e.to_string())?;
                    if reply.type_ != incr {
                        return Err("old X11 requestor did not receive an INCR header".into());
                    }
                    old_ready.send(requestor).map_err(|e| e.to_string())?;
                    break requestor;
                }
                Some(_) => {}
                None => std::thread::sleep(Duration::from_millis(1)),
            }
        }
    };
    old_disconnected.send(()).map_err(|e| e.to_string())?;
    reconnect.recv().map_err(|e| e.to_string())?;

    let (connection, screen) =
        smithay::reexports::x11rb::connect(Some(display)).map_err(|e| e.to_string())?;
    let root = &connection.setup().roots[screen];
    let requestor = connection.generate_id().map_err(|e| e.to_string())?;
    if requestor != old_requestor {
        return Err(format!(
            "X server did not reuse requestor ID: old={old_requestor} new={requestor}"
        ));
    }
    connection
        .create_window(
            0,
            requestor,
            root.root,
            0,
            0,
            1,
            1,
            0,
            WindowClass::INPUT_ONLY,
            0,
            &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE),
        )
        .map_err(|e| e.to_string())?;
    let atom = |name: &[u8]| {
        connection
            .intern_atom(false, name)
            .map_err(|e| e.to_string())?
            .reply()
            .map(|reply| reply.atom)
            .map_err(|e| e.to_string())
    };
    let clipboard = atom(b"CLIPBOARD")?;
    let target = atom(b"image/png")?;
    let property = atom(b"NICKEL_REUSED_REQUESTOR")?;
    let incr = atom(b"INCR")?;
    connection
        .convert_selection(
            requestor,
            clipboard,
            target,
            property,
            smithay::reexports::x11rb::CURRENT_TIME,
        )
        .map_err(|e| e.to_string())?;
    connection.flush().map_err(|e| e.to_string())?;

    let deadline = Instant::now() + Duration::from_secs(15);
    let mut incremental = false;
    let mut bytes = Vec::new();
    loop {
        if Instant::now() >= deadline {
            return Err("reused X11 requestor transfer timed out".into());
        }
        match connection.poll_for_event().map_err(|e| e.to_string())? {
            Some(Event::SelectionNotify(event)) if event.requestor == requestor => {
                if event.property == smithay::reexports::x11rb::NONE {
                    return Err("reused X11 requestor was rejected".into());
                }
                let reply = connection
                    .get_property(false, requestor, property, AtomEnum::ANY, 0, u32::MAX)
                    .map_err(|e| e.to_string())?
                    .reply()
                    .map_err(|e| e.to_string())?;
                if reply.type_ == incr {
                    incremental = true;
                    connection
                        .delete_property(requestor, property)
                        .map_err(|e| e.to_string())?;
                    connection.flush().map_err(|e| e.to_string())?;
                } else {
                    return Ok((requestor, reply.value));
                }
            }
            Some(Event::PropertyNotify(event))
                if incremental
                    && event.window == requestor
                    && event.atom == property
                    && event.state == Property::NEW_VALUE =>
            {
                let reply = connection
                    .get_property(true, requestor, property, AtomEnum::ANY, 0, u32::MAX)
                    .map_err(|e| e.to_string())?
                    .reply()
                    .map_err(|e| e.to_string())?;
                if reply.value.is_empty() {
                    return Ok((requestor, bytes));
                }
                bytes.extend_from_slice(&reply.value);
                connection.flush().map_err(|e| e.to_string())?;
            }
            Some(_) => {}
            None => std::thread::sleep(Duration::from_millis(1)),
        }
    }
}

pub(super) fn receive_x11_dnd_selection(
    display: &str,
    ready: std::sync::mpsc::Sender<u32>,
    request: std::sync::mpsc::Receiver<()>,
) -> Result<Vec<u8>, String> {
    use smithay::reexports::x11rb::{
        connection::Connection,
        protocol::{
            Event,
            xproto::{AtomEnum, ConnectionExt, CreateWindowAux, EventMask, Property, WindowClass},
        },
        wrapper::ConnectionExt as _,
    };
    use std::time::{Duration, Instant};

    let (connection, screen) =
        smithay::reexports::x11rb::connect(Some(display)).map_err(|e| e.to_string())?;
    let root = &connection.setup().roots[screen];
    let requestor = connection.generate_id().map_err(|e| e.to_string())?;
    connection
        .create_window(
            root.root_depth,
            requestor,
            root.root,
            0,
            0,
            16,
            16,
            0,
            WindowClass::INPUT_OUTPUT,
            root.root_visual,
            &CreateWindowAux::new()
                .event_mask(EventMask::PROPERTY_CHANGE | EventMask::STRUCTURE_NOTIFY),
        )
        .map_err(|e| e.to_string())?;
    let atom = |name: &[u8]| {
        connection
            .intern_atom(false, name)
            .map_err(|e| e.to_string())?
            .reply()
            .map(|reply| reply.atom)
            .map_err(|e| e.to_string())
    };
    let xdnd_aware = atom(b"XdndAware")?;
    connection
        .change_property32(
            smithay::reexports::x11rb::protocol::xproto::PropMode::REPLACE,
            requestor,
            xdnd_aware,
            AtomEnum::ATOM,
            &[5],
        )
        .map_err(|e| e.to_string())?;
    connection
        .map_window(requestor)
        .map_err(|e| e.to_string())?;
    connection.flush().map_err(|e| e.to_string())?;
    ready.send(requestor).map_err(|e| e.to_string())?;
    request.recv().map_err(|e| e.to_string())?;

    let selection = atom(b"XdndSelection")?;
    let target = atom(b"image/png")?;
    let property = atom(b"NICKEL_XDND_SELECTION")?;
    let incr = atom(b"INCR")?;
    connection
        .convert_selection(
            requestor,
            selection,
            target,
            property,
            smithay::reexports::x11rb::CURRENT_TIME,
        )
        .map_err(|e| e.to_string())?;
    connection.flush().map_err(|e| e.to_string())?;

    let deadline = Instant::now() + Duration::from_secs(15);
    let mut incremental = false;
    let mut bytes = Vec::new();
    loop {
        if Instant::now() >= deadline {
            return Err("X11 DnD selection transfer timed out".into());
        }
        match connection.poll_for_event().map_err(|e| e.to_string())? {
            Some(Event::SelectionNotify(event)) if event.requestor == requestor => {
                if event.property == smithay::reexports::x11rb::NONE {
                    return Err("X11 DnD selection request was rejected".into());
                }
                let reply = connection
                    .get_property(false, requestor, property, AtomEnum::ANY, 0, u32::MAX)
                    .map_err(|e| e.to_string())?
                    .reply()
                    .map_err(|e| e.to_string())?;
                if reply.type_ == incr {
                    incremental = true;
                    connection
                        .delete_property(requestor, property)
                        .map_err(|e| e.to_string())?;
                    connection.flush().map_err(|e| e.to_string())?;
                } else {
                    return Ok(reply.value);
                }
            }
            Some(Event::PropertyNotify(event))
                if incremental
                    && event.window == requestor
                    && event.atom == property
                    && event.state == Property::NEW_VALUE =>
            {
                let reply = connection
                    .get_property(true, requestor, property, AtomEnum::ANY, 0, u32::MAX)
                    .map_err(|e| e.to_string())?
                    .reply()
                    .map_err(|e| e.to_string())?;
                if reply.value.is_empty() {
                    return Ok(bytes);
                }
                bytes.extend_from_slice(&reply.value);
                connection.flush().map_err(|e| e.to_string())?;
            }
            Some(_) => {}
            None => std::thread::sleep(Duration::from_millis(1)),
        }
    }
}

pub(super) fn receive_replaced_x11_clipboard(
    display: &str,
    property_name: &str,
    replace_ready: std::sync::mpsc::Sender<()>,
    replace_go: std::sync::mpsc::Receiver<()>,
) -> Result<Vec<u8>, String> {
    use smithay::reexports::x11rb::{
        connection::Connection,
        protocol::{
            Event,
            xproto::{AtomEnum, ConnectionExt, CreateWindowAux, EventMask, Property, WindowClass},
        },
        wrapper::ConnectionExt as _,
    };
    use std::time::{Duration, Instant};

    let (connection, screen) =
        smithay::reexports::x11rb::connect(Some(display)).map_err(|e| e.to_string())?;
    let root = &connection.setup().roots[screen];
    let requestor = connection.generate_id().map_err(|e| e.to_string())?;
    connection
        .create_window(
            0,
            requestor,
            root.root,
            0,
            0,
            1,
            1,
            0,
            WindowClass::INPUT_ONLY,
            0,
            &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE),
        )
        .map_err(|e| e.to_string())?;
    let atom = |name: &[u8]| {
        connection
            .intern_atom(false, name)
            .map_err(|e| e.to_string())?
            .reply()
            .map(|reply| reply.atom)
            .map_err(|e| e.to_string())
    };
    let clipboard = atom(b"CLIPBOARD")?;
    let target = atom(b"image/png")?;
    let property = atom(property_name.as_bytes())?;
    let unrelated = atom(b"NICKEL_UNRELATED_PROPERTY")?;
    let incr = atom(b"INCR")?;
    let request = || -> Result<(), String> {
        connection
            .convert_selection(
                requestor,
                clipboard,
                target,
                property,
                smithay::reexports::x11rb::CURRENT_TIME,
            )
            .map_err(|e| e.to_string())?;
        connection.flush().map_err(|e| e.to_string())
    };
    request()?;

    let deadline = Instant::now() + Duration::from_secs(15);
    let mut notification = 0;
    let mut incremental = false;
    let mut bytes = Vec::new();
    loop {
        if Instant::now() >= deadline {
            return Err("replacement X11 selection transfer timed out".into());
        }
        let Some(event) = connection.poll_for_event().map_err(|e| e.to_string())? else {
            std::thread::sleep(Duration::from_millis(1));
            continue;
        };
        match event {
            Event::SelectionNotify(event) if event.requestor == requestor => {
                if event.property == smithay::reexports::x11rb::NONE {
                    return Err("replacement selection request was rejected".into());
                }
                notification += 1;
                let reply = connection
                    .get_property(false, requestor, property, AtomEnum::ANY, 0, u32::MAX)
                    .map_err(|e| e.to_string())?
                    .reply()
                    .map_err(|e| e.to_string())?;
                if reply.type_ != incr {
                    return Err("replacement fixture did not enter INCR".into());
                }
                if notification == 1 {
                    // Queue unrelated traffic and replace the exact request/property
                    // before acknowledging its INCR header.
                    connection
                        .change_property8(
                            smithay::reexports::x11rb::protocol::xproto::PropMode::REPLACE,
                            requestor,
                            unrelated,
                            AtomEnum::STRING,
                            b"unrelated",
                        )
                        .map_err(|e| e.to_string())?;
                    replace_ready.send(()).map_err(|e| e.to_string())?;
                    replace_go
                        .recv_timeout(Duration::from_secs(5))
                        .map_err(|e| e.to_string())?;
                    request()?;
                } else {
                    incremental = true;
                    connection
                        .delete_property(requestor, property)
                        .map_err(|e| e.to_string())?;
                    connection.flush().map_err(|e| e.to_string())?;
                }
            }
            Event::PropertyNotify(event)
                if incremental
                    && event.window == requestor
                    && event.atom == property
                    && event.state == Property::NEW_VALUE =>
            {
                let reply = connection
                    .get_property(true, requestor, property, AtomEnum::ANY, 0, u32::MAX)
                    .map_err(|e| e.to_string())?
                    .reply()
                    .map_err(|e| e.to_string())?;
                if reply.value.is_empty() {
                    return Ok(bytes);
                }
                bytes.extend_from_slice(&reply.value);
                connection.flush().map_err(|e| e.to_string())?;
            }
            _ => {}
        }
    }
}

pub(super) fn wait_for_x11_incremental_abort(
    display: &str,
    ready: std::sync::mpsc::Sender<()>,
) -> Result<(), String> {
    use smithay::reexports::x11rb::{
        connection::Connection,
        protocol::{
            Event,
            xproto::{AtomEnum, ConnectionExt, CreateWindowAux, EventMask, Property, WindowClass},
        },
    };
    use std::time::{Duration, Instant};

    let (connection, screen) =
        smithay::reexports::x11rb::connect(Some(display)).map_err(|e| e.to_string())?;
    let root = &connection.setup().roots[screen];
    let requestor = connection.generate_id().map_err(|e| e.to_string())?;
    connection
        .create_window(
            0,
            requestor,
            root.root,
            0,
            0,
            1,
            1,
            0,
            WindowClass::INPUT_ONLY,
            0,
            &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE),
        )
        .map_err(|e| e.to_string())?;
    let atom = |name: &[u8]| {
        connection
            .intern_atom(false, name)
            .map_err(|e| e.to_string())?
            .reply()
            .map(|reply| reply.atom)
            .map_err(|e| e.to_string())
    };
    let clipboard = atom(b"CLIPBOARD")?;
    let image_png = atom(b"image/png")?;
    let property = atom(b"NICKEL_STALLED_SELECTION")?;
    let incr = atom(b"INCR")?;
    connection
        .convert_selection(
            requestor,
            clipboard,
            image_png,
            property,
            smithay::reexports::x11rb::CURRENT_TIME,
        )
        .map_err(|e| e.to_string())?;
    connection.flush().map_err(|e| e.to_string())?;

    let deadline = Instant::now() + Duration::from_secs(15);
    let mut acknowledged = false;
    loop {
        if Instant::now() >= deadline {
            return Err("stalled requestor did not observe abort".into());
        }
        let Some(event) = connection.poll_for_event().map_err(|e| e.to_string())? else {
            std::thread::sleep(Duration::from_millis(1));
            continue;
        };
        match event {
            Event::SelectionNotify(event) if event.requestor == requestor => {
                let reply = connection
                    .get_property(false, requestor, property, AtomEnum::ANY, 0, u32::MAX)
                    .map_err(|e| e.to_string())?
                    .reply()
                    .map_err(|e| e.to_string())?;
                if reply.type_ != incr {
                    return Err("stalled fixture did not enter INCR".into());
                }
                acknowledged = true;
                ready.send(()).map_err(|e| e.to_string())?;
            }
            Event::PropertyNotify(event)
                if acknowledged
                    && event.window == requestor
                    && event.atom == property
                    && event.state == Property::DELETE =>
            {
                return Ok(());
            }
            _ => {}
        }
    }
}

pub(super) fn exercise_x11_admission_limit(
    display: &str,
    result: std::sync::mpsc::Sender<Result<(usize, usize), String>>,
    stop: std::sync::mpsc::Receiver<()>,
) {
    let run = || -> Result<(usize, usize), String> {
        use smithay::reexports::x11rb::{
            connection::Connection,
            protocol::{
                Event,
                xproto::{ConnectionExt, CreateWindowAux, EventMask, WindowClass},
            },
        };
        use std::time::{Duration, Instant};

        let (connection, screen) =
            smithay::reexports::x11rb::connect(Some(display)).map_err(|e| e.to_string())?;
        let root = &connection.setup().roots[screen];
        let atom = |name: &[u8]| {
            connection
                .intern_atom(false, name)
                .map_err(|e| e.to_string())?
                .reply()
                .map(|reply| reply.atom)
                .map_err(|e| e.to_string())
        };
        let clipboard = atom(b"CLIPBOARD")?;
        let image_png = atom(b"image/png")?;
        let mut requestors = Vec::new();
        for index in 0..33 {
            let requestor = connection.generate_id().map_err(|e| e.to_string())?;
            connection
                .create_window(
                    0,
                    requestor,
                    root.root,
                    0,
                    0,
                    1,
                    1,
                    0,
                    WindowClass::INPUT_ONLY,
                    0,
                    &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE),
                )
                .map_err(|e| e.to_string())?;
            let property = atom(format!("NICKEL_ADMISSION_{index}").as_bytes())?;
            connection
                .convert_selection(
                    requestor,
                    clipboard,
                    image_png,
                    property,
                    smithay::reexports::x11rb::CURRENT_TIME,
                )
                .map_err(|e| e.to_string())?;
            requestors.push(requestor);
        }
        connection.flush().map_err(|e| e.to_string())?;
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut accepted = 0;
        let mut rejected = 0;
        while accepted + rejected != requestors.len() {
            if Instant::now() >= deadline {
                return Err(format!(
                    "admission fixture timed out after {accepted} accepted and {rejected} rejected"
                ));
            }
            match connection.poll_for_event().map_err(|e| e.to_string())? {
                Some(Event::SelectionNotify(event)) if requestors.contains(&event.requestor) => {
                    if event.property == smithay::reexports::x11rb::NONE {
                        rejected += 1;
                    } else {
                        accepted += 1;
                    }
                }
                Some(_) => {}
                None => std::thread::sleep(Duration::from_millis(1)),
            }
        }
        result
            .send(Ok((accepted, rejected)))
            .map_err(|e| e.to_string())?;
        let _ = stop.recv_timeout(Duration::from_secs(15));
        Ok((accepted, rejected))
    };
    if let Err(error) = run() {
        let _ = result.send(Err(error));
    }
}

pub(super) fn serve_x11_incremental_clipboard(
    display: &str,
    selection_name: &str,
    payload: Vec<u8>,
    ready: std::sync::mpsc::Sender<()>,
) -> Result<(), String> {
    use smithay::reexports::x11rb::{
        connection::Connection,
        protocol::{
            Event,
            xproto::{
                AtomEnum, ConnectionExt, CreateWindowAux, EventMask, PropMode,
                SELECTION_NOTIFY_EVENT, SelectionNotifyEvent, WindowClass,
            },
        },
        wrapper::ConnectionExt as _,
    };
    use std::time::{Duration, Instant};

    let (connection, screen) =
        smithay::reexports::x11rb::connect(Some(display)).map_err(|e| e.to_string())?;
    let root = &connection.setup().roots[screen];
    let owner = connection.generate_id().map_err(|e| e.to_string())?;
    connection
        .create_window(
            root.root_depth,
            owner,
            root.root,
            0,
            0,
            1,
            1,
            0,
            WindowClass::INPUT_OUTPUT,
            root.root_visual,
            &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE),
        )
        .map_err(|e| e.to_string())?;
    let atom = |name: &[u8]| {
        connection
            .intern_atom(false, name)
            .map_err(|e| e.to_string())?
            .reply()
            .map(|reply| reply.atom)
            .map_err(|e| e.to_string())
    };
    let selection_atom = atom(selection_name.as_bytes())?;
    let targets = atom(b"TARGETS")?;
    let image_png = atom(b"image/png")?;
    let incr = atom(b"INCR")?;
    connection
        .set_selection_owner(
            owner,
            selection_atom,
            smithay::reexports::x11rb::CURRENT_TIME,
        )
        .map_err(|e| e.to_string())?;
    connection.flush().map_err(|e| e.to_string())?;
    ready.send(()).map_err(|e| e.to_string())?;

    let deadline = Instant::now() + Duration::from_secs(15);
    let mut served_payload = false;
    while !served_payload {
        if Instant::now() >= deadline {
            return Err("X11 selection owner timed out".into());
        }
        let Some(event) = connection.poll_for_event().map_err(|e| e.to_string())? else {
            std::thread::sleep(Duration::from_millis(1));
            continue;
        };
        let Event::SelectionRequest(request) = event else {
            continue;
        };
        let property = if request.property == smithay::reexports::x11rb::NONE {
            request.target
        } else {
            request.property
        };
        if request.target == targets {
            connection
                .change_property32(
                    PropMode::REPLACE,
                    request.requestor,
                    property,
                    AtomEnum::ATOM,
                    &[targets, image_png],
                )
                .map_err(|e| e.to_string())?;
        } else if request.target == image_png {
            connection
                .change_window_attributes(
                    request.requestor,
                    &smithay::reexports::x11rb::protocol::xproto::ChangeWindowAttributesAux::new()
                        .event_mask(EventMask::PROPERTY_CHANGE),
                )
                .map_err(|e| e.to_string())?;
            connection
                .change_property32(
                    PropMode::REPLACE,
                    request.requestor,
                    property,
                    incr,
                    &[payload.len() as u32],
                )
                .map_err(|e| e.to_string())?;
        } else {
            connection
                .send_event(
                    false,
                    request.requestor,
                    EventMask::NO_EVENT,
                    SelectionNotifyEvent {
                        response_type: SELECTION_NOTIFY_EVENT,
                        sequence: 0,
                        time: request.time,
                        requestor: request.requestor,
                        selection: request.selection,
                        target: request.target,
                        property: smithay::reexports::x11rb::NONE,
                    },
                )
                .map_err(|e| e.to_string())?;
            connection.flush().map_err(|e| e.to_string())?;
            continue;
        }
        connection
            .send_event(
                false,
                request.requestor,
                EventMask::NO_EVENT,
                SelectionNotifyEvent {
                    response_type: SELECTION_NOTIFY_EVENT,
                    sequence: 0,
                    time: request.time,
                    requestor: request.requestor,
                    selection: request.selection,
                    target: request.target,
                    property,
                },
            )
            .map_err(|e| e.to_string())?;
        connection.flush().map_err(|e| e.to_string())?;
        if request.target != image_png {
            continue;
        }

        let mut offset = 0;
        loop {
            let event = connection.wait_for_event().map_err(|e| e.to_string())?;
            let Event::PropertyNotify(event) = event else {
                continue;
            };
            if event.window != request.requestor
                || event.atom != property
                || event.state != smithay::reexports::x11rb::protocol::xproto::Property::DELETE
            {
                continue;
            }
            let end = (offset + 64 * 1024).min(payload.len());
            connection
                .change_property8(
                    PropMode::REPLACE,
                    request.requestor,
                    property,
                    image_png,
                    &payload[offset..end],
                )
                .map_err(|e| e.to_string())?;
            connection.flush().map_err(|e| e.to_string())?;
            if offset == payload.len() {
                served_payload = true;
                break;
            }
            offset = end;
        }
    }
    Ok(())
}

pub(super) fn serve_x11_pending_clipboard_fixture(
    display: &str,
    reject_transfer: bool,
    ready: std::sync::mpsc::Sender<()>,
    requested: std::sync::mpsc::Sender<()>,
    stop: std::sync::mpsc::Receiver<()>,
) -> Result<(), String> {
    use smithay::reexports::x11rb::{
        connection::Connection,
        protocol::{
            Event,
            xproto::{
                AtomEnum, ConnectionExt, CreateWindowAux, EventMask, PropMode,
                SELECTION_NOTIFY_EVENT, SelectionNotifyEvent, WindowClass,
            },
        },
        wrapper::ConnectionExt as _,
    };
    use std::time::{Duration, Instant};

    let (connection, screen) =
        smithay::reexports::x11rb::connect(Some(display)).map_err(|e| e.to_string())?;
    let root = &connection.setup().roots[screen];
    let owner = connection.generate_id().map_err(|e| e.to_string())?;
    connection
        .create_window(
            root.root_depth,
            owner,
            root.root,
            0,
            0,
            1,
            1,
            0,
            WindowClass::INPUT_OUTPUT,
            root.root_visual,
            &CreateWindowAux::new(),
        )
        .map_err(|e| e.to_string())?;
    let atom = |name: &[u8]| {
        connection
            .intern_atom(false, name)
            .map_err(|e| e.to_string())?
            .reply()
            .map(|reply| reply.atom)
            .map_err(|e| e.to_string())
    };
    let clipboard = atom(b"CLIPBOARD")?;
    let targets = atom(b"TARGETS")?;
    let image_png = atom(b"image/png")?;
    connection
        .set_selection_owner(owner, clipboard, smithay::reexports::x11rb::CURRENT_TIME)
        .map_err(|e| e.to_string())?;
    connection.flush().map_err(|e| e.to_string())?;
    ready.send(()).map_err(|e| e.to_string())?;

    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if Instant::now() >= deadline {
            return Err("pending clipboard fixture timed out".into());
        }
        let Some(Event::SelectionRequest(request)) =
            connection.poll_for_event().map_err(|e| e.to_string())?
        else {
            std::thread::sleep(Duration::from_millis(1));
            continue;
        };
        let property = if request.property == smithay::reexports::x11rb::NONE {
            request.target
        } else {
            request.property
        };
        if request.target == targets {
            connection
                .change_property32(
                    PropMode::REPLACE,
                    request.requestor,
                    property,
                    AtomEnum::ATOM,
                    &[targets, image_png],
                )
                .map_err(|e| e.to_string())?;
            connection
                .send_event(
                    false,
                    request.requestor,
                    EventMask::NO_EVENT,
                    SelectionNotifyEvent {
                        response_type: SELECTION_NOTIFY_EVENT,
                        sequence: 0,
                        time: request.time,
                        requestor: request.requestor,
                        selection: request.selection,
                        target: request.target,
                        property,
                    },
                )
                .map_err(|e| e.to_string())?;
            connection.flush().map_err(|e| e.to_string())?;
            continue;
        }
        if request.target != image_png {
            continue;
        }
        requested.send(()).map_err(|e| e.to_string())?;
        if reject_transfer {
            connection
                .send_event(
                    false,
                    request.requestor,
                    EventMask::NO_EVENT,
                    SelectionNotifyEvent {
                        response_type: SELECTION_NOTIFY_EVENT,
                        sequence: 0,
                        time: request.time,
                        requestor: request.requestor,
                        selection: request.selection,
                        target: request.target,
                        property: smithay::reexports::x11rb::NONE,
                    },
                )
                .map_err(|e| e.to_string())?;
            connection.flush().map_err(|e| e.to_string())?;
            return Ok(());
        }
        let _ = stop.recv_timeout(Duration::from_secs(15));
        return Ok(());
    }
}

#[test]
fn launcher_uses_active_output_global_origin() {
    let placement = internal_shell_surface_placement(
        SurfaceRole::Launcher,
        None,
        (960, 720),
        &outputs(),
        Some("right"),
    );

    assert_eq!(placement.output.as_deref(), Some("right"));
    assert_eq!(placement.geometry, (18, 896, 960, 720));
}

#[test]
fn launcher_placement_anchors_the_actual_content_sized_surface() {
    let placement = internal_shell_surface_placement(
        SurfaceRole::Launcher,
        None,
        (640, 600),
        &outputs(),
        Some("right"),
    );

    assert_eq!(placement.geometry, (18, 1016, 640, 600));
    assert_eq!(placement.geometry.1 + placement.geometry.3 as i32, 1616);
}

#[test]
fn context_menu_moves_away_from_trusted_control_without_leaving_output() {
    assert_eq!(
        avoid_trusted_control_collision(
            (760, 113, 220, 282),
            (0, 0, 1280, 800),
            &[(788, 12, 480, 210)],
        ),
        (560, 113, 220, 282)
    );
    assert_eq!(
        avoid_trusted_control_collision((10, 10, 300, 250), (0, 0, 640, 480), &[(0, 0, 320, 200)],),
        (10, 208, 300, 250)
    );
}

#[test]
fn native_keyboard_uses_authority_height_dock_and_output_without_rescaling() {
    let mut outputs = outputs();
    outputs[0].0.scale = 1.5;
    for (top, y) in [(true, -120), (false, 592)] {
        let placement =
            super::internal_keyboard_surface_placement(Some("left"), top, 368, &outputs).unwrap();
        assert_eq!(placement.geometry, (-1920, y, 1920, 368));
        assert_eq!(placement.output.as_deref(), Some("left"));
    }
    let resized =
        super::internal_keyboard_surface_placement(Some("right"), false, 280, &outputs).unwrap();
    assert_eq!(resized.geometry, (0, 1400, 2560, 280));
    assert!(
        super::internal_keyboard_surface_placement(Some("removed"), false, 368, &outputs).is_none()
    );
    assert!(super::internal_keyboard_surface_placement(None, false, 368, &[]).is_none());
}

#[test]
fn volume_osd_uses_requested_interaction_output_without_launcher_affinity() {
    let placement = internal_shell_surface_placement(
        SurfaceRole::VolumeOsd,
        Some("right"),
        (320, 88),
        &outputs(),
        Some("left"),
    );
    assert_eq!(placement.output.as_deref(), Some("right"));
    assert_eq!(placement.geometry, (0, 240, 320, 88));
    let fallback = internal_shell_surface_placement(
        SurfaceRole::VolumeOsd,
        Some("removed"),
        (320, 88),
        &outputs(),
        None,
    );
    assert_eq!(fallback.output.as_deref(), Some("left"));
}

#[test]
fn switching_active_output_relocates_one_launcher_to_negative_origin() {
    let right = internal_shell_surface_placement(
        SurfaceRole::Launcher,
        None,
        (960, 720),
        &outputs(),
        Some("right"),
    );
    let left = internal_shell_surface_placement(
        SurfaceRole::Launcher,
        None,
        (960, 720),
        &outputs(),
        Some("left"),
    );

    assert_eq!(right.output.as_deref(), Some("right"));
    assert_eq!(left.output.as_deref(), Some("left"));
    assert_eq!(left.geometry, (-1902, 176, 960, 720));
    assert_ne!(right.geometry, left.geometry);
}

#[test]
fn codex_menu_uses_clicked_panel_output_global_coordinates() {
    let anchor = ShellPopoverAnchor {
        control: "panel-codex".into(),
        output: "right".into(),
        bounds: Geometry {
            x: 2200,
            y: 1392,
            width: 48,
            height: 48,
        },
        preferred: AnchorSide::Above,
    };

    let placement = internal_codex_project_menu_placement(Some(&anchor), &outputs(), Some("left"));

    assert_eq!(placement.output.as_deref(), Some("right"));
    assert_eq!(placement.origin, (1964, 936));
    assert_eq!(placement.scale, 1.0);
}

#[test]
fn codex_menu_fallback_includes_negative_output_origin() {
    let placement = internal_codex_project_menu_placement(None, &outputs(), Some("left"));

    assert_eq!(placement.output.as_deref(), Some("left"));
    assert_eq!(placement.origin, (-1920, 216));
}

#[test]
fn codex_menu_fits_a_nested_960_by_600_output_above_the_panel() {
    let outputs = vec![(
        crate::internal_shell::InternalOutput {
            name: "nested".into(),
            width: 960,
            height: 600,
            scale: 1.0,
            x: 0,
            y: 0,
        },
        0,
        0,
    )];
    let placement = internal_codex_project_menu_placement(None, &outputs, None);
    let (width, height) = placement.menu_size.expect("sized menu");
    assert_eq!((width, height), (520, 528));
    assert!(placement.origin.0 >= 0);
    assert!(placement.origin.0 + width as i32 <= 960);
    assert!(placement.origin.1 >= 0);
    assert!(placement.origin.1 + height as i32 <= 544);
}

#[test]
fn codex_chat_frame_is_centered_inside_nonzero_output_work_area() {
    let placement = internal_codex_chat_placement(&outputs(), Some("right"));

    assert_eq!(placement.output.as_deref(), Some("right"));
    assert_eq!(placement.origin, (720, 572));
    assert_eq!(placement.scale, 1.0);
    let outer =
        crate::session::window_frame::outer_geometry(crate::session::shell_layout::Geometry {
            x: placement.origin.0,
            y: placement.origin.1,
            width: crate::internal_codex::CHAT_SIZE.0 as i32,
            height: crate::internal_codex::CHAT_SIZE.1 as i32,
        });
    assert!(outer.x >= 0);
    assert!(outer.y >= 240);
    assert!(outer.x + outer.width <= 2560);
    assert!(outer.y + outer.height <= 240 + 1440 - crate::winit_shell::PANEL_HEIGHT as i32);
}

#[test]
fn codex_chat_frame_preserves_negative_output_origin() {
    let placement = internal_codex_chat_placement(&outputs(), Some("left"));

    assert_eq!(placement.output.as_deref(), Some("left"));
    assert_eq!(placement.origin, (-1520, 32));
    let outer =
        crate::session::window_frame::outer_geometry(crate::session::shell_layout::Geometry {
            x: placement.origin.0,
            y: placement.origin.1,
            width: crate::internal_codex::CHAT_SIZE.0 as i32,
            height: crate::internal_codex::CHAT_SIZE.1 as i32,
        });
    assert!(outer.x >= -1920);
    assert!(outer.y >= -120);
    assert!(outer.x + outer.width <= 0);
    assert!(outer.y + outer.height <= -120 + 1080 - crate::winit_shell::PANEL_HEIGHT as i32);
}

#[test]
fn codex_chat_frame_fits_a_nested_960_by_600_output() {
    let outputs = vec![(
        crate::internal_shell::InternalOutput {
            name: "nested".into(),
            width: 960,
            height: 600,
            scale: 1.0,
            x: 0,
            y: 0,
        },
        0,
        0,
    )];
    let placement = internal_codex_chat_placement(&outputs, Some("nested"));
    let (width, height) = placement.chat_size.expect("bounded chat size");
    assert_eq!((width, height), (950, 494));
    let outer =
        crate::session::window_frame::outer_geometry(crate::session::shell_layout::Geometry {
            x: placement.origin.0,
            y: placement.origin.1,
            width: width as i32,
            height: height as i32,
        });
    assert!(outer.x >= 0 && outer.y >= 0);
    assert!(outer.x + outer.width <= 960);
    assert!(outer.y + outer.height <= 544);
}
