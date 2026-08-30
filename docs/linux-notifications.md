# Linux notifications

The Nickel shell implements `org.freedesktop.Notifications` on the user session bus. It advertises
body, persistence, and action support. New notifications, in-place replacement, application close,
expiry, action invocation, and user dismissal emit the standard protocol signals and reasons.

History is bounded as documented in `runtime-cache-limits.md`. Action vectors must contain key/label
pairs; incomplete or empty pairs are ignored and at most three valid actions are presented. Clicking
an action emits `ActionInvoked` before closing the notification. Clicking elsewhere dismisses it.

The daemon is part of the supervised Nickel shell rather than the compositor. If the shell exits,
the bus name is released; the replacement shell claims it again without ending the compositor
session. `nickel-test-notification` verifies Nickel owns the name before testing replacement and
close signaling, so another installed notification daemon cannot produce a false-positive result.

## Recorded native acceptance

On 2026-08-30, after replacing the supervised shell without restarting the compositor, the
display-manager-launched session's replacement shell reclaimed `org.freedesktop.Notifications`.
`nickel-test-notification` then created notification ID 1, replaced it in place with the same ID,
closed it through the standard D-Bus method, and received `NotificationClosed(1, 3)` from Nickel.
