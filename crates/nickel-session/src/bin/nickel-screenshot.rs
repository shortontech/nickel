#[cfg(unix)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::{
        env, fs,
        os::unix::net::UnixDatagram,
        path::PathBuf,
        process,
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    let control = env::var_os("NICKEL_SESSION_CONTROL")
        .map(PathBuf::from)
        .ok_or("NICKEL_SESSION_CONTROL is not set")?;
    let output = env::args_os().nth(1).map(PathBuf::from).unwrap_or_else(|| {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let pictures = env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(env::temp_dir)
            .join("Pictures");
        pictures.join(format!("Nickel Screenshot {timestamp}.png"))
    });
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
    Err("Nickel compositor screenshots are only available on Unix sessions".into())
}
