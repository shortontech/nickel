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
    socket.send_to(
        format!("capture-output\t{}", output.display()).as_bytes(),
        control,
    )?;

    let mut response = [0_u8; 4096];
    let length = socket.recv(&mut response)?;
    let response = std::str::from_utf8(&response[..length])?;
    let _ = fs::remove_file(&reply_path);
    if response == "ok" {
        // Compatibility with the first asynchronous capture implementation,
        // which returned OpenGL rows in the opposite orientation.
        image::open(&output)?.flipv().save(&output)?;
        println!("{}", output.display());
        Ok(())
    } else if response == "ok\tnative" {
        println!("{}", output.display());
        Ok(())
    } else {
        Err(response.to_owned().into())
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
    fn recognizes_long_help() {
        assert!(matches!(
            parse_command(["--help".into()]).unwrap(),
            Command::Help
        ));
    }

    #[test]
    fn recognizes_short_help() {
        assert!(matches!(
            parse_command(["-h".into()]).unwrap(),
            Command::Help
        ));
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
