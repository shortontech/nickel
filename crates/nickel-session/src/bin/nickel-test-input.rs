use std::ffi::OsString;

use nickel_session_protocol::{
    InputState, PointerInteraction, PreviewTargetAction, ShellSemanticTarget, TestInput, TestKey,
    TestPointerButton, WindowMenuTargetAction,
};

const HELP: &str = "\
Inject one input event into a Nickel nested session started with --test-control.

Usage:
  nickel-test-input windows
  nickel-test-input workspaces
  nickel-test-input outputs
  nickel-test-input surfaces
  nickel-test-input output-connect NAME WIDTH HEIGHT SCALE_120 normal|90|180|270
  nickel-test-input output-disconnect NAME
  nickel-test-input workspace-create
  nickel-test-input workspace-switch ID
  nickel-test-input workspace-remove ID
  nickel-test-input workspace-move WINDOW_ID WORKSPACE_ID
  nickel-test-input window activate|close|minimize|maximize|fullscreen WINDOW_ID
  nickel-test-input session restart-shell|lock|unlock|suspend|logout|reboot|power-off
  nickel-test-input idle-inhibition
  nickel-test-input caches
  nickel-test-input semantic panel-app APPLICATION_ID hover|click [OUTPUT]
  nickel-test-input semantic preview WINDOW_ID hover|activate|close|menu
  nickel-test-input semantic menu WINDOW_ID close|maximize|minimize
  nickel-test-input move X Y
  nickel-test-input move-relative DX DY
  nickel-test-input button left|right pressed|released
  nickel-test-input key a|c|p|enter|escape|tab|alt|shift|control|meta|left|right|up|down|space|backspace|delete|f11|print-screen pressed|released
";

enum Parsed {
    Input(TestInput),
    Semantic(ShellSemanticTarget),
    Windows,
    Workspaces,
    Outputs,
    Surfaces,
    OutputConnect {
        name: String,
        width: i32,
        height: i32,
        scale_120: u32,
        transform: nickel_session_protocol::OutputTransform,
    },
    OutputDisconnect(String),
    WorkspaceCreate,
    WorkspaceSwitch(u64),
    WorkspaceRemove(u64),
    WorkspaceMove {
        window: u64,
        workspace: u64,
    },
    WindowAction {
        window: u64,
        action: nickel_session_protocol::WindowAction,
    },
    SessionAction(Option<nickel_session_protocol::SessionAction>),
    Unlock,
    IdleInhibition,
    Caches,
    Help,
}

fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Parsed, String> {
    let args = args
        .into_iter()
        .map(|value| {
            value
                .into_string()
                .map_err(|_| "arguments must be UTF-8".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    if matches!(args.as_slice(), [value] if value == "-h" || value == "--help") {
        return Ok(Parsed::Help);
    }
    match args.as_slice() {
        [command] if command == "windows" => Ok(Parsed::Windows),
        [command] if command == "workspaces" => Ok(Parsed::Workspaces),
        [command] if command == "outputs" => Ok(Parsed::Outputs),
        [command] if command == "surfaces" => Ok(Parsed::Surfaces),
        [command, name] if command == "output-disconnect" => {
            Ok(Parsed::OutputDisconnect(name.clone()))
        }
        [command, name, width, height, scale_120, transform] if command == "output-connect" => {
            Ok(Parsed::OutputConnect {
                name: name.clone(),
                width: width
                    .parse()
                    .map_err(|_| format!("invalid output width {width:?}"))?,
                height: height
                    .parse()
                    .map_err(|_| format!("invalid output height {height:?}"))?,
                scale_120: scale_120
                    .parse()
                    .map_err(|_| format!("invalid output scale {scale_120:?}"))?,
                transform: match transform.as_str() {
                    "normal" => nickel_session_protocol::OutputTransform::Normal,
                    "90" => nickel_session_protocol::OutputTransform::Rotate90,
                    "180" => nickel_session_protocol::OutputTransform::Rotate180,
                    "270" => nickel_session_protocol::OutputTransform::Rotate270,
                    _ => return Err(format!("unknown output transform {transform:?}")),
                },
            })
        }
        [command] if command == "workspace-create" => Ok(Parsed::WorkspaceCreate),
        [command, id] if command == "workspace-switch" => Ok(Parsed::WorkspaceSwitch(
            id.parse()
                .map_err(|_| format!("invalid workspace ID {id:?}"))?,
        )),
        [command, id] if command == "workspace-remove" => Ok(Parsed::WorkspaceRemove(
            id.parse()
                .map_err(|_| format!("invalid workspace ID {id:?}"))?,
        )),
        [command, window, workspace] if command == "workspace-move" => Ok(Parsed::WorkspaceMove {
            window: window
                .parse()
                .map_err(|_| format!("invalid window ID {window:?}"))?,
            workspace: workspace
                .parse()
                .map_err(|_| format!("invalid workspace ID {workspace:?}"))?,
        }),
        [command, action, window] if command == "window" => Ok(Parsed::WindowAction {
            window: window
                .parse()
                .map_err(|_| format!("invalid window ID {window:?}"))?,
            action: match action.as_str() {
                "activate" => nickel_session_protocol::WindowAction::Activate,
                "close" => nickel_session_protocol::WindowAction::Close,
                "minimize" => nickel_session_protocol::WindowAction::Minimize,
                "maximize" => nickel_session_protocol::WindowAction::MaximizeRestore,
                "fullscreen" => nickel_session_protocol::WindowAction::FullscreenRestore,
                _ => return Err(format!("unknown window action {action:?}")),
            },
        }),
        [command, action] if command == "session" => {
            if action == "unlock" {
                return Ok(Parsed::Unlock);
            }
            Ok(Parsed::SessionAction(match action.as_str() {
                "restart-shell" => Some(nickel_session_protocol::SessionAction::RestartShell),
                "lock" => Some(nickel_session_protocol::SessionAction::Lock),
                "suspend" => Some(nickel_session_protocol::SessionAction::Suspend),
                "logout" => None,
                "reboot" => Some(nickel_session_protocol::SessionAction::Reboot),
                "power-off" => Some(nickel_session_protocol::SessionAction::PowerOff),
                _ => return Err(format!("unknown session action {action:?}")),
            }))
        }
        [command] if command == "idle-inhibition" => Ok(Parsed::IdleInhibition),
        [command] if command == "caches" => Ok(Parsed::Caches),
        [command, kind, application_id, interaction]
        | [command, kind, application_id, interaction, _]
            if command == "semantic" && kind == "panel-app" =>
        {
            let output = args.get(4).cloned();
            Ok(Parsed::Semantic(ShellSemanticTarget::PanelApplication {
                application_id: application_id.clone(),
                output,
                interaction: match interaction.as_str() {
                    "hover" => PointerInteraction::Hover,
                    "click" => PointerInteraction::LeftClick,
                    _ => return Err(format!("unknown panel interaction {interaction:?}")),
                },
            }))
        }
        [command, kind, window, action] if command == "semantic" && kind == "preview" => {
            Ok(Parsed::Semantic(ShellSemanticTarget::PreviewWindow {
                window: nickel_session_protocol::WindowId(
                    window
                        .parse()
                        .map_err(|_| format!("invalid window ID {window:?}"))?,
                ),
                action: match action.as_str() {
                    "hover" => PreviewTargetAction::Hover,
                    "activate" => PreviewTargetAction::Activate,
                    "close" => PreviewTargetAction::Close,
                    "menu" => PreviewTargetAction::OpenMenu,
                    _ => return Err(format!("unknown preview action {action:?}")),
                },
            }))
        }
        [command, kind, window, action] if command == "semantic" && kind == "menu" => {
            Ok(Parsed::Semantic(ShellSemanticTarget::WindowMenu {
                window: nickel_session_protocol::WindowId(
                    window
                        .parse()
                        .map_err(|_| format!("invalid window ID {window:?}"))?,
                ),
                action: match action.as_str() {
                    "close" => WindowMenuTargetAction::Close,
                    "maximize" => WindowMenuTargetAction::MaximizeRestore,
                    "minimize" => WindowMenuTargetAction::Minimize,
                    _ => return Err(format!("unknown menu action {action:?}")),
                },
            }))
        }
        [command, x, y] if command == "move" => Ok(Parsed::Input(TestInput::PointerMove {
            x: x.parse()
                .map_err(|_| format!("invalid X coordinate {x:?}"))?,
            y: y.parse()
                .map_err(|_| format!("invalid Y coordinate {y:?}"))?,
        })),
        [command, dx, dy] if command == "move-relative" => {
            Ok(Parsed::Input(TestInput::PointerMoveRelative {
                dx: dx.parse().map_err(|_| format!("invalid X delta {dx:?}"))?,
                dy: dy.parse().map_err(|_| format!("invalid Y delta {dy:?}"))?,
            }))
        }
        [command, button, state] if command == "button" => {
            Ok(Parsed::Input(TestInput::PointerButton {
                button: match button.as_str() {
                    "left" => TestPointerButton::Left,
                    "right" => TestPointerButton::Right,
                    _ => return Err(format!("unknown pointer button {button:?}")),
                },
                state: parse_state(state)?,
            }))
        }
        [command, key, state] if command == "key" => Ok(Parsed::Input(TestInput::Key {
            key: match key.as_str() {
                "a" => TestKey::A,
                "c" => TestKey::C,
                "p" => TestKey::P,
                "enter" => TestKey::Enter,
                "escape" => TestKey::Escape,
                "tab" => TestKey::Tab,
                "alt" => TestKey::LeftAlt,
                "shift" => TestKey::LeftShift,
                "control" => TestKey::LeftControl,
                "meta" => TestKey::LeftMeta,
                "left" => TestKey::Left,
                "right" => TestKey::Right,
                "up" => TestKey::Up,
                "down" => TestKey::Down,
                "space" => TestKey::Space,
                "backspace" => TestKey::Backspace,
                "delete" => TestKey::Delete,
                "f11" => TestKey::F11,
                "print-screen" => TestKey::PrintScreen,
                _ => return Err(format!("unknown key {key:?}")),
            },
            state: parse_state(state)?,
        })),
        _ => Err("expected move, move-relative, button, or key command; use --help".into()),
    }
}

fn parse_state(value: &str) -> Result<InputState, String> {
    match value {
        "pressed" => Ok(InputState::Pressed),
        "released" => Ok(InputState::Released),
        _ => Err(format!("unknown input state {value:?}")),
    }
}

#[cfg(unix)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use nickel_session_protocol::{
        ClientEnvelope, Command, Request, ServerEnvelope, ServerMessage, decode, encode,
    };
    use std::{env, fs, os::unix::net::UnixDatagram, path::PathBuf, process, time::Duration};

    let (request, semantic) = match parse(env::args_os().skip(1))? {
        Parsed::Help => {
            print!("{HELP}");
            return Ok(());
        }
        Parsed::Input(input) => (Some(Request::Command(Command::TestInput { input })), None),
        Parsed::Semantic(target) => (None, Some(target)),
        Parsed::Windows => (
            Some(Request::Query(nickel_session_protocol::Query::Windows)),
            None,
        ),
        Parsed::Workspaces => (
            Some(Request::Query(nickel_session_protocol::Query::Workspaces)),
            None,
        ),
        Parsed::Outputs => (
            Some(Request::Query(nickel_session_protocol::Query::Outputs)),
            None,
        ),
        Parsed::Surfaces => (
            Some(Request::Query(
                nickel_session_protocol::Query::ShellSurfaces,
            )),
            None,
        ),
        Parsed::OutputConnect {
            name,
            width,
            height,
            scale_120,
            transform,
        } => (
            Some(Request::Command(Command::TestOutput {
                output: nickel_session_protocol::TestOutput::Connect {
                    name,
                    logical_width: width,
                    logical_height: height,
                    scale_120,
                    transform,
                },
            })),
            None,
        ),
        Parsed::OutputDisconnect(name) => (
            Some(Request::Command(Command::TestOutput {
                output: nickel_session_protocol::TestOutput::Disconnect { name },
            })),
            None,
        ),
        Parsed::WorkspaceCreate => (Some(Request::Command(Command::CreateWorkspace)), None),
        Parsed::WorkspaceSwitch(workspace) => (
            Some(Request::Command(Command::SwitchWorkspace {
                workspace: nickel_session_protocol::WorkspaceId(workspace),
                output: None,
            })),
            None,
        ),
        Parsed::WorkspaceRemove(workspace) => (
            Some(Request::Command(Command::RemoveWorkspace {
                workspace: nickel_session_protocol::WorkspaceId(workspace),
            })),
            None,
        ),
        Parsed::WorkspaceMove { window, workspace } => (
            Some(Request::Command(Command::MoveWindowToWorkspace {
                window: nickel_session_protocol::WindowId(window),
                workspace: nickel_session_protocol::WorkspaceId(workspace),
            })),
            None,
        ),
        Parsed::WindowAction { window, action } => (
            Some(Request::Command(Command::WindowAction {
                window: nickel_session_protocol::WindowId(window),
                action,
            })),
            None,
        ),
        Parsed::SessionAction(Some(action)) => (
            Some(Request::Command(Command::SessionAction { action })),
            None,
        ),
        Parsed::SessionAction(None) => (Some(Request::Command(Command::LogOut)), None),
        Parsed::Unlock => (Some(Request::Command(Command::Unlock)), None),
        Parsed::IdleInhibition => (
            Some(Request::Query(
                nickel_session_protocol::Query::IdleInhibition,
            )),
            None,
        ),
        Parsed::Caches => (
            Some(Request::Query(
                nickel_session_protocol::Query::CacheDiagnostics,
            )),
            None,
        ),
    };
    let control = env::var_os("NICKEL_SESSION_CONTROL")
        .map(PathBuf::from)
        .ok_or("NICKEL_SESSION_CONTROL is not set")?;
    let runtime = env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(env::temp_dir);
    let reply_path = runtime.join(format!("nickel-test-input-{}.sock", process::id()));
    let _ = fs::remove_file(&reply_path);
    let socket = UnixDatagram::bind(&reply_path)?;
    socket.set_read_timeout(Some(Duration::from_secs(15)))?;
    let token = env::var("NICKEL_SESSION_TOKEN")?;
    let mut request_id = 1;
    let (destination, request) = if let Some(target) = semantic {
        let destination = env::var_os("NICKEL_SHELL_TEST_CONTROL")
            .map(PathBuf::from)
            .ok_or("NICKEL_SHELL_TEST_CONTROL is not set")?;
        (
            destination,
            Request::Query(nickel_session_protocol::Query::ShellSemanticTarget { target }),
        )
    } else {
        (
            control.clone(),
            request.expect("non-semantic request exists"),
        )
    };
    let envelope = ClientEnvelope {
        token: token.clone(),
        request_id: 1,
        request,
    };
    socket.send_to(&encode(&envelope)?, destination)?;
    let mut response = vec![0_u8; nickel_session_protocol::MAX_FRAME_BYTES];
    let mut length = socket.recv(&mut response)?;
    let mut response_envelope = decode::<ServerEnvelope>(&response[..length])?;
    if response_envelope.request_id != request_id {
        return Err("test input response has the wrong request ID".into());
    }
    if let ServerMessage::ShellSemanticTarget(target) = response_envelope.message.clone() {
        request_id += 1;
        socket.send_to(
            &encode(&ClientEnvelope {
                token,
                request_id,
                request: Request::Command(Command::TestInput {
                    input: TestInput::ShellPointer { target },
                }),
            })?,
            control,
        )?;
        length = socket.recv(&mut response)?;
        response_envelope = decode::<ServerEnvelope>(&response[..length])?;
        if response_envelope.request_id != request_id {
            return Err("test input response has the wrong request ID".into());
        }
    }
    let _ = fs::remove_file(&reply_path);
    let response = response_envelope;
    match response.message {
        ServerMessage::Ack => Ok(()),
        ServerMessage::Windows(windows) => {
            for window in windows {
                println!(
                    "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                    window.id.0,
                    window.application_id,
                    window.title,
                    if window.active { "active" } else { "inactive" },
                    if window.minimized {
                        "minimized"
                    } else {
                        "shown"
                    },
                    if window.maximized {
                        "maximized"
                    } else {
                        "restored"
                    },
                    if window.fullscreen {
                        "fullscreen"
                    } else {
                        "windowed"
                    },
                    window.geometry.map_or_else(
                        || "unmapped".to_owned(),
                        |geometry| format!(
                            "{},{} {}x{}",
                            geometry.x, geometry.y, geometry.width, geometry.height
                        ),
                    )
                );
            }
            Ok(())
        }
        ServerMessage::IdleInhibition { surfaces } => {
            println!("{surfaces}");
            Ok(())
        }
        ServerMessage::CacheDiagnostics(diagnostics) => {
            println!(
                "previews={}/{} bytes={}",
                diagnostics.preview_entries,
                diagnostics.preview_capacity,
                diagnostics.preview_bytes
            );
            Ok(())
        }
        ServerMessage::Workspaces(state) => {
            for workspace in state.ordered {
                println!(
                    "{}\t{}\t{}",
                    workspace.id.0,
                    if workspace.id == state.active {
                        "active"
                    } else {
                        "inactive"
                    },
                    workspace
                        .windows
                        .iter()
                        .map(|window| window.0.to_string())
                        .collect::<Vec<_>>()
                        .join(",")
                );
            }
            Ok(())
        }
        ServerMessage::Outputs(outputs) => {
            for output in outputs {
                println!(
                    "{}\t{},{} {}x{}\tscale={}/120\t{:?}\t{}",
                    output.name,
                    output.geometry.x,
                    output.geometry.y,
                    output.geometry.width,
                    output.geometry.height,
                    output.scale_120,
                    output.transform,
                    if output.primary {
                        "primary"
                    } else {
                        "secondary"
                    }
                );
            }
            Ok(())
        }
        ServerMessage::ShellSurfaces(surfaces) => {
            for surface in surfaces {
                println!(
                    "{:?}\t{}\t{}",
                    surface.role,
                    surface.output.as_deref().unwrap_or("unmapped"),
                    surface.geometry.map_or_else(
                        || "hidden".to_owned(),
                        |geometry| format!(
                            "{},{} {}x{}",
                            geometry.x, geometry.y, geometry.width, geometry.height
                        )
                    )
                );
            }
            Ok(())
        }
        ServerMessage::Error { message, .. } => Err(message.into()),
        _ => Err("unexpected test input response".into()),
    }
}

#[cfg(not(unix))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    match parse(std::env::args_os().skip(1))? {
        Parsed::Help => {
            print!("{HELP}");
            Ok(())
        }
        Parsed::Input(_)
        | Parsed::Semantic(_)
        | Parsed::Windows
        | Parsed::Workspaces
        | Parsed::Outputs
        | Parsed::Surfaces
        | Parsed::OutputConnect { .. }
        | Parsed::OutputDisconnect(_)
        | Parsed::WorkspaceCreate
        | Parsed::WorkspaceSwitch(_)
        | Parsed::WorkspaceRemove(_)
        | Parsed::WorkspaceMove { .. }
        | Parsed::WindowAction { .. }
        | Parsed::IdleInhibition => {
            Err("nested compositor test input is only available on Unix".into())
        }
        Parsed::Caches => Err("nested compositor diagnostics are only available on Unix".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_each_input_family() {
        assert!(matches!(
            parse(["idle-inhibition".into()]),
            Ok(Parsed::IdleInhibition)
        ));
        assert!(matches!(parse(["caches".into()]), Ok(Parsed::Caches)));
        assert!(matches!(
            parse(["workspaces".into()]),
            Ok(Parsed::Workspaces)
        ));
        assert!(matches!(
            parse([
                "semantic".into(),
                "panel-app".into(),
                "org.kde.konsole".into(),
                "hover".into(),
                "DP-1".into(),
            ]),
            Ok(Parsed::Semantic(ShellSemanticTarget::PanelApplication {
                application_id,
                output: Some(output),
                interaction: PointerInteraction::Hover,
            })) if application_id == "org.kde.konsole" && output == "DP-1"
        ));
        assert!(matches!(
            parse([
                "semantic".into(),
                "preview".into(),
                "9".into(),
                "menu".into(),
            ]),
            Ok(Parsed::Semantic(ShellSemanticTarget::PreviewWindow {
                window: nickel_session_protocol::WindowId(9),
                action: PreviewTargetAction::OpenMenu,
            }))
        ));
        assert!(matches!(
            parse([
                "output-connect".into(),
                "DP-test".into(),
                "1024".into(),
                "768".into(),
                "180".into(),
                "90".into()
            ]),
            Ok(Parsed::OutputConnect {
                width: 1024,
                height: 768,
                scale_120: 180,
                transform: nickel_session_protocol::OutputTransform::Rotate90,
                ..
            })
        ));
        assert!(matches!(
            parse(["workspace-switch".into(), "2".into()]),
            Ok(Parsed::WorkspaceSwitch(2))
        ));
        assert!(matches!(
            parse(["workspace-move".into(), "7".into(), "3".into()]),
            Ok(Parsed::WorkspaceMove {
                window: 7,
                workspace: 3
            })
        ));
        assert!(matches!(
            parse(["session".into(), "restart-shell".into()]),
            Ok(Parsed::SessionAction(Some(
                nickel_session_protocol::SessionAction::RestartShell
            )))
        ));
        assert!(matches!(
            parse(["session".into(), "logout".into()]),
            Ok(Parsed::SessionAction(None))
        ));
        assert!(matches!(
            parse(["move".into(), "64".into(), "700".into()]),
            Ok(Parsed::Input(TestInput::PointerMove { x: 64, y: 700 }))
        ));
        assert!(matches!(
            parse(["move-relative".into(), "12".into(), "-7".into()]),
            Ok(Parsed::Input(TestInput::PointerMoveRelative {
                dx: 12,
                dy: -7
            }))
        ));
        assert!(matches!(
            parse(["button".into(), "right".into(), "pressed".into()]),
            Ok(Parsed::Input(TestInput::PointerButton {
                button: TestPointerButton::Right,
                state: InputState::Pressed
            }))
        ));
        assert!(matches!(
            parse(["key".into(), "alt".into(), "released".into()]),
            Ok(Parsed::Input(TestInput::Key {
                key: TestKey::LeftAlt,
                state: InputState::Released
            }))
        ));
    }

    #[test]
    fn rejects_unknown_inputs() {
        assert!(parse(["button".into(), "middle".into(), "pressed".into()]).is_err());
        assert!(parse(["key".into(), "home".into(), "pressed".into()]).is_err());
    }
}
