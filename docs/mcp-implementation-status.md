# Specs 0230 and 0231 implementation checkpoint

Both specifications remain active. Their completion criteria include native
Linux and Windows acceptance and cross-machine debugging. Linux unit and nested
native checks alone do not establish completion.

## Current gaps confirmed in source

| Requirement | Current state | Remaining work |
| --- | --- | --- |
| Windows desktop control and local approval | Winit now owns listener lifecycle and bounded private Settings IPC with process-image, user/session/integrity checks. The owner continuously samples the current input desktop and WTS session state, revokes runtime authority on a protected-desktop transition, and rechecks that state when activating a connection watch. Generation-bearing window actions now request activate, minimize, maximize, restore, close, move and resize through a freshly revalidated owner boundary and return a separate fresh observed outcome. Authorized client-area capture resolves its native source on the owner, acquires bounded pixels off the UI thread, excludes composed trusted chrome by reading the window DC, and revalidates before publication. The owner now exposes the production Start Menu catalog with stable generations and a 512-entry wire bound; application leases see only their verified catalog identity, while full-session leases see the bounded catalog. Lease approval and resumption remain denied. | Complete UIA, launch dispatch/output placement and remaining operations, then validate trusted indication, secure-desktop transitions, capture fidelity, local accessibility and control on Windows and across machines. |

Windows low-level key and pointer hooks now distinguish OS-attributed injected
events from physical activity. Physical activity advances only a lock-free epoch;
Nickel retains no key, coordinate or timing history. The owner consumes epoch
changes before queued remote work and cancels shared remote-input ownership without
revoking the lease. Windows keyboard transactions and owned held chords now use
bounded native batches behind fresh window, focus, desktop, resource, permit and
physical-input checks. The hook synchronously releases registered remote keys when
physical input arrives; the owner also releases them on focus loss, timeout,
disconnect, revocation, protected desktop and emergency stop. Payloads and timing
are not retained or logged. Pointer transactions and one owned drag use freshly
validated client-relative points, refuse obscured targets, map across the complete
virtual desktop and retain only the synthesized button bit needed for release.
Native Windows execution and remaining protected-resource acceptance are still
unavailable, so these paths do not open approval.
| Logical client disconnect/resumption | Lease requests, local approvals, reconnects and desktop permits now require an unexpired owner-ready watch. Last-watch loss invalidates authority and triggers native cleanup; overlapping watches preserve connection continuity. | The stdio client adapter and saturation/cancellation fixes are integrated; complete broader native lifecycle acceptance. See `mcp-connection-watch.md`. |
| Launch under an output lease | Verified Wayland/X11 launch ancestry now drives output placement before scene insertion, under the original lease and output incarnation. Nested native success/cancellation/replacement tests passed. | Complete broker/daemon attribution and broader physical, overload, deadline and cancellation acceptance. |
| Full diagnostic coverage | The compositor snapshot explicitly reports unavailable domains. Ordinary shell transients use production visibility and protection evidence; typed shell actions, effect ordering, every typed platform refresh, and bounded external accessibility are integrated. Pointer diagnostics correlate hosted UI hits with a live bounded semantic tree generation/node ordinal or a fixed Nickel frame role. The Windows owner now returns a generation-bearing, freshly revalidated snapshot containing its protected-filtered scoped windows/outputs, focus, remote held-input state, operation/admission/lease metrics, trace lifecycle and payload-free warning/error source locations; every absent Windows domain is named explicitly. Windows event polling/subscriptions use the same full-debug owner boundary and currently retain only coalesced, payload-free remote keyboard/drag ownership transitions. | Complete GPU timing, shared renderer resources/caches, remaining event and trace categories, the protected-safe portion of unsupported Codex transient diagnostics, and the Windows domains still reported unavailable. |
| Physical DRM trace acceptance | Production DRM render dispatch is instrumented. The active seat is shared with the user's compositor. | Test on an isolated seat or machine; primary-GPU selection does not isolate the current udev backend's device enumeration. |
| Full native acceptance | Recent native work uses a separate Xvfb-backed compositor with its own Xwayland and Wayland clients. | Complete the specifications' physical emergency-stop, assistive workflow, mixed-DPI/multi-output, Windows, and cross-machine gates. |

This table records confirmed gaps, not an exhaustive completion audit. Each
specification's full verification list remains authoritative.

Output-scoped launch retains at most 128 placement intents for 30 seconds. Native
output identity and generation, root process incarnation and verified bounded
ancestry are rechecked before first scene insertion. Both Wayland and X11 defer
mapping while identity resolution is pending. Authorization covers the placement
and scene insertion itself. Lost authority or output identity releases the launched
application to ordinary local mapping without granting control outside the lease.

Nested native tests covered X11 and Wayland launch onto a secondary output, a
verified descendant created after 2.4 seconds, revocation before its first window,
and retirement/recreation of a same-name output. Existing forged-X11-PID and
Wayland map/identity acknowledgement regressions also passed. Evidence remains in
`/tmp/nickel-mcp-output-native/` and `/tmp/nickel-output-native-owner-tests.txt`.
Single-instance brokers and descendants whose root has exited remain unattributed;
physical DRM, identity overload and exact deadline/cancellation timing need further
acceptance. These are explicit remaining work, not waived completion criteria.

## Recent evidence

Mandatory watch enforcement passed all 79 remote-control package tests and the
integrated Linux Nickel all-target check. A rebuilt native compositor rejected
permission requests without a watch. After standard MCP progress acknowledged
readiness, watch loss cancelled its pending request, a replacement watch established
a new pending incarnation, and both stale approval and stale denial were rejected.
Fresh approval succeeded exactly once. The fixture revoked its lease and stopped
its owned compositor/Xvfb. Evidence: `mandatory-watch-integrated-results.txt` and
`mandatory-watch-native-results.txt` in `target/mcp-native-2026-09-10/`; the exact
fixture is `/tmp/nickel-mcp-native/pending-approval-watch-test.py`.
The subsequently identified reserve-before-expiry, activate-after-expiry race
now has a regression that failed before its fix. Activation preserves its validated
reservation while expiring the old watch before replacement readiness, so the gap
revokes non-resumable authority, cancels pending requests/input and advances the
operation generation. Explicit cancellation still prevents reservation revival.
This timing case has deterministic owner coverage; it is not claimed as a native
scheduling test.

Pending approval cards now carry a checked monotonic `pending_generation` through
session protocol version 26, Settings semantic messages, and Linux/Windows local
decision handlers. Equivalent still-pending requests coalesce without changing
the generation. Replacement or cancellation followed by an identical new request
cannot reuse an earlier card. Approval and denial check both generation and payload;
generation exhaustion rejects replacement while preserving the existing card.

The protocol suite, Settings recovery/semantic-action regression and Linux Nickel
all-target check passed (`pending-approval-generation-results.txt` and
`pending-approval-native-build.txt` under `target/mcp-native-2026-09-10/`). A live
nested compositor accepted an initial request, disconnected/reconnected the same
identity and received an identical new request. Both old-generation decisions were
rejected, the fresh request remained pending, and its approval created exactly one
lease. The fixture revoked it and stopped its owned compositor and Xvfb. Evidence:
`pending-approval-native-results.txt`. This run used explicit client disconnect;
mandatory watch lifecycle integration and native Windows execution remain separate
validation work.

Settings refresh failures now mark the current runtime observation unavailable,
reset its acknowledgement and remove stale client/approval/active-lease cards and
pairing codes. Historical audits remain available. A successful runtime snapshot
replaces the unavailable projection. The production-UI regression verifies that
an approval semantic action disappears after a timeout and returns only after a
fresh snapshot; the focused test and Settings Clippy passed
(`settings-stale-runtime-results.txt`, `settings-stale-runtime-clippy.txt`).
This changes local presentation, not authority in the runtime: loss of the
Settings connection does not itself claim to revoke a remote lease. Nested Linux
visual acceptance now covers connection loss and recovery, as recorded below.

Windows startup now constructs its runtime owner and bounded local request queue.
Settings uses a private named pipe whose peers must match the pinned original
process image and user/session/integrity. Windows-target Clippy passed for platform
and Settings with warnings denied, and for Nickel with existing dead-code warnings
(`/tmp/nickel-windows-transport-clippy.txt`, `/tmp/nickel-windows-owner-clippy.txt`).
No native Windows execution is claimed. The owner now uses its fail-closed input
desktop and WTS session observation to deny protected-desktop watch activation and
to revoke runtime authority before queued work after a lock transition. Native
operations and their input ownership remain unfinished, so lease approval and
resumption are denied. The physical-key hook now latches an atomic cancellation epoch immediately.
Permits reject stopped epochs before waiting for the authority mutex, at commit
and at result delivery; re-enable requires cleanup of the exact stopped epoch.
Native input release and listener cleanup still run on the owner. Chord
recognition now uses atomic state with enable generations and has executable
Linux semantic tests, but full native Windows cancellation acceptance is not
established.

Settings no longer presents an unacknowledged runtime query as confirmed Disabled.
Its missing-runtime fallback uses the unavailable state and an explicit diagnostic;
the rejected/unavailable row reads “Unavailable” rather than “Change rejected”.
Linux Settings Clippy and the Windows Settings all-target check passed
(`settings-runtime-unavailable-clippy.txt`,
`settings-runtime-unavailable-windows-check.txt`). Native Windows validation and
live visual acceptance of this fallback remain outstanding.

Windows process application evidence now requires a fresh retained-handle liveness
check before exposing the cached OS package identity. The Windows regression
spawns an owned test child, probes its real process handle, seeds a cached label
to cover unpackaged test binaries, kills/reaps the child and requires both process
matching and application evidence to become unavailable. The seeded label tests
lifetime gating only, not package-family verification. Windows-target all-target
Clippy for `nickel-platform` passed with warnings denied
(`windows-identity-exit-clippy.txt`). The regression has been cross-compiled, not
executed on Windows. Windows MCP owner integration and native acceptance remain
outstanding.

Catalog preparation now reads application-scale settings once per request and
applies that snapshot to every inspected entry, avoiding repeated reads and mixed
scale policies within one inventory. The supplied-settings command path shares
ordinary command construction and trusted-capability stripping with local launch.
Model regression coverage checks exact arguments, custom scale environment and
removal of all three trusted session variables. Focused model tests and scoped
Clippy passed (`catalog-scale-snapshot-results.txt`,
`catalog-scale-snapshot-clippy.txt`). A subsequent native multi-entry check supplied
settings through a FIFO exactly once. The running compositor returned multiple
verified application entries and released preparation admission without needing
a second writer. This would time out if a later entry reopened the FIFO. The
fixture removed its FIFO and desktop entry, revoked its lease and logged out
(`catalog-single-read-native-results.txt`, `catalog-single-read-native-build.txt`).

Application-scale configuration reads used by launch/catalog preparation now
consume at most 64 KiB plus one overflow-detection byte. Oversized input returns
`InvalidData` before policy parsing; existing launch callers retain their default
settings fallback on load errors. Saves reject output over the same limit before
atomic replacement. File-backed regression coverage verifies the exact boundary,
oversized-read rejection and preservation of the previous file after an oversized
save. DPI tests and scoped core Clippy passed (`application-scale-bounds-results.txt`,
`application-scale-bounds-clippy.txt`). Subsequent nested MCP acceptance launched
native X11 clients using both a valid custom-scale file and an oversized file with
the same policy prefix. The valid file produced the expected toolkit environment;
the oversized file used defaults without parsing that prefix. Inventory, launch
and owner diagnostics remained responsive. The fixture removed its temporary
configuration/desktop entry, terminated its clients, revoked its lease and logged
out (`scale-bounds-native-results.txt`, `scale-bounds-native-build.txt`). This
bounds read allocation, not the wait for a blocked filesystem operation.

Native inventory cancellation acceptance blocked a real application-scale
configuration read in the preparation worker. While blocked, compositor
diagnostics and local revocation remained responsive, a competing inventory
preparation was rejected in under one second, and the revoked HTTP caller
received an error without catalog data. The stalled worker retained its single
admission after caller timeout. Releasing the read freed admission; fresh
authorized inventory succeeded while the revoked lease remained denied. The
fixture removed its FIFO, revoked its remaining lease and logged out the owned
nested compositor. Evidence: `catalog-cancel-native-results.txt` and
`catalog-cancel-native-build.txt` under `target/mcp-native-2026-09-10/`.
This proves bounded admission and owner responsiveness during a stalled read;
it does not make the underlying blocking filesystem read interruptible.

Installed application inventory now includes an optional `verified_application`
identity, obtained from executable inspection before any window exists. Both
application-scoped and full-session inventory use the bounded preparation worker;
filesystem inspection stays off the compositor owner. Full-session inventory
retains entries with unverifiable targets and reports no verified identity for
them. Application inventory includes only matching verified executables. The
owner rechecks catalog generation and lease authority before returning the result;
launch and each resulting window independently revalidate executable identity.

Native acceptance began with no application windows, obtained an identity under
a bootstrap full-session lease, approved an application lease, revoked the full
lease and launched using only the application lease. Its XRes-verified window
matched the inventory identity, scoped enumeration exposed only matching entries,
and a different executable was denied. The fixture revoked the lease, terminated
its owned client and logged out the nested compositor. Evidence:
`catalog-identity-native-results.txt`, `catalog-identity-native-build.txt`,
`catalog-identity-results.txt`, and `catalog-identity-clippy.txt` under
`target/mcp-native-2026-09-10/`. Local installed-app selection without bootstrap
inventory authority, shared runtimes, Windows and output-scoped launch remain open.

Ordinary shell-surface diagnostic records now include the production presenter
scale factor and pending-redraw flag, sampled with placement on the owner thread.
These supplement coordinator scene generation; a cleared redraw flag does not
claim that pixels were presented. Existing visible/protected role filtering is
unchanged. The production-owner regression configures the launcher at scale 1.5,
verifies the scale and pending redraw with stable surface identity, and continues
to check hidden-launcher and locked-session exclusion. It passed
(`shell-presenter-diagnostic-results.txt`); scoped Clippy is recorded in
`shell-presenter-diagnostic-clippy.txt`. A running nested compositor subsequently
confirmed that the visible launcher's redraw flag settles after rendering, and
that adding a 1.5-scale virtual output yields desktop/panel records at 1.5 alongside
the original output's 1.0 records. Output retirement and launcher hiding remove
their records; revocation denies subsequent diagnostics. Evidence:
`shell-presenter-native-results.txt`. The fixture revoked its lease and logged out
the owned compositor. Physical mixed-DPI presentation and protected transient/content
diagnostics remain open.

A deterministic blocking-pool regression now exercises the production capture
queue helper with one occupied worker. It queues a capture under a live lease,
revokes that lease, releases the worker and checks that authority rejection wins
over an intentionally invalid pixel-buffer error. This establishes that the
queued encoder checked revocation before inspecting pixels, and that its capture
reservation was released on rejection. Cleanup releases the occupied worker even
when an assertion unwinds. All 69 remote-control tests and scoped Clippy passed
(`queued-capture-results.txt`, `queued-capture-clippy.txt`). This is a production
worker/authority regression, not a live compositor capture acceptance test.

The blocking PNG encoder now checks its original permit before beginning work
and after encoding. A task that waited in the runtime queue cannot start encoding
after request cancellation, deadline expiry or lease interruption; an encoding
that finishes after cancellation drops its result. The existing final compositor
check still resolves current resource membership and protection before delivery.
Capture admission remains held by both the worker and queued response bytes.
This does not preempt a PNG encoder already running. The 68 remote-control tests
and scoped Clippy passed (`capture-encoder-lifetime-results.txt` and
`capture-encoder-lifetime-clippy.txt`); a dedicated native queued-encoder
cancellation check has not yet been run.

Full-session launch now retains execute-only targets through an `O_PATH`
descriptor when header inspection is denied, instead of falling back to the
replaceable pathname. No application identity is assigned without a readable ELF
header, and the kernel still enforces execution permissions. Other header I/O
failures fail preparation; readable non-ELF/script behavior remains unchanged.
An explicitly executed native regression, running unprivileged, confirms reads
are denied, replaces the prepared `true` pathname with `false`, and proves the
retained original still executes successfully. Evidence:
`target/mcp-native-2026-09-10/execute-only-launch-native-results.txt`.
The fixture uses the basename `false` so multicall coreutils dispatches the
replacement normally. The 11 other identity tests passed
(`execute-only-launch-results.txt`); scoped all-target Clippy is recorded in
`execute-only-launch-clippy.txt`.
Readable script fallback and output-constrained launch placement remain open.

Permission-request retention and local lease creation now share resource scope
validation: identifiers must be nonempty, at most 512 bytes and free of control
characters; surface/window/output generations must be nonzero. Platform identity
verification remains separate. Regression coverage rejects oversized UTF-8 and
ASCII identifiers, control characters and zero generations across resource types,
proves invalid replacements preserve the exact pending approval, and accepts a
512-byte UTF-8 boundary. The remote-control crate tests passed
(`resource-scope-bounds-results.txt`), as did scoped Clippy with warnings denied
(`resource-scope-bounds-clippy.txt`). A separate running nested Nickel session then
rejected all 23 malformed HTTP requests without changing the local pending
projection or creating leases. The preserved request remained locally approvable
and revocable; a valid 512-byte UTF-8 identifier round-tripped through the local
projection and was denied locally. The fixture revoked its lease and logged out
the owned session (`resource-scope-bounds-native-results.txt`).
This does not complete installed-application
selection/approval before its first window or output-scoped launch placement.

The workspace regression run after the notification, launch identity and input
diagnostic changes passed: 2,150 tests, zero failures, 30 ignored across 88 suites.
The ignored count includes the two native launch tests explicitly executed in
their separate runs below; it does not establish the remaining native gates.
Evidence: `target/mcp-native-2026-09-10/workspace-launch-current-results.txt`.
Workspace all-target, all-feature Clippy also passed with warnings denied
(`workspace-launch-current-clippy.txt`).

An opt-in native XWayland launch regression now publishes a forged `_NET_WM_PID`
before mapping a real X11 window into an isolated compositor. The normal identity
worker verifies the connection owner's process through XRes. The test confirms
the forged property reached the native surface but did not consume the unrelated
child's pending launch; changing the pending launch to the verified owner then
acknowledges it. It passed with the real XWayland server (one test, 6.38 seconds).
This test requires XWayland and a writable `XDG_RUNTIME_DIR`, and must run alone
because production XWayland startup updates `DISPLAY`. The fixture restores that
environment variable and kills/reaps its owned child. Evidence:
`target/mcp-native-2026-09-10/launch-x11-native-results.txt`; scoped all-target
Clippy is recorded in `launch-x11-clippy.txt`. This closes the previously missing
forged-property launch acknowledgement check; output-constrained initial placement
remains open.

Compositor-owned local notification sockets are now nonblocking, including launch,
lock, shell state and capture-completion notifications. Existing send-error policy
retires slow subscribers instead of waiting for their datagram queues to drain.
A failed capture-completion send remains a delivery failure; it cannot stall the
owner. This protects input/revocation responsiveness during notification delivery.

The opt-in native launch test now decodes the actual subscriber event, checks the
generation/lineage fields, and proves no event is sent for ineligible or duplicate
observations. It also fills a receiver until a fresh sender gets `WouldBlock`,
then verifies owner notification returns before a one-second recovery drain,
removes the stalled subscriber and still delivers to a healthy subscriber. A
recovery thread bounds test failure time if blocking sends regress. The test passed
(`launch-subscriber-native-results.txt`); scoped all-target Clippy is recorded in
`launch-subscriber-clippy.txt`. This is a bounded subscriber-pressure check, not
completion of every compositor latency/stress gate.

An opt-in native owner regression now maps a real Wayland Zenity dialog into an
isolated `NickelSession` and explicitly controls identity delivery. It verifies
that a mapped window without identity waits, a verified but unmapped window waits,
and an unrelated verified process cannot satisfy the launched child's lineage.
Restoring both mapping and the matching verified process consumes the pending
acknowledgement once. The test uses the production owner/identity probe, without
adding an IPC privilege bypass, and kills/reaps its owned dialog on every exit.
It is ignored by default because it requires `/usr/bin/zenity` and a writable
`XDG_RUNTIME_DIR`; it was explicitly executed and passed here.

Evidence: `target/mcp-native-2026-09-10/launch-ordering-native-results.txt` and
`launch-ordering-clippy.txt`. This exercises owner acceptance with controlled
identity delivery, not every possible asynchronous wire interleaving. The separate
XWayland test above covers forged-property launch acknowledgement.
Output-constrained initial placement remains open.

Launch acknowledgement no longer consumes the forgeable X11 `_NET_WM_PID`
property. It now resolves a mapped window's current process incarnation from the
production identity worker (XRes ownership for XWayland, peer credentials for
Wayland), before applying the existing launch-lineage test. Pending/unavailable,
exited and stale process incarnations cannot acknowledge a launch. Identity results
that arrive before the first Wayland buffer do not acknowledge an unmapped window;
mapping retries after identity readiness, and identity completion retries after map.
This remains local launch feedback, not application identity or inherited control.

The focused launch suite passed, including a new real-process incarnation/exit
regression. Evidence is in
`target/mcp-native-2026-09-10/verified-launch-{results,build,clippy}.txt`.
Native forged-PID launch acknowledgement and both asynchronous ordering cases still
need acceptance coverage. This removes an unsafe prerequisite for output launch
association; it does not implement output-scoped launch or initial placement.

The strict Linux workspace Clippy gate (`--workspace --all-targets --all-features
-- -D warnings`) was rerun after the input/cache diagnostic changes. A broader
Windows cross-check uncovered a Linux-only desktop test importing the Linux
coordinator unconditionally. The test now has the correct platform gate; two
Linux-only bindings were also corrected. The desktop topology test still executes
and passes on Linux (`platform-boundary-results.txt`).

`cargo check -p nickel -p nickel-platform --all-targets --target
x86_64-pc-windows-gnu` now passes using the available clang/MinGW sysroot
(`windows-current-check.txt`). Strict Windows Clippy did not pass: the broader
build reports unused hosted/session APIs and screenshot fields, recorded in
`windows-current-clippy.txt`. These warnings were not suppressed. This is compile
coverage only, not native Windows execution or completion of its missing MCP
desktop owner. The final Linux strict Clippy result is recorded in
`workspace-current-clippy.txt`.

Input-recipient diagnostics now recheck current protection rather than relying
only on previously projected window/application records. Hosted keyboard focus,
native keyboard focus and native pointer recipient/hit mappings reject a newly
protected target. Hosted hit testing already performed this fresh check.

The production protection lifecycle regression retains old records, turns on
application protection before frame reconciliation, and verifies keyboard and hit
diagnostics become unavailable. Turning the flag off alone preserves that denial
until the protected frame is reconciled; afterward the ordinary target returns.
The regression passed (`input-protection-results.txt`), with scoped all-target
Clippy recorded in `input-protection-clippy.txt`. This is production-owner headless
coverage; the previous native hosted/native hit checks were not rerun for this
protection-read change.

Pointer hit diagnostics now also map ordinary hosted application surfaces to
their canonical diagnostic window. The mapping requires both current production
hit-test membership and an unprotected application already projected in this
snapshot. Protected, hidden and omitted applications remain unavailable; this
does not expose widget-level hit regions or hosted grab recipients.

The production internal-application lifecycle test passed after adding semantic
pointer input and assertions for mapped and omitted applications. Native Nickel
File checks verified its hit window, removal on minimize, restoration on activation
and continued exclusion of the trusted indicator. Scoped all-target Clippy,
formatting and diff checks passed. Evidence:
`target/mcp-native-2026-09-10/hosted-hit-{results,native-results,build,clippy}.txt`.

Input diagnostics now distinguish the ordinary native window under the pointer
(`pointer_hit_test.window`) from the routed pointer recipient. The projection uses
the production `pointer_surface_under` hit test and filters identities through the
authorized snapshot's windows. Unprojected/protected hits are unavailable; a native
hit with no input surface has a null window. No pointer coordinates or raw surface
identifiers are added. Internal hit testing and shortcut diagnostics remain open.

Native Wayland Gtk and XWayland xev checks both pressed in one window, moved over a
second while retaining the implicit grab, and observed the first as recipient and
the second as hit target. After release both matched the second window. Moving
over the trusted control indicator returned unavailable and exposed no protected
identity/geometry. Scoped all-target Clippy, formatting and diff checks passed.
Evidence: `target/mcp-native-2026-09-10/pointer-hit-{native-results,build,clippy}.txt`.
These are compositor hit/recipient observations, not X11 client-side grab queries.

Full-debug snapshots now project the shell's owned CPU image caches: launcher
icons, wallpaper, tray images and allowed window previews, as fixed entry/retained
pixel-byte counts. Preview membership is filtered before counting against the
ordinary windows already included in the authorized snapshot; omitted/protected
windows contribute neither entries nor bytes. No images, paths or cache keys are
projected. The synchronous read shares the snapshot generation/time and reports
`null` if the in-process shell is absent. GPU/shared/external renderer resources
remain explicitly unavailable.

The existing production cache-accounting regression now adds an excluded preview
and verifies unchanged scoped counts, including an empty allowed set. Native
full-debug reads matched local wallpaper diagnostics and the known retained
800x800 RGBA thumbnail (2,560,000 bytes) loaded from a repository fixture. The
isolated launcher rendered with an empty icon cache; this check does not claim
launcher-icon population or native protected-preview acceptance. Ordinary/paused
leases were denied. Evidence is under
`target/mcp-native-2026-09-10/shell-cache-{results,native-results,build,clippy}.txt`.

`cargo test --workspace` completed successfully across 88 test suites: 2,148 passed,
zero failed and 28 ignored (`workspace-current-results.txt`). This run includes the
recent graceful connection, origin audit, admission and lease-metric changes.
It does not prove the still-missing implementations or native platform gates.

The specification review also corrected the short full-debug approval preset to
30 minutes, retaining 20 minutes for ordinary control. This small correction was
made after workspace compilation; the two affected Settings approval tests passed
against the final source (`debug-duration-results.txt`). A native pointer click on
the visible “Allow 30 minutes” button granted the exact pending full-debug request
for 1,800 seconds and cleared its approval card. The test lease was revoked and
the isolated session cleaned up. Evidence: `debug-duration-native-results.txt`,
`debug-duration-settings.png`, and `debug-duration-clippy.txt` in the same artifact
directory. The existing custom, two-hour and until-logout choices remain available.

Lease metrics now use one production aggregate projection for the public endpoint
and the full-debug snapshot. It reports five fixed scope counts, active total and
pending approvals. Active counts use the actual lease authority's current-time
eligibility check, so paused, disconnected, revoked and expired leases do not count.
The diagnostic read uses `try_lock` outside the final authorization transaction,
returns `null` if busy, and carries its own compositor-relative observation time.
The subsequent full-debug check still gates delivery. No client/lease/resource
identity enters these aggregate fields. `lease_metrics` is no longer listed as an
unavailable diagnostic domain.

All 67 remote-control tests passed. Native diagnostic and public counts agreed
through approval, pause, resume, revocation, a pending approval and actual expiry.
The focused regression also checks disconnect/reconnect and deadline eligibility
before cleanup. Evidence is under
`target/mcp-native-2026-09-10/lease-metrics-{results,native-results,build,clippy}.txt`.

Full-debug snapshots now include the production HTTP admission collector as
`admission`: generation, collector-relative monotonic timestamp, admitted/rejected
request counts, and aggregate active global/authenticated request counts. This is
the same listener-local collector used by public operational metrics. No client
table, identity, label, peer address or request payload is projected. Sampling
uses `try_lock`; an absent/busy collector is explicitly `null` rather than blocking
the compositor. The subsequent lease-metrics change above completes both aggregate
projections; neither is now listed as an unavailable diagnostic category.

All 66 remote-control tests passed, including a contention test that requires the
snapshot to return unavailable while another thread still holds the collector
lock. Native full-debug observations counted their own active request and advanced
generation/time. An 80-request burst produced HTTP 429 responses and increased the
actual admission/rejection counters, with the local owner still responsive.
Ordinary and paused leases could not read diagnostics; public aggregate metrics
remained available and contained no client identity. Evidence is recorded under
`target/mcp-native-2026-09-10/admission-diagnostic-{results,native-results,build,clippy}.txt`.
This bounded burst does not establish the specifications' full latency/stress gate.

Verified connection-origin changes now have a bounded local audit: 128 records,
monotonic generation/time, the server-generated client identity, actual socket IP,
and TLS state. Unchanged peers coalesce; credentials, labels, forwarded headers and
request payloads are never retained. Revocation does not erase the retained history.
Settings shows the latest 16 records and the eviction count. This records peer
changes, not every socket opening or logical client lifecycle event; abrupt logical
disconnect detection remains separate unfinished work.

The 65 remote-control and 31 session-protocol tests passed. In an isolated native
session, authenticated requests from different loopback addresses populated the
local history, a repeated peer coalesced, wrong credentials added nothing, and forged
forwarding headers did not affect the recorded origin. A 140-change run retained
exactly 128 ordered events with eviction reporting, while the local response stayed
within its wire limit. History was absent from the agent diagnostic snapshot and
public metrics. Native Settings rows were visually inspected. Evidence:
`target/mcp-native-2026-09-10/connection-audit-{results,native-results,build,clippy}.txt`
and `connection-audit-settings.png`. These checks use local HTTP; previous verified
TLS origin tests still run in the remote-control suite.

The MCP `client_connection` tool accepts only `{ "action": "disconnect" }` or
`{ "action": "reconnect" }`, for the calling authenticated identity. It does not
accept a target client or require an active lease. A credential-bearing request
permit rechecks authentication, cancellation and its two-second delivery deadline
at the desktop owner; reconnect additionally rejects a locked/recovery session.
The owner reconciles held pointer/keyboard input, capture and traces before replying.
This is a graceful identity-wide transition, not automatic abrupt-disconnect
detection or closure of the HTTP transport. Clients sharing an identity share this
transition; individual HTTP response completion has no such effect.

The isolated Xvfb-backed native test observed a real Gtk client's drag release,
cleared keyboard ownership, cancelled pending approval, revoked a non-resumable
lease, resumed an eligible lease, rejected continuation of its old drag, retained
a local pause across reconnect and preserved another client's diagnostic authority.
A target-client argument was rejected. The 64 remote-control tests also cover
cancelled/expired permits, wrong credentials, lock rejection and missing request
execution scopes. Evidence is in `target/mcp-native-2026-09-10/` under
`graceful-connection-{results,native-results,build,clippy}.txt`.

In-flight MCP cancellation now reaches worker permits through a required request
execution scope. A transport cancellation, failed method, or dropped handler
cancels pending work; owner authorization rechecks it after acquiring authority.
Server permit construction fails closed if the execution scope is missing.
Successful completion detaches established standing operations from the SDK's
terminal-response token, while their lease generation and expiry remain enforced.
This distinction is necessary because the pinned SDK also cancels that token
after normal MCP 2026 replies.

All 63 remote-control tests passed (`request-lifetime-results.txt`). Native
nested testing stalled application preparation, reset the HTTP connection, and
unblocked the worker after 0.34 seconds: no process spawned, before the two-second
delivery deadline. A fresh launch succeeded under the unchanged lease
(`request-lifetime-native-results.txt`). A successfully started MCP 2026 frame
trace survived normal terminal-response teardown and recorded a repaint beyond
the initial request deadline (`request-lifetime-trace-native-results.txt`). These
files are under `target/mcp-native-2026-09-10/`. The owned process, FIFO, desktop
entry, leases, and session were cleaned up. These checks do not establish logical
client disconnect policy or rollback of an already accepted native effect.

`ControlPlane::disconnect_client` now centralizes a confirmed logical disconnect:
cancel only that client's pending requests, release its shared-input ownership,
retire non-resumable leases, and invalidate operations on retained resumable
leases. Duplicate notification is idempotent. Authenticated reconnection does not
restore pending cards, expired/revoked leases, cancelled gestures, or locally
paused authority. The graceful MCP operation now invokes these transitions
through the native owner and releases synthesized input before replying.
Automatic transport lifetime detection remains pending.

All 62 remote-control tests passed (`client-transition-results.txt`); the
extended focused regression also checks pause preservation
(`client-transition-regression-results.txt`). Coverage includes two-client
isolation, pending-card cancellation without denial cooldown, wrong credentials,
lease resumption policy, operation generation, and expiry. This is shared
authority coverage, not native disconnect acceptance.

Trusted indication now retains unexpired paused/disconnected leases and shows
their state separately from the active-authority count. Pausing no longer hides
the scope, peer, lifetime, or Stop control; it still denies remote operations.
Expiry and revocation remove the retained indication. The shared lease owner
exposes connection state read-only for this projection.

Four production command/protection tests passed (`paused-indicator-results.txt`),
including indicator persistence through pause/resume and removal on revocation.
The view state/height tests passed (`paused-indicator-view-results.txt`). Native
nested acceptance denied diagnostics while paused, restored observation after
local resume, visually verified the paused card with zero active authority, and
clicked Stop while paused to revoke the lease and disable the listener. Evidence
is `paused-indicator-native-results.txt` and `paused-indicator.png` under
`target/mcp-native-2026-09-10/`. Disconnected-state presentation has unit coverage;
this run did not prove native disconnect/resumption. The isolated session was
cleaned up.

Trusted indication now presents each active lease as separate wrapped client,
scope, peer/transport, and remaining-time lines. Full-debug scope is explicit.
The shared vertical scroll component exposes the full bounded lease list, while
the heading, count, Stop button, and emergency chord stay outside the scroll
viewport. The surface uses a bounded height that respects output margins.

Native nested testing with three leases verified the visible fields, full-debug
label, scrolling to the last lease, and unchanged authority during scrolling.
Clicking the fixed Stop button after scrolling revoked all three leases and
disabled the listener (`indicator-layout-native-results.txt`). Both
`indicator-layout-before.png` and `indicator-layout-scrolled.png` were visually
checked under `target/mcp-native-2026-09-10/`. Height-bound unit coverage passed
(`indicator-layout-results.txt`). Physical mixed-DPI and very small output
acceptance remain pending. The isolated native session was cleaned up.

The listener now supplies the accepted TCP peer and configured transport type to
client identity establishment and authenticated MCP requests. Each client keeps
its last verified peer IP and TLS state; forwarded-address/protocol headers are
ignored. The trusted local snapshot projects this in Settings and the active
indicator. Remote discovery summaries and public metric labels have not gained
peer addresses. This is current client metadata, not a connection-history audit
or a representation of every concurrent connection. Pairing clients obtain this
metadata on their first authenticated MCP request.

All 61 remote-control tests passed (`peer-origin-results.txt`), including real
HTTP and certificate-verified TLS sockets with forged forwarding headers. Native
nested testing sent an authenticated request from `127.0.0.2` and verified that
address and HTTP state, despite spoofed address/HTTPS headers. Both Settings and
the trusted indicator were visually checked in
`target/mcp-native-2026-09-10/peer-origin-settings.png`; the log is
`peer-origin-native-results.txt` there. The owned window and lease were cleaned
up and the session closed.

Full-debug method completion history now includes the first successful resource
authorization observed before completion: session-local numeric client audit ID,
lease ID, operation ID, and lease operation generation. The permit carries a
single-assignment correlation cell from the measured request to the production
worker/owner. It is scoped to that collector and survives thread dispatch;
rejected authorization does not populate it. A method's native outcome remains
separate, and the record does not prove presentation or delivery. Calls without
a successful resource-authorization boundary have no such correlation. Public
Prometheus exposition still contains only fixed-cardinality aggregates.

The 58 remote-control tests passed (`operation-correlation-results.txt`),
including worker-thread propagation, protected-target rejection, collector
isolation, and payload exclusion. Snapshot reads now return unavailable instead
of waiting on collector contention. Native nested testing correlated two repaint
calls with their lease and local permission-audit client and distinct operation
IDs; ordinary/paused diagnostic reads remained denied and public metrics omitted
correlation fields (`operation-correlation-native-results.txt`). The isolated
session was shut down with no live leases or windows. This does not complete
connection-origin correlation or the specifications' remaining diagnostic domains.

The follow-up collector contract tests passed (`correlation-lifetime-results.txt`):
interleaved futures keep distinct correlation cells, cancellation records the
authorization available at completion, and a later worker update cannot rewrite
that retained record. Filling and overflowing the 128-entry history with maximum
numeric correlation IDs preserves eviction order and keeps the serialized metric
snapshot below 64 KiB. These tests exercise collector concurrency and lifetime;
they do not establish native transport-disconnect or gesture-cancellation behavior.

Changed pending requests now carry compositor-owned warning flags for access
changes (resource, debug access, or reconnection policy) and duration increases.
Settings highlights both using the shared validation-status component. Flags
accumulate while the request is pending, including across equivalent requests
and reversions, and reset for a fresh request after a local decision. Until
logout counts as longer than a finite duration. Only two booleans are retained;
the comparison does not keep previous resource labels or request payloads.

All 56 remote-control tests passed (`request-change-results.txt`), including
accumulation/reset, duration ordering, and debug/resumption cases. Native nested
testing confirmed the local snapshot flags, persistence across reversion, stale
approval rejection, and zero granted authority. Both warning rows were visually
checked in `target/mcp-native-2026-09-10/request-change-settings.png`; the native
log is `request-change-native-results.txt` in that directory. The test request
was denied and the owned Settings/compositor session was closed. Older snapshot
payloads default to no changes (`request-change-protocol-results.txt`).

Local duration approval now offers 20 minutes, 2 hours, until logout, and positive
custom minutes in Settings. The trusted command carries the exact displayed
pending request separately from the locally chosen duration. Authority checks
still reject altered scope/debug/resumption/renewal identity, and zero/overflow
choices do not consume the pending request. Omitting the explicit duration field
is a protocol error; explicit null means until logout. Existing requested-duration
approval remains available. Custom text is transient and grants no authority by
itself.

Native nested acceptance verified all four duration choices against the active
lease deadline, rejected zero and changed requests, and clicked the actual
Settings **Allow 2 hours** button for a displayed 30-second request. The result
was a two-hour lease. Evidence under `target/mcp-native-2026-09-10/` includes
`local-duration-native-results.txt`, `local-duration-click-results.txt`, and the
visually inspected `local-duration-settings.png`. The initial click attempt
missed the fixture's window lifetime; the repeat first verified the Settings
window was live, clicked the visible button, and asserted the resulting lease.
The owned Settings window and leases were cleaned up. This does not establish
Windows or physical assistive-workflow acceptance.

Focused authority, Settings semantic-target, protocol, and control-disposition
tests passed (`local-duration-*-results.txt`). Native testing used the same
explicit duration payloads; the subsequent required-field parser check was unit
tested separately.

The local duration path also passed renewal acceptance
(`local-duration-renewal-native-results.txt`): a 30-second lease became a
locally selected 60-second lease, then until logout, retaining its ID and scope
while advancing renewal generation. A shortening choice left the request pending;
an old card could not approve the next pending renewal. The focused authority
regression (`local-duration-renewal-results.txt`) additionally verifies that these
renewals retain the existing shared-input owner. Native input held across this
particular custom-duration path has not been tested. The isolated session was
cleaned up with no active or pending leases.

- [Event subscriptions](mcp-event-subscription-acceptance.md): native delivery,
  scope/client isolation, capacity, disconnect, finite duration, listener restart.
- [Warning metadata](mcp-diagnostic-log-acceptance.md): native production failures,
  bounded retention, collection without payload fields, full-debug authorization.
- [Frame traces and local audit](mcp-frame-trace-acceptance.md): native nested
  dispatch, cancellation, lifetime, retention, client isolation, lifecycle audit,
  and trusted Settings rendering. DRM instrumentation is compile-checked.

## Workspace validation

A subsequent launch-worker review found blocking mutex acquisition in the
supposedly nonblocking worker snapshot. Snapshot and admission now use try-lock:
contention returns unavailable/busy immediately, while RAII cleanup still
releases admission and advances its generation. The two focused production-worker
tests passed (`worker-contention-results.txt` in the local artifact directory).
The contention test holds the state mutex while another thread attempts both
operations and requires the result before releasing that mutex. This is a worker
contract test, not native compositor acceptance.

The first `cargo test --workspace` run on 2026-09-10 stopped in the main Nickel
test suite. Its first failure expected hidden internal applications to disappear
from diagnostics; subsequent tests encountered the poisoned shared test mutex.
The production projection intentionally retains ordinary hidden applications
with explicit visibility and suspended-renderer state. The regression test now
checks that state, released keyboard focus, stable identity, and eventual removal.
The corrected test passed independently.

The fresh workspace run passed the main Nickel suite: 731 passed, 12 ignored.
It then stopped on four Codex fixture compatibility tests because the fixture's
schema omitted methods required by the checked-in protocol profile. Updating the
fixture schema resolved those failures; all 14 process tests passed. Evidence is
in `target/mcp-native-2026-09-10/workspace-implementation-results.txt` and
`codex-fixture-profile-results.txt`.

The remaining crates passed `cargo test --workspace --exclude nickel`, including
integration and documentation tests, recorded in `workspace-remaining-results.txt`.
That run required refreshing the source/reuse, Settings-control, display-list,
and consumer inventories to match the reviewed implementation. The storage
architecture check now recognizes shared staged writes and forbids direct renames;
the display-list check confines its new inspection exceptions to test code.
Together with the earlier main-suite run, this covers the default workspace test
scope. It does not cover ignored release-only measurements or missing native
platform acceptance.

The all-target/all-feature workspace Clippy run passed with warnings denied:
`workspace-clippy-results.txt`. `cargo fmt --all --check` and `git diff --check`
also passed.

## Windows identity foundation

`nickel-platform/src/process_identity.rs` queries a process with limited query
and synchronize rights, retains the owned handle, checks liveness with a zero
wait, and obtains package-family identity from the OS. No window title, class,
or self-reported AUMID can establish application inheritance through this API.
Unpackaged executable identity and publisher/signature evidence are still absent.
The future desktop owner must separately track HWND creation/destruction and
revalidate each window/process relationship; this probe grants no authority and
is not yet wired into MCP.

The probe also records the OS session ID while the retained process is live.
`is_unprotected_in_session` checks that session and re-queries Windows process
protection on every call. A failed query, protected level, or process exit makes
the process ineligible. This is only a necessary process check: desktop identity,
integrity boundaries, protected UI, and resource authorization still require
the future Windows owner. Session IDs alone do not identify a desktop.

The same probe exposes a fresh OS token integrity RID using a query-only token
handle and fixed-size storage. It validates the returned label pointer and
mandatory SID before reading the RID, closes the token on all paths, and rejects
process exit during the query. No token contents are serialized or logged.
Comparison against the owner's integrity level and enforcement remain part of
the missing Windows desktop integration; this evidence API grants no access.

The Windows target check, including its native current-process test, passed in
`windows-process-identity-check.txt` under the same local artifact directory.
Windows-target all-target Clippy also passed with warnings denied
(`windows-process-identity-clippy.txt`). The test is compiled only; execution
on Windows remains pending.

The session/protection extension also passed Windows-target all-target Clippy
with warnings denied (`windows-process-session-clippy.txt`). Its current-process
test now checks stable session identity, ordinary process protection, and
rejection of a different session. These assertions are compiled, not executed.

The integrity extension passed Windows-target all-target Clippy with warnings
denied (`windows-process-integrity-clippy.txt`). The compiled tests include
current-process integrity consistency and rejection of truncated or malformed
mandatory SIDs. Native Windows token and protected-process acceptance is pending.

## Settings runtime observation recovery

The Settings status refresh now removes pending approvals, live grants, and
pairing material when its runtime observation fails. It shows an unavailable
diagnostic and restores current controls only from a successful fresh snapshot.
This changes the local presentation; a failed Settings query does not revoke
authority in the compositor.

A real Settings window was tested in an owned nested Linux compositor on Xvfb
`:98`. A private datagram relay forwarded its normal session protocol and then
returned an error only for remote-control status queries before restoring normal
responses. All three observation phases completed, and the compositor retained
the original pending request throughout. Native screenshots show the approval
controls in the live phase and their removal with the unavailable diagnostic
during failure. Recovery was verified by successful fresh status queries; the
recovery screenshot shows the restored connected-client controls. The semantic
regression test separately verifies restoration of the pending approval action.

Evidence: `target/mcp-native-2026-09-10/settings-live-status-native-results.txt`,
`settings-status-live.png`, `settings-status-unavailable.png`, and
`settings-status-recovered.png`. The fixture terminated its Settings process,
logged out its compositor, and stopped its Xvfb server. This validates the nested
Linux path, not native Windows or physical output behavior.

## Shell renderer projection

Full-debug snapshots now include `shell_renderers` for the same visible,
unprotected ordinary shell surfaces listed in `shell_surfaces`. Each record
contains the production presenter's cumulative GPU/fallback work, raster buffer
bytes and reuse/import counters, configured policy and current path, surface
identity generation, and snapshot observation timestamp. These counters do not
claim presentation completion, GPU allocation accounting, or shared-cache scope.
Hidden surfaces and the locked shell are excluded by the production projection.

The owner regression and scoped Nickel all-target Clippy passed in
`target/mcp-native-2026-09-10/shell-renderer-diagnostic-results.txt`. Native MCP
acceptance against the rebuilt copied executable confirmed nonzero rendering
work for the launcher, correlated timestamps and identities, simultaneous 1.0
and 1.5 presenter scales on owned virtual outputs, removal after hiding/output
retirement, and denial after revocation. Results are in
`shell-renderer-native-results.txt`. The owned nested compositor and Xvfb were
stopped after the test. Physical output and Windows acceptance remain pending.

## Integrated watch expiry and emergency epochs

The combined patches passed all 85 remote-control tests and scoped all-target
Clippy with warnings denied (`target/mcp-native-2026-09-10/watch-emergency-integrated-results.txt`).
The emergency regression holds the authority mutex during an accepted effect,
triggers the atomic latch from another thread, verifies immediate permit rejection
without acquiring that mutex, and rejects the effect's late result. Re-enable
supersession and epoch exhaustion are covered. These checks establish authority
invalidation and result suppression, not rollback of an already-accepted native
effect or native Windows key/button release.


## Workspace integration checkpoint

The default workspace run passed Nickel's main suite (737 passed, 15 ignored),
then stopped at the strict source inventory because newly reviewed MCP/Windows
modules and the client executable had not been recorded. The inventory and module
ownership ledger were updated; its focused tests and
`cargo test --workspace --exclude nickel` then passed. Evidence:
`target/mcp-native-2026-09-10/workspace-mcp-integrated-results.txt` and
`workspace-mcp-remaining-results.txt`.

After that checkpoint, the lock-free Windows chord recognizer, sealed direct-script
launches and client saturation/cancellation follow-up were integrated. The updated inventory and all nine adapter tests passed in
`mcp-followup-integrated-results.txt`. That run then hit a stale dependency artifact
from a sibling checkout; cleaning the affected packages and rerunning produced
five passing Windows recognizer tests, sixteen passing process-identity/script
tests, and successful workspace all-target/all-feature Clippy with warnings denied
in `mcp-followup-fresh-results.txt`. The earlier workspace pass alone is not evidence
for these subsequent deltas. The script native acceptance and explicit interpreter
/path-sensitive compatibility limits are recorded in
`/tmp/nickel-mcp-script-report.md`. Client EOF/cancellation saturation uses actual
loopback HTTP streams; remote TLS and quiet native connection maintenance have
separate adapter evidence. No Windows native execution is claimed.


## Local audio and shared indicator integration

The local cue selector and bounded playback worker are integrated, with a protected
local Settings preference. Agent validation covered all five cues using private
PipeWire-Pulse and a null sink; physical acoustic gain/mute and Windows playback
remain unverified. See `mcp-audible-indications.md`. Native fixtures must disable
`audible_indications` before launch or use their own dummy audio service.

The production indicator Application is shared between platform hosts. Its UiHost
semantic tests exercise labels and local Stop activation. Windows now has hidden,
retained owned-window creation and display-affinity checks; these do not establish
UIA registration, persistent visibility or complete capture exclusion. Windows
approval remains denied while that owner integration is incomplete.

The integrated audio/indicator checkpoint passed 88 remote-control tests, four
indicator tests, two audio tests (the explicit native audio test stayed ignored
in this host-safe run), the Settings audible-preference test, all three source
reuse checks, formatting and diff checks. Evidence:
`target/mcp-native-2026-09-10/audio-indicator-integrated-results.txt`.
The numbered acceptance audit is tracked in `mcp-verification-gates.md`; all 24
items remain open at their full specified scope.
Full-workspace all-target/all-feature Clippy also passed with warnings denied after
this integration (`audio-indicator-clippy-results.txt`).


## Shell capture, semantic observation and appearance integration

`list_surfaces` and `capture_surface` now use canonical generation-bearing ordinary
shell identities, fresh production-host protection, whole-surface output membership,
and the existing fenced renderer/PNG capture path. Each asynchronous capture stage
rechecks the original permit and resource. Native capture acceptance is being
completed in `/tmp/nickel-mcp-surface-native/` before final acceptance claims.

`inspect_surface` reads the real shell UiHost or retained output viewport without
changing focus or rebuilding its tree. Surface/tree generations qualify the bounded
node ordinals. The shared conversion also serves hosted-window inspection. Protected,
hidden and retired surfaces are denied through the capture resource resolver. Shell
semantic mutations remain separate unfinished work; external accessibility is not
implemented by this observation path. Targeted owner tests passed for visible focused
launcher semantics, stable observation generation, hidden denial, and capture resource
protection/retirement/output membership (`surface-semantics-tests.txt`). That command
later failed compiling an unrelated metrics test tuple introduced during integration;
the tuple is fixed and combined checks are running.

Appearance read/transactions are integrated from the reviewed agent patch. They
use typed ShellSettings values and bounded staging, preserve unrelated fields,
validate observed generation/prior/file revision, and recheck cancellation,
emergency epoch and deadline immediately before rename under existing authority.
The same commit-boundary recheck now covers shell behavior. Production LiveShell
reconciliation applies committed preferences even if cancellation races the already
accepted rename; cancelled results do not claim successful remote completion.
The external-writer race between final metadata check and rename remains documented
in `crates/nickel-remote-control/APPEARANCE.md`.

Agent native appearance tests passed before integration, including full-debug versus
ordinary-lease admission, held-input denial, stale/external-write rejection, visible
palette change, restoration and revoked read/write denial. The light-theme trusted
indicator contrast defect found in those captures is being fixed separately. Windows
appearance owner/native execution and OS-wide appearance publication remain missing.

The combined integration passed 89 authority tests, three appearance tests, two
settings staging tests, source inventory checks and full-workspace all-target,
all-feature Clippy with warnings denied. Evidence:
`appearance-surface-integrated-results.txt`. This supersedes the earlier metrics
merge compilation failure, without claiming a full new workspace test run.

The indicator contrast fix is integrated: all ordinary labels use the theme's
primary text color. Seven production UI tests passed, including actual light/dark
paint colors, contrast, DPI sizing and pointer Stop after resize. Inactive panel
host protection is now included conservatively alongside the active host before
capture or semantic observation. A new native semantic fixture is underway.
The indicator follow-up also passed integrated all-target/all-feature Nickel Clippy
with warnings denied and formatting (`indicator-contrast-integrated-results.txt`).

## Native shell observation and Windows host checkpoint

The combined owned native shell fixture completed successfully, including visible
focused search/Home semantics, repeated tree stability without moving focus,
single-surface/output isolation, fresh hidden denial, distinct output panel hosts,
rotated 1.25-scale capture, same-name output retirement, revocation, and lock denial.
The negative cross-output assertion used a still-active primary-output lease;
locked HTTP 401 is distinguished from an MCP resource denial. Evidence:
`shell-capture-semantics-native-results.txt` and
`shell-capture-semantics-native-report.md` under the dated artifact directory.
Root inspected a native launcher PNG. Owned compositor/Xvfb were stopped; their
copied binary and fixtures remain in `/tmp/nickel-mcp-surface-native/`.
This binary predates the indicator contrast fix and later input diagnostic fields.

Windows now reconciles actual per-output indicator UiHosts and routes local events
and controller actions to them. The host presents before native exposure, retains
its owned HWND, establishes opaque layered-window/display-affinity state, and
cancels authority before failed indication/topology teardown. Integrated Windows
all-target cross-check passed with the existing screenshot dead-code warning
(`windows-indicator-integrated-results.txt`). Native Windows drawing, persistence,
UIA and capture-API exclusion are still unproven; approvals remain disabled.

Input diagnostics now distinguish an ordinary shell keyboard recipient and current
pointer hit by generation-bearing surface identity. They resolve current protection
and visibility through the same production resource evidence as capture. A hit is
not reported as a grabbed pointer recipient. Lock/recovery hides the complete
input projection. The focused-launcher/lock owner regression and full-workspace
all-target/all-feature Clippy passed (`shell-input-diagnostic-results.txt`);
separate native input-diagnostic validation is underway.

## Result cancellation and live shortcut diagnostics

A reproduced shared-permit bug returned success when transport cancellation arrived
inside an accepted effect. The regression failed before the fix
(`permit-result-cancellation-before.txt`). The shared authority now rechecks atomic
cancellation, emergency epoch and admitted deadline before accepting the effect and
before returning its result, and confirms ready connection presence at delivery.
The admitted deadline includes the latest currently ready watch expiry as well as
the request and lease deadlines, so staged commit checks cannot outlive connection
presence while the authority mutex is held. Failure releases the logical input
reservation; native keyboard/pointer owners have error cleanup for accepted presses.
This does not undo an already accepted native effect. All 89 authority tests and
scoped Clippy passed (`permit-result-cancellation-results.txt`), including cancellation
within the callback and watch expiry during an accepted callback with reservation
release. These timing regressions are deterministic owner tests, not physical-key
native acceptance.

Native shell input diagnostics passed on the owned nested compositor: the focused
launcher and pointer hit identify its current surface, expose no invented window
recipient, and do not confuse hover with a captured-pointer recipient. Repeated
inspection preserves focus; hide, actual protected screenshot interaction and lock
remove or deny the projection. Evidence: `shell-input-native-results.txt` and
`shell-input-native-report.md`. Owned native processes were stopped.

The shortcut diagnostic projection is integrated from actual compositor registration
state, capped at 128 inspected entries. It includes registration revision and the
containing observation point, projects only fixed physical binding/action metadata,
and reports string-bearing bindings as unprojected. Input edges do not change the
registration revision; counter exhaustion is explicit. No held-key or emergency
tracker state enters the projection. Combined checks and native validation are
underway. The remaining typed settings/action inventory is in
`mcp-settings-domain-inventory.md`; it remains an implementation checklist.
The combined shortcut/permit integration passed the registration-revision test,
two bounded projection tests, all 89 authority tests and full-workspace all-target,
all-feature Clippy with warnings denied (`shortcut-permit-integrated-results.txt`).
A further review found that ConnectionWatch::close waits for the authority mutex
before making transport loss visible; an atomic invalidation fix and contention
regression are now being implemented. The expiry/cancellation result fix does not
by itself prove immediate watch-loss visibility during an accepted effect.

Native shortcut diagnostics passed against the real owned compositor: an ordinary
lease is denied while full-debug returns the live bounded registration table with
correlated generation/time. Actual launcher typing and a held Shift leave static
registrations and their revision unchanged; exact field allowlists contain no typed
payload, key-event history or held state. Revocation and lock deny delivery.
Evidence: `shortcut-native-results.txt` and `shortcut-native-report.md`; the fixture
is `/tmp/nickel-mcp-surface-native/shortcut-test.py`. Owned native processes stopped.

The subsequent workspace checkpoint passed all 759 main Nickel tests (16 ignored),
then exposed two missing audit entries: the audible-indications Settings switch and
test-only paint inspection in the trusted indicator contrast test. Both entries
are now recorded; the latter admits no production display-list authority. The
remaining workspace run stopped at that audit, so this is not a full workspace
pass. Logs: `workspace-surface-shortcut-results.txt` and
`workspace-surface-shortcut-remaining-results.txt`.

Connection-watch atomic cancellation is now integrated. Each dispatch retains the
original ready-watch incarnations in a typed commit boundary; transport closure or
watch request cancellation becomes visible before mutex cleanup. Boundary checks
before staged commits and result publication reject lost authority. Agent validation
passed 92 authority tests and reproduced the erroneous successful result with the
fix removed. Root integration passed all 92 authority tests, seven declarative
authority audits, Nickel documentation tests and strict Nickel/remote-control
all-target/all-feature Clippy (`watch-atomic-integrated-results.txt`).

The authority-nonblocking followup is now integrated: watch polling uses shared
readiness, incarnation, expiry, lifetime and emergency state; close invalidates
before attempting mutex cleanup. Graceful owner reconciliation removes dead
registrations before native input reconciliation and reply. Abrupt cleanup still
uses the normal owner tick. Its isolated 94-test authority/HTTP run passed; native
held-input release measurement and combined root checks remain underway.

Windows trusted indication now registers an AccessKit UI Automation provider on
the actual retained hidden HWND before exposure. Its bounded read-only tree comes
from the production UiHost, and the sole admitted action forwards the current local
Stop target to that host on the owner thread. Retirement invalidates actions before
detaching the subclass and destroying the window. Five executable projection/action
tests and agent Linux/Windows compile checks passed. Root combined checks are in
`watch-uia-integrated-results.txt`; native Windows execution remains unavailable and
the Windows approval gate remains closed pending the other native authority owners.
The combined run passed all 94 authority tests and five trusted-accessibility
tests, then stopped because the root command named a nonexistent inventory test
target. The corrected `nickel-core --test reuse_authority` audit passed all three
tests, and strict Nickel/remote-control all-target/all-feature Clippy passed in
`watch-uia-audit-clippy-results.txt`. The original command error was not a test
failure and does not make the unfinished workspace run a full pass.

The application-scaling agent patch is deliberately not integrated yet. Independent
review found that its journal writer lacked file/directory durability, the local
Settings path could race remote transactions, and current-value equality did not
prove an uncertain external setter had completed. Helper spawn also did not fence
the later native mutation. Corrections are underway in the shared storage layer
and typed GTK/Qt backends. Earlier native GTK/Qt success is retained as evidence of
the tested workflow, not proof of these concurrency and cancellation requirements.

The integrated Windows UIA owner/provider passed the GNU Windows all-target
cross-check (`windows-uia-integrated-crosscheck-fresh.txt`), retaining the existing
screenshot-tool dead-code warning. The first attempt used a stale shared-cache
`nickel-input` artifact missing an API present in source; refreshing all primary
Rust source timestamps inside the serialized build lock produced the coherent
passing result. This is compile evidence only, not native UIA execution.

Native watch-loss acceptance passed for the atomic and authority-nonblocking watch
snapshot in an owned X11 session. Real xev events and independent XQueryPointer /
scoped XInput device-state queries prove held button/key release on last-watch TCP
abort, overlap preservation, and release before successful graceful disconnect
reply. Later text is denied and does not reach the client. Observed abrupt release
samples were 0.82 ms (button) and 0.84 ms (key), not worst-case bounds. Evidence:
`watch-x11-native-report.md` and `watch-x11-native-results.txt`. All owned processes
were cleaned up. Physical backends, Windows, saturated queues and a deliberately
stalled native owner callback remain outside this fixture's proof.

The resumed workspace run reached the workbench consumer inventory, then failed
because the indicator's test-only paint exception lacked an owning consumer row.
The shell-runtime row now owns that exception and cites its production-host tests;
the corrected consumer inventory test passes (`indicator-consumer-audit-results.txt`).
The larger run is recorded in `workspace-watch-uia-remaining-results.txt`; remaining
targets and documentation tests after its stopping point still require completion.

The revised shell semantic action checkpoint passed native origin-output and
affected-overlay scope tests, including denial of another output's launcher and
Control Center dismissal. Its reviewed patch is integrated in primary; all 95
authority tests and two production-owner semantic tests passed, followed by strict
Nickel/remote-control all-target/all-feature Clippy
(`shell-semantic-actions-integrated-results.txt`).
It includes search/navigation and guarded visibility effects, with the remaining
native launch/device effects still under implementation.

Native cross-client arbitration passed on the atomic/nonblocking-watch snapshot.
One client's held key or drag rejects another client's keyboard/pointer/focus
mutations while permitting read-only inventory; ending or revoking the hold admits
the second client. Actual parent-Xvfb input reaches winit and the Xwayland core
keyboard, and its Control hold survives remote isolated-key revocation. Moving
an output-scoped drag target to a second virtual output releases the native button
and denies continuation. Evidence: `arbitration-x11-native-report.md` and
`arbitration-x11-native-results.txt`. Owned resources were cleaned. These results
do not establish physical backend, Windows or saturated-queue behavior.

Durable storage is integrated; all 12 storage tests, documentation tests and strict
all-target storage Clippy passed in `durable-storage-integrated-results.txt`.
The reviewed patch passed 12 storage tests, Linux and Windows strict cross-Clippy,
and an owned Linux syscall trace proving preparation-worker file sync, owner rename,
then completion-worker directory sync. Windows durable staging remains explicitly
unsupported; the existing ordinary writer is unchanged. Scaling callers still
need their own corrected transaction integration and native validation.

Windows retained executable evidence is integrated. Its five pure policy/lifetime
tests, three source-reuse audits and strict platform all-target Clippy passed
(`windows-evidence-integrated-results.txt`). The kernel mapped-image verifier is
shared with local transport. An opaque clone retains the file pin after process
exit; no raw file-ID grant escapes it, and executable equality alone does not
establish normalized application membership. The owner-held catalog/launch-receipt
registry remains under implementation. Native Windows process/pin validation is
still unavailable; the agent's Windows cross-check is compile evidence only.

The remaining declarative-macro targets, workspace documentation tests and workspace
format check passed (`workspace-watch-uia-final-targets.txt`). Together with the
recorded consumer audit correction, this finishes the targets skipped when the
earlier workspace run stopped. It does not replace a fresh full-suite checkpoint
after subsequent feature integration; main Nickel's larger suite and later
changes retain their separately stated validation scopes.

The reviewed owner-wakeup patch is integrated; root authority and production-owner
tests followed by strict all-target/all-feature Nickel/remote-control Clippy passed
in `watch-owner-wake-integrated-results.txt`. Its agent validation passed 19 watch
tests, three production-owner tests, strict Linux Clippy and Windows cross-check.
The dedicated signal bypasses ordinary request capacity; failed setup preserves
shell startup and periodic fallback. Native saturated-queue release testing is
underway and has not yet been established by those unit/compile results.

The revised semantic installed-launch continuation is integrated. It retains and
revalidates the invoking output, reports post-first-step errors as partial
completion, and sizes launcher and Control Center hosts to their destination.
Quick Settings follows its production panel anchor. Root integration checks passed
two production semantic/geometry tests, all three source inventory tests, and
strict all-target/all-feature Nickel Clippy (session 19025, terminal exit 0).
The final agent native fixture at
`/tmp/nickel-mcp-shell-action-native/results.txt` proves secondary-output X11
launch with the pointer on primary, secondary-output-only Quick Settings creation
and inspection, narrow-scope denial, held-input denial and watch cancellation.
These checks do not establish all remaining shell semantic effects or Windows.

Native owner-wake acceptance passed against the integrated implementation. While
only an owned compositor's main thread was paused, 32 real read-only RPCs filled
its production queue and four further calls returned the actual queue-busy error.
Watch loss denied new permission requests immediately; native held input remained
until owner resumption, then real key/button release and device-state cleanup
occurred. A graceful HTTP reply waited for native release. An idle abort also
released input without ordinary RPC traffic. Evidence:
`watch-owner-wake-native-report.md` and `watch-owner-wake-native-results.txt`.
All owned processes, trace attachments and sockets were cleaned up. Timings are
samples, and ready periodic timers prevent attributing the saturated-run timing
solely to the dedicated wake; production-owner tests separately prove its priority.

The regular-file reader and shared bounded D-Bus transport are integrated. Root
validation passed 16 storage tests, nine transport tests, three inventory checks
and strict package Clippy (`regular-storage-integrated-results.txt` and
`bounded-dbus-integrated-results.txt`). The transport bounds authentication and
frames before zbus allocation and rejects received descriptors; the file reader
rejects nonregular files without waiting for a FIFO writer.

Scaling and guarded device controls remain outside primary while review findings
are corrected. Scaling must preserve the engine's expected external value through
native preparation, distinguish definitely unsubmitted failures from uncertain
acceptance, and retain worker observation timestamps. BlueZ discovery must retain
its sender-owned connection until stop or cancellation; a boolean-property mock
does not prove that lifecycle. Guarded PipeWire metadata proxy admissions also
need a total bound under registry churn. Earlier native backend successes do not
supersede these findings or establish complete MCP acceptance.

Transport review also found that one poll of zbus send can perform several native
socket writes. Both pending setter consumers therefore need a shared check at
each write, with a partial-write cancellation regression; a single check before
polling does not prove continuous authorization throughout the send. The current
shared transport's framing tests establish receive bounds, not this mutation
boundary. Neither consumer is integrated until this is corrected.

The complete workspace checkpoint after launch-v2 passed: `cargo test --workspace`
and `cargo fmt --all --check`, root session 58023, terminal exit 0. Evidence:
`workspace-launch-v2-results.txt`. Main Nickel ran 769 passing tests with 16
explicitly ignored native/benchmark cases; the remaining workspace and
documentation suites completed successfully. This checkpoint includes the shared
regular reader, bounded D-Bus receive transport and launch-v2, but excludes the
pending scaling, guarded device, Windows registry, external accessibility and
output-identification patches. Ignored tests are not native acceptance evidence.

Launcher preference loading now uses the shared bounded regular-file reader,
preserving missing-file errors, empty defaults, UTF-8 validation and both complete
lists at the existing maximum ID size. Root session 27924 completed successfully:
five launcher preference tests, three source inventory tests and strict core
all-target Clippy (`launcher-preferences-bounded-results.txt`). This closes the
unbounded file-read gap; typed remote favorites transactions remain outstanding.

The Windows application registry is integrated. It reuses installed shortcut
discovery and retained process/image evidence; receipts reject processes created
before invocation, and catalog snapshots retain worker observation intervals.
Catalog reads do not acquire launch ancestry pins. Root integration validation
passed seven registry policy tests, three inventory checks and strict all-target,
all-feature Nickel Clippy (`windows-registry-integrated-results.txt`, session
86095, exit 0). Agent Windows cross-checks passed with the recorded Nickel
dead-code warnings; strict platform cross-Clippy passed. Native Windows launch,
path pinning and write-sharing tests remain unexecuted, so Windows approval gates
remain closed. Native window/output resource-owner implementation is next.

Both shell-settings loaders now share bounded regular-file reads. A child-process
regression reproduced the original FIFO hang and killed the blocked probe after
two seconds (`/tmp/nickel-shell-settings-read-before.txt`). Root integration
session 11650 completed successfully: the shell-settings suite, appearance
transaction tests, shell-behavior preparation regression and strict core Clippy
passed (`shell-settings-bounded-integrated-results.txt`). Missing-file and version
semantics are preserved. The change also closes replacement-to-FIFO races between
MCP preparation's preliminary metadata check and its subsequent content read;
ordinary disk I/O still depends on the operating system.

Scaling remains held for review corrections. The typed rejection draft fixed the
external-prior rebase, but its reply timeout/disconnection path still converted a
possibly accepted write into `NotAccepted`. That path must return `Uncertain` and
retain its journal intent; dedicated receipt-loss regressions and removal of
blanket string-to-rejection conversions are underway. Earlier toolkit tests do
not establish this failure path.

The final scaling corrections are applied for integrated validation. Native
preparation carries the engine's original expected value, definitely unsubmitted
failures may clear their intent only while still authorized, and timeout or lost
completion receipts remain uncertain. Toolkit observations carry their worker
start/completion interval and are explicitly non-atomic. The isolated agent run
passed 24 focused tests and a private real-dconf/Qt fixture, including a delayed
read and lost-receipt no-retry case (`/tmp/nickel-scale-final-review-report.md`).
Primary-checkout results are recorded separately once the current run completes.

The checked D-Bus write transport is now integrated. Root session 16431 exited
successfully after 14 native socket tests and strict platform all-target Clippy
(`guarded-dbus-integrated-results.txt`). It checks original authority immediately
before each nonblocking write and excludes other frames across partial sends.
Failed/incomplete sends permanently retire the dedicated transport; typed errors
distinguish zero accepted bytes from uncertain prefix acceptance. Tests include
actual successful short writes, revocation between them after the peer drains the
first prefix, ordinary-frame exclusion, authentication and receive bounds. This
proves the shared transport boundary; device/scaling consumer completion and
standing lifecycle validation remain separate integration requirements.

Typed output identification is integrated. It reuses the production badge raster
and lifetime, scopes a remote request to one exact output incarnation, and keeps
trusted controls above the badge. A retained timer token bounds replacement churn;
generation ownership prevents revoked remote cleanup from hiding newer local
labels. Root session 18129 passed five owner tests, wire validation, source
inventory checks and strict affected-crate Clippy
(`output-identification-integrated-results.txt`). The owned winit fixture compared
actual secondary-output pixels with the local production badge and passed exact
output filtering, revocation, abrupt-watch cleanup and local replacement
(`/tmp/nickel-mcp-watch-x11/identification-results.txt`). Physical DRM remains
unavailable for safe isolated acceptance.

The scaling, guarded-device, shell-device, launcher-favorites, Windows
resource-observation and native-accessibility handoffs are integrated in the
primary checkout. Scaling passed 24 focused tests, real private dconf/Qt
transactions, source inventory and strict Clippy. The guarded-device stack passed
the vendored PipeWire suite, 99 remote-control tests, focused Linux lifecycle and
semantic-action tests, source inventory and strict Clippy
(`device-stack-integrated-results.txt`). Favorites passed its core schema,
remote transaction, shell reconciliation and source-inventory tests plus strict
combined Clippy (`favorites-integrated-results.txt`). Windows observation passed
four portable owner tests and Linux/Windows cross-Clippy
(`windows-resource-integrated-results.txt`); native Windows execution remains
unavailable. Accessibility passed four authority tests, source inventory and
strict Clippy (`accessibility-integrated-results.txt`). Its native delayed-click
revocation and remote-menu retirement checks passed. A follow-up fixed internal
context-menu focus and normalized input routing, and moves menus away from the
trusted indicator without lowering trusted chrome. Replacement-menu Escape,
physical semantic activation, stale-revocation survival and committed-action
retention now pass in the owned GTK fixture
(`gtk-menu-escape-native.txt`, `gtk-menu-input-native.txt`). GTK-shell
advertisement is enabled. The exact source inventory now contains 332 Rust
sources.

A fresh post-integration checkpoint passed `cargo fmt --all --check`,
`cargo test --workspace`, the vendored PipeWire suite and
`git diff --check` (`workspace-post-integration-results.txt`). The diagnostic
snapshot no longer reports the integrated shell semantic-action domain as
unavailable. External accessibility remains an independently bounded on-demand
observation and is labelled as not embedded in the coherent snapshot rather than
reported as wholly absent.

Wallpaper settings now use the shared bounded regular-file reader and a locked,
revision-checked staged writer. Full-debug exposes a typed, path-free observation
and transaction for position changes and custom-image reset; the schema neither
accepts nor returns image paths or contents. Preparation stays on the bounded
settings worker, and the desktop owner rechecks lease, emergency epoch, request
lifetime, deadline and shared input immediately before rename, then requests the
production shell reload after an accepted commit. Portable schema, size, UTF-8,
cancellation and file-ABA tests pass. Approved native image selection, pixel/decode
budgets and native visual acceptance remain open. The exact source inventory now
contains 334 Rust sources.

Terminal settings now reject nonregular, changing, oversized and non-UTF-8
configuration transport through the shared 64 KiB reader. Full-debug exposes a
complete typed presentation transaction for font family/size, scrollback, cursor,
colors and close-on-success. The path-free schema reports only whether a custom
shell or initial directory exists and preserves those hidden launch-policy values
through locked staged revision CAS without retaining them in remote generation
state. The desktop owner rechecks authorization,
request lifetime, emergency epoch, deadline and shared input immediately before
rename. Results explicitly apply to newly created terminals; existing terminals
are unchanged. Portable schema, bounds, cancellation, hidden-field and file-ABA
tests pass. Native terminal rendering acceptance and a safe typed launch-policy
contract remain open. The exact source inventory now contains 336 Rust sources.

Optional-feature preference and runtime persistence now share the 64 KiB regular
file reader and reject nonregular, changing, oversized and non-UTF-8 transport.
Preference updates use the shared cross-process transaction lock without event-loop
polling, sleeps or stale-lock deletion, while retaining the existing atomic whole-file
writer and preserving unrelated Codex/keyboard fields. All 14 optional-feature
tests, the storage reuse audit and strict core Clippy pass. This is prerequisite
storage work; the typed OSK preference transaction and runtime acknowledgement gate
remain open.

Full-debug now exposes the typed Automatic/Enabled/Disabled OSK preference through
the production optional-feature writer. Preparation preserves every Codex field;
the desktop owner checks fresh file revision, preference generation, lease,
emergency epoch, request lifetime, deadline and shared input at commit. The schema
rejects visibility, docking, height, environment override, recipient, epoch and
text/input controls. Results include only coarse effective enablement, touchscreen
presence, override status and runtime generation, with `pending` until the existing
LiveShell configuration path acknowledges the committed generation. Portable
schema, cancellation, local-replacement, preservation and acknowledgement tests
and strict combined Clippy pass. Native touchscreen/override/teardown acceptance
remains open. The exact source inventory now contains 338 Rust sources.

All ShellSettings writers now use the shared stable transaction lock. Appearance
and shell-behavior preparation retain that lock from their bounded read through
the final revision-checked rename, so cooperative local settings writes return a
bounded busy result instead of crossing the remote transaction boundary. The
revision check still rejects uncooperative file replacement and ABA. Focused
local/remote exclusion, cancellation, expiry and ABA tests and strict combined
Clippy pass.

Full-debug now exposes file-icon provider and theme settings through a typed,
path-free transaction. The installed catalog is platform-owned, deduplicated and
bounded to 256 IDs of at most 128 bytes. Only an exact available catalog ID may be
selected; Nickel and system-default choices remain explicit. Missing configured
themes remain visible as unavailable so temporary removal does not erase intent,
while malformed path-like legacy values remain preserved internally and are not
disclosed. Observation generation includes settings revision, bounded catalog and
provider-content revision. The shared ShellSettings lock and desktop owner enforce
revision CAS, lease/emergency/deadline and idle-input checks before rename, then
request the production cache reload without claiming pixel presentation. Portable
schema, bounds, preservation and ABA tests pass. Native presented-icon acceptance
remains open: this host has no Xvfb, its Wayland socket rejected nested EGL display
creation, and nested X11 did not reach the authenticated test-control readiness
barrier even with Mesa/software rendering. Both owned attempts were stopped before
lease approval or mutation (`file-icons-native-results.txt`). The exact source
inventory now contains 340 Rust sources.

Full-debug now exposes Codex enablement as a typed, policy-aware preference without
exposing or accepting executable paths, source labels, credentials, accounts,
projects, threads or backend payloads. The transaction preserves the existing
source and all on-screen-keyboard fields, uses the shared bounded optional-feature
staging and revision CAS, and rechecks lease, emergency epoch, request lifetime,
deadline and shared-input idleness at the final rename. The compositor owner
directly creates or tears down its existing internal Codex host and hidden project
menu, reports runtime generation and pending state, and refuses disablement while
chat windows are active. Source selection remains unavailable pending a verified
source authority. Portable schema, cancellation, preservation and file-ABA tests,
plus a real in-process compositor test of host creation, hidden-menu ownership and
complete teardown, pass. Presented-pixel and active-chat rejection acceptance remain
open because this host's nested backends cannot reach authenticated test-control
readiness and the running user session has not been reloaded. The exact source
inventory now contains 342 Rust sources.

Full-debug now exposes bounded idle dim and suspend preferences through a typed
transaction while excluding the security-sensitive lock timeout, inhibitor
ownership, authentication, shutdown/restart and listener controls. Preparation
uses the shared ShellSettings lock and revision CAS; the desktop owner rechecks
lease, emergency epoch, request lifetime, deadline, protected focus and complete
shared-input idleness at the final rename. It then replaces the production
`IdleController` policy, starts a fresh idle interval so a shortened timeout cannot
immediately suspend an active session, and undims an already dimmed compositor.
Snapshots distinguish configured and applied values with a generation and pending
state. Portable controlled-clock, schema, cooperative-writer, replacement,
protected-field preservation and acknowledgement tests plus strict combined
Clippy pass. Native compositor dimming and real suspend acceptance remain open;
no test may suspend the user's active session. The exact source inventory now
contains 344 Rust sources.

Full-debug now exposes a path-free application inventory refresh through the
production platform discovery, launcher/panel reconciliation, Linux run-signature
publisher, and launcher icon-cache owner. One diagnostic worker admits the scan;
entry, application, aggregate metadata, and Windows recursion bounds prevent an
unbounded catalog from reaching the compositor. Authorization is checked before
and after preparation and again while the compositor reserves shared input at the
final commit; active physical, shell, remote-held, touch, controller, or grab state
rejects reconciliation. The outcome reports its own generation, preparation time,
accepted count, partial status, and owner reconciliation without claiming rendered
pixels. Schema, scan-bound, launcher reconciliation, Linux compile, formatting,
and focused tests pass. A read-only native Linux scan of this host's production
application directories completed within every catalog budget. Native cancellation
and presented launcher/icon acceptance remain open. The installed Windows Rust target is currently insufficient for this
slice because `x86_64-w64-mingw32-gcc` is unavailable, and no native Windows
execution is claimed. No source files were added; the exact inventory remains 344.

The safe platform re-query action now has exact `connectivity`, `audio`,
`peripherals`, `maintenance`, and `default_associations` domains.
Linux queues acknowledged immediate NetworkManager/BlueZ or PipeWire observations
on their existing production workers; requested preparation cannot publish or
replace shell state. Windows uses its existing native status readers on the bounded
diagnostic worker. Device, access-point, saved-profile, BlueZ-object and audio-output
counts and retained IDs/labels are capped, with truncation reported as partial. After preparation the
compositor rechecks full-debug authority, reserves shared input, rejects active
physical, shell, touch, controller, grab or remote-held interaction, advances its
own generation, and reconciles only the fresh returned snapshots. The outcome
contains coarse availability, timing, generation and reconciliation without SSIDs,
device names, paths, credentials or a pixel-presentation claim. Schema, bounds,
prepublication and native Linux connectivity and PipeWire production-worker tests pass. Peripheral
refresh reuses the production service after bounding its helper process groups to two seconds and
64 KiB per output stream. The private snapshot is reduced before owner delivery to availability,
counts, and partial status, excluding printer/job identities, volume and filesystem paths, provider
details, and errors; it makes no compositor reconciliation or presentation claim. Native Linux
timeout, output-flood, production-service refresh, and redaction tests pass.
Maintenance refresh reuses the production service after routing Linux PackageKit
and firewall helpers through the same bounded runner and Secret Service through
the bounded authenticated D-Bus transport. It returns update/restart facts,
optional firewall and malware health, a known-permission-state count, secure
storage observation availability, and partial status. It excludes provider and
distribution names, permission identities, errors, commands, and credential
state. The native Linux production backend and redaction tests pass.
Live delayed revocation and Control Center presentation acceptance and native Windows execution
remain open. The diagnostic returns unavailable on Windows rather than invoking
the maintenance service's unbounded PowerShell helpers; native job containment
and acceptance are required before that path can open. No source files were
added; the exact inventory remains 344.

Default-association refresh queries a fixed four-target freedesktop set through
the production association service. Linux `xdg-mime` reads and writes now use
the shared process-group deadline and 64 KiB output cap. The diagnostic returns
only targets queried, effective-default count, directly-writable count,
availability, and partial status; association keys, handler identities, names,
paths, provider details, and errors never cross the boundary. Windows returns
unavailable until its registry traversal has a verified latency boundary.

GTK-originated internal window menus now receive compositor keyboard focus when
their surface is first inserted. The owned native GTK/X11 origin fixture passes
remote maximize/restore, delayed post-revocation gesture denial, independent
physical gestures, delayed menu denial, remote-menu retirement, physical
replacement survival, and physical semantic-action takeover. This closes the
previous replacement-menu Escape/focus and physical dispatch seam; the isolated
compositor, Xvfb, D-Bus services, GTK client, and listener were stopped afterward.

The coherent diagnostic snapshot now includes the compositor-owned Codex feature
projection: support, installation class, enabled state, runtime health, and
configuration generation at the snapshot observation point. Source labels,
executable paths, provider diagnostics, account state, projects, threads, and
credentials are excluded. A redaction regression seeds private source/failure
text and proves it cannot reach serialized output. Explicit feature re-probing
remains separate because the configured source may be an arbitrary executable or
remote host and cannot be invoked as a generic diagnostic action.

The coherent snapshot also reports payload-free aggregate counts for production
effects awaiting compositor reconciliation: desktop scene updates, clipboard
image-copy frames, launch attribution observations, deferred output retirements,
and whether shell focus work is pending. It never includes clipboard pixels,
launch commands, output identities, or shell targets. The broader `effects`
domain remains explicitly unavailable. Completed remote shell-command, guarded
device-control, and application-launch commits now enter the bounded event stream
in production order with fixed coarse outcomes. Native platform acknowledgement
coverage and non-shell effect categories remain to be projected and exercised.

Ordinary transient shell surfaces are now admitted to bounded inventory and
on-demand semantic inspection only when their production host is visible and its
live application/tree protection state is clear. This covers notifications,
window previews, window/application context menus, screenshot UI and the
on-screen keyboard. Hidden or protected variants remain absent. Lock, Codex
project/chat placeholders and trusted-control surfaces remain excluded because
they do not provide this exact host-owned evidence through the shell projection.

Pointer diagnostics now correlate a protected-safe hosted hit with the exact
bounded semantic tree generation and node ordinal at that observation point,
without coordinates, labels, values, actions or input payloads. The compositor
also retires presentations and continuations for shell IDs removed by topology
reconciliation, so a replaced transient cannot remain remotely hittable. A live
nested compositor opened a real GTK window menu, observed it through MCP, moved
physical test input to an advertised local semantic bound, and reported the
current menu surface plus a semantic generation and node. The fixture's broader
scope, protected-process, provider-timeout, revocation and lock checks also
passed; all owned native processes were stopped afterward.

The same pointer record now reports Nickel-owned hosted-window decorations as
one of a fixed titlebar, window-button, or resize-edge role. It reuses the
production frame hit test and exposes no pointer coordinates, window title, or
client payload. Focused owner coverage places the pointer on both a live button
node and a hosted titlebar. With semantic and frame hits represented, the broad
`internal_hit_testing` unavailable marker has been removed.

The bounded desktop-event stream now records production workspace create,
remove and selection outcomes as the resulting active workspace and workspace
count. Redundant notifications coalesce, and the event carries no window
membership, title, client identity or input payload.

Launcher and panel semantic pin toggles and reorder actions now stage on the
bounded favorites worker and commit through the existing preferences owner. The
commit retains the originating surface/output permit, current installed catalog,
file revision, shared-input reservation and cancellation boundary, then applies
only the accepted persisted preferences to the runtime launcher. An owned nested
X11 fixture invoked the advertised launcher unpin action through MCP, observed a
confirmed persisted/runtime change, and proved that private recent history was
preserved; its existing stale-write, scope and revocation matrix also passed.

The coherent snapshot now retains the latest bounded result for each allowlisted
platform refresh domain. Each record carries its own generation, native worker
observation interval, explicit five-second freshness state, availability and
partial/reconciliation status; provider identities, errors and native labels stay
excluded. The snapshot also exposes the shared diagnostic worker's generation,
timestamps and busy state so an in-flight query is distinguishable from missing
data. An owned PipeWire/X11 fixture refreshed real dummy-sink state and correlated
the retained record and idle worker with the enclosing snapshot observation.
Unimplemented platform queries remain explicitly unavailable.

The latest bounded installed-application refresh is retained by the same coherent
snapshot with its native discovery interval, generation, freshness, application
count, partial result and launcher reconciliation status. It contains no catalog
IDs, names, paths, scan roots or errors. The owned launcher fixture refreshed the
real isolated catalog, correlated the retained record with the snapshot timestamp,
and then passed its mutation, semantic, stale-write, scope and revocation matrix.

Application-catalog and allowlisted platform refresh completions now enter the
bounded event stream with only their fixed domain, retained generation and partial
bit. The owned catalog and PipeWire fixtures correlated each event with the exact
retained snapshot record. Application identities, device/provider labels, paths,
errors and returned native state remain outside the event payload.

The coherent snapshot now exposes the existing bounded trace lifecycle audit as
generation-bearing start, stop, timeout and cancellation records. Projection
removes client, lease and trace IDs while retaining only fixed category/transition,
duration limit, elapsed time and session-relative observation time. An owned
nested X11 fixture exercised explicit stop, natural timeout and lease-revocation
cancellation, then read each transition through a fresh full-debug snapshot.

Ordinary shell presentation insertions and retirements now enter the same
bounded event stream with a generation-bearing surface identity, fixed role and
visibility bit. Lock, trusted-control and unsupported Codex roles never enter
this category; titles, text, pixels, paths, outputs and client identities are
absent.

Native nested acceptance opened a real GTK context menu, found its matching
insertion event through the MCP diagnostic snapshot, and then used physical
Escape to verify a retirement event for the same surface generation. The
fixture's existing protection, timeout, revocation and lock matrix also passed,
and all owned compositor, Xvfb and D-Bus processes were stopped.

The snapshot publication boundary now diffs the bounded protected-filtered
ordinary-window inventory and records changes to geometry, workspace, active,
minimized, maximized and fullscreen state. The retained baseline is bounded by
the production window inventory, and events use only the generation-bearing
window ID and typed state; titles and application identities are excluded.
Native nested acceptance used real GTK titlebar double-clicks and verified both
the maximized and restored events through MCP before continuing the existing
transient, protection, timeout, revocation and lock matrix. All owned fixture
processes were stopped afterward.

Keyboard-focus events now cover ordinary windows, ordinary compositor-hosted
shell surfaces and the absence of a remotely observable recipient. Shell records
carry only the current surface generation and a fixed role; protected, unknown or
cleared focus produces the same payload-free cleared state. Unchanged focus is
coalesced. An owned nested X11 fixture opened the launcher and correlated its
native keyboard focus with the exact launcher surface generation in the event
stream before passing the existing persistence, scope and revocation matrix.

Production-effect events now carry the session-local operation number assigned
to the authorized MCP request. This lets a full-debug observer correlate shell
owner completion with the existing bounded operation record without putting a
client, lease, application, target, command or path in the event. Direct
installed-application launches now emit the previously missing confirmed launch
effect. Native nested acceptance launched a real catalog entry and matched its
effect to the successful server operation; the existing launcher persistence,
semantic, stale-write, scope and revocation matrix then passed unchanged.

The diagnostic snapshot now treats production effects as an implemented domain:
it publishes a coherent payload-free count of pending owner work and an ordered,
bounded stream of typed completions correlated to MCP operations. The stale
whole-domain `effects` unavailability declaration has been removed. Individual
effect kinds that Nickel does not yet implement remain absent rather than being
reported as completed.

Native Linux acceptance also exercised every typed platform refresh domain in
one live session: connectivity, audio, peripherals, maintenance and default
associations. Each refresh reached its production worker, retained its own
generation and observation interval, and emitted the matching payload-free
completion event. Linux therefore no longer advertises a generic unspecified
platform-query gap.

Windows default-association refresh now runs its existing production registry
inspection behind a two-second, single-flight worker boundary. A timed-out read
returns unavailable and retains the sole admission slot until its worker exits,
so retries cannot accumulate threads or publish late results. The deadline and
retained-admission path passes a focused behavioral test, and the actual Windows
Nickel branch compiles with a disposable MinGW toolchain. Native Windows registry
execution remains unverified on this Linux host.

The coherent snapshot now retains the latest successful external accessibility
traversal as a payload-free subsystem record. It carries the MCP operation number,
fixed scope, provider/owner timestamps, node count, truncation and five-second
freshness state. Tree nodes, window and application identity, provider metadata,
names, text, values and actions are excluded at collection. The full native GTK
AT-SPI matrix correlated a real traversal with its operation record, then passed
password/editable subtree exclusion, scope, output movement, focus, transient,
provider-timeout, revocation, input-ownership, retirement and lock checks.

Each successfully owner-validated external accessibility traversal now also
enters the bounded desktop event stream. The event uses the same operation
number as the retained observation and server completion plus only the fixed
scope, node count and truncation bit. Native GTK acceptance correlated all three
records before completing the broader AT-SPI lifecycle matrix.

The coherent snapshot now includes an explicit protected-filtered resource
summary. It aggregates current renderer-surface count, software frame bytes,
fallback raster bytes, and retained shell image entries/bytes exclusively from
the per-surface and cache records already admitted into that same snapshot.
Shared GPU caches and external client allocations remain explicitly unavailable
because their ownership cannot yet exclude trusted surfaces. Native nested
acceptance recomputed every total from the published source records and matched
them exactly without resource identities or paths.
