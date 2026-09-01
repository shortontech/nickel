//! Process-wide lifecycle accounting for dependency-owned resource containers.
//!
//! These counters describe Nickel owner instances only. They deliberately do
//! not infer dependency-internal entry counts, retained bytes, or activity.

use std::sync::atomic::{AtomicUsize, Ordering};

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
    pub fn new(kind: DependencyOwnerKind) -> Self {
        let counters = counters(kind);
        let active = counters.active.fetch_add(1, Ordering::Relaxed) + 1;
        counters.peak.fetch_max(active, Ordering::Relaxed);
        Self { kind }
    }
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
    fn churn_releases_active_owners_and_retains_peak() {
        let before = dependency_owner_diagnostics(DependencyOwnerKind::SmithayRenderer);
        for _ in 0..8 {
            let owners = (0..4)
                .map(|_| DependencyOwnerToken::new(DependencyOwnerKind::SmithayRenderer))
                .collect::<Vec<_>>();
            assert_eq!(
                dependency_owner_diagnostics(DependencyOwnerKind::SmithayRenderer).active_owners,
                before.active_owners + owners.len()
            );
            drop(owners);
        }
        let after = dependency_owner_diagnostics(DependencyOwnerKind::SmithayRenderer);
        assert_eq!(after.active_owners, before.active_owners);
        assert!(after.peak_owners >= before.active_owners + 4);
    }
}
