# Screenshot plugin

`main.jsx` defines Nickel's ordinary screenshot overlay; `main.js` is the
compiled entry shipped with the shell. Run
`nickel-plugin dev assets/plugins/screenshot` to edit it in an isolated nested
profile, then press Print Screen in that session.

The host owns capture, selection, cropping, clipboard writes, file saves, and
focus. It passes viewport and selection geometry, status, and capture generation
through `nickel.data`. The image stays in host memory and reaches JSX as the
`capture` image asset. Toolbar buttons send a typed `screenshot-action` with
the current generation; stale requests are rejected.

The `screenshot-control` capability is reserved for this bundled plugin.
Settings can disable or re-enable its overlay without leaving a native
screenshot tool running behind it.
