# Desktop widget extension

Copy this directory to Nickel's per-user `plugins/` directory under its
manifest ID, then enable it on the Plugins page in Settings. The enable review
shows the `org.nickel.desktop/desktop-widget` slot it changes. The widget
appears on the desktop while the desktop plugin is active.

Settings exposes an **Unread messages** integer setting. Changing it refreshes
the running widget without disabling the package. The example reads
`nickel.data.settings.unread` and returns one bounded `Widget` node. Replace the
fixed example value with your own projected data when a host service for that
data is available.
