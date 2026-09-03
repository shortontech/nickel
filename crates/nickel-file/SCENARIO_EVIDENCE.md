# Nickel File semantic scenario evidence

The production `FileApp` is exercised through `nickel_ui_testkit::Scenario`, which owns a real
`UiHost`; the scenarios do not call the application reducer directly or use copied hit geometry.

Covered states and transitions:

- populated grid and Details directories with stable file targets;
- empty, unavailable, unreadable, loading, and disconnected presentations;
- asynchronous initial loading and failed initial paths that retain the requested tab identity with
  a visible recoverable error;
- accessibility, keyboard, and controller context-menu routes;
- overlay Cancel dismissal and focus return;
- production pointer scrolling;
- narrow, minimum, medium, and wide responsive compositions at 100%, 125%, and 200% scale;
- light and dark appearance, long and multiline names, Unicode, RTL, hidden files, and
  multiselection;
- focus loss, controller removal, suspension, and released input ownership;
- zero retained build scratch before and after lifecycle transitions.

The scenario trace is asserted for keyboard, controller, and accessibility operations. Existing
focused raster tests separately prove ordinary controller focus and menu-row focus movement. The
fixture matrix also renders every declared case to a real raster, then checks finite geometry,
clipping, overlap, actionable bounds, accessible names, and minimum text contrast.

Hidden headless inspection of the current File Pilot-inspired composition covers wide Details,
minimum Details, narrow grid, minimum command, loading, and unreadable variants. It confirms that
the sidebar collapses to the Places control, toolbar actions remain contained, Details columns fit
the minimum workspace, exceptional states replace stale entry content, and grid rows retain their
content-sized height rather than stretching into unused canvas. The grid label layout reserves
multiple lines independently of row growth, so long-name wrapping does not regress the dense
spacing. The minimum command surface keeps its search field fixed and makes all results reachable
through a virtualized scroll region.

Provider-policy evidence additionally covers the user-visible Settings selector, retention and
labeling of an unavailable configured Linux theme, rejection of missing or non-local theme names,
Nickel fallback reasons, and physical-size rasterization of scalable theme artwork. An open Nickel
File checks external settings while idle so provider and theme changes refresh without unrelated
input.

Commands:

```text
cargo test -p nickel-file
cargo clippy -p nickel-file --all-targets --all-features -- -D warnings
cargo check -p nickel-file --target x86_64-pc-windows-gnu
cargo test -p nickel-settings unavailable_named_file_icon_theme_remains_visible_and_accessible
cargo test -p nickel-platform unavailable_or_non_local_theme_names_return_no_artwork
cargo test -p nickel-platform scalable_theme_artwork_rasterizes_at_the_requested_physical_size
cargo build -p nickel-file --target x86_64-pc-windows-gnu
cargo run -p nickel-ui-workbench --features file-provider -- headless render-variant file.browser wide-details-light /tmp/nickel-file-wide-details-light.png
cargo run -p nickel-ui-workbench --features file-provider -- headless render-variant file.browser minimum-details-light /tmp/nickel-file-minimum-details-light.png
cargo run -p nickel-ui-workbench --features file-provider -- headless render-variant file.browser narrow-grid-dark /tmp/nickel-file-narrow-grid-dark.png
cargo run -p nickel-ui-workbench --features file-provider -- headless render-variant file.browser minimum-command-surface /tmp/nickel-file-minimum-command.png
cargo run -p nickel-ui-workbench --features file-provider -- headless render-variant file.browser loading /tmp/nickel-file-loading.png
cargo run -p nickel-ui-workbench --features file-provider -- headless render-variant file.browser unreadable /tmp/nickel-file-unreadable.png
```

Live nested-compositor acceptance was run in the Smithay winit backend at 1200x768. Renderer-owned
screenshots confirmed native Linux browsing with the default Nickel provider, the explicitly
selected installed `breeze` System theme, and a deliberately missing System theme falling back to
Nickel artwork. The live populated grid retained four dense columns, and the
`bitcards-integrations` label wrapped to two lines without changing row growth or clipping. The
compositor reported each instance as a mapped, shown window and captured its final frame through
`nickel-screenshot`.

The Windows GNU build was linked as an x86-64 PE executable and run through Steam Proton
Experimental inside that same nested compositor. It opened `C:\users\steamuser`, reached its event
loop, browsed 15 entries, and rendered both the Windows System-provider path and the explicitly
selected Nickel provider end to end. This is useful cross-platform runtime evidence, but Proton's
Wine shell is not evidence of native Windows Explorer icon fidelity.

Native Windows acceptance was subsequently completed on SteamMachine from commit `b6f8683` using
the MSVC toolchain. All 76 `nickel-file` tests passed, as did focused shell-settings and Windows
platform-adapter tests. Captures from the rebuilt application visibly distinguished the default
System provider's Explorer-backed yellow and special-folder artwork (including OneDrive) from the
explicit Nickel provider's repository-owned glossy blue artwork and distinct Pictures and Music
variants. The run used an isolated `LOCALAPPDATA`, leaving persistent user settings unchanged.

An acceptance-only input driver also exercised Ctrl+L path entry, pointer selection, Enter to browse
into a child folder, and Backspace to return. The final content, breadcrumb, and status bar all
showed the returned parent directory. A stale title after Backspace was recorded as a separate
native title-refresh defect; it did not affect browsing state. The audit also observed a separate,
reproducible long-text overflow defect for later UI work.

A mount-isolated Linux acceptance process replaced `/usr/share/icons`,
`/usr/local/share/icons`, `$HOME/.icons`, and `$HOME/.local/share/icons` with empty filesystems; an
independent inspection reported zero entries in every root. Nickel File still opened and displayed
complete Nickel artwork for ordinary and special folders. This exercises the clean-installation
condition without uninstalling packages from the development host.

For Specs 0123 and 0124, the native Windows run supersedes Proton as the Windows acceptance
evidence. Together with the native Linux, isolated clean-Linux, deterministic fixture, and automated
scenario results above, the agreed verification matrix is complete and both specifications are
archived.
