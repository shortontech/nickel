# Linux Status Notifier Tray

## Goal

Display Linux StatusNotifierItem tray icons in Nickel's panel without exposing D-Bus or Linux protocol types to portable UI code. Provide an all-Rust test item so registration, display, activation, and removal can be exercised under `nickel-session`.

## Scope

- Add a platform-neutral `TrayItem` model containing an opaque ID, tooltip, icon pixels, and activation capability.
- Implement the StatusNotifierWatcher and host integration only in Nickel's Linux platform adapter.
- Accept item registration by D-Bus service name or object path, read the standard item properties, and prefer supplied pixmaps over themed icon names.
- Refresh the panel from a non-blocking background feed. Removing or disconnecting an item removes its icon.
- Render tray items between task icons and the clock without changing the existing launcher/task hit regions.
- Send `Activate` for a left click on a tray icon. Menu protocol support and notification balloons are deferred.
- Add a Rust test binary that registers a deterministic icon and records activation.

Windows notification-area and macOS menu-bar adapters are explicitly out of scope. StatusNotifier D-Bus names and payloads must not enter the shared model or panel renderer.

## Verification

- Unit-test tray layout and hit testing independently of D-Bus.
- Unit-test pixmap conversion and selection.
- Run workspace tests and Clippy with warnings denied.
- Under `nickel-session`, launch the test tray item, confirm its icon appears, left-click it, and confirm activation is logged.
- Stop the test item and confirm the icon disappears.

## Completion

Move this document to `specs/done/` after the Linux adapter, test item, panel rendering, and live lifecycle verification are complete.
