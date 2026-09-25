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
