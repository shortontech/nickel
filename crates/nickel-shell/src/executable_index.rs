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
    sync::{Arc, Mutex, OnceLock, RwLock, mpsc},
    time::{Duration, Instant, SystemTime},
};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};

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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct PredictionMetrics {
    /// Rows are likely-graphical, likely-terminal, unknown, and unavailable;
    /// columns are no qualifying window and qualifying window.
    pub observations: [[u64; 2]; 4],
    pub descendant_windows: u64,
}

impl PredictionMetrics {
    fn record(&mut self, evidence: Option<&ExecutableEvidence>, window: Option<bool>) {
        let class = match evidence.map(|evidence| evidence.class) {
            Some(ExecutableClass::LikelyGraphical) => 0,
            Some(ExecutableClass::LikelyTerminal) => 1,
            Some(ExecutableClass::Unknown) => 2,
            None => 3,
        };
        let observed = usize::from(window.is_some());
        self.observations[class][observed] = self.observations[class][observed].saturating_add(1);
        if window == Some(true) {
            self.descendant_windows = self.descendant_windows.saturating_add(1);
        }
    }
}

static PREDICTION_METRICS: OnceLock<Mutex<PredictionMetrics>> = OnceLock::new();

pub(crate) fn record_prediction_observation(
    evidence: Option<&ExecutableEvidence>,
    descendant_window: Option<bool>,
) {
    let mut metrics = PREDICTION_METRICS
        .get_or_init(|| Mutex::new(PredictionMetrics::default()))
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    metrics.record(evidence, descendant_window);
    tracing::debug!(
        likely_graphical_without_window = metrics.observations[0][0],
        likely_graphical_with_window = metrics.observations[0][1],
        likely_terminal_without_window = metrics.observations[1][0],
        likely_terminal_with_window = metrics.observations[1][1],
        unknown_without_window = metrics.observations[2][0],
        unknown_with_window = metrics.observations[2][1],
        unavailable_without_window = metrics.observations[3][0],
        unavailable_with_window = metrics.observations[3][1],
        descendant_windows = metrics.descendant_windows,
        "updated bounded executable prediction outcomes"
    );
}

#[allow(dead_code)]
pub(crate) fn prediction_metrics() -> PredictionMetrics {
    *PREDICTION_METRICS
        .get_or_init(|| Mutex::new(PredictionMetrics::default()))
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ScanProgress {
    pub generation: u64,
    pub directories_scanned: usize,
    pub entries_inspected: usize,
    pub commands_published: usize,
    pub retained_bytes_estimate: usize,
    pub bytes_read: u64,
    pub elapsed_micros: u64,
    pub refresh_queue_capacity: usize,
    pub peak_open_files: usize,
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
            max_prefix_bytes: 64 * 1024,
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
                bytes_read: 0,
                elapsed_micros: 0,
                refresh_queue_capacity: 1,
                peak_open_files: 1,
                evictions: 0,
                complete: false,
            },
        }
    }
}

pub(crate) struct ExecutableIndex {
    snapshot: Arc<RwLock<Arc<IndexSnapshot>>>,
    refresh: mpsc::SyncSender<RefreshReason>,
    _watcher: Option<RecommendedWatcher>,
}

#[derive(Clone, Copy, Debug)]
enum RefreshReason {
    Filesystem,
    Environment,
}

impl ExecutableIndex {
    fn start(path: Option<std::ffi::OsString>, budgets: ScanBudgets) -> Self {
        let snapshot = Arc::new(RwLock::new(Arc::new(IndexSnapshot::default())));
        let (refresh, receiver) = mpsc::sync_channel(1);
        let watcher = path
            .as_deref()
            .and_then(|path| watch_effective_path(path, budgets, refresh.clone()));
        let worker_snapshot = Arc::clone(&snapshot);
        let _ = std::thread::Builder::new()
            .name("nickel-executable-index".into())
            .spawn(move || scan_worker(path, budgets, worker_snapshot, receiver));
        Self {
            snapshot,
            refresh,
            _watcher: watcher,
        }
    }

    pub(crate) fn classify(&self, command: &str) -> Option<ExecutableEvidence> {
        let command = normalized_command_name(command)?;
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
        let _ = self.refresh.try_send(RefreshReason::Environment);
    }
}

fn watch_effective_path(
    path: &std::ffi::OsStr,
    budgets: ScanBudgets,
    refresh: mpsc::SyncSender<RefreshReason>,
) -> Option<RecommendedWatcher> {
    let callback_refresh = refresh;
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        if event.is_ok() {
            let _ = callback_refresh.try_send(RefreshReason::Filesystem);
        }
    })
    .ok()?;
    let current_directory = std::env::current_dir().ok();
    let mut watched = HashSet::new();
    for directory in std::env::split_paths(path).take(budgets.max_directories) {
        let Some(directory) = effective_path_directory(directory, current_directory.as_deref())
        else {
            continue;
        };
        let canonical = std::fs::canonicalize(&directory).unwrap_or(directory);
        if watched.insert(canonical.clone()) {
            let _ = watcher.watch(&canonical, RecursiveMode::NonRecursive);
        }
    }
    (!watched.is_empty()).then_some(watcher)
}

fn normalized_command_name(command: &str) -> Option<&str> {
    let path = Path::new(command);
    (path.components().count() == 1)
        .then(|| path.file_name().and_then(|name| name.to_str()))
        .flatten()
}

pub(crate) fn global_executable_index() -> &'static ExecutableIndex {
    static INDEX: OnceLock<ExecutableIndex> = OnceLock::new();
    INDEX.get_or_init(|| ExecutableIndex::start(std::env::var_os("PATH"), ScanBudgets::default()))
}

fn scan_worker(
    mut path: Option<std::ffi::OsString>,
    budgets: ScanBudgets,
    snapshot: Arc<RwLock<Arc<IndexSnapshot>>>,
    receiver: mpsc::Receiver<RefreshReason>,
) {
    let mut generation = 1_u64;
    loop {
        scan_path_generation(path.as_deref(), budgets, generation, &snapshot);
        generation = generation.saturating_add(1);
        match receiver.recv_timeout(Duration::from_secs(30)) {
            Ok(RefreshReason::Filesystem) => {}
            Ok(RefreshReason::Environment) | Err(mpsc::RecvTimeoutError::Timeout) => {
                path = std::env::var_os("PATH");
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}

fn publish(
    snapshot: &RwLock<Arc<IndexSnapshot>>,
    commands: &HashMap<String, Arc<ExecutableEvidence>>,
    mut progress: ScanProgress,
    elapsed: Duration,
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
    progress.elapsed_micros = elapsed.as_micros().min(u128::from(u64::MAX)) as u64;
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
    let current_directory = std::env::current_dir().ok();
    let mut complete = true;

    'directories: for (directory_index, directory) in directories.enumerate() {
        if directory_index >= budgets.max_directories || started.elapsed() >= budgets.max_elapsed {
            complete = false;
            break;
        }
        let Some(directory) = effective_path_directory(directory, current_directory.as_deref())
        else {
            continue;
        };
        let canonical = std::fs::canonicalize(&directory).unwrap_or(directory);
        if !seen_directories.insert(canonical.clone()) {
            continue;
        }
        progress.directories_scanned += 1;
        let Ok(entries) = std::fs::read_dir(canonical) else {
            publish(snapshot, &commands, progress, started.elapsed());
            continue;
        };
        for entry in entries {
            if progress.entries_inspected >= budgets.max_directory_entries {
                complete = false;
                break 'directories;
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
                let Some((evidence, bytes_read)) =
                    inspect_executable(&path, &identity, generation, budgets)
                else {
                    continue;
                };
                progress.bytes_read = progress.bytes_read.saturating_add(bytes_read as u64);
                let evidence = Arc::new(evidence);
                identities.insert(identity.clone(), Arc::clone(&evidence));
                evidence
            };
            *aliases += 1;
            commands.insert(command, evidence);
        }
        publish(snapshot, &commands, progress, started.elapsed());
    }
    progress.complete = complete;
    publish(snapshot, &commands, progress, started.elapsed());
}

fn effective_path_directory(
    directory: PathBuf,
    current_directory: Option<&Path>,
) -> Option<PathBuf> {
    if directory.as_os_str().is_empty() {
        current_directory.map(Path::to_owned)
    } else {
        Some(directory)
    }
}

fn inspect_executable(
    path: &Path,
    identity: &FileIdentity,
    generation: u64,
    budgets: ScanBudgets,
) -> Option<(ExecutableEvidence, usize)> {
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
    let inspected_bytes = bytes.len();
    Some((
        ExecutableEvidence {
            class,
            confidence_percent,
            generation,
            resolved_path: path.to_owned(),
            reasons,
        },
        inspected_bytes,
    ))
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
    if let Some(dependencies) = elf_program_needed_libraries(bytes, class, little_endian)? {
        return Ok(dependencies);
    }
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

fn elf_program_needed_libraries(
    bytes: &[u8],
    class: u8,
    little_endian: bool,
) -> Result<Option<Vec<String>>, EvidenceReason> {
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
    let (program_offset, program_size, program_count, word_size, dynamic_entry_size) = match class {
        1 => (
            number(28, 4)?,
            number(42, 2)?,
            number(44, 2)?,
            4_usize,
            8_usize,
        ),
        2 => (
            number(32, 8)?,
            number(54, 2)?,
            number(56, 2)?,
            8_usize,
            16_usize,
        ),
        _ => return Err(EvidenceReason::Malformed),
    };
    let program_offset = usize::try_from(program_offset).map_err(|_| EvidenceReason::Malformed)?;
    let program_size = usize::try_from(program_size).map_err(|_| EvidenceReason::Malformed)?;
    let program_count = usize::try_from(program_count).map_err(|_| EvidenceReason::Malformed)?;
    if program_count == 0 {
        return Ok(None);
    }
    let minimum_size = if class == 1 { 32 } else { 56 };
    if program_size < minimum_size || program_count > 1_024 {
        return Err(EvidenceReason::Malformed);
    }
    let mut loads = Vec::new();
    let mut dynamic = None;
    for index in 0..program_count {
        let base = program_offset
            .checked_add(
                index
                    .checked_mul(program_size)
                    .ok_or(EvidenceReason::Malformed)?,
            )
            .ok_or(EvidenceReason::Malformed)?;
        let kind = number(base, 4)?;
        let (offset_at, address_at, size_at) = if class == 1 { (4, 8, 16) } else { (8, 16, 32) };
        let offset = usize::try_from(number(base + offset_at, word_size)?)
            .map_err(|_| EvidenceReason::Malformed)?;
        let address = number(base + address_at, word_size)?;
        let size = usize::try_from(number(base + size_at, word_size)?)
            .map_err(|_| EvidenceReason::Malformed)?;
        match kind {
            1 => loads.push((address, offset, size)),
            2 => dynamic = Some((offset, size)),
            _ => {}
        }
    }
    let Some((dynamic_offset, dynamic_size)) = dynamic else {
        return Ok(None);
    };
    let dynamic_end = dynamic_offset
        .checked_add(dynamic_size)
        .ok_or(EvidenceReason::Malformed)?;
    let dynamic = bytes
        .get(dynamic_offset..dynamic_end)
        .ok_or(EvidenceReason::InspectionBudget)?;
    let mut needed = Vec::new();
    let mut string_address = None;
    let mut string_size = None;
    for entry in dynamic.chunks_exact(dynamic_entry_size).take(4_096) {
        let entry_offset = entry.as_ptr() as usize - bytes.as_ptr() as usize;
        let tag = number(entry_offset, word_size)?;
        let value = number(entry_offset + word_size, word_size)?;
        match tag {
            0 => break,
            1 if needed.len() < 256 => needed.push(value),
            5 => string_address = Some(value),
            10 => string_size = Some(value),
            _ => {}
        }
    }
    let string_address = string_address.ok_or(EvidenceReason::Malformed)?;
    let string_size = usize::try_from(string_size.ok_or(EvidenceReason::Malformed)?)
        .map_err(|_| EvidenceReason::Malformed)?;
    if string_size > 4 * 1024 * 1024 {
        return Err(EvidenceReason::InspectionBudget);
    }
    let string_offset = loads
        .iter()
        .find_map(|(address, offset, size)| {
            let relative = string_address.checked_sub(*address)?;
            (relative < *size as u64)
                .then(|| offset.checked_add(relative as usize))
                .flatten()
        })
        .ok_or(EvidenceReason::Malformed)?;
    let strings_end = string_offset
        .checked_add(string_size)
        .ok_or(EvidenceReason::Malformed)?;
    let strings = bytes
        .get(string_offset..strings_end)
        .ok_or(EvidenceReason::InspectionBudget)?;
    let dependencies = needed
        .into_iter()
        .map(|offset| {
            let offset = usize::try_from(offset).map_err(|_| EvidenceReason::Malformed)?;
            strings
                .get(offset..)
                .and_then(|tail| tail.split(|byte| *byte == 0).next())
                .filter(|name| !name.is_empty() && name.len() <= 512)
                .and_then(|name| std::str::from_utf8(name).ok())
                .map(str::to_owned)
                .ok_or(EvidenceReason::Malformed)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Some(dependencies))
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
    fn lookup_never_substitutes_a_path_entry_with_the_same_basename() {
        assert_eq!(normalized_command_name("tool"), Some("tool"));
        assert_eq!(normalized_command_name("./tool"), None);
        assert_eq!(normalized_command_name("/tmp/tool"), None);
        assert_eq!(normalized_command_name("directory/tool"), None);
    }

    #[test]
    fn empty_path_segments_resolve_to_the_captured_current_directory() {
        let current = Path::new("/work/current");
        assert_eq!(
            effective_path_directory(PathBuf::new(), Some(current)).as_deref(),
            Some(current)
        );
        assert_eq!(effective_path_directory(PathBuf::new(), None), None);
        assert_eq!(
            effective_path_directory(PathBuf::from("/bin"), Some(current)),
            Some(PathBuf::from("/bin"))
        );
    }

    #[test]
    fn native_path_notification_coalesces_a_background_refresh() {
        let directory = tempfile::tempdir().unwrap();
        let path = std::env::join_paths([directory.path()]).unwrap();
        let index = ExecutableIndex::start(
            Some(path),
            ScanBudgets {
                max_elapsed: Duration::from_secs(1),
                ..ScanBudgets::default()
            },
        );
        let initial_deadline = Instant::now() + Duration::from_secs(2);
        while !index.progress().complete {
            assert!(
                Instant::now() < initial_deadline,
                "initial scan did not finish"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        let initial_generation = index.progress().generation;
        executable(&directory.path().join("notified-tool"), b"#!/bin/sh\n");
        let refresh_deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let progress = index.progress();
            if progress.generation > initial_generation
                && progress.complete
                && index.classify("notified-tool").is_some()
            {
                break;
            }
            assert!(
                Instant::now() < refresh_deadline,
                "filesystem notification did not publish a refreshed generation"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(index.progress().refresh_queue_capacity, 1);
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
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        for (directory, names) in [
            (first.path(), ["one", "two"]),
            (second.path(), ["three", "four"]),
        ] {
            for name in names {
                executable(&directory.join(name), b"#!/bin/sh\n");
            }
        }
        let path = std::env::join_paths([first.path(), second.path()]).unwrap();
        let snapshot = RwLock::new(Arc::new(IndexSnapshot::default()));
        scan_path_generation(
            Some(&path),
            ScanBudgets {
                max_directory_entries: 3,
                max_elapsed: Duration::from_secs(5),
                ..ScanBudgets::default()
            },
            9,
            &snapshot,
        );
        let snapshot = snapshot.read().unwrap();
        assert!(!snapshot.progress.complete);
        assert_eq!(snapshot.progress.generation, 9);
        assert_eq!(snapshot.progress.entries_inspected, 3);
        assert_eq!(snapshot.commands.len(), 3);
    }

    #[test]
    fn native_path_scan_reports_bounded_resource_evidence() {
        let snapshot = RwLock::new(Arc::new(IndexSnapshot::default()));
        let budgets = ScanBudgets::default();
        scan_path_generation(std::env::var_os("PATH").as_deref(), budgets, 11, &snapshot);
        let snapshot = snapshot.read().unwrap();
        let progress = snapshot.progress;
        let graphical = snapshot
            .commands
            .values()
            .filter(|evidence| evidence.class == ExecutableClass::LikelyGraphical)
            .count();
        let terminal = snapshot
            .commands
            .values()
            .filter(|evidence| evidence.class == ExecutableClass::LikelyTerminal)
            .count();
        eprintln!(
            "generation={} directories={} inspected={} commands={} graphical={} terminal={} bytes_read={} retained_bytes={} elapsed_us={} complete={} evictions={}",
            progress.generation,
            progress.directories_scanned,
            progress.entries_inspected,
            progress.commands_published,
            graphical,
            terminal,
            progress.bytes_read,
            progress.retained_bytes_estimate,
            progress.elapsed_micros,
            progress.complete,
            progress.evictions,
        );
        assert!(progress.directories_scanned <= budgets.max_directories);
        assert!(progress.commands_published <= budgets.max_commands);
        assert!(progress.elapsed_micros <= 3_000_000);
        assert_eq!(progress.refresh_queue_capacity, 1);
        assert_eq!(progress.peak_open_files, 1);
        assert!(
            progress.bytes_read <= progress.entries_inspected as u64 * budgets.max_prefix_bytes
        );
    }

    #[test]
    fn prediction_metrics_compare_classes_with_attributed_windows_without_identity_data() {
        let evidence = |class| ExecutableEvidence {
            class,
            confidence_percent: 50,
            generation: 9,
            resolved_path: "/private/command".into(),
            reasons: vec![EvidenceReason::GuiLibrary],
        };
        let graphical = evidence(ExecutableClass::LikelyGraphical);
        let terminal = evidence(ExecutableClass::LikelyTerminal);
        let unknown = evidence(ExecutableClass::Unknown);
        let mut metrics = PredictionMetrics::default();

        metrics.record(Some(&graphical), Some(false));
        metrics.record(Some(&terminal), None);
        metrics.record(Some(&unknown), Some(true));
        metrics.record(None, None);

        assert_eq!(metrics.observations[0], [0, 1]);
        assert_eq!(metrics.observations[1], [1, 0]);
        assert_eq!(metrics.observations[2], [0, 1]);
        assert_eq!(metrics.observations[3], [1, 0]);
        assert_eq!(metrics.descendant_windows, 1);
        let debug = format!("{metrics:?}");
        assert!(!debug.contains("private"));
        assert!(!debug.contains("command"));
    }
}
