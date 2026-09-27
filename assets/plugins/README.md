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
invalid packages without hiding valid siblings. The current shell still needs
a dynamic native surface registry before external packages can be enabled from
Settings. The bundled plugins in this directory are live comparison paths.
