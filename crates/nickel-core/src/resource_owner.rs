//! Process-wide lifecycle accounting for dependency-owned resource containers.
//!
//! These counters describe Nickel owner instances only. They deliberately do
//! not infer dependency-internal entry counts, retained bytes, or activity.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::{error::Error, fmt};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DependencyOwnerKind {
    CosmicTextFontSystem,
    SmithayRenderer,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DependencyOwnerDiagnostics {
    pub active_owners: usize,
    pub peak_owners: usize,
}

struct OwnerCounters {
    active: AtomicUsize,
    peak: AtomicUsize,
}

impl OwnerCounters {
    const fn new() -> Self {
        Self {
            active: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
        }
    }
}

static COSMIC_TEXT_FONT_SYSTEMS: OwnerCounters = OwnerCounters::new();
static SMITHAY_RENDERERS: OwnerCounters = OwnerCounters::new();
const MAX_SMITHAY_RENDERER_OWNERS: usize = 1;

fn counters(kind: DependencyOwnerKind) -> &'static OwnerCounters {
    match kind {
        DependencyOwnerKind::CosmicTextFontSystem => &COSMIC_TEXT_FONT_SYSTEMS,
        DependencyOwnerKind::SmithayRenderer => &SMITHAY_RENDERERS,
    }
}

/// A non-cloneable token held beside one dependency-owned resource container.
#[derive(Debug)]
pub struct DependencyOwnerToken {
    kind: DependencyOwnerKind,
}

impl DependencyOwnerToken {
    #[must_use]
    pub fn new_cosmic_text_font_system() -> Self {
        let kind = DependencyOwnerKind::CosmicTextFontSystem;
        let counters = counters(kind);
        let active = counters.active.fetch_add(1, Ordering::Relaxed) + 1;
        counters.peak.fetch_max(active, Ordering::Relaxed);
        Self { kind }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SmithayRendererAdmissionError;

impl fmt::Display for SmithayRendererAdmissionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a Smithay renderer backend is already active")
    }
}

impl Error for SmithayRendererAdmissionError {}

/// Acquires Nickel's single process-wide Smithay renderer-backend owner.
///
/// Smithay may create dependency-internal renderers below a native `GpuManager`,
/// but Nickel admits only one top-level backend (`GpuManager` or winit renderer)
/// at a time. Dropping the returned token releases that admission.
pub fn try_acquire_smithay_renderer_owner()
-> Result<DependencyOwnerToken, SmithayRendererAdmissionError> {
    let counters = counters(DependencyOwnerKind::SmithayRenderer);
    counters
        .active
        .compare_exchange(
            0,
            MAX_SMITHAY_RENDERER_OWNERS,
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .map_err(|_| SmithayRendererAdmissionError)?;
    counters
        .peak
        .fetch_max(MAX_SMITHAY_RENDERER_OWNERS, Ordering::Relaxed);
    Ok(DependencyOwnerToken {
        kind: DependencyOwnerKind::SmithayRenderer,
    })
}

impl Drop for DependencyOwnerToken {
    fn drop(&mut self) {
        let previous = counters(self.kind).active.fetch_sub(1, Ordering::Relaxed);
        debug_assert!(previous > 0, "dependency owner counter underflow");
    }
}

#[must_use]
pub fn dependency_owner_diagnostics(kind: DependencyOwnerKind) -> DependencyOwnerDiagnostics {
    let counters = counters(kind);
    DependencyOwnerDiagnostics {
        active_owners: counters.active.load(Ordering::Relaxed),
        peak_owners: counters.peak.load(Ordering::Relaxed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smithay_admission_is_singleton_and_released_on_drop() {
        let before = dependency_owner_diagnostics(DependencyOwnerKind::SmithayRenderer);
        for _ in 0..8 {
            let owner = try_acquire_smithay_renderer_owner().expect("first owner is admitted");
            assert_eq!(
                dependency_owner_diagnostics(DependencyOwnerKind::SmithayRenderer).active_owners,
                1
            );
            assert!(matches!(
                try_acquire_smithay_renderer_owner(),
                Err(SmithayRendererAdmissionError)
            ));
            drop(owner);
        }
        let after = dependency_owner_diagnostics(DependencyOwnerKind::SmithayRenderer);
        assert_eq!(after.active_owners, before.active_owners);
        assert_eq!(after.peak_owners, 1);
    }
}
