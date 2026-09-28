# UwU~! Windows UWP shell host

`nickel-uwu` supplies the Windows shell services needed to present packaged
UWP applications when Explorer is not the registered desktop shell. It is a
Rust library used by Nickel, with an optional diagnostic binary for manual
development work.

This crate uses private Windows interfaces and build-specific addresses. The
current constants were verified on Windows build 26200. Each callable address
and patched import target is checked against the loaded module before use, so
an unknown build fails during startup instead of calling an unchecked target.

## Integration contract

A shell should start this host only when `GetShellWindow()` is null. Run it on
a dedicated STA thread that owns its window and message pump for the entire
session.

Nickel embeds the host as follows:

```rust
let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);

let host = std::thread::Builder::new()
    .name("uwu-sta".into())
    .spawn(move || nickel_uwu::run_embedded_host(ready_tx, true))?;

ready_rx.recv_timeout(std::time::Duration::from_secs(30))??;
```

`run_embedded_host` does not return during normal operation. Its arguments are:

- a one-shot sender that reports successful startup or the startup error;
- whether the host should set process DPI awareness before creating an HWND.

Set the second argument to `false` when the containing process has already
configured DPI awareness. Keep the thread alive for as long as UWP presentation
is required. Nickel checks for an existing shell before starting, retries after
another shell exits, and treats an unexpected host-thread exit as a reason to
retry.

The crate also exports `run_managed_host(parent_pid)` for a separate helper
process. It keeps a synchronization handle to the parent and exits after the
parent. `run_host` exists for the diagnostic command-line frontend.

## Startup and presentation sequence

The host performs these operations on its STA thread:

1. Create a hidden top-level window and register it with `SetShellWindowEx`.
2. Initialize COM as `COINIT_APARTMENTTHREADED`.
3. Redirect `twinui.pcshell.dll`'s fallback `CreateWindowInBand` request from
   `ZBID_IMMERSIVE_BACKGROUND` (12) to `ZBID_DESKTOP` (1).
4. Create and start the private immersive shell controller.
5. Start a 250 ms window-reconciliation timer and enter the Win32 message
   loop.
6. Match each `ApplicationFrameWindow` with its CoreWindow by application ID,
   drive the private discovery, visibility, and layout callbacks, then uncloak
   the view.
7. Retain the controller and import redirect for the lifetime of the host and
   restore the import slot during shutdown.

Presentation is considered complete only after the wrapper is associated,
its wait flags are clear, the CoreWindow is attached to the frame, both
windows are uncloaked, and the windows remain ready for one second. Failed or
slow presentations remain eligible for retries with backoff. Matching refuses
ambiguous application IDs and never steals a CoreWindow already owned by a
different frame.

## Code to carry into another shell

The reusable implementation is split by responsibility:

- `host.rs`: STA lifetime, startup order, message pump, and readiness signal;
- `shell_window.rs`: hidden shell HWND registration and cleanup;
- `fallback_band1.rs`: guarded import redirection for the fallback window;
- `controller.rs`: immersive shell controller construction and lifetime;
- `auto_present.rs`: frame/CoreWindow matching, retries, and readiness policy;
- `wrapper_inspect.rs`: validated lookup of the private view wrapper;
- `presentation_callbacks.rs`: private discovery, visibility, and layout calls.

Copy these modules together. Their private interface assumptions, controller
lifetime, STA affinity, and cleanup order are coupled. Keep build-specific
addresses in the modules that validate them, and update them from matching
Microsoft symbols before enabling a new Windows build.

The embedding shell must also provide the dependency features listed in
`Cargo.toml`, call the host only after deciding that no other desktop shell is
active, and supervise the host so a failed startup does not block the shell's
main UI.

## Building and checking

From the workspace root:

```powershell
cargo build -p nickel-uwu --no-default-features
cargo test -p nickel-uwu --no-default-features
cargo clippy -p nickel-uwu --all-targets --all-features -- -D warnings
```

The default `diagnostics` feature builds `nickel-windows-frame-probe` and the
manual tools under `src/bin/`. Production users should disable default
features, as Nickel does in `crates/nickel/Cargo.toml`.

## Current limits

- The implementation is Windows-only and tied to the verified private ABI of
  Windows build 26200.
- It must own the registered shell window; it will refuse to start while
  Explorer or another shell owns it.
- It presents classic `ApplicationFrameWindow` hosted UWP views. Other window
  models do not use this path.
- Initial frame placement follows Windows' frame service and the current
  desktop work area. A replacement shell is responsible for publishing an
  accurate work area for its own bars and reserved screen regions.
