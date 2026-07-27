use std::{
    ffi::OsStr,
    process::{Command, Output},
    thread,
    time::{Duration, Instant},
};

pub fn prepare_secure_storage() -> Result<(), Box<dyn std::error::Error>> {
    run_timed(
        "dbus-update-activation-environment",
        [
            OsStr::new("--systemd"),
            OsStr::new("PAM_KWALLET5_LOGIN"),
            OsStr::new("WAYLAND_DISPLAY"),
            OsStr::new("KDE_FULL_SESSION"),
            OsStr::new("KDE_SESSION_VERSION"),
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
    Err("secure storage did not become ready; shell was not started".into())
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
    if collection_is_locked(collection)? {
        unlock_collection(collection)?;
        wait_for_collection_unlock(collection, Duration::from_secs(120))?;
    }
    Ok(())
}

fn collection_is_locked(collection: &str) -> Result<bool, Box<dyn std::error::Error>> {
    let output = output_timed(
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
    Ok(String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .last()
        != Some("false"))
}

fn unlock_collection(collection: &str) -> Result<(), Box<dyn std::error::Error>> {
    let output = output_timed(
        "busctl",
        [
            "--user",
            "call",
            "org.freedesktop.secrets",
            "/org/freedesktop/secrets",
            "org.freedesktop.Secret.Service",
            "Unlock",
            "ao",
            "1",
            collection,
        ],
    )?;
    let response = String::from_utf8_lossy(&output.stdout);
    let prompt = unlock_prompt_path(&response)?;
    if prompt == "/" {
        return Ok(());
    }
    run_timed(
        "busctl",
        [
            OsStr::new("--user"),
            OsStr::new("call"),
            OsStr::new("org.freedesktop.secrets"),
            OsStr::new(prompt),
            OsStr::new("org.freedesktop.Secret.Prompt"),
            OsStr::new("Prompt"),
            OsStr::new("s"),
            OsStr::new(""),
        ],
    )
}

fn wait_for_collection_unlock(
    collection: &str,
    timeout: Duration,
) -> Result<(), Box<dyn std::error::Error>> {
    let started = Instant::now();
    while started.elapsed() < timeout {
        if !collection_is_locked(collection)? {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(250));
    }
    Err("KDE Wallet remained locked; shell was not started".into())
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

fn unlock_prompt_path(response: &str) -> Result<&str, &'static str> {
    response
        .split_whitespace()
        .last()
        .map(|path| path.trim_matches('"'))
        .ok_or("Secret Service returned no unlock prompt identity")
}

#[cfg(test)]
mod tests {
    use super::{default_collection_path, unlock_prompt_path};

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

    #[test]
    fn extracts_unlock_prompt_path() {
        assert_eq!(
            unlock_prompt_path("aoo 0 \"/org/freedesktop/secrets/prompt/p0\"\n").unwrap(),
            "/org/freedesktop/secrets/prompt/p0"
        );
    }
}
