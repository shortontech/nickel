# Contributing to Nickel

Thanks for helping build Nickel. Contributions to Rust platform code, JSX shell
packages, documentation, tests, packaging, accessibility, and hardware support
are all welcome. Issues labeled `good first issue` are intended to be reasonably
self-contained; issues labeled `help wanted` are available for contributors to
take on.

Before starting a large change, open an issue so the intended behavior and
platform impact can be discussed. Small fixes do not need advance approval.

## Development setup

Install stable Rust, clone the repository, and run commands from its root. The
main development entry points are:

```bash
cargo run -p nickel
cargo run -p nickel --no-default-features --features backend-winit --bin nickel-nested
cargo run -p nickel --bin nickel-plugin -- validate assets/plugins/nickel-default
```

The first command starts Nickel with the default platform configuration. The
second starts a nested Linux development session. The third validates the
default shell package; substitute another package directory when working on a
different shell or plugin.

See [Linux sessions](docs/linux-sessions.md), the [Cargo workspace
guide](docs/cargo-workspace.md), and the [plugin development
guide](assets/plugins/README.md) for more focused setup instructions.

## Architecture and scope

Nickel's trusted application and platform code is Rust. Rust owns application
state, policy, rendering, validation, effects, native resources, and platform
integration. Production shell presentation is authored in TSX and CSS under
`assets/plugins/` and runs through Nickel's capability-constrained host—not a
browser, Node, Deno, or React runtime.

Keep native policy, capability checks, protected data, and platform operations
out of package code. Prefer composition exports and replacements when extending
a shell instead of copying the default package. Request only the capabilities
and surfaces a package actually needs.

Portable behavior such as search, ranking, navigation, task switching, package
validation, and capability enforcement should remain independent of native
window APIs. Put platform-specific behavior behind narrow, explicit adapters.

Active design specifications live in `specs/`. Move a completed specification
to `specs/done/` when its work is finished.

## Making changes

- Follow standard Rust formatting and naming conventions.
- Keep unsafe code localized, document it with a `SAFETY:` justification, and
  add focused coverage.
- Use explicit `cfg(target_os = "...")` boundaries for platform-specific code.
- Add behavior tests with behavior changes. Prefer the semantic scenario
  harness in `nickel-core::scenario` for shell interaction behavior.
- Add focused runtime or presentation tests for JSX package loading,
  composition, capabilities, callbacks, layout, and effects.
- Validate every changed package and keep any checked-in generated JavaScript
  synchronized with its source. Package-specific READMEs document their
  regeneration commands.
- Record manual platform coverage where behavior depends on native shell
  ownership, focus, compositor integration, thumbnails, DPI, permissions, or
  multiple monitors.

Do not include secrets, credentials, personal data, build output, runtime
sockets, or unrelated generated files in a change.

## Checks

Run the checks relevant to the change. Before submitting a broad change, run:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

Also validate each shell package you changed, for example:

```bash
cargo run -p nickel --bin nickel-plugin -- validate assets/plugins/nickel-default
cargo run -p nickel --bin nickel-plugin -- validate assets/plugins/nickel-cupertino-dock
```

If a full workspace check is impractical on your platform, run the closest
focused checks and say clearly in the pull request what was and was not run.

## Commits and pull requests

Use concise, imperative commit subjects, such as `Add fuzzy ranking pipeline`.
Keep commits scoped and include tests with behavioral changes.

A pull request should:

- explain the user-visible behavior and motivation;
- link relevant issues or specifications;
- list automated checks and platforms tested;
- identify platform-specific behavior that remains untested; and
- include screenshots or recordings for visual changes.

Contributions are accepted under the repository's same dual-license terms:
[MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at the recipient's option.
