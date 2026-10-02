# Nested runtime acceptance

Build the participating binaries locally:

```sh
cargo build -p nickel --no-default-features --features backend-winit \
  --bin nickel-nested --bin nickel-test-input --bin nickel-nested-acceptance
./target/debug/nickel-nested-acceptance
```

The harness creates a private runtime and configuration directory, starts the
nested compositor with explicit test control, and waits for native output
readiness. It checks the default shell as one package: taskbar layout, opening
the launcher with Meta, opening and closing the JSX Settings window, retaining
the package when optional windows close, and retiring/re-enabling its surfaces.
It also checks installed sibling windows, separate dialogs and overlays, native
screenshot input/lifecycle, and socket component layout pagination. Native
Desktop and Lock surfaces remain required. No separate Settings executable is
built or launched, and there is no memory budget acceptance gate.

Input is sent through the nested session's private socket. The harness does not
create a host-visible controller or inject a Guide key. Failures request logout,
then terminate the compositor if necessary and remove the private runtime.
Each readiness/interaction wait is bounded. This harness targets the new default
package; it must run after stock cutover, not against the retired per-page hosts.

A working graphical host display is required. Mesa software rendering can be
selected explicitly where the default EGL vendor cannot create a nested display:

```sh
__EGL_VENDOR_LIBRARY_FILENAMES=/usr/share/glvnd/egl_vendor.d/50_mesa.json \
LIBGL_ALWAYS_SOFTWARE=1 ./target/debug/nickel-nested-acceptance
```

To exercise X11 from a Wayland host while retaining a valid `DISPLAY`:

```sh
WAYLAND_DISPLAY=does-not-exist \
__EGL_VENDOR_LIBRARY_FILENAMES=/usr/share/glvnd/egl_vendor.d/50_mesa.json \
LIBGL_ALWAYS_SOFTWARE=1 ./target/debug/nickel-nested-acceptance
```

The records below describe the previous harness and its previous architecture;
they do not establish acceptance for the shared shell package.

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
