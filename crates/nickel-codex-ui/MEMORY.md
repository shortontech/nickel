# Codex transcript memory and streaming budgets

The server owns authoritative history. The live backend projection retains lifecycle
metadata only; `ProjectedItem::text` remains an empty compatibility field. The sidebar
keeps thread summaries, dropping hydrated `Thread::turns` once the active transcript
has consumed them. Eviction never deletes server history.

## Production limits

| Owner | Limit | Overflow policy |
| --- | --- | --- |
| Backend projection | 2 MiB capacity estimate; 2,048 item records; 128 threads; 32 terminal turns/thread | Retire completed metadata deterministically by ID. If live metadata still exceeds capacity, explicitly fail the connection; retain the active turn ID for diagnostics. |
| Backend identifiers/error | 4 KiB each | Reject oversized lifecycle metadata with an explicit failure; cap retained error text at a UTF-8 boundary. |
| Backend request correlation | 128 requests | Reject additional requests; release correlation on response, timeout, or connection failure. |
| Backend interactions | 32 pending requests; 32 question IDs/request; 32 KiB question-ID capacities; 4 KiB ID/method | Return an explicit RPC error for the additional interaction, preserving existing requests. |
| UI transcript | 2,000 items; 8 MiB total body capacities; 256 KiB/body | Retire completed history first; if needed omit old active display data without changing turn/approval authority. Truncate one oversized body at a UTF-8 boundary and append a visible local-limit marker. |
| UI item identity/type | 4 KiB each | Reject oversized incoming item metadata with a visible failure. |
| UI aliases and retired IDs | 2,000 entries and 2 MiB string capacities in each collection | Retire oldest routing/tombstone records. Late deltas for retained tombstones are ignored. |
| Exploration sets | Four sets, each 2,000 entries and 4 KiB/value | Bound retained detail; report that displayed counts are lower bounds when capacity is exceeded. |
| UI pending interactions/diagnostics | 32 pending; 100 diagnostics, 512 characters each | Preserve existing interactions and mark excess delivery as disconnected; diagnostics evict oldest. |

Completed-item eviction also invalidates selection ownership and updates indices for
merged agent/activity items. A late completion for an old turn cannot clear a newer
active turn. Histories are reloaded through the existing resume/refresh workflow,
but reloading does not bypass local body limits; omission messages state that server
history remains unchanged.

## Derived ownership and frame work

Each item content generation has an immutable snapshot and a shared `OnceLock`
Markdown/selection projection. Rendering and full-document selection consume the
same parsed representation. The full selection document is lazy: ordinary layout
parses visible cards, while select-all/copy or active cross-item selection can
materialize offscreen text. Membership, identity, content, and truncation changes
replace the document generation. Mutation releases the state's old snapshot and
document references; a currently presented frame may retain its previous generation
until that frame is replaced.

Snapshots duplicate at most 8 MiB of body data for the current transcript generation.
Parsed trees count source and nested string capacities, vector capacities, and
selection-run storage. Each retained item projection is admitted under
`2 KiB + 16 × body capacity + 4 × ID capacity` (less than 164 MiB across the maximum
transcript). Selection-index/run-copy allowance is independently limited to
`2 KiB + 8 × body capacity + 4 × ID capacity` (less than 100 MiB across the maximum
transcript). Pathological Markdown expansion displays the same source as plain text
with an explicit formatting-omission notice. Ordinary Markdown keeps its styling.
Selection text is shared through `Arc<str>`; the document's owned IDs/index are
counted separately. These are conservative retained-allocation allowances, not RSS
or allocator-overhead measurements. Parsing/serialization scratch and an old
presented generation are additional bounded owners.

Polling dispatches at most 128 events or 4 ms per poll and returns to the normal
minimum polling deadline when work was consumed. One event is not preempted halfway
through its reducer. Streaming deltas only mutate bounded authoritative text and
invalidate a generation; consumption rebuilds a changed item at most once per
presentation batch. No cumulative-linear parsing claim is made.

Delivery and attachment-compatible command/snapshot budgets are documented in
[`../nickel-codex/DELIVERY.md`](../nickel-codex/DELIVERY.md).

## Validation and remaining measurements

Regression tests cover UTF-8 overflow/completion, aggregate-capacity churn, retired
late events, immutable projection reuse, visible-only layout, offscreen copy,
repeated selection-owner release, pathological formatting, and initial text in
merged response cards. Backend tests exercise 5,000 metadata insertions and explicit
live-capacity failure. The WebSocket fixture exercises lifecycle events, approvals,
and a 9 MiB outgoing image request.

Run the synthetic release comparison with:

```sh
cargo test -p nickel-codex-ui --release streaming_projection_measurement -- --ignored --nocapture
```

The predeclared target is at least 75% less elapsed parsing work than rebuilding
after every delta, with `ceil(deltas / 128)` rebuilds. With lazy full-document
selection explicitly consumed after each batch:

| Body bytes / deltas | Previous rebuild pattern | Batched pattern | Retained body + projection |
| --- | --- | --- | --- |
| 4,080 / 60 | 60 parses, 1,337 µs | 1 parse, 59 µs | 69,279 B |
| 32,708 / 481 | 481 parses, 85,362 µs | 4 parses, 982 µs | 550,411 B |
| 131,036 / 1,927 | 1,927 parses, 2,373,030 µs | 16 parses, 26,556 µs | 2,201,971 B |

The retained column excludes the immutable source snapshot and full selection
document's index, whose budgets are described above. This compares the previous
per-delta algorithm inside a repeatable test; it is not
a before/after end-to-end desktop benchmark. Real long-session retained/peak RSS,
allocator bytes, input/frame latency, reconnect against live authoritative history,
and platform interaction testing remain acceptance measurements for the user test
pass. Queue release burst/RSS measurements are recorded separately in DELIVERY.md.
