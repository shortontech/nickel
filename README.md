# Nickel

**One Rust desktop for Windows and Linux, because apparently one operating
system was not enough trouble.** Nickel brings its own desktop, taskbar,
launcher, task switching, system controls, controller navigation, and apps. It
targets both platforms. On Linux, it also runs its own compositor. Naturally.

**Windows without Explorer. UWP apps that actually open.** Nickel is the first
independent Windows shell to run UWP apps without keeping Explorer alive behind
the curtains, rattling chains and pretending not to be there. It does this with
the [Universal Windows Usher](crates/nickel-uwu/README.md). Universal. Windows.
Usher. UwU. We reverse-engineered the Windows shell deeply enough to make it say
“UwU.” You are welcome. `^_^`

The naming scheme only gets worse from here: Nickel is the desktop, Plating is
the settings app, and File is called File because even we have limits.

## Try it without surrendering your desktop

Nickel uses stable Rust. No nightly incantations are required. Run commands
from the repository root.

### Windows

```powershell
cargo run -p nickel
```

This starts Nickel beside your current Windows desktop. When Nickel is installed
as the shell and owns the Windows shell window, it starts UwU automatically. If
Explorer is still registered, Nickel politely shares the session instead of
starting a turf war. See the
[UwU research notes](crates/nickel-uwu/README.md) for the implementation,
diagnostic commands, and a heroic quantity of COM archaeology.

### Linux nested session

Run Nickel inside an existing Linux desktop, like a desktop-shaped ship in a
desktop-shaped bottle:

```bash
cargo run -p nickel --no-default-features --features backend-winit --bin nickel-nested
```

For a direct DRM/udev session or an SDDM login session, see
[Linux sessions](docs/linux-sessions.md). The direct session is still under
development; bring logs and a healthy respect for input devices.

## Shiny objects

- A GPU-rendered desktop and taskbar with application grouping, native icons,
  previews, and task switching. Pixels should earn their keep.
- An application launcher with fuzzy search, pinned apps, and launch history.
- Controller navigation with PlayStation, Xbox, Switch, and generic gamepads.
  Confirm and cancel follow the controller family, because muscle memory is a
  user interface contract.
- Nickel Plating for display, network, audio, and other system controls. Yes,
  the settings app is called Plating. We committed to the bit.
- Nickel File, a Markdown viewer, a terminal, and a Codex chat application.
  It is a desktop; eventually it started collecting apps.
- A shared shell experience on Windows and Linux, including a Smithay
  compositor on Linux. Same desk, different arguments with the kernel.

### How to drive it

| Input | Action |
| --- | --- |
| `Windows` | Open or close the launcher |
| `Windows` + `R` | Open Run |
| `Alt` + `Tab` | Switch windows |
| `Alt` + `` ` `` | Cycle windows in the current application |
| `Windows` + `E` | Open Nickel File on Windows |
| `Windows` + left-drag | Move a window |
| `Windows` + right-drag | Resize a window |

The launcher also supports arrow-key navigation, `Enter` to launch, and
`Escape` to close the active Nickel surface.

### Couch controls

| Controller input | Action |
| --- | --- |
| D-pad or left stick | Navigate |
| South face button | Confirm |
| East face button or Select | Cancel |
| Start | Open the context menu |
| Left or right shoulder | Switch panes |
| Guide | Open or close the launcher |

Nintendo layouts use the east face button to confirm and the south face button
to cancel. Nickel detects the controller family instead of asking a Switch
owner to pretend the letters are in Xbox places.

## How unfinished is it?

Nickel is under active development, which is the dignified way to say that some
buttons are ambitions. Work remains on notifications, hardware controls, Wi-Fi
connection management, accessibility, touch-keyboard support, multiple
monitors, and the direct Linux session.

## Rabbit holes

- [Cargo workspace layout](docs/cargo-workspace.md) — crates and their roles.
- [Linux sessions](docs/linux-sessions.md) — nested, direct, and login-session
  setup and diagnostics.
- [UwU research notes](crates/nickel-uwu/README.md) — Windows UWP discovery,
  experiments, and diagnostics. Yes, that still means Universal Windows
  Usher.
- [Codex backend diagnostics](docs/codex-backend-diagnostics.md) — offline
  replay and backend tests.
- [Active specifications](specs/) and [completed specifications](specs/done/).

## Contributing

Before submitting a change, appease the usual three-headed Cargo guardian:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

Include behavior tests where practical and record the platforms tested. “It
worked on my machine” is useful evidence once you tell us which machine.

## License

Copyright 2026 Steven Horton.

Nickel is dual-licensed under the [MIT License](LICENSE-MIT) or the
[Apache License, Version 2.0](LICENSE-APACHE), at your option.

## Similar projects

- [GyroShell](https://github.com/Pdawg-bytes/GyroShell)
- [Cairo Shell](https://github.com/cairoshell/cairoshel)
