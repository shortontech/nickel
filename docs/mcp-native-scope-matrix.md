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
harness tests and strict harness Clippy passed. Evidence:
`/tmp/nickel-native-ordinary-scopes-build.log`,
`/tmp/nickel-native-ordinary-scopes-focused.log`, and
`/tmp/nickel-native-ordinary-scopes-wayland.log`.

This proves ordinary native Wayland executable identity and later same-application
window admission for these real examples. It does not prove Flatpak/shared-runtime
or broker identity, remote application launch, transients, cross-output/workspace
movement, Xwayland application behavior, physical input, assistive workflows, or
native Windows acceptance.
