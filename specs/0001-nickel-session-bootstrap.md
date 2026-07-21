# Nickel Session Bootstrap

## Objective

Introduce `nickel-session` early enough to dogfood compositor-owned window management. The first backend runs nested inside the current desktop session, so development does not yet depend on DRM, libinput, logind, or a display-manager entry.

## Boundaries

- `nickel-session` is a persistent Rust process built on Smithay.
- The nested backend accepts ordinary Wayland `xdg-shell` clients and renders their surfaces in a host window.
- The compositor owns a normalized registry containing a stable window ID, title, application ID, state, and stacking order.
- Window policy and registry behavior must not depend on Smithay protocol objects.
- The visible Nickel shell remains a separate process. A transport between it and `nickel-session` will follow once the registry is usable.
- Direct DRM/input/session ownership, XWayland, locking, and display-manager integration are out of scope for this bootstrap.

## Initial Behavior

Running `nickel-session` creates a nested compositor and prints its Wayland socket name. Commands launched with that socket in `WAYLAND_DISPLAY` can map, commit, update metadata, and close top-level windows. The session records those lifecycle changes and can focus the most recently mapped window.

The existing launcher must execute the selected application when Enter is pressed. Launching is best-effort, reports failures without terminating Nickel, and leaves the launcher window open. Linux desktop-entry field codes and shell syntax must not be passed to a shell.

## Verification

- Unit tests cover registry creation, metadata updates, focus changes, and removal.
- Launcher tests cover safe desktop-entry command parsing and field-code removal.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` passes.
- `cargo test --workspace --all-features` passes.
- Manual nested-session verification launches at least one Wayland client, observes it in the registry, and closes it without terminating the compositor.

## Follow-up

Add the session-to-shell transport and task-switcher UI, then implement a direct Linux backend. Archive this specification only after the nested lifecycle proof is complete.
