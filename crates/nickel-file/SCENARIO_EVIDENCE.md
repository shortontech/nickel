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
cargo run -p nickel-ui-workbench --features file-provider -- headless render-variant file.browser wide-details-light /tmp/nickel-file-wide-details-light.png
cargo run -p nickel-ui-workbench --features file-provider -- headless render-variant file.browser minimum-details-light /tmp/nickel-file-minimum-details-light.png
cargo run -p nickel-ui-workbench --features file-provider -- headless render-variant file.browser narrow-grid-dark /tmp/nickel-file-narrow-grid-dark.png
cargo run -p nickel-ui-workbench --features file-provider -- headless render-variant file.browser minimum-command-surface /tmp/nickel-file-minimum-command.png
cargo run -p nickel-ui-workbench --features file-provider -- headless render-variant file.browser loading /tmp/nickel-file-loading.png
cargo run -p nickel-ui-workbench --features file-provider -- headless render-variant file.browser unreadable /tmp/nickel-file-unreadable.png
```

Live nested compositor acceptance remains distinct from these deterministic host scenarios. The
available `nickel-session` backends require winit or udev and do not provide an invisible headless
compositor, so native Windows and Linux acceptance is still outstanding. In particular, completion
still requires visible browsing checks with Windows System and Nickel artwork, Linux Nickel and an
installed System theme, unavailable-theme fallback, and a clean Linux environment without desktop
icon packages. Specs 0123 and 0124 must remain active until that evidence exists.
