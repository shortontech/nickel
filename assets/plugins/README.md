# Nickel plugin packages

A package directory contains a versioned `plugin.json` and the compiled
JavaScript entry named by its `entry` field. JSX is source for the developer
toolchain; Nickel runs the generated JavaScript without Node or a browser.

Validate a package and its initial JSX tree:

```sh
cargo run -p nickel --no-default-features --features backend-winit \
  --bin nickel-plugin -- validate /path/to/plugin
```

Inspect installed packages beneath Nickel's per-user `plugins/` configuration
directory:

```sh
cargo run -p nickel --no-default-features --features backend-winit \
  --bin nickel-plugin -- list
```

Each immediate child directory must match its manifest ID. Discovery reports
invalid packages without hiding valid siblings. Settings can enable one
installed panel package and multiple surface-free taskbar badge extensions;
other external surface kinds still need dynamic native surface registration.

The component vocabulary includes `Panel`, `Row`, `Column`, `Text`, `Image`,
`ImageButton`, `Button`, `Badge`, `TextField`, `Progress`, `Dialog`, `Menu`, and `MenuItem`. A
full-viewport `Surface` component is used by the bundled desktop background
plugin. The host owns image bytes and exposes them by asset name to JSX.
The [task badge example](example-task-badge/) shows a surface-free extension
that contributes UI to the bundled taskbar's declared slot.
