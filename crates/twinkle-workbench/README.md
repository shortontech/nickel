# Twinkle component workbench

Standalone native component fixtures and preview tools. This crate has no Nickel dependency.
The library exposes `registry()` and `register()` for downstream fixture catalogs.

- `cargo run -p twinkle-workbench -- native`: component gallery with variant selection, reset and semantic/controller/keyboard activation.
- `cargo run -p twinkle-workbench -- list`: list generic fixtures.
- `cargo run -p twinkle-workbench -- validate`: check deterministic variant rendering and layout diagnostics.
- `cargo run -p twinkle-workbench -- render FIXTURE VARIANT OUTPUT.png`: export a selected variant.

Nickel application fixtures, shell providers and acceptance/evidence commands live in
`nickel-workbench`, which consumes this shared component inventory.

The optional standalone TSX preview uses the same native controls, retained patch admission, focus, keyboard and controller input as a Rust application:

```sh
cargo run -p twinkle-workbench --features jsx --bin twinkle-jsx-preview
cargo run -p twinkle-workbench --features jsx --bin twinkle-jsx-preview -- render preview.png
cargo test -p twinkle-workbench --features jsx --lib
```

Its ordinary confirmation dispatcher admits one bounded known operation per event, with at most 128 retained receipts. Unknown effects roll back provisional state and native patches. It has no Nickel catalog, manifest, service grants, session discovery or default privileged dispatcher. JSX preparation happens on load; event dispatch uses already loaded scheduler calls and typed patches.

Linux coverage: production semantic editing/activation, focused normalized keyboard text, native directional controller navigation, keyed state after row reorder, locale/theme subscription updates preserving editor identity/focus, and unknown-effect rollback. The native window survived an eight-second startup/idle probe in the active Wayland session without reported errors; the process was deliberately ended by `timeout` (exit 124). The software raster was visually inspected. This is not physical gamepad, manual Alt-Tab, multi-monitor or Windows acceptance.

The default Rust gallery dependency tree excludes V8/Oxc. Enable `jsx` only for declarative preview/runtime work. Both modes exclude Nickel crates.
