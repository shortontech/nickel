# Developing a Nickel plugin

On Linux, panel, dock, window, dialog, and overlay plugins register their own
shell surface identity. Panels and docks use their declared size and bottom
offset; windows, dialogs, and overlays are centered on the selected output by
default. Set `anchor` to `top-left`, `top-right`, `bottom-left`, or
`bottom-right` and add signed `offset_x` and `offset_y` to place one at a
corner. Placement is clamped to the output.
An overlay can set `passive: true` to appear above ordinary windows without
activating its native window on Windows.
They do not replace the built-in taskbar readiness surface.

Nickel loads a compiled JavaScript entry from a directory containing
`plugin.json`. The [bundled hello panel](../assets/plugins/hello-panel/) is a
minimal working example. The [taskbar](../assets/plugins/taskbar/) shows how a
first-party plugin requests host actions.
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
`nickel.request({ type: "show-plugin-surface", surfaceId: "details" })`. The
host accepts only a window, dialog, or overlay ID from that plugin's own manifest and
ignores a request for a surface that is already open. The
[two-window example](../assets/plugins/example-two-windows/) shows the request
in a working package. A window can request
`nickel.request({ type: "hide-plugin-surface", surfaceId: "details" })` to close
itself or a declared sibling; closing its last ordinary surface disables the
package.
The [dialog example](../assets/plugins/example-dialog/) shows `useState`,
`onClose`, a `show-settings` request, and a saved plugin setting.
The [separate dialog example](../assets/plugins/example-surface-dialog/) declares
an initially closed `dialog` surface beside a home window. It opens the dialog
with `show-plugin-surface` and dismisses it with
`nickel.request({ type: "hide-plugin-surface", surfaceId: "confirm" })`.
The dialog's optional `owner` field names a declared window on the same output
scope. An owned dialog can open only while that window is live; closing the
owner also closes its dialog. Windows presents it as a native owned window and
blocks input to that owner until the dialog closes.
Closing the package's last ordinary window also retires its open dialogs.
The [overlay example](../assets/plugins/example-overlay/) uses the same
`show-plugin-surface` and `hide-plugin-surface` requests for a centered,
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

`nickel-settings --plugin-status` prints the live plugin status as JSON when
run as an authorized Settings companion. It lets Windows development sessions
inspect which first-party plugins actually started.
On Windows, `nickel-settings.exe --plugin-status --plugin-status-file status.json`
writes the same snapshot to a file, including when the GUI executable has no
console output handle.

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
profile. For first-party shell plugins, pass a bundled directory such as
`assets/plugins/launcher` or `assets/plugins/taskbar`.
The developer command runs edited JavaScript in the isolated shell without
installing a second copy of that plugin. Keep its shipped `plugin.json`
unchanged while developing it. Saving a sibling `.js` source such as the
taskbar's `menu.js` also restarts the session. For external plugins, the
developer command supports up to 16 panel, dock, window, dialog, or overlay surfaces in one package, or
one surface-free taskbar badge, taskbar action, Control
Center section, or installed-plugin widget/action contribution;
`nickel-plugin validate <directory>` runs the same
source compilation and checks without launching a shell.
If a surface needs data to render its initial JSX tree, add a
`"validation_data"` object in `plugin.json` keyed by surface ID. Each value
must be an object of synthetic sample values. Validation combines it with
host-owned `settings`, `slots`, and `surface` data for that surface; sample
data cannot replace those fields. This checks the initial tree and manifest;
use `dev` to exercise input, requested actions, and live state changes.
The bundled Run dialog is wholly rendered by its JS plugin. Disabling that
plugin closes an open Run dialog; its shortcut stays inactive until the plugin
is enabled again in Settings.
Disabling the bundled Launcher plugin also closes its surface and leaves its
Meta shortcut inactive. Run remains available through Super+R, and the
independent Settings shortcut can re-enable Launcher.
Disabling the bundled Taskbar plugin retires its visible bar. On nested Linux,
the desktop reclaims the bar's reserved work area until it is re-enabled.
Taskbar context menus are JSX views owned by that plugin. Disabling Taskbar
closes an open menu; menu requests stay unavailable until it is re-enabled.
Disabling Volume OSD closes that overlay and clears its retained native UI;
audio changes do not recreate a Rust fallback.
Disabling Control Center closes ordinary Quick Settings. The trusted display
projection chooser remains available for recovery.
The ordinary view uses a plugin overlay; the trusted chooser uses a separate
host surface. On Linux, opening it from the taskbar places the overlay beside
that control on its output.
Disabling Window Preview closes its visible cards and task switcher view;
hovering a taskbar group no longer creates a Rust preview. Re-enabling the
plugin restores the JSX preview.
Disabling Notifications closes ordinary popups and history, releases its UI
memory, and leaves Super+N inactive until re-enabled. Ordinary notification
popups and history use the plugin's passive overlay surface. Pending remote
access and Codex approval requests retain a trusted notification surface so
users can review and decide them even while the plugin is disabled.
Super+N focuses the plugin overlay for keyboard navigation; passive arrivals
leave the current application focused.
For a dock, set the surface `kind` to `"dock"`, choose a logical `width` and
`height`, and set `bottom_offset` for the gap above the output edge. The
`Panel` component's ARGB `background` can be translucent. Several installed
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

The runtime provides `h`, `Panel`, `Div`, `Row`, `Column`, `Text`, `Button`,
`Window`, `FixedWindow`, `Slot`, `Dialog`, `Image`, `ImageButton`, `Slider`, `useState`, `useRef`, and other small native
components.
An optional `"stylesheet": "ui.css"` in `plugin.json` loads a CSS file of at most
256 KiB. `className` accepts space-separated class names on `Window`, `Panel`,
`Box`, `Div` (also `<div>`), `Row`, `Column`, `ScrollView`, `Spacer`, `Slot`, `Text`, `Button`,
`TextField`, and `Slider`. For example, `<Button className="primary" onClick={save}>Save</Button>`
matches `button.primary { padding: 8px; background: #345678; }`. The supported
selectors are element names, `.class`, and `#id`, combined without descendant
selectors. Pseudo-classes such as `:focus` and plugin `onFocus`/`onBlur`
callbacks are not exposed yet. Rules use source order. Supported declarations are `padding`,
`margin`, `border` (solid only), `border-width`, `border-color`,
`border-radius`, `font-size`, `line-height` (pixel lengths), `background` or
`background-color`, `color`, `gap`, `width`, `height`, `min-width`, `max-width`,
`min-height`, `max-height`, `display`, `flex-direction`, `flex`, `flex-grow`,
`flex-shrink`, `flex-basis`, `align-items`, `justify-content`, and
`grid-template-columns`. Generic `<div>` defaults to a vertical block layout;
`display: flex` defaults to a row and `display: grid` uses Nickel's native grid.
Grid tracks support pixels, fractions, `auto`, bounded `repeat()`, and `minmax()`.
Lengths support pixels, percentages, `auto`, `min-content`, and `max-content`
where Nickel's layout context permits them. Colors accept hex, `rgba()`,
and `transparent`. Unsupported selectors or declarations fail validation with a
CSS error. Button and text-field behavior and accessibility remain native;
their plugin-facing paint comes from CSS. The existing JSX `width` and `height`
props remain available. Row, Column, and specialized widgets still have some
legacy sizing behavior while the generic layout path expands.

When the host supplies a right to left reading direction, `Row`, horizontal
flex layouts, and grids mirror their visual child order. Text and artwork keep
their own content direction.

Color declarations may use Nickel palette tokens such as
`var(--nickel-panel)`, `var(--nickel-surface)`, `var(--nickel-text)`,
`var(--nickel-muted)`, and `var(--nickel-accent)`. The complete token set is
`background`, `panel`, `surface`, `surface-hover`, `text`, `muted`, `accent`,
`accent-soft`, `complement`, `raised`, `control`, `border`, `soft-text`,
`selected`, and `selected-border`, each prefixed with `--nickel-`. Nickel resolves
these when compiling the stylesheet. A host can refresh the resolved colors
when its appearance changes; the bundled launcher is wired to do so. Other
CSS custom properties are not supported yet.

`<Slider value={hue / 359} accessibilityLabel="Hue" onChange={fraction =>
nickel.request({type: "appearance-hue", fraction})} />` is a native slider with a
value from 0 to 1. It receives a stable automatic control ID unless `id` is
provided. CSS `background`, `color`, and `border-color` style its track, fill,
and thumb; width and spacing use the ordinary CSS declarations.

`<Window width={520} height={340}>...</Window>` is the JSX
surface root. `<FixedWindow>` is a JavaScript helper that returns a `Window`
with fixed placement; it does not create a second renderer. In the current
manifest version, numeric root dimensions must stay within the declared surface bounds;
`"100%"` fills the host surface. Requested output, edge, anchor, work-area
reservation, and bottom offset are checked against the manifest before the
render is accepted. The manifest still determines native placement until the
surface authority envelope replaces its duplicated geometry. A plugin cannot
create an undeclared native window by changing JSX. `id` is optional: the host
supplies its surface identity. An explicit ID must match that identity.
One plugin may declare several surfaces. Nickel renders the plugin for each
surface with `nickel.data.surface.id` set to that host's ID, so the JSX can
return the matching `Window` root. Each render currently has its own runtime;
sharing one runtime across sibling windows is a later host change.
Controls do not need explicit IDs for ordinary rendering or event handling.
Nickel derives stable control IDs from the tree path; list items rendered from
arrays inside a `Window` must use unique `key` values, such as `key={item.id}`.
The root `Window` or `FixedWindow` can handle Enter and Escape through
`onSubmit` and `onEscape` JavaScript callbacks. Their requested desktop actions
receive the same capability checks as button callbacks.
For example, a plugin granted `control-center-show` can call
`nickel.request({type: "toggle-control-center"})` from a button or a root
shortcut handler.
The same pattern supports `toggle-launcher` with `launcher-show`,
`toggle-on-screen-keyboard` with `on-screen-keyboard-show`, and
`toggle-projects-menu` with `projects-menu-show`.
`dismiss-launcher` also requires `launcher-show`.

`<Slot id="content" />` marks a place where a host can insert its own component
tree into the JSX layout. The ID names the insertion point; a slot with no
host-provided content is empty. CSS can style the slot box with `slot#content`
or a class name. The Settings window uses this to place its active page while
its window and navigation are authored in JSX.

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
typed effect, as the bundled taskbar does when moving a pinned app.
Plugins granted `windows-read` receive a bounded `nickel.data.windows` array.
Each window has a string `id`, title, application ID, active state, and
`canActivate`/`canClose` flags. A plugin with `windows-focus` may request
`{ type: "window-action", action: "activate", window: id }`; `windows-context`
permits `action: "close"`. Nickel checks the current window and its capability
again when handling the request.
`Dialog` accepts `onClose`, called when the host dismisses an open dialog by
Escape, outside input, or focus loss. The handler should clear the state that
controls `open`; dialog buttons may still update state and request typed effects.
For example, `{ type: "show-settings" }` opens Nickel Settings when the plugin
has the `settings-show` capability. Nickel launches its bundled Settings
executable when installed beside the shell, including from a nested session;
the application catalog remains a fallback.
Add `screen: "appearance"` (or another Settings screen name) to open a
specific page. Unknown screen names are rejected before the request reaches
the desktop.
`{ type: "show-control-center" }` opens Quick Settings with the
`control-center-show` capability.
Declare a typed setting in `plugin.json` to expose it in Settings and read its
current value from `nickel.data.settings`. A component may save its own declared
setting with `nickel.request({ type: "set-plugin-setting", key: "open-count",
value: 1 })` when the plugin has `settings-write`. The host checks the setting
name and value against the manifest, persists the change, and refreshes the
plugin's active surfaces. That refresh starts fresh component state, so keep
durable values in declared settings rather than `useState`.
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

Manifests may declare typed composition relationships. A target declares a
`provides_slots` entry with an ID, a `badge`, `widget`, `action`, or `section`
contract, and whether replacement is allowed. An extension declares a
`contributes` entry with `target_plugin`, `target_slot`, matching `contract`,
and `mode` (`add` or `replace`). Nickel validates these declarations and shows
them in Settings' enable review. The first executable slot is the taskbar's
`task-badge` slot: a package with no surface can declare one `badge`
contribution targeting `org.nickel.taskbar/task-badge`. Its `App` returns
`h(Badge, { item: "application-id", label: "Unread mail", count: 3 })`; Nickel
places the badge beside the matching task. Additive badge plugins compose in
priority and plugin ID order, with a limit of three visible badges per task.
A `replace` contribution replaces the slot's base badges; if several are
enabled, the highest priority wins, with plugin ID breaking ties. Additive
contributions then follow the winner.

Installed plugins can also provide their own `widget` slots. The provider's
JSX reads `nickel.data.slots[slotId]`, an array of bounded objects with
`pluginId`, `label`, `value`, `percent`, and `color`. Nickel refreshes this data
when a contributor is enabled, disabled, or changes a setting. The provider
chooses where and how to render it. A replacement contribution wins by priority
and plugin ID; additive contributions follow, with at most eight widgets per
slot. The [widget host](../assets/plugins/example-widget-host/) and
[widget contributor](../assets/plugins/example-widget-contributor/) demonstrate
the relationship. Run both in one isolated session:

```sh
nickel-plugin dev assets/plugins/example-widget-host assets/plugins/example-widget-contributor
```

Saving either package restarts that session. For an installed plugin, Settings
shows the declared target and slot before enablement.
An installed provider may also declare an `action` slot. It receives bounded
`{pluginId, id, label}` entries in `nickel.data.slots[slotId]` and can render
them as buttons. On click it requests
`{type: "invoke-plugin-slot-action", slot, pluginId, id}`. Nickel checks that
the action is still projected and dispatches the callback in the contributing
plugin's own JS instance, where its declared grants apply. The
[action contributor](../assets/plugins/example-action-contributor/) adds an
Open launcher button to the widget host's `commands` slot. Run the host and
both contributors together with one `nickel-plugin dev` command.
The taskbar also provides a `task-action` slot with the `action` contract. A
surface-free extension can return
`h(Action, { id: "find-apps", item: "org.example.app", label: "Find apps", onClick: applicationId => nickel.request("show-launcher") })`.
Omit `item` to show the action in every application menu. Nickel renders at
most four contributed actions in that JSX menu, ordered by replacement winner
then additive priority and plugin ID. The callback runs in the contributing
plugin's own JS host; its requested effects still require that plugin's
declared capabilities and current host validation. The
[example task action](../assets/plugins/example-task-action/) demonstrates this.
Control Center provides a `control-section` slot with the `section` contract.
An extension can return
`h(Section, { id: "find-apps", label: "Applications", value: "Search the catalog", onClick: () => nickel.request("show-launcher") })`.
Up to four sections appear in its scrollable JSX view. Replacement selection
and additive ordering match the taskbar slots. The callback executes in the
contributor's JS host under its own grants. The
[example control section](../assets/plugins/example-control-section/) is a
working package.
For a quick edit loop, run
`nickel-plugin dev assets/plugins/example-control-section` (or use the
[task action example](../assets/plugins/example-task-action/)). The command
stages the extension in an isolated profile and reloads it after edits.
Settings measures the retained native component tree in each extension's own
account. The target plugin's rendered UI measurement also includes contributed
nodes, so these category totals overlap; they should not be added to estimate
process memory. Each extension's JavaScript heap measurement remains unavailable.

Settings lists each plugin with an enable switch and shows its runtime health,
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
choice cycling, and a text editor. Changes are validated against the manifest
and applied to a running installed panel, badge, or widget without disabling it.
The manifest may also declare bounded `author` and `version` strings. Settings
shows both in the plugin list and in the enable review; missing values are
labeled unknown or unspecified.
For installed plugins, Nickel saves the reviewed manifest access and package
identity when enabling. If the package later changes its JavaScript entry code
or a declared image,
entry path, version, capabilities, surfaces, or extension declarations, automatic startup stops and
Settings shows that a fresh review is required. Re-enabling records the new
declarations.
