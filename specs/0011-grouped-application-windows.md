# Grouped Application Windows

## Goal

Represent multiple windows from the same application as one panel item while retaining enough identity and title metadata to choose and manage individual windows.

## Behavior

- Extend the portable open-window model with a sanitized window title.
- Group windows by resolved application ID. Windows without a resolved application remain separate groups keyed by window ID.
- Preserve compositor stacking order inside each group and mark a group active when any member is active.
- Render one panel icon per group using the resolved application icon and name.
- Clicking a single-window group activates that window.
- Hovering a group opens a preview surface above the panel. Member cards are arranged side by side and contain a live window thumbnail, application/window title, and a close button in the top-right corner.
- The preview remains open while the pointer moves between the panel button and preview surface, then dismisses after both lose hover/focus.
- Clicking a preview activates that window. Clicking its close button closes only that window.
- Right-clicking a preview opens a per-window menu with Close, Maximize/Restore, and Minimize actions.
- Empty titles fall back to the application name, then `Untitled window`.

## Architecture

`nickel-session` owns raw title, stacking data, state-changing window actions, and thumbnail capture. It exposes bounded preview frames through a local session transport; `nickel-ui` must not read arbitrary application memory or use desktop screenshots. `nickel-ui` performs platform-neutral grouping after Linux app-ID reconciliation and renders preview cards from neutral RGBA frames.

Preview capture is demand-driven while a panel group is hovered. Frames are size-bounded, rate-limited, and discarded when the preview closes. The portable model may describe minimize/maximize capability and state, but Smithay and Windows implementations remain platform adapters.

Virtual desktop membership is deliberately omitted. The grouped model should remain extensible so the next slice can filter or annotate groups by desktop without changing their application identity.

## Verification

- Test title serialization and parsing.
- Test stable grouping, unresolved-window separation, active state, and title fallbacks.
- Test group, preview-card, close-button, and menu-row hit regions.
- Test activation, close, minimize, maximize, and restore routing against a specific window ID.
- Run two windows with the same app ID under `nickel-session`; verify one panel icon, side-by-side titled previews, live thumbnail updates, activation, close-button behavior, and right-click actions.
- Run workspace tests and Clippy with warnings denied.

## Completion

Move this document to `specs/done/` after grouped rendering, hover previews, thumbnail transport, and per-window actions pass live verification.
