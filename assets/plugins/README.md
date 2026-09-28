# Nickel plugin packages

A package directory contains a versioned `plugin.json` and the compiled
JavaScript entry named by its `entry` field. JSX is source for the developer
toolchain; Nickel runs the generated JavaScript without Node or a browser.

Validate a package and its initial JSX tree:

```sh
cargo run -p nickel --no-default-features --features backend-winit \
  --bin nickel-plugin -- validate /path/to/plugin
```

The validator supplies bounded synthetic data for Nickel's bundled shell
surfaces, including launcher and desktop, so their initial trees can be checked
without starting the shell. Live interactions still need `nickel-plugin dev`.

The [first-party Settings package](settings/) uses the same manifest format
but has a separate native Settings component adapter. Its package and page
scripts are validated by the `nickel-settings` test suite; the shell plugin
CLI currently validates shell components.

Inspect installed packages beneath Nickel's per-user `plugins/` configuration
directory:

```sh
cargo run -p nickel --no-default-features --features backend-winit \
  --bin nickel-plugin -- list
```

Each immediate child directory must match its manifest ID. Discovery reports
invalid packages without hiding valid siblings. Settings can enable installed
packages with up to 16 panel, dock, window, dialog, or overlay surfaces, plus surface-free taskbar badge,
taskbar action, desktop widget, and Control Center section extensions. Use
`nickel --safe-mode` to start with installed
packages inactive while keeping bundled shell plugins available.

The component vocabulary includes `Panel`, `Row`, `Column`, `Text`, `Image`,
`ImageButton`, `Button`, `Badge`, `Action`, `Widget`, `Section`, `TextField`, `Progress`, `Dialog`, `Menu`, and `MenuItem`. A
full-viewport `Surface` component is used by the bundled desktop background
plugin. The host owns image bytes and exposes them by asset name to JSX.
Use `secure={true}` on a `TextField` for passwords or other private input. The
host masks its paint and blocks remote semantic inspection of that surface.
The [task badge example](example-task-badge/) shows a surface-free extension
that contributes UI to the bundled taskbar's declared slot.
The [desktop widget example](example-desktop-widget/) contributes a bounded
value and progress display to the bundled desktop plugin.
The [widget host](example-widget-host/) declares a slot that another installed
plugin can fill. Run it with the [widget contributor](example-widget-contributor/)
using `nickel-plugin dev assets/plugins/example-widget-host assets/plugins/example-widget-contributor`.
The [action contributor](example-action-contributor/) adds a callback button to
that host's `commands` slot; run the three directories together to inspect both
display and interactive composition.
The [dialog example](example-dialog/) opens a component dialog and requests
Settings through a declared capability.
The [reserved panel example](example-reserved-panel/) spans each output and
stacks with the bundled taskbar while reserving desktop work area.
The [separate dialog example](example-surface-dialog/) opens an owned native
dialog surface. The [overlay example](example-overlay/) opens a translucent
top-right surface from a component window and dismisses it independently.
The [task action example](example-task-action/) adds a callback to the
taskbar's JSX application menu.
The [control section example](example-control-section/) adds a callback row
to the bundled Control Center.
