# Input platform acceptance matrix

Updated: 2026-08-30

This matrix tracks evidence for the input work described by private Specifications 0099–0105.
`pass` applies only to the evidence class in its column. A build or automated test is never native
interaction acceptance.

Status vocabulary: `pass`, `failed`, `unavailable`, `unsupported`, `untested`.

Current Linux evidence host: Ubuntu 26.04 LTS, Linux 7.0.0-30-generic, x86_64,
rustc/cargo 1.94.1. Commands are run from the workspace root unless a row says otherwise.

| OS | Runtime | Architecture | Layout | Outputs / scale | Device | Capability | Automated | Native build | Nested live | Installed live | Evidence date |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Linux | Smithay nested winit backend | x86_64 | host default | 1200x768, scale 120/120, Flipped180 | keyboard / pointer | focused keys, text, IME, pointer, wheel, focus reset | pass | pass | partial | not applicable | 2026-08-30 |
| Linux | Smithay nested winit backend | x86_64 | host default | 1200x768, scale 120/120, Flipped180 | keyboard | bare, Alt, and Alt+Shift Print Screen | pass | pass | pass | not applicable | 2026-08-30 |
| Linux | Smithay nested winit backend | x86_64 | host default | 1200x768 at 120/120 plus virtual 800x600 at 180/120 | keyboard | active-window capture on secondary mixed-scale output | pass | pass | pass (960x614 clipboard image) | not applicable | 2026-08-30 |
| Linux | Nickel Smithay session | x86_64 | host default | two outputs, scale values not yet recorded | keyboard / pointer / touch | compositor shortcuts, lock/focus transitions, screenshot actions | pass | pass | not applicable | untested | 2026-08-30 |
| Linux | SDL / Gilrs | x86_64 | not applicable | not applicable | Xbox-class controller | normalized navigation, hysteresis, repeat, disconnect | pass | pass | untested | untested | 2026-08-30 |
| Windows | SDL / Win32 | x86_64 | untested | untested | keyboard / pointer | focused input and registered global shortcuts | pass | pass (cross-build only) | not applicable | untested | 2026-08-30 |
| Windows | SDL / Win32 | x86_64 | untested | untested | Xbox-class controller | normalized navigation | pass | pass (cross-build only) | not applicable | untested | 2026-08-30 |
| Windows | Nickel shell | x86_64 | untested | untested | keyboard / pointer | Print Screen crop, clipboard, save, modified capture | pass | pass (cross-build only) | not applicable | untested | 2026-08-30 |
| macOS | native runtime | untested | untested | untested | all | focused/global/controller/screenshot acceptance | untested | untested | not applicable | unsupported | 2026-08-30 |
| BSD | native runtime | untested | untested | untested | all | focused/global/controller/screenshot acceptance | untested | untested | not applicable | unsupported | 2026-08-30 |

## Recorded automated and build evidence

- `cargo test -p nickel-input --all-targets --all-features`: pass; 29 deterministic vocabulary,
  shortcut, registration, SDL/winit key, text, IME, pointer, wheel, touch and focus-gain/loss
  equivalence, SDL/Gilrs controller equivalence, controller churn, and replay/property tests, plus
  two source-free replay-tool tests. Focus gain is preserved without resetting held state; focus
  loss remains the explicit reset edge.
- `cargo test -p nickel-input --no-default-features`: pass; 16 backend-neutral tests with no
  operating-system UI feature enabled.
- `cargo clippy -p nickel-input --all-targets --all-features -- -D warnings`: pass.
- Strict `-D warnings` clippy across the migrated core, UI, Settings, shell, File, gaze, Shapes, and
  session-protocol packages: pass. Session passes with only
  `clippy::items-after-test-module` allowed for a preserved, unrelated XWayland popup-placement edit;
  no input-epic lint is suppressed.
- `cargo test -p nickel-core`: pass; 86 unit and semantic scenario tests.
- `cargo test -p nickel-session --bins --tests`: pass; 89 tests including production Smithay input
  routing, authenticated screenshot-shell identity, semantic pointer press/release, and session
  protocol behavior.
- `cargo test -p nickel-session-protocol`: pass; 13 versioned wire-contract tests including the
  authenticated screenshot role and screenshot semantic-target round trips.
- Focused Nickel shell test: pass; screenshot selection start/end/confirm resolve through the live
  shell's production screenshot geometry and pointer hit-testing path.
- `cargo test -p nickel-file` and strict package clippy: pass.
- Nickel gaze-grid and shapes-test checks and strict package clippy: pass after focused input
  migration.
- `cargo test -p nickel-settings`: pass; 30 tests, including normalized keyboard and pointer input
  dispatched through production navigation geometry and reducers.
- `cargo test -p nickel-ui`: pass; 145 unit tests plus asset and documentation tests. Embedded
  application hosts share one normalized text, IME, shortcut, and clipboard-command path; focused
  tests verify commit and submit happen once and copy/cut/paste preserve their payloads.
- `cargo check -p nickel-shell --bin nickel --target x86_64-pc-windows-gnu`: pass. This is native
  build evidence only; it is not Windows interaction acceptance. The Windows adapter now waits for
  its native keyboard hook result before reporting `Available`; startup failure is explicit.
- `cargo build --release -p nickel-session --no-default-features --features backend-udev` and
  `cargo build --release -p nickel-shell --bin nickel`: pass. These are native optimized build
  results, not installed-session interaction acceptance.
- Explicitly test-controlled nested Smithay: bare Print Screen mapped the real screenshot surface and
  displayed compositor-captured pixels; Alt+Print Screen placed the focused Shapes window pixels on
  the clipboard; Alt+Shift+Print Screen produced a clipboard path whose PNG reopened successfully.
  A second 800x600 output at 1.5x scale also captured its focused window as a 960x614 clipboard
  image. The interactive screenshot surface was also compositor-centered at `0,4 1200x760`; its
  renderer-owned selection start/end, double-click confirmation, and cancel targets traversed the
  authenticated production pointer path and cancel unmapped the surface. The live checks retained
  dimensions and pass/fail only, and deleted the temporary file.
- The same nested production input path delivered a semantic pointer hover, a `120,-240` v120 wheel
  frame, and focused Escape to the Rust pointer probe. Its shared winit adapter observed normalized
  wheel `dx=-1 dy=2 discrete=Some((-1, 2))`; Escape closed the focused probe. No typed text was
  retained.
- In the nested Settings application, production pointer hit testing focused Search, normalized text
  input changed the visible result set, Escape cleared it, and a `120,-240` v120 wheel frame visibly
  scrolled the Appearance screen. In the fixture-backed embedded Codex UI, normalized text and
  Ctrl+A/C/X/V exercised the production composer selection and clipboard path. Moving focus from
  Codex to Settings while Control was held, releasing it there, and returning to Codex produced an
  ordinary `a`, proving focus-loss reset without retaining the entered text or clipboard payload.
- With Settings focused, bare Meta opened the real launcher; text went only to launcher Search.
  Escape first cleared its query and then dismissed it, after which text again reached the previously
  focused Settings field. A semantic click also activated the visible sliver of an overlapped Codex
  window rather than the center point occupied by Settings, exercising compositor hit testing.
- A semantic panel hover opened the real Settings window Preview; its production Menu target mapped
  ContextMenu at `52,564`, normalized Down traversed its keyboard path, and Escape closed both shell
  overlays and restored Settings as the active ordinary window.
- Immediately after a launcher map/unmap transition, the first compositor capture contained the
  complete 1200x768 scene rather than only damaged regions. Bare Print Screen then mapped Screenshot
  at `0,4 1200x760`, transferred compositor keyboard focus to it, and one focused Escape unmapped it
  and restored Settings as the active ordinary window. No screenshot was retained.

## Evidence still required

- Live Wayland IME preedit/commit. Persistent nested-Smithay focused text, ordinary clipboard
  shortcuts, Settings, fixture-backed embedded Codex UI, context-menu keyboard focus, launcher focus
  isolation/restoration, pointer/wheel normalization, focus-loss reset, and screenshot focus transfer
  now have live evidence. All three Print Screen bindings have live nested evidence; screenshot
  selection/confirmation/cancel also has complete nested semantic-path evidence.
- Installed Nickel multi-output and mixed-scale screenshot workflow, including XWayland focus.
- Live Windows focused input, registered shortcuts, controller navigation, and screenshot pixels,
  clipboard, save, and reopen workflow.
- Xbox-class controller navigation on Linux and Windows.
- Native macOS and BSD implementations and hosts; these remain unsupported rather than inferred
  from foreign builds.

Acceptance artifacts must not retain typed text, clipboard contents, credentials, or private
screenshots unless the user explicitly selects them.
