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
    #[cfg(test)]
    pub(crate) fn discover_fixture(root: &Path) -> Self {
        Self::discover_in(&[root.to_owned()], &mut || Ok(())).unwrap()
    }
    pub(crate) fn approves_preview_id(&self, id: &str) -> bool {
        self.candidates
            .binary_search_by(|candidate| candidate.choice.id.as_str().cmp(id))
            .is_ok()
    }
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

    /// Accepts a path only from the native chooser completion, never the public ABI.
    pub(crate) fn validate_chosen(
        path: PathBuf,
        check: impl FnMut() -> Result<(), String>,
    ) -> Result<ValidatedSelection, String> {
        if !supported_extension(&path) {
            return Err("Choose a PNG, JPEG, or WebP image".into());
        }
        let path = path
            .canonicalize()
            .map_err(|_| "Chosen image is unavailable")?;
        let revision = regular_file_revision(&path)
            .map_err(|_| "Chosen image is unavailable")?
            .ok_or("Chosen image is unavailable")?;
        let id = opaque_id(&path, &path);
        let catalog = Self {
            candidates: vec![Candidate {
                choice: ImageChoice {
                    id: id.clone(),
                    configured: false,
                },
                path,
                revision,
            }],
        };
        catalog.validate_selection(&id, check)
    }

    pub(crate) fn presentation(&self) -> Vec<(String, String)> {
        self.candidates
            .iter()
            .map(|candidate| {
                (
                    candidate.choice.id.clone(),
                    candidate
                        .path
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .chars()
                        .filter(|c| !c.is_control())
                        .take(120)
                        .collect(),
                )
            })
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn previews(
        &self,
    ) -> std::collections::BTreeMap<String, (u16, std::sync::Arc<image::RgbaImage>)> {
        self.previews_for(
            self.candidates
                .iter()
                .take(8)
                .map(|candidate| candidate.choice.id.as_str()),
            || Ok(()),
        )
    }

    /// Decode only requested approved identities, with the existing eight-image
    /// work budget. A native worker can cancel superseded demand between file and
    /// decoder stages; cancellation also prevents publishing a partial old batch.
    pub(crate) fn previews_for<'a>(
        &self,
        requested: impl IntoIterator<Item = &'a str>,
        mut check: impl FnMut() -> Result<(), String>,
    ) -> std::collections::BTreeMap<String, (u16, std::sync::Arc<image::RgbaImage>)> {
        let mut previews = std::collections::BTreeMap::new();
        let mut attempted = std::collections::BTreeSet::new();
        // Bound inspected requests too: a malicious or obsolete request stream
        // cannot make this native job iterate indefinitely over rejected IDs.
        for id in requested.into_iter().take(MAX_CATALOG_ENTRIES) {
            if check().is_err() {
                return Default::default();
            }
            let Some(index) = self
                .candidates
                .iter()
                .position(|candidate| candidate.choice.id == id)
            else {
                continue;
            };
            if !attempted.insert(index) {
                continue;
            }
            if let Ok(selected) = self.validate_selection(id, &mut check) {
                // Use the catalog ordinal, not request order: scrolling/reordering
                // demand must not alias two retained image identities.
                previews.insert(
                    format!("wallpaper:{id}"),
                    (64_000 + index as u16, selected.preview),
                );
            }
            if check().is_err() {
                return Default::default();
            }
            if attempted.len() == 8 {
                break;
            }
        }
        previews
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
        let mut reader = image::ImageReader::new(Cursor::new(bytes.as_slice()))
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
        // Inspect and decode the same bounded byte snapshot. Re-reading the
        // entire file here duplicates I/O/allocation and can only introduce a
        // different candidate between header validation and decoding. The
        // before/after revision checks still reject replacement or mutation.
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
            preview: std::sync::Arc::new(decoded.thumbnail(160, 90).to_rgba8()),
        })
    }
}

pub(crate) struct ValidatedSelection {
    preview: std::sync::Arc<image::RgbaImage>,
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
    fn wallpaper_preview_batches_finish_all_visible_consumers_and_discard_replaced_demand() {
        use crate::wallpaper_previews::WallpaperPreviews;
        use std::{
            sync::Arc,
            time::{Duration, Instant},
        };
        let directory = tempfile::tempdir().unwrap();
        for index in 0..24 {
            RgbaImage::from_pixel(16, 9, Rgba([index, 2, 3, 255]))
                .save(directory.path().join(format!("{index:02}.png")))
                .unwrap();
        }
        let catalog = Catalog::discover_in(&[directory.path().to_owned()], &mut || Ok(())).unwrap();
        let assets = catalog
            .candidates
            .iter()
            .map(|candidate| format!("wallpaper:{}", candidate.choice.id))
            .collect::<Vec<_>>();
        let keys = ["first", "second"].map(|surface_id| nickel_core::plugins::PluginSurfaceKey {
            plugin_id: "test".into(),
            surface_id: surface_id.into(),
        });
        let mut first = vec![assets[0].clone(); 130];
        first.push(format!("wallpaper:{}", "0".repeat(64)));
        first.extend_from_slice(&assets[..10]);
        let first = Arc::new(first);
        let second = Arc::new(assets[10..20].to_vec());
        let mut previews = WallpaperPreviews::default();
        previews.set_catalog(catalog);
        previews.sync_surfaces(
            [(&keys[0], first), (&keys[1], second)].into_iter(),
            Instant::now(),
        );
        let timeout = Instant::now() + Duration::from_secs(5);
        let mut completions = 0;
        let mut prior = 0;
        while let Some(deadline) = previews.next_deadline() {
            assert!(Instant::now() < timeout);
            if previews.poll(deadline) {
                completions += 1;
                assert!(
                    previews.images().len() - prior <= 8,
                    "one completion exceeded the decode work budget"
                );
                prior = previews.images().len();
            }
            std::thread::yield_now();
        }
        assert_eq!(
            previews.images().len(),
            20,
            "visible consumers must not be truncated to the first batch"
        );
        assert_eq!(completions, 3);
        assert!(
            assets[..20]
                .iter()
                .all(|asset| previews.images().contains_key(asset))
        );
        let replacement = Arc::new(assets[20..].to_vec());
        previews.sync_surfaces(std::iter::once((&keys[0], replacement)), Instant::now());
        assert!(
            previews.images().is_empty(),
            "offscreen images must be released immediately"
        );
        previews.sync_surfaces(
            std::iter::once((&keys[1], Arc::new(vec![assets[0].clone()]))),
            Instant::now(),
        );
        while let Some(deadline) = previews.next_deadline() {
            assert!(Instant::now() < timeout);
            previews.poll(deadline);
            assert!(
                previews.images().keys().all(|asset| asset == &assets[0]),
                "a superseded completion was published"
            );
            std::thread::yield_now();
        }
        assert_eq!(previews.images().len(), 1);
        previews.clear_catalog();
        assert!(previews.images().is_empty());
        previews.sync_surfaces(
            std::iter::once((&keys[1], Arc::new(vec![assets[0].clone()]))),
            Instant::now(),
        );
        assert!(previews.next_deadline().is_none());
        // The same visible declaration may outlive an unavailable observation.
        // Catalog recovery must re-admit its demand even though the surface's
        // asset list has not changed.
        previews.set_catalog(
            Catalog::discover_in(&[directory.path().to_owned()], &mut || Ok(())).unwrap(),
        );
        assert!(previews.next_deadline().is_none());
        previews.sync_surfaces(
            std::iter::once((&keys[1], Arc::new(vec![assets[0].clone()]))),
            Instant::now(),
        );
        assert!(previews.next_deadline().is_some());
        while let Some(deadline) = previews.next_deadline() {
            assert!(Instant::now() < timeout);
            previews.poll(deadline);
            std::thread::yield_now();
        }
        assert_eq!(previews.images().len(), 1);
    }

    #[test]
    fn wallpaper_preview_worker_is_lazy_cancellable_and_retires_surface_demand() {
        use crate::wallpaper_previews::WallpaperPreviews;
        use std::{
            sync::Arc,
            time::{Duration, Instant},
        };
        fn finish(previews: &mut WallpaperPreviews) {
            let timeout = Instant::now() + Duration::from_secs(5);
            while let Some(deadline) = previews.next_deadline() {
                assert!(Instant::now() < timeout, "preview worker did not settle");
                previews.poll(deadline);
                std::thread::yield_now();
            }
        }
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("wallpaper.png");
        RgbaImage::from_pixel(12, 9, Rgba([1, 2, 3, 255]))
            .save(&path)
            .unwrap();
        let catalog = Catalog::discover_in(&[directory.path().to_owned()], &mut || Ok(())).unwrap();
        let id = catalog.candidates[0].choice.id.clone();
        let asset = format!("wallpaper:{id}");
        let key = nickel_core::plugins::PluginSurfaceKey {
            plugin_id: "test".into(),
            surface_id: "settings".into(),
        };
        let demand = Arc::new(vec![asset.clone()]);
        let mut previews = WallpaperPreviews::default();
        previews.set_catalog(catalog);
        assert!(previews.images().is_empty());
        assert!(
            previews.next_deadline().is_none(),
            "catalog observation must not start decoding"
        );
        previews.sync_surfaces(std::iter::empty(), Instant::now());
        assert!(previews.next_deadline().is_none());
        previews.sync_surfaces(std::iter::once((&key, Arc::clone(&demand))), Instant::now());
        assert!(previews.next_deadline().is_some());
        // Retire before consuming the result, regardless of whether the worker
        // has already completed. An old completion must not revive the image.
        previews.sync_surfaces(std::iter::empty(), Instant::now());
        finish(&mut previews);
        assert!(previews.images().is_empty());
        previews.sync_surfaces(std::iter::once((&key, Arc::clone(&demand))), Instant::now());
        finish(&mut previews);
        let image = Arc::clone(&previews.images()[&asset].1);
        assert_eq!(image.get_pixel(0, 0).0, [1, 2, 3, 255]);
        for _ in 0..100 {
            previews.sync_surfaces(std::iter::once((&key, Arc::clone(&demand))), Instant::now());
        }
        assert!(
            previews.next_deadline().is_none(),
            "unchanged visible demand must stay idle"
        );
        assert!(Arc::ptr_eq(&image, &previews.images()[&asset].1));

        RgbaImage::from_pixel(12, 9, Rgba([9, 8, 7, 255]))
            .save(&path)
            .unwrap();
        previews.set_catalog(
            Catalog::discover_in(&[directory.path().to_owned()], &mut || Ok(())).unwrap(),
        );
        assert!(previews.images().is_empty());
        assert!(previews.next_deadline().is_none());
        previews.sync_surfaces(std::iter::once((&key, Arc::clone(&demand))), Instant::now());
        finish(&mut previews);
        assert_eq!(
            previews.images()[&asset].1.get_pixel(0, 0).0,
            [9, 8, 7, 255]
        );
        previews.sync_surfaces(std::iter::empty(), Instant::now());
        assert!(previews.images().is_empty());
        assert!(previews.next_deadline().is_none());
    }

    #[test]
    fn preview_demand_decodes_only_requested_approved_rows_with_stable_image_ids() {
        let directory = tempfile::tempdir().unwrap();
        for index in 0..12 {
            RgbaImage::from_pixel(4, 3, Rgba([index, 2, 3, 255]))
                .save(directory.path().join(format!("{index:02}.png")))
                .unwrap();
        }
        let catalog = Catalog::discover_in(&[directory.path().to_owned()], &mut || Ok(())).unwrap();
        let first = catalog.candidates[0].choice.id.as_str();
        let last = catalog.candidates[11].choice.id.as_str();
        let last_pixel: u8 = catalog.candidates[11]
            .path
            .file_stem()
            .unwrap()
            .to_str()
            .unwrap()
            .parse()
            .unwrap();
        let mut checks = 0;
        let empty = catalog.previews_for(std::iter::empty(), || {
            checks += 1;
            Ok(())
        });
        assert!(empty.is_empty());
        assert_eq!(checks, 0, "empty demand does no validation or file work");
        let previews = catalog.previews_for([last, last, "../outside.png", first], || Ok(()));
        assert_eq!(previews.len(), 2);
        assert_eq!(previews[&format!("wallpaper:{last}")].0, 64_011);
        assert_eq!(
            previews[&format!("wallpaper:{last}")].1.get_pixel(0, 0).0,
            [last_pixel, 2, 3, 255]
        );
        assert_eq!(previews[&format!("wallpaper:{first}")].0, 64_000);
        let reversed = catalog.previews_for([first, last], || Ok(()));
        assert_eq!(
            previews, reversed,
            "demand order does not change image identity"
        );
        let bounded = catalog.previews_for(
            catalog
                .candidates
                .iter()
                .rev()
                .map(|candidate| candidate.choice.id.as_str()),
            || Ok(()),
        );
        assert_eq!(bounded.len(), 8);
        assert!(bounded.contains_key(&format!("wallpaper:{last}")));
        assert!(!bounded.contains_key(&format!("wallpaper:{first}")));

        // Decode one row, then invalidate the remaining batch. No completed old
        // result may escape cancellation and be installed in a newer viewport.
        let mut checks = 0;
        let cancelled = catalog.previews_for([first, last], || {
            checks += 1;
            if checks >= 7 {
                Err("superseded".into())
            } else {
                Ok(())
            }
        });
        assert!(cancelled.is_empty());
        assert_eq!(checks, 7);

        std::fs::write(&catalog.candidates[11].path, b"changed since discovery").unwrap();
        assert!(
            catalog.previews_for([last], || Ok(())).is_empty(),
            "stale catalog revisions remain rejected"
        );
    }

    #[test]
    fn native_catalog_previews_are_bounded_and_invalid_images_are_omitted() {
        let directory = tempfile::tempdir().unwrap();
        for index in 0..10 {
            RgbaImage::from_pixel(400, 300, Rgba([1, 2, 3, 255]))
                .save(directory.path().join(format!("Landscape {index}.png")))
                .unwrap();
        }
        let catalog = Catalog::discover_in(&[directory.path().to_owned()], &mut || Ok(())).unwrap();
        let previews = catalog.previews();
        assert_eq!(previews.len(), 8);
        assert!(
            previews
                .values()
                .all(|(_, image)| image.width() <= 160 && image.height() <= 90)
        );
        assert!(
            catalog
                .presentation()
                .iter()
                .all(|(_, label)| label.starts_with("Landscape ") && !label.contains('/'))
        );
        let invalid = directory.path().join("invalid.png");
        std::fs::write(&invalid, b"not an image").unwrap();
        assert!(Catalog::validate_chosen(invalid, || Ok(())).is_err());
    }

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
    fn wallpaper_snapshot_decode_rejects_changes_after_read_header_and_decode() {
        for mutation_checkpoint in [2, 3, 4] {
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join("fixture.png");
            RgbaImage::from_pixel(4, 3, Rgba([1, 2, 3, 255]))
                .save(&path)
                .unwrap();
            let catalog = Catalog::discover_in(&[root.path().to_owned()], &mut || Ok(())).unwrap();
            let id = &catalog.candidates[0].choice.id;
            let mut checkpoint = 0;
            let result = catalog.validate_selection(id, || {
                checkpoint += 1;
                if checkpoint == mutation_checkpoint {
                    std::fs::write(&path, b"changed after the bounded read").unwrap();
                }
                Ok(())
            });
            assert!(
                matches!(result, Err(ref error) if error.contains("changed")),
                "checkpoint {mutation_checkpoint} must revoke the decoded byte snapshot"
            );
        }
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
