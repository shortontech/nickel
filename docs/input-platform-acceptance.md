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
| Platform-neutral UI | active-widget identity and handled/fallback disposition | source tests present, not rerun | untested | not applicable | run full `nickel-ui` suite |
| Linux Smithay | physical keyboard/pointer/touch normalization | source tests present; focused tests partly rerun | test build pass | untested | real devices, focus/grab and lock/suspend teardown |
| Linux Smithay | completed-frame touch cancellation | focused vendor test pass | vendor test build pass | untested | real down/frame/cancel and absence of later motion/up |
| Linux Smithay | XDG move/resize, titlebar, Super+pointer, internal move | focused move tests pass | test build pass | untested | nested/installed grab, cursor and configure behavior |
| Linux XWayland | move/resize and compensation | source tests present, not rerun | test build pass | untested | real XWayland configure/focus/teardown |
| Linux Gilrs | identity, navigation, repeat, disconnect, focus fence | source tests present, not rerun | compiled as dependency | untested | physical controller not used |
| Unix controller broker/transport | connection/lease/stream generation, transfer/revoke/reset | source tests present, not rerun | compiled as dependency | untested | live transfer and neutral barrier |
| Windows focused/global input | winit, hook suppression, typed shortcuts | source tests present, not rerun | unavailable on this Linux pass | untested | Windows host, layouts, IME, hook registration/suppression |
| Windows foreign move/resize | bound source/button, contested control, release/reconciliation/takeover | core reducer pass; cfg tests not rerun | unavailable | untested | hooks, `SetWindowPos`, late effects, DPI, takeover |
| Windows controller pipe | bounded authenticated delivery and generation fencing | source tests present, not rerun | unavailable | untested | live pipe replacement/disconnect and controller |
| BSD native runtime | all capabilities | untested | untested | unsupported | implementation and host |

## Commands executed on this branch

Exact results from this worktree at `7b2cd6f` before the documentation commit. `RUSTC_WRAPPER=`
avoids treating a local compiler-cache failure as product evidence.

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

## Present tests not claimed as executed

The tree also contains focused coverage for `GeometryAuthority`, UI active-widget disposition,
controller broker transfers, Unix/Windows delivery, XDG/XWayland move/resize, lock/suspend cancel,
and normalized internal touch cancel. Presence supports the inventory, not a pass result. The
integration commits from `4a286f2` through `7b2cd6f` contain concise subjects but no embedded test
transcripts, so this document does not invent command results from those commits.

Suggested focused commands:

```sh
env RUSTC_WRAPPER= cargo test -p nickel-core geometry_authority
env RUSTC_WRAPPER= cargo test -p nickel-session-protocol --test controller_broker_stateful
env RUSTC_WRAPPER= cargo test -p nickel-ui
env RUSTC_WRAPPER= cargo test -p nickel --lib --all-features session::
```

## Native acceptance still required

- Linux nested and installed: layouts/IME; device removal; changed and unchanged touch contacts
  across down/frame/cancel; matching and unrelated release; XDG/XWayland move/resize; titlebar,
  Super+pointer and internal move; cursor ownership; lock, suspend, output removal and target teardown.
- Windows: synchronous hook suppression; source/button binding; injected-input rejection; bounded
  missing-release reconciliation; actual `SetWindowPos` failure/late effects; native takeover;
  foregrounding, DPI, monitors, destruction; named-pipe disconnect and replacement.
- Physical controllers on Linux and Windows: reconnect identity, held/repeat, focus fence, transfer,
  revoke/reset, overflow/backlog and neutral recovery.
- Mixed-scale/multi-output geometry: transform rebase and compensation without overwriting a newer
  native or external owner.

Acceptance artifacts retain only opaque identities, counts, dimensions, timing and outcomes. Do not
retain text, clipboard payloads, credentials, private screenshots, window titles or trace names
without explicit authorization.
