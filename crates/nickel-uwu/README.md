# UwU~! Windows UWP shell host

`nickel-uwu` supplies the Windows shell services needed to present packaged
UWP applications when Explorer is not the registered desktop shell. It is a
Rust library used by Nickel.

The crate uses private Windows interfaces. It reads the CodeView identity from
each loaded Windows DLL, obtains the exact matching public PDB from Microsoft's
symbol server, and resolves private entry points by name. Resolved RVAs are
cached in an unsigned manifest keyed by the PDB GUID and age under
`%LOCALAPPDATA%\Nickel\symbols`. Every callable address and patched import
target is also checked against the live object or loaded module before use.
Missing or incompatible symbols fail startup instead of calling an unchecked
target.

## Integration contract

Start the host only when `GetShellWindow()` is null. Run it on a dedicated STA
thread that owns its hidden shell window and message pump for the entire
session:

```rust
let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);

let host = std::thread::Builder::new()
    .name("uwu-sta".into())
    .spawn(move || nickel_uwu::run_embedded_host(ready_tx, true))?;

ready_rx.recv_timeout(std::time::Duration::from_secs(30))??;
```

Pass `false` as the second argument when the containing process has already
configured DPI awareness. `run_embedded_host` does not return during normal
operation. Retain and supervise its thread for as long as UWP presentation is
required.

Before calling `IApplicationActivationManager::ActivateApplication`, call:

```rust
nickel_uwu::prepare_app(aumid)?;
```

This synchronously registers the AUMID with the shell's view-event dispatcher.
The registration is retained and reused for the lifetime of the host. Register
before activation so the application cannot publish its view before the shell
subscribes.

The crate also exports `run_managed_host(parent_pid)` for a separate helper
process. It retains a synchronization handle to the parent and exits after the
parent.

## Startup and presentation sequence

The host performs these operations on its STA thread:

1. Create a hidden top-level window and register it with `SetShellWindowEx`.
2. Initialize COM as `COINIT_APARTMENTTHREADED`.
3. Redirect `twinui.pcshell.dll`'s fallback `CreateWindowInBand` request from
   `ZBID_IMMERSIVE_BACKGROUND` (12) to `ZBID_DESKTOP` (1).
4. Create and start the private immersive shell controller.
5. Create the private application-frame pool.
6. Register each AUMID with `IViewEventDispatcher` before activation.
7. Receive the view wrapper from the dispatcher, acquire its frame proxy,
   associate the presented CoreWindow, establish frame position and view size,
   complete readiness, uncloak, and foreground the view.
8. Retain the dispatcher subscriptions, frame pool, controller, and import
   redirect until shutdown.

The dispatcher supplies the view identity and wrapper interfaces. This
implementation uses those interfaces to associate each frame with its
CoreWindow.

## Code to carry into another shell

The implementation is split by responsibility:

- `host.rs`: STA lifetime, preactivation registrations, event processing,
  message pumping, and readiness signaling;
- `shell_window.rs`: hidden shell HWND registration and cleanup;
- `fallback_band1.rs`: guarded import redirection for the fallback window;
- `controller.rs`: immersive shell controller construction and lifetime;
- `view_event_trace.rs`: typed view-event subscription and wrapper ownership;
- `frame_service_direct.rs`: frame-pool and frame-proxy acquisition;
- `presentation_callbacks.rs`: frame association, sizing, readiness,
  uncloaking, and foreground calls.

Copy these modules together. Their private interface assumptions, controller
lifetime, STA affinity, and cleanup order are coupled. A new Windows build is
accepted when its matching public symbols contain the required private entry
points and the validated private object layouts remain compatible.

## Building and checking

From the workspace root:

```powershell
cargo build -p nickel-uwu
cargo test -p nickel-uwu
cargo clippy -p nickel-uwu --all-targets -- -D warnings
```

## Current limits

- The implementation is Windows-only and depends on private interfaces and
  object layouts that may still require adaptation after a Windows update.
- First use of a new Windows module build requires access to Microsoft's symbol
  server. Cached PDBs and manifests are reused offline for that exact module.
- It must own the registered shell window and refuses to start while another
  shell owns it.
- It presents classic `ApplicationFrameWindow` hosted UWP views. Other window
  models do not use this path.
- Initial positioning currently uses the work area and fixed default geometry.
- Ongoing resize, requested view modes, and close propagation still require
  lifecycle handling beyond initial presentation.
