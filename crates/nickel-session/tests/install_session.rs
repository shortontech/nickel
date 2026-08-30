use std::{fs, os::unix::fs::PermissionsExt, path::Path, process::Command};

fn executable(path: &Path) {
    fs::write(path, b"fixture").expect("write fixture executable");
    let mut permissions = fs::metadata(path).expect("fixture metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("mark fixture executable");
}

#[test]
fn installer_stages_self_contained_sddm_session_from_any_working_directory() {
    let fixture = tempfile::tempdir().expect("fixture directory");
    let release = fixture.path().join("release");
    let root = fixture.path().join("root");
    fs::create_dir(&release).expect("release directory");
    for binary in [
        "nickel-login",
        "nickel-session",
        "nickel",
        "nickel-settings",
    ] {
        executable(&release.join(binary));
    }
    fs::write(
        release.join("nickel-session"),
        b"#!/bin/sh\n[ \"$1\" = --available-backends ] && echo udev\n",
    )
    .expect("write fixture session executable");

    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root");
    let installer = repository.join("packaging/install-nickel-session.sh");
    let status = Command::new(&installer)
        .current_dir(fixture.path())
        .env("NICKEL_RELEASE_DIR", &release)
        .env("NICKEL_INSTALL_ROOT", &root)
        .status()
        .expect("run installer");
    assert!(status.success());

    for binary in [
        "nickel-login",
        "nickel-session",
        "nickel",
        "nickel-settings",
    ] {
        let installed = root.join("usr/local/bin").join(binary);
        let expected: &[u8] = if binary == "nickel-session" {
            b"#!/bin/sh\n[ \"$1\" = --available-backends ] && echo udev\n"
        } else {
            b"fixture"
        };
        assert_eq!(fs::read(&installed).expect("installed binary"), expected);
        assert_ne!(
            fs::metadata(installed)
                .expect("installed metadata")
                .permissions()
                .mode()
                & 0o111,
            0
        );
    }
    let desktop = fs::read_to_string(root.join("usr/share/wayland-sessions/nickel.desktop"))
        .expect("installed session entry");
    assert!(desktop.contains("Exec=/usr/local/bin/nickel-login"));
    assert!(desktop.contains("TryExec=/usr/local/bin/nickel-login"));
    assert!(desktop.contains("DesktopNames=Nickel"));
    let portals = fs::read_to_string(root.join("usr/share/xdg-desktop-portal/nickel-portals.conf"))
        .expect("installed portal preference");
    assert!(portals.contains("default=gtk"));
    assert!(portals.contains("org.freedesktop.impl.portal.Secret=kwallet"));
    assert!(portals.contains("org.freedesktop.impl.portal.ScreenCast=wlr"));
    assert!(portals.contains("org.freedesktop.impl.portal.Screenshot=wlr"));
}

#[test]
fn installer_rejects_session_without_native_backend() {
    let fixture = tempfile::tempdir().expect("fixture directory");
    let release = fixture.path().join("release");
    let root = fixture.path().join("root");
    fs::create_dir(&release).expect("release directory");
    for binary in [
        "nickel-login",
        "nickel-session",
        "nickel",
        "nickel-settings",
    ] {
        executable(&release.join(binary));
    }
    fs::write(release.join("nickel-session"), b"#!/bin/sh\nexit 0\n")
        .expect("write fixture session executable");

    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root");
    let status = Command::new(repository.join("packaging/install-nickel-session.sh"))
        .env("NICKEL_RELEASE_DIR", &release)
        .env("NICKEL_INSTALL_ROOT", &root)
        .status()
        .expect("run installer");

    assert!(!status.success());
    assert!(!root.join("usr/local/bin/nickel-session").exists());
}
