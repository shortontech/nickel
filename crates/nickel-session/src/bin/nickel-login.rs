use std::{
    env,
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::Command,
};

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
    use super::sibling_binary;

    #[test]
    fn rejects_missing_sibling() {
        let directory =
            std::env::temp_dir().join(format!("nickel-login-test-missing-{}", std::process::id()));
        assert!(sibling_binary(&directory, "nickel-ui").is_err());
    }
}
