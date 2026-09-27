# Nickel JSX bootstrap

`bootstrap.js` defines the small component and hook API evaluated inside each
plugin's JavaScript context. It has no platform or shell service bindings;
hosts supply `nickel.data` and validate requested effects. The
`nickel-plugin-runtime` crate owns the Boa context and render/event
transactions shared by the shell and Settings hosts. Native
component parsing, presentation, and effect validation remain host-owned.
Settings uses one shared native component adapter and transaction wrapper in
`crates/nickel-settings/src/settings_components.rs`; each page supplies its own
tree shape and typed effect validator.

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

Settings starts each page Boa context when its page is first opened and retires
it after navigation to another page. Building the navigation destinations for
other pages does not allocate those contexts. The navigation context stays
alive while Settings is open.

`settings-navigation.jsx` declares destination order, grouping, labels, and
headers. The native `ResponsiveNavigation` adapter owns focus, responsive
layout, and the trusted page slots while the remaining page views migrate.
Rebuild it with the same `tsc` command and its source filename.

`settings-bar.jsx` owns the ordinary Bar controls and workspace preview. Its
radio and slider callbacks request typed changes that the Settings host checks
against the current topology projection before using the existing reducers.
The native Bar view remains available if JSX fails. It uses the same build
command with `settings-bar.jsx` as the source.

`settings-optional-features.jsx` owns the ordinary Codex and on-screen keyboard
cards. Its callbacks request typed changes, retry, and disable confirmation;
the Settings host checks current policy, runtime state, and environment
overrides before applying them. Its Boa context starts only while this page is
open, and the native view remains available if JSX fails. Build it with the
same command and its source filename.

`settings-network.jsx` owns the Network page's Wi-Fi switch, discovered network
list, and adapter summary. Requests carry the observed network index and
profile; the host checks the current projection before invoking the existing
platform path. The native Network view remains available if JSX fails. Build
it with the same command and its source filename.
