# Nickel default shell package

This directory contains the default shell module graph described by Spec 0263.
`plugin.json` uses the current validated package schema, with composition metadata
and bounded grants for every surface. JSX imports refer to emitted JavaScript
paths so the developer compiler can stage the entire graph. Nickel activates this
package as its stock shell; optional windows share its runtime.

The package supplies Taskbar, Launcher, Quick Settings, Notifications, and the
optional Settings window through ordinary JSX components and CSS. Its manifest
exports public replacement contracts and declares capabilities and native surfaces.
A selected derived shell can replace components or omit a surface entirely.

Settings consumes `readPluginSettings().settings` and
`readPluginSettingsPages().pages`. Qualified provider identities keep equal setting
IDs distinct. Native resources and operations remain capability checked; the
components own navigation, layout, drafts, and confirmation UI. Foreign custom
page component references and independent contributor lifecycle are still being
completed.

## Public contracts

- `Shell` / `shell`
- `Taskbar` / `shell.taskbar`
- `Launcher` / `shell.launcher`
- `QuickSettings` / `shell.quickSettings`
- `Notifications` / `shell.notifications`
- `Settings` / `shell.settings`
- `SettingsNavigation` / `shell.settings.navigation`
- `SettingControl` / `shell.settings.controls`

The Taskbar renders `nickel.contributions("taskbar.items")`; Quick Settings
renders `nickel.contributions("system.controls")`. Each entry exposes `id`,
`provider`, `version`, a stable `key`, and a `component` suitable for JSX.
The native host resolves and invokes that component in its owning package
context, retaining its callbacks and validating effects against its own grants.

Public component data props cross package contexts as snapshots. Callable event
props such as `onChange` return no value and invoke the original owner's
handler through a native callback grant. Each effect uses the grant and identity
of the code that produced it. Values needed during rendering are passed as data;
Settings evaluates its value getter before invoking a replacement control.
Components selected from the same package keep their local JavaScript values.

## Application search

`nickel.applications.search(query)` requests native fuzzy search for the calling
package. `nickel.applications.searchResults()` returns a copied snapshot with
`query`, `results`, `total`, and `truncated`. Both inventory and search reads
require `applications-read`. Queries accept at most 512 characters, and a
snapshot contains at most 256 applications. The query echoed in the snapshot
lets a component distinguish pending search from an empty result.

Applications carry stable `id`, `icon`, `kind`, `pinOrder`, and `recentOrder`
metadata. Launch and pin actions use `id`; `icon` is a bounded stable asset key.
The default Launcher owns its query, tabs, pagination, and menus. It does not
change the native launcher's query. Native project overview remains an explicit
entry point until project capabilities replace it.

## Settings controls and pages

Generic Settings controls include switches, sliders, selects, actions, editable
text/numbers, repeated keyed rows, shortcut chords, and grouped controls. The
replaceable `ColorPicker` supplies swatches, hue, saturation, brightness, optional
alpha, and CSS color entry. Text and number controls retain local drafts until
Apply and show validation errors.

Ordinary pages register Appearance, Default Apps, Displays, Wi-Fi, Bluetooth,
Desktop/taskbar preferences, Idle behavior, Preferred applications, File artwork,
Plugins, Optional features, Keyboard shortcut reference, and About. The Settings
binary and per-page hosts are retired.

The package loads through a shared composed runtime. Focused tests verify
registration, declared surfaces, inherited selection, caller-owned callbacks,
native children, and distinct package image namespaces. Live nested checks have
passed Launcher, Settings, hue/intensity/color controls, and installed window
lifecycle. Overall acceptance remains incomplete while native screenshot capture,
full control styling, and remaining composition lifecycle work are being finished.

## Public component lookup

The package host publishes `composition.exports` from the validated manifest.
`nickel.component("shell.taskbar")` returns that module's actual component
function. Exported modules share the entry's module cache, hooks, and CSS, even
when the entry does not import them directly. Missing modules or exports and
non-function exports reject package startup. The default `Shell` resolves its
surface components through these contracts.

The stock composed host applies inherited replacements and contributions. Each
package retains its own execution context and grants.

## Appearance capability migration

`Appearance.jsx` registers an ordinary Settings page in this package's module
graph. It reads configured preferences and resolved native hue/intensity, and
uses `nickel.appearance` / `nickel.wallpaper` for changes. Theme modes, preset
accent hues, custom hue entry, hue/intensity sliders, system accent inheritance,
transparency, animations, wallpaper positions/catalog selection/reset, and
appearance reset are implemented in JSX. The manifest requests the four domain
grants. No page-specific Rust host or action-index adapter was added.

Wallpaper labels/previews and image choosing use public native capabilities; the
chooser reports cancellation and unavailable platform support. File artwork uses
its ordinary registered Settings page.

`DefaultApps.jsx` registers the Applications page through the same module graph.
It consumes association targets/handlers and native revision strings, renders
search and current/protected/read-only state, and reports native consent and
write results. Writes invoke `nickel.associations.setDefault` with stable native
identities; no Rust Settings projection or indexed page request is involved.
Native parser validation and synthetic identity/consent/protection/search checks
passed. Large catalog pagination and live visual acceptance remain unfinished.

`Displays.jsx` owns display selection, draft layout, drag arrangement/snapping,
mode selection, scale, and primary display selection. It submits whole layouts
with native connector identities; changed native snapshots discard stale drafts.
Preview confirmation uses capability snapshot state. Orientation controls require
an explicit native operation-availability flag. Application-scale policy and
identify-display operations use their public availability flags. No old Display page adapter is
used by this module. Native preview/revert authority remains in Nickel.

## Embedded build inputs

The checked-in `src/*.js` modules are emitted from the authorable JSX using the
installed TypeScript compiler in JavaScript mode. Regenerate them from the
repository root after changing JSX:

```sh
tsc --allowJs --noResolve --checkJs false --jsx react --jsxFactory h --jsxFragmentFactory Fragment --target ES2022 --module ES2022 --outDir /tmp/nickel-default-js assets/plugins/nickel-default/src/*.jsx
for source in assets/plugins/nickel-default/src/*.jsx; do
  name=$(basename "$source" .jsx)
  cp "/tmp/nickel-default-js/$name.js" assets/plugins/nickel-default/src/
done
```

Cargo embeds ordinary package files verbatim into a generated catalog.
`nickel_shell::bundled_plugin_assets::load_package("nickel-default")` loads the
manifest, module sources, CSS, and declared images entirely from that catalog.
Neither building nor running Nickel invokes npm or a JSX compiler. JSX remains
included for inspection, while the manifest entry and imports select emitted JS.
The stock shell uses this ordinary package host.

## Package surface actions

Ordinary components use `nickel.surfaces.show(id)`, `.hide(id)`, `.focus(id)`,
and `.setPlacement(id, {anchor, offsetX, offsetY})`. Nickel validates the ID
against the caller's declared surfaces and enforces passive-window and placement
rules. These operations do not target another package implicitly. The default
taskbar opens its own launcher/quick settings, and the launcher opens its own
Settings window and closes itself after invoking application launch.

## Package activation

The shared shell source catalog registers embedded `nickel-default` and activates
it as the stock shell through the normal package lifecycle. Activation opens only
its taskbar. Launcher and Settings declare `initially_open: false`, and transient
surfaces continue to require explicit show requests. Showing or closing windows
uses the same paths as an installed package. A package keeps its shared runtime
and Settings registrations when all windows close; explicit disable or a runtime
failure retires it. Global launcher, Settings, and Quick Settings intents target
these declared package surfaces. Ordinary taskbar rendering and input use the generic package
host; native display projection recovery remains trusted infrastructure.

The public `shell.settings.plugins` export registers an ordinary `Plugins` Settings
page when imported from the shell module graph. Import `./Plugins.js` alongside
other Settings modules. It reads `nickel.plugins.get()` / `list()` with
`plugins-read`, and requests `enable(id, revision)` / `disable(id, revision)` with
both `plugins-read` and `plugins-control`. Revisions are strings; requests are
checked against the current registry, prior enabled state, caller grants and
lock state before using the existing package activation/retirement lifecycle.

The inventory reports native runtime health, approved capabilities, declared
surfaces/composition and existing package memory counters. Missing counters are
`null`, not zero. Tracked peaks are estimates of tracked resources; per component
and total process memory are unavailable. Enabling an installed package remains
subject to its existing package validation and approval requirements.

## Active shell selection

`nickel.plugins.selectShell(id, revision)` selects a shell package from the public
plugin inventory. Shell inventory entries expose `shell` and `selected`; the
inventory also exposes `selectedShell`. Selection uses the ordinary package
activation lifecycle and persists alongside package activation settings.

Only the selected shell installs visible roots. Enabled base packages keep their
runtime and Settings registrations for dependency authority. Global launcher,
Quick Settings, and Settings requests use the selected package's declared
surfaces; an omitted surface is unavailable. Authorized surface actions from
inherited shell components address the selected package's corresponding surface.
