# Post-audit input regression notes

Reported during live use of the release build of `6c16784` on 2026-09-07.
These reports remain open independently of the requested archival of specs 0209–0219.
No fix or causal link to the audit changes is established.

## Preview-induced whole-screen stalls — open, specified

The user reports long whole-screen slowdown, nearly crashing Nickel, when previews appear. Native
output rendering captures thumbnails before presenting and synchronously waits for GPU completion,
then maps/copies pixels back to the CPU. Animated commits can repeatedly invalidate previews;
persistent failures can schedule retries every 16 ms. These are confirmed responsiveness hazards,
not a measured attribution of the reported stall to a particular driver or monitor.
See [spec 0225](../specs/0225-nonblocking-native-window-previews.md) for nonblocking/fallback behavior,
bounded work, backoff, diagnostics, and acceptance. The exact triggering preview UI remains to be
confirmed. No live stress reproduction or implementation change was performed.

## Media/volume hotkeys do nothing — open, specified

The user reports no volume change or OSD. Native consumer-control notification only sends to old
external-shell subscribers; it lacks the in-process dispatch used for other global shortcuts. Native
audio status also bypasses the event that arms the OSD. See
[spec 0221](../specs/0221-native-media-key-dispatch-and-volume-osd.md). These are source-confirmed
gaps; hardware event arrival and backend behavior still require an instrumented acceptance pass.

## Missing desktop icons — open, specified

The user also reported missing desktop icons. Source inspection found missing topology/viewport
initialization and passive desktop input routing in the native shell coordinator. The topology gap
predates audit baseline `077183c`. See active
[spec 0220](../specs/0220-native-desktop-icon-topology-and-input.md) for the causal chain, repair scope,
and required native rendering/input regression coverage. No implementation fix has been applied.

## On-screen keyboard unavailable — open

The user reports that the on-screen keyboard does not exist/is unavailable in the current session.
The feature has an existing implementation and [documented controls](on-screen-keyboard.md), so
record this as a reported availability regression, not proof that the implementation was removed.
Follow-up inspection confirmed saved enablement at generation 1 and no startup override in the
running compositor's environment. Its client socket/token environment is absent, while the keyboard
refresh/configure/input path still uses synchronous external session requests instead of the typed
in-process authority. Refresh therefore fails before resolving enablement. A separate native input
arm supplies settings generation instead of recipient epoch. See
[spec 0222](../specs/0222-native-on-screen-keyboard-authority.md) for the full source evidence and
recipient-lease requirements. Exact live mapping/input behavior awaits implementation and testing.

Follow-up: inspect effective enablement and the Settings/tray entry points, then determine whether
invocation fails to map the keyboard or the invocation control itself is absent. Verify focus stays
with the recipient when showing, typing with, and hiding the keyboard. If a regression is reproduced,
add coverage through production semantic input and effects before implementing a fix.

## Transient repeated `@` terminal input — open, not reproduced

- Context: reconnecting a DisplayLink second monitor, then Alt-Tabbing to a terminal.
- Report: each typed key appeared to send `@`; the submitted text contained repeated `@` characters
  interspersed with letters and mention-like text.
- Testing `Alt+s` did not reproduce it. The user subsequently confirmed normal typing worked again.
- Modifier state, focus transitions, terminal/application key handling, and output hotplug are
  investigation paths, not established causes. No keyboard event trace was captured.

Follow-up: identify the terminal and foreground application, then reproduce reconnect → Alt-Tab →
plain typing in a disposable prompt without executing it. Compare plain `s`, `Alt+s`, and plain `s`
again, recording press/release and focus transitions if it recurs. Do not treat arbitrary terminal
input as commands or restart the display manager as a routine diagnostic step.

Acceptance: plain typing remains correct after repeated hotplug/focus cycles, modifiers release
correctly, and any reproduced regression has a focused production-path test. Until then, keep this
report open rather than diagnosing a stuck modifier from the symptom alone.

## Memory follow-up proposals

- [0223 — bounded system-status delivery](../specs/0223-bounded-native-system-status-delivery.md):
  replace accumulated audio snapshot histories/relay queues with bounded latest-state delivery,
  preserving ordered commands and race-free wakes. Burst retention is unbounded in the inspected
  path; no process-level leak or current MiB saving has been measured.
- [0224 — move owned preview pixels](../specs/0224-move-owned-preview-pixels-without-recloning.md):
  remove the clone-only normalization step from preview refresh without adding another cache.
  This targets temporary allocations/copies, not proven idle resident-memory savings.

All new specifications are active. These investigations made no runtime or implementation changes.
