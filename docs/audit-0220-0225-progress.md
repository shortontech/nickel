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

## 0223: bounded native system-status delivery (implementation checkpoint)

Release retention evidence, 2026-09-07:
`CARGO_BUILD_JOBS=4 cargo test -p nickel --lib --release status_mailbox_retention_evidence -- --ignored --nocapture`
passed. The synthetic workload publishes 1,000 changing audio snapshots with 64 devices to three
stalled subscribers. One historical unbounded fanout stage retained 3,000 device-list payloads
totaling 40,128,000 bytes of vector/string capacity. The mailbox retained one shared latest payload
of 13,376 bytes across all three receivers, verified by Arc identity, with three wake callbacks.
Publication took 45.149565 ms for that baseline and 4.079075 ms for the mailbox in this single run;
these are not latency percentiles or compositor frame measurements. The test verifies final volume.

Capacity excludes queue nodes, Arc headers, allocator metadata, producer graph storage, and RSS.
Allocation operation counts and process peak RSS were not measured. This comparison models one
former fanout stage, not the complete old relay pipeline, and proves no particular idle RAM saving.
The ownership/lifecycle inventories now include the mailbox as `pending_measure`: four pending
domain slots bound backlog count, but individual backend vectors/strings remain uncapped. Hidden
or suspended UI retains its subscription and receives latest state; receiver/source replacement
or drop releases ownership rather than tying backend delivery to a particular visible surface.
Native media-repeat acceptance and remaining workspace gates are still required.

Validation for this measurement/inventory checkpoint: Nickel library suite 637 passed, 12 ignored;
the release retention workload passed separately; strict Nickel all-target/all-feature Clippy,
formatting and diff whitespace checks passed. Routine workbench validation passed with 28 fixtures,
45 cache and 45 lifecycle records, 22 consumer records, and 22 live acceptance records. Inventory
validation checks the records, not execution of those live acceptance cases; final-completion
inventory admission is not claimed. No running shell executable was replaced.

Added a typed four-slot mailbox for audio/network/Bluetooth snapshots and settings invalidation.
Each slot holds the latest immutable Arc payload; replacement releases the old pending payload.
Wake callbacks execute outside the mutex. Empty-to-pending publication wakes; drain removes pending
slots under the same lock; registering a wake after publication also wakes. Snapshot count is bounded,
but device-vector/string size and backend graph storage are not capped by this design.

Audio/control workers now register weak mailbox senders, sharing one published Arc across subscribers.
Receiver teardown removes registrations even when a backend is quiet. The native platform receiver
subscribes directly to both backends and owns its settings watcher. The native calloop source uses a
coalesced ping and drains current domains, replacing the previous source/receiver on reinitialization.
The two platform relay threads, native relay thread, and parked per-receiver settings thread are gone
from this path. Notify may still own implementation threads; no process-wide thread count is claimed.
Existing ordered audio/control command queues were not changed. The external audio shortcut feed
retains its blocking adapter, now reading latest audio state; this is not a general event-bus rewrite.

Initial compilation exposed incorrect relative module paths in nested Linux adapters; corrected to
the canonical platform path. Four focused tests then passed: 10,000-update stalled consumer, old
payload retirement, late wake/reentrant publication, quiet teardown, and concurrent publish/drain
are covered across those cases. Added tests cover shared payload retirement across subscribers and
real calloop wake/rearm/source removal. Full Nickel library suite passed: 636 passed, 11 ignored.
Formatting, diff whitespace checks, and strict all-target/all-feature Nickel Clippy passed.

Remaining 0223 requirements: release allocation/retention measurements, reviewed ownership inventory,
full-workspace gates, and native media/OSD acceptance under coalesced bursts.
No idle-RSS saving, allocation-count reduction, or complete physical media-feedback acceptance is claimed.

Coalesced feedback follow-up: the audio slot now retains two bounded activity facts along with its
latest snapshot. Volume/mute changes within a pending burst survive a return to the prior observed
value; availability transitions reset that value activity so reconnect-only changes remain silent.
The consumer displays the final observed state and invalidates the OSD even when the snapshot equals
its previous value. Coalesced unavailability also retires an already-visible OSD unless subsequent
available value activity warrants fresh feedback. No intermediate device lists or command history
are retained. Comments specify this distinction between replaceable state and feedback facts.

A mailbox-to-LiveShell regression verifies volume and mute round trips, the final OSD label, and
reconnect suppression/retirement. Full Nickel library suite: 637 passed, 11 ignored. Formatting,
diff whitespace checks, and strict all-target/all-feature Nickel Clippy passed. Physical media-key
repeat and compositor-visible presentation still need live acceptance; ordered command queues were
not modified by this feedback change.

## Remaining implementation (all specs)

Desktop scrolling checkpoint: native axis events now take the normalized desktop route before
generic widget routing. Smithay wheel v120 values become fractional line deltas with the normalized
sign convention; continuous touchpad values remain logical pixel distances. Integer discrete values
are compatibility hints, not the authoritative fractional amount. Scrolling does not claim keyboard
focus or create pointer capture. Existing captures still keep routing on their owning desktop.

The desktop overflow reducer now applies wheel lines at its existing three-cell step rate, while
pixel deltas remain pixel distances. It no longer reveals the selected item after passive motion or
scroll events, which previously could undo the user's scroll. Keyboard/button selection paths retain
reveal. Regression coverage checks half-wheel-step preservation, exact 1.5-pixel movement, no hover
snapback with a selected item, and runtime scroll delivery without focus/capture changes. This is
vertical scrolling of the existing overflow plane, not a redesigned desktop layout or touch gesture
implementation. The initial test compile needed an explicit closure parameter type; after that fix
both focused scroll-behavior tests passed before the runtime routing regression was added.
Final scroll checkpoint validation: full Nickel library suite 630 passed, 11 ignored; formatting,
diff whitespace checks, and strict all-target/all-feature Nickel Clippy passed. Native gesture and
physical wheel/touchpad acceptance remain outstanding; no running executable was replaced.

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

Follow-up inspection checked the actual Cargo.lock-pinned Smithay Git revision
`e3d461a057ba244d213a8498ec372b0799cca103`, not just the similarly versioned registry source.
Its GLES `finish_internal` falls back to `glFinish` when `export_sync_point` cannot create a fence.
`GlesFrame::drop` calls `finish_internal` and waits on the returned sync point: Nickel's early
clear/draw error returns can therefore block through unfinished-frame destruction too. A successful
explicit finish marks the frame finished, so the later Drop sees a signaled point; that distinction
matters for error handling. The pinned implementation also retains PBO ReadPixels followed by
MapBufferRange with no application readiness gate. A preview implementation must avoid the fallback
and unfinished-frame drop waits, not merely remove the explicit wait in Nickel. No dependency code
was changed, and no nonblocking guarantee is claimed from this source inspection alone.

Specs remain active. Full workspace/lint gates and the specified native acceptance are not claimed
by the focused checks above. Existing docs/spec changes from the investigation remain preserved.

## 0225: separate source dirtiness from preview presentation

The compositor now advances preview presentation generation only for completed frame storage or
retirement, not ordinary client commits or scratch-buffer retirement. Surface reassociation still
retires old pixels and advances presentation when a frame existed. The existing task-switcher key
uses that presentation revision, so source invalidation alone no longer forces CPU thumbnail resize
and overlay upload. The protocol retains its `preview_cache_generation` field name for compatibility,
with its completed-presentation meaning documented. Capture/content generations remain independent.

Two production-session regressions cover 1,000 content invalidations, failed replacement preserving
the old pixel allocation/revision, successful replacement, surface reassociation, and scratch versus
frame retirement. These are cache lifecycle tests, not semantic input or GPU responsiveness tests.
Focused preview tests passed: 27 passed, 2 ignored. This change does not remove blocking GPU waits,
introduce capture cadence/backoff, or prove bounded event-loop latency; those requirements remain open.

Validation: full Nickel library suite 639 passed, 12 ignored. After adding full-clear revision
coverage, the affected retirement test and strict Nickel all-target/all-feature Clippy passed again;
formatting and diff whitespace checks passed. Full-clear advances presentation only if completed
frames existed. No native input, mixer state, or running executable was changed.

## 0225: bounded capture-failure retries

Failed captures now retry after 100, 200, 400, and 800 ms, then stop after the fifth failure for that
admission/source identity. Candidate selection checks the cooldown even when ordinary client commits
advance content generation. Retrying clears the attempt marker for the newest content rather than
inventing a new source generation. Success removes failure state; source reassociation and renewed
visible admission reset it. Per-window failure metadata is restricted to the existing admitted set
and retires on hide/close/clear. The last completed frame remains intact after capture failure.

Native output presentation no longer treats preview failure as a reason for its 16 ms render-recovery
loop. One coalesced preview timer drains only due retry IDs; different deadlines remain pending.
Admission changes remove the previous calloop source, with an epoch guard against stale callbacks;
success or exhausted attempts cancel the timer when no retries remain. The nested backend's existing
capture interval can delay a retry further, but cannot shorten its failure cooldown.

Focused preview tests: 29 passed, 2 ignored. New deterministic session tests verify four exact cooldowns,
4,000 source invalidations that cannot bypass them, capped failure despite an hour of elapsed synthetic
time, reset/retirement, and two-window due-time separation. Existing real calloop tests cover timer
replacement and one-shot delayed dispatch. These tests exercise retry policy, not driver failure or
input-to-dismiss latency. A due retry still requests output rendering; readiness-driven capture,
successful-capture cadence/fairness, renderer-replacement reset, and removal of blocking GPU waits
remain unfinished. No native stress reproduction or session replacement was performed.

Validation for the retry checkpoint: full Nickel library suite 641 passed, 12 ignored; strict Nickel
all-target/all-feature Clippy, formatting and diff whitespace checks passed. These are Linux build
and deterministic test results, not native compositor acceptance or full workspace certification.

## 0225: explicit failed-submission completion

Native and nested capture now share `finish_preview_submission`: draw errors are retained until
explicit frame completion has run, then the original draw error is returned. This avoids propagating
clear/draw failure through Smithay's unfinished-frame Drop, which explicitly waits on a fence.
The small shared ordering test models successful/failed draw and completion combinations, preserves
the original error, and rejects implicit Drop waiting. It is not a real GLES failure-injection test.
Successful capture still uses the explicit wait/readback path, and explicit finish retains Smithay's
own glFinish fallback. No asynchronous-readback or complete nonblocking claim is made.

Further pinned-source inspection shows `MultiRenderer::Offscreen` and `Bind` use the target renderer
when one exists, and `copy_framebuffer` maps from that same target. MultiFrame completion can transfer
the render result to the target, with a CPU-copy fallback. Thus a staged native preview pipeline must
also choose a stable renderer owner; simply fencing whichever output's multi-GPU renderer happens to
be active can retain the cross-GPU completion path. A primary-GPU-owned preview submission is a
candidate design, but has not been implemented or validated. Output presentation still uses its
existing multi-GPU path; no DisplayLink or session configuration was changed.

Validation: the focused submission-ordering test passed, strict Nickel all-target/all-feature Clippy
passed (including compilation of both native and nested paths), and formatting/diff whitespace checks
passed. The full library suite was not rerun for this small follow-up; the preceding retry checkpoint's
641-pass result remains its own evidence, not a test result for this new helper.

## 0225: staged primary-GPU native readback (partial)

Native output rendering no longer calls preview capture before DRM/EVDI presentation. It schedules
one coalesced preview-work source. That source uses the primary GPU alone, submits at most one new
thumbnail per turn, and retains one texture/PBO/fence slot. The fence is created after PBO ReadPixels
submission and flushed; map/copy runs only after a readiness query reports completion on a later
turn. Pending polls do not request output rendering. Completed current pixels invalidate presentation.
No borrowed renderer is moved to another thread, and target-GPU transfer is not used for this work.

Per-window submission timestamps impose a 100 ms minimum interval. Least-recently-attempted candidates
are chosen first from the bounded admitted set; unchosen attempts are released rather than queued.
Admission/content tags and primary GPU/context identity reject obsolete completions. Obsolete work
keeps the pending slot until readiness or a one-second timeout, preventing client commits from
continually submitting behind a slow fence. Timeout enters the existing capped failure backoff.
Interest retirement prunes timestamp metadata; pending GPU resources can outlive hide until ready
or timeout. Lock/suspend and backend ownership retirement clear pending state without an explicit wait.
Ordinary idle presentation with no capture work does not arm this source.

The pending payload ceiling is one 240x135 RGBA texture plus one equally sized PBO: 259,200 logical
payload bytes, separately from the existing CPU cache. Actual GL/driver allocation, deferred-delete
retention, and source import costs are not measured; the new inventory row remains pending_measure.
Trace-level diagnostics include submission duration, map/copy duration, elapsed submission-to-install,
timeout and logical pending payload. Elapsed time is CPU monotonic wall time, not measured GPU duration.

This is not complete nonblocking acceptance. Smithay can still fall back to glFinish if its internal
fence export fails despite advertised capability; that API limitation is not bypassed. GL submission,
source import and cleanup latency remain unmeasured. The nested backend still uses synchronous
readback. Continuous source changes may reject pending snapshots before installation; live animation
coverage, renderer replacement/hotplug generations, semantic dismissal, complete counters, and native
frame/input measurements remain required. The readiness tests are pure policy/fence-adapter tests,
not a substitute for those native checks. No running compositor was rebuilt or replaced.

Validation: full Nickel library suite 644 passed, 12 ignored; strict Nickel all-target/all-feature
Clippy passed; routine workbench validation passed with 46 cache and lifecycle records. After adding
the existing renderer lifecycle activation/retirement counters to pending identity (in addition to
GPU node and EGL handle), both focused readiness checks and strict Clippy passed again. Formatting
and diff whitespace checks also passed. Driver
readback, stale animated completions, and native latency remain unverified.

Parallel 0222 work: a read-only review found that native OSK uses generic Overlay focus policy and
can steal its recipient's focus on press. A focused implementation is assigned in isolated worktree
`/external/.worktrees/nickel-0222-keyboard-routing`, branch `fix/0222-native-keyboard-routing`:
non-focus-taking identity, normalized pointer leases/cancellation, primary-button lease protection,
and stale resize cancellation. Geometry and internal application recipient authority remain separate
required follow-ups; no agent changes have been integrated or accepted yet.

## 0222: native keyboard pointer leases and authoritative geometry

Integrated agent commit `45335a5` as `5c2ef92` after review, following geometry checkpoint `bbfb6b8`.
The native keyboard now has explicit non-focus-taking surface identity. Normalized pointer edges
reach the existing `keyboard_host_input` reducer with press-time recipient leases; secondary-button
noise cannot overwrite a primary lease. Topology reconciliation cancels old gestures, and recipient
epoch changes clear stale resize ownership. Existing authority rejection remains the final stale
input check. Generic touch no longer steals text focus, but normalized touch leases are not implemented.

Native placement now uses the authority's output name, dock side and configured height through the
same `shell_layout::keyboard_area` used by external keyboard surfaces. A missing named owner output
does not silently relocate typing controls to another output. Coordinator layout dimensions update
before scene generation; runtime scene geometry, host logical dimensions and scale update without
destroying the surface ID. This also fixes the previous `relocate` path silently rejecting scene size
changes. Retaining identity is necessary for an in-progress keyboard resize gesture.

New placement/runtime contracts cover top/bottom docking, configured height, negative output origin,
fractional scale without double conversion, missing output, and in-place scene resizing. The agent's
production semantic-key test covers unequal generation/epoch, press/release delivery and blur
cancellation; its runtime test checks focus preservation and captured release across a client.
After integration, the full Nickel library suite passed 649 tests with 12 ignored, and strict
all-target/all-feature Nickel Clippy passed. The follow-up semantic typing test at resized coordinator
geometry also passed. No live typing, touch, controller, internal-app recipient, or complete preference
acceptance is claimed. Specs remain active.

The clean completed keyboard worktree and disposable test artifacts were removed after integration;
its branch and commits remain preserved. The isolated Smithay no-wait API worktree is still active.
The native preview guard now requires both ExportFence and Fencing, because shared-context texture
import/draw has its own synchronous fallback without Fencing. The vendor/API patch is not integrated
yet, so the runtime fence-export fallback remains open in the primary checkout.

## 0225: opt-in no-wait Smithay finalization integrated

Reviewed and integrated agent commit `e7373bf` as `2311248`. The public pinned Smithay API could not
disable its finalization fallback, so `vendor/smithay` preserves the pinned crate and a narrow
three-file GLES API patch. `GlesFrame::try_finish` shares normal GL state restoration, target
synchronization, profiler span closure and cleanup, but returns SyncExportFailed instead of calling
glFinish on fence-export failure. It also omits potentially synchronous GPU clock calibration.
The finished flag is set before fallible bookkeeping, so failed consuming finalization cannot make
Drop repeat that work with the blocking policy. Normal Frame::finish behavior remains unchanged.
Native preview submission uses try_finish plus both capability guards; nested capture still uses
its existing synchronous completion/readback path. Opaque driver call latency, null GL FenceSync
handling, real GPU failure injection, and live input/presentation measurements remain unverified.

The 4.7 MB vendor import contains the crate source/build inputs/licenses, not the full upstream
workspace. Its two upstream C files are existing GBM feature probes, not new Nickel application
components. `vendor/smithay/NICKEL-PATCHES.md` records provenance, scope, validation and removal
criteria. Sixteen existing upstream trailing-whitespace lines are preserved; the upstream-relative
API patch and Nickel edits are whitespace-clean. No blanket whitespace-check exemption was added.

Agent evidence: both production fallback-policy tests passed; all-feature Nickel preview tests
47 passed, 2 ignored; strict Clippy passed. After integration, primary Nickel library tests passed
649 with 12 ignored, strict all-target/all-feature Clippy and formatting passed, and Cargo's inverse
dependency tree selected exactly one Smithay at `/projects/nickel/vendor/smithay`. These synthetic
checks do not establish live GPU or native session acceptance. The clean renderer worktree and its
disposable build artifacts were removed; branch and commits are preserved.

Next isolated implementation: normalized native Desktop/OSK touch device/contact routing and
cancellation, worktree `/external/.worktrees/nickel-normalized-touch`, branch
`fix/native-normalized-touch`. The pointer/geometry work does not stand in for touch lease safety.

Additional checks for the 0224 implementation passed:

- `CARGO_BUILD_JOBS=4 cargo test -p nickel --lib live_shell::tests --quiet`: 88 passed, 3 ignored.
- `cargo fmt --all --check` and `git diff --check`.
- `CARGO_BUILD_JOBS=4 cargo clippy -p nickel --all-targets --all-features -- -D warnings`.

The release workload used a library test binary; the running release executable was not replaced.

## Native keyboard recipient follow-up

Added a distinct opaque internal-surface recipient token to keyboard snapshots. Native apps and
shell overlays do not necessarily have a Wayland seat target; their focus transitions now advance
the keyboard lease independently. The snapshot field is optional/defaulted for older serialized
snapshots and contains no typed content. Internal delivery reuses the existing UI key adapter and
host reducers, flushes coordinator input, and schedules native presentation. Shell overlays clear
the previous client seat target at the shared focus boundary.

A production session/UiHost adapter test verifies text reaches an internal app and an overlay,
distinct preference generation and recipient epoch, rejection of the first owner's stale input,
and rejection after focus surrender. This is synthetic adapter evidence, not live typing acceptance.
Modified internal key chords remain unsupported and explicitly rejected; plain-key navigation and
text delivery do not establish full chord/clipboard or automatic native text-field activation coverage.

Integrated `ab27be7` as `3fcd29e`: normalized Desktop/OSK touch retains physical device/contact
identity, captured target and last release coordinates. Surface retirement cancels before removing
coordinator identities and retains contact tombstones to consume later releases. Device removal
cancels only that device; seat-wide cancellation flushes immediately. Synthetic adapter tests cover
equal slots on separate devices and release after retirement; the semantic coordinator key fixture
now exercises touch typing and cancellation. Physical touch coordinates still use the existing
first-output transform, so multi-output touch mapping remains unfinished.

Primary all-feature Nickel library tests after integration: **661 passed, 12 ignored**; strict
Nickel all-target/all-feature Clippy passed. Follow-up review also corrected the generic hosted-app
touch branch to reconcile seat/OSK ownership, not only normalized desktop touches. The native
recipient test now transfers focus through that generic touch adapter and passes. These checks do
not replace the remaining workspace gates or live session acceptance.

Removed the integrated, clean `/external/.worktrees/nickel-normalized-touch` worktree and its
disposable artifacts after checking no process used it. Branch and commits remain available for
recovery. No installed or running binary was replaced, and the live session was not restarted.

## Workspace acceptance follow-up

The first default-feature `cargo test --workspace --quiet` run reached the source reuse gate and
failed because its Nickel inventory still counted 92 files rather than 96. Reviewed the four added
modules against their callers and existing authorities; `docs/code-reuse-audit.md` records their
distinct responsibilities and remaining input gap. Refreshed the inventory to 263 workspace Rust
sources (96 Nickel), correcting the audit prose's older 236-source count as well. The rerun passes
all three reuse-authority checks; full workspace completion is recorded separately once terminal.

Default-feature compilation also exposed an unused nested-only CPU-buffer failure helper. It is now
gated to `backend-winit`/tests, with the ownership difference from asynchronous native capture
documented at the method. No native failure accounting was removed.

Keyboard follow-up inspection confirms that `nickel-ui::FocusedInputDispatcher` already owns editing
chords. The native host must transport normalized input **and** clipboard outcomes: currently
`InternalUiRuntime::step` retains only `HostEventOutcome.changed`, and the session selection owner
only represents XWayland. Merely dropping the modifier rejection and forwarding a chord would still
lose copy results and provide no paste offer. This is an identified implementation seam, not evidence
that modified native input is complete. Added a protocol compatibility test for snapshots with the
optional internal-recipient field omitted, and for distinct native/window identity namespaces.

The workspace rerun is terminal, **not green**: after passing the refreshed reuse gate, it failed
`nickel-markdown-ui::cli::valid_and_missing_documents_keep_viewer_alive_without_sidecar_files`.
The launched viewer reported `Could not find wayland compositor` and exited before inspection.
No Xvfb, Xephyr, Weston, or Cage executable was found; available Xorg has no dummy driver.
No live display was substituted. A follow-up run excludes only this named GUI test to gather the
remaining workspace evidence, without changing or ignoring the test in source. Full GUI acceptance
remains pending in an isolated display environment.

The new keyboard protocol compatibility test passed. Full workspace strict Clippy also passed:
`CARGO_BUILD_JOBS=4 cargo clippy --workspace --all-targets --all-features -- -D warnings`.

The follow-up workspace run excluding the unavailable Markdown GUI test reached another stale
expectation: the workbench final-completion test expected 44 cache rows, but the new mailbox and
native readback owners bring the reviewed inventory to 46. Updated that exact expectation and added
a regression proving each new owner independently rejects final completion when its status remains
pending, even with all other statuses synthetically admitted. All four focused completion tests and
strict workbench all-target/all-feature Clippy passed. A no-fail-fast workspace rerun is collecting
the remaining results; the GUI exclusion remains explicit and no inventory status was promoted.

0224 now has a passing full-refresh fixture, not only a cache-helper test. A narrow owned-preview
source callback replaces frame acquisition while executing production `refresh_fast_changes`
admission, comparison, deadline and redraw logic. The fixture checks source allocation identity on
admission and pixel/dimension replacement, existing Arc identity for equal pixels, and actual
`close_window_preview` retirement. It does not test the external transport or measure GPU rendering.

The no-fail-fast workspace run is now terminal with exit 0:
`CARGO_BUILD_JOBS=4 cargo test --workspace --no-fail-fast --quiet -- --skip valid_and_missing_documents_keep_viewer_alive_without_sidecar_files`.
This includes the 46-test workbench suite and doctests. It is a passing run with one explicit GUI
exclusion, not a passing unfiltered workspace gate. The full-refresh fixture added during that run
was separately compiled and passed, followed by strict Nickel all-target/all-feature Clippy.

0223 allocation measurement now uses the existing `CountingSystemAllocator` in the library test
binary, installed only under `cfg(test)`. Thread-local sampling excludes parallel test/worker
allocation noise. The stalled workload counts allocation/reallocation calls during publication,
including cloned source statuses and queue/Arc allocation; channel/waker setup and later collection
are outside the sample. Retained capacity remains a separate device-vector/string estimate. This
does not add a shipped allocator, claim process RSS, or measure the entire old relay pipeline.

Release status result: 1,000 updates, 64 devices, three stalled subscribers; historical fanout
**387,099 allocation calls**, mailbox **130,000**. Retained device/string capacities remain
40,128,000 bytes across 3,000 historical snapshots versus 13,376 bytes in one shared mailbox payload.
Mailbox wakes: three. Publication timings in this allocator-instrumented run: 40.695845 ms versus
4.067046 ms. This is one historical queue stage, not a whole-process before/after measurement.
Command: `CARGO_BUILD_JOBS=4 cargo test -p nickel --lib --release status_mailbox_retention_evidence -- --ignored --nocapture`.

0224 warmed release refresh result (1,000 frames, 240×135 RGBA; map admission and provider allocation
outside sampling): unchanged frames **2,000 → 0 allocation calls**, changing frames **2,000 → 1,000**.
Both workloads remove 129,600,000 bytes of redundant pixel-copy payload. Counts include Arc/map
allocations; payload does not include Arc headers. Measured times: unchanged 4.422661 → 2.344318 ms,
changing 2.101509 → 0.077754 ms. The changing path still allocates one Arc header per replacement;
the provider still allocates its incoming pixels. No steady-state PSS or GPU saving is inferred.
Command: `CARGO_BUILD_JOBS=4 cargo test -p nickel --lib --release owned_preview_refresh_release_evidence -- --ignored --nocapture`.
Strict Nickel all-target/all-feature Clippy passed after adding the counters; the subsequent warming
change is test-only. An all-feature library run checks the allocator-enabled test binary separately.

Allocator-enabled all-feature Nickel library run completed: **662 passed, 12 ignored**.

## Renderer acquisition failure follow-up (0225)

Source review found that primary renderer acquisition failure before submission returned without
charging the preview failure budget when no pending job existed. Eligible work stayed ready, so
ordinary output activity could repeatedly schedule the same unavailable renderer. The native error
branch now charges eligible work through the existing per-window retry/backoff authority. A shared
eligibility predicate keeps scheduling and failure admission consistent; no second retry timer or
failure table was introduced. Already-cooled-down, exhausted and unchanged cached entries are not
charged. A pending job's failure remains charged once through its existing path.

A passing session adapter test simulates repeated availability failures and output-like repeated
checks: two eligible windows exhaust five attempts each, cooldown checks add no failures, a third
unchanged cached window is untouched, and the old pixel allocation and presentation generation are
preserved. This covers bounded failure accounting without a GL device; it does not measure driver
lookup latency or prove the complete native event-loop responsiveness requirement.

Verification: all-feature preview suite **49 passed, 2 ignored**; strict Nickel all-target/all-feature
Clippy, formatting and diff checks passed. No live GPU workload or session restart was performed.

## Audio reconnect command ownership (0221/0223)

Review found an actual ordered-command loss outside the mailbox: after `run_connection` failed,
`audio_worker` called `commands.try_recv()` solely to check disconnection and discarded an
`Ok(command)` result. A queued volume/mute action could disappear on each reconnect. The worker now
retains a probed head command in one pending slot across failed setup and consumes it before later
queue entries. This also preserves termination when the queue is empty/disconnected and PipeWire
cannot connect; simply removing the probe would lose that shutdown behavior. Backend I/O,
unavailable publication and the existing delay remain in the worker.

A synthetic reconnect test queues adjustment/mute pairs across two failed connection attempts and
asserts all four arrive in order before disconnect terminates the loop. It does not connect to
PipeWire or change user audio. Further live repeat/final-volume acceptance remains pending; a queue
delivery assertion alone does not prove asynchronous device property acknowledgments.

The companion unavailable-server test proves an empty disconnected command source stops after one
failed connection attempt. Focused audio tests: **4 passed, 2 live PipeWire tests ignored**. No live
volume or mute command was issued.

## Audio command acknowledgment ordering follow-up

The connection loop drained multiple commands before dispatching more PipeWire events, while each
relative adjustment/toggle read the last observed graph. Two queued +5 commands could therefore
both derive 55 from an observed 50 rather than deriving 55 then 60. Queue preservation alone does
not solve this asynchronous read/modify/write gap.

The worker now waits for the matching core sync sequence after a command, explicitly enumerates
the effective sink's properties, and processes another matching roundtrip before consuming the next
command. Graph locks are released before event dispatch. This uses the existing audio worker, one
completion slot and no compositor wait or new thread. Each roundtrip has a two-second deadline;
dispatch errors/timeouts leave the attempted command consumed and reconnect without replay, since
the server may already have applied a toggle. The pending-head reconnect rule still preserves later
commands. UI status remains derived from received graph properties, not requested target values.

Synthetic acknowledgment tests cover distinct sequences, property updates between two relative
adjustments, an old sequence that cannot satisfy a new wait, immediate timeout, and dispatch failure
without a retry. Real server/device acknowledgment behavior and held-key throughput still need
live validation; these tests do not simulate the full PipeWire protocol or assert hardware success.

Post-audio-ordering verification at `a4435d9`: all-feature Nickel library suite **667 passed,
12 ignored**. Focused audio suite: **6 passed, 2 live tests ignored**; strict Nickel
all-target/all-feature Clippy passed. No live audio operation was issued.

The isolated native keyboard/clipboard implementation remains unintegrated. Root review identified
a batch-ordering hazard in its draft admission handling: an earlier rejected cut must not discard
the ownership result of a later accepted cut after that later operation edits the document. The
agent is adding mixed-operation coverage and correcting that path. Async completion/closure handling
also needs transaction-aware retirement so closure cannot overwrite success or retire a newer read.
These are review findings, not claims of completed clipboard behavior. The product text limit still
awaits the user's choice; the suggested 16 MiB cap has not been treated as approved.

## Configured native touch output mapping (0220/0222)

The libinput boundary now carries `Device::output_name()` for touch down/motion into the generic
session input adapter. That metadata is absent from Smithay's generic Device trait and was
previously discarded. Normal and recovery touch routing resolve the named output's existing logical
geometry, without applying scale twice. An explicitly mapped but absent output fails closed instead
of redirecting touch to an unrelated monitor. The ordinary input entry point remains unchanged for
other backends, and devices without a libinput mapping retain the previous first-output fallback.
No per-device mapping cache or new settings policy was introduced.

A passing session adapter test covers the named second output, 1.5× scale, negative logical origin,
and removal without fallback. All-feature compilation, strict Nickel all-target/all-feature Clippy,
formatting and diff checks pass. No physical device or live session input was exercised. This does
not establish calibration for rotated outputs, map unconfigured devices, or complete live
multi-monitor touch acceptance. Earlier first-output limitations now apply to devices without a
configured libinput output hint, rather than every native touchscreen.

## Isolated GUI gate recovery

`CARGO_BUILD_JOBS=4 cargo build --workspace` passed at `9b27f8f`. To remove the remaining GUI-test
exclusion without using the user's desktop, downloaded the configured Ubuntu repository's
`xvfb` package (`2:21.1.22-1ubuntu1`) with `apt-get download` and extracted it with `dpkg-deb -x`
under `/tmp/nickel-gui-test.Lho6xt`. No system package was installed or global configuration changed.
The staged executable resolves its dependencies from the existing system libraries.

The previously failing Markdown CLI suite now passes **2 tests**, including the valid/missing
document startup test, under `xvfb-run -a -s '-screen 0 1280x720x24 -nolisten tcp'`. Its private
display uses generated Xauthority; Wayland and Nickel session-control environment variables are
cleared for the child, `WINIT_UNIX_BACKEND=x11` and `LIBGL_ALWAYS_SOFTWARE=1` are scoped to the test.
The runner stopped its display after completion. This is isolated GUI startup evidence, not native
Wayland/GPU/audio acceptance. A full workspace no-fail-fast run with **no test-name exclusion** is
now using the same isolated runner; its terminal result will be recorded separately.

## 0224 completion audit and final allocation evidence

The unfiltered isolated workspace run at `2b52b1e` completed with exit 0:
`cargo test --workspace --no-fail-fast --quiet`, with the private Xvfb environment described above.
No test-name exclusion was applied; built-in ignored tests remain ignored. The Nickel library
reported **659 passed, 12 ignored**, nickel-ui **335 passed, 2 ignored**, and the workbench
**46 passed**. Strict workspace all-target/all-feature Clippy also passed. Neither command exercised
the running native desktop; these results do not establish live acceptance for the other specs.

The release comparison now measures the redundant clone's additional live pixel-vector capacity
while its source is still alive, and runs both 240×135 and 480×270 RGBA images. The aspect-retention
regression now calls production `update_preview_image`, not the historical copy baseline.

| Dimensions | Frames per case | Removed cumulative pixel-copy bytes | Removed extra live pixel capacity | Unchanged allocation calls, old → new | Changing allocation calls, old → new |
| --- | --- | --- | --- | --- | --- |
| 240×135 | 1,000 | 129,600,000 | 129,600 bytes | 2,000 → 0 | 2,000 → 1,000 |
| 480×270 | 1,000 | 518,400,000 | 518,400 bytes | 2,000 → 0 | 2,000 → 1,000 |

Both unchanged and changing cases remove one full pixel allocation/copy per supplied frame.
The fourfold pixel-area increase produces fourfold payload/capacity, not additional retained caches.
Capacity measures this component's redundant buffer only: it is not allocator metadata, whole-refresh
heap high-water, GPU storage, RSS or PSS. Provider allocations and warmed map admission remain outside
the sample. Timings from this single instrumented run (old → new): 240×135 unchanged
4.476306 → 2.372421 ms, changing 2.404410 → 0.092266 ms; 480×270 unchanged
172.447491 → 11.669688 ms, changing 9.460208 → 0.089812 ms. Timing is observational, not a latency
guarantee or a claim that compositor stalls are resolved. Command:
`CARGO_BUILD_JOBS=4 cargo test -p nickel --lib --release owned_preview_refresh_release_evidence -- --ignored --nocapture`.

Requirement mapping for 0224: production refresh provides owned-frame admission, borrowed content
comparison, unchanged Arc identity, pixel/dimension replacement and close retirement; bounded group
churn covers release and the unchanged 32-entry limit. Source review confirms the 500 ms deadline,
visible-group filtering and content equality remain intact. Preview frame tests cover retained pixel
identity across theme changes, source-aspect containment, semantic activation geometry and ordering.
No production renderer or activation policy was changed by the copy removal. This scoped evidence
does not substitute for 0225 native GPU/readback or DisplayLink responsiveness acceptance.

Final focused command `CARGO_BUILD_JOBS=4 cargo test -p nickel --lib preview -- --nocapture`:
**48 passed, 2 ignored**. The ignored allocation comparison was run separately in release mode as
recorded above. Formatting and diff checks passed. Spec 0224 is archived under `specs/done/`;
the remaining five specs stay active. The keyboard agent's reviewed commit `ff956b3` is ready but
not yet integrated; its clipboard limit remains unconfigured pending the user's choice.

## Native keyboard chord integration and semantic clipboard follow-up

Integrated agent commit `ff956b3` as `b62263f`, retaining the newer libinput output mapping and preview
retry fixes. Native keyboard chords now use shared normalized UI key policy without changing physical
seat modifiers. Clipboard outcomes survive native-runtime and shell-coordinator transport. Copy/cut
admission precedes editor mutation, secure-field restrictions remain shared, and descriptor transfers
use deadlines and bounded worker permits. The product text-size limit is still **unconfigured**;
native clipboard admission remains disabled until that policy is resolved. No default was inferred
from the unanswered 16 MiB suggestion.

Integration review found that semantic UiEvent routing in launcher/control-center surfaces still
bypassed the agent's normalized-only clipboard handling. Those surfaces now accept either event form
through the same limit-aware host call and preserve its ownership outcome. A coordinator regression
rejects an oversized semantic Cut, then successfully cuts the preserved selection under an admitted
limit. Existing dependency invalidation stays in place; hidden control-center input remains rejected.
The failure wording now refers to the rejected operation, not the whole batch, because a different
copy/cut in that batch may have succeeded.

Root verification: native keyboard lease test **1 passed**; coordinator suite **15 passed**;
clipboard-filtered all-feature tests **8 passed, 4 live tests ignored** across Nickel/nickel-ui;
source-reuse authority tests **3 passed**. Strict workspace all-target/all-feature Clippy passed,
followed by the small hidden-control guard restoration. The source inventory now covers 265 Rust
sources, including 98 in Nickel, with the two new clipboard modules' ownership boundaries documented.

Remaining acceptance: real Wayland/XWayland clipboard interoperability, live keyboard visibility,
typing/focus and device/output behavior, and the clipboard-size decision. Async paste leases currently
protect surface identity and keyboard epoch, **not field identity inside an unchanged surface**.
The implementation comment now states that exact scope rather than promising field-level safety.
Spec 0222 is not archived, and no running desktop, audio, clipboard or input state was changed.

Post-follow-up full all-feature library run completed with exit 0:
`CARGO_BUILD_JOBS=4 cargo test -p nickel -p nickel-ui --lib --all-features --quiet`.
Nickel: **671 passed, 12 ignored**; nickel-ui: **336 passed, 2 ignored**. Strict workspace
all-target/all-feature Clippy was rerun after the hidden-control guard and passed; formatting and
diff checks passed. The integrated keyboard worktree was clean, with no remaining agent build,
before cleanup of its checkout and 3.3 GiB disposable target cache. Its branch/commit are preserved.

## Native preview work diagnostics (0225)

The cache-diagnostics response now includes `native_preview_work`, a fixed-size counter snapshot.
Current pending count and texture/PBO payload bytes are derived from the actual pending owner at
query time, so retirement reports zero without requiring every exit path to clear a mirrored gauge.
Successful admission records peak combined logical pending payload. This separates retained GPU-side
texture/readback payload from the existing completed CPU preview-cache accounting; it does not
estimate driver allocations or process RSS.

Counters cover turns, unsignaled polls, submissions/failures, successful installs, failed map/size
validation, stale/context/renderer-loss/lock cancellation, and timeouts. Cumulative submit and map/copy
CPU wall time include failed calls; successful completion age includes event-loop scheduling delay
and must not be described as GPU elapsed time. The snapshot is exposed on explicit diagnostics
queries without normal-level per-frame logging, additional workers, or retained timing samples.
Protocol defaults accept older payloads without the new field, with a passing roundtrip/default test.

This is partial diagnostics completion, not native acceptance: overlay rebuild/upload timings,
per-output frame/input latency distributions, full event-loop fault-injection scenarios and controlled
GPU/DisplayLink measurements remain required. Existing trace records retain window identity; the
new snapshot is aggregate per current native backend lifetime and does not attribute output latency.

Verification: all-feature Nickel preview tests **49 passed, 2 ignored**; diagnostics protocol test
**1 passed**; all-feature compilation and strict workspace all-target/all-feature Clippy passed.
Formatting and diff checks passed. No live preview stress or desktop change was performed.

## Native hover-preview delivery bridge (0225)

Source tracing confirmed a second native preview gap: `WindowFeed::internal()` has no socket, so
its `preview()` cannot return pixels. The native coordinator also never calls the external shell's
`sync_transient_overlays`, leaving hover interest unregistered. A finished GPU readback alone could
therefore not populate native hover cards.

Native shell synchronization now obtains the visible hover window IDs from the existing LiveShell
group, reconciles them through the session's existing overlay-interest admission, and supplies
borrowed completed frame dimensions/pixels directly to the existing UI preview cache. Session IDs
remain private to the adapter. No self-RPC, JSON conversion, helper thread, additional frame cache or
new preview-selection policy is introduced. The existing session admission ceiling still determines
which visible windows have frames; the remaining cards keep placeholders.

Changed pixels are copied once into the UI's existing Arc-backed image owner because the session
must retain its completed CPU frame for its other consumers. Equal pixels preserve Arc identity.
This is a required ownership-boundary copy, not the redundant same-owner clone removed by 0224.
The coordinator remembers only the completed presentation revision, avoiding pixel comparison during
ordinary input/scene updates; empty caches can still refill after a close/reopen with the same revision.
Frame retirement removes the UI copy. Native completion requests a preview-only content refresh
before output presentation; it does not mark unrelated shell content dirty merely to install pixels.

A passing ownership fixture covers pending/no-frame behavior, delivered bytes, unchanged Arc identity,
pixel/dimension replacement, retirement and no requests after hide. Full native routing/rendered
acceptance and controlled latency measurement remain outstanding; this fixture does not establish
those. The asynchronous store comment was updated to state the actual caller-validated lease invariant.

Verification: all-feature preview suite **50 passed, 2 ignored**; coordinator suite **15 passed**;
strict Nickel all-target/all-feature Clippy, formatting and diff checks passed. No live desktop
restart, binary replacement, input injection or preview stress was performed.

## Hover presentation-owner retirement verification

Extended the native delivery fixture through production `window_preview_scene`, not just the image
map. It now proves that the rendered frame retains replaced/retired pixels until scene refresh,
then releases their last strong ownership. Closing with a populated rendered frame drops both the
image map and presentation host immediately. Changed thumbnail dimensions preserve the semantic
activation target's bounds. The focused all-feature test passed. This is host/display-list ownership
evidence, not proof of GPU texture retirement or live input latency.
