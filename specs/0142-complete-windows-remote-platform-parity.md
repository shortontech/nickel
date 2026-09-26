# Spec 0142: Complete Windows Remote Platform Parity

## Problem

The Windows remote desktop interface can read display layout but rejects every layout
transaction. It returns unavailable printer and removable-volume snapshots even though Nickel
has native Windows peripheral adapters. It also rejects pointer targets on Nickel-owned
surfaces. These operations work through the Linux remote desktop interface. Separately, the
platform contract still records the implemented Windows image chooser as unsupported.

## Decision

Add Windows ownership paths for the three remote operations without bypassing lease,
resource-generation, protected-state, input-idle, deadline, or cancellation checks. Expose a
capability only when its production owner can complete and verify the operation. Keep the
Windows Settings display work in Spec 0138 coordinated with, but distinct from, the remote
display transaction API.

## Display layout transactions

- Use a Windows display owner with stable adapter/target identity and a fresh complete topology
  observation. The owner validates the prior layout, output incarnations, requested primary,
  enabled outputs, positions, and supported scale before changing hardware.
- Refuse requests that disable the last enabled output, touch unknown or retired outputs, or
  change fields the chosen Windows API cannot preserve.
- Apply one validated topology through the native Windows display API, then query it again.
  Report confirmed state only from that query; report failures and uncertain results honestly.
- Support `Apply`, `Keep`, and `Revert` with an owner-held, bounded recovery window. On expiry or
  lost authority, restore the confirmed prior layout when safe and report recovery state.
- Keep tentative `Apply` changes out of persistent Windows display settings. A process exit
  during the recovery window must not make the unconfirmed layout the saved configuration.
- Keep `transaction_supported` false with a specific reason on systems where the required
  native capability or ownership cannot be established.

## Printer and removable-volume remote controls

- Bridge the existing Windows printer and storage observations into bounded remote snapshots
  with opaque resource IDs, generations, truncation counts, and scrubbed diagnostics.
- Execute only the remote protocol's allowlisted actions. Revalidate the target and lease at the
  final commit boundary and verify the result with a fresh native observation.
- Give native calls a bounded, cancellable production owner. A call that cannot be cancelled or
  safely bounded remains unavailable for remote mutation; an ordinary Settings or local native
  adapter is not sufficient evidence for remote safety.
- Distinguish provider absence, permission denial, stale target, rejection, timeout, and uncertain
  completion without exposing raw printer names or volume paths through remote diagnostics.

## Pointer targets on Nickel surfaces

- Resolve a remote `Surface` target against the Windows shell's current surface identity,
  generation, placement, and output. Reject stale, protected, hidden, or out-of-bounds targets.
- Translate local surface coordinates through production geometry at execution time. Verify
  target authority again before input injection; a replacement surface must not inherit an old
  request.
- Preserve the existing Windows handling of ordinary-window and output targets.

## Platform contract and evidence

- Update `platform_contract.rs` and `assets/ui-platform-contracts.tsv` so Windows
  `ImageFileDialog` names the implemented common item dialog adapter and its current fixture
  evidence. Keep fixture and native live evidence distinct; do not mark a capability live-verified
  from compilation or a synthetic test.
- Add or extend contract entries for remote display transactions, remote peripherals, and remote
  surface pointer targets so availability and evidence cannot silently diverge again.

## Verification

1. Exercise the same remote protocol scenarios on Linux and Windows fixtures: stale generations,
   protected surfaces, revoked leases, busy input, unsupported capabilities, bounds, timeout,
   and recovery. Use production reducers and hit testing for shell interaction scenarios.
2. Test Windows display validation with synthetic negative coordinates, mixed scale, disabled
   outputs, primary changes, topology replacement, and final-display refusal. Native mutation
   tests require opt-in, capture the prior topology, and restore it after every result.
3. Test Windows peripheral observations and controls with missing devices, permissions, target
   disappearance, slow providers, cancellation, and late completion. Native changing tests require
   opt-in and must restore reversible fixture state.
4. Test surface pointer identity and coordinate mapping across multiple outputs, scale changes,
   hiding, destruction, and surface replacement. Run native Windows focus and DPI coverage.
5. Run Windows and Linux target builds, affected tests, formatting, and strict Clippy. Record
   native results separately from fixture and cross-target compile evidence.

Completion requires supported Windows remote operations to be observed and verified by their
production owner. An operation without a safe native owner must remain explicitly unavailable
with an accurate reason and contract evidence.

## Current verification record

- Windows fixture tests pass for display topology validation, active-only fallback, surface
  pointer coordinates and current-observation revalidation across scale, visibility, output,
  replacement, protection, destruction, and bounds; peripheral projection; and the platform
  contract. The Windows library
  compiles and formatting passes.
- A read-only native DisplayConfig probe found an available target outside the active monitor
  inventory on the current Windows setup. Remote snapshots report incomplete topology and
  refuse layout transactions there. On 2026-09-25, the probe found one active monitor, two
  available targets, and 150 possible paths; the inactive available target returned a monitor
  device path and friendly name; its monitor device path differs from the active target's path.
  A repeat read-only probe on 2026-09-25 still found one active and two available targets and reports the specific reason
  `Windows available display target is inactive`. The opt-in native display mutation test has
  not run. [Windows documents](https://learn.microsoft.com/en-us/windows/win32/api/wingdi/ns-wingdi-displayconfig_path_target_info)
  `targetAvailable` as availability, which alone does not prove physical connection.
  [QueryDisplayConfig](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-querydisplayconfig)
  does not return source or target mode information for inactive paths, which this owner would
  need to stage and verify their placement. The probe read the path and friendly name through
  [target device information](https://learn.microsoft.com/en-us/windows/win32/api/wingdi/ns-wingdi-displayconfig_target_device_name).
- The display owner now validates and applies a supplied DisplayConfig without
  `SDC_SAVE_TO_DATABASE`; Keep saves the confirmed layout and queries the saved configuration.
  Guarded rollback restores the captured active DisplayConfig temporarily. If Keep attempted a
  database write, rollback first restores the separately captured saved configuration. This
  closes the earlier tentative-save gap described by
  [ChangeDisplaySettingsEx](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-changedisplaysettingsexa)
  and [SetDisplayConfig](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setdisplayconfig).
  Synthetic source-mode transformation and read-only native active/database queries pass on
  Windows. The read-only native `SetDisplayConfig` validation call also passes. Transaction
  support is now reported separately from topology completeness and requires readable active and
  saved modes plus successful native validation. A native changing round trip remains unverified
  on a complete multi-monitor fixture.
  Windows placement validation also rejects a requested layout with a nonprimary output at the
  desktop origin, where the position-based native path cannot identify the requested primary
  unambiguously. The focused validation fixture passes.
  An opt-in native read through `WindowsDesktopAuthority::read_display_layout`, a live in-process
  Full Control & Debug lease, and the production output reconciliation path passed. It reported
  one output, incomplete topology, `transaction_supported=false`, and the specific inactive
  target reason. An authenticated Apply request through the same owner and production output
  reconciliation path returned that exact reason without changing the display. This verifies
  the production owner's read and refusal on the current fixture; it does not verify a changing
  transaction. The platform contract now records display observation separately as
  `native_read_verified`; display transactions remain `fixture_only` until a changing native
  round trip passes. Linux display observation retains fixture evidence under the user's Linux
  test exclusion.
- Windows fixture tests cover final-authority loss after a native Apply or Revert. Apply retains
  its recovery plan for immediate rollback; a verified Revert clears recovery state even if the
  request reply expires. Physical input epoch and idle state are rechecked during Apply staging.
  The native Apply helper also refuses an incomplete or transaction-unavailable observation before
  any staging, even if a future caller bypasses the desktop owner's early check. The focused
  Windows fixture and strict Clippy pass.
  Apply and Keep recheck the recovery deadline after synchronous native work and readback, so a
  slow call cannot report a usable confirmation window after it has expired. An expired Keep
  leaves the guarded rollback pending, including when the native save had already succeeded.
- Windows printer and removable-volume reads use the tested single-flight bounded platform
  observation worker. Its fixture test covers deadline expiry, rejection of overlapping reads,
  and release after a late native completion. Printer mutations remain unavailable because the
  native spooler calls lack a cancellable production owner; the platform contract and scrubbed
  remote snapshot/outcome record that limitation. Shared outcome mapping preserves busy,
  permission, unsupported, and rejection categories without returning native details. The
  Windows unavailable-control path drops native printer and job lookup IDs after projecting its
  scrubbed snapshot. A shared fixture also verifies that accepted printer/job actions require
  fresh native confirmation and that invalidation rejects an old opaque target.
- The exact Windows workspace strict Clippy command and formatting check pass. Focused Windows
  screenshot controller and screenshot command tests pass. All 17 focused Windows remote-owner
  tests pass. `cargo test --workspace --no-run` builds every workspace test target on Windows;
  it does not execute the suite. Native focus and DPI results are recorded below. Peripheral
  changing coverage remains pending. Linux tests are excluded by the current user instruction.
- The Windows image chooser contract now cites a focused fixture that distinguishes a native
  cancellation from a native failure. The chooser fixture and both platform-contract tests pass
  on Windows. This is fixture evidence; opening and using the native dialog remains unverified.
- The contract's Windows surface-pointer fixture now has a recorded Windows test run; it passes
  for logical-to-client coordinate conversion. The pointer owner also reprojects current shell
  ancestry at each target resolution, including after cursor movement, and rejects a changed
  parent incarnation before input injection. A Windows fixture passed for parent replacement.
- A native Windows test creates a hidden, non-activating window under per-monitor DPI awareness,
  reads its actual DPI and client bounds, and verifies the production surface coordinate mapper
  stays inside that client area while the window remains outside foreground focus. It passes on
  Windows. A separate opt-in, read-only native run selected a live Nickel Panel by PID and HWND,
  observed DPI 96 with a 1920×56 client area, and found an exposed point through the production
  logical-to-client mapper and native occlusion check. The Panel was not foreground; the run made
  no focus change. A separate opt-in native focus test used Nickel's Windows focus adapter to
  activate an offscreen fixture and restore the prior foreground window; it passed. With
  `NICKEL_WINDOWS_SURFACE_POINTER_MOVE_TEST=1`, the live Panel test also used Nickel's native input
  adapter to move to the exposed point, confirmed the WindowFromPoint hit, restored the prior cursor
  position, and kept foreground focus unchanged. A first attempt stopped at the physical-input-idle
  guard before movement; the next guarded attempt passed. The test Nickel process used
  `--no-desktop-windows` and was closed afterward. An additional opt-in native test created a
  temporary Nickel shell without desktop surfaces, acquired a live Full Control & Debug lease,
  and sent a Surface Move through `WindowsDesktopAuthority::pointer_action` and the production
  owner poll. The owner moved to the visible Panel point; the test confirmed the native hit,
  restored and checked the original cursor position, and kept foreground focus unchanged. The
  initial fixture attempt correctly revoked its permit because the test owner lacked a desktop
  session; the fixture now supplies the process session. A later attempt correctly refused movement
  while physical input was active; the next
  idle attempt passed and checked cursor restoration. The contract cites
  this owner-level `native_input_verified` result. A network listener request, focus changes on
  a Nickel-owned surface, click/drag input, and scale changes across monitors remain unverified.
- After the Windows peripheral owner and truncation changes, `cargo test --workspace --no-run`
  compiled every workspace test target and `cargo build --workspace` passed on Windows. The broad
  test suite was not executed; focused Windows suites and opt-in native reads are recorded above.
- After the native display owner test and the observation/transaction contract split,
  `cargo test --workspace --no-run` again compiled every workspace test target on Windows and
  `cargo build --workspace` passed. The broad test suite remains unexecuted because an unrelated
  Windows test invokes `LockWorkStation`; only the focused suites and opt-in native tests noted
  here were run.
- Focused Windows suites now pass end to end: display topology (6 fixtures; 3 native opt-in tests
  ignored by default), peripheral projection/control contracts (8), resource ownership and pointer
  bounds (23; 1 live opt-in test ignored by default), and shell diagnostics (12). These fixture
  results include stale identity, protection, bounds, recovery, native-owner limitations, and
  scrubbed observations; they do not substitute for a changing display or peripheral run.
- The added peripheral fixture verifies that an empty printer and volume inventory stays
  available, while a printer provider failure leaves volume availability intact and omits the
  provider's private diagnostic text. A second fixture caught and verifies the corrected
  `omitted_print_jobs` count when a printer is truncated by the remote limit. Both pass on Windows.
- An opt-in native peripheral read passed through the production bounded refresh worker and
  scrubbed remote projection. The current Windows fixture reported two printers and one removable
  volume, with only generated opaque IDs in the remote entries. The platform contract now records
  remote peripheral observations as available separately from unavailable remote mutations. It
  records the bounded native read as `native_read_verified`. A complete live remote request is
  unavailable in this build because `RemoteControlRuntime::apply` deliberately disables the MCP
  listener on both platforms. The contract's `available` field describes the adapter, not endpoint
  reachability; the focused Windows `shipped_runtime_keeps_remote_listener_disabled` test passes.
  No changing peripheral operation or complete live request was verified. The
  opt-in test calls the same bounded native read helper as the Windows desktop
  authority, preventing its native observation path from drifting from the production request.
- An additional opt-in native Windows test exercised `WindowsDesktopAuthority::read_peripheral_controls`
  directly with an in-process live Full Control & Debug lease and connection watch. It verified
  the opaque remote snapshot, rejection of a lease without Debug access, and rejection after
  authority was disabled. No listener, visible shell, pointer movement, or peripheral mutation
  was involved. This production-owner read is the contract's `native_read_verified` evidence.
