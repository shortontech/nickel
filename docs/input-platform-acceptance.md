# Input and window-operation platform acceptance

Updated: 2026-09-14

This matrix records evidence reproducible from the current integrated branch. Automated tests prove
reducer and adapter contracts; they do not prove native interaction. This pass did not launch a
compositor, inject live input, restart an installed session, or use a Windows host.

Vocabulary: `pass` (the named command passed), `failed`, `unavailable` (required host/tool absent),
`unsupported`, and `untested`. A native row never inherits `pass` from a unit test, cross-build,
nested fixture, historical observation, or the presence of test source.

## Current matrix

| Platform/runtime | Capability | Automated contract | Build this pass | Native interaction | Evidence gap |
| --- | --- | --- | --- | --- | --- |
| Platform-neutral core | operation identity, admission, completion, handoff, cancellation, tails | pass | pass as dependency | not applicable | none for enumerated transitions |
| Platform-neutral core | revisioned geometry, compensation, late/superseded fencing | pass | pass as dependency | not applicable | native settlement is platform-specific |
| Platform-neutral UI | active-widget identity, shared pointer/keyboard/controller activation prefix, handled/fallback disposition | pass: 406 passed, 2 ignored | pass | not applicable | none for enumerated headless transitions |
| Linux Smithay | physical keyboard/pointer/touch normalization | pass in serialized session and focused library suites | pass | untested | real devices, focus/grab and lock/suspend teardown |
| Linux Smithay | completed-frame touch cancellation | focused vendor test pass | vendor test build pass | untested | real down/frame/cancel and absence of later motion/up |
| Linux Smithay | XDG move/resize, titlebar, Super+pointer, internal move | focused move tests pass | test build pass | untested | nested/installed grab, cursor and configure behavior |
| Linux XWayland | move/resize, unknown-causality settlement and conditional compensation | source tests present, not rerun | test build pass | untested | real request/configure ordering, deadline, focus and teardown |
| Linux Gilrs | identity, navigation, repeat, disconnect, focus fence | source tests present, not rerun | compiled as dependency | untested | physical controller not used |
| Unix controller broker/transport | live route/surface plus connection/lease/stream generation, transfer/revoke/reset | pass in protocol and session suites | pass | untested | live transfer and neutral barrier |
| Windows focused/global input | winit, hook suppression, typed shortcuts | source tests present, not rerun | unavailable on this Linux pass | untested | Windows host, layouts, IME, hook registration/suppression |
| Windows foreign move/resize | bound source/button, contested control, release/reconciliation/takeover | source-reviewed fail-closed gates present; Windows-only tests not run | unavailable | untested | bounded identity-bearing native completion mechanism, hooks, DPI, monitors, takeover |
| Windows controller pipe | nonblocking client/server adapters, bounded correlated delivery and generation fencing | source tests present; production server integration absent | unavailable | untested | production `WindowsPipeServer` construction, live partial I/O, replacement/disconnect and controller |
| BSD native runtime | all capabilities | untested | untested | unsupported | implementation and host |

## Commands executed on this branch

Exact integrated results below were recorded through `ac7bc88c`. Older focused evidence remains listed
after the current integrated gates. `RUSTC_WRAPPER=` avoids treating a local compiler-cache failure
as product evidence.

```sh
cargo fmt --all --check
git diff --check
cargo check --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Pass. The PipeWire build script reported its existing SPA plugindir fallback warning.

```sh
cargo test -p nickel-core -p nickel-session-protocol -p nickel-ui -p nickel-file \
  -p nickel-ui-testkit --lib -- --test-threads=1
```

Pass. Reported library totals include `nickel-core` 183/0, `nickel-file` 181/0,
`nickel-session-protocol` 58/0, `nickel-ui` 406/0 with 2 ignored, and
`nickel-ui-testkit` 29/0. The `nickel-ui` suite includes the modal accessibility dispatch
regression. The testkit touch scenario exercises separately supplied normalized authority rather
than envelope self-certification.

```sh
cargo test -p nickel --lib session:: -- --test-threads=1
```

Pass: 472 passed, 0 failed, 6 ignored. This covers the integrated session authority route,
nonzero recipient leases, queued controller route-epoch and overflow/recovery fencing,
output-scoped native touch cancellation, native keyboard/clipboard routing, screenshot Escape
handling, focus lifecycle, internal-maximize supersession, XDG/XWayland operations, and geometry
ownership. The ignored rows explicitly require live native facilities.

```sh
env RUSTC_WRAPPER= cargo test -p nickel-core window_operation -- --nocapture
```

Pass: 12 tests, 0 failed. Covered typed cancellation, late acquisition cleanup, admission conflict,
externally contested control, resize-edge validity, release-before-activation, seat-wide security
cancel, per-window exclusion, pending-handoff release, source transfer, terminal admission release,
and unrelated release.

```sh
env RUSTC_WRAPPER= cargo test -p nickel-core --test stateful_interaction_acceptance -- --nocapture
```

Pass: 3 tests, 0 failed. Generated lifecycles release admission and fence tails; late geometry/focus
does not revive superseded authority; no-output placement is bounded by time, mapping and revision.

```sh
env RUSTC_WRAPPER= cargo test -p nickel-core --test interaction_acceptance_oracles -- --nocapture
```

Pass: 2 tests, 0 failed. Handled launcher background input has no fallback activation, and a stale
focus tail cannot hide a reopened surface.

```sh
env RUSTC_WRAPPER= cargo test -p nickel-core geometry_authority
```

Pass: 10 tests, 0 failed. This includes conditional compensation revision fencing, unknown-owner
withdrawal, bounded settlement, no-output revision guards and transform rebasing.

```sh
(cd vendor/smithay && env RUSTC_WRAPPER= cargo test --lib --no-default-features \
  input::touch::tests::completed_frame_cancel_terminates_changed_and_unchanged_contacts)
```

Pass: 1 test, 0 failed (43 filtered out). The public Smithay touch boundary cancels both a contact
changed in the completed frame and an unchanged retained contact, then rejects later motion/up.

```sh
env RUSTC_WRAPPER= cargo test -p nickel --lib --all-features \
  'session::grabs::move_grab::tests'
env RUSTC_WRAPPER= cargo test -p nickel --lib --all-features \
  removing_internal_surface_cancels_its_active_move_authority -- --nocapture
```

Pass: the move-grab filter ran 4 tests, 0 failed; the teardown filter ran 1 test, 0 failed. They
compile production Smithay adapters and exercise shared begin/update/completion, competing admission,
unexpected native-grab loss, maximized restore threshold, and internal-surface teardown. The first
build emitted the existing PipeWire SPA plugindir fallback warning. This is not native acceptance.

```sh
env RUSTC_WRAPPER= cargo test -p nickel --lib --all-features \
  client_request_is_independent_but_notification_without_token_is_unknown
env RUSTC_WRAPPER= cargo test -p nickel --lib --all-features \
  equal_unknown_notification_stays_pending_until_absolute_deadline
env RUSTC_WRAPPER= cargo test -p nickel --lib --all-features \
  internal_move_compensation_restores_only_the_last_owned_placement
env RUSTC_WRAPPER= cargo test -p nickel --lib --all-features \
  focused_non_desktop_surface_receives_the_same_normalized_keyboard_path
```

Pass: each filter ran 1 test with 0 failures. They verify XWayland independent/unknown causality,
unknown evidence remaining pending until its absolute deadline, revision-owned internal-move
compensation, and normalized keyboard routing into the same non-desktop internal host boundary.
Builds emitted the existing PipeWire SPA plugindir fallback warning.

## Present tests not claimed as executed

The tree also contains focused coverage for `GeometryAuthority`, conditional compensation, internal
restore revision fencing, shared default activation, normalized internal keyboard/controller host
identity, controller broker transfers, Windows nonblocking client/server adapters, XDG/XWayland
move/resize and unknown-causality settlement, lock/suspend cancel, and normalized internal touch
cancel. Presence supports the inventory, not a pass result. Integration commits contain concise
subjects but no embedded test transcripts, so this document does not invent results from them.

Suggested focused commands:

```sh
env RUSTC_WRAPPER= cargo test -p nickel-session-protocol --test controller_broker_stateful
env RUSTC_WRAPPER= cargo test -p nickel --lib --all-features session::
```

## Native acceptance still required

- Linux nested and installed: layouts/IME; device removal; changed and unchanged touch contacts
  across down/frame/cancel; matching and unrelated release; XDG/XWayland move/resize; titlebar,
  Super+pointer and internal move; XWayland unknown-causality/deadline observation; conditional
  compensation; cursor ownership; lock, suspend, output removal and target teardown.
- Windows: synchronous hook suppression; source/button binding; injected-input rejection; bounded
  missing-release reconciliation; actual `SetWindowPos` failure/late effects; native takeover;
  foregrounding, DPI, monitors, destruction; named-pipe partial I/O, disconnect and replacement.
- Physical controllers on Linux and Windows: reconnect identity, held/repeat, focus fence, transfer,
  revoke/reset, overflow/backlog and neutral recovery.
- Mixed-scale/multi-output geometry: transform rebase and compensation without overwriting a newer
  native or external owner.

Acceptance artifacts retain only opaque identities, counts, dimensions, timing and outcomes. Do not
retain text, clipboard payloads, credentials, private screenshots, window titles or trace names
without explicit authorization.
