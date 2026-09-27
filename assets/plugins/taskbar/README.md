# Taskbar plugin comparison path

`main.jsx` is the source for Nickel's bundled JavaScript taskbar view.
`main.js` is generated with:

```sh
tsc --allowJs --checkJs false --jsx react --jsxFactory h --target ES2020 \
  --outDir assets/plugins/taskbar assets/plugins/taskbar/main.jsx
```

Set `NICKEL_DEV_PLUGIN_TASKBAR=1` in a nested Nickel session to compare this
view with the current Rust taskbar. The host supplies bounded grouped tasks
and a clock label. The plugin requests typed launcher, task, and control-center
actions; the host rechecks the task ID against its current group before
focusing or launching anything. Icons, tray, menus, drag behavior, and
multi-output state are still in the migration queue.
