# Session recovery

The `nickel` executable owns both the Linux compositor session and the user-facing shell. Normal
`nickel --backend udev` and `nickel --backend winit` launches host shell surfaces inside the
compositor process and communicate through typed in-process authority.

There is no independently restartable shell process, PID registration barrier, or `--role shell`
recovery path. Restarting the shell therefore means restarting the session. The compositor retains
its own recovery panel for fatal internal runtime failures and safe logout. This panel is not a
Wayland client and remains available when normal shell presentation cannot be drawn. System
virtual-terminal chords remain available.

XWayland is supervised separately. A failed XWayland process is torn down and restarted without
ending the Wayland compositor or its native clients. Optional login services publish explicit
readiness states; failure is reported to the shell and retried without silently replacing the
configured provider.

Historical shell-child recovery evidence predates the unified runtime and no longer describes a
supported execution mode. XWayland recovery remains independently testable.
