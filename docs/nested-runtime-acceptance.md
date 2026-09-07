# Nested runtime acceptance

Build and run the bounded live nested-session check with:

```sh
cargo build -p nickel --no-default-features --features backend-winit \
  --bin nickel --bin nickel-test-input --bin nickel-nested-acceptance
./target/debug/nickel-nested-acceptance
```

The harness creates a private `XDG_RUNTIME_DIR`, starts the unified `nickel`
binary with the winit backend and explicit test control, waits for authenticated
shell readiness, checks the compositor's shell-surface inventory, injects a Meta
key press and release, samples runtime wakeup diagnostics across a two-second
idle interval, and requests logout. Every phase has a deadline. On failure, the
harness terminates its compositor child and removes its temporary runtime data.

This is a live graphical acceptance check, so it requires a working host display.
The reported idle wakeup delta is diagnostic rather than a fixed performance
threshold: machine and renderer behavior differs, while an unbounded redraw bug
remains immediately visible in repeated measurements and CPU profiles.
