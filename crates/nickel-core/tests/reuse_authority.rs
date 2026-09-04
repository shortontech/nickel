use std::{
    fs,
    path::{Path, PathBuf},
};

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
