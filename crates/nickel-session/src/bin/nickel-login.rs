use std::{
    env,
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::Command,
};

const CURRENT_DESKTOP: &str = "Nickel:KDE";
const KDE_SESSION_VERSION: &str = "6";
const XDG_HOME_DEFAULTS: [(&str, &str); 4] = [
    ("XDG_CONFIG_HOME", ".config"),
    ("XDG_DATA_HOME", ".local/share"),
    ("XDG_STATE_HOME", ".local/state"),
    ("XDG_CACHE_HOME", ".cache"),
];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let executable = env::current_exe()?;
    let directory = executable
        .parent()
        .ok_or("Nickel login launcher has no executable directory")?;
    let session = sibling_binary(directory, "nickel-session")?;
    let shell = sibling_binary(directory, "nickel-ui")?;

    prepare_login_environment()?;

    let error = Command::new(session)
        .arg("--backend")
        .arg("udev")
        .arg("--command")
        .arg(shell)
        .exec();
    Err(error.into())
}

fn prepare_login_environment() -> Result<(), Box<dyn std::error::Error>> {
    let home = env::var_os("HOME").map(PathBuf::from);
    // SAFETY: nickel-login is single-threaded and has not launched a child.
    unsafe {
        env::set_var("XDG_SESSION_TYPE", "wayland");
        env::set_var("XDG_CURRENT_DESKTOP", CURRENT_DESKTOP);
        env::set_var("XDG_SESSION_DESKTOP", "Nickel");
        env::set_var("KDE_FULL_SESSION", "true");
        env::set_var("KDE_SESSION_VERSION", KDE_SESSION_VERSION);
        for (variable, relative) in XDG_HOME_DEFAULTS {
            if env::var_os(variable).is_none() {
                let directory = home
                    .as_deref()
                    .ok_or_else(|| format!("{variable} and HOME are not set"))?
                    .join(relative);
                env::set_var(variable, directory);
            }
        }
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
    Ok(())
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
    use super::{CURRENT_DESKTOP, KDE_SESSION_VERSION, XDG_HOME_DEFAULTS, sibling_binary};

    #[test]
    fn advertises_kde_compatibility() {
        assert_eq!(
            CURRENT_DESKTOP.split(':').collect::<Vec<_>>(),
            ["Nickel", "KDE"]
        );
        assert_eq!(KDE_SESSION_VERSION, "6");
    }

    #[test]
    fn provides_standard_xdg_home_defaults() {
        assert_eq!(
            XDG_HOME_DEFAULTS,
            [
                ("XDG_CONFIG_HOME", ".config"),
                ("XDG_DATA_HOME", ".local/share"),
                ("XDG_STATE_HOME", ".local/state"),
                ("XDG_CACHE_HOME", ".cache"),
            ]
        );
    }

    #[test]
    fn rejects_missing_sibling() {
        let directory =
            std::env::temp_dir().join(format!("nickel-login-test-missing-{}", std::process::id()));
        assert!(sibling_binary(&directory, "nickel-ui").is_err());
    }
}
