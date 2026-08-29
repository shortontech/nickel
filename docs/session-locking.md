# Session locking

Nickel's Linux lock is compositor-enforced. Entering the locked state immediately removes ordinary
clients from pointer, touch, keyboard, preview-capture, and presentation authority. Every connected
output receives an opaque compositor cover; the trusted shell's `io.nickel.shell.lock` surfaces are
the only client surfaces composited above it.

The shell authenticates the current account through the host's existing `login` PAM service. The
adapter loads `libpam.so.0` at runtime, retains the password only in zeroizing buffers, and sends an
unlock request only after both PAM authentication and account checks succeed. Nickel does not store
passwords or implement an alternative password database.

Session-control datagrams carry Linux peer credentials. Lock, unlock, logout, restart-shell,
suspend, reboot, and power-off commands are accepted only from the currently supervised shell PID.
Application launches remove the session socket and token from their environment. An explicitly
enabled nested `--test-control` session may cross only the lock/unlock boundary for semantic tests;
it cannot invoke power or logout actions.

If the shell exits while locked, the compositor remains locked and opaque while the supervisor
starts a replacement. The replacement receives the locked snapshot, recreates a full-output lock
surface, and must authenticate normally. Output hotplug while locked creates or removes matching
lock surfaces without exposing the underlying desktop.

On 2026-08-29, nested acceptance locked a mapped native Wayland KCalc client, killed the shell with
`SIGKILL`, and captured the compositor output both before and during replacement. Client contents
remained opaque throughout; the replacement recreated the full-output lock UI, and an authenticated
nested test unlock revealed the same surviving KCalc process. The test capability does not exercise
PAM and therefore is not native authentication acceptance.

Native acceptance must still verify successful password unlock, suspend/resume while locked, and
the same invariants on every physical output from an SDDM-launched Nickel session.
