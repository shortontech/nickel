# Server-Side Window Decorations

## Goal

Give normal Wayland application windows one coherent Nickel-owned frame instead of mixing Qt,
GTK, Chromium, and toolkit-default borders.

## Protocol

- Advertise `zxdg_decoration_manager_v1` and prefer `server_side` decoration mode.
- Do not decorate Nickel desktop, panel, launcher, menus, overlays, or other shell surfaces.
- Respect fullscreen and intentionally undecorated surfaces.
- Avoid double frames when a client refuses server-side decoration and continues drawing a
  client-side header bar.

## Frame

- Draw the frame in `nickel-session`, outside the client content surface.
- Use a consistent titlebar height, resize border, focused/unfocused palette, and title text.
- Use the installed FontAwesome window glyphs: minimize `U+F2D1`, maximize `U+F2D0`, restore
  `U+F2D2`, and close `U+F2D3`.
- Keep glyph geometry and hit targets stable across output scales.

## Interaction

- Titlebar drag moves the window.
- Border and corner drags resize it.
- Double-clicking the titlebar toggles maximize/restore.
- Minimize, maximize/restore, and close buttons invoke the existing compositor window actions.
- Frame clicks focus and raise the associated application without forwarding the click into client
  content.

## Layout

- Initial, moved, maximized, restored, and rescued geometry treats the frame and content as one
  window.
- Maximized windows occupy the work area without covering the Nickel panel.
- Fullscreen windows have no frame and occupy the selected output.
- Frames participate in output damage and screenshot composition.

## Verification

- Confirm Qt, GTK, Chromium, and Nickel-owned application behavior.
- Confirm applications that negotiate server-side decorations do not draw a second frame.
- Exercise focus, move, all resize edges, minimize, maximize/restore, close, task switching,
  multiple outputs, output scaling, and fullscreen.
- Confirm all shell surfaces remain undecorated and absent from task switching.

## Completion

Archive this specification when ordinary compliant Wayland applications use functional Nickel
frames and refusing clients remain usable without double decorations.
