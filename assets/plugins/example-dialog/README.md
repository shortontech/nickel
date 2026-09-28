# Settings dialog example

This installed panel opens a JSX dialog. Its buttons request the typed
`show-settings` and `set-plugin-setting` effects. Nickel opens Settings only
with the declared `settings-show` grant, and saves the panel's own bounded
`open-count` preference only with `settings-write`. Escape, outside input, and
focus loss call `onClose` so component state follows the native dialog state.

Generate the JavaScript entry and validate the package from the repository root:

```sh
tsc --allowJs --checkJs false --noCheck --jsx react --jsxFactory h \
  --target ES2020 --outDir assets/plugins/example-dialog assets/plugins/example-dialog/main.jsx
cargo run -p nickel --bin nickel-plugin -- validate assets/plugins/example-dialog
```

Use `nickel-plugin dev assets/plugins/example-dialog` to try it in a test shell.
