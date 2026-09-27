# Developing a Nickel plugin

On Linux, panel and dock plugins register their own shell surface identity.
Their declared size and bottom offset control compositor placement, and they
do not replace the built-in taskbar readiness surface.

Nickel loads a compiled JavaScript entry from a directory containing
`plugin.json`. The [bundled hello panel](../assets/plugins/hello-panel/) is a
minimal working example. The [taskbar](../assets/plugins/taskbar/) shows how a
first-party plugin requests host actions.

Build the Rust development tools, then start an isolated nested session on
Linux:

```sh
cargo build -p nickel --bin nickel-plugin --bin nickel-nested --features backend-winit
target/debug/nickel-plugin dev assets/plugins/hello-panel
```

On Windows, build `nickel-plugin.exe` and `nickel.exe` beside each other, then
run `nickel-plugin.exe dev assets/plugins/hello-panel`. The command starts a
separate Nickel shell without desktop windows and gives it a temporary
`LOCALAPPDATA`/`APPDATA` profile. It stops that shell and removes the profile
when you press Ctrl+C. This is a native Windows test shell, so its taskbar and
plugin surfaces appear on the current desktop.

The command validates the manifest and JavaScript, stages the package in a
temporary Nickel profile, and launches the test shell. Saving `plugin.json`,
the declared JavaScript entry, or its sibling `.jsx`/`.tsx` source validates
and restarts the test shell. An invalid edit prints its error and leaves
the previous session running. Press Ctrl+C to stop and remove the temporary
profile. The developer command supports up to 16 panel or dock surfaces in one
package, or one surface-free taskbar badge, taskbar action, desktop widget,
or Control Center section contribution;
`nickel-plugin validate <directory>` runs the same
source compilation and checks without launching a shell.
For a dock, set the surface `kind` to `"dock"`, choose a logical `width` and
`height`, and set `bottom_offset` for the gap above the output edge. The
`Panel` component's ARGB `background` can be translucent. Several installed
panel and dock plugins can be enabled together. A package can declare several
surfaces, each with a unique ID. Nickel starts a separate component instance for
each and exposes `nickel.data.surface` with its `id`, `kind`, `width`, and
`height`; the same entry can return different layouts for each ID. Surface
declarations are static until the package is updated.

The installed entry is plain JavaScript. If `main.jsx` or `main.tsx` exists
beside a declared `main.js` entry, both commands run a local
`node_modules/.bin/tsc` or `tsc` from `PATH` before validation. The `dev`
command also recompiles after each source edit. Both compile in a temporary
directory and leave the package's `main.js` untouched. TypeScript 5.6 or newer
is needed for this automatic
compile path. Generate the `.js` entry when packaging for installation; for
example:

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

The runtime provides `h`, `Panel`, `Row`, `Column`, `Text`, `Button`, `Dialog`,
`Image`, `ImageButton`, `useState`, `useRef`, and other small native components.
Images use a host-provided asset key and explicit `width` and `height` (1 to
8192 logical pixels); `fit` is `contain`, `cover`, or `stretch`. `ImageButton`
also needs an ID, an accessibility label, and an `onClick` handler. A missing
host asset renders a placeholder without giving the plugin filesystem access.
`Button` may also handle `onDrag({ phase, x, y, bounds })`; Nickel captures the
pointer through start, move, end, and cancel events. The callback can request a
typed effect, as the bundled taskbar does when moving a pinned app.
`Dialog` accepts `onClose`, called when the host dismisses an open dialog by
Escape, outside input, or focus loss. The handler should clear the state that
controls `open`; dialog buttons may still update state and request typed effects.
Nickel does not embed
React, a browser DOM, or the TypeScript compiler. The host checks declared
capabilities and current shell state before executing a requested effect.

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
identity when enabling. If the package later changes its JavaScript entry code,
entry path, version, capabilities, surfaces, or extension declarations, automatic startup stops and
Settings shows that a fresh review is required. Re-enabling records the new
declarations.
