# Session recovery

The `nickel` executable owns both the native Linux compositor session and the user-facing shell;
`nickel-nested` provides the winit development session. Both launch host shell surfaces inside the
compositor process and communicate through typed in-process authority.

There is no independently restartable shell process, PID registration barrier, or `--role shell`
recovery path. When the active shell package fails, the compositor shows its own recovery panel.
Retry reloads the failed package in place without restarting the compositor or its applications;
if loading fails again, recovery remains available. Desired plugin activation is preserved.
The panel also offers safe logout. It is not a Wayland client and remains available when normal
shell presentation cannot be drawn. System virtual-terminal chords remain available.

XWayland is supervised separately. A failed XWayland process is torn down and restarted without
ending the Wayland compositor or its native clients. Optional login services publish explicit
readiness states; failure is reported to the shell and retried without silently replacing the
configured provider.

Historical shell-child recovery evidence predates the unified runtime and no longer describes a
supported execution mode. XWayland recovery remains independently testable.
