# Codex Backend Diagnostics

Codex support is deliberately testable without Nickel UI. These commands validate offline replay and
probe a CLI without starting a model turn:

```bash
cargo run -p nickel-codex-fixture -- validate crates/nickel-codex-fixture/fixtures
cargo run -p nickel-codex --bin nickel-codex-test -- replay crates/nickel-codex-fixture/fixtures/basic.json
cargo run -p nickel-codex --bin nickel-codex-test -- probe --backend installed
cargo run -p nickel-codex-ui -- --replay crates/nickel-codex-fixture/fixtures/basic.json
```

`nickel-codex-test` emits versioned JSONL on stdout. Installed Codex is preferred only after generated
schema and initialization compatibility checks; release builds retain a pinned bundled fallback.
Nickel delegates account authentication to the experimental Codex app-server login RPC and never
handles passwords or raw OpenAI credentials itself. To test a clean profile without touching the
ordinary Codex profile, set an absolute child-only override:

```bash
mkdir -p /absolute/private/test-profile
NICKEL_CODEX_HOME=/absolute/private/test-profile cargo run -p nickel-codex-ui -- --backend installed
```

Only the spawned Codex app-server receives `CODEX_HOME`; Nickel, probes, and the parent environment
remain unchanged.

An authenticated first turn must be started on the same app-server connection that creates its thread;
subsequent one-shot turns resume the persisted thread explicitly:

```bash
cargo run -p nickel-codex --bin nickel-codex-test -- start-thread --cwd "$PWD" --text "Hello"
cargo run -p nickel-codex --bin nickel-codex-test -- turn THREAD_ID --text "Continue"
```

The standalone graphical client runs independently of the Nickel shell:

```bash
cargo run -p nickel-codex-ui -- --backend installed
```
