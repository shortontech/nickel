# Nickel

**One Rust desktop for Windows and Linux.** Nickel brings its own desktop,
taskbar, launcher, task switching, system controls, controller navigation, and
apps. On Linux, the same executable also runs a Smithay compositor.

**Windows without Explorer, including UWP apps.** Nickel is the first
independent Windows shell to run UWP apps without keeping Explorer alive. It
does this through the
[Universal Windows Usher](crates/nickel-uwu/README.md)—UwU for short—which
recreates the Windows shell services those applications expect.

Nickel is the desktop, Plating is the settings app, and File is called File
because even we have limits.

## Try it

Nickel uses stable Rust. Run commands from the repository root.

### Windows

```powershell
cargo run -p nickel
```

This starts Nickel beside your current Windows desktop. When Nickel is installed
as the shell and owns the Windows shell window, it starts UwU automatically. If
Explorer is still registered, Nickel politely shares the session instead of
replacing it. See the
[UwU research notes](crates/nickel-uwu/README.md) for the implementation,
diagnostic commands, and the underlying COM research.

Tagged releases include a per-user x86-64 MSI. Its optional **Make Nickel the default shell**
feature is disabled by default; selecting it starts Nickel instead of Explorer at the next sign-in.
Removing the feature or uninstalling Nickel restores the normal Windows shell fallback. See the
[Windows installer notes](packaging/windows/README.md) for packaging and native acceptance details.

#### Known issues in v0.1.1

- On Windows with display scaling above 100%, Nickel Bar and the launcher can be positioned off
  screen. Setting display scaling to 100% is a temporary workaround. Fixed in v0.1.2
  ([#13](https://github.com/shortontech/nickel/issues/13)).
- Newly installed applications do not appear in launcher search until its application index is
  refreshed. Fixed in v0.1.2 ([#11](https://github.com/shortontech/nickel/issues/11)).
- The installer does not clearly explain how to enable Nickel as the Windows shell or that a new
  sign-in is required after setup ([#12](https://github.com/shortontech/nickel/issues/12)).

### Linux nested session

Run Nickel inside an existing Linux desktop, like a desktop-shaped ship in a
desktop-shaped bottle:

```bash
cargo run -p nickel --no-default-features --features backend-winit --bin nickel-nested
```

For a direct DRM/udev session or an SDDM login session, see
[Linux sessions](docs/linux-sessions.md). The direct session is still under
development.

## Features

- A GPU-rendered desktop and taskbar with application grouping, native icons,
  previews, and task switching.
- An application launcher with fuzzy search, pinned apps, and launch history.
- Controller navigation with PlayStation, Xbox, Switch, and generic gamepads.
  Confirm and cancel follow the controller family, because muscle memory is a
  user interface contract.
- Nickel Plating for display, network, audio, and other system controls.
- Nickel File, a Markdown viewer, a terminal, and a Codex chat application.
- A shared shell experience on Windows and Linux, including a Smithay
  compositor on Linux.

## Architecture

Portable shell state, search, ranking, navigation, and UI live in focused Rust
crates. Narrow platform adapters connect that shared behavior to Windows APIs or
to Nickel's Linux compositor. This keeps interaction policy deterministic and
testable while each platform retains its native windowing and system services.

See the [Cargo workspace guide](docs/cargo-workspace.md) for the crate map and
responsibilities.

## Keyboard and mouse

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

### Controller

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

## Project status

Nickel is experimental and under active development. The core desktop,
launcher, task switching, controller navigation, and bundled applications are
usable today. Work remains on notifications, hardware controls, Wi-Fi connection
management, accessibility, touch-keyboard support, multiple-monitor coverage,
and the direct Linux session.

## Further reading

- [Linux sessions](docs/linux-sessions.md) — nested, direct, and login-session
  setup and diagnostics.
- [UwU research notes](crates/nickel-uwu/README.md) — Windows UWP discovery,
  experiments, and diagnostics.
- [Codex backend diagnostics](docs/codex-backend-diagnostics.md) — offline
  replay and backend tests.
- [Active specifications](specs/) and [completed specifications](specs/done/).

## Contributing

Before submitting a change, run the usual three Cargo checks:

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

- [Cairo Shell](https://github.com/cairoshell/cairoshell) - the most popular alternative shell
- [GyroShell](https://github.com/Pdawg-bytes/GyroShell)
