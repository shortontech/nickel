# Nickel plugin packages

A package directory contains a versioned `plugin.json` and the compiled
JavaScript entry named by its `entry` field. JSX is source for the developer
toolchain; Nickel runs the generated JavaScript without Node or a browser.

Validate a package and its initial JSX tree:

```sh
cargo run -p nickel --no-default-features --features backend-winit \
  --bin nickel-plugin -- validate /path/to/plugin
```

The validator supplies bounded synthetic data for Nickel's bundled plugin
surfaces, including the launcher, so their initial trees can be checked
without starting the shell. Live interactions still need `nickel-plugin dev`.

The [default shell package](nickel-default/) contains ordinary JSX/CSS modules
for shell surfaces and an optional Settings window. Plugins register settings
or custom JSX pages through the public registry; they do not need a native
Settings adapter. The default shell is the stock presentation package; derived
shells can replace its exported components and omit its Settings window.

Inspect installed packages beneath Nickel's per-user `plugins/` configuration
directory:

```sh
cargo run -p nickel --no-default-features --features backend-winit \
  --bin nickel-plugin -- list
```

Each immediate child directory must match its manifest ID. Discovery reports
invalid packages without hiding valid siblings. Settings can enable installed
packages with up to 16 panel, dock, window, dialog, or overlay surfaces.
Surface-free composition providers publish ordinary components to public collections
such as `taskbar.items`, `system.controls`, and `settings.pages`. Use
`nickel --safe-mode` to start with installed
packages inactive while keeping bundled shell plugins available.

The component vocabulary includes `Window`, `FixedWindow`, `Row`, `Column`, `Text`, `Image`,
`ImageButton`, `Button`, `Badge`, `TextField`, `Progress`, `Dialog`, `Menu`, and `MenuItem`. The
host owns image bytes and exposes them by asset name to JSX. The desktop,
lock screen, screenshot tool, file manager, and Codex remain Rust UI.
Plugin CSS scopes `color`, `font-size`, and `line-height` to a surface tree and
inherits them through layout elements into text, buttons, and text fields.
Styles on a child override inherited values.
Use `secure={true}` on a `TextField` for passwords or other private input. The
host masks its paint and blocks remote semantic inspection of that surface.
The [dialog example](example-dialog/) opens a component dialog and requests
Settings through a declared capability.
The [reserved panel example](example-reserved-panel/) spans each output and
stacks with the bundled taskbar while reserving desktop work area.
The [separate dialog example](example-surface-dialog/) opens an owned native
dialog surface. The [overlay example](example-overlay/) opens a translucent
top-right surface from a component window and dismisses it independently.
The [control contribution example](example-control-section/) publishes an ordinary
component to `system.controls`. The host expands public components in their owning
package context and validates each effect using that package's capabilities.
The bundled [on-screen keyboard](on-screen-keyboard/) shows a keyboard layout
written in JSX with host-checked key effects and recipient leases.
