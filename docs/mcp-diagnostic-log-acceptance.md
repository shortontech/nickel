# MCP diagnostic warning history

The full-debug snapshot exposes `diagnostic_logs`, a separate in-memory collector
of warning/error source metadata. It never reads file logs or visits tracing
message fields or span values. Each record contains its generation, monotonic
collection time, fixed severity and subsystem categories, a numeric static-callsite
code, and numeric source-line detail. It omits target strings and source paths.
Retention is limited to 256 records; eviction and lock-contention drops are explicit. A busy or uninitialized
collector is reported as unavailable. Logging collection has its own clock and
generation rather than claiming synchronous compositor observation.

The collector uses a warning-level filter independently of the file-log filter.
It retains the existing exclusions for input, text rendering, and credential-
bearing transport diagnostics. This source metadata helps locate failing code;
it does not implement structured failure details or temporary trace categories.

## Validation on 2026-09-10

Linux native acceptance used an isolated Xvfb-backed Nickel winit session with
its own runtime, configuration, state directories, and loopback MCP listener.
With `NICKEL_LOG=off`, a local datagram client deliberately removed its return
socket path before issuing ordinary window queries. This exercised the real
session-control reply failure path, without changing the compositor for testing.

- 281 real reply failures produced warning records pointing to
  `control_protocol.rs`, with 256 retained records and ordered generations/times.
- A private-path canary, the formatted warning message, and event fields were
  absent from the remote projection.
- An ordinary full-session lease and a paused full-debug lease could not read
  the diagnostic snapshot. Trusted local control remained responsive.
- Unit tests exercise real tracing events with a field whose `Debug` formatter
  panics if called, bounded eviction, excluded input/transport targets, and
  nonblocking contention. A disabled informational event's field expression
  must not run merely because this collector is installed.

Local artifacts are in `target/mcp-native-2026-09-10/`:
`diagnostic-log-native-results.txt`, `diagnostic-log-unit-results.txt`,
`diagnostic-log-build.txt`, and `diagnostic-log-clippy.txt`. The disposable native
client lives at `/tmp/nickel-mcp-native/diagnostic-log-test.py`; this is recorded
local acceptance, not a portable automated native harness. Windows native
validation and the remaining Spec 0231 diagnostic domains are still outstanding.
