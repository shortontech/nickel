//! Native-owned, immutable logical collection sources. A catalog belongs to one
//! presentation owner; references are never shared between package mounts.

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use twinkle::VirtualHeightIndex;

const MAX_SOURCES: usize = 64;
const MAX_SOURCE_ROWS: usize = 10_000;
const MAX_TOTAL_ROWS: usize = 100_000;
const MAX_PAYLOAD_BYTES: usize = 8 * 1024 * 1024;
pub(crate) const MAX_KEY_BYTES: usize = 512;
const MAX_REVISION: u64 = (1u64 << 53) - 1;
static NEXT_REVISION: AtomicU64 = AtomicU64::new(1);

fn allocate_revision(counter: &AtomicU64) -> Result<u64, String> {
    let mut current = counter.load(Ordering::Relaxed);
    loop {
        if current > MAX_REVISION {
            return Err("virtual source revisions exhausted".into());
        }
        match counter.compare_exchange_weak(
            current,
            current + 1,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => return Ok(current),
            Err(next) => current = next,
        }
    }
}

/// An admitted logical source. Clones of the enclosing Arc do not copy item
/// keys or extents. Keys are retained independently of materialized row trees.
#[derive(Debug)]
pub struct VirtualSource {
    revision: u64,
    identity: u64,
    keys: Arc<[Arc<str>]>,
    ordinals: Arc<BTreeMap<Arc<str>, usize>>,
    geometry: Arc<VirtualHeightIndex>,
    estimates: Arc<VirtualHeightIndex>,
    measurement_context: Option<VirtualMeasurementContext>,
    gap: f32,
    payload_bytes: usize,
}

/// Native layout authority for measured extents. `layout_revision` must change
/// when row templates or font/style metrics change independently of the source.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VirtualMeasurementContext {
    pub width: f32,
    pub scale: f32,
    pub layout_revision: u64,
}

impl VirtualSource {
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Native collection lifetime, stable across data revisions but never
    /// reused after retirement. It identifies keyed rows, not source authority.
    pub fn identity(&self) -> u64 {
        self.identity
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    pub fn key(&self, ordinal: usize) -> Option<&str> {
        self.keys.get(ordinal).map(AsRef::as_ref)
    }

    pub fn ordinal(&self, key: &str) -> Option<usize> {
        self.ordinals.get(key).copied()
    }

    /// Resolve a logical key into the existing native virtual-window geometry.
    /// No row trees or logical arrays are constructed or copied. Callers retain
    /// scroll/focus authority and must use the current admitted source revision.
    pub fn reveal_window(
        &self,
        key: &str,
        viewport: f32,
        overscan: f32,
    ) -> Option<twinkle::VirtualWindow> {
        if !viewport.is_finite() || viewport <= 0.0 || !overscan.is_finite() || overscan < 0.0 {
            return None;
        }
        let ordinal = self.ordinal(key)?;
        let offset = self
            .geometry
            .window_for_range(ordinal..ordinal + 1)?
            .leading;
        Some(self.geometry.window(offset, viewport, overscan))
    }

    pub fn geometry(&self) -> &VirtualHeightIndex {
        &self.geometry
    }

    pub(crate) fn geometry_handle(&self) -> Arc<VirtualHeightIndex> {
        Arc::clone(&self.geometry)
    }

    pub fn gap(&self) -> f32 {
        self.gap
    }

    pub fn measurement_context(&self) -> Option<VirtualMeasurementContext> {
        self.measurement_context
    }

    /// Accounted source payload, excluding allocator and BTree node overhead.
    /// Separate row/source-count limits also bound that overhead.
    pub fn payload_bytes(&self) -> usize {
        self.payload_bytes
    }
}

/// Immutable source admission for one package/mount lifetime. `owner` is a
/// native collection identity supplied by the presentation adapter, not a
/// package-global capability. Dropping this catalog retires its namespace.
#[derive(Clone, Debug, Default)]
pub struct VirtualSourceCatalog {
    sources: BTreeMap<String, Arc<VirtualSource>>,
    repair_identities: HashSet<u64>,
    rows: usize,
    payload_bytes: usize,
}

impl VirtualSourceCatalog {
    /// Validate changed source data once and replace atomically. Native-issued
    /// revisions never repeat, including across catalog lifetimes. Querying a revision uses
    /// `resolve` and never revalidates/hashes the logical source.
    pub fn admit(
        &mut self,
        owner: &str,
        keys: &[String],
        heights: &[f32],
        gap: f32,
    ) -> Result<Arc<VirtualSource>, String> {
        if owner.is_empty() || owner.len() > 1024 {
            return Err("invalid virtual source owner".into());
        }
        let previous = self.sources.get(owner);
        if keys.len() != heights.len() || keys.len() > MAX_SOURCE_ROWS {
            return Err("invalid virtual source row count".into());
        }
        if !gap.is_finite()
            || !(0.0..=1024.0).contains(&gap)
            || heights
                .iter()
                .any(|height| !height.is_finite() || !(1.0..=8192.0).contains(height))
        {
            return Err("invalid virtual source geometry".into());
        }
        let mut seen = HashSet::with_capacity(keys.len());
        if keys
            .iter()
            .any(|key| key.is_empty() || key.len() > MAX_KEY_BYTES || !seen.insert(key.as_str()))
        {
            return Err("virtual source keys must be unique and bounded".into());
        }
        let rows = self.rows - previous.map_or(0, |source| source.len()) + keys.len();
        if (previous.is_none() && self.sources.len() >= MAX_SOURCES) || rows > MAX_TOTAL_ROWS {
            return Err("virtual source catalog capacity exceeded".into());
        }
        let uniform = heights
            .first()
            .copied()
            .filter(|first| heights.iter().all(|height| height == first));
        let extent_bytes = if uniform.is_some() || heights.is_empty() {
            0
        } else {
            heights.len() * 2 * std::mem::size_of::<f32>()
        };
        // Each key is stored once, with two Arc handles (ordinal vector/map),
        // its refcounts, and the map's ordinal payload.
        let source_bytes = keys.iter().map(String::len).sum::<usize>()
            + keys.len() * (2 * std::mem::size_of::<Arc<str>>() + 3 * std::mem::size_of::<usize>())
            + extent_bytes;
        let payload_bytes =
            self.payload_bytes - previous.map_or(0, |source| source.payload_bytes) + source_bytes;
        if payload_bytes > MAX_PAYLOAD_BYTES {
            return Err("virtual source payload budget exceeded".into());
        }
        let revision = allocate_revision(&NEXT_REVISION)?;
        let geometry = if heights.is_empty() || uniform.is_some() {
            VirtualHeightIndex::uniform(heights.len(), uniform.unwrap_or(1.0), gap)
        } else {
            VirtualHeightIndex::new(heights, gap)
        };
        let keys: Arc<[Arc<str>]> = keys.iter().map(|key| Arc::from(key.as_str())).collect();
        let ordinals = keys
            .iter()
            .enumerate()
            .map(|(ordinal, key)| (Arc::clone(key), ordinal))
            .collect();
        let geometry = Arc::new(geometry);
        let source = Arc::new(VirtualSource {
            revision,
            identity: previous.map_or(revision, |source| source.identity),
            keys,
            ordinals: Arc::new(ordinals),
            estimates: Arc::clone(&geometry),
            geometry,
            measurement_context: None,
            gap,
            payload_bytes: source_bytes,
        });
        self.sources.insert(owner.to_owned(), Arc::clone(&source));
        self.rows = rows;
        self.payload_bytes = payload_bytes;
        Ok(source)
    }

    /// Atomically apply native measurements for one layout context. Context
    /// changes discard old corrections, not logical keys or callbacks. Work is
    /// proportional to the supplied materialized rows and logarithmic paths;
    /// no logical-key or base-extent array is copied.
    pub fn measure(
        &mut self,
        owner: &str,
        revision: u64,
        context: VirtualMeasurementContext,
        measurements: &[(usize, f32)],
    ) -> Result<(Arc<VirtualSource>, usize), String> {
        if !context.width.is_finite()
            || context.width <= 0.0
            || !context.scale.is_finite()
            || context.scale <= 0.0
        {
            return Err("invalid virtual measurement context".into());
        }
        let previous = self.resolve(owner, revision)?;
        if measurements.len() > previous.len() {
            return Err("too many virtual row measurements".into());
        }
        let mut seen = HashSet::with_capacity(measurements.len());
        if measurements.iter().any(|(row, height)| {
            *row >= previous.len() || !height.is_finite() || *height < 0.0 || !seen.insert(*row)
        }) {
            return Err("invalid or duplicate virtual row measurement".into());
        }
        let mut geometry = Arc::clone(if previous.measurement_context == Some(context) {
            &previous.geometry
        } else {
            &previous.estimates
        });
        let mut allocations = 0;
        for &(row, height) in measurements {
            let (next, work) = geometry
                .with_measured_height(row, height)
                .ok_or("invalid corrected virtual geometry")?;
            geometry = next;
            allocations += work;
        }
        if previous.measurement_context == Some(context)
            && Arc::ptr_eq(&geometry, &previous.geometry)
        {
            return Ok((previous, allocations));
        }
        let source_bytes =
            previous.payload_bytes - previous.geometry.retained_bytes() + geometry.retained_bytes();
        let payload_bytes = self.payload_bytes - previous.payload_bytes + source_bytes;
        if payload_bytes > MAX_PAYLOAD_BYTES {
            return Err("virtual measurement payload budget exceeded".into());
        }
        let source = Arc::new(VirtualSource {
            revision: previous.revision,
            identity: previous.identity,
            keys: Arc::clone(&previous.keys),
            ordinals: Arc::clone(&previous.ordinals),
            estimates: Arc::clone(&previous.estimates),
            geometry,
            measurement_context: Some(context),
            gap: previous.gap,
            payload_bytes: source_bytes,
        });
        self.sources.insert(owner.to_owned(), Arc::clone(&source));
        self.payload_bytes = payload_bytes;
        Ok((source, allocations))
    }

    /// Resolve only within this mount and native collection owner. A stale
    /// revision cannot accidentally bind to replacement keys or callbacks.
    pub fn resolve(&self, owner: &str, revision: u64) -> Result<Arc<VirtualSource>, String> {
        self.sources
            .get(owner)
            .filter(|source| source.revision == revision)
            .map(Arc::clone)
            .ok_or_else(|| "unknown or retired virtual source".into())
    }

    pub fn retire(&mut self, owner: &str) -> bool {
        let Some(source) = self.sources.remove(owner) else {
            return false;
        };
        self.rows -= source.len();
        self.payload_bytes -= source.payload_bytes;
        true
    }

    pub(crate) fn set_repair_identities(&mut self, identities: impl Iterator<Item = u64>) {
        self.repair_identities = identities.collect();
    }

    pub(crate) fn retain_owners(&mut self, owners: &HashSet<&str>) {
        self.sources.retain(|owner, source| {
            if owners.contains(owner.as_str()) || self.repair_identities.contains(&source.identity)
            {
                return true;
            }
            self.rows -= source.len();
            self.payload_bytes -= source.payload_bytes;
            false
        });
    }

    pub fn source_count(&self) -> usize {
        self.sources.len()
    }

    pub(crate) fn owner_for_revision(&self, revision: u64) -> Option<&str> {
        self.sources
            .iter()
            .find_map(|(owner, source)| (source.revision() == revision).then_some(owner.as_str()))
    }
    pub fn logical_rows(&self) -> usize {
        self.rows
    }
    pub fn payload_bytes(&self) -> usize {
        self.payload_bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focused_ancestry_protection_is_scoped_and_does_not_revive_retired_sources() {
        let mut catalog = VirtualSourceCatalog::default();
        let keys = vec!["key".to_owned()];
        let focused = catalog.admit("focused", &keys, &[24.0], 0.0).unwrap();
        let other = catalog.admit("other", &keys, &[24.0], 0.0).unwrap();
        catalog.set_repair_identities(std::iter::once(focused.identity()));
        catalog.retain_owners(&HashSet::new());
        assert_eq!(catalog.source_count(), 1);
        assert!(catalog.resolve("focused", focused.revision()).is_ok());
        assert!(catalog.resolve("other", other.revision()).is_err());
        assert!(catalog.resolve("other", focused.revision()).is_err());
        let replacement = catalog.admit("focused", &keys, &[40.0], 0.0).unwrap();
        assert_eq!(replacement.identity(), focused.identity());
        assert_ne!(replacement.revision(), focused.revision());
        assert!(catalog.resolve("focused", focused.revision()).is_err());
        catalog.set_repair_identities(std::iter::empty());
        catalog.retain_owners(&HashSet::new());
        assert_eq!(catalog.source_count(), 0);
        assert_eq!(catalog.logical_rows(), 0);
        assert_eq!(catalog.payload_bytes(), 0);
        catalog.set_repair_identities(std::iter::once(focused.identity()));
        let fresh = catalog.admit("focused", &keys, &[24.0], 0.0).unwrap();
        assert_ne!(fresh.identity(), focused.identity());
        catalog.retain_owners(&HashSet::new());
        assert_eq!(catalog.source_count(), 0);
    }

    #[test]
    fn logical_reveal_selects_bounded_native_windows_without_materialized_rows() {
        for count in [100usize, 1_000, 10_000] {
            let keys: Vec<_> = (0..count).map(|row| format!("row-{row}")).collect();
            let heights: Vec<_> = (0..count)
                .map(|row| if row % 2 == 0 { 20.0 } else { 70.0 })
                .collect();
            let mut catalog = VirtualSourceCatalog::default();
            let source = catalog.admit("rows", &keys, &heights, 5.0).unwrap();
            let bytes = catalog.payload_bytes();
            for ordinal in [0, count / 2, count - 1] {
                let window = source.reveal_window(&keys[ordinal], 120.0, 40.0).unwrap();
                assert!(window.range.contains(&ordinal));
                assert!(window.range.len() <= 12);
                let offset = source
                    .geometry
                    .window_for_range(ordinal..ordinal + 1)
                    .unwrap()
                    .leading;
                assert_eq!(
                    window,
                    VirtualHeightIndex::new(&heights, 5.0).window(offset, 120.0, 40.0)
                );
                assert_eq!(catalog.payload_bytes(), bytes);
                assert!(Arc::ptr_eq(
                    &source,
                    &catalog.resolve("rows", source.revision()).unwrap()
                ));
            }
            assert!(source.reveal_window("absent", 120.0, 40.0).is_none());
            assert!(source.reveal_window(&keys[0], f32::NAN, 40.0).is_none());
            assert!(source.reveal_window(&keys[0], 0.0, 40.0).is_none());
            assert!(source.reveal_window(&keys[0], 120.0, -1.0).is_none());
        }
    }

    #[test]
    fn measurements_share_keys_and_reset_only_layout_dependent_geometry() {
        for count in [100usize, 1_000, 10_000] {
            let keys: Vec<_> = (0..count).map(|row| format!("row-{row}")).collect();
            let mut heights = vec![20.0; count];
            let mut catalog = VirtualSourceCatalog::default();
            let original = catalog.admit("rows", &keys, &heights, 5.0).unwrap();
            let context = VirtualMeasurementContext {
                width: 400.0,
                scale: 1.0,
                layout_revision: 1,
            };
            let (measured, allocations) = catalog
                .measure(
                    "rows",
                    original.revision(),
                    context,
                    &[(0, 40.0), (count - 1, 70.0)],
                )
                .unwrap();
            assert!(allocations <= 2 * (count.ilog2() as usize + 2));
            assert_eq!(measured.revision(), original.revision());
            assert!(Arc::ptr_eq(&measured.keys, &original.keys));
            assert!(Arc::ptr_eq(&measured.ordinals, &original.ordinals));
            assert!(Arc::ptr_eq(&measured.estimates, &original.geometry));
            assert_eq!(original.geometry.height(0), Some(20.0));
            heights[0] = 40.0;
            heights[count - 1] = 70.0;
            for offset in [0.0, 400.0, (count * 25 - 100) as f32] {
                assert_eq!(
                    measured.geometry.window(offset, 100.0, 40.0),
                    VirtualHeightIndex::new(&heights, 5.0).window(offset, 100.0, 40.0)
                );
            }
            assert_eq!(catalog.payload_bytes(), measured.payload_bytes());
            let (same, work) = catalog
                .measure("rows", original.revision(), context, &[(0, 40.0)])
                .unwrap();
            assert!(Arc::ptr_eq(&same, &measured));
            assert_eq!(work, 0);
            for changed in [
                VirtualMeasurementContext {
                    width: 401.0,
                    ..context
                },
                VirtualMeasurementContext {
                    scale: 2.0,
                    ..context
                },
                VirtualMeasurementContext {
                    layout_revision: 2,
                    ..context
                },
            ] {
                catalog
                    .measure("rows", original.revision(), context, &[(0, 40.0)])
                    .unwrap();
                let (reset, work) = catalog
                    .measure("rows", original.revision(), changed, &[])
                    .unwrap();
                assert_eq!(work, 0);
                assert!(Arc::ptr_eq(&reset.geometry, &original.geometry));
                assert_eq!(catalog.payload_bytes(), original.payload_bytes());
                assert_eq!(reset.measurement_context(), Some(changed));
            }
        }
    }

    #[test]
    fn invalid_measurements_and_budget_overflow_are_atomic() {
        let keys: Vec<_> = (0..10_000).map(|row| format!("{row:0>128}")).collect();
        let mut catalog = VirtualSourceCatalog::default();
        let original = catalog
            .admit("rows", &keys, &vec![20.0; keys.len()], 0.0)
            .unwrap();
        for owner in ["b", "c", "d"] {
            catalog
                .admit(owner, &keys, &vec![20.0; keys.len()], 0.0)
                .unwrap();
        }
        catalog
            .admit("padding", &keys[..5_000], &vec![20.0; 5_000], 0.0)
            .unwrap();
        let bytes = catalog.payload_bytes();
        let context = VirtualMeasurementContext {
            width: 400.0,
            scale: 1.0,
            layout_revision: 1,
        };
        for measurements in [
            vec![(0, f32::NAN)],
            vec![(0, -1.0)],
            vec![(10_000, 20.0)],
            vec![(0, 20.0), (0, 30.0)],
        ] {
            assert!(
                catalog
                    .measure("rows", original.revision(), context, &measurements)
                    .is_err()
            );
        }
        for invalid in [
            VirtualMeasurementContext {
                width: 0.0,
                ..context
            },
            VirtualMeasurementContext {
                scale: f32::INFINITY,
                ..context
            },
        ] {
            assert!(
                catalog
                    .measure("rows", original.revision(), invalid, &[])
                    .is_err()
            );
        }
        assert!(
            catalog
                .measure("other", original.revision(), context, &[])
                .is_err()
        );
        assert!(
            catalog
                .measure("rows", original.revision() + 1, context, &[])
                .is_err()
        );
        let measurements: Vec<_> = (0..10_000).map(|row| (row, 30.0)).collect();
        assert!(
            catalog
                .measure("rows", original.revision(), context, &measurements)
                .unwrap_err()
                .contains("budget")
        );
        assert_eq!(catalog.payload_bytes(), bytes);
        assert!(Arc::ptr_eq(
            &original,
            &catalog.resolve("rows", original.revision()).unwrap()
        ));
    }

    #[test]
    fn payload_budget_and_revision_exhaustion_reject_without_partial_admission() {
        let keys: Vec<_> = (0..MAX_SOURCE_ROWS)
            .map(|index| format!("{index:0>128}"))
            .collect();
        let heights = vec![20.0; keys.len()];
        let mut catalog = VirtualSourceCatalog::default();
        for index in 0..4 {
            catalog
                .admit(&index.to_string(), &keys, &heights, 0.0)
                .unwrap();
        }
        let before = (
            catalog.logical_rows(),
            catalog.payload_bytes(),
            catalog.source_count(),
        );
        assert!(
            catalog
                .admit("excess", &keys, &heights, 0.0)
                .unwrap_err()
                .contains("payload budget")
        );
        assert_eq!(
            (
                catalog.logical_rows(),
                catalog.payload_bytes(),
                catalog.source_count()
            ),
            before
        );
        assert!(catalog.retire("0"));
        catalog.admit("replacement", &keys, &heights, 0.0).unwrap();
        assert_eq!(catalog.payload_bytes(), before.1);
        let counter = AtomicU64::new(MAX_REVISION);
        assert_eq!(allocate_revision(&counter).unwrap(), MAX_REVISION);
        assert!(allocate_revision(&counter).is_err());
        assert!(allocate_revision(&counter).is_err());
    }

    #[test]
    fn source_queries_share_geometry_and_keep_offscreen_key_authority() {
        for count in [100usize, 1_000, 10_000] {
            let keys: Vec<_> = (0..count).map(|index| format!("row-{index}")).collect();
            let heights: Vec<_> = (0..count)
                .map(|index| if index % 2 == 0 { 20.0 } else { 70.0 })
                .collect();
            let mut catalog = VirtualSourceCatalog::default();
            let source = catalog.admit("native/rows", &keys, &heights, 5.0).unwrap();
            let snapshot = catalog.clone();
            for offset in [0.0, 400.0, 1_000_000.0] {
                let resolved = snapshot.resolve("native/rows", source.revision()).unwrap();
                assert!(Arc::ptr_eq(&source, &resolved));
                assert_eq!(
                    resolved.geometry().window(offset, 100.0, 40.0),
                    twinkle::VirtualWindow::from_heights(&heights, 5.0, offset, 100.0, 40.0)
                );
                assert_eq!(resolved.key(count - 1), Some(keys[count - 1].as_str()));
                assert_eq!(resolved.ordinal(&keys[count - 1]), Some(count - 1));
            }
            assert!(catalog.resolve("other/rows", source.revision()).is_err());
            let mut other_mount = VirtualSourceCatalog::default();
            other_mount
                .admit("native/rows", &keys, &heights, 5.0)
                .unwrap();
            assert!(
                other_mount
                    .resolve("native/rows", source.revision())
                    .is_err()
            );
            assert!(catalog.retire("native/rows"));
            assert!(!catalog.retire("native/rows"));
            assert!(catalog.resolve("native/rows", source.revision()).is_err());
            assert_eq!(
                (
                    catalog.source_count(),
                    catalog.logical_rows(),
                    catalog.payload_bytes()
                ),
                (0, 0, 0)
            );
            // An in-flight admitted generation remains readable, but cannot be
            // resolved through the retired catalog for fresh input.
            assert_eq!(source.key(0), Some("row-0"));
            let replacement = catalog.admit("native/rows", &keys, &heights, 5.0).unwrap();
            assert_ne!(replacement.revision(), source.revision());
            assert_ne!(replacement.identity(), source.identity());
            assert!(catalog.resolve("native/rows", source.revision()).is_err());
        }
    }

    #[test]
    fn invalid_replacements_are_atomic_and_new_revisions_revoke_old_references() {
        let mut catalog = VirtualSourceCatalog::default();
        let source = catalog
            .admit("rows", &["a".into(), "b".into()], &[20.0, 40.0], 0.0)
            .unwrap();
        let bytes = catalog.payload_bytes();
        for (keys, heights, gap) in [
            (vec!["a".into(), "a".into()], vec![20.0, 40.0], 0.0),
            (vec!["a".into()], vec![20.0, 40.0], 0.0),
            (vec!["a".into()], vec![f32::NAN], 0.0),
            (vec!["a".into()], vec![20.0], -1.0),
            (vec!["".into()], vec![20.0], 0.0),
        ] {
            assert!(catalog.admit("rows", &keys, &heights, gap).is_err());
            assert!(Arc::ptr_eq(
                &source,
                &catalog.resolve("rows", source.revision()).unwrap()
            ));
            assert_eq!(catalog.payload_bytes(), bytes);
        }
        let replacement = catalog
            .admit("rows", &["b".into(), "a".into()], &[40.0, 20.0], 0.0)
            .unwrap();
        assert_eq!(replacement.identity(), source.identity());
        assert!(catalog.resolve("rows", source.revision()).is_err());
        assert_eq!(replacement.ordinal("a"), Some(1));
        assert_eq!(source.ordinal("a"), Some(0));
    }

    #[test]
    fn catalog_row_and_source_limits_release_on_retirement() {
        let keys: Vec<_> = (0..MAX_SOURCE_ROWS)
            .map(|index| index.to_string())
            .collect();
        let heights = vec![20.0; keys.len()];
        let mut catalog = VirtualSourceCatalog::default();
        for index in 0..10 {
            catalog
                .admit(&index.to_string(), &keys, &heights, 0.0)
                .unwrap();
        }
        assert_eq!(catalog.logical_rows(), MAX_TOTAL_ROWS);
        assert!(
            catalog
                .admit("excess", &["a".into()], &[20.0], 0.0)
                .is_err()
        );
        catalog.retire("0");
        catalog.admit("replacement", &keys, &heights, 0.0).unwrap();
        let mut catalog = VirtualSourceCatalog::default();
        for index in 0..MAX_SOURCES {
            catalog.admit(&index.to_string(), &[], &[], 0.0).unwrap();
        }
        assert!(catalog.admit("excess", &[], &[], 0.0).is_err());
        catalog.retire("0");
        catalog.admit("replacement", &[], &[], 0.0).unwrap();
    }

    #[test]
    fn source_key_byte_boundary_preserves_native_inventory_identity() {
        let mut catalog = VirtualSourceCatalog::default();
        let keys = ["a".repeat(512), "é".repeat(256)];
        let accepted = catalog.admit("rows", &keys, &[20.0, 30.0], 0.0).unwrap();
        assert_eq!(accepted.key(0), Some(keys[0].as_str()));
        assert_eq!(accepted.ordinal(&keys[1]), Some(1));
        for invalid in ["a".repeat(513), "é".repeat(257)] {
            assert!(catalog.admit("rows", &[invalid], &[20.0], 0.0).is_err());
        }
        assert_eq!(accepted.key(1), Some(keys[1].as_str()));
        assert!(Arc::ptr_eq(
            &accepted,
            &catalog.resolve("rows", accepted.revision()).unwrap()
        ));
        assert_eq!(catalog.logical_rows(), 2);
    }
}
