# Window Context Menu

## Goal

Establish reusable context-menu behavior, beginning with right-click actions for open-window icons in the panel.

## Behavior

- `nickel-ui` owns one warm, undecorated context-menu window rather than creating a window on every click.
- Right-clicking a task icon opens the menu directly above that icon and associates it with the selected neutral `WindowId`.
- The initial menu contains `Close window`; unsupported window operations are not displayed.
- Hovering an enabled item provides visible feedback. Left-clicking it invokes the action and dismisses the menu.
- Clicking the panel outside a task icon, opening the launcher, or losing menu focus dismisses the menu.
- Closing a window while its menu is open dismisses the stale menu.

## Platform Boundary

- The UI emits a neutral `WindowAction` for a neutral `WindowId`.
- The Linux adapter translates that action to the temporary Nickel session transport.
- `nickel-session` resolves its session-scoped ID and sends the Wayland close request.
- Windows will map the same action to its native window backend without inheriting Wayland or Unix transport semantics.

## Verification

- Menu layout and hit testing have focused tests.
- Right-clicking a live task icon shows the menu in the expected location.
- Selecting `Close window` closes the correct application and removes its task icon.
- `cargo test --workspace` and workspace Clippy pass.
