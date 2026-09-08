# Vendored Smithay

Source: https://github.com/Smithay/smithay.git

Revision: `e3d461a057ba244d213a8498ec372b0799cca103` (Smithay 0.7.0).
Upstream license is retained in `LICENSE.txt`.

This directory contains the crate's source, manifest, build script and its two
GBM C feature probes, README and license. Examples, benchmarks, sibling workspace
packages and Git metadata are omitted; corresponding manifest entries are removed.
The root Cargo git patch preserves the original pinned dependency declaration.

## Local patch: opt-in nonblocking frame finalization

`GlesFrame::try_finish` shares normal finalization bookkeeping but returns
`GlesError::SyncExportFailed` when completion-fence export fails. It never uses
the ordinary `glFinish` fallback and omits GPU profiler clock calibration.
Normal `Frame::finish` and unfinished-frame drop retain upstream behavior.
The finished flag is set before bookkeeping, so an unsuccessful consuming finish
cannot make Drop retry finalization with the blocking policy.

Target texture synchronization, GL state restoration, profiling span closure,
and deferred resource cleanup remain intact. Earlier draw/import calls still
have their own requirements: Nickel must require both `Fencing` and `ExportFence`
before attempting optional native preview rendering. Server-side texture waits
are not removed. No wall-clock bound on opaque driver calls, imports, locks or
cleanup is claimed. GL texture FenceSync failure handling remains upstream's
existing behavior, not a new guarantee of this patch.

The `frame_finish` unit tests exercise the production fallback policy without a
graphics device. Nickel's preview-submission helper separately tests failed-draw
and failed-finish ordering. These are not live GPU or frame-drop integration tests.

Keep changes limited to `gles/mod.rs`, `gles/error.rs`, and `gles/frame_finish.rs`
when rebasing this API patch. Remove the vendor override once an upstream version
provides the same opt-in contract and its error paths have been re-audited.

## Validation at initial integration

- Standalone `cargo test --lib --no-default-features --features renderer_gl
  frame_finish::tests`: two production-policy tests passed without a graphics device.
- Nickel `cargo clippy -p nickel --all-targets --all-features -- -D warnings`: passed.
- Nickel `cargo test -p nickel --lib --all-features preview`: 47 passed, two
  release-only timing workloads ignored.
- `cargo tree -p nickel -i smithay`: selects only this vendored Smithay instance.
- Nickel workspace formatting passed. The upstream-relative API patch has no
  whitespace errors; the vendor import preserves 16 existing upstream trailing-
  whitespace lines rather than modifying unrelated source or disabling checks.

These checks prove compilation and synthetic behavior, not native GPU timing,
driver fence-failure recovery, or live compositor input/presentation acceptance.
