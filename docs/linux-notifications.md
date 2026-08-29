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
