use std::{fs, path::Path};

fn rust_sources(root: &Path, found: &mut Vec<std::path::PathBuf>) {
    for entry in fs::read_dir(root).expect("source directory should be readable") {
        let path = entry.expect("source entry should be readable").path();
        if path.is_dir() {
            if path.file_name().is_some_and(|name| name == "target") {
                continue;
            }
            rust_sources(&path, found);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            found.push(path);
        }
    }
}

#[test]
fn production_components_do_not_restore_deprecated_focus_borders() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("nickel-ui should live below the workspace root");
    let mut sources = Vec::new();
    rust_sources(&workspace.join("crates"), &mut sources);

    let forbidden = [
        "focus_border",
        "controller_pane_border",
        "navigation_scope_highlight",
        ".border(theme.borders.focus",
        ".border(theme.borders.controller_focus",
    ];
    let mut violations = Vec::new();
    for path in sources {
        if path.ends_with("focus_background_policy.rs") {
            continue;
        }
        let source = fs::read_to_string(&path).expect("Rust source should be UTF-8");
        let compact: String = source
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect();
        for pattern in forbidden {
            if compact.contains(pattern) {
                violations.push(format!("{} contains `{pattern}`", path.display()));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "focus must use child backgrounds, not decorative outer borders:\n{}",
        violations.join("\n")
    );
}
