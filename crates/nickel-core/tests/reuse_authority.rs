use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("nickel-core is nested under the workspace crates directory")
        .to_path_buf()
}

fn rust_sources(root: &Path, output: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_sources(&path, output);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            output.push(path);
        }
    }
}

#[test]
fn reuse_audit_source_inventory_covers_every_workspace_rust_source() {
    let root = workspace_root();
    let inventory = fs::read_to_string(root.join("assets/code-reuse-source-inventory.tsv"))
        .expect("code-reuse source inventory must be readable");
    let rows = inventory
        .lines()
        .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
        .map(|line| {
            let (name, count) = line
                .split_once('\t')
                .unwrap_or_else(|| panic!("invalid source inventory row: {line}"));
            (
                name.to_owned(),
                count
                    .parse::<usize>()
                    .unwrap_or_else(|_| panic!("invalid source count in row: {line}")),
            )
        })
        .collect::<Vec<_>>();
    let expected = rows.iter().cloned().collect::<BTreeMap<_, _>>();
    assert_eq!(
        expected.len(),
        rows.len(),
        "code-reuse source inventory crate names must be unique"
    );
    let crates = root.join("crates");
    let mut observed = BTreeMap::new();
    for entry in fs::read_dir(&crates).expect("workspace crates directory must be readable") {
        let path = entry.expect("crate entry must be readable").path();
        if !path.is_dir() {
            continue;
        }
        let mut sources = Vec::new();
        rust_sources(&path, &mut sources);
        if !sources.is_empty() {
            observed.insert(
                path.file_name().unwrap().to_string_lossy().into_owned(),
                sources.len(),
            );
        }
    }
    assert_eq!(
        observed, expected,
        "Rust sources changed; audit the new or removed production behavior and refresh the checked-in inventory"
    );
}

#[test]
fn logical_rectangle_and_intersection_have_one_shared_authority() {
    let root = workspace_root();
    let authority = fs::read_to_string(root.join("crates/nickel-core/src/geometry.rs")).unwrap();
    assert_eq!(authority.matches("pub struct LogicalRect").count(), 1);
    assert_eq!(authority.matches("pub fn intersection_area").count(), 1);

    for consumer in [
        "crates/nickel-core/src/dpi.rs",
        "crates/nickel-session/src/shell_layout.rs",
        "crates/nickel-shell/src/platform/linux.rs",
    ] {
        let source = fs::read_to_string(root.join(consumer)).unwrap();
        assert!(
            !source.contains("fn intersection_area"),
            "{consumer} restored parallel logical-rectangle intersection arithmetic"
        );
    }
    let shell_layout =
        fs::read_to_string(root.join("crates/nickel-session/src/shell_layout.rs")).unwrap();
    assert!(shell_layout.contains("nickel_core::geometry::LogicalRect"));
}

#[test]
fn nickel_core_has_one_configuration_path_and_atomic_write_authority() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let authority = source.join("persistence.rs");
    let mut files = Vec::new();
    rust_sources(&source, &mut files);
    let forbidden = [
        "XDG_CONFIG_HOME",
        "LOCALAPPDATA",
        "Library/Application Support/Nickel",
        "tmp-{}",
    ];
    let mut violations = Vec::new();
    for file in files {
        if file == authority {
            continue;
        }
        let text = fs::read_to_string(&file).unwrap();
        for token in forbidden {
            if text.contains(token) {
                violations.push(format!("{} contains {token:?}", file.display()));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "parallel persistence authority returned:\n{}",
        violations.join("\n")
    );

    for (module, expected_calls) in [
        ("launcher_preferences.rs", 1),
        ("wallpaper_settings.rs", 1),
        ("shell_settings.rs", 1),
        ("optional_features.rs", 2),
        ("dpi.rs", 2),
    ] {
        let text = fs::read_to_string(source.join(module)).unwrap();
        let production = text.split("#[cfg(test)]").next().unwrap();
        assert_eq!(
            production.matches("atomic_write(").count(),
            expected_calls,
            "{module} must route each complete settings write through the shared authority"
        );
        assert!(
            !production.contains("fs::write("),
            "{module} restored a non-atomic settings writer"
        );
    }
}
