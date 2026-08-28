use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    env,
    fs::{self, File},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};
use thiserror::Error;

pub const MANIFEST_TEXT: &str = include_str!("../models/open-see-face-v1.toml");

#[derive(Clone, Debug, Deserialize)]
pub struct ModelManifest {
    pub name: String,
    pub version: String,
    pub source_revision: String,
    pub license: String,
    pub license_file: String,
    pub artifacts: Vec<ModelArtifact>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ModelArtifact {
    pub role: String,
    pub file: String,
    pub url: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug)]
pub struct ModelBundle {
    pub manifest: ModelManifest,
    pub directory: PathBuf,
    pub downloaded: Vec<String>,
}

#[derive(Debug, Error)]
pub enum CacheError {
    #[error("model manifest is invalid: {0}")]
    Manifest(#[from] toml::de::Error),
    #[error("no user home directory is available for the model cache")]
    MissingHome,
    #[error("model artifact {path} has {actual} bytes; expected {expected}")]
    WrongLength {
        path: PathBuf,
        expected: u64,
        actual: u64,
    },
    #[error("model artifact {path} has SHA-256 {actual}; expected {expected}")]
    WrongDigest {
        path: PathBuf,
        expected: String,
        actual: String,
    },
    #[error("failed to download {url}: {source}")]
    Download {
        url: String,
        #[source]
        source: Box<ureq::Error>,
    },
    #[error("model cache I/O failed for {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

pub fn parse_manifest() -> Result<ModelManifest, CacheError> {
    Ok(toml::from_str(MANIFEST_TEXT)?)
}

pub fn default_cache_root() -> Result<PathBuf, CacheError> {
    if let Some(path) = env::var_os("XDG_CACHE_HOME").filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(path).join("nickel/models"));
    }
    let home = env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .ok_or(CacheError::MissingHome)?;
    #[cfg(target_os = "macos")]
    return Ok(PathBuf::from(home).join("Library/Caches/nickel/models"));
    #[cfg(target_os = "windows")]
    return Ok(env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(home).join("AppData/Local"))
        .join("Nickel/models"));
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    Ok(PathBuf::from(home).join(".cache/nickel/models"))
}

pub fn acquire_bundle(explicit_directory: Option<&Path>) -> Result<ModelBundle, CacheError> {
    let manifest = parse_manifest()?;
    let directory = explicit_directory.map(Path::to_path_buf).unwrap_or(
        default_cache_root()?
            .join(safe_component(&manifest.name))
            .join(&manifest.version),
    );
    fs::create_dir_all(&directory).map_err(|source| io_error(&directory, source))?;
    let mut downloaded = Vec::new();

    for artifact in &manifest.artifacts {
        let path = directory.join(&artifact.file);
        if verify_artifact(&path, artifact).is_ok() {
            continue;
        }
        if explicit_directory.is_some() {
            verify_artifact(&path, artifact)?;
            continue;
        }
        eprintln!(
            "downloading {} {} ({}, {} bytes, {}) from {} to {}",
            manifest.name,
            manifest.version,
            artifact.role,
            artifact.bytes,
            manifest.license,
            artifact.url,
            path.display()
        );
        download_artifact(artifact, &path)?;
        downloaded.push(artifact.file.clone());
    }

    Ok(ModelBundle {
        manifest,
        directory,
        downloaded,
    })
}

pub fn verify_artifact(path: &Path, artifact: &ModelArtifact) -> Result<(), CacheError> {
    let metadata = fs::metadata(path).map_err(|source| io_error(path, source))?;
    if metadata.len() != artifact.bytes {
        return Err(CacheError::WrongLength {
            path: path.to_path_buf(),
            expected: artifact.bytes,
            actual: metadata.len(),
        });
    }
    let mut file = File::open(path).map_err(|source| io_error(path, source))?;
    let mut hasher = Sha256::new();
    io::copy(&mut file, &mut hasher).map_err(|source| io_error(path, source))?;
    let actual = format!("{:x}", hasher.finalize());
    if actual != artifact.sha256 {
        return Err(CacheError::WrongDigest {
            path: path.to_path_buf(),
            expected: artifact.sha256.clone(),
            actual,
        });
    }
    Ok(())
}

fn download_artifact(artifact: &ModelArtifact, destination: &Path) -> Result<(), CacheError> {
    let temporary = destination.with_extension(format!("part-{}", std::process::id()));
    let result = (|| {
        let response = ureq::get(&artifact.url)
            .call()
            .map_err(|source| CacheError::Download {
                url: artifact.url.clone(),
                source: Box::new(source),
            })?;
        let mut reader = response
            .into_parts()
            .1
            .into_reader()
            .take(artifact.bytes + 1);
        let mut file = File::create(&temporary).map_err(|source| io_error(&temporary, source))?;
        let mut hasher = Sha256::new();
        let mut written = 0_u64;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let count = reader
                .read(&mut buffer)
                .map_err(|source| io_error(&temporary, source))?;
            if count == 0 {
                break;
            }
            file.write_all(&buffer[..count])
                .map_err(|source| io_error(&temporary, source))?;
            hasher.update(&buffer[..count]);
            written += count as u64;
        }
        file.sync_all()
            .map_err(|source| io_error(&temporary, source))?;
        if written != artifact.bytes {
            return Err(CacheError::WrongLength {
                path: temporary.clone(),
                expected: artifact.bytes,
                actual: written,
            });
        }
        let actual = format!("{:x}", hasher.finalize());
        if actual != artifact.sha256 {
            return Err(CacheError::WrongDigest {
                path: temporary.clone(),
                expected: artifact.sha256.clone(),
                actual,
            });
        }
        fs::rename(&temporary, destination).map_err(|source| io_error(destination, source))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn safe_component(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_owned()
}

fn io_error(path: &Path, source: io::Error) -> CacheError {
    CacheError::Io {
        path: path.to_path_buf(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use super::{CacheError, ModelArtifact, parse_manifest, safe_component, verify_artifact};
    use sha2::{Digest, Sha256};
    use std::{fs, time::SystemTime};

    #[test]
    fn checked_in_manifest_is_complete() {
        let manifest = parse_manifest().expect("manifest should parse");
        assert_eq!(manifest.license, "BSD-2-Clause");
        assert_eq!(manifest.artifacts.len(), 3);
        assert_eq!(
            manifest
                .artifacts
                .iter()
                .map(|artifact| (
                    artifact.role.as_str(),
                    artifact.file.as_str(),
                    artifact.bytes
                ))
                .collect::<Vec<_>>(),
            vec![
                ("face_detection", "mnv3_detection_opt.onnx", 568302),
                (
                    "face_landmarks_wink_optimized",
                    "lm_model4_opt.onnx",
                    13501414,
                ),
                ("pupil_gaze", "mnv3_gaze32_split_opt.onnx", 3922610,),
            ]
        );
        assert!(manifest.artifacts.iter().all(|artifact| {
            artifact.url.starts_with("https://")
                && artifact.sha256.len() == 64
                && artifact.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        }));
    }

    #[test]
    fn verifies_length_and_digest() {
        let directory = std::env::temp_dir().join(format!(
            "nickel-gaze-cache-test-{}-{:?}",
            std::process::id(),
            SystemTime::now()
        ));
        fs::create_dir_all(&directory).expect("temporary directory should exist");
        let path = directory.join("model.onnx");
        fs::write(&path, b"model bytes").expect("fixture should write");
        let artifact = ModelArtifact {
            role: "test".into(),
            file: "model.onnx".into(),
            url: "https://example.invalid/model.onnx".into(),
            bytes: 11,
            sha256: format!("{:x}", Sha256::digest(b"model bytes")),
        };
        verify_artifact(&path, &artifact).expect("matching artifact should verify");
        fs::write(&path, b"wrong bytes").expect("fixture should rewrite");
        assert!(matches!(
            verify_artifact(&path, &artifact),
            Err(CacheError::WrongDigest { expected, actual, .. })
                if expected == artifact.sha256 && actual != expected
        ));
        fs::write(&path, b"short").expect("fixture should rewrite with a wrong length");
        assert!(matches!(
            verify_artifact(&path, &artifact),
            Err(CacheError::WrongLength {
                expected: 11,
                actual: 5,
                ..
            })
        ));
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn model_name_becomes_safe_cache_component() {
        assert_eq!(
            safe_component("OpenSeeFace gaze probe bundle"),
            "openseeface-gaze-probe-bundle"
        );
    }
}
