# Overlay surface

This package starts with one ordinary window. Its button opens a separate,
top-right translucent overlay; the overlay's button dismisses it. The host
creates and retires the native surface and reports its retained UI memory.
The `anchor` and signed offsets in `plugin.json` place it inside the output.

Run `nickel-plugin dev assets/plugins/example-overlay` to try it in an isolated
Nickel session. The `.jsx` source is compiled before validation and reload;
`main.js` is the packaged runtime entry.
