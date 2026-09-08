# Native-shell follow-up implementation record

Goal: complete active specs 0220–0225 without replacing or restarting the running session.
This record tracks partial work, not acceptance of the whole goal.

## 0224: owned preview refresh

Implemented `update_preview_image` in `live_shell.rs`. Production refresh compares the incoming owned
image by reference and moves it into Arc only on a change. Unchanged updates preserve the existing
Arc; no clone-only normalization helper remains in production. The old copying routine is retained
only under `cfg(test)` as an explicitly named historical benchmark baseline.

The focused regression passed: first admission and replacement preserve the incoming pixel pointer,
equal content preserves cache identity, changed dimensions/content replace it, and retired groups
release their images. This tests the production refresh helper; broader live acceptance is pending.

Release command, 2026-09-07:

`CARGO_BUILD_JOBS=4 cargo test -p nickel --lib --release owned_preview_refresh_release_evidence -- --ignored --nocapture`

For 1,000 updates at 240×135 RGBA:

| Workload | Copying baseline | Owned transfer | Eliminated pixel-copy payload |
| --- | --- | --- | --- |
| Unchanged | 4.178697 ms | 2.204662 ms | 129,600,000 bytes |
| Changing | 2.186280 ms | 0.086467 ms | 129,600,000 bytes |

Provider allocations are outside the timed region. The copying baseline verifies that the copied
pixel pointer differs while the source is alive; changed owned updates verify pointer transfer.
Payload counts describe that removed pixel copy, not aggregate allocator calls or measured RSS.
Arc headers, provider decode/copy cost, renderer imports, GPU memory, and end-to-end presentation
latency are not measured. The result does not fix the native GPU synchronization problem in 0225.

## 0221: native media dispatch and audio feedback

The native notifier now dispatches consumer controls directly through `InternalShellCoordinator` and
the injected `SessionHost`, returning before legacy subscriber fanout. The production host enqueues
to the existing PipeWire/MPRIS workers; its boolean result reports queue acceptance, not completed
audio/player execution. Rejected commands are logged without keyboard text. Windows retains its
native WM_APPCOMMAND ownership and does not duplicate these commands.

Native audio snapshots now arm the existing 1,500 ms OSD on volume/mute changes after initial
observation. Startup, device-metadata-only changes, and reconnection do not show unsolicited OSDs;
unavailability retires the OSD. Subsequent external mixer value changes do show feedback. At a known
volume limit, an accepted up/down request can display the last observed value without inventing an
increment. OSD mapping uses the current preferred interaction output, not launcher affinity, and
new runtime surfaces derive scale from their resolved placement. Limit feedback invalidates only
the OSD rather than every shell scene.

New recording-host tests exercise the coordinator and native session notifier, proving single
dispatch with zero/legacy subscribers, rejection handling, no optimistic result for ordinary
commands, unchanged keyboard focus, and OSD expiry. State/placement tests cover startup/metadata/
reconnect suppression, volume/mute changes, and requested-output/fallback placement.

`CARGO_BUILD_JOBS=4 cargo test -p nickel --lib --quiet` passed: 613 passed, 11 ignored.
`cargo fmt --all --check`, `git diff --check`, and
`CARGO_BUILD_JOBS=4 cargo clippy -p nickel --all-targets --all-features -- -D warnings` also passed.
The previously run `native_` subset passed 16 tests before the final placement regression was added.
Physical-key repeat/cancellation and actual mixer/player output remain to be exercised; the existing
compositor repeat implementation was preserved, not independently proven by notifier tests.
No physical or virtual media keys were sent to the user's desktop. Spec 0221 remains active pending
the remaining ingress and native acceptance coverage and full-workspace gates.

## 0222: keyboard authority bridge (partial implementation)

Added keyboard snapshot/configuration/input methods to the typed SessionHost boundary. The native
host reads one current shared snapshot and enqueues existing typed authority commands; only the
external PlatformSessionHost uses client RPC. Default unsupported/test hosts fail closed rather than
silently invoking external transport. Enqueue success does not fabricate a snapshot acknowledgement.

The session publishes on initial shell setup, configuration changes, authority command completion,
and before shell polling; publication replaces one snapshot and wakes only when changed. Focus and
auto-show request callbacks only wake the shell: publication is deferred until after Smithay releases
its keyboard lock. The first full-suite run exposed a deadlock when immediate publication read
`current_focus()` inside a locked focus callback. After this correction, the previously hung existing
`background_xwm_teardown_preserves_wayland_and_lock_focus` regression passed in 0.11 seconds.
LiveShell keyboard operations now use the injected host. The native generic UI arm passes recipient
epoch rather than preference generation. Show/hide no longer recursively refreshes an unacknowledged
auto-show snapshot, which could otherwise recurse while the command waits on the same event loop.

`CARGO_BUILD_JOBS=4 cargo test -p nickel --lib keyboard_ --quiet` passed 17 tests. New cases verify
snapshot replacement, typed configuration/input payloads, unequal epoch/generation, closed-channel
errors, actual calloop authority application before snapshot acknowledgement, and nonrecursive queued
auto-show. These tests do not inject input into the running desktop or modify saved preferences.

After correcting the focus-callback lock ordering, the full
`CARGO_BUILD_JOBS=4 cargo test -p nickel --lib --quiet` run passed: 616 passed, 11 ignored.
`cargo fmt --all --check` and `git diff --check` also passed. The running executable was not rebuilt
or replaced; only development/test artifacts changed.
`CARGO_BUILD_JOBS=4 cargo clippy -p nickel --all-targets --all-features -- -D warnings` passed.

This is not a complete 0222 implementation: normalized native gesture routing/leases, internal-app
recipient ownership, authoritative mapped geometry, complete preference/override matrix, and live
typing acceptance remain. The existing keyboard poll is still present; event-only refresh and command
admission must be reviewed with 0223 rather than treating a shared snapshot as an unlimited-work budget.

## 0220: native desktop topology and rendering (partial implementation)

`InternalOutput` now carries canonical logical origins from Smithay space geometry. The native
coordinator synchronizes desktop layout before constructing output slots, reserves space only for
outputs with a panel, and selects each output's retained desktop viewport before rendering. The
projection subtracts the whole-output origin, not the usable-area origin; top-panel reservations
therefore remain visible in local icon coordinates. Comments document these coordinate contracts.

A synthetic coordinator regression begins with directory entries but no layout outputs, then renders
the secondary before the primary without pointer input. It checks semantic labels and exact local
projection against production placement, negative origins, differing scales, both panel edges,
reversed primary enumeration, and three disconnect/reconnect cycles. Reconnection restores output
affinity and parked viewport storage remains bounded. The initial test incorrectly assumed the first
grid cell: existing policy can preserve global (0, 0) within a negative-origin output. The corrected
assertion checks exact model-to-surface projection and visible bounds, without changing that policy.

`CARGO_BUILD_JOBS=4 cargo test -p nickel --lib --quiet` passed: 617 passed, 11 ignored.
`cargo fmt --all --check`, `git diff --check`, and
`CARGO_BUILD_JOBS=4 cargo clippy -p nickel --all-targets --all-features -- -D warnings` passed.
This is rendering/topology evidence, not native input or live acceptance. The UI-only native event
route still discards button/modifier details before reaching the coordinator. Normalized ingress,
capture/focus cancellation, saved-layout fixtures, directory changes, and physical monitor testing
remain required. No running session, release executable, or saved desktop layout was changed.

The coordinator now accepts `HostEvent::Normalized` for Desktop and passes the original event to
the production `LiveShell::desktop_input` reducer. Rendering and input share one viewport-selection
helper, so a previously rendered secondary cannot redirect a primary desktop click. The coordinator
regression now resolves the icon through semantic geometry, renders another output, selects through
normalized input, opens a secondary-button context menu, and cancels menu/capture on focus loss.
It also delivers a real modifier snapshot and checks that focus loss clears selection modifiers,
since their matching key releases may be delivered to the newly focused client. This is a desktop
state fix, not an explanation or verified fix for the reported terminal `@` input.

The focused coordinator regression passed after these additions. This does **not** yet connect
Smithay's device ingress: native button routing still collapses buttons to pressed/released.
Full-fidelity device normalization, per-device capture,
keyboard ownership, and focus/hotplug cancellation must be completed before native acceptance.
After the normalized-dispatch and modifier-reset changes, the full Nickel library suite again
passed (617 passed, 11 ignored), as did formatting, diff whitespace checks, and strict Nickel
all-target/all-feature Clippy. These checks do not replace the required full-workspace gates.

The runtime-to-coordinator handoff now preserves complete `HostBatch` values rather than extracting
only `UiEvent`. This retains normalized device payloads and host focus transitions. Coordinator-owned
scenes enqueue to their actual owner; directly hosted applications reduce locally without adding
duplicate events to that queue. Runtime contract tests verify exact secondary-button/release/device/
order/position preservation, ordered focus gain/loss, queue draining, and absence of duplicate local
application dispatch. The desktop regression also checks modifier cancellation through the host's
`window_focused=false` batch, not only through a synthetic normalized FocusLost event.

Session focus changes wake the shell and deferred lifecycle batches drain before shell polling,
outside Smithay keyboard callbacks. Pointer paths flush cancellation/blur even when the next target
is a client and the runtime does not consume that input. Device ingress still needs its full-fidelity
adapter and capture policy; preserving batches does not by itself restore all desktop interactions.
After the final client-transition flush changes, the full Nickel library suite passed (619 passed,
11 ignored); formatting, diff whitespace checks, and strict all-target/all-feature Nickel Clippy
also passed. Runtime contract coverage is not physical-device or native desktop acceptance.

## Remaining implementation

Native desktop keyboard checkpoint: after compositor shortcuts run, a focused desktop now receives
normalized key presses/releases rather than only lossy UI actions. Physical identity uses the existing
winit scancode converter with Smithay's XKB-minus-eight offset; logical characters come from the
modified XKB symbol. Unknown identities remain native values instead of being guessed from text.
Device/order assignment shares the desktop input adapter with pointer events. Repeated backend
presses are marked as repeats; focus-loss batches clear retained presses, and device removal retires
that device's entries. This tracks supplied repeats; it does not add or prove a native repeat timer.

A regression exposed missing portable function/keypad mappings in the shared winit adapter. The
adapter now maps function keys, keypad keys, digits, and page navigation to existing portable enum
variants. Shared tests cover each added mapping; the native test distinguishes physical A from a
layout-produced q and verifies keypad Enter, release edges, device identity, and event order.
Runtime coverage verifies repeated press/release delivery and focus-loss retirement while a key is
held. The runtime module/state were renamed from desktop_pointer to desktop_input to match their
combined ownership. These are adapter contracts, not a completed live keyboard acceptance matrix.
Validation: 30 nickel-input tests passed with winit enabled; full Nickel library suite 627 passed,
11 ignored. After the module/state rename, all 52 desktop-focused tests passed. Formatting,
diff whitespace checks, and strict all-target/all-feature Clippy for nickel and nickel-input passed.

Hover/focus follow-up: native desktop input and generic widgets now share runtime hover ownership.
Moving from a panel into a desktop cancels the old panel hover before desktop motion; leaving an
uncaptured desktop emits normalized PointerEvent::Leave. Captured motion retains the original target.
The desktop clears visual hover on Leave but preserves menu/selection, following the user's final
clarification. An earlier uncommitted departure-dismissal experiment was removed; it is not policy.

FocusLost instead dismisses the menu, cancels pointer transactions, and clears shared selection and
selection modifiers, including the early branch where another output owns the menu. Native desktop
button presses now claim runtime focus; session reconciliation clears the old Wayland seat target
through the common internal-focus boundary even though the desktop has no application-window ID.
Runtime tests distinguish pointer departure from client-click blur and check ordered panel/desktop
hover handoff. Coordinator tests release the secondary button before Leave and then separately
assert menu/selection teardown on focus loss. Physical Alt-Tab, keyboard navigation, and complete
native focus acceptance remain unverified; the existing generic keyboard route is still insufficient
for desktop navigation. Comments record this pointer/focus distinction alongside the implementation.
Validation: full library suite 625 passed, 11 ignored. After moving shared focus-loss cleanup ahead
of the cross-output early return, all 50 desktop-focused tests passed; formatting, diff whitespace
checks, and strict all-target/all-feature Nickel Clippy passed. No running executable was replaced.

Native pointer ingress checkpoint: Smithay motion and button branches now try the desktop adapter
before the boolean-button UI path. The adapter retains button identity and edge, assigns device
identities retired on removal, and snapshots aggregate seat modifiers with each batch. The shared
desktop modifier reducer consumes that snapshot before input, including Ctrl held before a desktop
visit. Primary-button handling no longer aliases unrelated middle/back/forward presses to selection.

Capture keeps a gesture on its starting desktop across client areas and tracks held device/button
pairs. Removed surfaces swallow outstanding releases without creating replacement scenes; device
removal cancels only captures involving that device. Four focused contracts cover client-above-
desktop hit exclusion, negative-origin logical projection without rescaling, secondary-button and
modifier preservation, capture over clients, surface retirement, device retirement, and unrelated
device removal. These tests drive the production runtime adapter, not physical input devices.

The comments explicitly scope this adapter to buttons and absolute-position motion. Scroll, hover
departure, keyboard focus/navigation, touch, and complete capture cancellation still require work;
no end-to-end native desktop acceptance or terminal-input diagnosis is claimed. In particular,
remaining generic hover ownership must be reconciled with the new desktop-first motion route.
Validation for this checkpoint: full Nickel library suite 623 passed, 11 ignored; formatting,
diff whitespace checks, and strict all-target/all-feature Nickel Clippy passed. No release executable
was built or replaced. The user independently restarted SDDM during this work; these source changes
were not installed into that restarted session.

- 0220: complete normalized native desktop input, capture/focus lifecycle, and remaining acceptance.
- 0221: remaining input/backend/native acceptance and final integration gates.
- 0222: finish gesture leases, native/internal recipient routing, geometry, and acceptance.
- 0223: bounded latest-state status delivery with race-free wake/rearm.
- 0225: nonblocking preview scheduling/capture with bounded work and retry backoff.

Additional 0225 implementation evidence: the installed Smithay GLES `copy_framebuffer` issues a PBO
readback, and `map_texture` immediately calls `MapBufferRange` without an application readiness gate.
Removing Nickel's explicit `SyncPoint::wait` alone therefore cannot establish nonblocking capture.
Any staged readback must check completion **after the readback submission**, not only the earlier
render completion, before mapping. Renderer-context ownership, stale completion rejection, and safe
resource retirement remain required; no asynchronous pipeline has been implemented yet.

Specs remain active. Full workspace/lint gates and the specified native acceptance are not claimed
by the focused checks above. Existing docs/spec changes from the investigation remain preserved.

Additional checks for the 0224 implementation passed:

- `CARGO_BUILD_JOBS=4 cargo test -p nickel --lib live_shell::tests --quiet`: 88 passed, 3 ignored.
- `cargo fmt --all --check` and `git diff --check`.
- `CARGO_BUILD_JOBS=4 cargo clippy -p nickel --all-targets --all-features -- -D warnings`.

The release workload used a library test binary; the running release executable was not replaced.
