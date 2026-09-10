# MCP event subscription acceptance

Validated on 2026-09-10 against a locally built Linux Nickel compositor using
`--backend winit --test-control`, Mesa software rendering, an isolated Xvfb
display, and Nickel's own Xwayland server. The listener used
`127.0.0.1:42639`. The user's desktop session was not used for input or lifecycle
tests. Lease decisions went through the trusted local session control endpoint.

## Native results

- A full-debug lease reads the bounded event resource and receives an initial
  resource-update notification. Creating a real X11 window produces a subsequent
  update through the production compositor event collector.
- Pausing the lease terminates its subscription and denies resource reads.
  Resuming permits fresh reads and a new subscription; it does not revive the
  old stream.
- An ordinary full-session lease cannot read or subscribe to debug history.
  Another authenticated client cannot use a known URI containing someone else's
  lease ID. Rejected subscriptions deliver no resource-update notifications.
- One client cannot hold duplicate subscriptions. Five independently approved
  clients cannot exceed the global limit of four streams.
- Closing an idle HTTP connection releases both global and per-client capacity.
  A new subscription succeeds after a 300 ms settling interval, without waiting
  for the stream's maximum lifetime.
- A quiet subscription completes after 60 seconds while the underlying lease
  remains valid. Its terminal response reports `resultType: complete`.
- Disabling the listener with a stream active returns through local control in
  under one second and ends the HTTP stream in under three seconds. Restarting
  rejects the old authority and admits a newly approved subscription.
- Resource template discovery returns `nickel://desktop-events/{lease_id}`.

The current SDK can acknowledge the requested subscription filter before the
application checks authority. An acknowledgement is not permission to read the
resource; unauthorized requests subsequently fail without event notifications.
Notifications contain the resource URI, not event payloads. Each resource read
and each compositor event poll checks current authority.

## Protocol and evidence

The native client used MCP `2026-07-28`, `subscriptions/listen` with
`notifications.resourceSubscriptions`, and the required `Mcp-Method` HTTP header.
Resource reads also supplied `Mcp-Name` with the URI. Requests carried
`io.modelcontextprotocol/protocolVersion` and
`io.modelcontextprotocol/clientCapabilities` in `_meta`.

Local run artifacts are under `target/mcp-native-2026-09-10/`:

- `subscription-native-results.txt`: notifications, pause/resume, duplicates,
  and finite lifetime.
- `subscription-isolation-results.txt`: resource discovery, client and scope
  isolation, global capacity, and disconnect recovery.
- `subscription-stop-results.txt`: listener shutdown and restart.
- `event-subscription-unit-results.txt`: 49 remote-control unit tests, including
  canonical resource identities, admission release, and cancellation-generation
  checks that reject continuation after a pause/resume cycle between polls.
- `subscription-build.txt` and `subscription-clippy.txt`: Linux build and lint
  validation.

The Python transport clients were disposable local test fixtures under
`/tmp/nickel-mcp-native/`, not shipped tooling. These artifacts document this
machine's run; they are not a portable automated native acceptance harness.

## Remaining coverage

This validates the implemented window identity, retirement, focus, and output
membership event subscription transport. It does not establish completion of
Spec 0231's other diagnostic domains, bounded logs, or temporary traces.
Windows, cross-machine HTTPS streaming, physical output hotplug, mixed-DPI
outputs, and physical emergency revocation during a stream still need their own
native acceptance. The specs remain active.
