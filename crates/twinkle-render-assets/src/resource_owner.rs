//! Process-wide accounting for Twinkle-owned font resources.

use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DependencyOwnerKind {
    CosmicTextFontSystem,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DependencyOwnerDiagnostics {
    pub active_owners: usize,
    pub peak_owners: usize,
}

static ACTIVE_FONT_SYSTEMS: AtomicUsize = AtomicUsize::new(0);
static PEAK_FONT_SYSTEMS: AtomicUsize = AtomicUsize::new(0);

/// Held by the process-wide font owner; never cloned or acquired by consumers.
#[derive(Debug)]
pub(crate) struct DependencyOwnerToken;

impl DependencyOwnerToken {
    pub(crate) fn new_cosmic_text_font_system() -> Self {
        let active = ACTIVE_FONT_SYSTEMS.fetch_add(1, Ordering::Relaxed) + 1;
        PEAK_FONT_SYSTEMS.fetch_max(active, Ordering::Relaxed);
        Self
    }
}

impl Drop for DependencyOwnerToken {
    fn drop(&mut self) {
        let previous = ACTIVE_FONT_SYSTEMS.fetch_sub(1, Ordering::Relaxed);
        debug_assert!(previous > 0, "font owner counter underflow");
    }
}

#[must_use]
pub fn dependency_owner_diagnostics(kind: DependencyOwnerKind) -> DependencyOwnerDiagnostics {
    match kind {
        DependencyOwnerKind::CosmicTextFontSystem => DependencyOwnerDiagnostics {
            active_owners: ACTIVE_FONT_SYSTEMS.load(Ordering::Relaxed),
            peak_owners: PEAK_FONT_SYSTEMS.load(Ordering::Relaxed),
        },
    }
}
