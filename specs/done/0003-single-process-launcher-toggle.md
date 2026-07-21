# Single-Process Launcher Toggle

## Objective

Remove launcher startup latency by making the panel and launcher two windows owned by one persistent `nickel-ui` process.

## Behavior

- `nickel-ui` discovers applications, opens persistent storage, initializes rendering, and creates both windows once.
- The compact panel window is visible at startup.
- The launcher window is initialized but hidden at startup.
- Clicking the panel toggles the existing launcher window between visible and hidden.
- Showing the launcher requests focus and redraws without repeating discovery, storage, font, or GPU initialization.
- Closing the launcher or pressing Escape on an empty query hides it; closing the panel exits the shell process.
- Search contents, selection, scroll position, decoded icons, and pins survive hide/show cycles.

## Boundaries

No panel-to-launcher IPC or child process is introduced. Both windows share application state and an event loop. Under `nickel-session`, a small session-control datagram asks the compositor to map or unmap the warm launcher surface because ordinary Wayland clients cannot control their own visibility. Sharing a single GPU device or atlas between surfaces is optional for this slice; eliminating repeated process and subsystem initialization is required.

## Verification

- Unit tests cover launcher visibility transitions.
- Workspace formatting, clippy, and tests pass.
- Manual nested-session testing confirms repeated clicks reuse the same compositor window identity and preserve launcher state.
