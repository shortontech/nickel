# Nickel JSX bootstrap

`bootstrap.js` defines the small component and hook API evaluated inside each
plugin's JavaScript context. It has no platform or shell service bindings;
hosts supply `nickel.data` and validate requested effects. The
`nickel-plugin-runtime` crate owns the Boa context and render/event
transactions shared by the shell and Settings hosts. Native
component parsing, presentation, and effect validation remain host-owned.
Settings uses one shared native component adapter and transaction wrapper in
`crates/nickel-settings/src/settings_components.rs`; each page supplies its own
tree shape and typed effect validator.

The first-party Settings package and its JSX build commands are documented in
[the Settings plugin README](../plugins/settings/README.md).
