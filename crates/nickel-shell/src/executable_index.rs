//! Bounded, background-only evidence about executables reachable through `PATH`.
//!
//! Heuristic classifications are deliberately observational. They must never
//! suppress the deferred terminal or grant additional launch authority.

use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::Read,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::{Arc, OnceLock, RwLock, mpsc},
    time::{Duration, Instant, SystemTime},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ExecutableClass {
    LikelyGraphical,
    LikelyTerminal,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum EvidenceReason {
    GuiLibrary,
    TerminalLibrary,
    ShellInterpreter,
    LanguageInterpreter,
    StaticOrUnresolvedElf,
    Malformed,
    InspectionBudget,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct FileIdentity {
    device: u64,
    inode: u64,
    size: u64,
    mode: u32,
    modified_nanos: u128,
    changed_nanos: i128,
}

impl FileIdentity {
    fn from_metadata(metadata: &std::fs::Metadata) -> Self {
        let modified_nanos = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map_or(0, |duration| duration.as_nanos());
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            size: metadata.len(),
            mode: metadata.mode(),
            modified_nanos,
            changed_nanos: i128::from(metadata.ctime()) * 1_000_000_000
                + i128::from(metadata.ctime_nsec()),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ExecutableEvidence {
    pub class: ExecutableClass,
    pub confidence_percent: u8,
    pub generation: u64,
    pub resolved_path: PathBuf,
    pub reasons: Vec<EvidenceReason>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ScanProgress {
    pub generation: u64,
    pub directories_scanned: usize,
    pub entries_inspected: usize,
    pub commands_published: usize,
    pub retained_bytes_estimate: usize,
    pub evictions: usize,
    pub complete: bool,
}

#[derive(Clone, Copy, Debug)]
struct ScanBudgets {
    max_directories: usize,
    max_directory_entries: usize,
    max_commands: usize,
    max_aliases_per_file: usize,
    max_prefix_bytes: u64,
    max_elapsed: Duration,
}

impl Default for ScanBudgets {
    fn default() -> Self {
        Self {
            max_directories: 128,
            max_directory_entries: 8_192,
            max_commands: 4_096,
            max_aliases_per_file: 16,
            max_prefix_bytes: 1024 * 1024,
            max_elapsed: Duration::from_secs(2),
        }
    }
}

#[derive(Clone, Debug)]
struct IndexSnapshot {
    commands: HashMap<String, Arc<ExecutableEvidence>>,
    progress: ScanProgress,
}

impl Default for IndexSnapshot {
    fn default() -> Self {
        Self {
            commands: HashMap::new(),
            progress: ScanProgress {
                generation: 0,
                directories_scanned: 0,
                entries_inspected: 0,
                commands_published: 0,
                retained_bytes_estimate: 0,
                evictions: 0,
                complete: false,
            },
        }
    }
}

pub(crate) struct ExecutableIndex {
    snapshot: Arc<RwLock<Arc<IndexSnapshot>>>,
    refresh: mpsc::SyncSender<()>,
}

impl ExecutableIndex {
    fn start(path: Option<std::ffi::OsString>, budgets: ScanBudgets) -> Self {
        let snapshot = Arc::new(RwLock::new(Arc::new(IndexSnapshot::default())));
        let (refresh, receiver) = mpsc::sync_channel(1);
        let worker_snapshot = Arc::clone(&snapshot);
        let _ = std::thread::Builder::new()
            .name("nickel-executable-index".into())
            .spawn(move || scan_worker(path, budgets, worker_snapshot, receiver));
        Self { snapshot, refresh }
    }

    pub(crate) fn classify(&self, command: &str) -> Option<ExecutableEvidence> {
        let command = Path::new(command)
            .file_name()
            .and_then(|name| name.to_str())?;
        self.snapshot
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .commands
            .get(command)
            .map(|evidence| evidence.as_ref().clone())
    }

    #[allow(dead_code)]
    pub(crate) fn progress(&self) -> ScanProgress {
        self.snapshot
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .progress
    }

    #[allow(dead_code)]
    pub(crate) fn request_refresh(&self) {
        let _ = self.refresh.try_send(());
    }
}

pub(crate) fn global_executable_index() -> &'static ExecutableIndex {
    static INDEX: OnceLock<ExecutableIndex> = OnceLock::new();
    INDEX.get_or_init(|| ExecutableIndex::start(std::env::var_os("PATH"), ScanBudgets::default()))
}

fn scan_worker(
    path: Option<std::ffi::OsString>,
    budgets: ScanBudgets,
    snapshot: Arc<RwLock<Arc<IndexSnapshot>>>,
    receiver: mpsc::Receiver<()>,
) {
    let mut generation = 1_u64;
    loop {
        scan_path_generation(path.as_deref(), budgets, generation, &snapshot);
        generation = generation.saturating_add(1);
        match receiver.recv_timeout(Duration::from_secs(30)) {
            Ok(()) | Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}

fn publish(
    snapshot: &RwLock<Arc<IndexSnapshot>>,
    commands: &HashMap<String, Arc<ExecutableEvidence>>,
    mut progress: ScanProgress,
) {
    progress.commands_published = commands.len();
    progress.retained_bytes_estimate = commands
        .iter()
        .map(|(command, evidence)| {
            command.len()
                + evidence.resolved_path.as_os_str().len()
                + evidence.reasons.len() * std::mem::size_of::<EvidenceReason>()
                + std::mem::size_of::<ExecutableEvidence>()
        })
        .sum();
    *snapshot.write().unwrap_or_else(|error| error.into_inner()) = Arc::new(IndexSnapshot {
        commands: commands.clone(),
        progress,
    });
}

fn scan_path_generation(
    path: Option<&std::ffi::OsStr>,
    budgets: ScanBudgets,
    generation: u64,
    snapshot: &RwLock<Arc<IndexSnapshot>>,
) {
    let started = Instant::now();
    let mut commands = HashMap::new();
    let mut identities: HashMap<FileIdentity, Arc<ExecutableEvidence>> = HashMap::new();
    let mut alias_counts: HashMap<FileIdentity, usize> = HashMap::new();
    let mut seen_directories = HashSet::new();
    let mut progress = ScanProgress {
        generation,
        ..IndexSnapshot::default().progress
    };
    let directories = path.map(std::env::split_paths).into_iter().flatten();
    let mut complete = true;

    'directories: for (directory_index, directory) in directories.enumerate() {
        if directory_index >= budgets.max_directories || started.elapsed() >= budgets.max_elapsed {
            complete = false;
            break;
        }
        let canonical = std::fs::canonicalize(&directory).unwrap_or(directory);
        if !seen_directories.insert(canonical.clone()) {
            continue;
        }
        progress.directories_scanned += 1;
        let Ok(entries) = std::fs::read_dir(canonical) else {
            publish(snapshot, &commands, progress);
            continue;
        };
        for (entry_index, entry) in entries.enumerate() {
            if entry_index >= budgets.max_directory_entries {
                complete = false;
                break;
            }
            if commands.len() >= budgets.max_commands || started.elapsed() >= budgets.max_elapsed {
                complete = false;
                break 'directories;
            }
            progress.entries_inspected += 1;
            let Ok(entry) = entry else { continue };
            let Some(command) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if commands.contains_key(&command) {
                continue;
            }
            let path = entry.path();
            let Ok(before) = std::fs::metadata(&path) else {
                continue;
            };
            if !before.is_file() || before.mode() & 0o111 == 0 {
                continue;
            }
            let identity = FileIdentity::from_metadata(&before);
            let aliases = alias_counts.entry(identity.clone()).or_default();
            if *aliases >= budgets.max_aliases_per_file {
                progress.evictions += 1;
                continue;
            }
            let evidence = if let Some(evidence) = identities.get(&identity) {
                Arc::clone(evidence)
            } else {
                let Some(evidence) = inspect_executable(&path, &identity, generation, budgets)
                else {
                    continue;
                };
                let evidence = Arc::new(evidence);
                identities.insert(identity.clone(), Arc::clone(&evidence));
                evidence
            };
            *aliases += 1;
            commands.insert(command, evidence);
        }
        publish(snapshot, &commands, progress);
    }
    progress.complete = complete;
    publish(snapshot, &commands, progress);
}

fn inspect_executable(
    path: &Path,
    identity: &FileIdentity,
    generation: u64,
    budgets: ScanBudgets,
) -> Option<ExecutableEvidence> {
    let mut file = File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.by_ref()
        .take(budgets.max_prefix_bytes)
        .read_to_end(&mut bytes)
        .ok()?;
    let after = file.metadata().ok()?;
    if &FileIdentity::from_metadata(&after) != identity {
        return None;
    }
    let (class, confidence_percent, reasons) =
        classify_bytes(&bytes, identity.size > bytes.len() as u64);
    Some(ExecutableEvidence {
        class,
        confidence_percent,
        generation,
        resolved_path: path.to_owned(),
        reasons,
    })
}

fn classify_bytes(bytes: &[u8], truncated: bool) -> (ExecutableClass, u8, Vec<EvidenceReason>) {
    if let Some(line) = bytes
        .strip_prefix(b"#!")
        .and_then(|rest| rest.split(|byte| *byte == b'\n').next())
    {
        let interpreter =
            String::from_utf8_lossy(&line[..line.len().min(256)]).to_ascii_lowercase();
        if ["/sh", "/bash", "/dash", "/zsh", "/fish"]
            .iter()
            .any(|name| interpreter.contains(name))
        {
            return (
                ExecutableClass::LikelyTerminal,
                75,
                vec![EvidenceReason::ShellInterpreter],
            );
        }
        if ["python", "ruby", "perl"]
            .iter()
            .any(|name| interpreter.contains(name))
        {
            return (
                ExecutableClass::LikelyTerminal,
                55,
                vec![EvidenceReason::LanguageInterpreter],
            );
        }
        return (ExecutableClass::Unknown, 0, vec![EvidenceReason::Malformed]);
    }
    if !bytes.starts_with(b"\x7fELF") {
        return (ExecutableClass::Unknown, 0, vec![EvidenceReason::Malformed]);
    }
    let dependencies = match elf_needed_libraries(bytes) {
        Ok(dependencies) => dependencies,
        Err(EvidenceReason::Malformed) => {
            return (ExecutableClass::Unknown, 0, vec![EvidenceReason::Malformed]);
        }
        Err(_) if truncated => {
            return (
                ExecutableClass::Unknown,
                0,
                vec![EvidenceReason::InspectionBudget],
            );
        }
        Err(reason) => return (ExecutableClass::Unknown, 0, vec![reason]),
    };
    let gui = dependencies.iter().any(|dependency| {
        [
            "libwayland-client",
            "libX11.so",
            "libgtk-",
            "libQt5Gui",
            "libQt6Gui",
            "libSDL2",
        ]
        .iter()
        .any(|prefix| dependency.starts_with(prefix))
    });
    let terminal = dependencies.iter().any(|dependency| {
        ["libncurses", "libtinfo"]
            .iter()
            .any(|prefix| dependency.starts_with(prefix))
    });
    match (gui, terminal) {
        (true, false) => (
            ExecutableClass::LikelyGraphical,
            70,
            vec![EvidenceReason::GuiLibrary],
        ),
        (false, true) => (
            ExecutableClass::LikelyTerminal,
            65,
            vec![EvidenceReason::TerminalLibrary],
        ),
        (true, true) => (
            ExecutableClass::Unknown,
            0,
            vec![EvidenceReason::GuiLibrary, EvidenceReason::TerminalLibrary],
        ),
        (false, false) => (
            ExecutableClass::Unknown,
            0,
            vec![EvidenceReason::StaticOrUnresolvedElf],
        ),
    }
}

fn elf_needed_libraries(bytes: &[u8]) -> Result<Vec<String>, EvidenceReason> {
    let class = *bytes.get(4).ok_or(EvidenceReason::Malformed)?;
    let little_endian = match bytes.get(5) {
        Some(1) => true,
        Some(2) => false,
        _ => return Err(EvidenceReason::Malformed),
    };
    let number = |offset: usize, width: usize| -> Result<u64, EvidenceReason> {
        let end = offset.checked_add(width).ok_or(EvidenceReason::Malformed)?;
        let value = bytes
            .get(offset..end)
            .ok_or(EvidenceReason::InspectionBudget)?;
        Ok(if little_endian {
            value
                .iter()
                .enumerate()
                .fold(0_u64, |total, (shift, byte)| {
                    total | (u64::from(*byte) << (shift * 8))
                })
        } else {
            value
                .iter()
                .fold(0_u64, |total, byte| (total << 8) | u64::from(*byte))
        })
    };
    let (section_offset, section_size, section_count, dynamic_entry_size) = match class {
        1 => (number(32, 4)?, number(46, 2)?, number(48, 2)?, 8_usize),
        2 => (number(40, 8)?, number(58, 2)?, number(60, 2)?, 16_usize),
        _ => return Err(EvidenceReason::Malformed),
    };
    let section_offset = usize::try_from(section_offset).map_err(|_| EvidenceReason::Malformed)?;
    let section_size = usize::try_from(section_size).map_err(|_| EvidenceReason::Malformed)?;
    let section_count = usize::try_from(section_count).map_err(|_| EvidenceReason::Malformed)?;
    if section_count == 0 {
        return Ok(Vec::new());
    }
    let minimum_section_size = if class == 1 { 40 } else { 64 };
    if section_size < minimum_section_size || section_count > 4_096 {
        return Err(EvidenceReason::Malformed);
    }
    let section = |index: usize| -> Result<(u32, usize, usize, usize), EvidenceReason> {
        let base = section_offset
            .checked_add(
                index
                    .checked_mul(section_size)
                    .ok_or(EvidenceReason::Malformed)?,
            )
            .ok_or(EvidenceReason::Malformed)?;
        let kind = u32::try_from(number(base + 4, 4)?).map_err(|_| EvidenceReason::Malformed)?;
        let (offset_at, size_at, link_at, width) = if class == 1 {
            (16, 20, 24, 4)
        } else {
            (24, 32, 40, 8)
        };
        let offset = usize::try_from(number(base + offset_at, width)?)
            .map_err(|_| EvidenceReason::Malformed)?;
        let size = usize::try_from(number(base + size_at, width)?)
            .map_err(|_| EvidenceReason::Malformed)?;
        let link =
            usize::try_from(number(base + link_at, 4)?).map_err(|_| EvidenceReason::Malformed)?;
        Ok((kind, offset, size, link))
    };

    let mut dependencies = Vec::new();
    for index in 0..section_count {
        let (kind, dynamic_offset, dynamic_size, string_index) = section(index)?;
        if kind != 6 {
            continue;
        }
        if string_index >= section_count {
            return Err(EvidenceReason::Malformed);
        }
        let (_, string_offset, string_size, _) = section(string_index)?;
        let strings_end = string_offset
            .checked_add(string_size)
            .ok_or(EvidenceReason::Malformed)?;
        let strings = bytes
            .get(string_offset..strings_end)
            .ok_or(EvidenceReason::InspectionBudget)?;
        let dynamic_end = dynamic_offset
            .checked_add(dynamic_size)
            .ok_or(EvidenceReason::Malformed)?;
        let dynamic = bytes
            .get(dynamic_offset..dynamic_end)
            .ok_or(EvidenceReason::InspectionBudget)?;
        for entry in dynamic.chunks_exact(dynamic_entry_size).take(4_096) {
            let entry_offset = entry.as_ptr() as usize - bytes.as_ptr() as usize;
            let width = if class == 1 { 4 } else { 8 };
            let tag = number(entry_offset, width)?;
            if tag == 0 {
                break;
            }
            if tag != 1 {
                continue;
            }
            let name_offset = usize::try_from(number(entry_offset + width, width)?)
                .map_err(|_| EvidenceReason::Malformed)?;
            let name = strings
                .get(name_offset..)
                .and_then(|tail| tail.split(|byte| *byte == 0).next())
                .filter(|name| !name.is_empty() && name.len() <= 512)
                .and_then(|name| std::str::from_utf8(name).ok())
                .ok_or(EvidenceReason::Malformed)?;
            if dependencies.len() < 256 {
                dependencies.push(name.to_owned());
            }
        }
    }
    Ok(dependencies)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::PermissionsExt};

    fn executable(path: &Path, contents: &[u8]) {
        fs::write(path, contents).unwrap();
        let mut permissions = fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).unwrap();
    }

    fn elf_with_dependencies(dependencies: &[&str]) -> Vec<u8> {
        let mut strings = vec![0];
        let offsets = dependencies
            .iter()
            .map(|dependency| {
                let offset = strings.len() as u64;
                strings.extend_from_slice(dependency.as_bytes());
                strings.push(0);
                offset
            })
            .collect::<Vec<_>>();
        let section_offset = 64_usize;
        let dynamic_offset = section_offset + 3 * 64;
        let dynamic_size = (offsets.len() + 1) * 16;
        let string_offset = dynamic_offset + dynamic_size;
        let mut bytes = vec![0_u8; string_offset + strings.len()];
        bytes[..6].copy_from_slice(b"\x7fELF\x02\x01");
        bytes[40..48].copy_from_slice(&(section_offset as u64).to_le_bytes());
        bytes[58..60].copy_from_slice(&64_u16.to_le_bytes());
        bytes[60..62].copy_from_slice(&3_u16.to_le_bytes());
        let dynamic_section = section_offset + 64;
        bytes[dynamic_section + 4..dynamic_section + 8].copy_from_slice(&6_u32.to_le_bytes());
        bytes[dynamic_section + 24..dynamic_section + 32]
            .copy_from_slice(&(dynamic_offset as u64).to_le_bytes());
        bytes[dynamic_section + 32..dynamic_section + 40]
            .copy_from_slice(&(dynamic_size as u64).to_le_bytes());
        bytes[dynamic_section + 40..dynamic_section + 44].copy_from_slice(&2_u32.to_le_bytes());
        let string_section = section_offset + 128;
        bytes[string_section + 4..string_section + 8].copy_from_slice(&3_u32.to_le_bytes());
        bytes[string_section + 24..string_section + 32]
            .copy_from_slice(&(string_offset as u64).to_le_bytes());
        bytes[string_section + 32..string_section + 40]
            .copy_from_slice(&(strings.len() as u64).to_le_bytes());
        for (index, offset) in offsets.into_iter().enumerate() {
            let entry = dynamic_offset + index * 16;
            bytes[entry..entry + 8].copy_from_slice(&1_u64.to_le_bytes());
            bytes[entry + 8..entry + 16].copy_from_slice(&offset.to_le_bytes());
        }
        bytes[string_offset..].copy_from_slice(&strings);
        bytes
    }

    #[test]
    fn bounded_inspection_classifies_scripts_and_library_evidence_without_execution() {
        assert_eq!(
            classify_bytes(b"#!/bin/sh\necho nope", false).0,
            ExecutableClass::LikelyTerminal
        );
        assert_eq!(
            classify_bytes(&elf_with_dependencies(&["libwayland-client.so.0"]), false).0,
            ExecutableClass::LikelyGraphical
        );
        assert_eq!(
            classify_bytes(
                &elf_with_dependencies(&["libX11.so.6", "libncurses.so.6"]),
                false
            )
            .0,
            ExecutableClass::Unknown
        );
        assert_eq!(
            classify_bytes(b"\x7fELF....", true).1,
            0,
            "absence of a GUI dependency cannot prove terminal behavior"
        );
    }

    #[test]
    fn path_order_alias_dedup_and_caps_are_deterministic() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        executable(&first.path().join("shared"), b"#!/bin/sh\n");
        executable(
            &second.path().join("shared"),
            &elf_with_dependencies(&["libwayland-client.so"]),
        );
        executable(
            &second.path().join("graphical"),
            &elf_with_dependencies(&["libX11.so.6"]),
        );
        std::os::unix::fs::symlink(second.path().join("graphical"), second.path().join("alias"))
            .unwrap();
        let path = std::env::join_paths([first.path(), second.path()]).unwrap();
        let snapshot = RwLock::new(Arc::new(IndexSnapshot::default()));
        scan_path_generation(
            Some(&path),
            ScanBudgets {
                max_elapsed: Duration::from_secs(5),
                ..ScanBudgets::default()
            },
            7,
            &snapshot,
        );
        let snapshot = snapshot.read().unwrap();
        assert_eq!(
            snapshot.commands["shared"].class,
            ExecutableClass::LikelyTerminal
        );
        assert_eq!(
            snapshot.commands["graphical"].class,
            ExecutableClass::LikelyGraphical
        );
        assert!(Arc::ptr_eq(
            &snapshot.commands["graphical"],
            &snapshot.commands["alias"]
        ));
        assert_eq!(snapshot.progress.generation, 7);
        assert!(snapshot.progress.complete);
        assert!(snapshot.progress.retained_bytes_estimate > 0);
    }

    #[test]
    fn stale_file_replacement_is_not_published() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("tool");
        executable(&path, b"#!/bin/sh\n");
        let identity = FileIdentity::from_metadata(&fs::metadata(&path).unwrap());
        executable(&path, &elf_with_dependencies(&["libX11.so.6"]));
        assert!(inspect_executable(&path, &identity, 1, ScanBudgets::default()).is_none());
    }

    #[test]
    fn entry_caps_publish_an_explicit_partial_generation() {
        let directory = tempfile::tempdir().unwrap();
        executable(&directory.path().join("one"), b"#!/bin/sh\n");
        executable(&directory.path().join("two"), b"#!/bin/sh\n");
        let path = std::env::join_paths([directory.path()]).unwrap();
        let snapshot = RwLock::new(Arc::new(IndexSnapshot::default()));
        scan_path_generation(
            Some(&path),
            ScanBudgets {
                max_directory_entries: 1,
                max_elapsed: Duration::from_secs(5),
                ..ScanBudgets::default()
            },
            9,
            &snapshot,
        );
        let snapshot = snapshot.read().unwrap();
        assert!(!snapshot.progress.complete);
        assert_eq!(snapshot.progress.generation, 9);
        assert_eq!(snapshot.progress.entries_inspected, 1);
        assert_eq!(snapshot.commands.len(), 1);
    }
}
