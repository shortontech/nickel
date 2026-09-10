# Nickel changes to pipewire-native 0.1.4

Source: crates.io pipewire-native 0.1.4, MIT, from repository commit
`2d0a70fee2ec33980008aedd56d18a3eed64111c` (crate `.cargo_vcs_info.json`).
The crate archive omitted its workspace-root license; `LICENSE` is copied verbatim
from https://gitlab.freedesktop.org/pipewire/pipewire-native-rs/-/raw/2d0a70fee2ec33980008aedd56d18a3eed64111c/LICENSE .
Original source copyright headers retained.

- Add `Core::try_flush_pending_once(max_pending)`: one nonblocking socket write,
  no callbacks, no retry; returns remaining bytes, with a 64 KiB maximum queue.
  This permits a dedicated control connection to retain authorization at actual
  socket acceptance instead of merely buffering a command with `set_param`.
- Existing event-loop flush and ordinary local audio behavior are unchanged.
- Unit tests exercise partial writes, backpressure, queue bounds, and disconnect
  discarding unsent data. These do not alone prove remote device authorization.

- Add per-connection `Core::set_dispatch_limits`: at most 128 messages and
  256 KiB per I/O callback, with incoming frames capped before receive-buffer
  growth at 64 KiB. Guarded connections opt in; ordinary defaults stay unchanged.
  Buffered messages are rescheduled after a budget yield so outer cancellation
  and deadlines regain control without losing buffered events.

- Add `Context::connect_timeout`, used only by guarded device connections.
  It shares one deadline across runtime/system socket candidates, connects with
  nonblocking async-io, checks socket SO_ERROR and deadline after readiness, and
  drops timed-out futures/sockets. Failed Core construction clears its proxy
  ownership graph and unsent buffers. Ordinary `connect` remains unchanged.
  Owned Unix-listener saturation tests verify canceled/expired attempts do not
  appear later after backlog space opens and that subsequent requests recover.
