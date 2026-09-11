# Native scope matrix: first increment

The checked-in Rust `nickel-linux-remote-control-acceptance` harness now exercises
repeated semantic input under exact shell-surface, output and full-session leases.
This is partial evidence for Spec 0230 verification items 3, 4 and 6; it does not
complete the native scope acceptance gate.

The harness opens its own nested launcher's production UI through the trusted
local session protocol. A bootstrap lease observes the real launcher surface and
its generation-bearing output, then is revoked. Each scope receives one local
approval, with exactly one active lease and no pending request. For each scope,
three distinct search edits use fresh semantic tree generations, and subsequent
observations verify the actual search text. The exact surface lease also denies
inspection of the separate panel.

After each edit, the local authoritative snapshot must retain the same lease,
scope and debug level, no pending requests, and unchanged permission/lease audit
history and eviction counts. This detects transient permission requests even when
their cards have disappeared before the next observation. Ordinary countdown of
the existing lease is allowed. No broader fallback lease remains during the
narrow-scope assertions.

The native Wayland run passed on September 11, 2026, using the worktree based on
`1c180ab` plus this change. All five focused harness tests and strict harness Clippy
passed. The full native run also retained the existing preapproval tool-denial,
privacy, renderer-diagnostic and emergency-revocation checks. Its compositor and
temporary runtime were cleaned up afterward.

Build and run from a reachable Wayland session:

```sh
cargo test -p nickel --features backend-winit --bin nickel-linux-remote-control-acceptance
cargo clippy -p nickel --features backend-winit --bin nickel-linux-remote-control-acceptance -- -D warnings
cargo build -p nickel --features backend-winit --bin nickel --bin nickel-linux-remote-control-acceptance
target/debug/nickel-linux-remote-control-acceptance
```

The commands above are the reproducible evidence path. A missing host backend
produces `SKIP`, which must not be counted as native acceptance.

Without `--ordinary-scopes`, ordinary window and application scopes remain
uncovered: the default fixture owns shell resources. The shell matrix does not
prove raw keyboard/pointer delivery, multiple-output confinement, workspace
movement, external application accessibility, a local assistive workflow,
physical keyboard/DRM behavior, or native Windows acceptance. The Wayland run
exercises nested presentation and compositor-owned shell semantics, not ordinary
Wayland application identity. Xwayland was not rerun for this increment.

## Ordinary Wayland clients

The optional `--ordinary-scopes` increment uses the unchanged repository examples
`nickel-ui/examples/keyboard_recipient.rs` and `standalone.rs`. Build these beside
the compositor and harness:

```sh
cargo build -p nickel-ui --example keyboard_recipient --example standalone
target/debug/nickel-linux-remote-control-acceptance --ordinary-scopes
```

The harness starts the examples on its private nested Wayland socket. A bootstrap
lease obtains real window generations and `verified_application` identities from
the production compositor; titles only locate the expected fixtures and never
establish authority. The bootstrap lease is revoked before narrow assertions.

One window lease repeatedly focuses the keyboard recipient and sends three native
keys. Fresh owner observations confirm focus, and the actual client's bounded
stdout receipt confirms each resulting text change. One application lease then
repeats the actions and admits a second keyboard-recipient process/window created
after approval, using the same native executable identity without another prompt.
Its inventory contains exactly those two windows.

The distinct counter executable remains excluded. Focus, capture and keyboard
requests must fail with the resource-boundary denial. Before the keyboard negative
control, the counter is focused locally and its live focus is verified through
the trusted owner query, eliminating lack of focus as an alternative rejection
reason. Every phase retains exactly one active lease and unchanged local approval
history. All fixture processes are killed and reaped on both success and failure.

Native Wayland acceptance passed September 11, 2026, on `2426214` plus this change,
including the complete earlier shell/privacy/emergency checks. Five focused
harness tests and strict harness Clippy passed. The commands above reproduce the
acceptance path.

This proves ordinary native Wayland executable identity and later same-application
window admission for these real examples. It does not prove Flatpak/shared-runtime
or broker identity, remote application launch, transients, cross-output/workspace
movement, Xwayland application behavior, physical input, assistive workflows, or
native Windows acceptance.

## Output and workspace movement

Run the ordinary-client matrix with `--movement` to include movement acceptance:

```sh
target/debug/nickel-linux-remote-control-acceptance --movement
```

This implies `--ordinary-scopes` and requires the same two repository examples.
Before startup, the harness creates one desktop entry for the real counter
example under its private `XDG_DATA_HOME`; it does not install a host application.
The production catalog must report the same verified executable identity as the
live counter window before the negative launch test is admitted.

The nested test-control capability creates a second Smithay output, publishes its
Wayland output global and runs production relayout. The harness creates a workspace
through the production session command and uses its returned authoritative state.
The resulting tests prove:

- A window lease and an application lease each retain native key delivery after
  the window moves to the second output, changes workspace there, and returns to
  the original workspace/output. Trusted owner queries confirm full geometry
  containment and workspace membership; the actual client confirms each key.
- An output lease permits input on its output, omits the moved window from its
  inventory and denies focus/capture/keyboard outside that output, then permits
  native input again on return using the same lease. The outside keyboard target
  is focused locally first to rule out a focus-only rejection.
- The application lease cannot enumerate or launch the unrelated real catalog
  executable after movement. Launch must fail specifically because the target is
  outside the application lease, not because its catalog entry is absent/stale.
- No movement requests another approval or changes the existing authority audit.

Native Wayland acceptance passed September 11, 2026, on `d2ca964` plus this change.
All preceding ordinary/shell cases and the combined diagnostic/capture/input stress,
privacy and emergency checks also passed. Five focused harness tests and strict
harness Clippy passed. Evidence: `/tmp/nickel-native-movement-build.log`,
`/tmp/nickel-native-movement-focused.log` and
`/tmp/nickel-native-movement-wayland.log`. The added workspace/output were removed,
all example processes reaped, and the owned compositor/runtime cleaned afterward.

The extra output is virtual: it exercises live Wayland clients and production
compositor resource/geometry/workspace authority, but has no independent physical
display presenter. This is not physical multi-monitor, DRM, mixed-DPI presentation,
Xwayland, or Windows acceptance. Continuous held-input movement and transient/broker
identity remain separate cases. The real negative launch test does not prove an
authorized application launch or output-placement workflow.
