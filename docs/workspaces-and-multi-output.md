# Workspaces and multi-output behavior

Nickel owns an ordered workspace model with stable, monotonically allocated IDs. Windows retain
their application identity independently of workspace membership. The panel, grouped previews,
and task switching expose only members of the active workspace; activating a member on another
workspace switches first and then restores that window's focus.

The Linux session supports create, remove, switch, and move-window commands through the session
protocol and the Control Center. `Super+Control+Left/Right` switches to the adjacent workspace;
adding `Shift` moves the active window instead. Directional operations stop at the first and last
workspace rather than wrapping. Removing a workspace merges its windows into the previous stable
neighbor, or the following workspace when removing the first. A grouped window's context menu can
move that exact member to the adjacent workspace; its semantic row resolves through the rendered
menu geometry and emits the same production session command as the keyboard path.

Output snapshots report logical geometry, panel-adjusted work area, fractional scale in Wayland
protocol units (`120 == 1.0`), transform, physical dimensions, and primary identity. Desktop and
panel surfaces follow compositor output insertion order, which is the display order advertised to
SDL. Launcher placement follows the output containing the invoking pointer. Preview and context
menu X coordinates are global and select the containing output. Every output reserves its own panel
work area.

The nested backend's explicit `--test-control` capability can create and remove virtual outputs.
This reaches the production Smithay output registry, shell display reconciliation, layout, and
surface enter/leave paths. It is unavailable on the native backend. `nickel-test-input surfaces`
reports renderer/compositor-owned shell geometry and output membership, so tests do not reproduce
private placement calculations.

## Acceptance matrix

| Behavior | Nested result | Native result |
| --- | --- | --- |
| Two workspace create/switch/move | Two native Wayland KCalc windows; direct commands and `Super+Control+Left` preserved membership, geometry, and focus | Pending |
| Maximize across workspace switch | KCalc retained maximized state and exact compositor geometry after switch away and return | Pending |
| Minimize across workspace switch | KCalc remained minimized and retained `288,186 960x558` after switching away and back; it was not reused as implicit focus | Pending |
| Fullscreen across workspace switch | KCalc remained fullscreen on `DP-test` at `1280,0 1024x768` after switching away and back | Pending |
| Mixed-scale transformed output | `1024x768`, scale `180/120`, rotated 90 degrees; protocol and shell surface queries agreed | Pending |
| Per-output shell placement | Desktop/panel geometry matched each output; launcher invoked on the secondary output resolved to `DP-test` and remained reachable | Pending |
| Hot unplug and reconnect | Secondary desktop/panel left before `wl_output` withdrawal; shell survived disconnect and recreated/reused the display surfaces on reconnect | Pending |
| Window rescue on output removal | A restored KCalc moved from `1320,186` to reachable primary geometry `40,186`, then returned to `1320,154` on reconnect. A fullscreen KCalc changed from secondary `1280,0 1024x768` to primary `0,0 1280x800`, then returned exactly to the secondary bounds | Pending |

The Windows adapter continues to expose the existing OS window model. Nickel does not synthesize
virtual desktops through undocumented Windows interfaces; portable workspace management is
therefore currently Linux-session-only.
