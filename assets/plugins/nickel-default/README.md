# Nickel default shell package draft

This directory is the reviewable package layout described by Spec 0263. It is
intentionally not activated by the current plugin loader. `nickel.json` uses the
planned package schema and points at one ES module graph rooted at
`src/Shell.jsx`; adding a current `plugin.json` would incorrectly opt the draft
into the one-script-per-surface loader.

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
4. The core Settings registry exists. JavaScript registration, reactive reads,
   live value/change bindings, and cross-provider component resolution remain
   unconnected to this package.
5. Unified capability clients behind the dotted grants in `nickel.json`.
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
