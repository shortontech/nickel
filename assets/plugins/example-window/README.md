# Component window example

This installed plugin declares a centered native `window` surface and renders
its content from a JSX `<Window>` root styled by `ui.css`. Its button opens a
component dialog, whose action asks
the host to open Nickel Settings under the `settings-show` grant.
The manifest also declares `icon.png` as `nickel-icon`; JSX displays it with
`<Image asset="nickel-icon" />`. Nickel decodes and renders the file in the
native host, without exposing its bytes or path to JavaScript.

Run `nickel-plugin dev assets/plugins/example-window` on Linux or Windows to
test it in an isolated shell. Generate `main.js` from `main.jsx` with the
TypeScript CLI before packaging the plugin for installation.
