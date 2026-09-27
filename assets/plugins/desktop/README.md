# Desktop plugin

Nickel renders `main.js` behind the native desktop file plane. The host gives
the plugin a bounded viewport size, theme background color, and an optional
`wallpaper` image asset. JavaScript receives no wallpaper path or file pixels.

The `Surface` component fills the desktop viewport without panel padding.
The plugin can place `Box` components at bounded `x` and `y` coordinates and
style text. It currently draws the desktop error banner in JSX as well as the
background and wallpaper.
Settings can disable this plugin to restore native wallpaper painting and
shows its measured retained UI memory. File icons, selection, dragging,
context menus, and file operations remain host owned while their plugin
interfaces are developed.
