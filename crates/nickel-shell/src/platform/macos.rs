use std::{
    ffi::{CStr, c_char, c_void},
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        OnceLock,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread,
};

pub use super::unsupported::{
    NotificationFeed, TrayFeed, activate_wifi_network, audio_status, bluetooth_status,
    capture_pointer, configure_volume_osd_window, execute_run_command,
    launcher_has_foreground_focus, network_status, release_pointer, select_audio_device,
    set_audio_volume, set_bluetooth_discovery, set_bluetooth_powered, set_wifi_enabled,
    show_window_system_menu, toggle_bluetooth_device, update_panel_fullscreen_state, wallpaper,
};
use crate::{
    launcher::Launcher,
    model::{
        Application, ApplicationDiscovery, ApplicationId, OpenWindow, WindowId, WindowPreview,
    },
    platform::{FeedState, GlobalShortcut, ShellCommand, WindowAction},
};

const OPTION_KEY: u32 = 1 << 11;
const SPACE_KEY: u32 = 0x31;
const R_KEY: u32 = 0x0f;
const LAUNCHER_HOTKEY_ID: u32 = 1;
const RUN_HOTKEY_ID: u32 = 2;
const HOTKEY_SIGNATURE: u32 = four_char_code(*b"Nikl");
const EVENT_CLASS_KEYBOARD: u32 = four_char_code(*b"keyb");
const EVENT_HOTKEY_PRESSED: u32 = 5;
const EVENT_PARAM_DIRECT_OBJECT: u32 = four_char_code(*b"----");
const TYPE_EVENT_HOTKEY_ID: u32 = four_char_code(*b"hkid");
const WINDOW_LIST_ON_SCREEN_ONLY: u32 = 1;
const WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS: u32 = 1 << 4;
const CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
const CF_NUMBER_SINT32: i32 = 3;
const CF_NUMBER_SINT64: i32 = 4;
const AX_ERROR_SUCCESS: i32 = 0;
const AX_VALUE_CGPOINT: i32 = 1;
const AX_VALUE_CGSIZE: i32 = 2;

static SHORTCUT_SENDER: OnceLock<Sender<GlobalShortcut>> = OnceLock::new();
static LAUNCHER_VISIBLE: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Debug)]
struct MacWindow {
    id: u64,
    pid: i32,
    owner: String,
    title: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

pub fn applications() -> Vec<Application> {
    let mut applications = discover_app_bundles()
        .into_iter()
        .map(|bundle| {
            Application::new(
                bundle.id,
                bundle.name,
                Some(bundle.path.to_string_lossy().into_owned()),
                None,
                Some(vec![
                    "open".into(),
                    bundle.path.to_string_lossy().into_owned(),
                ]),
            )
        })
        .collect::<Vec<_>>();
    for window in visible_windows() {
        let id = application_id(&window.owner, window.pid);
        if applications
            .iter()
            .any(|application| application.application_id() == &id)
        {
            continue;
        }
        applications.push(Application::new(
            id.as_str().to_owned(),
            window.owner,
            bundle_for_pid(window.pid).map(|path| path.to_string_lossy().into_owned()),
            None,
            None,
        ));
    }
    applications.sort_by(|left, right| left.name().cmp(right.name()));
    applications
}

pub fn application_discovery() -> ApplicationDiscovery {
    ApplicationDiscovery::ready(applications())
}

pub fn launch_application(application: &Application) -> Result<Option<u32>, super::LaunchError> {
    application
        .launch()
        .map(|child| Some(child.id()))
        .map_err(|error| super::LaunchError::Platform(error.to_string()))
}

pub fn application_icon(reference: &str) -> Option<image::RgbaImage> {
    let bundle = Path::new(reference);
    let icon = bundle_icon_path(bundle)?;
    load_macos_icon(&icon)
}

#[derive(Clone, Debug)]
struct AppBundle {
    id: String,
    name: String,
    path: PathBuf,
}

fn discover_app_bundles() -> Vec<AppBundle> {
    let mut bundles = Vec::new();
    for root in application_roots() {
        collect_app_bundles(&root, 0, &mut bundles);
    }
    bundles.sort_by(|left, right| left.name.cmp(&right.name));
    bundles.dedup_by(|left, right| left.id == right.id);
    bundles
}

fn application_roots() -> Vec<PathBuf> {
    let mut roots = vec![PathBuf::from("/Applications")];
    if let Some(home) = std::env::var_os("HOME") {
        roots.push(PathBuf::from(home).join("Applications"));
    }
    roots
}

fn collect_app_bundles(root: &Path, depth: u8, bundles: &mut Vec<AppBundle>) {
    if depth > 2 {
        return;
    }
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("app"))
        {
            bundles.push(app_bundle(path));
        } else if path.is_dir() {
            collect_app_bundles(&path, depth + 1, bundles);
        }
    }
}

fn app_bundle(path: PathBuf) -> AppBundle {
    let info = path.join("Contents/Info.plist");
    let id = plist_value(&info, "CFBundleIdentifier")
        .unwrap_or_else(|| format!("macos:{}", path.to_string_lossy()));
    let name = plist_value(&info, "CFBundleDisplayName")
        .or_else(|| plist_value(&info, "CFBundleName"))
        .unwrap_or_else(|| {
            path.file_stem()
                .and_then(|name| name.to_str())
                .unwrap_or("Application")
                .to_owned()
        });
    AppBundle { id, name, path }
}

fn plist_value(path: &Path, key: &str) -> Option<String> {
    let output = Command::new("/usr/bin/plutil")
        .args(["-extract", key, "raw", "-o", "-"])
        .arg(path)
        .output()
        .ok()?;
    output.status.success().then(|| {
        String::from_utf8_lossy(&output.stdout)
            .trim()
            .trim_matches('"')
            .to_owned()
    })
}

fn bundle_for_pid(pid: i32) -> Option<PathBuf> {
    let output = Command::new("/bin/ps")
        .args(["-p", &pid.to_string(), "-o", "args="])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let command = String::from_utf8_lossy(&output.stdout);
    let app_end = command.find(".app/").map(|index| index + ".app".len())?;
    let path = PathBuf::from(&command[..app_end]);
    path.exists().then_some(path)
}

fn bundle_for_window(window: &MacWindow) -> Option<PathBuf> {
    bundle_for_pid(window.pid).or_else(|| bundle_for_owner(&window.owner))
}

fn bundle_for_owner(owner: &str) -> Option<PathBuf> {
    let owner = normalized_app_name(owner);
    discover_app_bundles()
        .into_iter()
        .find(|bundle| normalized_app_name(&bundle.name) == owner)
        .map(|bundle| bundle.path)
}

fn bundle_icon_path(bundle: &Path) -> Option<PathBuf> {
    let info = bundle.join("Contents/Info.plist");
    let mut icon = plist_value(&info, "CFBundleIconFile")?;
    if Path::new(&icon).extension().is_none() {
        icon.push_str(".icns");
    }
    Some(bundle.join("Contents/Resources").join(icon)).filter(|path| path.exists())
}

fn load_macos_icon(path: &Path) -> Option<image::RgbaImage> {
    if path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("png"))
    {
        return image::open(path).ok().map(image::DynamicImage::into_rgba8);
    }
    let cache = user_cache_dir().join("icons");
    fs::create_dir_all(&cache).ok()?;
    let metadata = fs::metadata(path).ok();
    let modified = metadata
        .as_ref()
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|modified| modified.as_secs())
        .unwrap_or_default();
    let length = metadata.map(|metadata| metadata.len()).unwrap_or_default();
    let output = cache.join(format!(
        "{:016x}-{modified:x}-{length:x}.png",
        stable_hash(path)
    ));
    if !output.exists() {
        let status = Command::new("/usr/bin/sips")
            .args(["-s", "format", "png"])
            .arg(path)
            .arg("--out")
            .arg(&output)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .ok()?;
        if !status.success() {
            return None;
        }
    }
    image::open(output)
        .ok()
        .map(image::DynamicImage::into_rgba8)
}

fn stable_hash(path: &Path) -> u64 {
    path.to_string_lossy()
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
        })
}

fn user_cache_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join("Library/Caches/Nickel"))
        .unwrap_or_else(|| std::env::temp_dir().join("nickel"))
}

pub struct WindowFeed;

impl WindowFeed {
    pub fn new() -> Self {
        Self
    }

    pub fn launcher_visible(&self) -> Option<bool> {
        None
    }

    pub fn snapshot(&self, _: &Launcher) -> FeedState<Vec<OpenWindow>> {
        FeedState::Ready(
            visible_windows()
                .into_iter()
                .enumerate()
                .map(|(index, window)| OpenWindow {
                    id: WindowId(window.id),
                    application_id: Some(application_id(&window.owner, window.pid)),
                    active: index == 0,
                    title: if window.title.is_empty() {
                        window.owner
                    } else {
                        window.title
                    },
                })
                .collect(),
        )
    }

    pub fn workspaces(&self) -> FeedState<Vec<super::WorkspaceSummary>> {
        FeedState::Ready(Vec::new())
    }

    pub fn preview(&self, _: WindowId) -> Option<WindowPreview> {
        None
    }

    pub fn supports_previews(&self) -> bool {
        false
    }

    pub fn icon(&self, window: WindowId) -> Option<image::RgbaImage> {
        let target = visible_windows()
            .into_iter()
            .find(|candidate| candidate.id == window.0)?;
        let bundle = bundle_for_window(&target)?;
        application_icon(&bundle.to_string_lossy())
    }
}

pub fn send_shell_command(command: ShellCommand) -> bool {
    let ShellCommand::WindowAction { window, action } = command else {
        return false;
    };
    let Some(target) = visible_windows()
        .into_iter()
        .find(|candidate| candidate.id == window.0)
    else {
        return false;
    };
    match action {
        WindowAction::Activate => activate_window(&target),
        WindowAction::Minimize => set_window_minimized(&target, true),
        WindowAction::Close => close_window(&target),
        WindowAction::Maximize => false,
    }
}

pub fn register_session_shell() -> Result<(), super::SessionRequestError> {
    Ok(())
}

pub fn launcher_hotkey_receiver() -> super::GlobalShortcutFeed {
    let (sender, receiver) = mpsc::channel();
    let _ = SHORTCUT_SENDER.set(sender.clone());
    if let Err(error) = thread::Builder::new()
        .name("nickel-macos-hotkeys".into())
        .spawn(move || run_hotkey_loop(sender))
    {
        tracing::warn!(%error, "failed to start macOS hotkey listener");
    }
    super::GlobalShortcutFeed {
        receiver,
        ownership: nickel_input::global::ShortcutOwnership::OperatingSystem,
        capability: nickel_input::global::ShortcutCapability::Unavailable(
            nickel_input::global::UnavailableReason::UnsupportedPlatform,
        ),
    }
}

pub fn launcher_visibility_applied(visible: bool) {
    LAUNCHER_VISIBLE.store(visible, Ordering::Release);
}

pub fn handle_focused_shortcut(_: nickel_core::hotkeys::KeyCode, _: nickel_core::hotkeys::KeyEdge) {
}

fn run_hotkey_loop(_sender: Sender<GlobalShortcut>) {
    let target = unsafe { GetApplicationEventTarget() };
    let event_type = EventTypeSpec {
        event_class: EVENT_CLASS_KEYBOARD,
        event_kind: EVENT_HOTKEY_PRESSED,
    };
    let install_status = unsafe {
        InstallEventHandler(
            target,
            Some(hotkey_handler),
            1,
            &event_type,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if install_status != 0 {
        tracing::warn!(install_status, "failed to install macOS hotkey handler");
        return;
    }

    let launcher_status = register_hotkey(SPACE_KEY, LAUNCHER_HOTKEY_ID);
    let run_status = register_hotkey(R_KEY, RUN_HOTKEY_ID);
    if launcher_status != 0 {
        tracing::warn!(
            launcher_status,
            "failed to register Option+Space for Nickel launcher"
        );
    }
    if run_status != 0 {
        tracing::warn!(run_status, "failed to register Option+R for Nickel Run");
    }
    if launcher_status == 0 || run_status == 0 {
        tracing::info!(
            launcher_registered = launcher_status == 0,
            run_registered = run_status == 0,
            "registered macOS Nickel hotkeys"
        );
        unsafe { RunApplicationEventLoop() };
    }
}

fn register_hotkey(key_code: u32, id: u32) -> i32 {
    let hotkey_id = EventHotKeyID {
        signature: HOTKEY_SIGNATURE,
        id,
    };
    let mut reference = std::ptr::null_mut();
    unsafe {
        RegisterEventHotKey(
            key_code,
            OPTION_KEY,
            hotkey_id,
            GetApplicationEventTarget(),
            0,
            &mut reference,
        )
    }
}

extern "C" fn hotkey_handler(
    _next_handler: *mut c_void,
    event: *mut c_void,
    _user_data: *mut c_void,
) -> i32 {
    let mut hotkey_id = EventHotKeyID {
        signature: 0,
        id: 0,
    };
    let status = unsafe {
        GetEventParameter(
            event,
            EVENT_PARAM_DIRECT_OBJECT,
            TYPE_EVENT_HOTKEY_ID,
            std::ptr::null(),
            std::mem::size_of::<EventHotKeyID>() as u32,
            std::ptr::null_mut(),
            (&mut hotkey_id as *mut EventHotKeyID).cast(),
        )
    };
    if status != 0 || hotkey_id.signature != HOTKEY_SIGNATURE {
        return status;
    }
    if let Some(sender) = SHORTCUT_SENDER.get() {
        match hotkey_id.id {
            LAUNCHER_HOTKEY_ID => {
                let shortcut = if LAUNCHER_VISIBLE.load(Ordering::Acquire) {
                    GlobalShortcut::HideLauncher
                } else {
                    GlobalShortcut::ShowLauncher
                };
                let _ = sender.send(shortcut);
            }
            RUN_HOTKEY_ID => {
                let _ = sender.send(GlobalShortcut::ShowRun);
            }
            _ => {}
        }
    }
    0
}

fn visible_windows() -> Vec<MacWindow> {
    let options = WINDOW_LIST_ON_SCREEN_ONLY | WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS;
    let array = unsafe { CGWindowListCopyWindowInfo(options, 0) };
    if array.is_null() {
        return Vec::new();
    }
    let count = unsafe { CFArrayGetCount(array) };
    let mut windows = Vec::new();
    let own_pid = std::process::id() as i32;
    for index in 0..count {
        let dictionary = unsafe { CFArrayGetValueAtIndex(array, index) };
        if dictionary.is_null() {
            continue;
        }
        let Some(window) = read_window(dictionary.cast()) else {
            continue;
        };
        if window.pid == own_pid
            || window.owner.eq_ignore_ascii_case("nickel")
            || window.owner.to_ascii_lowercase().contains("nickel")
            || window.width < 80.0
            || window.height < 40.0
            || window.y < 24.0 && window.height <= 64.0
        {
            continue;
        }
        windows.push(window);
    }
    unsafe { CFRelease(array.cast()) };
    windows
}

fn read_window(dictionary: CFDictionaryRef) -> Option<MacWindow> {
    if dictionary_i64(dictionary, unsafe { kCGWindowLayer })? != 0 {
        return None;
    }
    if dictionary_f64(dictionary, unsafe { kCGWindowAlpha }).unwrap_or(1.0) <= 0.0 {
        return None;
    }
    let id = dictionary_i64(dictionary, unsafe { kCGWindowNumber })? as u64;
    let pid = dictionary_i64(dictionary, unsafe { kCGWindowOwnerPID })? as i32;
    let owner = dictionary_string(dictionary, unsafe { kCGWindowOwnerName })
        .unwrap_or_else(|| format!("Process {pid}"));
    let title = dictionary_string(dictionary, unsafe { kCGWindowName }).unwrap_or_default();
    let bounds_dictionary = unsafe { CFDictionaryGetValue(dictionary, kCGWindowBounds.cast()) };
    if bounds_dictionary.is_null() {
        return None;
    }
    let mut rect = CGRect::default();
    if !unsafe { CGRectMakeWithDictionaryRepresentation(bounds_dictionary.cast(), &mut rect) } {
        return None;
    }
    Some(MacWindow {
        id,
        pid,
        owner,
        title,
        x: rect.origin.x,
        y: rect.origin.y,
        width: rect.size.width,
        height: rect.size.height,
    })
}

fn activate_window(window: &MacWindow) -> bool {
    let app = unsafe { AXUIElementCreateApplication(window.pid) };
    if app.is_null() {
        return false;
    }
    let _ = unsafe {
        AXUIElementSetAttributeValue(app, ax_frontmost_attribute(), kCFBooleanTrue.cast())
    };
    let raised = matching_ax_window(app, window).is_some_and(|ax_window| {
        let unminimized = set_ax_bool(ax_window, ax_minimized_attribute(), false);
        let raised =
            unsafe { AXUIElementPerformAction(ax_window, ax_raise_action()) } == AX_ERROR_SUCCESS;
        unsafe { CFRelease(ax_window.cast()) };
        unminimized || raised
    });
    unsafe { CFRelease(app.cast()) };
    raised
}

fn set_window_minimized(window: &MacWindow, minimized: bool) -> bool {
    let app = unsafe { AXUIElementCreateApplication(window.pid) };
    if app.is_null() {
        return false;
    }
    let changed = matching_ax_window(app, window).is_some_and(|ax_window| {
        let changed = set_ax_bool(ax_window, ax_minimized_attribute(), minimized);
        unsafe { CFRelease(ax_window.cast()) };
        changed
    });
    unsafe { CFRelease(app.cast()) };
    changed
}

fn close_window(window: &MacWindow) -> bool {
    let app = unsafe { AXUIElementCreateApplication(window.pid) };
    if app.is_null() {
        return false;
    }
    let closed = matching_ax_window(app, window).is_some_and(|ax_window| {
        let button = copy_ax_attribute(ax_window, ax_close_button_attribute());
        unsafe { CFRelease(ax_window.cast()) };
        button.is_some_and(|button| {
            let closed = unsafe { AXUIElementPerformAction(button.cast(), ax_press_action()) }
                == AX_ERROR_SUCCESS;
            unsafe { CFRelease(button) };
            closed
        })
    });
    unsafe { CFRelease(app.cast()) };
    closed
}

fn matching_ax_window(app: AXUIElementRef, window: &MacWindow) -> Option<AXUIElementRef> {
    let windows = copy_ax_attribute(app, ax_windows_attribute())?;
    let count = unsafe { CFArrayGetCount(windows.cast()) };
    let mut fallback = None;
    for index in 0..count {
        let candidate = unsafe { CFArrayGetValueAtIndex(windows.cast(), index) };
        if candidate.is_null() {
            continue;
        }
        let title = ax_string(candidate.cast(), ax_title_attribute()).unwrap_or_default();
        if (!window.title.is_empty() && title == window.title)
            || ax_bounds_match(candidate.cast(), window)
        {
            unsafe { CFRetain(candidate) };
            unsafe { CFRelease(windows) };
            return Some(candidate.cast());
        }
        if fallback.is_none() {
            unsafe { CFRetain(candidate) };
            fallback = Some(candidate.cast());
        }
    }
    unsafe { CFRelease(windows) };
    fallback
}

fn ax_bounds_match(window: AXUIElementRef, target: &MacWindow) -> bool {
    let Some(position) = copy_ax_attribute(window, ax_position_attribute()) else {
        return false;
    };
    let Some(size) = copy_ax_attribute(window, ax_size_attribute()) else {
        unsafe { CFRelease(position) };
        return false;
    };
    let mut point = CGPoint::default();
    let mut dimensions = CGSize::default();
    let position_ok = unsafe {
        AXValueGetValue(
            position.cast(),
            AX_VALUE_CGPOINT,
            (&mut point as *mut CGPoint).cast(),
        )
    };
    let size_ok = unsafe {
        AXValueGetValue(
            size.cast(),
            AX_VALUE_CGSIZE,
            (&mut dimensions as *mut CGSize).cast(),
        )
    };
    unsafe {
        CFRelease(position);
        CFRelease(size);
    }
    position_ok
        && size_ok
        && (point.x - target.x).abs() < 12.0
        && (point.y - target.y).abs() < 48.0
        && (dimensions.width - target.width).abs() < 24.0
        && (dimensions.height - target.height).abs() < 48.0
}

fn set_ax_bool(element: AXUIElementRef, attribute: CFStringRef, value: bool) -> bool {
    let cf_value = if value {
        unsafe { kCFBooleanTrue }
    } else {
        unsafe { kCFBooleanFalse }
    };
    (unsafe { AXUIElementSetAttributeValue(element, attribute, cf_value.cast()) })
        == AX_ERROR_SUCCESS
}

fn copy_ax_attribute(element: AXUIElementRef, attribute: CFStringRef) -> Option<CFTypeRef> {
    let mut value = std::ptr::null();
    let status = unsafe { AXUIElementCopyAttributeValue(element, attribute, &mut value) };
    (status == AX_ERROR_SUCCESS && !value.is_null()).then_some(value)
}

fn ax_string(element: AXUIElementRef, attribute: CFStringRef) -> Option<String> {
    let value = copy_ax_attribute(element, attribute)?;
    let string = cf_string(value.cast());
    unsafe { CFRelease(value) };
    string
}

fn dictionary_string(dictionary: CFDictionaryRef, key: CFStringRef) -> Option<String> {
    let value = unsafe { CFDictionaryGetValue(dictionary, key.cast()) };
    (!value.is_null())
        .then(|| cf_string(value.cast()))
        .flatten()
}

fn dictionary_i64(dictionary: CFDictionaryRef, key: CFStringRef) -> Option<i64> {
    let value = unsafe { CFDictionaryGetValue(dictionary, key.cast()) };
    if value.is_null() {
        return None;
    }
    let mut integer = 0_i64;
    if unsafe {
        CFNumberGetValue(
            value.cast(),
            CF_NUMBER_SINT64,
            (&mut integer as *mut i64).cast(),
        )
    } {
        return Some(integer);
    }
    let mut short = 0_i32;
    unsafe {
        CFNumberGetValue(
            value.cast(),
            CF_NUMBER_SINT32,
            (&mut short as *mut i32).cast(),
        )
    }
    .then_some(i64::from(short))
}

fn dictionary_f64(dictionary: CFDictionaryRef, key: CFStringRef) -> Option<f64> {
    dictionary_i64(dictionary, key).map(|value| value as f64)
}

fn cf_string(value: CFStringRef) -> Option<String> {
    let mut buffer = [0_i8; 1024];
    if unsafe {
        CFStringGetCString(
            value,
            buffer.as_mut_ptr(),
            buffer.len() as isize,
            CF_STRING_ENCODING_UTF8,
        )
    } {
        unsafe { CStr::from_ptr(buffer.as_ptr()) }
            .to_str()
            .ok()
            .map(str::to_owned)
    } else {
        None
    }
}

fn application_id(owner: &str, pid: i32) -> ApplicationId {
    if let Some(bundle) = bundle_for_pid(pid).or_else(|| bundle_for_owner(owner)) {
        let info = bundle.join("Contents/Info.plist");
        if let Some(id) = plist_value(&info, "CFBundleIdentifier") {
            return ApplicationId::new(id);
        }
    }
    ApplicationId::new(format!("macos:{}:{pid}", owner.to_ascii_lowercase()))
}

fn normalized_app_name(name: &str) -> String {
    name.chars()
        .filter(|character| {
            !matches!(
                *character,
                '\u{200e}'
                    | '\u{200f}'
                    | '\u{202a}'..='\u{202e}'
                    | '\u{2066}'..='\u{2069}'
            ) && !character.is_control()
        })
        .collect::<String>()
        .to_ascii_lowercase()
}

fn ax_windows_attribute() -> CFStringRef {
    cf_static_string("AXWindows\0")
}

fn ax_title_attribute() -> CFStringRef {
    cf_static_string("AXTitle\0")
}

fn ax_frontmost_attribute() -> CFStringRef {
    cf_static_string("AXFrontmost\0")
}

fn ax_minimized_attribute() -> CFStringRef {
    cf_static_string("AXMinimized\0")
}

fn ax_close_button_attribute() -> CFStringRef {
    cf_static_string("AXCloseButton\0")
}

fn ax_position_attribute() -> CFStringRef {
    cf_static_string("AXPosition\0")
}

fn ax_size_attribute() -> CFStringRef {
    cf_static_string("AXSize\0")
}

fn ax_raise_action() -> CFStringRef {
    cf_static_string("AXRaise\0")
}

fn ax_press_action() -> CFStringRef {
    cf_static_string("AXPress\0")
}

fn cf_static_string(value: &'static str) -> CFStringRef {
    unsafe {
        CFStringCreateWithCString(
            std::ptr::null(),
            value.as_ptr().cast::<c_char>(),
            CF_STRING_ENCODING_UTF8,
        )
    }
}

const fn four_char_code(bytes: [u8; 4]) -> u32 {
    ((bytes[0] as u32) << 24)
        | ((bytes[1] as u32) << 16)
        | ((bytes[2] as u32) << 8)
        | bytes[3] as u32
}

#[repr(C)]
#[derive(Clone, Copy)]
struct EventHotKeyID {
    signature: u32,
    id: u32,
}

#[repr(C)]
struct EventTypeSpec {
    event_class: u32,
    event_kind: u32,
}

type CFTypeRef = *const c_void;
type CFArrayRef = *const c_void;
type CFDictionaryRef = *const c_void;
type CFStringRef = *const c_void;
type AXUIElementRef = *const c_void;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CGSize {
    width: f64,
    height: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CGRect {
    origin: CGPoint,
    size: CGSize,
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFBooleanTrue: CFTypeRef;
    static kCFBooleanFalse: CFTypeRef;

    fn CFRelease(value: CFTypeRef);
    fn CFRetain(value: CFTypeRef) -> CFTypeRef;
    fn CFArrayGetCount(array: CFArrayRef) -> isize;
    fn CFArrayGetValueAtIndex(array: CFArrayRef, index: isize) -> CFTypeRef;
    fn CFDictionaryGetValue(dictionary: CFDictionaryRef, key: CFTypeRef) -> CFTypeRef;
    fn CFNumberGetValue(number: CFTypeRef, number_type: i32, value: *mut c_void) -> bool;
    fn CFStringCreateWithCString(
        allocator: CFTypeRef,
        value: *const c_char,
        encoding: u32,
    ) -> CFStringRef;
    fn CFStringGetCString(
        string: CFStringRef,
        buffer: *mut c_char,
        buffer_size: isize,
        encoding: u32,
    ) -> bool;
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    static kCGWindowNumber: CFStringRef;
    static kCGWindowOwnerPID: CFStringRef;
    static kCGWindowOwnerName: CFStringRef;
    static kCGWindowName: CFStringRef;
    static kCGWindowLayer: CFStringRef;
    static kCGWindowAlpha: CFStringRef;
    static kCGWindowBounds: CFStringRef;

    fn CGWindowListCopyWindowInfo(option: u32, relative_to_window: u32) -> CFArrayRef;
    fn CGRectMakeWithDictionaryRepresentation(
        dictionary: CFDictionaryRef,
        rect: *mut CGRect,
    ) -> bool;
}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXUIElementCreateApplication(pid: i32) -> AXUIElementRef;
    fn AXUIElementCopyAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: *mut CFTypeRef,
    ) -> i32;
    fn AXUIElementSetAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: CFTypeRef,
    ) -> i32;
    fn AXUIElementPerformAction(element: AXUIElementRef, action: CFStringRef) -> i32;
    fn AXValueGetValue(value: CFTypeRef, value_type: i32, output: *mut c_void) -> bool;
}

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn GetApplicationEventTarget() -> *mut c_void;
    fn InstallEventHandler(
        target: *mut c_void,
        handler: Option<extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> i32>,
        event_type_count: u32,
        event_types: *const EventTypeSpec,
        user_data: *mut c_void,
        handler_ref: *mut *mut c_void,
    ) -> i32;
    fn RegisterEventHotKey(
        hotkey_code: u32,
        hotkey_modifiers: u32,
        hotkey_id: EventHotKeyID,
        target: *mut c_void,
        options: u32,
        hotkey_ref: *mut *mut c_void,
    ) -> i32;
    fn GetEventParameter(
        event: *mut c_void,
        name: u32,
        desired_type: u32,
        actual_type: *const u32,
        buffer_size: u32,
        actual_size: *mut u32,
        data: *mut c_void,
    ) -> i32;
    fn RunApplicationEventLoop();
}
