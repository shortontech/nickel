use std::{collections::BTreeSet, fs, path::Path};

const HEADER: [&str; 20] = [
    "order",
    "surface",
    "crate",
    "architecture_state",
    "migration_status",
    "owner",
    "platform_scope",
    "semantic_roles",
    "semantic_actions",
    "paint_authority",
    "host_authority",
    "custom_paint_exception",
    "workbench_fixtures",
    "scenario_evidence",
    "visual_variants",
    "accessibility_evidence",
    "controller_evidence",
    "live_acceptance",
    "resource_evidence",
    "governing_specs",
];

const REQUIRED_SURFACES: [&str; 21] = [
    "ui-examples",
    "markdown-core",
    "markdown-viewer",
    "file-manager",
    "gaze-grid",
    "settings",
    "shell-runtime",
    "desktop",
    "panel",
    "notification",
    "lock",
    "screenshot",
    "window-preview",
    "window-context-menu",
    "control-center",
    "codex-project-menu",
    "launcher-dashboard",
    "launcher-search",
    "hosted-codex-chat",
    "codex-chat",
    "workbench-custom-paint",
];

const FIXTURES: [&str; 23] = [
    "codex.chat",
    "core.counter",
    "file.representative",
    "gaze.grid",
    "launcher.dashboard",
    "markdown.core",
    "markdown.viewer",
    "settings.narrow-rtl",
    "settings.wide",
    "shell.codex-project-menu",
    "shell.control-center",
    "shell.desktop",
    "shell.launcher-search",
    "shell.lock",
    "shell.notification",
    "shell.panel",
    "shell.runtime",
    "shell.screenshot",
    "shell.window-preview",
    "shared.collection-states",
    "shared.custom-paint",
    "shared.menus",
    "shared.primitives",
];

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workbench is a workspace crate")
}

#[test]
fn consumer_inventory_is_explicit_evidence_bearing_and_honest() {
    let source = fs::read_to_string(root().join("assets/ui-consumers.tsv"))
        .expect("consumer inventory is readable");
    let mut lines = source.lines();
    assert_eq!(
        lines.next().unwrap().split('\t').collect::<Vec<_>>(),
        HEADER,
        "Spec 0132 evidence schema drifted"
    );

    let fixtures = FIXTURES.into_iter().collect::<BTreeSet<_>>();
    let admitted_exceptions = [
        "assets/ui-authority-exceptions.tsv",
        "assets/ui-hit-authority-exceptions.tsv",
    ]
    .into_iter()
    .flat_map(|path| {
        fs::read_to_string(root().join(path))
            .unwrap()
            .lines()
            .filter(|line| !line.starts_with('#') && !line.trim().is_empty())
            .map(|line| line.split('\t').next().unwrap().to_owned())
            .collect::<Vec<_>>()
    })
    .collect::<BTreeSet<_>>();
    let mut surfaces = BTreeSet::new();
    let mut named_exceptions = BTreeSet::new();
    for (index, line) in lines.enumerate() {
        let row = line.split('\t').collect::<Vec<_>>();
        assert_eq!(row.len(), HEADER.len(), "row {} has wrong width", index + 2);
        assert!(row.iter().all(|value| !value.trim().is_empty()));
        assert_eq!(row[0].parse::<usize>().unwrap(), index + 1);
        assert!(surfaces.insert(row[1]), "duplicate surface {}", row[1]);
        assert!(
            root().join("crates").join(row[2]).is_dir(),
            "{} names missing crate {}",
            row[1],
            row[2]
        );
        assert!(!row.iter().any(|value| {
            let value = value.to_ascii_lowercase();
            value.contains("unknown") || value.contains("transitional")
        }));
        assert!(matches!(
            row[3],
            "library_frame"
                | "standalone_UiHost"
                | "standalone_UiHost_HostAdapter"
                | "embedded_UiFrame"
                | "embedded_UiHost"
                | "shell_runtime"
        ));
        assert!(matches!(
            row[4],
            "architecture_verified_acceptance_pending"
                | "architecture_and_live_verified"
                | "headless_verified_live_not_applicable"
        ));
        if row[4] == "architecture_and_live_verified" {
            assert!(row[17].starts_with("verified_"));
            assert!(!row[17].contains("pending"));
        }
        assert!(
            row[19]
                .split(',')
                .all(|spec| spec.len() == 4 && spec.bytes().all(|byte| byte.is_ascii_digit()))
        );
        assert!(matches!(row[9], "frame" | "mixed"));
        if row[11] == "none" {
            assert_eq!(row[9], "frame");
        } else {
            assert_eq!(row[9], "mixed");
            for exception in row[11].split(',') {
                assert!(
                    admitted_exceptions.contains(exception),
                    "{} names unadmitted exception {}",
                    row[1],
                    exception
                );
                assert!(
                    named_exceptions.insert(exception.to_owned()),
                    "exception authority must have one owning consumer"
                );
            }
        }
        assert!(matches!(
            row[6],
            "cross_platform" | "linux_primary_windows_macos_contract"
        ));

        for fixture in row[12].split(',') {
            assert!(
                fixture == "missing_required" || fixtures.contains(fixture),
                "{} names nonexistent fixture {fixture}",
                row[1]
            );
        }
        if row[4] == "headless_verified_live_not_applicable" {
            assert_ne!(row[12], "missing_required");
            assert_eq!(row[17], "not_applicable_library");
            assert!(!row[18].contains("pending"));
        } else {
            assert!(
                row[12].contains("missing_required")
                    || row[15].contains("partial")
                    || row[15].contains("pending")
                    || row[16].contains("partial")
                    || row[16].contains("pending")
                    || row[17].contains("pending")
                    || row[18].contains("partial")
                    || row[18].contains("pending"),
                "{} overclaims completed acceptance",
                row[1]
            );
        }

        for evidence in row[13].split(';') {
            let path = evidence.split("::").next().unwrap();
            assert!(root().join(path).exists(), "missing evidence path {path}");
        }
        assert!(
            row[19]
                .split(',')
                .all(|spec| { spec.len() == 4 && spec.bytes().all(|byte| byte.is_ascii_digit()) })
        );
    }

    assert_eq!(
        surfaces,
        REQUIRED_SURFACES.into_iter().collect(),
        "required consumer surfaces must be individually accountable"
    );
    assert_eq!(
        named_exceptions, admitted_exceptions,
        "every authority exception needs exactly one owning consumer"
    );
}
