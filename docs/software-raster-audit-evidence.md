# Software raster cache evidence (0212, 0217, 0219 scale)

Measured on Linux, 2026-09-07, with the optimized `nickel-ui` unit-test binary.
The predeclared targets were 40% less per-sample storage, candidate and retained
raster allocations within 2 MiB, and no more than 25% additional cold raster work
against the previous tuple algorithm. These are payload and microbenchmark
targets, not claims about whole-session RSS savings.

## Reproduction and observations

Run `cargo test --release -p nickel-ui --lib software_raster_memory_and_timing_evidence -- --ignored --nocapture`.
The benchmark also contains the previous 20-byte tuple collection/drawing
algorithm, using identical shaped text, pixels, clipping, and warm glyph images.

Observed results from a direct binary run under `/usr/bin/time -v`:

| Measurement | Result |
| --- | --- |
| Initial frame including font initialization | 28.12 ms |
| 500 warm display-list repaints | 11.56 ms |
| 500 changing styled labels | 45.17 ms |
| 1,000 uncached label rasters, previous algorithm | 29.38 ms |
| 1,000 uncached label rasters, compact algorithm | 32.62 ms (1.110x) |
| Same sample allocation, previous / compact | 40,960 / 24,576 bytes (-40%) |
| Churn retained raster / glyph image payload | 24,576 / 7,463 bytes |
| Churn known payload peak | 32,039 bytes |
| Whole benchmark process maximum RSS | 13,420 KiB |

A preceding run measured a 1.109x uncached-label ratio. Whole-process maximum
RSS includes test infrastructure, fonts, both comparison renderers, and temporary
buffers; there is no before/after RSS comparison. The cold algorithm comparison
excludes shaping and initial font loading. Native GPU/VRAM and Windows evidence
remain outside these software unit tests.

## Ownership and bounds

Both plain and styled rendering enter the same per-glyph cache policy. Retained
image data capacity is capped at 2 MiB and 2,048 entries, including negative
lookups. The entire Swash owner is also replaced after 2,048 cache misses so its
opaque scaler state cannot accumulate indefinitely across generations. Scale
changes and suspension replace the owner; destruction drops it. The shared
process font system is unchanged.

Swash has no preallocation limit for a single glyph: one rasterized glyph may
temporarily exceed 2 MiB. Its actual data capacity is recorded in the transient
peak, rendered, and its owner immediately retired. Hash-table control/header
storage and private scaler scratch are opaque and are not included in known
payload bytes. Entry/generation caps constrain those owners, not their exact
allocation size. Shaping buffers and input display lists are separate from the
raster cache and may still scale with input text.

Raster samples use 12 bytes each, preserving full RGBA colors and integer
coordinates. Candidate capacity plus existing raster/decoration capacities is
limited to 2 MiB. Admission uses capacity, and overflow discards the candidate
while continuing direct pixel drawing. An unchanged rejected command remembers
the decision and allocates no candidate on subsequent repaints. Styled
decorations stream directly too; retained decoration capacity shares the same
budget. Cached rasters reference the preceding production command identity
without duplicating text/spans. Per-command slot metadata is separate from the
2 MiB sample/decoration payload budget and scales with the display list.

`software_raster_diagnostics()` exposes glyph hits/misses, resets/rejections,
glyph transient peak, candidate capacity peak, cumulative candidate allocation,
known cache peak, and rejected raster count. `cache_diagnostics()` reports actual
known retained payload; `pixel_capacity_bytes()` reports framebuffer capacity
separately. Full invalidation preserves command/raster identity while marking
the framebuffer invalid. Empty valid frames remain clean.

## Regression evidence

Focused renderer tests compare pixels with Cosmic Text's previous callback
path for Unicode, sparse/dense labels, fractional origins/clipping, and several
font sizes. Emoji use available system fallback fonts; this is not a guarantee
that a particular installed color font was selected. Separate styled tests
cover colored bold/italic/monospace spans, strikes, and all underline styles
across cold and cached repaints. Plain, styled, and mixed size churn exercise
the common glyph policy, reset recovery, and suspension release.

Oversized label tests exceed the candidate budget, verify direct rendering and
bounded capacity, then repeat and verify zero further candidate allocation.
Scale-only tests compare identical commands against a new renderer at 0.75,
1.25, and 2.0 scales, including plain/styled text, filled geometry, unchanged
resize, and suspension recovery. Clipped command replacement cannot reuse old
pixels. Physical multi-DPI session testing remains a manual acceptance step.
