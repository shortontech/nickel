# Desktop plugin

Edit `main.jsx`, then compile it to the shipped `main.js` with:

```sh
tsc --allowJs --checkJs false --noCheck --jsx react --jsxFactory h \
  --target ES2020 --outDir assets/plugins/desktop assets/plugins/desktop/main.jsx
```

Nickel renders `main.js` for the desktop presentation. The host gives the
plugin a bounded viewport, theme colors, file tile positions and labels, and
shared `wallpaper` and icon image assets. JavaScript receives no wallpaper or
file path or raw image pixels.

The `Surface` component fills the desktop viewport without panel padding.
The plugin places `FileTile` components at host-projected positions and can
place `Box` components at bounded `x` and `y` coordinates. It draws the
wallpaper, file tiles, and desktop error banner in JSX. The Rust host keeps
transparent hit targets for accessibility and context-menu anchors. A tile's
`onSelect` handler requests `desktop-select` on a primary press; Nickel checks
the current projected file ID before applying its native selection policy.
The `onClick` handler requests `desktop-open` with its projected file ID for
double-click, Enter, controller Confirm, and semantic tile activation; Nickel
checks that ID against the current desktop before opening it. Modifier
interpretation, group selection, dragging, and menus remain Rust-owned.
Settings can disable the
plugin to restore native painting and shows its measured retained UI memory.
