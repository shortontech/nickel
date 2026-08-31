use std::{fs, path::Path};

use serde_json::Value;
use sha2::{Digest, Sha256};

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workbench is a workspace crate")
}

#[cfg(all(
    feature = "file-provider",
    feature = "gaze-provider",
    feature = "markdown-viewer-provider",
    feature = "shell-provider"
))]
#[test]
fn retained_reachability_report_matches_a_fresh_full_provider_generation() {
    let output = std::env::temp_dir().join(format!(
        "nickel-ui-reachability-drift-{}.json",
        std::process::id()
    ));
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_nickel-ui-workbench"))
        .args(["reachability-report", output.to_str().unwrap()])
        .status()
        .expect("fresh reachability generator runs");
    assert!(status.success());
    let fresh = fs::read(&output).expect("fresh reachability report is readable");
    fs::remove_file(output).expect("temporary reachability report is removed");
    let retained = fs::read(root().join("assets/evidence/ui-reachability-report.json"))
        .expect("retained reachability report is readable");
    assert_eq!(
        sha256(&fresh),
        sha256(&retained),
        "retained reachability evidence drifted"
    );
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[test]
fn durable_ui_evidence_is_complete_content_addressed_and_within_budget() {
    let manifest_path = root().join("assets/evidence/ui-evidence.json");
    let manifest: Value = serde_json::from_slice(
        &fs::read(&manifest_path).expect("durable UI evidence manifest is readable"),
    )
    .expect("durable UI evidence manifest is valid JSON");
    assert_eq!(manifest["schema"], 1);
    assert_eq!(manifest["repository_revision"].as_str().unwrap().len(), 40);

    let reachability = &manifest["reachability"];
    let report_path = root().join(reachability["path"].as_str().unwrap());
    let report_bytes = fs::read(report_path).expect("retained full path report is readable");
    assert_eq!(sha256(&report_bytes), reachability["sha256"]);
    let report: Value = serde_json::from_slice(&report_bytes).expect("path report is valid JSON");
    assert_eq!(report["schema"], reachability["report_schema"]);
    assert_eq!(report["variants"].as_array().unwrap().len(), 66);
    assert_eq!(report["path_count"], 1125);
    assert_eq!(report["reached_count"], report["path_count"]);
    assert_eq!(report["issue_count"], 0);
    assert!(
        reachability["generator_command"]
            .as_str()
            .unwrap()
            .contains("reachability-report")
    );

    let live = &manifest["workbench_live_acceptance"];
    assert_eq!(live["evidence_class"], "nested_native_workbench");
    assert!(
        live["limitations"]
            .as_str()
            .unwrap()
            .contains("not external assistive-technology")
    );
    let modalities = live["modalities"].as_object().unwrap();
    for modality in ["pointer", "keyboard", "controller", "accessibility"] {
        let evidence = &modalities[modality];
        assert_eq!(evidence["result"], "passed");
        assert_eq!(evidence["sha256"].as_str().unwrap().len(), 64);
        assert!(evidence["bytes"].as_u64().unwrap() > 0);
        assert!(!evidence["action"].as_str().unwrap().is_empty());
    }

    let performance = &manifest["performance_comparison"];
    assert!(
        performance["generator_command"]
            .as_str()
            .unwrap()
            .contains("--full-comparison")
    );
    let incremental = performance["clean_incremental_ms"].as_f64().unwrap();
    let old_matrix = performance["old_launcher_matrix_ms"].as_f64().unwrap();
    assert!(incremental <= performance["incremental_budget_ms"].as_f64().unwrap());
    assert!(old_matrix <= performance["full_matrix_budget_ms"].as_f64().unwrap());
    assert!(old_matrix / incremental >= performance["minimum_speedup"].as_f64().unwrap());
    assert!(
        (performance["measured_speedup"].as_f64().unwrap() - old_matrix / incremental).abs() < 0.1
    );

    for field in ["rustc", "cpu", "renderer", "profile"] {
        assert!(!manifest["environment"][field].as_str().unwrap().is_empty());
    }
}
