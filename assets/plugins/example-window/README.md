# Component window example

This installed plugin declares a centered native `window` surface and renders
its content with JSX. Its button opens a component dialog, whose action asks
the host to open Nickel Settings under the `settings-show` grant.

Run `nickel-plugin dev assets/plugins/example-window` on Linux or Windows to
test it in an isolated shell. Generate `main.js` from `main.jsx` with the
TypeScript CLI before packaging the plugin for installation.
