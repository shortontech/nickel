# Typed device settings

`read_device_settings` reads one bounded domain: audio volume/mute, Wi-Fi power,
or Bluetooth power/discovery. `control_device_settings` accepts one typed action,
an observation generation, and the complete prior values. Both require an active
full-session lease with Full Control & Debug Nickel. Windows reports unavailable.
No network names, addresses, device identifiers, pairing secrets, credentials,
commands, or arbitrary paths are accepted or returned.

Linux reads native state off the compositor through the existing bounded native
transports. Unavailable or incomplete observations fail rather than inventing
default values. The compositor retains at most 16 observations and 16 pending or
standing origins. Each observation generation is consumed at admission. A native
control revalidates the complete observed values and private target incarnation:
PipeWire server cookie and node serial, or D-Bus unique owner and adapter path.
Multiple Bluetooth adapters are explicitly ambiguous in this bounded slice.

This is observed-value revalidation, **not atomic native compare-and-set**.
PipeWire and D-Bus provide no atomic transaction against other native clients.
Concurrent changes after the final native observation remain possible; a readback
that does not confirm the requested effect returns `requested` or `uncertain`.
`confirmed` describes native observed state or owned discovery acknowledgement,
not rendered presentation. Do not automatically retry an uncertain outcome.

Controls reuse the production guarded device worker, original request deadline,
lease, watch incarnation, emergency cancellation and compositor input authority.
Typed writes recheck full-debug input authority at every native write. Physical
input, lock/recovery, and protected shell transitions invalidate owner tickets;
failed or abandoned request lifetimes cannot produce a later native write.
A queued action's result is revalidated by the compositor before acknowledgement.

Accepted discovery retains its same-sender connection beyond the initial request
only while the original approved lease/watch and owner ticket remain valid.
Stopping discovery releases this lease's own sender; it does not stop discovery
owned by another application. Other clients may therefore keep global discovery
active. No additional lease renewal or independent expiry is introduced.

This slice does not expose device selection, saved networks, Wi-Fi credentials,
Bluetooth pairing, connected-device addresses, or platform permissions.
