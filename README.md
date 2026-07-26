# Nickel

Nickel is a cross-platform desktop shell written in Rust. It provides a desktop, taskbar,
application launcher, task switching, system controls, and settings without requiring Windows
Explorer or a Linux desktop environment.

Nickel currently works best on Windows. Linux session support is under active development, with
macOS planned for later.

## The Nickel Family

The component names follow the project all the way down:

- **Nickel Bar** — the taskbar and system tray
- **Nickel Flip** — task switching and live window previews
- **Nickel File** — the file manager
- **Nickel Plating** — Control Center and system settings
- **Bare Nickel** — the planned lean Windows configuration

## What Nickel Does

### Desktop and Nickel Bar

- Draws the desktop wallpaper, including an optional animated wallpaper
- Shows running applications with native icons
- Groups and cycles multiple windows from the same application
- Tracks the active window
- Hosts notification-area icons and their context menus
- Displays the clock and opens Nickel Plating
- Reserves space when applications are maximized
- Hides behind borderless fullscreen applications

### Launcher and Run

- Indexes installed applications and Start Menu shortcuts
- Includes shortcuts from the user Desktop
- Searches applications with fuzzy matching
- Supports pinned applications and launch history
- Provides keyboard navigation and scrolling
- Opens Run with command history, clipboard support, and IME-aware text input

### Nickel Flip

- Switches between windows with live DWM previews on Windows
- Cycles forward or backward
- Cycles windows within one application
- Supports mouse selection from the preview
- Preserves fullscreen applications while switching

### Nickel Plating

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
| `Windows` | Open or close the launcher |
| `Windows` + `R` | Open Run |
| `Alt` + `Tab` | Open Nickel Flip and move to the next window |
| `Alt` + `` ` `` | Cycle windows in the current application |
| Arrow keys | Move through launcher results |
| `Enter` | Launch the selected result |
| `Escape` | Close the active Nickel surface |

Hardware volume, mute, play/pause, stop, previous, next, fast-forward, and rewind controls work when
Nickel is the Windows shell.

### Mouse

- Click the Nickel Bar button to open or close the launcher.
- Click an application on Nickel Bar to activate it.
- Repeatedly click a grouped application to cycle through its windows.
- Hover a grouped application to see live window previews.
- Click the clock and system area to open Nickel Plating.
- Hold `Windows` and left-drag to move a window.
- Hold `Windows` and right-drag to resize a window.

## Running Nickel

Nickel uses stable Rust.

```bash
cargo build --workspace
cargo test --workspace
```

Launch the desktop shell:

```bash
cargo run -p nickel-ui
```

Launch Nickel Settings:

```bash
cargo run -p nickel-settings
```

Launch Nickel File:

```bash
cargo run -p nickel-file
```

### Linux Nested Session

Run Nickel inside an existing Linux desktop:

```bash
cargo run -p nickel-session -- --backend winit --command target/debug/nickel-ui
```

### Linux Direct Session

The direct backend requires DRM, GBM, libinput, udev, libseat, and EGL development packages. Build
it without the nested backend:

```bash
cargo build -p nickel-ui
cargo build -p nickel-session --no-default-features --features backend-udev
```

Run it from a text VT:

```bash
RUST_LOG=info target/debug/nickel-session \
  --backend udev --command target/debug/nickel-ui
```

Set `NICKEL_DRM_DEVICE=/dev/dri/cardN` to select a specific GPU.

## Current Limitations

- Notifications are not displayed yet.
- Battery, brightness, Bluetooth, and power controls are not implemented.
- Wi-Fi connection management is incomplete.
- Some packaged Windows applications and system settings require additional activation support.
- Accessibility and touch-keyboard integration are incomplete.
- Multiple-monitor behavior needs more testing.
- Nickel File currently provides basic directory browsing and file launching.
- The direct Linux session is not ready for general use.
- macOS support has not started.

The Windows shell compatibility checklist lives in
[`specs/0021-windows-shell-contract.md`](specs/0021-windows-shell-contract.md).

## Project Layout

```text
crates/
|-- nickel-components/  Shared interface components and text editing
|-- nickel-core/        Shell state and behavior
|-- nickel-file/        Nickel File browser and file manager
|-- nickel-logging/     Native logging
|-- nickel-platform/    Shared native platform adapters
|-- nickel-session/     Linux compositor and session
|-- nickel-settings/    Nickel Plating settings application
`-- nickel-ui/          Desktop shell and platform integration
```

Active design work lives in [`specs/`](specs/). Completed specifications live in
[`specs/done/`](specs/done/).

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

## Eventually: Bare Nickel

**Bare Nickel** will be a lean Windows configuration with replaceable shell components and bundled
bloat removed. Keep what makes Windows useful, replace the desktop ecosystem with Nickel, and plate
the remainder.
