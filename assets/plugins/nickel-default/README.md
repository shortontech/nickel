# Nickel default shell package

This directory contains the default shell module graph described by Spec 0263.
`plugin.json` uses the current validated package schema, with composition metadata
and bounded grants for every surface. JSX imports refer to emitted JavaScript
paths so the developer compiler can stage the entire graph. Nickel activates this
package as its stock shell; optional windows share its runtime.

The package supplies Taskbar, Launcher, Quick Settings, Notifications, and the
optional Settings window and Volume OSD through ordinary JSX components and CSS. Its manifest
exports public replacement contracts and declares capabilities and native surfaces.
A selected derived shell can replace components or omit a surface entirely.

Settings consumes `readPluginSettings().settings` and
`readPluginSettingsPages().pages`. Qualified provider identities keep equal setting
IDs distinct. Native resources and operations remain capability checked; the
components own navigation, layout, drafts, and confirmation UI. Foreign custom
page components execute in their published owner's retained package context.
Contributor registrations survive hidden surfaces and are retired on disable or
runtime failure; activation generations prevent stale callbacks from regaining
authority after re-enable.

## Public contracts

- `Shell` / `shell`
- `Taskbar` / `shell.taskbar`
- `Launcher` / `shell.launcher`
- `QuickSettings` / `shell.quickSettings`
- `Notifications` / `shell.notifications`
- `VolumeOSD` / `shell.volumeOSD`
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
metadata, plus optional `description`, absolute `path`, and `lastUsedUnixSeconds`.
Legacy history has no timestamp until the next recorded launch. Launch and pin
actions use `id`; `icon` is a bounded stable asset key. Independent
`settingsResults` and `actionResults` provide native fuzzy matches for supported
shell destinations without changing application ranking or the application total.

The default Launcher presents separate Pinned and Recent sections before typing.
Search preserves the same 608×628 surface, with horizontal category filters,
full-width grouped results, a compact selected-result action row, and recent
shortcuts. Metadata opens through More rather than occupying a permanent pane.
Keyboard focus updates the selected result; result activation and Open launch it. The launcher owns its query, category, pagination, and menus;
it does not change the native launcher's query. Files currently exposes known
Places; it is not a filesystem index. Native project overview remains available
from Places. Unsupported elevation and reveal actions are not displayed.

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
passed the shared shell, Launcher, optional JSX Settings, hue/intensity/color
controls, native screenshot capture, component layout inspection, installed
window lifecycle, and clean shutdown. Runtime, presentation, and development-tool
tests cover package composition, contributor lifecycle, CSS controls, and package
validation. Manual native Windows UI acceptance remains with the user.

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

`Appearance.tsx` registers an ordinary Settings page in this package's module
graph. It reads configured preferences and resolved native hue/intensity, and
uses `nickel.appearance` / `nickel.wallpaper` for changes. Theme modes, preset
accent hues, custom hue entry, hue/intensity sliders, system accent inheritance,
transparency, animations, wallpaper positions/catalog selection/reset, and
appearance reset are implemented in JSX. The manifest requests the four domain
grants. No page-specific Rust host or action-index adapter was added.

Wallpaper labels/previews and image choosing use public native capabilities; the
chooser reports cancellation and unavailable platform support. File artwork uses
its ordinary registered Settings page.

`DefaultApps.tsx` registers the Applications page through the same module graph.
It consumes association targets/handlers and native revision strings, renders
search and current/protected/read-only state, and reports native consent and
write results. Writes invoke `nickel.associations.setDefault` with stable native
identities; no Rust Settings projection or indexed page request is involved.
Native parser validation and synthetic identity/consent/protection/search checks
passed. The ABI deliberately bounds each catalog to 128 targets and handlers and
reports truncation; users can open the operating system's default-app settings for
entries outside that bound. Paginated traversal of larger catalogs is not implemented.

`Displays.tsx` owns display selection, draft layout, drag arrangement/snapping,
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
tsc --allowJs --noResolve --checkJs false --jsx react --jsxFactory h --jsxFragmentFactory Fragment --target ES2022 --module ES2022 --outDir /tmp/nickel-default-js assets/plugins/nickel-default/src/*.tsx
for source in assets/plugins/nickel-default/src/*.tsx; do
  name=$(basename "$source" .tsx)
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

## Window operations and menu intent

`nickel.windows.showMenu(id)` and the native window-menu hotkey show the selected
shell's optional `window-menu` surface. `shell.windowMenu` is an ordinary
composition export. Omission makes the frontend unavailable. `windows.menu()`
reports the current target identity; `dismissMenu()` dismisses and restores native
application focus. `dismissMenu({restoreFocus:false})` clears the captured menu
intent and hides its surface without moving focus, suitable for `Window.onBlur`.
The option must be a boolean. These requests require `windows-context`; facts require
`windows-read`.

Public operations include `minimize`, `maximize`, `restore`, `toggleMaximize`,
`toggleFullscreen`, `snapLeading`, `snapTrailing`, `moveToWorkspace`, and
`moveToOutput`, alongside `activate` and
`close`. Native code checks current window capabilities and state before issuing
commands. Movement destinations come from `windows.destinations()` and are
validated against the current native workspace/output inventory. `maximize` is idempotent; `restore` restores a minimized window first,
then fullscreen or maximized state on subsequent requests. Arbitrary fullscreen
and snapping are unavailable in the Windows adapter and are reported as such.

Window previews are the ordinary `shell.window-preview` component on the optional
`window-preview` overlay. `nickel.windowPreviews.get()` exposes at most twelve
hover candidates or five task-switch candidates, bounded native titles, native
capabilities, selection, opaque image keys, and a revision. Activation, close,
and menu requests carry the native window ID and that revision. The host checks
read/action grants, lock state, package lifetime, and current native candidates.

The native preview window role remains a placement and thumbnail bridge: Windows
attaches native thumbnails to that window and Linux routes native preview focus,
highlight, and task-switch placement through it. Its content uses the ordinary
shared package runtime. The same declaration is excluded from generic panel
presentation, so only the preview bridge presents its content.

## Guarded shell selection

The ordinary Plugins page uses `nickel.plugins.get()` for inventory, grants,
operation results, and the current shell preview. **Preview shell** starts a
bounded preview with `nickel.plugins.selectShell(id, revision)`. Disabled packages
first show their requested capabilities, surfaces, and composition changes; the
review action is **Enable and preview shell**.

While a preview is active, the page shows the candidate and previous shell,
**Keep this shell**, and **Restore previous shell**, even when the inventory search
hides the candidate. Activation and additional selection controls stay disabled
until the preview finishes. Confirmation is presented from native snapshot state,
so it does not depend on local review state surviving a shell change.

`plugins.get().shellPreview` is either `null` or
`{token, previousShell, selectedShell, deadlineUnixMilliseconds, canConfirm, canRevert}`.
`plugins.confirmShell(token, revision)` and `plugins.revertShell(token, revision)`
carry the opaque preview token and current inventory revision. The page respects
the native operation flags; native code checks grants, package lifetime, lock
state, token, and revision again. Only confirmation saves the shell selection.
Expired, failed, or reverted previews restore the previous shell.

The native recovery timer and trusted recovery controls remain usable when the
candidate omits Settings or its package fails. The ordinary Plugins page displays
confirmation whenever it is available in the selected shell; it does not own the
recovery timer or provide a Rust page host. This selector manages already known
packages and does not install packages.

The launcher uses an anchored overlay with a 120×36 All apps button and an
alphabetical app list. Default-shell `nickel-plugin dev` previews copy saved
launcher pins and recents into the temporary profile at startup. Changes made
inside the preview remain in that temporary profile.
