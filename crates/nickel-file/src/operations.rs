//! Platform-neutral policy for file rename and transfer operations.
//!
//! This module deliberately produces typed effects. Native filesystem,
//! clipboard, and drag adapters execute those effects only after revalidating
//! the provider-owned identities and capabilities represented here.

use std::{
    ffi::OsString,
    path::{Component, Path, PathBuf},
};

use crate::FileIdentity;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransferIntent {
    Copy,
    Move,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConflictPolicy {
    Ask,
    KeepBoth,
    Skip,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ItemCapabilities {
    pub readable: bool,
    pub removable: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransferSource {
    pub provider: String,
    pub identity: FileIdentity,
    pub path: PathBuf,
    pub capabilities: ItemCapabilities,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClipboardOffer {
    pub intent: TransferIntent,
    pub sources: Vec<TransferSource>,
}

impl ClipboardOffer {
    pub fn new(
        intent: TransferIntent,
        sources: Vec<TransferSource>,
    ) -> Result<Self, OperationError> {
        if sources.is_empty() {
            return Err(OperationError::EmptySelection);
        }
        if sources.len() > MAX_TRANSFER_ITEMS {
            return Err(OperationError::TooManyItems);
        }
        if sources.iter().any(|source| !source.capabilities.readable) {
            return Err(OperationError::UnsupportedSource);
        }
        if intent == TransferIntent::Move
            && sources.iter().any(|source| !source.capabilities.removable)
        {
            return Err(OperationError::UnsupportedSource);
        }
        Ok(Self { intent, sources })
    }
}

pub const MAX_TRANSFER_ITEMS: usize = 4096;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OperationError {
    EmptySelection,
    TooManyItems,
    UnsupportedSource,
    DestinationNotWritable,
    EmptyName,
    DotName,
    ContainsSeparator,
    AbsoluteName,
    NameConflict(PathBuf),
    RecursiveDirectory(PathBuf),
    SourceDisappeared(PathBuf),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OperationEffect {
    Rename {
        identity: FileIdentity,
        from: PathBuf,
        to: PathBuf,
    },
    Transfer {
        intent: TransferIntent,
        sources: Vec<TransferSource>,
        destination: PathBuf,
        conflicts: ConflictPolicy,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OperationState {
    Idle,
    Queued(OperationEffect),
    Running {
        completed: usize,
        total: usize,
    },
    Conflict {
        path: PathBuf,
    },
    Completed {
        affected: Vec<PathBuf>,
    },
    PartiallyCompleted {
        affected: Vec<PathBuf>,
        failed: Vec<(PathBuf, OperationError)>,
    },
    Cancelled,
    Failed(OperationError),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperationQueue {
    pub state: OperationState,
    cancel_requested: bool,
}

impl Default for OperationQueue {
    fn default() -> Self {
        Self {
            state: OperationState::Idle,
            cancel_requested: false,
        }
    }
}

impl OperationQueue {
    pub fn queue(&mut self, effect: OperationEffect) -> bool {
        if matches!(
            self.state,
            OperationState::Queued(_)
                | OperationState::Running { .. }
                | OperationState::Conflict { .. }
        ) {
            return false;
        }
        self.cancel_requested = false;
        self.state = OperationState::Queued(effect);
        true
    }

    pub fn begin(&mut self) -> Option<OperationEffect> {
        let OperationState::Queued(effect) = &self.state else {
            return None;
        };
        let effect = effect.clone();
        let total = match &effect {
            OperationEffect::Rename { .. } => 1,
            OperationEffect::Transfer { sources, .. } => sources.len(),
        };
        self.state = OperationState::Running {
            completed: 0,
            total,
        };
        Some(effect)
    }

    pub fn request_cancel(&mut self) {
        if matches!(
            self.state,
            OperationState::Queued(_)
                | OperationState::Running { .. }
                | OperationState::Conflict { .. }
        ) {
            self.cancel_requested = true;
        }
    }

    pub fn cancellation_requested(&self) -> bool {
        self.cancel_requested
    }

    pub fn record_progress(&mut self, completed: usize) {
        if let OperationState::Running {
            completed: done,
            total,
        } = &mut self.state
        {
            *done = completed.min(*total);
        }
    }

    pub fn finish(&mut self, affected: Vec<PathBuf>, failed: Vec<(PathBuf, OperationError)>) {
        self.state = if self.cancel_requested {
            OperationState::Cancelled
        } else if failed.is_empty() {
            OperationState::Completed { affected }
        } else if affected.is_empty() {
            OperationState::Failed(failed.into_iter().next().expect("nonempty failures").1)
        } else {
            OperationState::PartiallyCompleted { affected, failed }
        };
        self.cancel_requested = false;
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenameEditor {
    pub identity: FileIdentity,
    pub original_path: PathBuf,
    pub text: String,
    pub selection: std::ops::Range<usize>,
    pub error: Option<OperationError>,
}

impl RenameEditor {
    pub fn begin(identity: FileIdentity, path: PathBuf) -> Self {
        let text = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let selection_end = basename_selection_end(&text);
        Self {
            identity,
            original_path: path,
            text,
            selection: 0..selection_end,
            error: None,
        }
    }

    pub fn commit(
        &mut self,
        existing_names: impl IntoIterator<Item = OsString>,
    ) -> Result<Option<OperationEffect>, OperationError> {
        let name = validate_name(&self.text)?;
        if self.original_path.file_name() == Some(name.as_os_str()) {
            return Ok(None);
        }
        if existing_names.into_iter().any(|existing| existing == name) {
            let conflict = self.original_path.with_file_name(&name);
            self.error = Some(OperationError::NameConflict(conflict.clone()));
            return Err(OperationError::NameConflict(conflict));
        }
        Ok(Some(OperationEffect::Rename {
            identity: self.identity,
            from: self.original_path.clone(),
            to: self.original_path.with_file_name(name),
        }))
    }
}

pub fn plan_paste(
    offer: &ClipboardOffer,
    destination_provider: &str,
    destination: &Path,
    writable: bool,
    conflicts: ConflictPolicy,
) -> Result<OperationEffect, OperationError> {
    if !writable {
        return Err(OperationError::DestinationNotWritable);
    }
    for source in &offer.sources {
        if source.path == destination || destination.starts_with(&source.path) {
            return Err(OperationError::RecursiveDirectory(source.path.clone()));
        }
    }
    let intent = if offer.intent == TransferIntent::Move
        && offer
            .sources
            .iter()
            .all(|source| source.provider == destination_provider && source.capabilities.removable)
    {
        TransferIntent::Move
    } else {
        TransferIntent::Copy
    };
    Ok(OperationEffect::Transfer {
        intent,
        sources: offer.sources.clone(),
        destination: destination.to_path_buf(),
        conflicts,
    })
}

pub fn validate_name(text: &str) -> Result<OsString, OperationError> {
    if text.is_empty() {
        return Err(OperationError::EmptyName);
    }
    if text == "." || text == ".." {
        return Err(OperationError::DotName);
    }
    let path = Path::new(text);
    if path.is_absolute() {
        return Err(OperationError::AbsoluteName);
    }
    if text.contains(['/', '\\']) {
        return Err(OperationError::ContainsSeparator);
    }
    if path
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(OperationError::ContainsSeparator);
    }
    #[cfg(windows)]
    if text.ends_with([' ', '.']) || is_windows_reserved(text) {
        return Err(OperationError::UnsupportedSource);
    }
    Ok(OsString::from(text))
}

fn basename_selection_end(name: &str) -> usize {
    if name.starts_with('.') && !name[1..].contains('.') {
        return name.len();
    }
    name.rfind('.')
        .filter(|index| *index > 0)
        .unwrap_or(name.len())
}

#[cfg(windows)]
fn is_windows_reserved(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || stem
            .strip_prefix("COM")
            .is_some_and(|n| matches!(n, "1"..="9"))
        || stem
            .strip_prefix("LPT")
            .is_some_and(|n| matches!(n, "1"..="9"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(path: &str, provider: &str, removable: bool) -> TransferSource {
        TransferSource {
            provider: provider.into(),
            identity: FileIdentity(1, 2),
            path: path.into(),
            capabilities: ItemCapabilities {
                readable: true,
                removable,
            },
        }
    }

    #[test]
    fn rename_selects_basename_and_emits_identity_guarded_effect() {
        let mut editor = RenameEditor::begin(FileIdentity(3, 4), "/tmp/report.final.txt".into());
        assert_eq!(&editor.text[editor.selection.clone()], "report.final");
        editor.text = "summary.txt".into();
        assert_eq!(
            editor.commit([OsString::from("other")]),
            Ok(Some(OperationEffect::Rename {
                identity: FileIdentity(3, 4),
                from: "/tmp/report.final.txt".into(),
                to: "/tmp/summary.txt".into()
            }))
        );
    }

    #[test]
    fn rename_rejects_ambiguous_and_conflicting_names_but_unchanged_is_noop() {
        for (name, error) in [
            ("", OperationError::EmptyName),
            ("..", OperationError::DotName),
            ("a/b", OperationError::ContainsSeparator),
        ] {
            assert_eq!(validate_name(name), Err(error));
        }
        let mut editor = RenameEditor::begin(FileIdentity(1, 1), "/tmp/a.txt".into());
        assert_eq!(editor.commit([]), Ok(None));
        editor.text = "b.txt".into();
        assert!(matches!(
            editor.commit([OsString::from("b.txt")]),
            Err(OperationError::NameConflict(_))
        ));
    }

    #[test]
    fn clipboard_preserves_order_and_distinguishes_cut() {
        let sources = vec![source("/a", "local", true), source("/b", "local", true)];
        let offer = ClipboardOffer::new(TransferIntent::Move, sources.clone()).unwrap();
        assert_eq!(offer.sources, sources);
        assert_eq!(offer.intent, TransferIntent::Move);
    }

    #[test]
    fn cross_provider_move_degrades_to_safe_copy() {
        let offer =
            ClipboardOffer::new(TransferIntent::Move, vec![source("/a", "one", true)]).unwrap();
        let effect = plan_paste(
            &offer,
            "two",
            Path::new("/target"),
            true,
            ConflictPolicy::Ask,
        )
        .unwrap();
        assert!(matches!(
            effect,
            OperationEffect::Transfer {
                intent: TransferIntent::Copy,
                ..
            }
        ));
    }

    #[test]
    fn recursive_and_read_only_destinations_are_rejected() {
        let offer =
            ClipboardOffer::new(TransferIntent::Copy, vec![source("/a", "local", false)]).unwrap();
        assert_eq!(
            plan_paste(
                &offer,
                "local",
                Path::new("/a/child"),
                true,
                ConflictPolicy::Ask
            ),
            Err(OperationError::RecursiveDirectory("/a".into()))
        );
        assert_eq!(
            plan_paste(
                &offer,
                "local",
                Path::new("/target"),
                false,
                ConflictPolicy::Ask
            ),
            Err(OperationError::DestinationNotWritable)
        );
    }

    #[test]
    fn queue_has_explicit_progress_partial_failure_and_cancellation() {
        let offer = ClipboardOffer::new(
            TransferIntent::Copy,
            vec![source("/a", "local", false), source("/b", "local", false)],
        )
        .unwrap();
        let effect = plan_paste(
            &offer,
            "local",
            Path::new("/target"),
            true,
            ConflictPolicy::Ask,
        )
        .unwrap();
        let mut queue = OperationQueue::default();
        assert!(queue.queue(effect.clone()));
        assert!(!queue.queue(effect));
        assert!(queue.begin().is_some());
        queue.record_progress(1);
        assert_eq!(
            queue.state,
            OperationState::Running {
                completed: 1,
                total: 2
            }
        );
        queue.finish(
            vec!["/target/a".into()],
            vec![("/b".into(), OperationError::SourceDisappeared("/b".into()))],
        );
        assert!(matches!(
            queue.state,
            OperationState::PartiallyCompleted { .. }
        ));

        assert!(queue.queue(OperationEffect::Rename {
            identity: FileIdentity(1, 2),
            from: "/a".into(),
            to: "/b".into()
        }));
        queue.request_cancel();
        queue.finish(Vec::new(), Vec::new());
        assert_eq!(queue.state, OperationState::Cancelled);
    }
}
