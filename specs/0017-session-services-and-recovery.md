# Session Services, Locking, and Recovery

## Goal

Provide the remaining user-facing services and safety behavior required to use Nickel as a complete daily desktop session.

## Behavior

- Implement a notification daemon and bounded notification history using portable notification models.
- Add logout, restart-shell, reboot, power-off, suspend, and lock actions with confirmation where destructive or disruptive.
- Implement compositor-enforced locking: hide client contents, capture input, prevent shell bypass, and require successful authentication before unlock.
- Add idle tracking and configurable transitions to dim, lock, and suspend.
- Integrate XDG desktop portals for file selection, opening URIs, screenshots/screen sharing, and related sandboxed application requests.
- Expose clear recovery UI when `nickel-ui`, XWayland, or an optional session service fails.
- Bound icon, thumbnail, preview, notification, and history caches and expose useful diagnostics.
- Complete keyboard, touch, and controller navigation for launcher, panel, previews, notifications, session controls, and lock/recovery surfaces.

Authentication must use an established system facility through a narrow Rust adapter; Nickel must not store passwords.

## Verification

- Test notification replacement, expiry, actions, history bounds, and daemon restart.
- Test every session action with mocked authorization, then manually verify lock/unlock, suspend/resume, logout, and shutdown.
- Confirm locked clients cannot receive input or expose readable contents through Nickel surfaces.
- Verify portal-backed file selection and screen sharing in representative sandboxed applications.
- Exercise shell, XWayland, and service crash recovery without losing the compositor session.

## Completion

Archive this specification when the service matrix passes in a display-manager-launched session and Nickel can recover or exit safely from each tested failure.
