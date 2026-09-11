# Native scope matrix: first increment

The checked-in Rust `nickel-linux-remote-control-acceptance` harness exercises
semantic input, native pointer dispatch, and production capture under exact
shell-surface, ordinary-window, application, output and full-session leases. This
is Linux evidence for Spec 0230 verification items 3, 4 and 6; it does not complete
the cross-platform native scope acceptance gate.

The harness opens its own nested launcher's production UI through the trusted
local session protocol. A bootstrap lease observes the real launcher surface and
its generation-bearing output, then is revoked. Each scope receives one local
approval, with exactly one active lease and no pending request. For each scope,
three distinct search edits use fresh semantic tree generations, and subsequent
observations verify the actual search text. The harness clicks the real search
field through surface-local, output-global and desktop-global pointer targets and
requires a fresh production semantic observation to retain field focus. Surface
and output captures must return an identity-bound, generation-bearing, decodable
PNG with dimensions matching the production owner. The exact surface lease also
denies inspection, pointer input and capture of the separate panel with the
resource-boundary error.

After each edit, the local authoritative snapshot must retain the same lease,
scope and debug level, no pending requests, and unchanged permission/lease audit
history and eviction counts. This detects transient permission requests even when
their cards have disappeared before the next observation. Ordinary countdown of
the existing lease is allowed. No broader fallback lease remains during the
narrow-scope assertions.

The native Wayland run passed on September 11, 2026, using the worktree based on
`4854119` plus this change. All six focused harness tests and strict harness and
fixture Clippy passed. The full `--ordinary-scopes` run also retained the existing
preapproval tool-denial, privacy, renderer-diagnostic and emergency-revocation
checks. Its compositor, client fixtures and temporary runtime were cleaned up
afterward.

Build and run from a reachable Wayland session:

```sh
cargo test -p nickel --features backend-winit --bin nickel-linux-remote-control-acceptance
cargo clippy -p nickel --features backend-winit --bin nickel-linux-remote-control-acceptance -- -D warnings
cargo build -p nickel --features backend-winit --bin nickel --bin nickel-linux-remote-control-acceptance
target/debug/nickel-linux-remote-control-acceptance
```

The commands above are the reproducible evidence path. A missing host backend
produces `SKIP`, which must not be counted as native acceptance.

Without `--ordinary-scopes`, ordinary window and application scopes remain
uncovered: the default fixture owns shell resources. The shell matrix does not
prove raw keyboard or external-client pointer delivery, multiple-output confinement, workspace
movement, external application accessibility, a local assistive workflow,
physical keyboard/DRM behavior, or native Windows acceptance. The Wayland run
exercises nested presentation and compositor-owned shell semantics, not ordinary
Wayland application identity. Xwayland was not rerun for this increment.

## Ordinary Wayland clients

The optional `--ordinary-scopes` increment uses the unchanged repository examples
`nickel-ui/examples/keyboard_recipient.rs` and `standalone.rs`. Build these beside
the compositor and harness:

```sh
cargo build -p nickel-ui --example keyboard_recipient --example standalone
target/debug/nickel-linux-remote-control-acceptance --ordinary-scopes
```

The harness starts the examples on its private nested Wayland socket. A bootstrap
lease obtains real window generations and `verified_application` identities from
the production compositor; titles only locate the expected fixtures and never
establish authority. The bootstrap lease is revoked before narrow assertions.

One window lease repeatedly focuses the keyboard recipient and sends three native
keys. Fresh owner observations confirm focus, and the actual client's bounded
stdout receipt confirms each resulting text change. The same lease captures the
real client area as an identity-bound decodable PNG and clicks the fixture's real
button; the client process confirms the application action. One application lease
then repeats the key, capture and pointer actions and admits a second
keyboard-recipient process/window created
after approval, using the same native executable identity without another prompt.
Its inventory contains exactly those two windows.

The distinct counter executable remains excluded. Focus, capture, pointer and
keyboard requests must fail with the resource-boundary denial. Before the negative
controls, the counter is focused locally and its live focus is verified through
the trusted owner query, eliminating lack of focus as an alternative rejection
reason. Every phase retains exactly one active lease and unchanged local approval
history. All fixture processes are killed and reaped on both success and failure.

The ordinary-window clicks have independent recipient-process confirmation. The
compositor-hosted launcher is not a separate Wayland client, so its pointer check
uses the production internal UI owner and a fresh semantic focus observation; it
does not claim external-client receipt. Protected lock and trusted-indication
content remains excluded by the existing privacy checks. This increment does not
invent a protected resource identity merely to issue a targeted denial.

Native Wayland acceptance passed September 11, 2026, on `4854119` plus this change,
including the complete earlier shell/privacy/emergency checks. Six focused
harness tests and strict harness and fixture Clippy passed. The combined stress
check completed 27 requests in 3.019 seconds with a 706 ms maximum response and
12,684 KiB RSS growth. The commands above reproduce the acceptance path.

This proves ordinary native Wayland executable identity and later same-application
window admission for these real examples. It does not prove Flatpak/shared-runtime
or broker identity, remote application launch, transients, cross-output/workspace
movement, Xwayland application behavior, physical input, assistive workflows, or
native Windows acceptance.

## Output and workspace movement

Run the ordinary-client matrix with `--movement` to include movement acceptance:

```sh
target/debug/nickel-linux-remote-control-acceptance --movement
```

This implies `--ordinary-scopes` and requires the same two repository examples.
Before startup, the harness creates one desktop entry for the real counter
example under its private `XDG_DATA_HOME`; it does not install a host application.
The production catalog must report the same verified executable identity as the
live counter window before the negative launch test is admitted.

The nested test-control capability creates a second Smithay output at 180/120
fractional scale with a 90-degree transform, publishes its Wayland output global,
and runs production relayout. The harness verifies the transformed logical geometry,
creates a workspace through the production session command, and uses its returned
authoritative state.
The resulting tests prove:

- A window lease and an application lease each retain native key delivery after
  the window moves to the second output, changes workspace there, and returns to
  the original workspace/output. Trusted owner queries confirm full geometry
  containment and workspace membership; the actual client confirms each key.
- An output lease permits input on its output, omits the moved window from its
  inventory and denies focus/capture/keyboard outside that output, then permits
  native input again on return using the same lease. The outside keyboard target
  is focused locally first to rule out a focus-only rejection.
- The application lease cannot enumerate or launch the unrelated real catalog
  executable after movement. Launch must fail specifically because the target is
  outside the application lease, not because its catalog entry is absent/stale.
- A separate output lease enumerates and launches that real catalog executable.
  The immediate outcome confirms the requested spawn and output while truthfully
  leaving placement unconfirmed; the harness then observes the resulting native
  window inside that output and uses the same lease for focus, capture, and a
  final global pointer hit confined to that output generation.
- No movement requests another approval or changes the existing authority audit.

The earlier 1x normal-transform movement path passed September 11, 2026, on
`d0d5fbb` plus its original movement change. That historical run predates the
fractional-scale and transform assertions described here and does not establish
their current native runtime result.
All preceding ordinary/shell cases and the combined diagnostic/capture/input stress,
privacy and emergency checks also passed. Six focused harness tests and strict
harness/example Clippy passed. The combined stress check completed 27 requests in
2.973 seconds with a 703 ms maximum response and 4,124 KiB RSS growth. The commands
above reproduce the acceptance path. The added workspace/output were removed, all
example processes reaped, and the owned compositor/runtime cleaned afterward.

The extra output is virtual: it exercises live Wayland clients and production
compositor resource/geometry/workspace authority across mixed logical scale and
output transform, but has no independent physical display presenter. This is not
physical multi-monitor, DRM, physical mixed-DPI presentation, Xwayland, or Windows
acceptance. The positive launch uses a directly executed
repository fixture; broker/daemon and Flatpak launch attribution remain separate
cases. Transient identity remains separate, and held-input coverage is described
below.
## Ordinary Xwayland clients

The parallel Xwayland increment uses the same repository examples and assertions.
Run it from a reachable X11 session with:

```sh
cargo build -p nickel-ui --example keyboard_recipient --example standalone
target/debug/nickel-linux-remote-control-acceptance --xwayland-ordinary-scopes
```

This option starts Nickel's private Xwayland server and forces both examples onto
its published `DISPLAY`. Production resolves each window owner through XRes, then
corroborates the window with `WM_CLASS` and derives application authority from the
owner process executable. The test repeats three focus/key actions under one
window approval, repeats them under one application approval, admits a later
window from the same executable, and denies focus, capture, and keyboard access
to the unrelated example.

The Xwayland option also enables fixed ShiftLeft and left-button receipts in the
real keyboard-recipient client. For each input kind, two overlapping
window-scoped leases prove that the first owner excludes the contender's key,
drag, focus, end and cancel requests. The owning lease then ends and cancels its
own holds with matching client releases. Separate holds release when that lease
is explicitly revoked and when a one-second lease expires. Retired owners cannot
continue, end or cancel the surviving contender's later hold; the contender can
keep it alive and end it with its own matching X11 release.

Native acceptance passed September 11, 2026, with the outer nested compositor on
the host Xwayland display (`DISPLAY=:0`, with host Wayland removed). The complete
earlier shell/privacy/stress/emergency suite passed in the same run. The ordinary
Wayland mode was rerun afterward and also passed unchanged. The X11 negative
control additionally caught and fixed a reply-ordering bug where an uncommitted
native keyboard plan could mask the correct resource-boundary denial with a
cancellation error.

The same native suite also passed with both options, including the existing
virtual output and workspace movement matrix:

```sh
target/debug/nickel-linux-remote-control-acceptance \
  --xwayland-ordinary-scopes --movement
```

This combined run covers window and application scope through output/workspace
movement, application-held continuation, output-boundary release, overlapping
focus denial, and client-confirmed X11 key/button edges. The additional output
is a compositor authority and geometry fixture without an independent physical
presenter.

Native Xwayland acceptance passed September 11, 2026, on `7d0809a` plus this
increment in both direct and combined movement modes. Six focused harness tests
and strict harness Clippy passed. The combined stress check completed 27
requests in 3.086 seconds with a 718 ms maximum response and 672 KiB RSS growth.
The harness removed the virtual workspace/output and reaped all client and
compositor processes afterward.

This remains bounded synthetic-input acceptance with two repository clients. It
does not cover PID reuse, sandbox brokers, shared runtimes, transient ownership,
physical input, assistive workflows, physical multi-monitor presentation, or
native Windows behavior.

## Held-input movement and arbitration

The `--movement` path also runs two-client held ShiftLeft and left-button drag
cases against the real Wayland keyboard-recipient example. The example enables
fixed down/up receipt markers only for this acceptance mode; arbitrary keys,
text, and coordinates are not added to its receipt log.

An output-scoped owner starts each hold while a window-scoped contender can
observe the same target. The contender's key, drag, and focus requests must fail
specifically because shared input or the native opposite input primitive is
busy (the pointer adapter currently labels any pressed keyboard input local). A
trusted local move takes the target to the virtual output. The client must
receive the native release, and the old
output lease must lose both continuation and inventory access. The window-scoped
contender then acquires the hold on the moved target. The previous owner's cancel
must fail without releasing the new owner's input; the new owner continues and
ends it with a matching native release. Returning the target does not resurrect
the old hold. Both leases remain unchanged, with no additional approval.

The application-scope cases keep that lease active and approve one overlapping
window-scoped contender before either hold begins. The application owner starts
the real key or button hold, and the contender's key, drag and focus operations
must fail while still being able to observe its exact authorized window. The
production output owner then moves that window to the virtual output. A single
production workspace-owner transition moves it to the second workspace and
follows it there, avoiding an intermediate unrelated focus. After each boundary,
the original owner continues the hold and the client receipt remains pressed
without a release.

The seat owner permits this only after freshly resolving the same window
incarnation, unprotected state, verified application identity, native focus and
live lease. Window-targeted drags retain their client-local anchor as the
production window geometry moves; output-targeted and desktop-targeted holds
retain their fixed global anchor. Output-scoped holds still fail their fresh
resource check and release when the same window leaves the output.

The harness then activates the unrelated real counter application. The client
must receive a native release, and focus, capture, pointer and keyboard requests
against that application must fail at the application resource boundary. The
old application owner cannot continue. After the contender starts a new hold on
the original window, a stale cancel from the application owner cannot release
it; the contender continues and ends with its own native release. The two
approved leases and permission/lease audit histories remain unchanged throughout
each movement and arbitration case.

The drag case exposed missing output evidence for window-targeted pointer
requests. The resolver now uses the same current-window output helper as keyboard
authorization. Before this fix the real output-scoped drag was denied before
press; the native case retains that positive authorization regression alongside
the negative boundary checks.

This covers output-boundary cancellation, application-scoped continuation across
one output and workspace move, cross-client arbitration, unrelated-focus
cancellation, stale-owner isolation and native release delivery. The
move-and-follow fixture is one capability-gated local session command backed by
the production workspace owner; it does not prove a physical shortcut or
Settings UI workflow.
The remote input remains synthetic. Physical multi-monitor presentation,
physical local-input takeover, more complex workspace sequences and native
Windows remain open. The combined Xwayland command above exercises this same
virtual movement matrix with real X11 client receipts.

Native Wayland acceptance passed September 11, 2026, on `c0f1878` plus this
increment: all four held-input cases and the complete preceding movement/scope
suite passed. The combined stress check completed 27 requests in 2.890 seconds
with a 661 ms maximum response and 8,120 KiB RSS growth. The virtual output
remains a compositor authority/geometry fixture, not a physical presenter.

## Bounded long connection and lease churn

`--long-churn` extends the default native Linux suite with 24 fresh authenticated
connection watches and 24 locally approved full-session leases. Even rounds let a
one-second lease expire; odd rounds revoke a 30-second lease through the trusted
local owner. Each retirement must leave the original lease and connection active,
reject the retired lease, record the expected audit transition, and return the
active connection and lease gauges to one.

While those connections and leases turn over, the original client repeatedly
uses production diagnostic snapshots, renderer repaint, alternating surface and
output capture, semantic inspection, and text input. The harness applies a
10-second ceiling to each measured response and a 120-second ceiling to the churn
phase. It samples the production compositor's Linux `/proc` `VmRSS` after every
round, with a 2 GiB absolute ceiling and a 128 MiB peak-growth ceiling. These are
acceptance bounds on sampled process RSS, not allocator accounting or a proof that
memory cannot grow in a longer session.

The public metrics response must remain below 128 KiB and 768 series. Every label
is checked against the published fixed method, outcome, scope, histogram, and
rate-limit sets; the complete series identity must be unchanged before and after
churn. Private connection labels, client IDs, tokens, and semantic-input canaries
must remain absent from diagnostics, metrics, error responses, and action results.

Build and run the matching optimized production executable and harness with the
shared native-build lock:

```sh
flock -x /tmp/nickel-mcp-build.lock -c 'cargo build --release -p nickel --features backend-winit --bin nickel --bin nickel-linux-remote-control-acceptance'
flock -x /tmp/nickel-mcp-build.lock -c 'target/release/nickel-linux-remote-control-acceptance --long-churn'
```

Native nested Wayland acceptance passed September 11, 2026, on `4854119` plus
this change. The long phase completed 24 fresh connections, 12 expiries, 12
explicit revocations, and 144 timed requests in 23.995 seconds. Its maximum
response was 183.6 ms; the fixed metrics inventory remained 677 series; sampled
RSS changed from 220,656 KiB to a 233,736 KiB final and peak value. The preceding
short stress completed 27 requests in 1.022 seconds with a 184.5 ms maximum
response and 15,360 KiB RSS growth. Six focused harness tests and strict focused
Clippy also passed.

This run used synthetic semantic input and a nested software-rendered Wayland
compositor. It provides no physical input, physical display/DRM, installed-session,
Xwayland, native Windows, allocator, or multi-hour soak evidence.
