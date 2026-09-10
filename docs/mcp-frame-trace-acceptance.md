# Bounded frame dispatch traces

`diagnostic_action` accepts `start_frame_trace` with `duration_seconds` from 1 to
60, and `stop_frame_trace`. Both require a current full-debug lease. The
`nested_frame_dispatch` category observes the production winit redraw handler.
`drm_frame_dispatch` observes the production DRM render dispatcher, including
inactive and retry returns. The active runtime selects the category; a session
without either backend rejects trace startup.
Its duration is wall time spent dispatching the redraw, including submission and
housekeeping. It does not claim GPU completion or display presentation latency.
Neither category claims successful submission simply because dispatch returned.

The compositor retains at most one active trace and 256 records. Records contain
only a sequence generation, monotonic time relative to trace start, output
identity generation, and dispatch duration. The collector observes existing
redraws; it does not install a continuous redraw loop. `diagnostic_snapshot`
returns `frame_trace` only to the lease that started it. Expired or explicitly
stopped traces retain their bounded result until replaced or their authority is
invalidated. Every recording operation checks the original lease and its
cancellation generation. An idle trace has no worker or pending effect.

## Linux native acceptance, 2026-09-10

Tests used an isolated Xvfb-backed Nickel winit compositor and its local approval
endpoint. They exercised production redraws and MCP requests:

- Real redraws produce nonzero dispatch durations correlated with an output in
  the diagnostic inventory.
- Zero-second and over-limit durations, ordinary leases, and a second active
  trace are rejected.
- A two-second trace stops collecting while its lease remains valid. Explicit
  stop also prevents later redraws from extending its result.
- A rapid local pause/resume cycle invalidates the original trace even between
  recording operations.
- Another full-debug client cannot read, stop, or replace an active trace.
  Revocation frees the slot for a freshly authorized trace.
- Repeated repaint requests exercise bounded retention through the native frame
  handler; unit tests separately cover the exact deadline boundary and eviction.

Local evidence is under `target/mcp-native-2026-09-10/`, in
`frame-trace-native-results.txt`, `frame-trace-retention-results.txt`,
`frame-trace-unit-results.txt`, `frame-trace-build.txt`, and
`frame-trace-clippy.txt`. Native transport fixtures under
`/tmp/nickel-mcp-native/` are disposable local fixtures, not shipped tooling or a
portable acceptance harness.

Trace lifecycle auditing is now available through the trusted local control
snapshot. Its separate 128-event ring records start, explicit stop, timeout, and
cancellation with session-local trace, client, and lease IDs, requested duration,
elapsed time, and a fixed backend category. It retains no trace samples or client-provided strings. A
one-second production housekeeping pass settles idle timeouts, recording the
actual deadline, and local authority changes settle cancellations. Each trace
records one terminal outcome, including when it is subsequently dropped.

`trace-audit-native-results.txt` verifies idle timeout, idempotent stop,
pause/resume cancellation, permission-audit correlation, and exclusion from MCP
snapshots. `trace-audit-unit-results.txt` covers 51 remote-control tests, including
bounded audit eviction and cancellation on drop. The Linux compositor and
Settings pass Clippy (`trace-audit-clippy.txt`).

The DRM implementation passes compilation and Clippy with both Linux backends
enabled. Physical DRM acceptance has not run: the local compositor owns the
active seat, and the udev backend enumerates all its GPUs even with
`NICKEL_DRM_DEVICE` selecting a virtual primary. Launching another udev session
would not isolate the test from the user's desktop. Native nested regression
results after this change are in `drm-frame-trace-native-results.txt` and
`drm-trace-audit-native-results.txt`; build, unit-test, and lint artifacts use the
`drm-frame-trace-` prefix.

Trusted Settings now displays a Recent diagnostic traces card with the latest
16 events, retained/evicted counts, backend, client/lease correlation, duration
limit, elapsed time, and terminal outcome. Native Settings was launched on the
isolated compositor, populated by real trace actions, and scrolled using native
pointer-axis input. Visual inspection confirmed readable rows; the Settings
window remained excluded from the full-debug inventory. Evidence:
`trace-audit-settings.png`, `trace-audit-ui-native-results.txt`,
`trace-audit-ui-build.txt`, and `trace-audit-ui-clippy.txt` in the same artifact
directory. The disposable fixture initially used an incorrect maximize action
name; correcting it to the protocol's `maximize_restore` resolved that test
client timeout.

DRM native acceptance, Windows tracing, other trace categories, and physical
emergency-stop acceptance during tracing remain outstanding. This does not complete Spec 0231.
