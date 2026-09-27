# Nickel JSX bootstrap

`bootstrap.js` defines the small component and hook API evaluated inside each
plugin's JavaScript context. It has no platform or shell service bindings;
hosts supply `nickel.data` and validate requested effects. The
`nickel-plugin-runtime` crate owns the Boa context and render/event
transactions shared by the shell host and the future Settings host. Native
component parsing, presentation, and effect validation remain host-owned.
