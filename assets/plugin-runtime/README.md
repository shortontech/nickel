# Nickel JSX runtime

`bootstrap.js` defines components, hooks, Settings registration, and the public
`nickel` capability clients evaluated inside a package's JavaScript context.
Nickel runs compiled JavaScript with Boa, without Node, a browser, or a DOM.
JSX and CSS remain ordinary authorable package files.

Rust supplies bounded capability snapshots and validates effects before native
execution. Presentation and interaction logic belong in JSX; a custom Settings
page uses the same components and capability functions as any other package UI.
The optional Settings window in [nickel-default](../plugins/nickel-default/)
reads registered settings and custom pages. Its controls are replaceable public
components and inherit overridable CSS variables.

`nickel-plugin-runtime` owns module loading, hooks, render/event transactions,
and shell composition. Packages have isolated JavaScript contexts. Foreign
component callbacks retain their producing package's authority; a parent grant
does not authorize its replacement's effects. Native component parsing, layout,
input, rendering, and operating-system services remain host-owned.

The `nickel-default` Settings window uses the shared package runtime and public
APIs. See the active shell-package spec for remaining composition and ABI work.
