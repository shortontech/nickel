# Launcher plugin prototype

`main.jsx` is the source of Nickel's experimental JavaScript launcher view.
`main.js` is the generated file that Nickel embeds at runtime. Rebuild it with:

```sh
tsc --allowJs --checkJs false --jsx react --jsxFactory h --target ES2020 \
  --outDir assets/plugins/launcher assets/plugins/launcher/main.jsx
```

Run the launcher in a nested shell with:

```sh
NICKEL_DEV_PLUGIN_LAUNCHER=1 cargo run -p nickel --no-default-features \
  --features backend-winit --bin nickel-nested
```

The host supplies `nickel.data.query`, up to 12 ranked search results, a
bounded pinned and recent app list, and Places. The plugin renders dashboard
and search buttons, then requests `launcher-set-query`,
`launcher-activate-result`, or `launcher-launch-dashboard` through
`nickel.request`. The host checks declared capabilities and confirms the app
ID against current launcher state before launching it. Search ranking and
application execution remain Rust services.

This is a comparison path while the full launcher, including the rest of its
dashboard, keyboard and controller behavior, menus, and dialogs, is migrated
and tested.
