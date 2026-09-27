# Nickel Settings plugin

`plugin.json` declares the first-party Settings window and service grants. The
separate `nickel-settings` process validates this manifest and loads these
embedded JavaScript sources through the shared JSX runtime. Permission review
and native recovery controls remain owned by Rust. Settings persists the bundled
package's enabled state in Nickel's activation settings. Disabling it retires
its JSX contexts and leaves the native Plugins recovery view available to
review and re-enable it. The shell activation registry does not yet manage the
separate Settings process; publishing its process-owned memory account to the
shell is a remaining migration step.
The Plugins page reports a measured lower bound for the Settings package's
retained Rust component trees and cached page projections. Boa heap, textures,
the shared native Settings frame, and process RSS remain unattributed.

`settings-pages.jsx` starts the bundled Settings process view migration. It
currently renders the Keyboard Shortcuts and About cards through the shared
runtime and a Settings-specific native adapter. Regenerate its shipped JS with:

```sh
tsc --allowJs --checkJs false --noCheck --jsx react --jsxFactory h \
  --target ES2020 --outDir assets/plugins/settings \
  assets/plugins/settings/settings-pages.jsx
```

`settings-plugins.jsx` renders the ordinary Plugins list, live memory labels,
enable and disable controls, and registered plugin settings. Its callbacks send
typed requests that the Settings host validates against the current status.
The permission review and emergency disable view remain native Rust. Rebuild
its shipped JS with the same command, replacing `settings-pages.jsx` with
`settings-plugins.jsx`.

Settings starts each page Boa context when its page is first opened and retires
it after navigation to another page. Building the navigation destinations for
other pages does not allocate those contexts. The navigation context stays
alive while Settings is open.

`settings-navigation.jsx` declares destination order, grouping, labels, and
headers. The native `ResponsiveNavigation` adapter owns focus, responsive
layout, and the trusted page slots while the remaining page views migrate.
Rebuild it with the same `tsc` command and its source filename.

`settings-bar.jsx` owns the ordinary Bar controls and workspace preview. Its
radio and slider callbacks request typed changes that the Settings host checks
against the current topology projection before using the existing reducers.
The native Bar view remains available if JSX fails. It uses the same build
command with `settings-bar.jsx` as the source.

`settings-optional-features.jsx` owns the ordinary Codex and on-screen keyboard
cards. Its callbacks request typed changes, retry, and disable confirmation;
the Settings host checks current policy, runtime state, and environment
overrides before applying them. Its Boa context starts only while this page is
open, and the native view remains available if JSX fails. Build it with the
same command and its source filename.

`settings-network.jsx` owns the Network page's Wi-Fi switch, discovered network
list, and adapter summary. Requests carry the observed network index and
profile; the host checks the current projection before invoking the existing
platform path. The native Network view remains available if JSX fails. Build
it with the same command and its source filename.

`settings-bluetooth.jsx` owns the Bluetooth and pairing page layout. Its
requests carry the observed device index and ID; the host checks the current
adapter state and device identity before using the existing Bluetooth handlers.
The native Bluetooth view remains available if JSX fails. Build it with the
same command and its source filename.

`settings-appearance.jsx` owns the Appearance page's card order and ordinary
controls: mode, accent, wallpaper, Interface, and Reset. Its custom hue dialog
also comes from JSX. The Settings host supplies the wallpaper image and native
preview, color, input, slider, select, switch, and popover widgets. It validates
typed requests before opening the file picker or saving settings. Invalid hue
input leaves the dialog open for correction. Native controls remain available
if JSX fails. Build it with the same command and its source filename.

`settings-default-apps.jsx` renders the curated association rows, catalog
search, family filters, and visible catalog rows. Rust owns the virtual list's
scroll geometry and projects only its current bounded window into JavaScript.
Typed chooser requests carry the projected target identity and are checked
before Rust opens the host-owned handler picker. Build it with the same command
and its source filename.

`settings-default-app-picker.jsx` owns the open handler picker's search and
visible candidate rows. Rust owns its popover placement, focus return, virtual
scroll range, association capability, and operating-system consent path. The
host checks the row, target, candidate, and current capability before applying
a JSX selection. The native picker remains available if this component fails.
Build it with the same command and its source filename.
