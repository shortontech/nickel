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
text/color/number values. It falls back to text for shortcuts, grouped values,
and repeated values. A color text field does not fulfill Spec 0263's appearance
requirements: accent swatches, custom hue/color selection, interface hue,
intensity, modes, transparency, and the remaining appearance controls must move
from the existing page to public capabilities before activation. Unresolved
custom component references are shown as unavailable rather than treated as
JavaScript functions. No private Settings projection or host was introduced.

Static checks verify manifest JSON, module/CSS targets, unique root surface IDs,
and absence of old CSS template tokens. No runtime or visual validation is
claimed: the draft manifest, visibility props, per-component snapshots, legacy
request dispatch, public composition resolution, and Settings bridge still need
integration. In particular, a single undifferentiated `nickel.data` cannot serve
all copied components; those reads must become capability snapshots or explicit
component inputs during cutover.

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
