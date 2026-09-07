#[cfg(target_os = "linux")]
use std::{
    env, fs,
    os::unix::process::CommandExt,
    path::{Path as LinuxPath, PathBuf},
    process::Command,
};

#[cfg(any(target_os = "linux", test))]
use std::path::Path;

#[cfg(any(target_os = "linux", test))]
const CURRENT_DESKTOP: &str = "Nickel:KDE";
#[cfg(any(target_os = "linux", test))]
const KDE_SESSION_VERSION: &str = "6";
#[cfg(target_os = "linux")]
const EGL_VENDOR_FILENAMES: &str = "__EGL_VENDOR_LIBRARY_FILENAMES";
#[cfg(target_os = "linux")]
const NVIDIA_EGL_VENDOR_MANIFEST: &str = "/usr/share/glvnd/egl_vendor.d/10_nvidia.json";
#[cfg(any(target_os = "linux", test))]
const XDG_HOME_DEFAULTS: [(&str, &str); 4] = [
    ("XDG_CONFIG_HOME", ".config"),
    ("XDG_DATA_HOME", ".local/share"),
    ("XDG_STATE_HOME", ".local/state"),
    ("XDG_CACHE_HOME", ".cache"),
];

#[cfg(target_os = "linux")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let executable = env::current_exe()?;
    let directory = executable
        .parent()
        .ok_or("Nickel login launcher has no executable directory")?;
    let nickel = sibling_binary(directory, "nickel")?;

    prepare_login_environment()?;

    let error = Command::new(nickel).arg("--backend").arg("udev").exec();
    Err(error.into())
}

#[cfg(not(target_os = "linux"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    Err("nickel-login is only available on Linux".into())
}

#[cfg(target_os = "linux")]
fn prepare_login_environment() -> Result<(), Box<dyn std::error::Error>> {
    let home = env::var_os("HOME").map(PathBuf::from);
    // SAFETY: nickel-login is single-threaded and has not launched a child.
    unsafe {
        env::set_var("XDG_SESSION_TYPE", "wayland");
        env::set_var("XDG_CURRENT_DESKTOP", CURRENT_DESKTOP);
        env::set_var("XDG_SESSION_DESKTOP", "Nickel");
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
        configure_nvidia_egl_vendor(
            LinuxPath::new("/sys/class/drm"),
            LinuxPath::new(NVIDIA_EGL_VENDOR_MANIFEST),
        );
    }

    Ok(())
}

#[cfg(target_os = "linux")]
fn configure_nvidia_egl_vendor(sysfs_drm: &LinuxPath, manifest: &LinuxPath) {
    if env::var_os(EGL_VENDOR_FILENAMES).is_some()
        || !manifest.is_file()
        || !nvidia_only_render_host(sysfs_drm)
    {
        return;
    }
    // SAFETY: nickel-login is single-threaded and has not launched the compositor child.
    unsafe { env::set_var(EGL_VENDOR_FILENAMES, manifest) };
}

#[cfg(target_os = "linux")]
fn nvidia_only_render_host(sysfs_drm: &LinuxPath) -> bool {
    let Ok(entries) = fs::read_dir(sysfs_drm) else {
        return false;
    };
    let mut found_nvidia = false;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with("card") || name.contains('-') {
            continue;
        }
        let Some(driver) = fs::read_link(entry.path().join("device/driver"))
            .ok()
            .and_then(|path| path.file_name().map(|name| name.to_owned()))
        else {
            return false;
        };
        if driver == "evdi" {
            continue;
        }
        if driver != "nvidia" {
            return false;
        }
        found_nvidia = true;
    }
    found_nvidia
}

#[cfg(any(target_os = "linux", test))]
fn sibling_binary(
    directory: &Path,
    name: &str,
) -> Result<std::path::PathBuf, Box<dyn std::error::Error>> {
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
    fn advertises_nickel_with_kde6_compatibility() {
        assert_eq!(CURRENT_DESKTOP, "Nickel:KDE");
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
        assert!(sibling_binary(&directory, "nickel").is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn egl_vendor_can_be_restricted_only_for_nvidia_plus_evdi() {
        use std::os::unix::fs::symlink;

        fn card(root: &std::path::Path, name: &str, driver: &str) {
            let device = root.join(name).join("device");
            std::fs::create_dir_all(&device).unwrap();
            symlink(format!("/drivers/{driver}"), device.join("driver")).unwrap();
        }

        let fixture = tempfile::tempdir().unwrap();
        card(fixture.path(), "card0", "evdi");
        card(fixture.path(), "card1", "nvidia");
        std::fs::create_dir_all(fixture.path().join("card1-DP-3")).unwrap();
        assert!(super::nvidia_only_render_host(fixture.path()));

        card(fixture.path(), "card2", "amdgpu");
        assert!(!super::nvidia_only_render_host(fixture.path()));
    }
}
