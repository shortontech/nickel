# Taskbar plugin comparison path

`main.jsx` is the source for Nickel's bundled JavaScript taskbar view.
`main.js` is generated with:

```sh
tsc --allowJs --checkJs false --jsx react --jsxFactory h --target ES2020 \
  --outDir assets/plugins/taskbar assets/plugins/taskbar/main.jsx
```

Set `NICKEL_DEV_PLUGIN_TASKBAR=1` in a nested Nickel session to compare this
view with the current Rust taskbar. The host supplies bounded grouped tasks,
tray items, icon slots, and a clock label. The plugin requests typed launcher,
task, tray, and control-center actions; the host rechecks item IDs against live
groups and the visible tray before acting. Task menus, pin/drag behavior,
Codex and keyboard controls, and full visual parity are still in the migration
queue.

The image slots reference buffers already owned by the host. Settings' tracked
native UI figure remains a lower bound and does not charge shared icon buffers
to this plugin.
