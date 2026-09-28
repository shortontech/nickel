# Settings dialog example

This installed panel opens a JSX dialog. Its confirmation button requests the
typed `show-settings` effect. Nickel opens Settings only when the package has
the declared `settings-show` grant. Escape, outside input, and focus loss call
`onClose` so the component state follows the native dialog state.

Generate the JavaScript entry and validate the package from the repository root:

```sh
tsc --allowJs --checkJs false --noCheck --jsx react --jsxFactory h \
  --target ES2020 --outDir assets/plugins/example-dialog assets/plugins/example-dialog/main.jsx
cargo run -p nickel --bin nickel-plugin -- validate assets/plugins/example-dialog
```

Use `nickel-plugin dev assets/plugins/example-dialog` to try it in a test shell.
