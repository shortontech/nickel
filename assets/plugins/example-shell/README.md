# Derived shell example

This package extends `nickel-default` version `^0.2` and uses its public exports.
It replaces `shell.taskbar`, explicitly reuses `shell.quickSettings`, and keeps
`shell.settings`. Its small `shell` entry selects a public component for each
of its three declared surfaces. It imports no default-shell source files.

The taskbar reads and activates native windows using its own `windows-read`
and `windows-focus` grants. It shows only its own declared Settings and Quick Settings surfaces. Default Quick Settings executes in the default package's
context with that package's grants; those grants do not transfer to this package.
No image grant or asset is needed because this taskbar uses window titles.

`registerSetting` adds **Example shell → Show shell title** to the shared Settings
registry. The inherited Settings component invokes the callback in this package's
context. This minimal example stores the value for the package's current lifetime;
restarting or reloading the package resets it to true.

## Copy, edit, and run

From the repository root, copy the whole directory to a directory you own. Change
both package IDs in `plugin.json`, its name/version, and then edit `main.jsx` and
`ui.css`. Keep `nickel-default` in `extends`/`requires`; adjust `uses` or `replaces`
using public export names. Declare only the surfaces you want your shell to offer.

```sh
cp -R assets/plugins/example-shell /tmp/my-nickel-shell
# Edit /tmp/my-nickel-shell/plugin.json, main.jsx, and ui.css.
cargo build -p nickel --bin nickel-plugin
# Check source, manifest, exported components, and declared surfaces.
target/debug/nickel-plugin validate /tmp/my-nickel-shell
# Stage an isolated profile and launch a live shell with source reload.
target/debug/nickel-plugin dev /tmp/my-nickel-shell
```

In the live shell, open Settings → Plugins and select **Example Derived Shell**
as the active shell. Quick Settings opens from its taskbar button. Open Settings
and select **Show shell title** in **Example shell** to exercise the owner callback.
Live edits are compiled and validated before activation. The default dependency
must remain installed and enabled; selecting another shell retires this shell's
visible surfaces while keeping approved dependency contexts.

The development command compiles sibling JSX in a temporary directory. Before
shipping an edited copy, regenerate its checked-in JavaScript:

```sh
tsc --allowJs --checkJs false --noCheck --jsx react --jsxFactory h \
  --target ES2020 --module ES2020 --outDir /tmp/example-shell-build main.jsx
cp /tmp/example-shell-build/main.js main.js
```

Run those packaging commands from your copied package directory. The shipped
`main.js` is ordinary ES module code; Nickel needs no JSX compiler at runtime.
