# Linux Notification Daemon

Status: complete.

## Goal

Make application desktop notifications work in the Nickel Linux session without depending on
Plasma Shell.

## Behavior

- Own `org.freedesktop.Notifications` on the user session bus and implement the standard
  `GetCapabilities`, `GetServerInformation`, `Notify`, and `CloseNotification` methods.
- Keep at most 100 notifications in memory. New notifications receive nonzero monotonically
  increasing IDs; a valid `replaces_id` updates the existing notification in place.
- Honor positive expiration timeouts, keep zero-timeout notifications until explicitly closed,
  and use a five-second server default for negative timeouts.
- Present the newest live notification in a non-taskbar Nickel toast surface on the primary
  output. Notifications with an empty summary may use the application name as their heading.
- Emit `NotificationClosed` when a notification expires, is explicitly closed, or is discarded
  because the bounded history is full.
- Advertise body and persistence capabilities. Actions, markup, inline replies, sound, and
  notification history UI are deferred to the broader session-services specification.
- If another notification daemon already owns the bus name, fail shell startup with a useful
  error instead of running a visually functional shell with broken notification delivery.

## Verification

- Unit-test ID allocation, replacement, explicit closure, expiry, and the 100-item bound.
- Exercise the D-Bus methods against a display-manager-launched Nickel session and confirm a toast
  becomes visible.
- Confirm the service owns `org.freedesktop.Notifications`, emits closure signals, and does not
  require `plasmashell`.
- Run workspace tests, strict Clippy, formatting, and the native udev build.

## Completion

Move this specification to `specs/done/` after live D-Bus delivery and toast visibility are
verified in the Nickel login session.
