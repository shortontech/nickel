use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

#[test]
fn every_shipped_icon_has_exactly_one_provenance_record() {
    let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let manifest_text = fs::read_to_string(assets.join("visual-fixtures.toml"))
        .expect("assets/visual-fixtures.toml must be readable");
    let manifest: toml::Value = toml::from_str(&manifest_text).expect("valid asset manifest");
    let mut admissions = BTreeMap::<PathBuf, Vec<String>>::new();

    for family in manifest["icon_family"]
        .as_array()
        .expect("icon_family entries")
    {
        let id = required_string(family, "id");
        let directory = required_string(family, "directory");
        required_string(family, "authorship");
        required_string(family, "license");
        for member in family["members"].as_array().expect("family members") {
            let member = member.as_str().expect("member path");
            admissions
                .entry(Path::new(directory).join(member))
                .or_default()
                .push(id.to_owned());
        }
    }

    for asset in manifest["asset"].as_array().expect("asset entries") {
        let id = required_string(asset, "id");
        let path = required_string(asset, "path");
        required_string(asset, "authorship");
        required_string(asset, "license");
        admissions
            .entry(PathBuf::from(path))
            .or_default()
            .push(id.to_owned());
    }

    let mut shipped = Vec::new();
    collect_files(&assets.join("icons"), Path::new("icons"), &mut shipped);
    shipped.sort();
    for path in &shipped {
        let records = admissions.get(path);
        assert!(
            matches!(records, Some(records) if records.len() == 1),
            "{path:?} must have exactly one provenance record; found {:?}",
            records
        );
    }
    for path in admissions.keys() {
        assert!(
            shipped.contains(path),
            "manifest admits missing or non-icon asset {path:?}"
        );
    }
}

#[test]
fn every_visual_reference_matches_its_admitted_bytes_and_dimensions() {
    let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let manifest_text = fs::read_to_string(assets.join("visual-fixtures.toml")).unwrap();
    let manifest: toml::Value = toml::from_str(&manifest_text).unwrap();
    for reference in manifest["reference"].as_array().expect("reference entries") {
        required_string(reference, "id");
        required_string(reference, "authorship");
        required_string(reference, "usage_status");
        required_string(reference, "source");
        let path = assets.join(required_string(reference, "path"));
        let bytes = fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        let actual_digest = format!("{:x}", Sha256::digest(&bytes));
        assert_eq!(actual_digest, required_string(reference, "sha256"));
        let image = image::load_from_memory(&bytes).expect("reference must decode");
        assert_eq!(
            u64::from(image.width()),
            reference["width"].as_integer().unwrap() as u64
        );
        assert_eq!(
            u64::from(image.height()),
            reference["height"].as_integer().unwrap() as u64
        );
    }
}

fn required_string<'a>(entry: &'a toml::Value, key: &str) -> &'a str {
    let value = entry[key]
        .as_str()
        .unwrap_or_else(|| panic!("missing {key}"));
    assert!(!value.trim().is_empty(), "{key} must not be empty");
    value
}

fn collect_files(directory: &Path, relative: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(directory).expect("read icon directory") {
        let entry = entry.expect("read icon entry");
        let path = entry.path();
        let relative = relative.join(entry.file_name());
        if path.is_dir() {
            collect_files(&path, &relative, files);
        } else {
            files.push(relative);
        }
    }
}
