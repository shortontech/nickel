# Grouped Application Windows

## Goal

Represent multiple windows from the same application as one panel item while retaining enough identity and title metadata to choose and manage individual windows.

## Behavior

- Extend the portable open-window model with a sanitized window title.
- Group windows by resolved application ID. Windows without a resolved application remain separate groups keyed by window ID.
- Preserve compositor stacking order inside each group and mark a group active when any member is active.
- Render one panel icon per group using the resolved application icon and name.
- Clicking a single-window group activates that window.
- Clicking a multi-window group opens a compact chooser containing the application name and each member window title; selecting a row activates that window.
- Right-click retains window-management behavior and must target a specific member rather than closing an entire group accidentally.
- Empty titles fall back to the application name, then `Untitled window`.

## Architecture

`nickel-session` owns raw title and stacking data and exposes activation through its existing control socket. `nickel-ui` performs platform-neutral grouping after Linux app-ID reconciliation. Panel rendering consumes grouped models and must not know Wayland identifiers or protocols.

Virtual desktop membership is deliberately omitted. The grouped model should remain extensible so the next slice can filter or annotate groups by desktop without changing their application identity.

## Verification

- Test title serialization and parsing.
- Test stable grouping, unresolved-window separation, active state, and title fallbacks.
- Test group hit regions and member selection.
- Run two windows with the same app ID under `nickel-session`; verify one panel icon, both titles in the chooser, and activation of each member.
- Run workspace tests and Clippy with warnings denied.

## Completion

Move this document to `specs/done/` after grouped rendering, member activation, and live same-application verification pass.
