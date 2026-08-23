use std::{fs, io::Read, path::Path};

use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::CodexError;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BundleManifest {
    pub profile: u32,
    pub upstream_version: String,
    pub upstream_repository: String,
    pub upstream_revision: String,
    pub upstream_license: String,
    pub schema_version: String,
    pub artifact: Vec<BundleArtifact>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BundleArtifact {
    pub target: String,
    pub filename: String,
    pub archive: String,
    pub archive_sha256: String,
    pub archive_member: String,
    pub sha256: String,
}

impl BundleManifest {
    pub fn read(path: &Path) -> Result<Self, CodexError> {
        let manifest: Self = toml::from_str(&fs::read_to_string(path)?)
            .map_err(|error| CodexError::Protocol(format!("invalid bundle manifest: {error}")))?;
        if manifest.artifact.is_empty()
            || manifest
                .artifact
                .iter()
                .any(|artifact| artifact.sha256.len() != 64 || artifact.archive_sha256.len() != 64)
        {
            return Err(CodexError::Protocol(
                "bundle manifest contains an unpinned artifact".into(),
            ));
        }
        Ok(manifest)
    }
}

pub fn stage_bundle(
    manifest_path: &Path,
    archives: &Path,
    output: &Path,
    target: &str,
    license: &Path,
) -> Result<(), CodexError> {
    let manifest = BundleManifest::read(manifest_path)?;
    let artifact = manifest
        .artifact
        .iter()
        .find(|artifact| artifact.target == target)
        .ok_or_else(|| {
            CodexError::Unavailable(format!("no bundled Codex artifact for {target}"))
        })?;
    let archive_path = archives.join(&artifact.archive);
    verify_digest(
        &fs::read(&archive_path)?,
        &artifact.archive_sha256,
        "archive",
    )?;
    let archive_file = fs::File::open(archive_path)?;
    let mut archive = tar::Archive::new(GzDecoder::new(archive_file));
    let mut binary = None;
    for entry in archive.entries()? {
        let mut entry = entry?;
        if entry.path()?.as_ref() == Path::new(&artifact.archive_member) {
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes)?;
            binary = Some(bytes);
            break;
        }
    }
    let binary =
        binary.ok_or_else(|| CodexError::Protocol("pinned binary member is absent".into()))?;
    verify_digest(&binary, &artifact.sha256, "binary")?;
    let runtime = output.join("runtime/codex");
    fs::create_dir_all(&runtime)?;
    let destination = runtime.join(&artifact.filename);
    fs::write(&destination, binary)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&destination, fs::Permissions::from_mode(0o755))?;
    }
    fs::copy(license, runtime.join("LICENSE-APACHE"))?;
    fs::write(runtime.join("manifest.toml"), fs::read(manifest_path)?)?;
    Ok(())
}

fn verify_digest(bytes: &[u8], expected: &str, kind: &str) -> Result<(), CodexError> {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let actual = format!("{:x}", hasher.finalize());
    if actual != expected {
        return Err(CodexError::Incompatible(format!(
            "{kind} digest mismatch: expected {expected}, found {actual}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_manifest_pins_all_declared_targets() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let manifest = BundleManifest::read(&root.join("packaging/codex/manifest.toml")).unwrap();
        for target in [
            "x86_64-unknown-linux-musl",
            "aarch64-unknown-linux-musl",
            "x86_64-pc-windows-msvc",
            "aarch64-pc-windows-msvc",
            "x86_64-apple-darwin",
            "aarch64-apple-darwin",
        ] {
            assert!(
                manifest
                    .artifact
                    .iter()
                    .any(|artifact| artifact.target == target)
            );
        }
        assert!(root.join("LICENSE-APACHE").is_file());
        assert!(!manifest.upstream_revision.is_empty());
    }
}
