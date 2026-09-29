# Nickel Settings plugin

`plugin.json` declares the first-party Settings window and service grants. The
separate `nickel-settings` process validates this manifest and loads these
embedded JavaScript sources through the shared JSX runtime. Permission review
and native recovery controls remain owned by Rust. Settings persists the bundled
package's enabled state through the shell activation registry. Disabling it
retires its JSX contexts and leaves the native Plugins recovery view available
to review and re-enable it. The separate Settings process follows the shell's
desired state through its periodic memory report response.
The manifest lists the page services it reads or changes, including application
associations, network, Bluetooth, and display controls. Rust validates each
typed action against current state before using those services.
The shell reports this lazy plugin as Idle while Settings is closed, Running
while its process reports fresh memory, and Disabled when turned off.
The Plugins page reports a measured lower bound for the Settings package's
retained Rust component trees and cached page projections, and the separate
process publishes that lower bound to the shell while running. Boa heap, textures,
the shared native Settings frame, and process RSS remain unattributed.

`settings-shell.jsx` owns the ordinary Settings `<Window>`, sidebar, search,
page header, and responsive navigation. Its `<Slot id="settings-content" />`
places the active page's native component tree inside the JSX layout. The
controller path still uses native `ResponsiveNavigation` while controller
presentation is a separate follow-up. Build its shipped JavaScript with the
same `tsc` command below, using `settings-shell.jsx` as the source.

`settings-pages.jsx` renders the Keyboard Shortcuts and About cards through the
same bounded component tree and native renderer as shell plugins. Their CSS is
in `settings-pages.css`; Settings fills its theme color and spacing tokens when
the appearance changes. Regenerate its shipped JS with:

```sh
tsc --allowJs --checkJs false --noCheck --jsx react --jsxFactory h \
  --target ES2020 --outDir assets/plugins/settings \
  assets/plugins/settings/settings-pages.jsx
```

`settings-plugins.jsx` and `settings-plugins.css` render the ordinary Plugins
list, live memory labels, enable and disable controls, and registered plugin
settings through shared controls. Its callbacks send
typed requests that the Settings host validates against the current status.
The permission review and emergency disable view remain native Rust. Rebuild
its shipped JS with the same command, replacing `settings-pages.jsx` with
`settings-plugins.jsx`.

Settings starts each page Boa context when its page is first opened and retires
it after navigation to another page. Building the navigation destinations for
other pages does not allocate those contexts. The navigation context stays
alive while Settings is open.

`settings-navigation.jsx` still declares destination order, grouping, labels,
headers, and searchable controls. The host accepts only known focus targets.
The JSX shell uses those declarations for its sidebar while the native
`ResponsiveNavigation` path remains for controller presentation.
Rebuild it with the same `tsc` command and its source filename.

`settings-bar.jsx` uses the shared `div`, `Text`, `Button`, and `Slider` components
with `settings-bar.css`. Its callbacks request typed changes that the Settings
host checks against the current topology projection before using the existing
reducers. The native Bar view remains available if JSX fails. Regenerate its
shipped JS with the same build command, using `settings-bar.jsx` as the source.

`settings-optional-features.jsx` uses shared components and
`settings-optional-features.css` for the Codex and on-screen keyboard cards.
The generic `<Switch>` preserves native switch semantics for on, off, mixed,
and unavailable states. Its callbacks request typed changes, retry, and disable
confirmation; the Settings host checks current policy, runtime state, and
environment overrides before applying them. Its Boa context starts only while
this page is open, and the native view remains available if JSX fails. Build
its shipped JS with the same command and its source filename.

`settings-network.jsx` uses shared components and `settings-network.css` for
the Network page's Wi-Fi switch, discovered network list, and adapter summary.
Requests carry the observed network index and
profile; the host checks the current projection before invoking the existing
platform path. The native Network view remains available if JSX fails. Build
it with the same command and its source filename.

`settings-bluetooth.jsx` uses shared controls and `settings-bluetooth.css` for
the Bluetooth and pairing page layout. Its
requests carry the observed device index and ID; the host checks the current
adapter state and device identity before using the existing Bluetooth handlers.
The native Bluetooth view remains available if JSX fails. Build it with the
same command and its source filename.

`settings-appearance.jsx` and `settings-appearance.css` define the Appearance
page through shared controls: mode, accent, wallpaper, Interface, Reset, and
the custom hue dialog. The Settings host supplies the wallpaper image and
popover placement and validates typed requests before opening the file picker
or saving settings. Invalid hue input leaves the dialog open for correction.
Build it with the same command and its source filename.

`settings-default-apps.jsx` and `settings-default-apps.css` render the curated
association rows, catalog search, family filters, and visible catalog rows
through shared controls. Rust owns the virtual list's
scroll geometry and projects only its current bounded window into JavaScript.
Typed chooser requests carry the projected target identity and are checked
before Rust opens the host-owned handler picker. Build it with the same command
and its source filename.

`settings-default-app-picker.jsx` and `settings-default-app-picker.css` render
the open handler picker's search and visible candidate rows through shared
controls. Its actions carry a source scope so they cannot be confused with
the parent Default Apps page's actions. Rust owns popover placement, focus return, virtual
scroll range, association capability, and operating-system consent path. The
host checks the row, target, candidate, and current capability before applying
a JSX selection. The native picker remains available if this component fails.
Build it with the same command and its source filename.

`settings-display.jsx` and `settings-display.css` render Display's enabled,
resolution, refresh, scale, action, and confirmation controls through the shared
component renderer. The host retains arrangement geometry and dragging, the
application scale radio group, output validation, and timed revert. Display
events use a source scope so they do not collide with other Settings pages.
Build its shipped JavaScript with the same command and `settings-display.jsx`.
