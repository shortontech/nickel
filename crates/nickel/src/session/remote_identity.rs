//! OS-backed process evidence. X11 property strings never establish authority.
use std::{
    collections::HashMap,
    fs,
    io::{Read, Write},
    os::fd::{AsRawFd, FromRawFd},
    os::unix::{
        ffi::{OsStrExt, OsStringExt},
        fs::{MetadataExt, OpenOptionsExt},
    },
    path::PathBuf,
    sync::mpsc::{self, SyncSender},
};

use sha2::{Digest, Sha256};
use smithay::{reexports::calloop::channel, xwayland::X11Surface};

use super::window_registry::WindowId;

pub(super) enum IdentitySource {
    WaylandPeer { pid: u32, app_id: Option<String> },
    X11Client(Box<X11Surface>),
}

#[derive(Clone)]
pub(super) struct ProcessIdentity {
    pid: u32,
    start_time: u64,
    device: u64,
    inode: u64,
    pub application: Option<String>,
    pub application_name: Option<String>,
    pub desktop_entry: Option<String>,
    pub catalog_generation: u64,
    pub protected: bool,
    flatpak_application: Option<String>,
}

impl ProcessIdentity {
    pub(super) fn pid(&self) -> u32 {
        self.pid
    }
    pub(super) fn inspect(pid: u32) -> Option<Self> {
        if pid == 0 {
            return None;
        }
        let start_time = process_start_time(pid)?;
        let link = format!("/proc/{pid}/exe");
        // Inspect identity without requiring permission to read executable bytes.
        let executable_file = fs::OpenOptions::new()
            .read(true)
            .custom_flags(nix::libc::O_PATH)
            .open(&link)
            .ok()?;
        let executable = normalize_executable_path(
            fs::read_link(format!("/proc/self/fd/{}", executable_file.as_raw_fd())).ok()?,
        );
        let file = executable_file.metadata().ok()?;
        let current_file = fs::metadata(&link).ok()?;
        if process_start_time(pid)? != start_time
            || current_file.dev() != file.dev()
            || current_file.ino() != file.ino()
        {
            return None;
        }
        let application = executable_application_identity(&executable, &file);
        let flatpak_application = inspect_flatpak_application(pid);
        let protected = protected_executable(&executable);
        let application_name = application
            .as_ref()
            .and_then(|_| executable.file_name())
            .map(|name| bounded_label(&name.to_string_lossy()));
        Some(Self {
            pid,
            start_time,
            device: file.dev(),
            inode: file.ino(),
            application,
            application_name,
            desktop_entry: None,
            catalog_generation: 0,
            protected,
            flatpak_application,
        })
    }

    /// Recheck the process incarnation and executed file at the dispatch boundary.
    /// A reused PID or exec cannot inherit evidence collected for an earlier image.
    pub fn is_current(&self) -> bool {
        let link = format!("/proc/{}/exe", self.pid);
        process_start_time(self.pid) == Some(self.start_time)
            && fs::metadata(link)
                .is_ok_and(|file| file.dev() == self.device && file.ino() == self.inode)
            && process_start_time(self.pid) == Some(self.start_time)
    }

    pub(super) fn same_current_process(&self, other: &Self) -> bool {
        self.pid == other.pid
            && self.start_time == other.start_time
            && self.is_current()
            && other.is_current()
    }
}

const MAX_FLATPAK_INFO_BYTES: u64 = 64 * 1024;

fn inspect_flatpak_application(pid: u32) -> Option<String> {
    let path = format!("/proc/{pid}/root/.flatpak-info");
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_CLOEXEC)
        .open(path)
        .ok()?;
    let metadata = file.metadata().ok()?;
    // Flatpak creates this root-owned, immutable sandbox marker. Refuse a file
    // the application user could replace or modify.
    if !metadata.is_file()
        || metadata.len() > MAX_FLATPAK_INFO_BYTES
        || metadata.uid() != 0
        || metadata.mode() & 0o022 != 0
    {
        return None;
    }
    let mut contents = String::new();
    file.take(MAX_FLATPAK_INFO_BYTES + 1)
        .read_to_string(&mut contents)
        .ok()?;
    let mut application_section = false;
    for line in contents.lines().take(4096) {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            application_section = line == "[Application]";
        } else if application_section && let Some(value) = line.strip_prefix("name=") {
            return normalized_desktop_id(value);
        }
    }
    None
}

fn normalized_desktop_id(value: &str) -> Option<String> {
    let value = value.trim().trim_end_matches(".desktop");
    (!value.is_empty()
        && value.len() <= 512
        && value.is_ascii()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')))
    .then(|| value.to_ascii_lowercase())
}

fn executable_application_identity(
    executable: &std::path::Path,
    file: &fs::Metadata,
) -> Option<String> {
    let mut digest = Sha256::new();
    digest.update(executable.as_os_str().as_bytes());
    digest.update(file.dev().to_le_bytes());
    digest.update(file.ino().to_le_bytes());
    (!shared_runtime_executable(executable))
        .then(|| format!("linux-executable:{:x}", digest.finalize()))
}

/// Retained native images prevent pathname replacement from changing the image
/// authorized for launch. Direct readable scripts use an additional sealed
/// snapshot owned by the prepared command and never gain application identity.
pub(super) struct LaunchExecutable {
    _file: fs::File,
    pub application: Option<String>,
}

fn resolve_launch_program(command: &std::process::Command) -> Option<PathBuf> {
    let program = std::path::Path::new(command.get_program());
    let cwd = command
        .get_current_dir()
        .map(std::path::Path::to_owned)
        .or_else(|| std::env::current_dir().ok())?;
    let path = if program.is_absolute() {
        program.to_owned()
    } else if program.components().count() == 1 {
        let search = command
            .get_envs()
            .find(|(name, _)| *name == "PATH")
            .map(|(_, value)| value.map(std::ffi::OsStr::to_owned))
            .unwrap_or_else(|| std::env::var_os("PATH"))?;
        std::env::split_paths(&search)
            .take(64)
            .map(|directory| cwd.join(directory).join(program))
            .find(|path| executable_metadata(path).is_some())?
    } else {
        cwd.join(program)
    };
    Some(path)
}

pub(super) fn protected_launch_target(command: &std::process::Command) -> bool {
    protected_executable(std::path::Path::new(command.get_program()))
        || resolve_launch_program(command)
            .and_then(|path| fs::canonicalize(path).ok())
            .is_some_and(|path| protected_executable(&path))
}

impl LaunchExecutable {
    pub fn prepare(command: &std::process::Command) -> Option<(Self, std::process::Command)> {
        Self::prepare_image(command, false)
            .ok()
            .flatten()
            .filter(|(proof, _)| proof.application.is_some())
    }

    pub fn prepare_full_session(
        command: &std::process::Command,
    ) -> Result<Option<(Self, std::process::Command)>, String> {
        Self::prepare_image(command, true)
    }

    fn prepare_image(
        command: &std::process::Command,
        allow_scripts: bool,
    ) -> Result<Option<(Self, std::process::Command)>, String> {
        let unavailable = || "application executable is unavailable".to_owned();
        let path = resolve_launch_program(command).ok_or_else(unavailable)?;
        let file = fs::OpenOptions::new()
            .read(true)
            .custom_flags(nix::libc::O_PATH)
            .open(path)
            .map_err(|_| unavailable())?;
        let descriptor = format!("/proc/{}/fd/{}", std::process::id(), file.as_raw_fd());
        let executable =
            normalize_executable_path(fs::read_link(&descriptor).map_err(|_| unavailable())?);
        let metadata = file.metadata().map_err(|_| unavailable())?;
        if protected_executable(&executable) {
            return Err("application launch target is protected".into());
        }
        if !metadata.is_file() || metadata.mode() & 0o111 == 0 {
            return Err(unavailable());
        }
        // Interpreter scripts and shared runtimes need stronger application
        // evidence. An interpreter's identity cannot authorize arbitrary scripts.
        let mut magic = [0; 4];
        let readable_elf =
            match fs::File::open(&descriptor).and_then(|mut file| file.read_exact(&mut magic)) {
                Ok(()) if magic == *b"\x7fELF" => true,
                // O_PATH can retain an execute-only image even when its header is
                // unreadable. Let exec enforce execution permission, but do not
                // infer application identity without confirming the image format.
                Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => false,
                Ok(()) => {
                    return if allow_scripts {
                        Self::prepare_script(command, file, &descriptor).map(Some)
                    } else {
                        Ok(None)
                    };
                }
                Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => {
                    return if allow_scripts {
                        Self::prepare_script(command, file, &descriptor).map(Some)
                    } else {
                        Ok(None)
                    };
                }
                Err(_) => return Err(unavailable()),
            };
        let application = readable_elf
            .then(|| executable_application_identity(&executable, &metadata))
            .flatten();
        let pinned = launch_command_with_program(command, descriptor);
        Ok(Some((
            Self {
                _file: file,
                application,
            },
            pinned,
        )))
    }
}

const MAX_LAUNCH_SCRIPT_BYTES: usize = 4 * 1024 * 1024;

fn launch_command_with_program(
    original: &std::process::Command,
    program: impl AsRef<std::ffi::OsStr>,
) -> std::process::Command {
    use std::os::unix::process::CommandExt;
    let mut pinned = std::process::Command::new(program);
    pinned
        .arg0(original.get_program())
        .args(original.get_args());
    if let Some(cwd) = original.get_current_dir() {
        pinned.current_dir(cwd);
    }
    for (name, value) in original.get_envs() {
        if let Some(value) = value {
            pinned.env(name, value);
        } else {
            pinned.env_remove(name);
        }
    }
    pinned
}

// Evaluate the original inode's execute permission with effective credentials,
// including ACLs. A private snapshot must not bypass that permission check.
fn executable_access(fd: std::os::fd::RawFd) -> std::io::Result<()> {
    // SAFETY: fd is an owned live descriptor; the path is a valid empty C string.
    // AT_EMPTY_PATH resolves the retained inode rather than a replaceable path.
    let result = unsafe {
        nix::libc::syscall(
            nix::libc::SYS_faccessat2,
            fd,
            c"".as_ptr(),
            nix::libc::X_OK,
            nix::libc::AT_EMPTY_PATH | nix::libc::AT_EACCESS,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

impl LaunchExecutable {
    fn prepare_script(
        original: &std::process::Command,
        source: fs::File,
        descriptor: &str,
    ) -> Result<(Self, std::process::Command), String> {
        use std::os::unix::{fs::PermissionsExt, process::CommandExt};
        executable_access(source.as_raw_fd())
            .map_err(|_| "application script execution is unavailable".to_owned())?;
        let before = source
            .metadata()
            .map_err(|_| "application script identity is unavailable".to_owned())?;
        let mut contents = Vec::new();
        fs::File::open(descriptor)
            .and_then(|file| {
                file.take((MAX_LAUNCH_SCRIPT_BYTES + 1) as u64)
                    .read_to_end(&mut contents)
            })
            .map_err(|_| "application script could not be read".to_owned())?;
        if contents.len() > MAX_LAUNCH_SCRIPT_BYTES {
            return Err("application script exceeds the launch snapshot limit".into());
        }
        let after = source
            .metadata()
            .map_err(|_| "application script identity is unavailable".to_owned())?;
        if before.len() != after.len()
            || before.mtime() != after.mtime()
            || before.mtime_nsec() != after.mtime_nsec()
            || before.ctime() != after.ctime()
            || before.ctime_nsec() != after.ctime_nsec()
        {
            return Err("application script changed during snapshot preparation".into());
        }
        if !contents.starts_with(b"#!") {
            return Err("application executable format is unavailable".into());
        }
        // SAFETY: the fixed NUL-terminated name and documented flags are valid.
        // Ownership of a successful memfd is transferred immediately to File.
        let fd = unsafe {
            nix::libc::memfd_create(
                c"nickel-launch-script".as_ptr(),
                nix::libc::MFD_CLOEXEC | nix::libc::MFD_ALLOW_SEALING,
            )
        };
        if fd < 0 {
            return Err("application script snapshot is unavailable".into());
        }
        // SAFETY: memfd_create returned a new descriptor owned by this function.
        let mut snapshot = unsafe { fs::File::from_raw_fd(fd) };
        snapshot
            .write_all(&contents)
            .and_then(|()| snapshot.set_permissions(fs::Permissions::from_mode(0o500)))
            .map_err(|_| "application script snapshot could not be prepared".to_owned())?;
        // SAFETY: snapshot owns fd. These seals make the captured bytes immutable.
        let sealed = unsafe {
            nix::libc::fcntl(
                fd,
                nix::libc::F_ADD_SEALS,
                nix::libc::F_SEAL_SEAL
                    | nix::libc::F_SEAL_SHRINK
                    | nix::libc::F_SEAL_GROW
                    | nix::libc::F_SEAL_WRITE,
            )
        };
        if sealed < 0 {
            return Err("application script snapshot could not be sealed".into());
        }
        let permission_source = source
            .try_clone()
            .map_err(|_| "application script identity is unavailable".to_owned())?;
        let mut pinned = launch_command_with_program(original, format!("/proc/self/fd/{fd}"));
        // Kernel shebang parsing is preserved, including its optional interpreter
        // argument. The child must keep this descriptor across interpreter exec:
        // spawn completion precedes the interpreter opening its script operand.
        // This necessarily exposes a descriptor path as $0/__file__; arbitrary
        // scripts that derive sibling asset paths from that value need adaptation.
        // SAFETY: the child hook uses only fcntl/syscall and error construction;
        // it neither allocates nor acquires locks after fork. Captured owned files
        // keep both descriptors valid for every use of the prepared command.
        unsafe {
            pinned.pre_exec(move || {
                executable_access(permission_source.as_raw_fd())?;
                let script_fd = snapshot.as_raw_fd();
                if nix::libc::fcntl(script_fd, nix::libc::F_SETFD, 0) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        Ok((
            Self {
                _file: source,
                application: None,
            },
            pinned,
        ))
    }
}

fn bounded_label(label: &str) -> String {
    label
        .chars()
        .filter(|character| !character.is_control())
        .take(256)
        .collect()
}

#[derive(Default)]
struct DesktopCatalog {
    generation: u64,
    files: HashMap<(u64, u64), Vec<DesktopIdentity>>,
    flatpaks: HashMap<String, Vec<DesktopIdentity>>,
}

#[derive(Clone)]
struct DesktopIdentity {
    id: String,
    name: String,
    executable: PathBuf,
    aliases: Vec<String>,
}

impl DesktopCatalog {
    fn build(
        generation: u64,
        applications: &[crate::model::Application],
        search_path: &std::ffi::OsStr,
    ) -> Self {
        let mut catalog = Self {
            generation,
            files: HashMap::new(),
            flatpaks: HashMap::new(),
        };
        let directories: Vec<_> = std::env::split_paths(search_path).take(64).collect();
        for application in applications.iter().take(4096) {
            if application.id().len() > 512 {
                continue;
            }
            let Some(command) = application
                .launch_command()
                .and_then(|command| command.first())
            else {
                continue;
            };
            let entry = DesktopIdentity {
                id: application.id().to_owned(),
                name: bounded_label(application.name()),
                executable: PathBuf::new(),
                aliases: application
                    .identity_aliases()
                    .iter()
                    .filter_map(|alias| normalized_desktop_id(alias))
                    .collect(),
            };
            if let Some(flatpak_id) = flatpak_id_from_command(application.launch_command().unwrap())
            {
                catalog
                    .flatpaks
                    .entry(flatpak_id)
                    .or_default()
                    .push(entry.clone());
                // `/usr/bin/flatpak` is a launcher shared by every sandbox and
                // cannot itself establish membership in any one application.
                continue;
            }
            let path = std::path::Path::new(command);
            let executable = if path.is_absolute() {
                Some(path.to_owned())
            } else if path.components().count() == 1 {
                // Preserve PATH ordering. Never search past an executable with
                // the same name to manufacture a match for another file.
                directories
                    .iter()
                    .map(|directory| directory.join(path))
                    .find(|path| executable_metadata(path).is_some())
            } else {
                None
            };
            let Some(executable) = executable else {
                continue;
            };
            let Some(file) = executable_metadata(&executable) else {
                continue;
            };
            catalog
                .files
                .entry((file.dev(), file.ino()))
                .or_default()
                .push(DesktopIdentity {
                    executable,
                    ..entry
                });
        }
        catalog
    }

    fn corroborate(&self, identity: &mut ProcessIdentity, surface_claim: Option<&str>) {
        identity.catalog_generation = self.generation;
        if identity.protected {
            return;
        }
        if let (Some(flatpak_id), Some(claim)) = (
            identity.flatpak_application.as_deref(),
            surface_claim.and_then(normalized_desktop_id),
        ) && let Some(entries) = self.flatpaks.get(flatpak_id)
            && let [entry] = entries.as_slice()
            && entry.matches_claim(&claim)
        {
            identity.application = Some(format!("linux-flatpak:{flatpak_id}"));
            identity.desktop_entry = Some(entry.id.clone());
            identity.application_name = Some(entry.name.clone());
            return;
        }
        if identity.application.is_none() {
            return;
        }
        let Some(entries) = self.files.get(&(identity.device, identity.inode)) else {
            return;
        };
        // Multiple desktop entries may launch the same binary. Preserve its
        // verified executable scope without assigning an ambiguous desktop name.
        if let [entry] = entries.as_slice()
            && executable_metadata(&entry.executable)
                .is_some_and(|file| file.dev() == identity.device && file.ino() == identity.inode)
        {
            identity.desktop_entry = Some(entry.id.clone());
            identity.application_name = Some(entry.name.clone());
        }
    }
}

impl DesktopIdentity {
    fn matches_claim(&self, claim: &str) -> bool {
        normalized_desktop_id(&self.id).as_deref() == Some(claim)
            || self.aliases.iter().any(|alias| alias == claim)
    }
}

fn flatpak_id_from_command(command: &[String]) -> Option<String> {
    let program = std::path::Path::new(command.first()?);
    if program.file_name()?.to_str()? != "flatpak" {
        return None;
    }
    let mut arguments = command.iter().skip(1);
    if arguments.next()? != "run" {
        return None;
    }
    arguments
        .find(|argument| !argument.starts_with('-'))
        .and_then(|argument| normalized_desktop_id(argument))
}

fn executable_metadata(path: &std::path::Path) -> Option<fs::Metadata> {
    fs::metadata(path)
        .ok()
        .filter(|file| file.is_file() && file.mode() & 0o111 != 0)
}

fn normalize_executable_path(path: PathBuf) -> PathBuf {
    let bytes = path.as_os_str().as_bytes();
    // Unlinking an installed file does not change the image of an already
    // running process. Its device/inode still distinguish it from replacements.
    bytes.strip_suffix(b" (deleted)").map_or_else(
        || path.clone(),
        |path| PathBuf::from(std::ffi::OsString::from_vec(path.to_vec())),
    )
}

fn process_start_time(pid: u32) -> Option<u64> {
    let mut stat = String::new();
    fs::File::open(format!("/proc/{pid}/stat"))
        .ok()?
        .take(4096)
        .read_to_string(&mut stat)
        .ok()?;
    stat.rsplit_once(')')?
        .1
        .split_whitespace()
        .nth(19)?
        .parse()
        .ok()
}

pub(super) fn protected_executable(path: &std::path::Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let name = name.strip_suffix(" (deleted)").unwrap_or(name);
    // These processes host permission/credential controls. Protect the whole
    // process because a page or prompt may change between individual requests.
    name == "nickel-settings"
        || name == "nickel-login"
        || name == "polkit-kde-authentication-agent-1"
        || name == "polkit-gnome-authentication-agent-1"
        || name == "lxqt-policykit-agent"
        || name == "gcr-prompter"
        || name == "gnome-keyring-prompt"
        || name == "pinentry"
        || name.starts_with("pinentry-")
}

fn shared_runtime_executable(path: &std::path::Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return true;
    };
    let name = name.strip_suffix(" (deleted)").unwrap_or(name);
    matches!(
        name,
        "sh" | "bash"
            | "dash"
            | "zsh"
            | "fish"
            | "java"
            | "node"
            | "nodejs"
            | "electron"
            | "flatpak"
            | "bwrap"
            | "dotnet"
            | "mono"
            | "perl"
            | "ruby"
            | "php"
    ) || name.starts_with("python")
        || name.starts_with("wine")
}

pub(super) enum WindowIdentity {
    Pending,
    Unavailable,
    Verified(ProcessIdentity),
}

impl WindowIdentity {
    pub(super) fn observation_process(&self) -> Option<ProcessIdentity> {
        match self {
            Self::Verified(process) if !process.protected && process.is_current() => {
                Some(process.clone())
            }
            _ => None,
        }
    }
    /// Verified OS process incarnation for local launch acknowledgement only.
    /// This does not grant control or application identity, including for shared
    /// runtimes and protected applications launched by the local user.
    pub fn current_process_id(&self) -> Option<u32> {
        match self {
            Self::Verified(process) if process.is_current() => Some(process.pid),
            _ => None,
        }
    }

    pub fn is_protected(&self) -> bool {
        match self {
            Self::Verified(process) => process.protected || !process.is_current(),
            Self::Pending | Self::Unavailable => true,
        }
    }

    pub fn application(&self) -> Option<&str> {
        match self {
            Self::Verified(process) if !process.protected && process.is_current() => {
                process.application.as_deref()
            }
            _ => None,
        }
    }
}

pub(super) struct IdentityResult {
    pub window: WindowId,
    pub identity: WindowIdentity,
}

pub(super) struct IdentityWorker {
    sender: SyncSender<(WindowId, IdentitySource)>,
}

impl IdentityWorker {
    pub fn start(reply: channel::SyncSender<IdentityResult>) -> std::io::Result<Self> {
        let (sender, requests) = mpsc::sync_channel::<(WindowId, IdentitySource)>(32);
        std::thread::Builder::new()
            .name("nickel-resource-identity".into())
            .spawn(move || {
                let search_path = std::env::var_os("PATH").unwrap_or_default();
                let mut catalog = DesktopCatalog::default();
                while let Ok((window, source)) = requests.recv() {
                    let (generation, applications, _) =
                        crate::platform::installed_application_signatures();
                    if generation != catalog.generation {
                        catalog = DesktopCatalog::build(generation, &applications, &search_path);
                    }
                    let surface_claim = match &source {
                        IdentitySource::WaylandPeer { app_id, .. } => app_id.clone(),
                        IdentitySource::X11Client(surface) => Some(surface.class()),
                    };
                    let pid = match source {
                        IdentitySource::WaylandPeer { pid, .. } => Some(pid),
                        // XRes asks the X server about the resource owner. Never use
                        // X11Surface::pid(), which reads the forgeable _NET_WM_PID.
                        IdentitySource::X11Client(surface) => surface.get_client_pid().ok(),
                    };
                    let identity = pid
                        .and_then(ProcessIdentity::inspect)
                        .map(|mut identity| {
                            catalog.corroborate(&mut identity, surface_claim.as_deref());
                            identity
                        })
                        .map_or(WindowIdentity::Unavailable, WindowIdentity::Verified);
                    if reply.send(IdentityResult { window, identity }).is_err() {
                        break;
                    }
                }
            })?;
        Ok(Self { sender })
    }

    pub fn request(&self, window: WindowId, source: IdentitySource) -> bool {
        self.sender.try_send((window, source)).is_ok()
    }
}

#[cfg(test)]
mod tests {
    // Copying a fixture executable and forking on another test thread can
    // temporarily inherit its writable fd, causing unrelated exec to fail with
    // ETXTBSY. Keep this module's native process/file fixtures serialized.
    static EXECUTABLE_FIXTURES: std::sync::Mutex<()> = std::sync::Mutex::new(());

    use super::*;

    fn application(id: &str, executable: &std::path::Path) -> crate::model::Application {
        crate::model::Application::new(
            id.into(),
            "Verified\n application".into(),
            None,
            None,
            Some(vec![executable.to_string_lossy().into_owned()]),
        )
    }

    fn flatpak_application(id: &str, alias: Option<&str>) -> crate::model::Application {
        let application = crate::model::Application::new(
            format!("{id}.desktop"),
            "Sandboxed application".into(),
            None,
            None,
            Some(vec![
                "/usr/bin/flatpak".into(),
                "run".into(),
                "--branch=stable".into(),
                id.into(),
            ]),
        );
        alias.map_or(application.clone(), |alias| {
            application.with_identity_alias(alias)
        })
    }

    fn synthetic_flatpak_process(id: &str) -> ProcessIdentity {
        ProcessIdentity {
            pid: std::process::id(),
            start_time: process_start_time(std::process::id()).unwrap(),
            device: 1,
            inode: 2,
            application: None,
            application_name: None,
            desktop_entry: None,
            catalog_generation: 0,
            protected: false,
            flatpak_application: Some(id.into()),
        }
    }

    #[test]
    fn flatpak_identity_requires_sandbox_catalog_and_surface_corroboration() {
        let id = "org.example.Editor";
        assert!(shared_runtime_executable(std::path::Path::new(
            "/usr/bin/flatpak"
        )));
        assert!(shared_runtime_executable(std::path::Path::new(
            "/usr/bin/bwrap"
        )));
        let catalog = DesktopCatalog::build(
            7,
            &[flatpak_application(id, Some("ExampleEditor"))],
            std::ffi::OsStr::new("/usr/bin"),
        );
        let mut wayland = synthetic_flatpak_process(&id.to_ascii_lowercase());
        catalog.corroborate(&mut wayland, Some(id));
        assert_eq!(
            wayland.application.as_deref(),
            Some("linux-flatpak:org.example.editor")
        );
        assert_eq!(
            wayland.desktop_entry.as_deref(),
            Some("org.example.Editor.desktop")
        );

        let mut xwayland = synthetic_flatpak_process(&id.to_ascii_lowercase());
        catalog.corroborate(&mut xwayland, Some("ExampleEditor"));
        assert_eq!(xwayland.application, wayland.application);

        for claim in [None, Some("org.example.Other"), Some("flatpak")] {
            let mut forged = synthetic_flatpak_process(&id.to_ascii_lowercase());
            catalog.corroborate(&mut forged, claim);
            assert!(forged.application.is_none());
            assert!(forged.desktop_entry.is_none());
        }
    }

    #[test]
    fn transient_inheritance_requires_the_exact_live_process_incarnation() {
        let current = ProcessIdentity::inspect(std::process::id()).unwrap();
        let same = current.clone();
        assert!(current.same_current_process(&same));

        let mut stale = same.clone();
        stale.start_time = stale.start_time.saturating_add(1);
        assert!(!current.same_current_process(&stale));

        let mut different = same;
        different.pid = different.pid.saturating_add(1);
        assert!(!current.same_current_process(&different));
    }

    #[test]
    fn ambiguous_flatpak_catalog_never_establishes_application_scope() {
        let id = "org.example.Editor";
        let catalog = DesktopCatalog::build(
            8,
            &[
                flatpak_application(id, None),
                crate::model::Application::new(
                    "org.example.Second.desktop".into(),
                    "Second".into(),
                    None,
                    None,
                    Some(vec!["flatpak".into(), "run".into(), id.into()]),
                )
                .with_identity_alias(id),
            ],
            std::ffi::OsStr::new("/usr/bin"),
        );
        let mut process = synthetic_flatpak_process(&id.to_ascii_lowercase());
        catalog.corroborate(&mut process, Some(id));
        assert!(process.application.is_none());
    }

    #[test]
    fn launch_acknowledgement_requires_a_current_verified_process_incarnation() {
        let _fixture = EXECUTABLE_FIXTURES.lock().unwrap();
        assert!(WindowIdentity::Pending.current_process_id().is_none());
        assert!(WindowIdentity::Unavailable.current_process_id().is_none());
        let mut child = std::process::Command::new("/usr/bin/sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let pid = child.id();
        let mut process = ProcessIdentity::inspect(pid).unwrap();
        process.start_time = process.start_time.saturating_add(1);
        assert!(
            WindowIdentity::Verified(process)
                .current_process_id()
                .is_none()
        );
        let process = ProcessIdentity::inspect(pid).unwrap();
        let identity = WindowIdentity::Verified(process);
        assert_eq!(identity.current_process_id(), Some(pid));
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(
            identity.current_process_id().is_none(),
            "exited process cannot satisfy a pending launch"
        );
    }

    #[test]
    fn full_session_pins_native_runtimes_without_granting_application_identity() {
        let _fixture = EXECUTABLE_FIXTURES.lock().unwrap();
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("sh");
        fs::copy("/usr/bin/sleep", &executable).unwrap();
        let mut command = std::process::Command::new(&executable);
        command.arg("30");
        assert!(LaunchExecutable::prepare(&command).is_none());
        let (proof, mut pinned) = LaunchExecutable::prepare_full_session(&command)
            .unwrap()
            .unwrap();
        assert!(proof.application.is_none());
        fs::remove_file(&executable).unwrap();
        fs::copy("/usr/bin/true", &executable).unwrap();
        let mut child = pinned.spawn().unwrap();
        let running = child.try_wait().unwrap().is_none();
        let _ = child.kill();
        child.wait().unwrap();
        assert!(running);
        let protected = directory.path().join("pinentry-fixture");
        fs::copy("/usr/bin/true", &protected).unwrap();
        assert!(
            LaunchExecutable::prepare_full_session(&std::process::Command::new(protected)).is_err()
        );
    }

    #[test]
    #[ignore = "native execute-only acceptance: run as an unprivileged user with /usr/bin/true and /usr/bin/false"]
    fn full_session_pins_execute_only_image_across_path_replacement() {
        let _fixture = EXECUTABLE_FIXTURES.lock().unwrap();
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        // Preserve argv[0] dispatch on systems shipping multicall coreutils.
        let executable = directory.path().join("false");
        fs::copy("/usr/bin/true", &executable).unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o111)).unwrap();
        assert_eq!(
            fs::File::open(&executable).unwrap_err().kind(),
            std::io::ErrorKind::PermissionDenied,
            "fixture must actually deny header reads"
        );
        let original = std::process::Command::new(&executable);
        assert!(LaunchExecutable::prepare(&original).is_none());
        let (proof, mut pinned) = LaunchExecutable::prepare_full_session(&original)
            .unwrap()
            .unwrap();
        assert!(proof.application.is_none());
        fs::remove_file(&executable).unwrap();
        fs::copy("/usr/bin/false", &executable).unwrap();
        assert!(
            !std::process::Command::new(&executable)
                .status()
                .unwrap()
                .success()
        );
        assert!(
            pinned.status().unwrap().success(),
            "must execute retained true, not replacement false"
        );
    }

    #[test]
    fn launch_protection_follows_symlinks_and_path_resolution() {
        let _fixture = EXECUTABLE_FIXTURES.lock().unwrap();
        let directory = tempfile::tempdir().unwrap();
        let protected = directory.path().join("pinentry-native-fixture");
        fs::copy("/usr/bin/true", &protected).unwrap();
        let alias = directory.path().join("ordinary-alias");
        std::os::unix::fs::symlink(&protected, &alias).unwrap();
        assert!(protected_launch_target(&std::process::Command::new(&alias)));
        let mut searched = std::process::Command::new("ordinary-alias");
        searched.env("PATH", directory.path());
        assert!(protected_launch_target(&searched));
        let mut relative = std::process::Command::new("./ordinary-alias");
        relative.current_dir(directory.path());
        assert!(protected_launch_target(&relative));
        assert!(!protected_launch_target(&std::process::Command::new(
            "/usr/bin/true"
        )));
    }

    #[test]
    fn application_launch_pins_the_verified_image_across_path_replacement() {
        let _fixture = EXECUTABLE_FIXTURES.lock().unwrap();
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("native-launch-fixture");
        fs::copy("/usr/bin/sleep", &executable).unwrap();
        let mut original = std::process::Command::new(&executable);
        original.arg("30").env_remove("NICKEL_SESSION_TOKEN");
        let (proof, mut pinned) = LaunchExecutable::prepare(&original).unwrap();
        fs::remove_file(&executable).unwrap();
        fs::copy("/usr/bin/true", &executable).unwrap();
        let mut child = pinned.spawn().unwrap();
        let observed = ProcessIdentity::inspect(child.id());
        let still_running = child.try_wait().unwrap().is_none();
        let _ = child.kill();
        child.wait().unwrap();
        assert!(
            still_running,
            "replacement executable was launched instead of pinned sleep"
        );
        assert_eq!(
            observed.unwrap().application.as_deref(),
            proof.application.as_deref()
        );
        assert!(
            pinned
                .get_envs()
                .any(|(name, value)| name == "NICKEL_SESSION_TOKEN" && value.is_none())
        );
    }

    #[test]
    fn script_snapshot_survives_replacement_mutation_and_delayed_interpreter_open() {
        let _fixture = EXECUTABLE_FIXTURES.lock().unwrap();
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let interpreter = directory.path().join("delayed-interpreter");
        fs::write(&interpreter, b"#!/bin/sh\nwhile [ ! -e \"$SCRIPT_RELEASE\" ]; do /usr/bin/sleep .01; done\nexec /bin/sh \"$@\"\n").unwrap();
        fs::set_permissions(&interpreter, fs::Permissions::from_mode(0o700)).unwrap();
        let script = directory.path().join("script");
        fs::write(
            &script,
            format!(
                "#!{}\nprintf '%s|%s|%s|%s' \"$1\" \"$SCRIPT_MARKER\" \"$PWD\" \"$0\"\n",
                interpreter.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
        let release = directory.path().join("release");
        let mut original = std::process::Command::new(&script);
        original
            .arg("spaced argument")
            .current_dir(directory.path())
            .env("SCRIPT_MARKER", "kept")
            .env("SCRIPT_RELEASE", &release)
            .env_remove("NICKEL_SESSION_TOKEN");
        let (proof, mut pinned) = LaunchExecutable::prepare_full_session(&original)
            .unwrap()
            .unwrap();
        assert!(proof.application.is_none());
        let snapshot_fd: i32 = std::path::Path::new(pinned.get_program())
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .parse()
            .unwrap();
        // SAFETY: pinned owns this live writable memfd, and the fixed byte slice
        // is readable for its declared length. Seals must reject even its owner.
        let write_result =
            unsafe { nix::libc::pwrite(snapshot_fd, b"changed".as_ptr().cast(), 7, 0) };
        assert_eq!(write_result, -1);
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(nix::libc::EPERM)
        );
        let saved = directory.path().join("original-inode");
        fs::rename(&script, &saved).unwrap();
        fs::write(&saved, b"#!/bin/sh\nexit 91\n").unwrap();
        fs::remove_file(&saved).unwrap();
        fs::write(&script, b"#!/bin/sh\nexit 92\n").unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
        pinned
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let child = pinned.spawn().unwrap();
        // Drop every parent-side descriptor before the delayed interpreter opens
        // its script operand. Only the child's inherited sealed fd can supply it.
        drop(pinned);
        drop(proof);
        fs::write(&release, b"ready").unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let expected = format!(
            "spaced argument|kept|{}|/proc/self/fd/",
            directory.path().display()
        );
        assert!(
            String::from_utf8(output.stdout)
                .unwrap()
                .starts_with(&expected)
        );
    }

    #[test]
    fn script_snapshot_preserves_kernel_shebang_option_and_execute_permission() {
        let _fixture = EXECUTABLE_FIXTURES.lock().unwrap();
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let script = directory.path().join("script");
        fs::write(&script, b"#!/bin/sh -e\nfalse\nexit 0\n").unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
        let original = std::process::Command::new(&script);
        let (_proof, mut pinned) = LaunchExecutable::prepare_full_session(&original)
            .unwrap()
            .unwrap();
        assert!(
            !pinned.status().unwrap().success(),
            "kernel must preserve the shebang -e option"
        );
        fs::set_permissions(&script, fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(
            pinned.spawn().unwrap_err().kind(),
            std::io::ErrorKind::PermissionDenied,
            "snapshot cannot bypass execute permission removed before dispatch"
        );
        fs::set_permissions(&script, fs::Permissions::from_mode(0o410)).unwrap();
        assert_eq!(
            LaunchExecutable::prepare_full_session(&original).is_ok(),
            std::process::Command::new(&script).status().is_ok(),
            "effective-credential execute access must match native exec, not any mode execute bit"
        );
    }

    #[test]
    fn failed_script_interpreter_exec_releases_parent_snapshot_on_drop() {
        let _fixture = EXECUTABLE_FIXTURES.lock().unwrap();
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let script = directory.path().join("script");
        fs::write(
            &script,
            format!(
                "#!{}\nexit 0\n",
                directory.path().join("missing-interpreter").display()
            ),
        )
        .unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
        let (proof, mut pinned) =
            LaunchExecutable::prepare_full_session(&std::process::Command::new(&script))
                .unwrap()
                .unwrap();
        let descriptor = std::path::PathBuf::from(pinned.get_program());
        let identity = fs::metadata(&descriptor).unwrap();
        assert_eq!(
            pinned.spawn().unwrap_err().kind(),
            std::io::ErrorKind::NotFound
        );
        drop(pinned);
        drop(proof);
        assert!(
            !fs::metadata(descriptor).is_ok_and(
                |current| current.dev() == identity.dev() && current.ino() == identity.ino()
            ),
            "failed exec must not retain its script snapshot after preparation is dropped"
        );
    }

    #[test]
    fn script_snapshot_rejects_oversized_and_unknown_formats_without_raw_fallback() {
        let _fixture = EXECUTABLE_FIXTURES.lock().unwrap();
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let script = directory.path().join("script");
        let mut bytes = vec![b'#'; MAX_LAUNCH_SCRIPT_BYTES + 1];
        bytes[..10].copy_from_slice(b"#!/bin/sh\n");
        fs::write(&script, bytes).unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(
            LaunchExecutable::prepare_full_session(&std::process::Command::new(&script)).is_err()
        );
        fs::write(&script, b"unrecognized executable format\n").unwrap();
        assert!(
            LaunchExecutable::prepare_full_session(&std::process::Command::new(script)).is_err()
        );
    }

    #[test]
    fn application_launch_does_not_assign_script_or_shared_runtime_authority() {
        let _fixture = EXECUTABLE_FIXTURES.lock().unwrap();
        let directory = tempfile::tempdir().unwrap();
        let script = directory.path().join("script");
        fs::write(&script, b"#!/bin/sh\nexit 0\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(LaunchExecutable::prepare(&std::process::Command::new(script)).is_none());
        assert!(LaunchExecutable::prepare(&std::process::Command::new("/bin/sh")).is_none());
    }

    #[test]
    fn desktop_name_requires_unique_matching_executable_without_changing_authority() {
        let _fixture = EXECUTABLE_FIXTURES.lock().unwrap();
        let executable = std::env::current_exe().unwrap();
        let mut identity = ProcessIdentity::inspect(std::process::id()).unwrap();
        let original_authority = identity.application.clone();
        let catalog = DesktopCatalog::build(7, &[application("fixture", &executable)], "".as_ref());
        catalog.corroborate(&mut identity, None);
        assert_eq!(identity.desktop_entry.as_deref(), Some("fixture"));
        assert_eq!(
            identity.application_name.as_deref(),
            Some("Verified application")
        );
        assert_eq!(identity.catalog_generation, 7);
        assert_eq!(identity.application, original_authority);

        let mut ambiguous = ProcessIdentity::inspect(std::process::id()).unwrap();
        DesktopCatalog::build(
            8,
            &[
                application("first", &executable),
                application("second", &executable),
            ],
            "".as_ref(),
        )
        .corroborate(&mut ambiguous, None);
        assert!(ambiguous.desktop_entry.is_none());
        assert_eq!(ambiguous.application, original_authority);

        let mut runtime = ProcessIdentity::inspect(std::process::id()).unwrap();
        runtime.application = None;
        catalog.corroborate(&mut runtime, None);
        assert!(runtime.desktop_entry.is_none());
        assert!(runtime.application.is_none());
    }

    #[test]
    fn desktop_matching_respects_path_shadowing_and_executable_replacement() {
        let _fixture = EXECUTABLE_FIXTURES.lock().unwrap();
        use std::os::unix::fs::symlink;
        let directory = tempfile::tempdir().unwrap();
        let first = directory.path().join("first");
        let second = directory.path().join("second");
        fs::create_dir(&first).unwrap();
        fs::create_dir(&second).unwrap();
        symlink("/bin/sleep", first.join("fixture")).unwrap();
        symlink(std::env::current_exe().unwrap(), second.join("fixture")).unwrap();
        let path = std::env::join_paths([&first, &second]).unwrap();
        let mut identity = ProcessIdentity::inspect(std::process::id()).unwrap();
        DesktopCatalog::build(1, &[application("fixture", "fixture".as_ref())], &path)
            .corroborate(&mut identity, None);
        assert!(identity.desktop_entry.is_none());

        let catalog = DesktopCatalog::build(
            2,
            &[application("fixture", &second.join("fixture"))],
            "".as_ref(),
        );
        fs::remove_file(second.join("fixture")).unwrap();
        symlink("/bin/sleep", second.join("fixture")).unwrap();
        catalog.corroborate(&mut identity, None);
        assert!(
            identity.desktop_entry.is_none(),
            "a stale catalog cannot label a replacement executable"
        );
    }

    #[test]
    fn unlinking_an_executed_file_does_not_revoke_the_unchanged_running_image() {
        let _fixture = EXECUTABLE_FIXTURES.lock().unwrap();
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("fixture-application");
        fs::copy("/bin/sleep", &executable).unwrap();
        let mut child = std::process::Command::new(&executable)
            .arg("10")
            .spawn()
            .unwrap();
        let proof = ProcessIdentity::inspect(child.id()).unwrap();
        fs::remove_file(&executable).unwrap();
        let still_current = proof.is_current();
        let _ = child.kill();
        let _ = child.wait();
        assert!(
            still_current,
            "the process still runs the originally verified file"
        );
    }

    #[test]
    fn exec_in_the_same_live_process_invalidates_previous_application_authority() {
        let _fixture = EXECUTABLE_FIXTURES.lock().unwrap();
        use std::{
            io::Write,
            process::{Command, Stdio},
            time::{Duration, Instant},
        };
        let mut child = Command::new("/bin/sh")
            .args(["-c", "read -r trigger; exec /bin/sleep 10"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let proof = ProcessIdentity::inspect(child.id()).unwrap();
        assert!(proof.is_current());
        assert!(
            proof.application.is_none(),
            "a shared interpreter is not an application identity"
        );
        child
            .stdin
            .take()
            .unwrap()
            .write_all(b"continue\n")
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while proof.is_current() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        let changed = !proof.is_current();
        let _ = child.kill();
        let _ = child.wait();
        assert!(
            changed,
            "exec must not inherit the previous executable's authority"
        );
    }

    #[test]
    fn live_process_evidence_detects_stale_incarnations_and_redacts_executable_paths_from_ids() {
        let _fixture = EXECUTABLE_FIXTURES.lock().unwrap();
        let mut identity = ProcessIdentity::inspect(std::process::id()).unwrap();
        assert!(identity.is_current());
        assert!(
            identity
                .application
                .as_deref()
                .unwrap()
                .starts_with("linux-executable:")
        );
        assert!(!identity.application.as_deref().unwrap().contains('/'));
        identity.start_time = identity.start_time.saturating_add(1);
        assert!(!identity.is_current());
        assert!(WindowIdentity::Verified(identity).is_protected());
    }

    #[test]
    fn permission_and_credential_processes_remain_protected_after_executable_replacement() {
        let _fixture = EXECUTABLE_FIXTURES.lock().unwrap();
        for name in [
            "nickel-settings",
            "nickel-settings (deleted)",
            "pinentry-qt",
            "polkit-kde-authentication-agent-1",
        ] {
            assert!(protected_executable(&PathBuf::from("/usr/bin").join(name)));
        }
        assert!(!protected_executable(std::path::Path::new(
            "/usr/bin/konsole"
        )));
        assert!(WindowIdentity::Pending.is_protected());
        assert!(WindowIdentity::Unavailable.application().is_none());
        assert!(shared_runtime_executable(std::path::Path::new(
            "/usr/bin/python3.14"
        )));
        assert!(shared_runtime_executable(std::path::Path::new(
            "/usr/bin/electron"
        )));
        assert!(!shared_runtime_executable(std::path::Path::new(
            "/usr/bin/konsole"
        )));
    }
}
