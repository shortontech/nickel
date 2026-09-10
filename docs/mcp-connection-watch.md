# Required MCP connection lifecycle

A ready `client_connection` watch is required before a client requests a control
lease, receives local approval, resumes a lease, or uses desktop or diagnostic
authority. Initialization, discovery, health, identity creation, public aggregate
metrics, and authenticated status remain available without a watch. There is no
stateless fallback that retains approved desktop authority without live presence.

A watch is an ordinary long-running MCP tool request. Call `tools/call` with
`name: "client_connection"`, `arguments: {"action":"watch"}`, and the standard
request `_meta.progressToken`. Normal MCP version headers and per-request metadata
still apply. Both MCP 2025-06-18 and 2026-07-28 use this contract over Nickel's
existing stateless HTTP endpoint; no proprietary HTTP endpoint is needed.

The client **transport** maintains presence automatically, independently of the
LLM, user activity, and ordinary tool calls. Wait for a standard progress
notification whose message is `Connection watch active` before starting permission
or control work. Open a replacement watch after about 30 seconds, wait for its
readiness, then close the earlier request. Each watch lasts at most 60 seconds;
two may overlap per identity, with sixteen admitted globally. Progress is sent
every second. This maintenance does not request approval, renew a lease, extend
expiry, or represent another user action.

Clients must support concurrent long-running tool requests, progress notifications,
request cancellation, and background transport maintenance. A client that cannot
supply these features needs a Rust client-side MCP adapter that owns the watch and
forwards ordinary requests. That adapter is not yet shipped. Such clients can
still discover Nickel and establish identity, but cannot obtain desktop authority.
`get_control_status` reports `connection_ready`, `connection_watch_required`, and
`connection_watch_seconds` so clients can diagnose their setup. Quiet user time
alone never constitutes disconnect when the transport maintains presence.

Nickel's desktop owner checks credentials and lock state, activates the reserved
watch, performs eligible reconnection, and reconciles cancelled input before
acknowledging readiness. Reservation or a failed readiness attempt grants no
authority. Every permission and native-operation boundary checks a ready watch
and its monotonic deadline under the authority lock; delayed task cleanup cannot
extend presence. New admission expires old watches first, so a late replacement
cannot skip disconnect cancellation.

Loss or completion of the last ready watch disconnects the identity. Pending
approval is cancelled, non-resumable leases are revoked, and resumable leases are
suspended. An authenticated replacement resumes only eligible unexpired resumable
leases. It never reverses local pause or restores cancelled input generations.
Closing one overlapping ready watch has no effect. Explicit disconnect, lock,
disable, revocation, or blocking invalidates old watch IDs; stale destruction
cannot cancel a new connection. Local approval also requires the exact current
pending-card generation, preventing an old card from approving an identical
request submitted after reconnect.

Last-watch destruction invalidates authority synchronously; native owners release
cancelled input on their dispatch loop. Normal completion waits for owner
reconciliation before the final tool result. Abrupt cancellation has no response
to acknowledge. Watch destruction signals a coalesced owner wake after invalidating
its incarnation, without waiting for the authority mutex or ordinary queue space.
Linux uses a checked nonblocking eventfd; Windows uses a payload-free checked
thread-message wake. Owners consume pending cleanup before ordinary queued work.
Clearing pending before reconciliation preserves a new signal arriving during that
work. Wake setup failure preserves the desktop and its periodic fallback. Failed wakes
retain pending work and produce a fixed owner-side diagnostic;
the existing one-second Linux timer and 100 ms Windows poll provide fallback
scheduling. These are scheduling fallbacks, not hard deadlines for an OS-stalled
owner or proof of native Windows input release (that backend remains unavailable). HTTP transport loss ends the watch; a silent network blackhole may
persist until its 60-second bound. Progress accepted into a transport buffer is
not proof that the peer received it. TCP close or normal completion of an ordinary
stateless request is never treated as logical disconnect.

Validation includes authority tests for failed/unready watches, monotonic expiry
before task cleanup, overlap, stale destruction, disconnect/reconnect, and stale
approval generations. The real HTTP service test covers progress readiness and
stream loss for both protocol generations. Previous opt-in implementation native
acceptance observed actual held drag/key release in an owned Wayland client,
overlap preservation, cancelled approval, permitted resumption, and preserved local
pause (`/tmp/nickel-mcp-watch/native-results.txt`). Those earlier native results
prove the shared lifecycle path, not the newly mandatory readiness gates; native
acceptance of the mandatory contract remains pending integration. Physical DRM,
Windows, silent blackholes, and normal 60-second watch expiry also remain native
validation gaps.
