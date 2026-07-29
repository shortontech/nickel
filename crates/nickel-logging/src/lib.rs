use std::{
    env,
    fs::{self, OpenOptions},
    io,
    path::{Path, PathBuf},
    sync::Mutex,
};

use tracing_subscriber::{EnvFilter, fmt, prelude::*};

const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;

pub fn init(application: &str) -> io::Result<PathBuf> {
    let directory = log_directory()?;
    fs::create_dir_all(&directory)?;
    let path = directory.join(format!("{application}.log"));
    rotate_if_needed(&path)?;
    let file = OpenOptions::new().create(true).append(true).open(&path)?;
    let filter = EnvFilter::try_from_env("NICKEL_LOG")
        .unwrap_or_else(|_| EnvFilter::new("nickel=debug,warn"));
    let subscriber = tracing_subscriber::registry().with(filter).with(
        fmt::layer()
            .with_ansi(false)
            .with_target(true)
            .with_writer(Mutex::new(file)),
    );
    let _ = subscriber.try_init();
    install_panic_logging();
    tracing::info!(application, path = %path.display(), "logging initialized");
    Ok(path)
}

fn log_directory() -> io::Result<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        if let Some(local_app_data) = env::var_os("LOCALAPPDATA") {
            return Ok(PathBuf::from(local_app_data).join("Nickel").join("logs"));
        }
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "LOCALAPPDATA is not set",
        ));
    }
    #[cfg(not(target_os = "windows"))]
    {
        let state_home = env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                env::var_os("HOME").map(|home| PathBuf::from(home).join(".local").join("state"))
            })
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "XDG_STATE_HOME and HOME are not set",
                )
            })?;
        Ok(state_home.join("nickel").join("logs"))
    }
}

fn rotate_if_needed(path: &Path) -> io::Result<()> {
    if path
        .metadata()
        .is_ok_and(|metadata| metadata.len() >= MAX_LOG_BYTES)
    {
        let previous = path.with_extension("log.previous");
        if previous.exists() {
            fs::remove_file(&previous)?;
        }
        fs::rename(path, previous)?;
    }
    Ok(())
}

fn install_panic_logging() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |information| {
        tracing::error!(panic = %information, "application panicked");
        previous(information);
    }));
}

#[cfg(test)]
mod tests {
    use super::rotate_if_needed;
    use std::{fs, io::Write};

    #[test]
    fn small_log_is_not_rotated() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("nickel.log");
        let mut file = fs::File::create(&path).unwrap();
        file.write_all(b"hello").unwrap();
        drop(file);
        rotate_if_needed(&path).unwrap();
        assert!(path.exists());
        assert!(!path.with_extension("log.previous").exists());
    }
}
