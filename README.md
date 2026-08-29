# Nickel

Nickel is an experimental desktop shell written entirely in Rust. On Windows, it provides a
GPU-rendered desktop, taskbar, application launcher, task switching, system controls, settings,
and a file browser without requiring Windows Explorer as the desktop shell.

The repository also contains a Linux compositor built with Smithay. It runs as a nested development
session or directly through DRM and udev. Nickel also supports macOS as an SDL overlay shell with
Nickel Bar, launcher, native app icons, and visible-window control.

## Included Applications

- **Nickel UI** — the desktop shell, taskbar, launcher, task switcher, and system controls
- **Nickel Settings** — display and system settings
- **Nickel File** — directory browsing and file launching
- **Nickel Markdown** — safe, selectable local Markdown viewing
- **Nickel Session** — the Linux compositor and session host

## What Nickel Does

### Desktop and Taskbar

- Draws the desktop wallpaper on Windows and Linux
- Shows running applications with native icons
- Groups and cycles multiple windows from the same application
- Tracks the active window
- Hosts notification-area icons and their context menus on supported platforms
- Displays the clock and opens system controls on Windows and Linux
- Reserves space when applications are maximized on Windows and Linux
- Hides behind borderless fullscreen applications

### Launcher and Run

- Indexes installed applications and Start Menu shortcuts on Windows
- Indexes `.desktop` applications on Linux and `.app` bundles on macOS
- Includes shortcuts from the user Desktop
- Searches applications with fuzzy matching
- Supports pinned applications and launch history
- Provides keyboard navigation and scrolling
- Opens Run with command history, clipboard support, and IME-aware text input

### Task Switching

- Switches between windows with live DWM previews on Windows
- Shows visible application windows with native app icons on macOS
- Cycles forward or backward
- Cycles windows within one application
- Supports mouse selection from the preview
- Preserves fullscreen applications while switching

### System Controls and Settings

- Shows the real display layout
- Identifies displays and selects the primary display
- Reports the active network
- Controls master volume
- Selects the default audio output
- Responds to hardware volume and media controls
- Displays a compact volume indicator

## Using Nickel

### Keyboard

| Input | Action |
| --- | --- |
| `Windows` | Open or close the launcher on Windows and Linux |
| `Option` + `Space` | Open or close the launcher on macOS |
| `Windows` + `R` | Open Run on Windows and Linux |
| `Alt` + `Tab` | Open Nickel Flip and move to the next window on Windows and Linux |
| `Alt` + `` ` `` | Cycle windows in the current application on Windows and Linux |
| Arrow keys | Move through launcher results |
| `Enter` | Launch the selected result |
| `Escape` | Close the active Nickel surface |

Hardware volume, mute, play/pause, stop, previous, next, fast-forward, and rewind controls work when
Nickel is the Windows shell.

### Mouse

- Click the Nickel Bar button to open or close the launcher.
- Click an application on Nickel Bar to activate it.
- Repeatedly click a grouped application to cycle through its windows.
- Hover a grouped application to see live window previews on Windows.
- Click the clock and system area to open Nickel Plating on Windows and Linux.
- Hold `Windows` and left-drag to move a window on Windows and Linux.
- Hold `Windows` and right-drag to resize a window on Windows and Linux.

## Running Nickel

Nickel uses stable Rust.

```bash
cargo build --workspace
cargo test --workspace
```

Launch the desktop shell:

```bash
cargo run -p nickel-shell
```

On macOS, install SDL3 first:

```bash
brew install sdl3
```

macOS window activation, minimize, and close use Accessibility APIs. Grant the terminal running
Nickel permission in System Settings > Privacy & Security > Accessibility.

Launch Nickel Settings:

```bash
cargo run -p nickel-settings
```

Launch Nickel File:

```bash
cargo run -p nickel-file
```

Open a local Markdown document:

```bash
cargo run -p nickel-markdown-ui -- README.md
```

### Linux Nested Session

Run Nickel inside an existing Linux desktop:

```bash
cargo run -p nickel-session -- --backend winit --command target/debug/nickel
```

Live compositor tests may add `--test-control` before `--command`. This explicitly enables the
capability-authenticated `TestInput` protocol command for the nested backend, allowing tests to
inject semantic keyboard and pointer events through the same Smithay input path as physical
devices. The flag is rejected by the direct backend and is disabled by default.

With the session-issued `NICKEL_SESSION_CONTROL` and `NICKEL_SESSION_TOKEN` environment variables,
`nickel-test-input` can inspect registered windows and inject individual production input events:

```bash
cargo run -p nickel-session --bin nickel-test-input -- windows
cargo run -p nickel-session --bin nickel-test-input -- workspaces
cargo run -p nickel-session --bin nickel-test-input -- outputs
cargo run -p nickel-session --bin nickel-test-input -- surfaces
cargo run -p nickel-session --bin nickel-test-input -- move 64 700
cargo run -p nickel-session --bin nickel-test-input -- button left pressed
cargo run -p nickel-session --bin nickel-test-input -- button left released
```

The same capability provides semantic workspace commands, nested output hotplug, and lock-boundary
acceptance without copied coordinates or private state mutation. It cannot invoke logout or power
actions. Run `nickel-test-input --help` for the complete command set.

The client does not mutate shell state directly: events still pass through the compositor's normal
hit testing, focus handling, and input reducers.

### Linux Direct Session

Protocol policies, compatibility evidence, and known limitations are tracked in
[`docs/linux-application-compatibility.md`](docs/linux-application-compatibility.md).
The compositor lock authority, PAM boundary, and remaining native acceptance are documented in
[`docs/session-locking.md`](docs/session-locking.md).

The direct backend requires DRM, GBM, libinput, udev, libseat, and EGL development packages. Build
it without the nested backend:

```bash
cargo build -p nickel-shell
cargo build -p nickel-session --no-default-features --features backend-udev
```

Run it from a text VT:

```bash
RUST_LOG=info target/debug/nickel-session \
  --backend udev --command target/debug/nickel
```

Set `NICKEL_DRM_DEVICE=/dev/dri/cardN` to select a specific GPU.

### Linux Login Session

Build the direct compositor, shell, and login launcher:

```bash
cargo build --release -p nickel-session --no-default-features --features backend-udev
cargo build --release -p nickel-shell
```

Install the completed build as an SDDM Wayland session:

```bash
sudo packaging/install-nickel-session.sh
```

The installer resolves the checkout containing the script rather than assuming a fixed repository
path. `NICKEL_RELEASE_DIR` may select another completed release directory, and
`NICKEL_INSTALL_ROOT` stages the exact installed layout under a temporary packaging root.

To remove the session, delete only the files installed by the script:

```bash
sudo rm /usr/share/wayland-sessions/nickel.desktop
sudo rm /usr/share/applications/nickel-settings.desktop
sudo rm /usr/share/icons/hicolor/512x512/apps/nickel-settings.png
sudo rm /usr/local/bin/nickel-login /usr/local/bin/nickel-session
sudo rm /usr/local/bin/nickel /usr/local/bin/nickel-settings
```

If a development build cannot start, select another desktop from SDDM's session chooser. From that
desktop, inspect the previous boot with `journalctl -b -1 | rg 'nickel|sddm-helper'`, rebuild both
the direct compositor and shell, and rerun the installer. A compositor startup failure exits back to
the display manager; an intentional logout exits successfully. Do not replace the installed binaries
with symlinks into `target/`: a later default-feature build can replace the direct-backend binary.

Nickel asks the user D-Bus session for its configured `org.freedesktop.secrets` provider; it does not
select or start a KWallet-, GNOME Keyring-, or KeePassXC-specific service. The operating system may
use a provider-specific PAM module to unlock the wallet at login. Providers without PAM integration
remain supported through the standard Secret Service unlock prompt, but automatic login-password
unlock is not universal. Nickel verifies the existing default collection, exposes readiness to the
shell, warns before launching known credential-dependent applications while storage is unavailable,
and never creates a replacement collection.

To pin reconnections to a specific provider, place its absolute executable path in
`$XDG_CONFIG_HOME/nickel/secret-service-provider` (or
`~/.config/nickel/secret-service-provider`). Nickel rejects a different process taking ownership of
`org.freedesktop.secrets`; without this optional pin it reports the current owner for diagnosis but
does not persist an automatic selection.

## Project Status

- Notifications are not displayed yet.
- Battery, brightness, Bluetooth, and power controls are not implemented.
- Wi-Fi connection management is incomplete.
- macOS support runs as an overlay shell: it shows Nickel Bar, opens the launcher with
  `Option` + `Space`, indexes and launches `.app` bundles, displays native app icons, and
  activates/minimizes/closes visible application windows when Accessibility permission is granted.
- macOS live previews, Run, tray/menu-bar integration, Nickel Bar clock and control center, and
  Wi-Fi/Bluetooth/audio controls are not implemented.
- Some packaged Windows applications and system settings require additional activation support.
- Accessibility and touch-keyboard integration are incomplete.
- Multiple-monitor behavior needs more testing.
- Nickel File currently provides basic directory browsing and file launching.
- The direct Linux session is not ready for general use.

The Windows shell compatibility checklist lives in
[`specs/0021-windows-shell-contract.md`](specs/0021-windows-shell-contract.md).

## Project Layout

```text
crates/
|-- nickel-ui/         Declarative UX layer, layout, state, and SDL presentation
|-- nickel-codex/      Typed Codex CLI selection, app-server RPC, and diagnostics
|-- nickel-codex-fixture/ Offline protocol fixtures, replay validation, and failure injection
|-- nickel-codex-ui/   Standalone declarative Codex chat application
|-- nickel-core/        Shell state and behavior
|-- nickel-file/        Nickel File browser and file manager
|-- nickel-logging/     Native logging
|-- nickel-markdown/    Safe typed Markdown parsing and presentation
|-- nickel-markdown-ui/ Standalone read-only Markdown viewer
|-- nickel-platform/    Shared native platform adapters
|-- nickel-session/     Linux compositor and session
|-- nickel-settings/    Nickel Plating settings application
`-- nickel-shell/      Desktop shell and platform integration
```

Active design work lives in [`specs/`](specs/). Completed specifications live in
[`specs/done/`](specs/done/).

## Codex Backend Diagnostics

Codex support is deliberately testable without Nickel UI. These commands validate offline replay and
probe a CLI without starting a model turn:

```bash
cargo run -p nickel-codex-fixture -- validate crates/nickel-codex-fixture/fixtures
cargo run -p nickel-codex --bin nickel-codex-test -- replay crates/nickel-codex-fixture/fixtures/basic.json
cargo run -p nickel-codex --bin nickel-codex-test -- probe --backend installed
cargo run -p nickel-codex-ui -- --replay crates/nickel-codex-fixture/fixtures/basic.json
```

`nickel-codex-test` emits versioned JSONL on stdout. Installed Codex is preferred only after generated
schema and initialization compatibility checks; release builds retain a pinned bundled fallback. Nickel
does not implement Codex authentication or call OpenAI subscription APIs directly.

An authenticated first turn must be started on the same app-server connection that creates its thread;
subsequent one-shot turns resume the persisted thread explicitly:

```bash
cargo run -p nickel-codex --bin nickel-codex-test -- start-thread --cwd "$PWD" --text "Hello"
cargo run -p nickel-codex --bin nickel-codex-test -- turn THREAD_ID --text "Continue"
```

The standalone graphical client runs independently of the Nickel shell:

```bash
cargo run -p nickel-codex-ui -- --backend installed
```

## Contributing

Before submitting a change:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

Include behavior tests where practical and record the platforms tested.

## License

Nickel is dual-licensed under the [MIT License](LICENSE-MIT) or the
[Apache License, Version 2.0](LICENSE-APACHE), at your option.
