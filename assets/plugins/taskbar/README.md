# Taskbar plugin

`main.jsx` is the source for Nickel's bundled JavaScript taskbar view.
`main.js` is generated with:

```sh
tsc --allowJs --checkJs false --jsx react --jsxFactory h --target ES2020 \
  --outDir assets/plugins/taskbar assets/plugins/taskbar/main.jsx
```

`menu.jsx` and `window-menu.jsx` are the sources for its two context menus.
Compile them with the same command, replacing `main.jsx` with the desired
source. `nickel-plugin dev assets/plugins/taskbar` compiles all three JSX
sources into its isolated profile when they change.

The bundled taskbar runs by default. The host supplies bounded grouped tasks,
tray items, icon slots, and a clock label. The plugin requests typed launcher,
task, tray, and control-center actions; the host rechecks item IDs against live
groups and the visible tray before acting. The JSX task buttons use Nickel's
captured pointer drag events to request a one-step pin move. Rust checks the
current pinned item ID and position before persisting it. Full visual parity
remains in the migration queue.

`task-badge` contributions arrive in `nickel.data.slots["task-badge"]`; the JSX
view chooses which task receives each badge and limits visible badges to three.
Right-clicking a task now requests its host-owned application menu through a
separate `windows-context` capability. The host checks the current group ID
and index before showing the menu.
The JSX taskbar also includes Codex projects and on-screen keyboard buttons
when the host reports those features as available. Their requests require
declared capabilities, and the host checks availability again before acting.
The application and per-window action menus render from the bundled JSX plugin.
The taskbar provides a `task-badge` slot. Independent, surface-free plugins can
return `Badge` components for task IDs; Nickel composes up to three badges per
task in declared priority and plugin ID order. Disabling a badge plugin removes
its contribution without restarting the taskbar.
The context menu reads `nickel.data.slots["task-action"]` and renders contributed
actions as JSX buttons. It invokes them through the same action slot path used
by other plugins. The host validates the projected action and captured item
before dispatching it.

The image slots reference buffers already owned by the host. Settings' tracked
native UI figure remains a lower bound and does not charge shared icon buffers
to this plugin.
