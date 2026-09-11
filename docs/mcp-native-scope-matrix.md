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

Ordinary window and application scopes remain uncovered: this fixture owns shell
resources and does not invent application identities. The new matrix does not
prove raw keyboard/pointer delivery, multiple-output confinement, workspace
movement, external application accessibility, a local assistive workflow,
physical keyboard/DRM behavior, or native Windows acceptance. The Wayland run
exercises nested presentation and compositor-owned shell semantics, not ordinary
Wayland application identity. Xwayland was not rerun for this increment.
