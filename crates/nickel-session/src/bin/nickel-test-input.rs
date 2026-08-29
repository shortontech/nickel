use std::ffi::OsString;

use nickel_session_protocol::{InputState, TestInput, TestKey, TestPointerButton};

const HELP: &str = "\
Inject one input event into a Nickel nested session started with --test-control.

Usage:
  nickel-test-input windows
  nickel-test-input move X Y
  nickel-test-input button left|right pressed|released
  nickel-test-input key a|tab|alt|shift|meta|print-screen pressed|released
";

enum Parsed {
    Input(TestInput),
    Windows,
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
        [command, x, y] if command == "move" => Ok(Parsed::Input(TestInput::PointerMove {
            x: x.parse()
                .map_err(|_| format!("invalid X coordinate {x:?}"))?,
            y: y.parse()
                .map_err(|_| format!("invalid Y coordinate {y:?}"))?,
        })),
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
                "tab" => TestKey::Tab,
                "alt" => TestKey::LeftAlt,
                "shift" => TestKey::LeftShift,
                "meta" => TestKey::LeftMeta,
                "print-screen" => TestKey::PrintScreen,
                _ => return Err(format!("unknown key {key:?}")),
            },
            state: parse_state(state)?,
        })),
        _ => Err("expected move, button, or key command; use --help".into()),
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

    let command = match parse(env::args_os().skip(1))? {
        Parsed::Help => {
            print!("{HELP}");
            return Ok(());
        }
        Parsed::Input(input) => Request::Command(Command::TestInput { input }),
        Parsed::Windows => Request::Query(nickel_session_protocol::Query::Windows),
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
    let envelope = ClientEnvelope {
        token: env::var("NICKEL_SESSION_TOKEN")?,
        request_id: 1,
        request: command,
    };
    socket.send_to(&encode(&envelope)?, control)?;
    let mut response = vec![0_u8; nickel_session_protocol::MAX_FRAME_BYTES];
    let length = socket.recv(&mut response)?;
    let _ = fs::remove_file(&reply_path);
    let response = decode::<ServerEnvelope>(&response[..length])?;
    match response.message {
        ServerMessage::Ack => Ok(()),
        ServerMessage::Windows(windows) => {
            for window in windows {
                println!(
                    "{}\t{}\t{}\t{}\t{}\t{}",
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
                    }
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
        Parsed::Input(_) | Parsed::Windows => {
            Err("nested compositor test input is only available on Unix".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_each_input_family() {
        assert!(matches!(
            parse(["move".into(), "64".into(), "700".into()]),
            Ok(Parsed::Input(TestInput::PointerMove { x: 64, y: 700 }))
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
        assert!(parse(["key".into(), "escape".into(), "pressed".into()]).is_err());
    }
}
