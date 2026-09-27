# Development panel

This small JSX panel is the first plugin-host prototype. Nickel renders its
`Panel`, `Row`, `Column`, `Text`, `TextField`, `Button`, and `Dialog` tags as
native `nickel-ui` components. Each text field routes `onChange(value)` to
its own JavaScript handler. The JavaScript runtime provides `h`, `useState`,
and `useRef`;
React is not loaded. `plugin.json` declares the entry, surface geometry,
output scope, and granted shell actions.

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

The example opens a component dialog. Its **Show** button requests the
`launcher-show` capability, which Nickel checks against `plugin.json` before
opening the launcher. The dialog uses space inside this prototype's 220-pixel
surface; a separate managed dialog surface is part of the next runtime step.

This prototype hosts one panel shape per output and one shared JavaScript app
instance. It does not yet have plugin discovery, Settings enable/disable or
memory reporting, multiple panel definitions, hot reload, or a production
crash boundary.
