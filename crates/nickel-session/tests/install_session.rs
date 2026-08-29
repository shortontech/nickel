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
        assert_eq!(fs::read(&installed).expect("installed binary"), b"fixture");
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
}
