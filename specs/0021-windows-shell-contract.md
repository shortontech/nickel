# Windows Shell Contract

## Goal

Maintain an explicit inventory of the desktop-shell responsibilities that become Nickel's when
`explorer.exe` is not running as the Windows shell. Use the inventory to prevent functionality
from being discovered only through manual regressions on a shell-only machine.

Microsoft documents how to replace Explorer through Shell Launcher, but does not publish one
complete compatibility contract for an interactive desktop replacement. The effective contract is
distributed across window messages, shell hooks, taskbar and notification-area APIs, COM
interfaces, Core Audio, WinRT, input services, and observable Explorer behavior.

## Status Vocabulary

- **Implemented**: Works without Explorer and has automated or recorded manual coverage.
- **Partial**: A useful path works, but compatibility or important behavior remains.
- **Missing**: Nickel does not yet own the responsibility.
- **Investigate**: Explorer's role or the appropriate supported Windows contract is not yet clear.

An item is not complete merely because it works while Explorer is also running.

## Contract Inventory

### Shell Ownership and Recovery

| Responsibility | Status | Windows contract or evidence | Required verification |
| --- | --- | --- | --- |
| Start as the configured interactive shell | Implemented | Winlogon `Shell`; Shell Launcher is an alternative deployment mechanism | Sign in with Nickel configured as shell and confirm no Explorer desktop process starts |
| Keep the shell restart policy explicit | Partial | Winlogon shell restart behavior | Crash and restart Nickel without silently changing the configured shell |
| Broadcast taskbar recreation | Partial | Registered `TaskbarCreated` message | Restart Nickel while tray applications remain alive and verify that icons register again |
| Provide a usable recovery path | Missing | Session process and Windows recovery tools | Recover Settings, networking, Task Manager, and Explorer without editing the registry blindly |
| Session shutdown, restart, sign-out, sleep, and lock | Missing | Session and power APIs | Exercise each operation from a shell-only session |
| Startup applications | Missing | Per-user and machine startup registration, Startup folders, scheduled startup | Compare a clean Explorer sign-in with a Nickel sign-in |

### Desktop, Displays, and Work Area

| Responsibility | Status | Windows contract or evidence | Required verification |
| --- | --- | --- | --- |
| Own the desktop surface and current wallpaper | Implemented | `IDesktopWallpaper`, DWM surface | Static and animated wallpaper; Fill, Fit, Stretch, Center, Tile, and Span |
| Keep the desktop behind application windows | Implemented | Native window ownership and z-order | Focus, minimize, show desktop, fullscreen exit, and shell restart |
| Reserve panel space for maximized windows | Implemented | AppBar messages and work area | Maximize native, borderless, terminal, and browser windows |
| Detect borderless fullscreen applications | Implemented | Window/frame bounds versus monitor bounds | Games and media windows hide the panel and restore it after focus leaves |
| Multiple monitors and hotplug | Partial | Monitor enumeration and display-change events | Primary/secondary changes, DisplayLink, add/remove, mixed scale, and negative coordinates |
| DPI and scale changes | Partial | Per-monitor DPI and taskbar recreation behavior | Move every shell surface between monitors with different scales |
| Display configuration UI | Partial | Display configuration APIs | Identify, select primary, apply layout, rotate, scale, and recover from invalid modes |

### Windows, Taskbar, and Switching

| Responsibility | Status | Windows contract or evidence | Required verification |
| --- | --- | --- | --- |
| Enumerate taskbar-eligible windows | Implemented | Top-level window enumeration, visibility, ownership, styles | Native, WinUI/.NET, Chromium, games, dialogs, and tool windows |
| Resolve application identity and icons | Partial | Executable/class icons, packaged-application identity | Win32, .NET, packaged Settings, UWP/WinUI, missing icons, and live icon changes |
| Group and cycle application windows | Implemented | Nickel policy over native windows | Multiple browser windows and filtered window lists |
| Track the actual foreground window | Implemented | Foreground-window APIs | Activation by mouse, keyboard, programmatic focus, and minimized restore |
| Activate, minimize, restore, maximize, move, resize, and close | Partial | Native window messages and placement APIs | Decorated and client-decorated windows, elevated apps, games, and hung windows |
| Alt+Tab and reverse cycling | Implemented | Nickel hotkey controller and nonactivating preview surface | Tap, hold, rapid release, click selection, restart timing, and missing previews |
| DWM live thumbnails | Implemented | DWM thumbnail APIs | Creation, repositioning, cleanup, minimize, close, and fullscreen |
| Taskbar progress, overlays, flashing, and jump lists | Missing | Taskbar COM interfaces and flash state | Test applications exercising each taskbar extension |
| Show desktop and window arrangement | Missing | Shell policy over window placement | Show/restore desktop, cascade, stack, and side-by-side where supported |

### Launcher, Search, Run, and Activation

| Responsibility | Status | Windows contract or evidence | Required verification |
| --- | --- | --- | --- |
| Bare Windows key launcher toggle | Implemented | `RegisterHotKey` plus shared hotkey state | Win, Win twice, panel click then Win, focused launcher, and focus loss |
| Preserve unregistered Windows-key chords | Partial | Selective registered hotkeys | Inventory Windows defaults and verify Nickel does not suppress unknown chords |
| Win+R and Run history | Implemented | Registered hotkey and Nickel Run UI | Text editing, spaces, paste, IME, history, paths, executables, and URIs |
| Index Start Menu and Desktop shortcuts | Implemented | User/machine Start Menu and Desktop shortcut folders | Recursive indexing, deduplication, renamed shortcuts, and icon extraction |
| Launch ordinary desktop applications | Implemented | Process and shell execution APIs | Arguments, working directory, elevation, files, URLs, and failures |
| Activate `ms-settings:` and packaged applications | Partial | Shell execution, app activation, packaged-app services | Network, display, Bluetooth, Windows Update, and unknown URI failures |
| Search documents, settings, commands, and recent items | Missing | Nickel search providers | Deterministic ranking and behavior without Windows Search UI |
| Clipboard and IME-aware text components | Partial | Winit text events, IME, clipboard | Composition, candidate placement, paste, selection, emoji, RTL, and accessibility |

### Notification Area and Notifications

| Responsibility | Status | Windows contract or evidence | Required verification |
| --- | --- | --- | --- |
| Host `Shell_NotifyIcon` tray registrations | Partial | `Shell_NotifyIcon`, `NOTIFYICONDATA`, Explorer compatibility messages | Add, modify, delete, process crash, restart, GUID and integer IDs, hidden state |
| Render stable tray icons without duplicates | Partial | Tray registration identity and icon ownership | Task Manager telemetry, animated icons, malformed icons, and stale cleanup |
| Dispatch left click, right click, keyboard selection, and context menus | Partial | Versioned tray callback semantics | Standard Explorer-compatible test app and Nickel's Rust tray test |
| System status icons | Partial | Network, volume, battery, Bluetooth, notifications | Hardware present/absent, offline, muted, charging, and unavailable services |
| Toast notification delivery and history | Missing | Windows notification platform and shell-owned presentation | Desktop and packaged applications, actions, dismissal, focus assist, expiration |
| Clock, calendar, locale, and date format | Partial | Windows locale and calendar settings | 12/24 hour, leading zero, short date, calendar variants, time-zone changes |

### Audio, Media, Network, and Hardware Controls

| Responsibility | Status | Windows contract or evidence | Required verification |
| --- | --- | --- | --- |
| Enumerate audio outputs and read/set master volume | Implemented | Windows Core Audio | Device arrival/removal, mute, default changes, and service restart |
| Select the default output device | Partial | Current policy adapter | Console, multimedia, communications roles and supported-API review |
| Volume wheel and mute keys | Missing | `WM_APPCOMMAND` / `HSHELL_APPCOMMAND`, Core Audio | Hardware keys with Explorer absent and with both shells running |
| Play, pause, stop, next, previous, fast-forward, and rewind | Missing | `WM_APPCOMMAND`, shell hook, media sessions | Foreground handler, unhandled fallback, multiple media sessions |
| Microphone mute and volume commands | Missing | Application commands and audio endpoint APIs | Hardware indicators, privacy state, and selected input |
| Wi-Fi status and saved-network connection | Partial | Native Wi-Fi APIs | Scan refresh, connect, disconnect, authentication failure, captive portal |
| Bluetooth status and saved-device connection | Missing | WinRT Bluetooth APIs | Adapter state, paired devices, connect/disconnect, authorization |
| Battery, brightness, airplane mode, and power profile | Missing | Power and device APIs | Portable hardware, external displays, missing capabilities |

### Input, Accessibility, and System Integration

| Responsibility | Status | Windows contract or evidence | Required verification |
| --- | --- | --- | --- |
| Unified key state for shell hotkeys and Super+pointer operations | Partial | Registered hotkeys, low-level hooks, pointer capture | Lost key-up, focus changes, secure desktop, startup races, rapid chords |
| Avoid globally consuming ordinary input | Partial | Hook chaining and selective suppression | Alphanumeric, IME, accessibility tools, games, elevated windows |
| Touch keyboard and text-input services | Missing | Text Services Framework and touch keyboard activation | Tablet mode, password fields, composition, handwriting |
| Accessibility tree and keyboard navigation | Missing | UI Automation | Narrator, high contrast, keyboard-only use, focus announcements |
| Clipboard history and system clipboard integration | Partial | Clipboard APIs | Text, images, ownership changes, remote clipboard, unavailable clipboard |
| File associations and default-application activation | Partial | Shell execution and association APIs | Open files, folders, URLs, unknown types, chooser UI |
| System error and consent surfaces | Investigate | UAC, secure desktop, brokers, system hosts | Elevation, credentials, SmartScreen, firewall prompts, crash dialogs |
| Packaged-app lifecycle and background brokers | Investigate | App activation and Windows brokers | Settings, Store apps, notifications, suspension, relaunch |

## Message and API Registry

Track each native dependency in one place as it is implemented:

- `WM_APPCOMMAND` and `HSHELL_APPCOMMAND` for unhandled media and volume commands.
- `RegisterShellHookWindow` and the registered `SHELLHOOK` message for shell events.
- `Shell_NotifyIcon`, `NOTIFYICONDATA`, callback versions, and `TaskbarCreated` for tray compatibility.
- AppBar messages and work-area updates for panel reservation.
- DWM thumbnail registration and cleanup for previews.
- Core Audio for endpoints, volume, mute, and device notifications.
- Native Wi-Fi and WinRT Bluetooth for connectivity.
- Per-monitor DPI, display-change, power, device, session, clipboard, and locale notifications.

Prefer documented contracts. If compatibility requires an undocumented interface or observed Explorer
behavior, isolate it in `nickel-platform`, document the risk here, and keep a fallback path.

## Verification Strategy

- Add a small Rust test application for each externally visible shell protocol rather than testing
  only against Nickel.
- Run contract tests once with Explorer present and once with Nickel as the only shell.
- Record manual coverage for focus, secure desktop, games, packaged apps, elevated windows, multiple
  monitors, touch, IME, accessibility, suspend/resume, and device hotplug.
- Test shell restart independently from application restart.
- Require regression coverage whenever an Explorer-owned responsibility is discovered in use.
- Keep policy and deterministic state transitions outside native Windows callbacks.

## References

- [Shell Launcher overview](https://learn.microsoft.com/en-us/windows/configuration/shell-launcher/)
- [The Taskbar](https://learn.microsoft.com/en-us/windows/win32/shell/taskbar)
- [Notifications and the Notification Area](https://learn.microsoft.com/en-us/windows/win32/shell/notification-area)
- [`WM_APPCOMMAND`](https://learn.microsoft.com/en-us/windows/win32/inputdev/wm-appcommand)
- [`RegisterShellHookWindow`](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-registershellhookwindow)

## Completion

This is a living contract and should remain active while Nickel supports Windows. Individual sections
may move into focused specifications, but this inventory is complete only when every responsibility is
implemented, explicitly delegated to a Windows service, or intentionally unsupported with documented
user-visible behavior.
