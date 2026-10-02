# Developing a Nickel plugin

On Linux, panel, dock, window, dialog, and overlay plugins register their own
shell surface identity. Panels and docks use their declared size and bottom
offset; windows, dialogs, and overlays are centered on the selected output by
default. Set `anchor` to `top-left`, `top-right`, `bottom-left`, or
`bottom-right` and add signed `offset_x` and `offset_y` to place one at a
corner. Placement is clamped to the output.
An overlay can set `passive: true` to appear above ordinary windows without
activating its native window on Windows.
Standalone surfaces coexist with the selected shell package.

Nickel loads a compiled JavaScript entry from a directory containing
`plugin.json`. The [bundled hello panel](../assets/plugins/hello-panel/) is a
minimal working example. The [default shell package](../assets/plugins/nickel-default/)
shows ordinary taskbar, launcher, Settings, and Quick Settings components using
public capability clients.
The [reserved panel example](../assets/plugins/example-reserved-panel/) shows
how to span each output and reserve work area alongside the taskbar. Remove
`reserve_work_area` and set `bottom_offset` for a floating dock.
The [window example](../assets/plugins/example-window/) shows a centered native
window containing JSX components, a packaged image, and a dialog.
A window plugin participates in ordinary window focus and stacking. Closing
its only surface disables the plugin without exiting Nickel; enable it again
in Settings to reopen it. For a package with several surfaces, closing one
window retires that surface and leaves its siblings running. Disable and
re-enable the package to reopen all its declared surfaces. A running sibling
can reopen one declared window with
`nickel.surfaces.show("details")`. The
host accepts only a window, dialog, or overlay ID from that plugin's own manifest and
ignores a request for a surface that is already open. The
[two-window example](../assets/plugins/example-two-windows/) shows the request
in a working package. A window can request
`nickel.surfaces.hide("details")` to close
itself or a declared sibling; closing its last ordinary surface disables the
package.
An open ordinary window can request a new position on its current output with
`nickel.surfaces.setPlacement("details", {anchor: "top-right", offsetX: -24, offsetY: 24})`.
Only a window declared by the requesting plugin can be moved. Offsets are
logical pixels within ±8192; Nickel clamps the result to the output. The
window manager still handles normal user movement and resizing.
The [dialog example](../assets/plugins/example-dialog/) shows `useState`,
`onClose`, a `show-settings` request, and a saved plugin setting.
The [separate dialog example](../assets/plugins/example-surface-dialog/) declares
an initially closed `dialog` surface beside a home window. It opens the dialog
with `nickel.surfaces.show("confirm")` and dismisses it with
`nickel.surfaces.hide("confirm")`.
The dialog's optional `owner` field names a declared window on the same output
scope. An owned dialog can open only while that window is live; closing the
owner also closes its dialog. Windows presents it as a native owned window and
blocks input to that owner until the dialog closes.
Closing the package's last ordinary window also retires its open dialogs.
The [overlay example](../assets/plugins/example-overlay/) uses the same
`nickel.surfaces.show` and `nickel.surfaces.hide` functions for a centered,
translucent overlay. Overlays start closed, can be dismissed independently,
and are retired with the package's last ordinary surface.

Build the Rust development tools, then start an isolated nested session on
Linux:

```sh
cargo build -p nickel --bin nickel-plugin --bin nickel-nested --bin nickel-test-input --features backend-winit
target/debug/nickel-plugin dev assets/plugins/hello-panel
```

The Linux dev command prints a private `test-control.env` path for its nested
session. In another terminal, load that file and send Super to the nested shell:

```sh
set -a
. /path/printed/by/nickel-plugin/test-control.env
set +a
target/debug/nickel-test-input key meta pressed
target/debug/nickel-test-input key meta released
target/debug/nickel-test-input surfaces
```

This targets the nested session, so it does not press a controller Guide button
or toggle the host desktop's launcher. The file is removed with the temporary
profile when dev mode stops.

The ordinary shell's Plugins Settings page reads `nickel.plugins.get()` and
`list()` with a `plugins-read` grant. Package status and telemetry are reported
by the shared host. Lifecycle and configuration requests require
`plugins-control` and the currently observed activation revision.

On Windows, build `nickel-plugin.exe` and `nickel.exe` beside each other, then
run `nickel-plugin.exe dev assets/plugins/hello-panel`. The command starts a
separate Nickel shell without desktop windows and gives it a temporary
`LOCALAPPDATA`/`APPDATA` profile. It stops that shell and removes the profile
when you press Ctrl+C. This is a native Windows test shell, so its taskbar and
plugin surfaces appear on the current desktop.

The command validates the manifest, JavaScript, and declared CSS, stages the package in a
temporary Nickel profile, and launches the test shell. Saving `plugin.json`,
a declared image or stylesheet, the JavaScript entry, or its sibling `.jsx`/`.tsx` source validates
and restarts the test shell. An invalid edit prints its error and leaves
the previous session running. Press Ctrl+C to stop and remove the temporary
profile. To edit shell components, use `assets/plugins/nickel-default`.
Saving a sibling JavaScript module such as `src/Taskbar.js` also restarts the
session. The developer command supports up to 16 declared surfaces per package
and ordinary surface-free public composition providers. You can pass several
package directories to develop dependencies and contributions together.
`nickel-plugin validate <directory>` runs source compilation and package checks
without launching a shell.

When a plugin needs initial host data for offline validation, add a
`"validation_data"` object in `plugin.json` keyed by surface ID. Each value
must be synthetic data for that surface. Host-owned resource fields take
precedence over sample data. Validation checks the initial tree and manifest;
use `dev` to exercise input, effects, and live resource changes.

The selected shell package owns its taskbar, launcher, Quick Settings,
notification, and optional Settings surfaces. Replacing an exported component
uses ordinary package composition. Disabling or selecting a different shell
retires the old visible surfaces; dependencies keep their own capability
contexts. Trusted recovery and approval surfaces remain native.
Bundled companion packages such as Run, Volume OSD, and Window Preview still
have their own activation lifecycle. Disabling them closes their ordinary
surfaces without creating a Rust presentation fallback.

For a dock, set the surface `kind` to `"dock"`, choose a logical `width` and
`height`, and set `bottom_offset` for the gap above the output edge. The
`FixedWindow` root's ARGB `background` can be translucent. Several installed
panel, dock, and window plugins can be enabled together. A `"window"` surface
uses its declared size, is centered on its output, and has no bottom offset.
A window, dialog, or overlay can declare an `anchor` and offsets, for example
`"anchor":"top-right","offset_x":-18,"offset_y":24`. Other surface kinds
cannot use window anchoring. A package can declare several
surfaces, each with a unique ID. Nickel starts a separate component instance for
each and exposes `nickel.data.surface` with its `id`, `kind`, `width`, and
`height`; the same entry can return different layouts for each ID. Surface
declarations are static until the package is updated.

The installed entry is plain JavaScript. If `main.jsx` or `main.tsx` exists
beside a declared `main.js` entry, both commands run a local
`node_modules/.bin/tsc` or `tsc` from `PATH` before validation. The `dev`
command also recompiles after each source edit. Both compile in a temporary
directory and leave the package's `main.js` untouched. On Windows the tool
looks for `tsc.cmd` in the local package or on `PATH`. TypeScript 5.6 or newer
is needed for this automatic compile path. Generate the `.js` entry when
packaging for installation, for example:

```sh
tsc --allowJs --checkJs false --noCheck --noEmitOnError --jsx react \
  --jsxFactory h --target ES2020 --lib ES2020 --module none --outDir . main.jsx
```

Copy [Nickel's JSX declarations](../assets/plugins/nickel-plugin.d.ts) into
your source project and reference them from your `.jsx` or `.tsx` file. The
hello-panel example includes a `tsconfig.json` for editor checking; run
`tsc -p assets/plugins/hello-panel` from the repository root to check its
props and hooks. Use `lib: ["ES2020"]`: plugins have no browser DOM, and the
DOM library's `Text` and `Image` globals conflict with Nickel's components.
The `dev` command transpiles with `--noCheck` so an editor type error does not
prevent testing; `tsc -p` gives the stricter check before packaging.

In a nested test session, inspect computed component geometry through the
authenticated test socket:

```sh
nickel-test-input layouts
nickel-test-input layout internal:2
```

`layouts` lists the current internal surface IDs, roles, geometry, plugin keys,
and resolved node counts. `layout` reports the selected surface's production
tree, including allocated and content bounds, flex sizes, scrolling, clipping,
and child indexes. IDs can change when a surface is recreated. These queries
are available only when nested test control is enabled; protected surfaces are
excluded.

The runtime provides `h`, `Window`, `FixedWindow`, `Div`, `Row`, `Column`, `Text`, `Button`,
`Slot`, `Dialog`, `Image`, `ImageButton`, `Slider`, `useState`, `useRef`, and other small native
components.
`Panel` remains as a compatibility helper for older plugins; it composes a
`FixedWindow`, `Box`, and `Row` and has no separate native renderer. New plugins should
use `Window` or `FixedWindow` as their surface root.
### CSS selectors and layout

An optional `"stylesheet": "ui.css"` in `plugin.json` loads a CSS file of at most
256 KiB. Modules can also import package CSS. Native components accept
space-separated `className` values. For example,
`<Button className="primary" onClick={save}>Save</Button>` matches
`button.primary { padding: 8px; background: #345678; }`.

Selectors support element names, `.class`, `#id`, their combinations, comma
lists, and descendant chains of up to eight selectors, such as
`.settings button.primary`. Matching rules apply in source order; Nickel does
not implement browser selector specificity. `:root` accepts custom property
definitions only.

Supported declarations are `padding`, `margin`, `border` (solid only),
`border-width`, `border-color`, `border-radius`, `font-size`, `line-height`
(pixel lengths), `box-shadow` (`x y blur spread color`), `background` or
`background-color`, `color`, `gap`, `width`,
`height`, `min-width`, `max-width`, `min-height`, `max-height`, `display`,
`flex-direction`, `flex`, `flex-grow`, `flex-shrink`, `flex-basis`, `align-items`,
`justify-content`, `text-align` (`start`, `center`, or `end`), and
`grid-template-columns`. Floating docks may additionally use
`-nickel-dock-magnification: <maximum-scale> <sibling-radius>` with a bounded
`transition-duration` in milliseconds. Nickel applies the falloff to neighboring
children in native layout, keeps hit geometry synchronized, and schedules the
interpolated frames itself. Text alignment inherits through containers.
Generic `<div>` defaults to a
vertical block layout; `display: flex` defaults to a row, and `display: grid`
uses Nickel's native grid. Grid tracks support pixels, fractions, `auto`,
bounded `repeat()`, and `minmax()`. Lengths support pixels, percentages, `auto`,
`min-content`, and `max-content` where the native layout context permits them.
Colors accept hex, `rgba()`, and `transparent`. Unsupported selectors or
declarations fail validation.

Centered, translucent taskbar surfaces are compositor materials: Nickel samples
and blurs the scene behind the surface before composing its CSS background,
border, shadow, and content. A full-width taskbar remains on the ordinary opaque
panel path.

The JSX `width` and `height` props remain available. Root component layout uses
ordinary CSS width, height, percentages, and flex sizing. When the host supplies
a right to left reading direction, rows, horizontal flex layouts, and grids
mirror their visual child order. Text and artwork retain their content direction.

`Button` accepts an integer `iconSize` from 8 through 128 logical pixels; the
default is 32. `iconPlacement="top"` places the icon above its visible label
inside one focusable hit area; the default is `"left"`. An optional `description` (up to 4096 bytes) adds a second text
line inside the same button hit area; style it with `text.button-description`.
`TextField` accepts `aria-label` or `accessibilityLabel` for its
accessible name, falling back to its placeholder when neither is supplied.

Compositor-owned package scenes receive `nickel.data.viewport` with the current
surface `width` and `height`, output name, and the output work area's
`availableWidth` and `availableHeight` in logical pixels. Available dimensions
can be null when the output is unknown. Components can use these bounds to adapt
their requested size and layout when moving between outputs.

### Inherited variables and semantic defaults

Custom properties such as `--card-accent` cascade in source order, inherit from
ancestors, and can be overridden on a component or native part. `var()` resolves
against that element's inherited properties before the result is checked as a
typed color, length, or other supported declaration. Nested fallbacks work:
`color: var(--label-color, var(--nickel-text));`.

Nickel supplies overridable semantic defaults. Color tokens are `background`,
`panel`, `surface`, `surface-hover`, `text`, `muted`, `accent`, `accent-soft`,
`complement`, `raised`, `control`, `border`, `soft-text`, `selected`, and
`selected-border`, each prefixed with `--nickel-`. Aliases include
`--nickel-surface-raised` and `--nickel-text-muted`. Metric defaults are
`--nickel-radius-control: 8px`, `--nickel-radius-card: 12px`,
`--nickel-spacing-control: 8px`, `--nickel-font-size: 14px`, and
`--nickel-line-height: 20px`. Host appearance changes refresh palette defaults;
package overrides remain part of the cascade.

```css
:root { --nickel-radius-control: 10px; --card-accent: var(--nickel-accent); }
.settings { --card-accent: #507080; }
.settings button {
    background: var(--card-accent);
    color: var(--button-label, var(--nickel-text));
    border-radius: var(--nickel-radius-control);
}
```

Resolution is bounded: function nesting is limited to eight levels, custom
property chains to 32 references, and resolved values to 16 KiB. Undefined
variables without a fallback, cycles, and values invalid for the destination
property are rejected. This is a typed native styling subset, not a browser CSS
engine; custom properties do not add arbitrary layout declarations or scripting.

### Interactive paint and native control parts

Buttons, text fields, selects, sliders, switches, checkboxes, color swatches,
select options, and menu items support `:hover`, `:active`, and `:focus` paint.
State rules can set `background`/`background-color`, `color`, `border`/`border-color`,
`border-width`, `border-radius`, `font-size`, and `line-height`, including
transparent backgrounds. State rules can additionally set `width` and `height`,
allowing controls such as dock icons to magnify through production layout and
hit testing. Other layout declarations such as padding, margin, or flex remain
unavailable in state rules. State and ordinary selectors must be in separate
rules. `Button` and `TextField` also accept `onFocus` and `onBlur`;
focus changes dispatch blur before focus, and window focus loss dispatches blur.

Native compound controls expose ordinary element selectors for their parts:

| Control | Part selectors |
| --- | --- |
| Switch | `switch-track`, `switch-thumb` |
| Checkbox | `checkbox-box`, `checkbox-mark` |
| Slider | `slider-track`, `slider-fill`, `slider-thumb` |
| ScrollView | `scrollbar-track`, `scrollbar-thumb` |
| Select | `select-header`, `option`, `select-indicator` |
| Progress | `progress-fill` |
| Color swatch | `color-swatch-fill`, `color-swatch-label` |
| Menu | `menu`, `menu-item`, `menu-shortcut`, `menu-indicator` |
| Text field | `text-field-caret`, `text-field-selection`, `text-field-menu`, `text-field-menu-item`, `text-field-menu-shortcut`, `text-field-menu-indicator` |

For ScrollView scrollbar parts, `scrollbar-track` width sets thickness and
the right component of `margin` sets the edge inset in pixels.
`scrollbar-thumb` height sets the minimum thumb length; native
viewport/content proportions and scroll position
control its actual length and position. Background, border, rounding and
`:hover`, `:active`, `:focus` paint are CSS controlled. Parts share the owning
ScrollView's ID, classes and variables. Native pointer acquisition keeps a
minimum target size; CSS thickness also enlarges it. Scroll semantics are unchanged.

Parts inherit their owner's classes and custom properties and can match its ID.
Select options can also have their own IDs.
Interactive parts use their owning control or menu item's state; decorative
parts do not become independent focus targets. Progress fill and text field
caret/selection are ordinary paint parts. State classes such as `.on`, `.mixed`,
`.disabled`, and `.selected` can style the corresponding native state.
Compound part geometry uses pixel lengths; root layout retains ordinary layout
sizing. Select header/option and menu row geometry determines native hit regions
and accessibility bounds. See the replaceable
[default control stylesheet](../assets/plugins/nickel-default/src/styles/controls.css)
for a working set of part rules. Native behavior, focus, and accessibility remain
host owned.

### Public slider values and appearance changes

A slider supports numeric `min`, `max`, and `step`; callbacks receive the selected
value in that range. Its default range is 0–1. It receives a stable automatic
control ID unless `id` is provided. For an appearance editor with
`appearance-read` and `appearance-control` grants:

```jsx
function HueControl() {
    const appearance = nickel.appearance.get();
    if (!appearance.available) return <Text>Appearance is unavailable.</Text>;
    const hue = appearance.configured.accent_hue ?? appearance.resolved.hue;
    return appearance.writable ? <Slider id="appearance-hue"
        accessibilityLabel="Interface hue" min={0} max={359} step={1} value={hue}
        onChange={value => nickel.appearance.set({
            ...appearance.configured, accent_hue: value
        })} /> : <Text>{"Hue: " + hue + "° · Read only"}</Text>;
}
```

`nickel.appearance.set` captures the observed configuration and generation for
native validation. Slider root and part paint comes from CSS; track/fill/thumb
rules can independently set their paint and pixel geometry. Their interactive
paint follows the slider's hover, pressed, and focus state.

`<Window width={520} height={340}>...</Window>` is the JSX
surface root. `<FixedWindow>` is a JavaScript helper that returns a `Window`
with fixed placement; it does not create a second renderer. In the current
manifest version, numeric root dimensions must stay within the declared surface bounds;
`"100%"` fills the host surface. Requested output, edge, anchor, work-area
reservation, and bottom offset are checked against the manifest before the
render is accepted. A grant for all outputs permits JSX to choose the primary
output; a work-area reservation grant permits JSX to omit the reservation;
and `bottom_offset` is the maximum dock distance JSX may request. These resolved
values reach native placement. The manifest still determines the surface kind
and anchor until the full authority envelope replaces its duplicated geometry.
`Window` and `FixedWindow` accept `onFocus` and `onBlur` callbacks for native
window activation and focus loss. They are separate from control focus callbacks.
Duplicate native reports are ignored, and an initial focus loss before activation
does not call `onBlur`. Control blur callbacks run before window blur in one
transaction against the existing callback generation. A package can dismiss a
transient window with `onBlur={() => nickel.surfaces.hide("menu")}`.

`window.dock { bottom: 20px; }` also sets the bottom distance for a `Window`
root with `className="dock"`; CSS `bottom` requires a `window` selector and
cannot exceed the manifest bound. An explicit JSX `bottomOffset` wins over CSS.
For a top-anchored window or overlay, `window.notice { top: 14px; }` sets the
distance from the output's top edge within the manifest's positive `offset_y`
bound. A root cannot set both CSS `top` and `bottom`.
A plugin cannot create an undeclared native window by changing JSX. `id` is optional: the host
supplies its surface identity. An explicit ID must match that identity.
When state or host data changes a root's allowed size, output scope, or
placement, Nickel updates the live native surface after validating the new
tree. An invalid dynamic request leaves the previous tree and surface in place.
One plugin may declare several surfaces. Nickel renders the plugin for each
surface with `nickel.data.surface.id` set to that host's ID, so the JSX can
return the matching `Window` root. Sibling windows share one JavaScript
runtime while retaining separate hooks, handlers, and data for each surface.
Closing a sibling retires its state; reopening it starts fresh.
Module-level JavaScript values are shared by that runtime, so read
`nickel.data.surface` inside a component or handler when behavior depends on
the current surface.
Controls do not need explicit IDs for ordinary rendering or event handling.
Nickel derives stable control IDs from the tree path; list items rendered from
arrays inside a `Window` must use unique `key` values, such as `key={item.id}`.
The root `Window` or `FixedWindow` can handle Enter and Escape through
`onSubmit` and `onEscape` JavaScript callbacks. Their requested desktop actions
receive the same capability checks as button callbacks.
For example, a plugin granted `control-center-show` can call
`nickel.surfaces.show("quick-settings")` from a button or root shortcut handler.
Use `nickel.surfaces.show`, `hide`, or `focus` for the selected shell's launcher,
Settings, notification, and Quick Settings surfaces with their corresponding
grants. `nickel.keyboard.toggle()` requires `on-screen-keyboard-show`, and
`nickel.projects.toggle()` requires `projects-menu-show`.

`<Slot id="content" />` is a native content insertion point for hosts that
provide a component tree. An empty insertion point renders nothing. CSS can
style its box with `slot#content` or a class name. Public package composition
uses exported components and semantic collections, described below; it does
not use native slot data projections.

Inside a `<Window>`, `<ScrollView id="items" grow={true}>` takes the remaining
height and shrinks when the window does. Use `height` for a fixed-size scroll
area; `grow` and `height` cannot be combined.
JavaScript execution has a 100,000-iteration limit per call frame. A loop that
exceeds it returns an error to the plugin host; a failed event rolls back its
component state so the next input can still run.
Images use an asset key and explicit `width` and `height` (1 to
8192 logical pixels); `fit` is `contain`, `cover`, or `stretch`. Installed
packages can declare up to 16 PNG, JPEG, or WebP files under `images` in
`plugin.json`, for example `"images": [{"id":"logo","path":"logo.png"}]`.
Use `<Image asset="logo" width={48} height={48} />` to display one. Each file
is limited to 4 MiB, all image files to 16 MiB, and their decoded total to
4 million pixels. Nickel decodes them in Rust and counts retained RGBA pixels
in the plugin's native memory lower bound; GPU texture memory remains a
separate unavailable category. `ImageButton`
also needs an ID, an accessibility label, and an `onClick` handler. A missing
host asset renders a placeholder without giving the plugin filesystem access.
`Button` may also handle `onDrag({ phase, x, y, bounds })`; Nickel captures the
pointer through start, move, end, and cancel events. The callback can request a
typed effect, as the default shell taskbar does when moving a pinned app.
Plugins granted `windows-read` can call `nickel.windows.list()` for a copied,
bounded window inventory. `nickel.windows.activate(id)` requires `windows-focus`;
`nickel.windows.close(id)` and window menu operations require `windows-context`.
The host rechecks the current window and grants before native execution.

`nickel.applications.list()` requires `applications-read` and returns up to 256
entries with stable ID, display name, and pinned state.
`nickel.applications.launch(id)` requires `applications-launch`;
`nickel.applications.togglePin(id)` and `movePin(id, direction)` require
`applications-pin`. Requests resolve against current application identities.

`nickel.notifications.get()` requires `notifications-read`. The bounded snapshot
contains ordinary notifications and history. `nickel.notifications.invoke(id,key)`
and `dismiss(id)` require `notifications-act` and recheck live notification
identity. Trusted approval notifications are excluded from plugin data/actions.

Plugins granted `run-command` use `nickel.run.get()` and
`nickel.run.execute(command, expectedRevision?)`. Execution captures the current
owner revision by default. Nickel rechecks the current grant, running owner,
unlocked session, and revision before passing parsed arguments to the native
Run adapter. Empty commands, NUL characters, and commands over 4096 characters
are rejected. The read snapshot includes `available`, `revision`, and a nullable
status message. The selected shell may declare an optional `run` surface for
the Run shortcut; omission leaves the shortcut unavailable.
Use `nickel.windows` for window actions and `nickel.audio.get/setVolume/setMuted`
for audio controls; `selectOutput(id)` selects a current output device.
Audio writes require `audio-control` and current native service authority.
The host validates them against current grants and native service state.

`Dialog` accepts `onClose`, called when the host dismisses an open dialog by
Escape, outside input, or focus loss. The handler should clear the state that
controls `open`; dialog buttons may still update state and request typed effects.
`nickel.surfaces.show("settings")` opens the selected shell's ordinary Settings
surface with `settings-show`. For a native page navigation intent, the checked
`show-settings` effect can include `screen: "appearance"` or another supported
destination; the host forwards navigation to that shell's registered page.
Settings renders the destination through its ordinary package runtime.

Use `registerSetting` and `registerSettingsPage` to publish ordinary settings
controls and JSX pages. Registration metadata and callbacks remain owned by the
providing package; generic Settings controls invoke that package's callback
through the public registry. Manifest metadata settings remain readable through
`nickel.data.settings`. The public plugin management client exposes
`nickel.plugins.setSetting(id,key,value,revision)` with `plugins-read` and
`plugins-control`; it validates the declared kind, observed prior value, and
current inventory revision before persisting an edit.

`Menu` contains up to 16 `MenuItem` children. An item can have an `onClick`
handler, a `disabledReason`, or nested `MenuItem` children with a `label` to
form a submenu. Items can also declare `shortcut` text and `separatorBefore`.
Set both `x` and `y` to place a menu at a pointer position relative to its
declared anchor surface.
The host renders and navigates these as native menu rows, including the
disabled state and submenu hierarchy.
Nickel does not embed
React, a browser DOM, or the TypeScript compiler. The host checks declared
capabilities and current shell state before executing a requested effect.
The desktop, lock screen, screenshot tool, file manager, and Codex use Rust UI.
Ordinary plugins can still declare their own windows and composition slots.

### Public package composition

The manifest's versioned `composition` object publishes components with
`exports`, extends a shell with `extends` and versioned `requires`, replaces
inherited contracts with `replaces`, or reuses a dependency's public export
through `uses`. Reuse references name the dependency `package` and `export`;
private implementation modules are not public imports.
`nickel.component("shell.taskbar")` returns the selected component for that
contract. The default shell also exposes contracts such as `shell.settings`,
`shell.settings.navigation`, and `shell.settings.controls`.
See the [default package's public exports](../assets/plugins/nickel-default/plugin.json).

Semantic `contributions` publish ordinary components to collections such as
`taskbar.items`, `system.controls`, and `settings.pages`. For example, the
[control contribution package](../assets/plugins/example-control-section/)
declares:

```json
"composition": {
  "api_version": 1,
  "id": "org.nickel.example-control-section",
  "version": "0.2.0",
  "exports": { "example.controls.applications": "./main.js#ControlSection" },
  "contributions": [{
    "collection": "system.controls",
    "id": "find-apps",
    "implementation": "./main.js#ControlSection"
  }]
}
```

Its exported `ControlSection` is ordinary JSX-compatible JavaScript. Quick
Settings reads `nickel.contributions("system.controls")` and renders the returned
components:

```jsx
const sections = nickel.contributions("system.controls");
<Column>{sections.map(entry => <entry.component key={entry.key} />)}</Column>
```

Entries expose stable identity and an actual component reference. Each component
runs in its producing package's context; nested callback effects retain that
package's grants. Ordering is deterministic by contribution priority and
identity. Contribution callbacks use the same ordinary event transaction as
other package components. Edit the existing contribution with:

```sh
nickel-plugin dev assets/plugins/example-control-section
```

Public Settings pages can be published with `registerSettingsPage` or ordinary
components contributed to `settings.pages`. Settings reads the public registry
and collection and renders those components through the same runtime. The
Plugins page's access review displays declared exports, replacements,
dependencies, and contributions before activation.

Nickel measures the retained native component tree in each extension's own
account. The target plugin's rendered UI measurement also includes contributed
nodes, so these category totals overlap; they should not be added to estimate
process memory. Each extension's JavaScript heap measurement remains unavailable.

The Plugins page lists each plugin with activation controls and shows its runtime health,
tracked memory, peak tracked memory, and the measured categories. Tracked memory
is a lower bound: native UI bytes are measured for rendered plugin trees, while
the embedded JavaScript heap and some shared allocations cannot yet be assigned
to an individual plugin. An unavailable category is shown as unavailable rather
than counted as zero. Disabling a plugin retires its host and clears its
reported memory.

Start Nickel with `--safe-mode` to keep installed plugins from starting
automatically for that session. Bundled shell plugins retain their saved
enablement. Installed packages remain listed in Settings and may be enabled
manually after the shell starts. Safe mode does not rewrite saved enablement;
the next normal start follows the saved choices again.

Plugins may declare up to 32 bounded settings in `plugin.json`: `boolean`,
`integer` with `min`/`max`, `text` with `max_length`, or `choice` with an
`options` list. Each setting has an ID, label, and default. Nickel validates
the schema and stores values separately per plugin. An installed plugin reads
its effective values through `nickel.data.settings`, and the Plugins page
shows the registered values. Settings provides switches, increment controls,
selects, and a text editor. Changes are validated against the manifest
and applied to the running package without disabling it.
The manifest may also declare bounded `author` and `version` strings. Settings
shows both in the plugin list and in the enable review; missing values are
labeled unknown or unspecified.
For installed plugins, Nickel saves the reviewed manifest access and package
identity when enabling. If the package later changes its JavaScript entry code
or a declared image,
entry path, version, capabilities, surfaces, or extension declarations, automatic startup stops and
Settings shows that a fresh review is required. Re-enabling records the new
declarations.

### Optional features and shortcut reference

`nickel.features.get()` requires `features-read` and returns a copied native
snapshot with an opaque revision, operation availability, keyboard preference,
Codex policy, configured and acknowledged generations, installation and health.
`features-control` permits `setKeyboardMode("automatic" | "enabled" | "disabled")`,
`setCodexEnabled(enabled, confirmed = false)`, and `retryCodex()`. Each request
captures the current revision. The host rechecks grants, session authority,
policy, environment overrides and the persisted preference before committing.
The native service reconciles the saved preference; an unavailable runtime is
reported separately from successful persistence. If `disableConfirmationRequired`
is true, JSX must confirm before calling `setCodexEnabled(false, true)`.
Linux resource counters currently report unavailable, so disabling always asks
for confirmation there. `retryCodex()` refreshes the existing native controller
on Linux. Windows reports this operation unavailable; its native subscription
still reconciles committed keyboard and Codex preferences.

`nickel.shortcuts.get()` requires `shortcuts-read` and returns stable shortcut
reference IDs, keys, scopes and actual global registration availability. Its
`editable` value is false: Nickel currently has no native shortcut remapping
service. Ordinary Settings pages register with `registerSettingsPage`; these
clients do not expose page indexes or native page views.

### Workspaces, desktop and display presets

`nickel.workspaces.get()` requires `workspaces-read`. It returns copied workspace
rows with stable decimal string IDs, the active ID, an opaque revision and
operation availability. `switch(id)`, `create()` and `remove(id)` require
`workspaces-switch` and capture the current revision. Nickel checks the current
inventory, grant and session lock before dispatching to the native owner.
`nickel.desktop.get()` and `toggleShowDesktop()` require `desktop-control`.
Workspace and show-desktop operations currently report unavailable on Windows.

`nickel.displays.previewProjection(mode)` accepts an ID from
`nickel.displays.get().projectionModes`. It requires `display-control` and uses
the same revision, ownership, 15-second recovery and `confirm()`/`revert()`
service as a full display layout. Presets preserve output modes and transforms.
Windows currently exposes an empty preset list. Super+P opens the trusted native
display recovery chooser independently of the selected shell's Quick Settings
component. Ordinary Quick Settings is an ordinary JSX surface; retired
`control-action` requests are unavailable.

### Build a derived shell

[The derived shell example](../assets/plugins/example-shell/README.md) provides a
complete copy/edit/validate/live-dev starting point. It extends the versioned
`nickel-default` public API, replaces the taskbar, explicitly reuses default Quick
Settings, and exposes a registered setting through inherited Settings. Components
execute with their author's grants in the shared package host; selecting a public
component does not grant its caller the provider's capabilities.
