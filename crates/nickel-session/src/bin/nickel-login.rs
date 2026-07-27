use std::{
    env,
    ffi::OsStr,
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Output},
    thread,
    time::{Duration, Instant},
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let executable = env::current_exe()?;
    let directory = executable
        .parent()
        .ok_or("Nickel login launcher has no executable directory")?;
    let session = sibling_binary(directory, "nickel-session")?;
    let shell = sibling_binary(directory, "nickel-ui")?;

    prepare_secure_storage()?;

    let error = Command::new(session)
        .arg("--backend")
        .arg("udev")
        .arg("--command")
        .arg(shell)
        .exec();
    Err(error.into())
}

fn prepare_secure_storage() -> Result<(), Box<dyn std::error::Error>> {
    // SAFETY: nickel-login is single-threaded and has not launched a child.
    unsafe {
        env::set_var("XDG_SESSION_TYPE", "wayland");
        env::set_var("XDG_CURRENT_DESKTOP", "Nickel");
        env::set_var("XDG_SESSION_DESKTOP", "Nickel");
    }

    let pam_socket = env::var_os("PAM_KWALLET5_LOGIN")
        .ok_or("SDDM did not provide the KWallet PAM login socket")?;
    if !Path::new(&pam_socket).exists() {
        return Err(format!(
            "KWallet PAM login socket does not exist: {}",
            Path::new(&pam_socket).display()
        )
        .into());
    }

    run_timed(
        "dbus-update-activation-environment",
        [
            OsStr::new("--systemd"),
            OsStr::new("PAM_KWALLET5_LOGIN"),
            OsStr::new("XDG_CURRENT_DESKTOP"),
            OsStr::new("XDG_SESSION_DESKTOP"),
            OsStr::new("XDG_SESSION_TYPE"),
        ],
    )?;
    run_timed(
        "systemctl",
        [
            OsStr::new("--user"),
            OsStr::new("--no-block"),
            OsStr::new("start"),
            OsStr::new("plasma-kwallet-pam.service"),
        ],
    )?;

    if wait_for_secret_service(Duration::from_secs(5)).is_err() {
        run_timed(
            "busctl",
            [
                OsStr::new("--user"),
                OsStr::new("call"),
                OsStr::new("org.freedesktop.DBus"),
                OsStr::new("/org/freedesktop/DBus"),
                OsStr::new("org.freedesktop.DBus"),
                OsStr::new("StartServiceByName"),
                OsStr::new("su"),
                OsStr::new("org.kde.secretservicecompat"),
                OsStr::new("0"),
            ],
        )?;
        wait_for_secret_service(Duration::from_secs(5))?;
    }

    verify_default_collection()
}

fn wait_for_secret_service(timeout: Duration) -> Result<(), Box<dyn std::error::Error>> {
    let started = Instant::now();
    while started.elapsed() < timeout {
        if Command::new("timeout")
            .args([
                "1s",
                "busctl",
                "--user",
                "--quiet",
                "status",
                "org.freedesktop.secrets",
            ])
            .status()
            .is_ok_and(|status| status.success())
        {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(200));
    }
    Err("secure storage did not become ready; compositor was not started".into())
}

fn verify_default_collection() -> Result<(), Box<dyn std::error::Error>> {
    let output = output_timed(
        "busctl",
        [
            "--user",
            "call",
            "org.freedesktop.secrets",
            "/org/freedesktop/secrets",
            "org.freedesktop.Secret.Service",
            "ReadAlias",
            "s",
            "default",
        ],
    )?;
    let response = String::from_utf8_lossy(&output.stdout);
    let collection = default_collection_path(&response)?;
    let locked = output_timed(
        "busctl",
        [
            "--user",
            "get-property",
            "org.freedesktop.secrets",
            collection,
            "org.freedesktop.Secret.Collection",
            "Locked",
        ],
    )?;
    if String::from_utf8_lossy(&locked.stdout)
        .split_whitespace()
        .last()
        != Some("false")
    {
        return Err(
            "the existing Secret Service collection is locked; compositor was not started".into(),
        );
    }
    Ok(())
}

fn run_timed<I, S>(program: &str, arguments: I) -> Result<(), Box<dyn std::error::Error>>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let status = Command::new("timeout")
        .arg("3s")
        .arg(program)
        .args(arguments)
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{program} failed or timed out with {status}").into())
    }
}

fn output_timed<I, S>(program: &str, arguments: I) -> Result<Output, Box<dyn std::error::Error>>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = Command::new("timeout")
        .arg("3s")
        .arg(program)
        .args(arguments)
        .output()?;
    if output.status.success() {
        Ok(output)
    } else {
        Err(format!(
            "{program} failed or timed out: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )
        .into())
    }
}

fn default_collection_path(response: &str) -> Result<&str, &'static str> {
    let path = response
        .split_whitespace()
        .last()
        .map(|path| path.trim_matches('"'))
        .ok_or("Secret Service returned no default collection identity")?;
    if path == "/" {
        Err("Secret Service has no default collection; refusing to create a replacement")
    } else {
        Ok(path)
    }
}

fn sibling_binary(directory: &Path, name: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let path = directory.join(name);
    if path.is_file() {
        Ok(path)
    } else {
        Err(format!("required Nickel executable is missing: {}", path.display()).into())
    }
}

#[cfg(test)]
mod tests {
    use super::{default_collection_path, sibling_binary};

    #[test]
    fn rejects_missing_sibling() {
        let directory =
            std::env::temp_dir().join(format!("nickel-login-test-missing-{}", std::process::id()));
        assert!(sibling_binary(&directory, "nickel-ui").is_err());
    }

    #[test]
    fn accepts_existing_default_collection() {
        assert_eq!(
            default_collection_path("o \"/org/freedesktop/secrets/collection/kdewallet\"\n")
                .unwrap(),
            "/org/freedesktop/secrets/collection/kdewallet"
        );
    }

    #[test]
    fn rejects_missing_default_collection() {
        assert!(default_collection_path("o \"/\"\n").is_err());
    }
}
