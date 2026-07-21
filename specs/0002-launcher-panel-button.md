# Launcher Panel Button

## Objective

Add the first persistent Nickel panel surface: one button that opens the existing launcher while allowing the panel to remain alive independently.

## Behavior

- `nickel-panel` opens a compact native window containing a visible `Nickel` launcher button.
- Hover and pointer feedback make the button visibly interactive.
- Clicking the button starts `nickel-ui` as a child process.
- Repeated clicks while that child is running do not create duplicate launchers.
- After the launcher exits, a later click starts it again.
- Failure to start the launcher is reported without terminating the panel.
- The launcher inherits `WAYLAND_DISPLAY`, allowing both processes to run inside `nickel-session` while dogfooding.

## Boundaries

The panel is a separate process from both `nickel-session` and the launcher. This slice does not add task buttons, a clock, reserved screen space, shell IPC, positioning policy, or a platform-native dock role.

## Verification

- Unit tests cover the launcher-child state machine.
- Workspace formatting, clippy, and tests pass.
- Manual nested-session verification confirms that clicking the button maps one launcher window and that the panel remains mapped.
