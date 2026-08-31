# Nickel File semantic scenario evidence

The production `FileApp` is exercised through `nickel_ui_testkit::Scenario`, which owns a real
`UiHost`; the scenarios do not call the application reducer directly or use copied hit geometry.

Covered states and transitions:

- populated directory with thirty stable file targets;
- empty-directory presentation;
- failed initial path with the production fallback directory and visible error status;
- accessibility, keyboard, and controller context-menu routes;
- overlay Cancel dismissal and focus return;
- production pointer scrolling;
- narrow and wide resize reconstruction, including 200% scale;
- focus loss, controller removal, suspension, and released input ownership;
- zero retained build scratch before and after lifecycle transitions.

The scenario trace is asserted for keyboard, controller, and accessibility operations. Existing
focused raster tests separately prove ordinary controller focus and menu-row focus movement.

Commands:

```text
cargo test -p nickel-file
cargo clippy -p nickel-file --all-targets --all-features -- -D warnings
```

Live nested compositor acceptance remains distinct from these deterministic host scenarios.
