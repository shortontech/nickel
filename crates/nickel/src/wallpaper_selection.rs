//! Bounded discovery and validation for remotely selectable wallpaper images.

use image::GenericImageView;
use nickel_remote_control::wallpaper::ImageChoice;
use nickel_storage::{RegularFileRevision, read_regular_file, regular_file_revision};
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    io::Cursor,
    path::{Path, PathBuf},
};

const MAX_CATALOG_ENTRIES: usize = 128;
const MAX_DIRECTORY_VISITS: usize = 512;
const MAX_DIRECTORY_DEPTH: usize = 4;
const MAX_IMAGE_BYTES: usize = 32 * 1024 * 1024;
const MAX_IMAGE_DIMENSION: u32 = 16_384;
const MAX_IMAGE_PIXELS: u64 = 16_777_216;
const MAX_DECODE_ALLOCATION: u64 = 128 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CandidateRevision {
    id: String,
    revision: RegularFileRevision,
}

pub(crate) struct Candidate {
    choice: ImageChoice,
    path: PathBuf,
    revision: RegularFileRevision,
}

pub(crate) struct Catalog {
    candidates: Vec<Candidate>,
}

impl Catalog {
    pub(crate) fn discover(mut check: impl FnMut() -> Result<(), String>) -> Result<Self, String> {
        Self::discover_in(&wallpaper_roots(), &mut check)
    }

    fn discover_in(
        roots: &[PathBuf],
        check: &mut impl FnMut() -> Result<(), String>,
    ) -> Result<Self, String> {
        let mut paths = Vec::new();
        let mut visits = 0usize;
        for root in roots {
            check()?;
            let Ok(root_metadata) = std::fs::symlink_metadata(root) else {
                continue;
            };
            if !root_metadata.is_dir() || root_metadata.file_type().is_symlink() {
                continue;
            }
            let Ok(canonical_root) = root.canonicalize() else {
                continue;
            };
            let mut pending = VecDeque::from([(canonical_root.clone(), 0usize)]);
            while let Some((directory, depth)) = pending.pop_front() {
                check()?;
                if visits >= MAX_DIRECTORY_VISITS || paths.len() >= MAX_CATALOG_ENTRIES {
                    break;
                }
                visits += 1;
                let Ok(entries) = std::fs::read_dir(&directory) else {
                    continue;
                };
                let mut entries = entries.filter_map(Result::ok).collect::<Vec<_>>();
                entries.sort_by_key(std::fs::DirEntry::file_name);
                for entry in entries {
                    check()?;
                    if visits >= MAX_DIRECTORY_VISITS || paths.len() >= MAX_CATALOG_ENTRIES {
                        break;
                    }
                    visits += 1;
                    let path = entry.path();
                    let Ok(metadata) = std::fs::symlink_metadata(&path) else {
                        continue;
                    };
                    if metadata.file_type().is_symlink() {
                        continue;
                    }
                    if metadata.is_dir() && depth < MAX_DIRECTORY_DEPTH {
                        pending.push_back((path, depth + 1));
                    } else if metadata.is_file() && supported_extension(&path) {
                        paths.push((canonical_root.clone(), path));
                    }
                }
            }
        }
        paths.sort();
        paths.dedup_by(|left, right| left.1 == right.1);

        let mut candidates = Vec::new();
        for (root, path) in paths.into_iter().take(MAX_CATALOG_ENTRIES) {
            check()?;
            let Ok(canonical) = path.canonicalize() else {
                continue;
            };
            if !canonical.starts_with(&root) {
                continue;
            }
            let Ok(Some(revision)) = regular_file_revision(&canonical) else {
                continue;
            };
            let id = opaque_id(&root, &canonical);
            candidates.push(Candidate {
                choice: ImageChoice {
                    id: id.clone(),
                    configured: false,
                },
                path: canonical,
                revision,
            });
        }
        candidates.sort_by(|left, right| left.choice.id.cmp(&right.choice.id));
        Ok(Self { candidates })
    }

    pub(crate) fn choices(&self, configured: Option<&Path>) -> Vec<ImageChoice> {
        self.candidates
            .iter()
            .map(|candidate| {
                let mut choice = candidate.choice.clone();
                choice.configured = configured.is_some_and(|path| path == candidate.path);
                choice
            })
            .collect()
    }

    pub(crate) fn revisions(&self) -> Vec<CandidateRevision> {
        self.candidates
            .iter()
            .map(|candidate| CandidateRevision {
                id: candidate.choice.id.clone(),
                revision: candidate.revision.clone(),
            })
            .collect()
    }

    pub(crate) fn validate_selection(
        &self,
        id: &str,
        mut check: impl FnMut() -> Result<(), String>,
    ) -> Result<ValidatedSelection, String> {
        if id.len() != 64 || !id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("wallpaper image is not in the approved catalog".into());
        }
        let candidate = self
            .candidates
            .iter()
            .find(|candidate| candidate.choice.id == id)
            .ok_or("wallpaper image is not in the approved catalog")?;
        check()?;
        let bytes = read_regular_file(&candidate.path, MAX_IMAGE_BYTES)
            .map_err(|_| "wallpaper image is inaccessible, changing, or exceeds the file budget")?
            .ok_or("wallpaper image is unavailable")?;
        check()?;
        if regular_file_revision(&candidate.path)
            .map_err(|_| "wallpaper image is unavailable")?
            .as_ref()
            != Some(&candidate.revision)
        {
            return Err("wallpaper image changed; read current wallpaper before retrying".into());
        }
        let mut reader = image::ImageReader::new(Cursor::new(bytes))
            .with_guessed_format()
            .map_err(|_| "wallpaper image format could not be identified")?;
        reader.limits(decode_limits());
        let (width, height) = reader
            .into_dimensions()
            .map_err(|_| "wallpaper image header could not be decoded")?;
        if width == 0
            || height == 0
            || u64::from(width).saturating_mul(u64::from(height)) > MAX_IMAGE_PIXELS
        {
            return Err("wallpaper image exceeds the pixel budget".into());
        }
        check()?;
        let bytes = read_regular_file(&candidate.path, MAX_IMAGE_BYTES)
            .map_err(|_| "wallpaper image is inaccessible, changing, or exceeds the file budget")?
            .ok_or("wallpaper image is unavailable")?;
        let mut reader = image::ImageReader::new(Cursor::new(bytes))
            .with_guessed_format()
            .map_err(|_| "wallpaper image format could not be identified")?;
        reader.limits(decode_limits());
        let decoded = reader
            .decode()
            .map_err(|_| "wallpaper image could not be decoded")?;
        if decoded.dimensions() != (width, height) {
            return Err("wallpaper image changed while decoding".into());
        }
        check()?;
        if regular_file_revision(&candidate.path)
            .map_err(|_| "wallpaper image is unavailable")?
            .as_ref()
            != Some(&candidate.revision)
        {
            return Err("wallpaper image changed; read current wallpaper before retrying".into());
        }
        Ok(ValidatedSelection {
            path: candidate.path.clone(),
            revision: candidate.revision.clone(),
        })
    }
}

pub(crate) struct ValidatedSelection {
    pub(crate) path: PathBuf,
    revision: RegularFileRevision,
}

impl ValidatedSelection {
    pub(crate) fn ensure_current(&self) -> Result<(), String> {
        if regular_file_revision(&self.path)
            .map_err(|_| "wallpaper image is unavailable")?
            .as_ref()
            != Some(&self.revision)
        {
            return Err("wallpaper image changed; read current wallpaper before retrying".into());
        }
        Ok(())
    }
}

fn supported_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            ["jpg", "jpeg", "png", "webp"]
                .iter()
                .any(|supported| extension.eq_ignore_ascii_case(supported))
        })
}

fn decode_limits() -> image::Limits {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_DIMENSION);
    limits.max_image_height = Some(MAX_IMAGE_DIMENSION);
    limits.max_alloc = Some(MAX_DECODE_ALLOCATION);
    limits
}

fn opaque_id(root: &Path, path: &Path) -> String {
    let mut digest = Sha256::new();
    digest.update(b"nickel-approved-wallpaper-v1\0");
    digest.update(root.to_string_lossy().as_bytes());
    digest.update(b"\0");
    digest.update(path.to_string_lossy().as_bytes());
    format!("{:x}", digest.finalize())
}

fn wallpaper_roots() -> Vec<PathBuf> {
    let mut roots = nickel_storage::config_path("wallpapers")
        .ok()
        .into_iter()
        .collect::<Vec<_>>();
    #[cfg(target_os = "linux")]
    roots.extend([
        PathBuf::from("/usr/share/backgrounds"),
        PathBuf::from("/usr/share/wallpapers"),
    ]);
    #[cfg(target_os = "windows")]
    if let Some(windows) = std::env::var_os("WINDIR") {
        roots.push(PathBuf::from(windows).join("Web").join("Wallpaper"));
    }
    roots
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};

    #[test]
    fn catalog_ids_cannot_escape_roots_and_selection_is_fully_decoded() {
        let root = tempfile::tempdir().unwrap();
        let approved = root.path().join("approved");
        std::fs::create_dir(&approved).unwrap();
        let valid = approved.join("valid.png");
        RgbaImage::from_pixel(4, 3, Rgba([1, 2, 3, 255]))
            .save(&valid)
            .unwrap();
        std::fs::write(approved.join("broken.png"), b"not an image").unwrap();
        let outside = root.path().join("outside.png");
        RgbaImage::from_pixel(1, 1, Rgba([9, 8, 7, 255]))
            .save(&outside)
            .unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, approved.join("escape.png")).unwrap();

        let catalog = Catalog::discover_in(&[approved], &mut || Ok(())).unwrap();
        assert_eq!(catalog.choices(None).len(), 2);
        let valid_id = catalog
            .candidates
            .iter()
            .find(|candidate| candidate.path == valid)
            .unwrap()
            .choice
            .id
            .clone();
        let selected = catalog.validate_selection(&valid_id, || Ok(())).unwrap();
        assert_eq!(selected.path, valid);
        assert!(
            catalog
                .validate_selection("../outside.png", || Ok(()))
                .is_err()
        );
        let broken_id = catalog
            .candidates
            .iter()
            .find(|candidate| candidate.path.ends_with("broken.png"))
            .unwrap()
            .choice
            .id
            .clone();
        assert!(catalog.validate_selection(&broken_id, || Ok(())).is_err());
    }

    #[test]
    fn oversized_file_and_pixel_dimensions_are_rejected() {
        let root = tempfile::tempdir().unwrap();
        let approved = root.path().join("approved");
        std::fs::create_dir(&approved).unwrap();
        std::fs::write(approved.join("large.png"), vec![0; MAX_IMAGE_BYTES + 1]).unwrap();
        let catalog = Catalog::discover_in(&[approved], &mut || Ok(())).unwrap();
        let id = catalog.candidates[0].choice.id.clone();
        assert!(catalog.validate_selection(&id, || Ok(())).is_err());
    }
}
