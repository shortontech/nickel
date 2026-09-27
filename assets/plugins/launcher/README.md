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
and search buttons inside native scroll views. The dashboard also shows recent
projects, account, Settings, and a component logout dialog. Actions go through
typed `nickel.request` calls. The host checks declared capabilities and current
launcher state, including app and project IDs, before acting. Search ranking,
favorite state, application execution, and session authority remain Rust
services. View switching and pin buttons request those Rust state changes.
Enter submits the leading host result while search is active. Escape clears a
query, then dismisses the launcher; an open component dialog receives Escape
first.
Right-clicking an application opens a JSX-defined native menu with Launch and
Pin actions. Its requests still pass through the host's current-ID checks.

This is a comparison path while the full launcher, including keyboard and
controller behavior, complete catalog paging, and menus, is migrated and
tested.
