# Consumer with a renamed dependency

This is deliberately a separate Cargo workspace. Its `lights` dependency is the Twinkle package, and its native counter uses the public `ui!` and `id!` macros without an extra `twinkle` alias.

Run from the repository root:

```sh
cargo check --manifest-path crates/twinkle/tests/fixtures/renamed-dependency/Cargo.toml
```

It must resolve the native window dependency without inheriting a patch from the repository root. `cargo tree` for this workspace must contain no JSX runtime, Oxc, V8, Nickel crate, or compositor dependency.
