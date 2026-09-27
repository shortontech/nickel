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

The host supplies `nickel.data.query` and up to 12 ranked application results.
The plugin renders the native text field and result buttons, and requests
`launcher-set-query` or `launcher-activate-result` through `nickel.request`.
The host checks the declared capabilities and confirms a result's ID before
launching it. Search ranking and application execution remain Rust services.

This is a comparison path while the full launcher, including dashboard,
keyboard and controller behavior, menus, and dialogs, is migrated and tested.
