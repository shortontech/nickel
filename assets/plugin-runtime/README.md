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
