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
opening the launcher. The dialog is managed by the panel's UI host; a separate
dialog window is not yet supported.

Installed packages under the user's Nickel `plugins/<plugin-id>/` directory
appear in Settings. Settings can enable or disable one external panel package
at a time and shows its declared access, health, and measured retained UI
memory. The panel uses the manifest's surface height and bottom offset. JS heap
measurement, multiple external surfaces, and hot reload remain open work.
