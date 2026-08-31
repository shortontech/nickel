use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug)]
struct Exception {
    maximum: usize,
    category: String,
    owner: String,
    reason: String,
    review: String,
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("nickel-ui is nested under the workspace crates directory")
        .to_path_buf()
}

fn exceptions(root: &Path) -> BTreeMap<String, Exception> {
    let source = fs::read_to_string(root.join("assets/ui-authority-exceptions.tsv"))
        .expect("UI authority exception inventory must be readable");
    source
        .lines()
        .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
        .map(|line| {
            let fields = line.split('\t').collect::<Vec<_>>();
            assert_eq!(
                fields.len(),
                6,
                "UI authority exception must have six tab-separated fields: {line}"
            );
            let maximum = fields[1]
                .parse()
                .expect("exception reference bound must be an integer");
            assert!(maximum > 0, "exception bound must be positive: {line}");
            assert!(
                !fields[2].is_empty(),
                "exception category is required: {line}"
            );
            assert!(
                !fields[3].is_empty(),
                "exception semantic owner is required: {line}"
            );
            assert!(
                !fields[4].is_empty(),
                "exception reason is required: {line}"
            );
            assert!(
                !fields[5].is_empty(),
                "exception review is required: {line}"
            );
            (
                fields[0].to_owned(),
                Exception {
                    maximum,
                    category: fields[2].to_owned(),
                    owner: fields[3].to_owned(),
                    reason: fields[4].to_owned(),
                    review: fields[5].to_owned(),
                },
            )
        })
        .collect()
}

fn rust_files(directory: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(directory).expect("source directory must be readable") {
        let path = entry.expect("source entry must be readable").path();
        if path.is_dir() {
            rust_files(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
}

fn display_list_references(source: &str) -> usize {
    code_references(source, "PaintCommand::")
}

fn hit_authority_references(source: &str) -> usize {
    let launcher_targets = code_references(source, "LauncherHitTarget");
    let generic_targets = code_references(source, "HitTarget") - launcher_targets;
    launcher_targets
        + generic_targets
        + ["fn target_point", "hits: Vec"]
            .into_iter()
            .map(|needle| code_references(source, needle))
            .sum::<usize>()
}

/// Removes comments and literal contents before counting authority-bearing
/// identifiers. Diagnostic text must not consume a migration budget.
fn code_references(source: &str, needle: &str) -> usize {
    source
        .lines()
        .filter(|line| {
            let trimmed = line.trim_start();
            !trimmed.starts_with("//") && !trimmed.starts_with("/*") && !trimmed.starts_with('*')
        })
        .map(|line| {
            let comment = line.find("//").unwrap_or(line.len());
            line[..comment]
                .match_indices(needle)
                .filter(|(position, _)| {
                    let prefix = &line[..*position];
                    prefix
                        .bytes()
                        .fold((0usize, false), |(quotes, escaped), byte| {
                            if escaped {
                                (quotes, false)
                            } else if byte == b'\\' {
                                (quotes, true)
                            } else if byte == b'"' {
                                (quotes + 1, false)
                            } else {
                                (quotes, false)
                            }
                        })
                        .0
                        % 2
                        == 0
                })
                .count()
        })
        .sum()
}

#[test]
fn consumers_cannot_grow_or_create_display_list_authority() {
    let root = workspace_root();
    let exceptions = exceptions(&root);
    let mut files = Vec::new();
    rust_files(&root.join("crates"), &mut files);
    let mut observed = BTreeSet::new();
    let mut violations = Vec::new();

    for path in files {
        let relative = path
            .strip_prefix(&root)
            .expect("workspace source is below its root")
            .to_string_lossy()
            .replace('\\', "/");
        if relative.starts_with("crates/nickel-ui/") {
            continue;
        }
        let source = fs::read_to_string(&path).expect("Rust source must be UTF-8");
        let references = display_list_references(&source);
        if references == 0 {
            continue;
        }
        observed.insert(relative.clone());
        match exceptions.get(&relative) {
            Some(exception) if references <= exception.maximum => {}
            Some(exception) => violations.push(format!(
                "{relative}: {references} PaintCommand references exceed admitted maximum {} ({}, owner {}, {}; review {})",
                exception.maximum, exception.category, exception.owner, exception.reason, exception.review
            )),
            None => violations.push(format!(
                "{relative}: {references} unadmitted PaintCommand references; describe UI declaratively or add a reviewed bounded custom-paint exception"
            )),
        }
    }

    let stale = exceptions
        .keys()
        .filter(|path| !observed.contains(*path))
        .cloned()
        .collect::<Vec<_>>();
    assert!(
        stale.is_empty(),
        "remove resolved UI authority exceptions from the inventory:\n{}",
        stale.join("\n")
    );
    assert!(
        violations.is_empty(),
        "application-owned display-list authority grew:\n{}",
        violations.join("\n")
    );
}

#[test]
fn consumers_cannot_grow_or_create_parallel_hit_authority() {
    let root = workspace_root();
    let source = fs::read_to_string(root.join("assets/ui-hit-authority-exceptions.tsv"))
        .expect("hit authority exception inventory must be readable");
    let admitted = source
        .lines()
        .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
        .map(|line| {
            let fields = line.split('\t').collect::<Vec<_>>();
            assert_eq!(
                fields.len(),
                5,
                "hit exception must have five fields: {line}"
            );
            assert!(fields[2..].iter().all(|field| !field.is_empty()));
            (
                fields[0].to_owned(),
                fields[1].parse::<usize>().expect("hit bound is an integer"),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut files = Vec::new();
    rust_files(&root.join("crates"), &mut files);
    let mut observed = BTreeSet::new();
    let mut violations = Vec::new();
    for path in files {
        let relative = path
            .strip_prefix(&root)
            .expect("workspace source is below root")
            .to_string_lossy()
            .replace('\\', "/");
        if relative.starts_with("crates/nickel-ui/") {
            continue;
        }
        let count =
            hit_authority_references(&fs::read_to_string(path).expect("Rust source must be UTF-8"));
        if count == 0 {
            continue;
        }
        observed.insert(relative.clone());
        match admitted.get(&relative) {
            Some(maximum) if count <= *maximum => {}
            Some(maximum) => violations.push(format!(
                "{relative}: {count} parallel hit-authority references exceed {maximum}"
            )),
            None => violations.push(format!(
                "{relative}: {count} unadmitted parallel hit-authority references"
            )),
        }
    }
    let stale = admitted
        .keys()
        .filter(|path| !observed.contains(*path))
        .cloned()
        .collect::<Vec<_>>();
    assert!(
        stale.is_empty(),
        "remove resolved hit exceptions:\n{}",
        stale.join("\n")
    );
    assert!(
        violations.is_empty(),
        "parallel hit authority grew:\n{}",
        violations.join("\n")
    );
}

#[test]
fn display_list_reference_counter_detects_seeded_regressions() {
    assert_eq!(
        display_list_references(
            "let _ = PaintCommand::Fill { rect, color };\ncommands.push(PaintCommand::Text { bounds, text });"
        ),
        2
    );
    assert_eq!(
        display_list_references("let ordinary_component = Button::new();"),
        0
    );
    assert_eq!(
        hit_authority_references("struct HitTarget; fn action_at(&self) {} hits: Vec<()>"),
        2
    );
    assert_eq!(
        hit_authority_references(
            r#"// LauncherHitTarget
               let diagnostic = "HitTarget and fn action_at";
               struct RealHitTarget;"#
        ),
        1
    );
}
