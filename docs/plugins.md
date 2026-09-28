# Developing a Nickel plugin

On Linux, panel, dock, window, dialog, and overlay plugins register their own
shell surface identity. Panels and docks use their declared size and bottom
offset; windows, dialogs, and overlays are centered on the selected output by
default. Set `anchor` to `top-left`, `top-right`, `bottom-left`, or
`bottom-right` and add signed `offset_x` and `offset_y` to place one at a
corner. Placement is clamped to the output.
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

The command validates the manifest and JavaScript, stages the package in a
temporary Nickel profile, and launches the test shell. Saving `plugin.json`,
a declared image, the JavaScript entry, or its sibling `.jsx`/`.tsx` source validates
and restarts the test shell. An invalid edit prints its error and leaves
the previous session running. Press Ctrl+C to stop and remove the temporary
profile. For first-party shell plugins, pass a bundled directory such as
`assets/plugins/launcher`, `assets/plugins/desktop`, or `assets/plugins/taskbar`.
The developer command runs edited JavaScript in the isolated shell without
installing a second copy of that plugin. Keep its shipped `plugin.json`
unchanged while developing it. Saving a sibling `.js` source such as the
taskbar's `menu.js` also restarts the session. For external plugins, the
developer command supports up to 16 panel, dock, window, dialog, or overlay surfaces in one package, or
one surface-free taskbar badge, taskbar action, desktop widget, Control
Center section, or installed-plugin widget/action contribution;
`nickel-plugin validate <directory>` runs the same
source compilation and checks without launching a shell.
For bundled launcher, desktop, Control Center, preview, and volume surfaces,
validation supplies bounded synthetic host data so their initial JSX tree can
render without a live session. It checks the initial tree and manifest; use
`dev` to exercise input, requested actions, and live state changes.
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
Disabling Window Preview closes its visible cards and task switcher view;
hovering a taskbar group no longer creates a Rust preview. Re-enabling the
plugin restores the JSX preview.
Disabling Notifications closes ordinary popups and history, releases its UI
memory, and leaves Super+N inactive until re-enabled. Pending remote access and
Codex approval requests retain a trusted notification surface so users can
review and decide them even while the plugin is disabled.
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

The runtime provides `h`, `Panel`, `Viewport`, `Row`, `Column`, `Text`, `Button`,
`Dialog`, `Image`, `ImageButton`, `useState`, `useRef`, and other small native
components.
`Viewport` fills its host window and accepts an ARGB `background` and `padding`
from 0 to 256 logical pixels. Use it as the root for a full-window layout such
as the bundled launcher; its size follows the declared surface and output.
Inside a `Viewport`, `<ScrollView id="items" grow={true}>` takes the remaining
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
`Dialog` accepts `onClose`, called when the host dismisses an open dialog by
Escape, outside input, or focus loss. The handler should clear the state that
controls `open`; dialog buttons may still update state and request typed effects.
For example, `{ type: "show-settings" }` opens Nickel Settings when the plugin
has the `settings-show` capability. Nickel launches its bundled Settings
executable when installed beside the shell, including from a nested session;
the application catalog remains a fallback.
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
The bundled desktop's `FileTile` registers `onFileAction({ action })` for Cut,
Copy, Rename, Properties, and Open in Terminal. It requests
`{ type: "desktop-file-action", id, action }` with the tile's projected ID and
declares `desktop-files-manage`. Nickel checks the grant, rendered tile, current
file identity, and output before carrying out the request. The bundled JSX
desktop presents the file context menu; native menus remain available when the
plugin is disabled, and file windows retain their own presentation. The bundled
desktop background menu is also JSX; its View and Sort By submenus request
typed `desktop-background-action` effects under `desktop-control`, while the
host checks the current menu context before changing desktop state.

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
contributions then follow the winner. The desktop also provides a
`desktop-widget` slot. A surface-free plugin can contribute a `widget` using
`h(Widget, { label: "Unread mail", value: "12 messages", percent: 60 })`.
The [example desktop widget](../assets/plugins/example-desktop-widget/) is a
working package. Widgets appear in a bounded stack on the desktop; at most
three are visible. Replacement selection and additive ordering follow the
same rules as badges.
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
