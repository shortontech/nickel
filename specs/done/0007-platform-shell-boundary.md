# Platform Shell Boundary

## Goal

Prevent the Linux dogfood backend from defining Nickel's portable shell model. Windows is the primary product target; Linux and macOS are adapters to the same UI-facing contract.

## Boundary

- Shell UI code consumes neutral `ApplicationId`, `Application`, `WindowId`, and `OpenWindow` values.
- Platform adapters own application discovery, native launch metadata, window enumeration, native-to-Nickel identity reconciliation, and shell-control transport.
- Identifiers exposed to the UI are opaque Nickel application identifiers. The UI must not trim `.desktop`, interpret Wayland app IDs, HWNDs, bundle identifiers, or transport payloads.
- The Linux adapter may use desktop entries, Freedesktop icons, Wayland app IDs, and Unix datagrams.
- The Windows adapter slot must exist explicitly and return an empty provider until its Win32/WinRT implementation is added; it must not route through Linux conventions.
- Rendering, search, pinning, panel layout, and task-icon presentation remain outside platform adapters.

## Initial Cleanup

- Move Linux discovery and session communication behind `platform::linux`.
- Move portable application/window types into a neutral model module.
- Resolve Wayland app IDs to Nickel application IDs inside the Linux adapter.
- Remove target-specific conditionals and Unix socket types from the main shell event loop.

## Verification

- Platform-neutral model and identity behavior have focused tests.
- Linux launcher discovery and live task icons retain current behavior.
- `cargo test --workspace` and workspace Clippy pass.
