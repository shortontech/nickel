# Nested runtime acceptance

Build and run the bounded live nested-session check with:

```sh
cargo build -p nickel --no-default-features --features backend-winit \
  --bin nickel-nested --bin nickel-test-input --bin nickel-nested-acceptance
cargo build -p nickel-settings --bin nickel-settings
./target/debug/nickel-nested-acceptance
```

The harness creates a private `XDG_RUNTIME_DIR`, starts the dedicated `nickel-nested`
binary with explicit test control, then waits for
compositor-owned shell readiness. It asserts that no
shell PID is expected or authenticated and no `--role shell` child exists,
checks the internal surface inventory and live health of eight bundled UI
plugins, measures the launcher's rendered native UI memory, then disables and
re-enables it while checking memory cleanup and surface retirement. It injects
Meta through the nested session's test-control socket and verifies that the
internal launcher becomes visible and closes again. It then samples compositor
CPU ticks across a two-second idle interval. After the idle sample it opens
Settings in the private nested session, waits for
its measured plugin UI memory to appear in shell status, closes Settings, and
checks that the report expires before requesting logout. The harness does not
create a uinput controller: Linux exposes that device to the
host desktop too, where its Guide button can activate the host launcher. Every
phase has a deadline. On
failure, the harness terminates its compositor child and removes its temporary
runtime data.

The harness also installs the shipped component-window example into the private
profile. It checks that enabling the package maps an ordinary window with a
retained UI memory report, sends a scoped pointer click to open its component
dialog, and observes the larger retained native tree. It clicks the dialog's
typed Settings action, checks that Settings opens in the private session and
reports its own UI memory, then closes it and waits for that report to expire.
Disabling the example unregisters
the window and clears that report, and re-enabling creates a fresh window identity.
It then installs a two-window package, focuses one window and sends Alt+F4
through the same scoped test-control socket. The test checks that the other
declared window remains, the package stays running, and its reported native UI
memory falls after the close. The surviving window retains its ordinary window
ID while the input lease rotates during output reconciliation. Its component
button then requests its declared sibling through the plugin API; the test
checks that the sibling maps again and the memory account grows.

This is a live graphical acceptance check, so it requires a working host display.
On hosts where GLVND's default vendor cannot create a nested EGL display, an installed Mesa software
renderer can be selected explicitly without changing the compositor under test:

```sh
__EGL_VENDOR_LIBRARY_FILENAMES=/usr/share/glvnd/egl_vendor.d/50_mesa.json \
LIBGL_ALWAYS_SOFTWARE=1 ./target/debug/nickel-nested-acceptance
```

To exercise the X11 host path from a Wayland session, provide a nonexistent `WAYLAND_DISPLAY` to
the harness while retaining a valid `DISPLAY`. The harness removes both Wayland selectors from the
nested child so winit selects X11:

```sh
WAYLAND_DISPLAY=does-not-exist \
__EGL_VENDOR_LIBRARY_FILENAMES=/usr/share/glvnd/egl_vendor.d/50_mesa.json \
LIBGL_ALWAYS_SOFTWARE=1 ./target/debug/nickel-nested-acceptance
```

The idle check allows up to one fully occupied CPU core across its two-second
window (on the Linux 100 Hz process clock), a deliberately broad bound intended
to catch an unbounded redraw loop without imposing a benchmark-grade threshold.

## Recorded plugin run, 2026-09-27

On the Wayland host display, the Mesa software command above passed after the
shared JSX evaluator extraction. The nested compositor ran bundled plugin UI
and an installed panel, changed a live plugin setting, measured plugin UI
memory, disabled and re-enabled the launcher, accepted
Meta input through the private test-control socket, and shut down cleanly. The
two-second idle sample used 14 compositor CPU ticks. The harness did not create
a uinput controller or send a Guide button to the host.

A later run on the same date also passed the Settings process memory report:
the nested Settings window published nonzero UI memory, then its shell status
row disappeared after the bounded report expiry.

The current acceptance now disables Launcher while open, checks that its
surface retires, sends Meta to the nested session while disabled, and verifies
that no native launcher appears. Re-enabling the plugin restores the shortcut.
It also sends Super+R to exercise the bundled Run dialog.
It disables Taskbar, verifies the bar surface retires and its native UI memory
clears, then re-enables it before exercising installed plugin panels.
It also opens Control Center with Super+A, disables it while visible, checks
that Super+A cannot reopen a native fallback, then re-enables it.
It opens notification history with Super+N, disables Notifications while visible,
checks that Super+N cannot reopen it, then re-enables the plugin and opens it
again. The shortcut keys are sent to the private nested session through its
test-control socket, including the Super key for Launcher; no Guide input is sent.

The X11 host command reached the nested test-control listener but the host X
server returned an XIO error. Readiness then failed with `WouldBlock`, so this
run does not establish X11 presentation parity.

`cargo check -p nickel --target x86_64-pc-windows-gnu --all-targets` passed on
the same branch. This checks Windows compilation; native Windows input and
presentation still require a Windows session.
