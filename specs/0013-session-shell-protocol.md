# Restartable Session Shell Protocol

## Goal

Give `nickel-session` and `nickel-ui` an explicit, versioned contract so the compositor can remain alive while the visible shell restarts.

## Behavior

- Replace title-based shell-window identification with authenticated shell roles for desktop, panel, launcher, context menu, previews, and lock/recovery surfaces.
- Replace the ad hoc datagram messages with a framed, versioned, bidirectional Rust protocol.
- Publish initial snapshots and incremental events for outputs, work areas, windows, focus, stacking, state, application identity, and preview frames.
- Route activation, close, minimize, maximize/restore, launcher visibility, preview, and shell-placement requests through typed commands.
- Bound message sizes, preview dimensions, queues, and update rates.
- Reject incompatible clients and connections from outside the active user session.
- On `nickel-ui` exit, preserve applications, clear stale shell surfaces, and restart the UI with bounded backoff. Provide a compositor-owned recovery surface if restart repeatedly fails.

Platform-neutral messages belong in a shared crate. Smithay and native Windows objects must not cross the boundary.

## Verification

- Test encoding, decoding, version rejection, size limits, reconnect snapshots, and stale-object removal.
- Test command authorization and invalid window IDs.
- Kill and restart `nickel-ui` while applications are running; verify their surfaces, state, and focus registry survive.
- Verify shell roles cannot appear as ordinary taskbar applications.
- Run workspace tests and Clippy with warnings denied.

## Completion

Archive this specification after all Linux session/UI coordination uses the typed protocol and live shell restart succeeds without restarting application clients.
