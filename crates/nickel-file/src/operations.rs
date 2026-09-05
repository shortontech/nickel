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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DragAction {
    Copy,
    Move,
    Link,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DragOffer {
    pub sources: Vec<TransferSource>,
    pub actions: Vec<DragAction>,
}

impl DragOffer {
    pub fn bounded(sources: Vec<TransferSource>) -> Result<Self, OperationError> {
        let clipboard = ClipboardOffer::new(TransferIntent::Copy, sources)?;
        let mut actions = vec![DragAction::Copy];
        if clipboard
            .sources
            .iter()
            .all(|source| source.capabilities.removable)
        {
            actions.push(DragAction::Move);
        }
        Ok(Self {
            sources: clipboard.sources,
            actions,
        })
    }

    pub fn negotiate(
        &self,
        requested: Option<DragAction>,
        same_provider: bool,
    ) -> Option<DragAction> {
        requested
            .filter(|action| self.actions.contains(action))
            .or_else(|| {
                let conventional = if same_provider {
                    DragAction::Move
                } else {
                    DragAction::Copy
                };
                self.actions.contains(&conventional).then_some(conventional)
            })
    }
}

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
        delete_after_verified_copy: bool,
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
    let same_provider_move = offer.intent == TransferIntent::Move
        && offer
            .sources
            .iter()
            .all(|source| source.provider == destination_provider && source.capabilities.removable);
    let intent = if same_provider_move {
        TransferIntent::Move
    } else {
        TransferIntent::Copy
    };
    Ok(OperationEffect::Transfer {
        intent,
        delete_after_verified_copy: offer.intent == TransferIntent::Move && !same_provider_move,
        sources: offer.sources.clone(),
        destination: destination.to_path_buf(),
        conflicts,
    })
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TransferReport {
    pub affected: Vec<PathBuf>,
    pub failed: Vec<(PathBuf, String)>,
    pub cancelled: bool,
}

pub fn execute_local_transfer(
    effect: &OperationEffect,
    cancelled: &std::sync::atomic::AtomicBool,
    mut progress: impl FnMut(usize, usize),
) -> TransferReport {
    use std::sync::atomic::Ordering;
    let OperationEffect::Transfer {
        intent,
        delete_after_verified_copy,
        sources,
        destination,
        conflicts,
    } = effect
    else {
        return TransferReport::default();
    };
    let mut report = TransferReport::default();
    for (index, source) in sources.iter().enumerate() {
        if cancelled.load(Ordering::Relaxed) {
            report.cancelled = true;
            break;
        }
        let Some(name) = source.path.file_name() else {
            report.failed.push((
                source.path.clone(),
                "source does not have a transferable file name".into(),
            ));
            progress(index + 1, sources.len());
            continue;
        };
        // Local selections carry a provider-owned filesystem identity. Check it
        // immediately before mutation so a removed/replaced path cannot redirect
        // a stale copy, move, paste, or drag transaction. Native clipboard/drag
        // offers use their adapter's synthetic authority and are revalidated by
        // the filesystem operation itself.
        if source.provider == "local"
            && crate::file_identity(&source.path).ok() != Some(source.identity)
        {
            report
                .failed
                .push((source.path.clone(), "source changed or disappeared".into()));
            progress(index + 1, sources.len());
            continue;
        }
        let mut target = destination.join(name);
        if target.exists() {
            match conflicts {
                ConflictPolicy::Ask => {
                    report.failed.push((
                        source.path.clone(),
                        std::io::Error::new(
                            std::io::ErrorKind::AlreadyExists,
                            "destination exists",
                        )
                        .to_string(),
                    ));
                    progress(index + 1, sources.len());
                    continue;
                }
                ConflictPolicy::Skip => {
                    progress(index + 1, sources.len());
                    continue;
                }
                ConflictPolicy::KeepBoth => target = unused_copy_name(&target),
            }
        }
        let result = if *intent == TransferIntent::Move {
            match std::fs::rename(&source.path, &target) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::CrossesDevices => {
                    copy_verify_and_maybe_delete(&source.path, &target, true)
                }
                Err(error) => Err(error),
            }
        } else {
            copy_verify_and_maybe_delete(&source.path, &target, *delete_after_verified_copy)
        };
        match result {
            Ok(()) => report.affected.push(target),
            Err(error) => report.failed.push((source.path.clone(), error.to_string())),
        }
        progress(index + 1, sources.len());
    }
    report
}

fn copy_verify_and_maybe_delete(
    source: &Path,
    target: &Path,
    delete_after_verified_copy: bool,
) -> std::io::Result<()> {
    if source.is_dir() {
        copy_directory(source, target)?;
    } else {
        std::fs::copy(source, target)?;
    }
    verify_copy(source, target)?;
    if delete_after_verified_copy {
        if source.is_dir() {
            std::fs::remove_dir_all(source)
        } else {
            std::fs::remove_file(source)
        }
    } else {
        Ok(())
    }
}

fn unused_copy_name(original: &Path) -> PathBuf {
    let parent = original.parent().unwrap_or_else(|| Path::new(""));
    let name = original
        .file_name()
        .expect("a transfer target always has a file name");
    let stem = original.file_stem().unwrap_or(name).to_string_lossy();
    let extension = original.extension().map(|value| value.to_string_lossy());
    (2_u32..)
        .map(|number| {
            let name = extension.as_ref().map_or_else(
                || format!("{stem} ({number})"),
                |extension| format!("{stem} ({number}).{extension}"),
            );
            parent.join(name)
        })
        .find(|candidate| !candidate.exists())
        .expect("an unused collision suffix exists")
}

fn verify_copy(source: &Path, destination: &Path) -> std::io::Result<()> {
    let source_meta = std::fs::metadata(source)?;
    let destination_meta = std::fs::metadata(destination)?;
    if source_meta.is_file() && source_meta.len() != destination_meta.len() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "copied length differs",
        ));
    }
    if source_meta.is_dir() {
        for entry in std::fs::read_dir(source)? {
            let entry = entry?;
            verify_copy(&entry.path(), &destination.join(entry.file_name()))?;
        }
    }
    Ok(())
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

pub fn copy_directory(source: &Path, destination: &Path) -> std::io::Result<()> {
    std::fs::create_dir(destination)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let target = destination.join(entry.file_name());
        if kind.is_symlink() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "symbolic links require provider negotiation",
            ));
        } else if kind.is_dir() {
            copy_directory(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

#[cfg(windows)]
fn is_windows_reserved(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || stem
            .strip_prefix("COM")
            .is_some_and(|n| n.len() == 1 && n.as_bytes()[0].is_ascii_digit() && n != "0")
        || stem
            .strip_prefix("LPT")
            .is_some_and(|n| n.len() == 1 && n.as_bytes()[0].is_ascii_digit() && n != "0")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(path: &str, provider: &str, removable: bool) -> TransferSource {
        TransferSource {
            provider: provider.into(),
            identity: crate::file_identity(Path::new(path)).unwrap_or(FileIdentity(1, 2)),
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

    #[test]
    fn cross_provider_move_deletes_only_after_verified_copy() {
        let source_dir = tempfile::tempdir().unwrap();
        let destination = tempfile::tempdir().unwrap();
        let source_path = source_dir.path().join("payload");
        std::fs::write(&source_path, b"verified bytes").unwrap();
        let offer = ClipboardOffer::new(
            TransferIntent::Move,
            vec![source(source_path.to_str().unwrap(), "remote", true)],
        )
        .unwrap();
        let effect = plan_paste(
            &offer,
            "local",
            destination.path(),
            true,
            ConflictPolicy::Ask,
        )
        .unwrap();
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let report = execute_local_transfer(&effect, &cancel, |_, _| {});
        assert!(report.failed.is_empty());
        assert!(!source_path.exists());
        assert_eq!(
            std::fs::read(destination.path().join("payload")).unwrap(),
            b"verified bytes"
        );
    }

    #[test]
    fn local_transfer_applies_keep_both_and_skip_conflict_policies() {
        let source_dir = tempfile::tempdir().unwrap();
        let destination = tempfile::tempdir().unwrap();
        let source_path = source_dir.path().join("report.txt");
        std::fs::write(&source_path, b"new").unwrap();
        std::fs::write(destination.path().join("report.txt"), b"old").unwrap();
        let offer = ClipboardOffer::new(
            TransferIntent::Copy,
            vec![source(source_path.to_str().unwrap(), "local", false)],
        )
        .unwrap();
        let keep_both = plan_paste(
            &offer,
            "local",
            destination.path(),
            true,
            ConflictPolicy::KeepBoth,
        )
        .unwrap();
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let report = execute_local_transfer(&keep_both, &cancel, |_, _| {});
        assert_eq!(report.affected, [destination.path().join("report (2).txt")]);
        assert!(report.failed.is_empty());
        assert_eq!(
            std::fs::read(destination.path().join("report.txt")).unwrap(),
            b"old"
        );

        let skip = plan_paste(
            &offer,
            "local",
            destination.path(),
            true,
            ConflictPolicy::Skip,
        )
        .unwrap();
        let report = execute_local_transfer(&skip, &cancel, |_, _| {});
        assert!(report.affected.is_empty());
        assert!(report.failed.is_empty());
        assert!(!destination.path().join("report (3).txt").exists());
    }

    #[test]
    fn local_transfer_rejects_a_replaced_source_identity() {
        let source_dir = tempfile::tempdir().unwrap();
        let destination = tempfile::tempdir().unwrap();
        let path = source_dir.path().join("report.txt");
        std::fs::write(&path, b"original").unwrap();
        let source = source(path.to_str().unwrap(), "local", true);
        std::fs::rename(&path, source_dir.path().join("old-report.txt")).unwrap();
        std::fs::write(&path, b"replacement").unwrap();
        let effect = plan_paste(
            &ClipboardOffer::new(TransferIntent::Move, vec![source]).unwrap(),
            "local",
            destination.path(),
            true,
            ConflictPolicy::Ask,
        )
        .unwrap();

        let report = execute_local_transfer(
            &effect,
            &std::sync::atomic::AtomicBool::new(false),
            |_, _| {},
        );
        assert!(report.affected.is_empty());
        assert_eq!(report.failed.len(), 1);
        assert!(path.exists(), "replacement must never be moved");
        assert!(!destination.path().join("report.txt").exists());
    }

    #[test]
    fn drag_offer_is_bounded_and_negotiates_provider_default_and_modifier() {
        let offer = DragOffer::bounded(vec![
            source("/a", "local", true),
            source("/b", "local", true),
        ])
        .unwrap();
        assert_eq!(offer.negotiate(None, true), Some(DragAction::Move));
        assert_eq!(offer.negotiate(None, false), Some(DragAction::Copy));
        assert_eq!(
            offer.negotiate(Some(DragAction::Copy), true),
            Some(DragAction::Copy)
        );
        assert_eq!(
            offer.negotiate(Some(DragAction::Link), true),
            Some(DragAction::Move)
        );
    }
}
