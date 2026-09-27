# Taskbar badge extension

This package adds a badge beside the task whose application ID is
`org.example.mail`. Change `item` in `main.js` to an application ID on your
taskbar, then copy this directory to Nickel's per-user `plugins/` configuration
directory and enable it in Settings. The enable review shows the taskbar slot
it changes. Disabling this package removes only its badge.

The plugin owns its JavaScript state. Nickel validates the declared slot and
count, routes the output through the taskbar plugin, and keeps window actions
in the host. A count of zero hides the badge. This example uses a fixed count;
service subscriptions for live data are still being developed.
