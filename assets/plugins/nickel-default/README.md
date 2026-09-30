# Nickel default shell package draft

This directory contains the default shell module graph described by Spec 0263.
`plugin.json` uses the current validated package schema, with composition metadata
and bounded grants for every surface. JSX imports refer to emitted JavaScript
paths so the developer compiler can stage the entire graph. It remains a draft:
stock activation awaits capability snapshots and complete Settings controls.

The taskbar, launcher, quick settings, notification JSX, and their CSS are
source-preserving copies of the shipped first-party plugins. The modules add CSS imports, named component exports, and unique window IDs.
Settings uses semantic CSS variables instead of the old Rust-substituted style
placeholders. Generic CSS selectors are
scoped to their component window so the former per-package styles do not bleed
across the shared graph. Their behavior, IDs, requests, and visuals remain
available for comparison during the host cutover.

Settings is intentionally registry-driven. It consumes the public registry snapshot shape through the intended
`readPluginSettings().settings` and `readPluginSettingsPages().pages` bridges.
Navigation identities include the provider package so equal registration IDs
from different providers remain distinct. The bridge must attach authorized
value/onChange functions and resolve page component references to ordinary
components; serializable registry metadata alone cannot supply either.
It does not copy the current Settings page projections, action-index routing,
request enums, `<Slot id="settings-content">`, or separate host lifecycle.

## Public contracts

- `Shell` / `shell`
- `Taskbar` / `shell.taskbar`
- `Launcher` / `shell.launcher`
- `QuickSettings` / `shell.quickSettings`
- `Notifications` / `shell.notifications`
- `Settings` / `shell.settings`
- `SettingsNavigation` / `shell.settings.navigation`
- `SettingControl` / `shell.settings.controls`

The package advertises `taskbar.items`, `launcher.providers`, `notifications`,
and `settings.pages` as the semantic contribution mount points owned by these
components. The actual contribution renderer remains a runtime dependency; the
copied components still read today's bounded slot data until that API lands.

## Runtime dependencies before activation

1. The shared module graph host now accepts relative JSX and CSS imports and
   local named/default exports. This entry avoids re-export-from syntax.
2. Multiple surface roots and package-owned surface visibility state.
3. Resolution of public component exports, replacements, and contributions.
4. Public Settings registration and cross-provider callback dispatch are wired.
   Live foreign value reads and cross-provider custom page components remain.
5. Unified capability clients behind the grants in `plugin.json`.
6. Migration of copied legacy requests and `nickel.data` snapshots to those
   clients without changing their public behavior.

The existing shipped packages remain unchanged and active while these runtime
pieces are implemented.

## Settings coverage and validation

The draft presents switches, sliders, select options, actions, and editable
text/number values and grouped controls. Color settings use the replaceable
`ColorPicker` with preset swatches, hue, saturation, brightness, optional alpha,
and CSS color text entry. It falls back to text for shortcuts and repeated values.
This generic picker does not complete Spec 0263's appearance migration:
interface hue, intensity, modes, transparency, and remaining appearance controls must move
from the existing page to public capabilities before activation. Unresolved
custom component references are shown as unavailable rather than treated as
JavaScript functions. No private Settings projection or host was introduced.

Static checks verify manifest JSON, module/CSS targets, unique root surface IDs,
and absence of old CSS template tokens. A focused native host test loads this
package from the embedded catalog, publishes Settings pages, and creates every
declared surface with one shared `JsxRuntime`. Additional surface construction
does not initialize registration modules again. Live visual validation remains.
Stock activation, native visibility updates, complete capability snapshots,
remaining legacy request dispatch, and inherited shell composition still need
integration before the package replaces the stock shell.

## Public component lookup

The package host publishes `composition.exports` from the validated manifest.
`nickel.component("shell.taskbar")` returns that module's actual component
function. Exported modules share the entry's module cache, hooks, and CSS, even
when the entry does not import them directly. Missing modules or exports and
non-function exports reject package startup. The default `Shell` resolves its
surface components through these contracts.

This lookup currently binds the package's own exports. Applying an inherited
package's replacements and contributions still requires the composition host;
lookup alone does not activate third-party shell replacement.

## Appearance capability migration

`Appearance.jsx` registers an ordinary Settings page in this package's module
graph. It reads configured preferences and resolved native hue/intensity, and
uses `nickel.appearance` / `nickel.wallpaper` for changes. Theme modes, preset
accent hues, custom hue entry, hue/intensity sliders, system accent inheritance,
transparency, animations, wallpaper positions/catalog selection/reset, and
appearance reset are implemented in JSX. The manifest requests the four domain
grants. No page-specific Rust host or action-index adapter was added.

Wallpaper image previews/names, arbitrary-image choosing, file artwork settings,
and stock host activation still need migration. Native parser validation and
synthetic public-client behavior checks passed; live visual acceptance remains.

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
an explicit native operation-availability flag; application-scale policy and
identify-display operations remain unfinished. No old Display page adapter is
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
This prepares the shared host; it does not change stock shell activation.
