# Session recovery

`nickel-session` supervises the user-facing shell independently of the compositor. An unexpected
shell exit does not close application clients or end the login session. Restarts use a bounded
one-to-four-second delay; a shell that remains healthy for thirty seconds clears the consecutive
failure count.

After three consecutive failures the compositor presents its own recovery panel on every output.
This panel is not a shell client and remains available when the shell executable cannot start.
While it is visible, ordinary keyboard, pointer, and touch input is withheld from application
clients. `Enter` requests an immediate supervised restart and `Escape` terminates the compositor
session cleanly so the display manager can return to its greeter. System virtual-terminal chords
remain available.

XWayland is supervised separately. A failed XWayland process is torn down and restarted without
ending the Wayland compositor or its native clients. Optional login services publish explicit
readiness states; failure is reported to the shell and retried without silently replacing the
configured provider.
