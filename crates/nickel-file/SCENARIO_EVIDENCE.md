# Nickel File semantic scenario evidence

The production `FileApp` is exercised through `nickel_ui_testkit::Scenario`, which owns a real
`UiHost`; the scenarios do not call the application reducer directly or use copied hit geometry.

Covered states and transitions:

- populated grid and Details directories with stable file targets;
- empty, unavailable, unreadable, loading, and disconnected presentations;
- failed initial path with the production fallback directory and a tab-owned visible error status;
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

Hidden headless inspection of the current File Pilot-inspired composition covers the wide Details,
minimum Details, and narrow grid variants. It confirms that the sidebar collapses to the Places
control, toolbar actions remain contained, Details columns fit the minimum workspace, and grid rows
retain their content-sized height rather than stretching into unused canvas. The grid label layout
reserves multiple lines independently of row growth, so long-name wrapping does not regress the
dense spacing.

Commands:

```text
cargo test -p nickel-file
cargo clippy -p nickel-file --all-targets --all-features -- -D warnings
cargo run -p nickel-ui-workbench -- headless render-variant nickel-file wide-details-light /tmp/nickel-file-wide-details-light.png
cargo run -p nickel-ui-workbench -- headless render-variant nickel-file minimum-details-light /tmp/nickel-file-minimum-details-light.png
cargo run -p nickel-ui-workbench -- headless render-variant nickel-file narrow-grid-dark /tmp/nickel-file-narrow-grid-dark.png
```

Live nested compositor acceptance remains distinct from these deterministic host scenarios. The
available `nickel-session` backends require winit or udev and do not provide an invisible headless
compositor, so native Windows and Linux acceptance is still outstanding.
