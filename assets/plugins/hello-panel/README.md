# Development panel

This small JSX panel is the first plugin-host prototype. Nickel renders its
`Panel`, `Row`, `Text`, and `Button` tags as native `nickel-ui` components. The
JavaScript runtime provides `h`, `useState`, and `useRef`; React is not loaded.

Build the JSX with TypeScript's CLI (used only as a development compiler):

```sh
tsc --allowJs --checkJs false --jsx react --jsxFactory h --target ES2020 \
  --outDir assets/plugins/hello-panel assets/plugins/hello-panel/main.jsx
```

Run the example in a nested Nickel session:

```sh
NICKEL_DEV_PLUGIN_PANEL=1 cargo run -p nickel --no-default-features \
  --features backend-winit --bin nickel-nested
```

To try a different compiled script, set `NICKEL_DEV_PLUGIN_PANEL_SOURCE` to
its path. `NICKEL_DEV_PLUGIN_PANEL_BOTTOM` sets the gap in logical pixels from
the bottom of each output (default 24). The example's ARGB `background` prop
controls panel transparency. Restart the nested session after changing the
script or offset.

This prototype hosts one panel shape per output and one shared JavaScript app
instance. It does not yet have plugin discovery, settings authorization,
multiple panel definitions, hot reload, or a production crash boundary.
