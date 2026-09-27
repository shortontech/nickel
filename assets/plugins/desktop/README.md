# Desktop plugin

Nickel renders `main.js` for the desktop presentation. The host gives the
plugin a bounded viewport, theme colors, file tile positions and labels, and
shared `wallpaper` and icon image assets. JavaScript receives no wallpaper or
file path or raw image pixels.

The `Surface` component fills the desktop viewport without panel padding.
The plugin places `FileTile` components at host-projected positions and can
place `Box` components at bounded `x` and `y` coordinates. It draws the
wallpaper, file tiles, and desktop error banner in JSX. The Rust host keeps
transparent hit targets for accessibility and context-menu anchors. Selection,
dragging, file actions, and menus remain Rust-owned. Settings can disable the
plugin to restore native painting and shows its measured retained UI memory.
