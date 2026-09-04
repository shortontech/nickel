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
fn production_sources_do_not_embed_extreme_background_fills() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("nickel-ui should live below the workspace root");
    let mut sources = Vec::new();
    rust_sources(&workspace.join("crates"), &mut sources);

    let forbidden = [
        [".background(0x", "000000"].concat(),
        [".background(0x", "ffffff"].concat(),
        ["background:0x", "000000"].concat(),
        ["background:0x", "ffffff"].concat(),
        ["[0.0,0.0,0.0", ","].concat(),
        ["[1.0,1.0,1.0", ","].concat(),
    ];
    let mut violations = Vec::new();
    for path in sources {
        let source = fs::read_to_string(&path).expect("Rust source should be UTF-8");
        let compact: String = source
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect();
        for pattern in &forbidden {
            if compact.contains(pattern) {
                violations.push(format!("{} embeds `{pattern}`", path.display()));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "pure black/white background literals require semantic surface tokens:\n{}",
        violations.join("\n")
    );
}
