# Bounded peripheral MCP controls

`read_peripheral_controls` exposes fixed printer, print-job, and removable-volume
states under a full-session Full Control & Debug Nickel lease. Each read creates
observation-local opaque IDs and a fresh generation. The projection contains no
printer or job names, native IDs, device or mount paths, addresses, provider
messages, filesystem inventory, credentials, or other text from the host.

`control_peripherals` accepts only setting an observed printer as the default
and cancelling an observed print job. Both actions require the exact generation,
opaque IDs, and prior state from one unconsumed observation. Printer installation,
removal, test pages, volume mount/unmount/eject, cleanup, and arbitrary native
commands are outside this MCP surface.

On Linux, the compositor production owner admits the operation and retains shared
input ownership while the platform worker re-observes the native identity and
prior state. The worker continuously checks the original request deadline,
transport lifetime, lease/watch generation, emergency state, and shared-input
ownership. Cancellation kills the owned process group. A fresh native readback
is required for `confirmed`; otherwise an accepted operation reports
`requested` or `uncertain` and must not be retried without a new read.

The current Windows print and storage APIs do not provide a safely cancellable
production boundary for this MCP path. Windows therefore returns bounded
`unavailable` diagnostic domains and `unavailable` control outcomes rather
than dispatching an uncancellable spooler or PowerShell operation.
