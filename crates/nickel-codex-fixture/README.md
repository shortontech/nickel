# Codex protocol fixtures

The fixture crate has two deterministic layers:

- checked-in domain scenarios consumed by `ReplayBackend`; and
- the `nickel-codex-fixture app-server` Rust child, which exercises real pipes, framing, process exit,
  request correlation, and bounded cleanup.

Canonical acceptance maps as follows:

| Case | Oracle |
| --- | --- |
| Initialize, account, models, empty threads | `basic.json` and `real_stdio_process_supports_typed_lifecycle_and_streaming` |
| Thread start, streamed successful turn | Rust child lifecycle test |
| Resume then new turn | typed `resume_thread` fixture response plus lifecycle test |
| Interleaved message, command, file, plan, reasoning | `interactions.json` |
| Command approval accept, decline, cancel | approval serialization and process interaction tests |
| File approval accept and decline | approval serialization and `interactions.json` |
| Structured user input | `interactions.json` and wrong-kind response test |
| Interrupt racing completion | completed-turn fixture plus explicit interrupt acceptance |
| Authentication and account update | `basic.json` and safe isolated live tier |
| Additive unknown event | `failure.json` |
| Malformed, invalid, oversized, conflicting ID | adversarial process and transcript validation tests |
| Stderr/read/write/EOF/crash/shutdown | bounded process lifecycle tests |
| Slow consumer/backpressure | flood mode and projected-state oracle |

`transcript-basic.json` demonstrates the direction/sequence/logical-time raw protocol format. Fixture
validation rejects secret-bearing fields, private absolute paths, unreviewed transcript methods, and
sequence gaps. Ordinary tests never record live sessions.

Run everything offline with:

```text
cargo run -p nickel-codex-fixture -- validate crates/nickel-codex-fixture/fixtures
cargo test -p nickel-codex -p nickel-codex-fixture
```

Review upstream drift and retain a versioned report with:

```text
cargo run -p nickel-codex-fixture -- compare-schema --codex /absolute/path/to/codex --out report.json
```

The report records executable, generated-schema, and accepted-profile digests plus additive methods.
