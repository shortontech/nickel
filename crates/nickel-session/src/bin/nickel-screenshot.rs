use std::{ffi::OsString, path::PathBuf};

const HELP: &str = "\
Capture the primary output from a running Nickel compositor.

Usage: nickel-screenshot [OUTPUT.png]

Arguments:
  [OUTPUT.png]  Destination path [default: ~/Pictures/Nickel Screenshot <timestamp>.png]

Options:
  -h, --help    Print help
";

enum Command {
    Capture(Option<PathBuf>),
    Help,
}

fn parse_command(args: impl IntoIterator<Item = OsString>) -> Result<Command, String> {
    let mut args = args.into_iter();
    let Some(argument) = args.next() else {
        return Ok(Command::Capture(None));
    };
    if argument == "-h" || argument == "--help" {
        if args.next().is_some() {
            return Err("help does not accept additional arguments".into());
        }
        return Ok(Command::Help);
    }
    if args.next().is_some() {
        return Err("expected at most one output path".into());
    }
    Ok(Command::Capture(Some(PathBuf::from(argument))))
}

#[cfg(unix)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use nickel_session_protocol::{
        CaptureResult, ClientEnvelope, Command as SessionCommand, Event, Request, ServerEnvelope,
        ServerMessage, decode, encode,
    };
    use std::{
        env, fs,
        os::unix::net::UnixDatagram,
        process,
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    let output = match parse_command(env::args_os().skip(1))? {
        Command::Help => {
            print!("{HELP}");
            return Ok(());
        }
        Command::Capture(output) => output,
    };
    let control = env::var_os("NICKEL_SESSION_CONTROL")
        .map(PathBuf::from)
        .ok_or("NICKEL_SESSION_CONTROL is not set")?;
    let output = if let Some(output) = output {
        output
    } else {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let pictures = env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or("HOME is not set; provide an explicit screenshot output path")?
            .join("Pictures");
        pictures.join(format!("Nickel Screenshot {timestamp}.png"))
    };
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }

    let runtime = env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(env::temp_dir);
    let reply_path = runtime.join(format!("nickel-screenshot-{}.sock", process::id()));
    let _ = fs::remove_file(&reply_path);
    let socket = UnixDatagram::bind(&reply_path)?;
    socket.set_read_timeout(Some(Duration::from_secs(15)))?;
    let request_id = 1;
    let request = ClientEnvelope {
        token: env::var("NICKEL_SESSION_TOKEN").map_err(|_| "NICKEL_SESSION_TOKEN is not set")?,
        request_id,
        request: Request::Command(SessionCommand::CaptureOutput {
            path: output.to_string_lossy().into_owned(),
            output: None,
        }),
    };
    socket.send_to(&encode(&request)?, control)?;

    let mut response = vec![0_u8; nickel_session_protocol::MAX_FRAME_BYTES];
    let length = socket.recv(&mut response)?;
    let acknowledgement = decode::<ServerEnvelope>(&response[..length])?;
    if acknowledgement.request_id != request_id {
        return Err("capture acknowledgement had the wrong request id".into());
    }
    if let ServerMessage::Error { message, .. } = acknowledgement.message {
        return Err(message.into());
    }
    if !matches!(acknowledgement.message, ServerMessage::Ack) {
        return Err("capture request was not acknowledged".into());
    }
    let length = socket.recv(&mut response)?;
    let completion = decode::<ServerEnvelope>(&response[..length])?;
    let _ = fs::remove_file(&reply_path);
    if completion.request_id != request_id {
        return Err("capture completion had the wrong request id".into());
    }
    match completion.message {
        ServerMessage::Event(Event::OutputCaptureCompleted {
            path,
            result: CaptureResult::Saved { .. },
        }) if std::path::Path::new(&path) == output => {
            println!("{}", output.display());
            Ok(())
        }
        ServerMessage::Event(Event::OutputCaptureCompleted {
            result: CaptureResult::Failed { message },
            ..
        }) => Err(message.into()),
        _ => Err("unexpected capture completion response".into()),
    }
}

#[cfg(not(unix))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    match parse_command(std::env::args_os().skip(1))? {
        Command::Help => {
            print!("{HELP}");
            Ok(())
        }
        Command::Capture(_) => {
            Err("Nickel compositor screenshots are only available on Unix sessions".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Command, parse_command};

    #[test]
    fn help_aliases_produce_the_same_command() {
        for argument in ["--help", "-h"] {
            assert!(
                matches!(parse_command([argument.into()]).unwrap(), Command::Help),
                "argument {argument:?}"
            );
        }
    }

    #[test]
    fn accepts_an_output_path() {
        let Command::Capture(Some(path)) = parse_command(["capture.png".into()]).unwrap() else {
            panic!("expected a capture command");
        };
        assert_eq!(path, std::path::Path::new("capture.png"));
    }

    #[test]
    fn rejects_additional_arguments() {
        let error = parse_command(["one.png".into(), "two.png".into()])
            .err()
            .expect("additional arguments should fail");
        assert_eq!(error, "expected at most one output path");
    }
}
