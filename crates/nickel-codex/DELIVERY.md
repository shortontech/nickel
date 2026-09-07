# Bounded Codex delivery

Live subscriptions, replay subscriptions, controller events, and controller commands
use the same nonblocking delivery queue with per-type budgets:

| Queue | Entries | Retained payload | Individual payload |
| --- | ---: | ---: | ---: |
| Protocol/replay events | 256 | 4 MiB | 1 MiB |
| Controller events and history snapshots | 256 | 16 MiB | 16 MiB |
| Controller commands including image attachments | 16 | 160 MiB | 160 MiB |

Payloads are
boxed serialized bytes, so counters measure allocation lengths, including string
escaping and nested content. Queue metadata is separately bounded by 256 slots.
A normal controller has three queues, hence at most 180 MiB of retained payload.
The larger command budget preserves the attachment API's 96 MiB aggregate
resident allowance, including base64 expansion and the request envelope. It does
not increase the attachment admission limits. History snapshots have their own
budget so legitimate multi-megabyte thread resumes do not trip delta limits.

Encoding stops at the per-type individual limit; an oversized source object is rejected without copying
its complete contents into another buffer. One producer encoder, one consumer
decoder, and the producer's coalescing scratch can be live per queue. These
temporaries are not part of queue-byte metrics: decoded Rust collections can
occupy more memory than serialized bytes. Admission bounds their encoded input
to that limit; incoming transport frames have a separate 8 MiB cap enforced during local
reading and by the WebSocket decoder. Caller-owned models and history snapshots
are governed by their own retention budgets. This is not a total-process RSS cap.
Outgoing requests have a separate 160 MiB serialization cap. Remote outgoing
delivery has 16 entries and a 160 MiB total byte budget measured using String
allocation capacities. A payload remains charged while the socket sends it,
and rejected/disconnected/dropped payloads release their byte permits. Closing
the remote connection uses an atomic flag and bypasses queue saturation.

Adjacent deltas of the same kind and item coalesce up to 16 KiB. Controller
coalescing also requires the same generation. Starts, completions, approvals,
errors and turn transitions are ordering barriers. Smaller coalescing limits
bound repeated encode/decode work during streaming.
Fixed-size compatibility hashes and encoded-size metadata avoid deserializing
non-deltas, incompatible items, or tails over 24 KiB on the producer. Exact item
identity is still checked when merging, so hash collisions cannot mix streams.

Overflow atomically releases pending payloads, substitutes one small terminal
failure, and closes the stream. No further events enter the failed stream.
Upstream failure becomes a controller failure, putting the UI in Disconnected
state. Retry/reconnect replaces the generation and reloads authoritative backend
state; the user can reselect the thread to reload its history. We do not present
partially dropped progress as synchronized. Pending approvals must be recovered
through the backend's authoritative resumed thread state; no stale approval is
automatically answered. This is fail-closed recovery, not lossless backpressure.

Command overflow rejects only the attempted command and publishes an actionable
OperationFailed diagnostic through a bounded atomic mailbox. The connection and
already accepted commands remain available; the draft and attachments remain
available for retry. A too-large command can be reduced and retried. Interrupt
and shutdown use atomic flags outside the command queue. The forwarding loop
handles at most 32 events between command polls; closed UI delivery terminates
the worker. No delivery operation waits for space. An already running backend
RPC can still delay shutdown until its existing request timeout.

Metrics expose current bytes/entries, high-water marks, coalesced deltas,
overflow count and maximum queue dwell time in microseconds. Controller metrics
also expose generation replacements as recovery count. Counters contain no
message content. Queue counters are per generation and reset on replacement.

## Validation

Seven focused queue tests cover exact text and approval ordering, chunk splitting, entry saturation,
byte saturation, oversized events, failure generation preservation, disconnected
consumers and storage release. A controller test saturates commands and checks
that interrupt, shutdown and retry still work and that rejection is explicit.
Compatibility regressions cover a 2 MiB attachment command and history snapshot,
and the WebSocket integration fixture transmits a 9 MiB image request. Outbound
tests cover byte and entry limits, accounting during send, rejection rollback,
receiver drop and close under saturation.

Repeat the release workload with:

```
cargo test --release -p nickel-codex --lib delivery::tests::release_burst_measurement -- --ignored --nocapture
```

On Linux, 2026-09-07, the workload delivered 100,000 128-byte deltas losslessly
while draining every 128 messages in 651 ms. It coalesced 99,218 deltas, retained
at most 16,468 bytes/one entry, and measured maximum dwell time 1,815 microseconds.
The subsequent stalled consumer reached 4,064,834 bytes/31 entries before an
explicit terminal overflow reduced storage to 126 bytes/one entry. Running the
compiled test directly under `/usr/bin/time -v` measured peak RSS 8,524 KiB.
These are synthetic queue measurements, not desktop RSS or frame latency.
