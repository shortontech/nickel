# Diagnostic action production-owner matrix

This verification contract defines deterministic owner-boundary coverage for every
`DiagnosticAction` variant. It supplements live native acceptance; it does not treat synthetic
prepared observations as proof that a host provider, GPU, or Windows API succeeded.

The shared scenario must submit requests to `NickelSession::handle_remote_desktop_request`, using a
real `DesktopPermit` issued by the production control plane. Application and platform observations
may be deterministic prepared fixtures because the scenario verifies owner reconciliation only.
Native Linux acceptance remains the source of evidence for an initialized renderer and live
platform-provider execution. Native Windows execution remains unverified.

| Action | Owner success | Busy/delay | Revoked or expired | Late prepared result | Protected state |
| --- | --- | --- | --- | --- | --- |
| `repaint` | accepted as an unconfirmed redraw request | not staged | rejected before mutation | request deadline applies | rejected |
| `refresh_scene` | production scene housekeeping runs | not staged | rejected before mutation | request deadline applies | rejected |
| `refresh_application_inventory` | fresh prepared catalog is reconciled | shared staging rejects concurrent preparation | rejected before mutation | rejected before publication or generation change | rejected |
| `refresh_platform_status` (all five domains) | each fresh typed observation is mapped and retained | shared staging rejects every domain while occupied | rejected before mutation | rejected before generation, retention, event, or shell change | rejected |
| `start_frame_trace` | live initialized-backend acceptance | active trace rejects at capacity | rejected before mutation | request deadline applies | rejected and standing trace revalidation removes ownership |
| `stop_frame_trace` | no-op without a trace and stops the caller's trace | not staged | rejected before mutation | request deadline applies | rejected |
| `identify_output` | exact current output identity owns a bounded overlay | timer capacity/registration failure is reported | rejected before mutation and revocation expires an active overlay | request deadline applies | rejected and active overlay is hidden/expired |

The five platform rows are `connectivity`, `audio`, `peripherals`, `maintenance`, and
`default_associations`. Their scenario outcomes must preserve domain-specific coarse fields and
must never claim presentation. Connectivity and audio may confirm shell reconciliation; the other
three report retained owner observations without inventing UI or native-provider confirmation.

Prepared application and platform results are valid for at most the diagnostic request's two-second
delivery window. The production owner must reject an older result even if a newly constructed permit
is otherwise live. Rejection must leave application/platform generations, retained snapshots, shell
state, and completion events unchanged.
