# Nested runtime acceptance

Build and run the bounded live nested-session check with:

```sh
cargo build -p nickel --no-default-features --features backend-winit \
  --bin nickel --bin nickel-test-input --bin nickel-nested-acceptance
./target/debug/nickel-nested-acceptance
```

The harness creates a private `XDG_RUNTIME_DIR`, starts the unified `nickel`
binary with the winit backend, explicit test control, and `--shell-process
disabled`, then waits for compositor-owned shell readiness. It asserts that no
shell PID is expected or authenticated and no `--role shell` child exists,
checks the internal surface inventory, injects Meta and verifies that the
internal launcher becomes visible, samples compositor CPU ticks across a
two-second idle interval, and requests logout. Every phase has a deadline. On
failure, the harness terminates its compositor child and removes its temporary
runtime data.

This is a live graphical acceptance check, so it requires a working host display.
The idle check allows up to one fully occupied CPU core across its two-second
window (on the Linux 100 Hz process clock), a deliberately broad bound intended
to catch an unbounded redraw loop without imposing a benchmark-grade threshold.
