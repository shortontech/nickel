# Taskbar menu action extension

Copy this directory to Nickel's per-user `plugins/` directory under its
manifest ID, then enable it in Settings. The enable review shows the
`org.nickel.taskbar/task-action` slot and the `launcher-show` grant. Open an
application's taskbar context menu and select **Find apps** to run this
extension's JavaScript callback and show the launcher.

`Action` accepts a unique `id`, a visible `label`, an optional `item` matching
one taskbar application ID, and `onClick(applicationId)`. Nickel checks the
current menu target and visible contribution before invoking the callback.
The callback may request only effects granted by the extension's manifest.
