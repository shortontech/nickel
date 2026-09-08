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

## Remaining implementation

- 0220: wire native desktop topology, viewport selection, and input to production authorities.
- 0221: native media dispatch and status-driven OSD behavior.
- 0222: typed keyboard snapshots/effects and correct recipient epochs/gesture leases.
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
