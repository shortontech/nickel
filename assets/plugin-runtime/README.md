# Nickel JSX bootstrap

`bootstrap.js` defines the small component and hook API evaluated inside each
plugin's JavaScript context. It has no platform or shell service bindings;
hosts supply `nickel.data` and validate requested effects. The
`nickel-plugin-runtime` crate owns the Boa context and render/event
transactions shared by the shell host and the future Settings host. Native
component parsing, presentation, and effect validation remain host-owned.

`settings-pages.jsx` starts the bundled Settings process view migration. It
currently renders the Keyboard Shortcuts and About cards through the shared
runtime and a Settings-specific native adapter. Regenerate its shipped JS with:

```sh
tsc --allowJs --checkJs false --noCheck --jsx react --jsxFactory h \
  --target ES2020 --outDir assets/plugin-runtime \
  assets/plugin-runtime/settings-pages.jsx
```

`settings-plugins.jsx` renders the ordinary Plugins list, live memory labels,
enable and disable controls, and registered plugin settings. Its callbacks send
typed requests that the Settings host validates against the current status.
The permission review and emergency disable view remain native Rust. Rebuild
its shipped JS with the same command, replacing `settings-pages.jsx` with
`settings-plugins.jsx`.

Settings starts each Boa context when its page is first opened. Building the
navigation destinations for other pages does not allocate those contexts.

`settings-navigation.jsx` declares destination order, grouping, labels, and
headers. The native `ResponsiveNavigation` adapter owns focus, responsive
layout, and the trusted page slots while the remaining page views migrate.
Rebuild it with the same `tsc` command and its source filename.
