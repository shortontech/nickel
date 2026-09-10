# Ordinary shell surface observation

`list_surfaces {lease_id}` returns visible ordinary Nickel shell surfaces within
that lease. Use the returned `id` and `generation` with:

- `capture_surface {lease_id, surface_id, generation}` for bounded PNG capture.
- `inspect_surface {lease_id, surface_id, generation}` for bounded production UI
  semantics, including the current `tree_generation` and snapshot-local ordinals.

These tools do not invent application-window identities for shell hosts. Surface
leases authorize their exact incarnation. Output leases require the entire surface
to remain inside the current native output. Full-session leases still exclude
protected resources. Hidden, retired, protected and unknown roles are denied.

Capture uses the existing offscreen renderer and bounded encoding admission,
rechecking authority and resource membership across asynchronous stages. Semantic
observation reads the existing UiHost or retained output viewport without activating
it, changing focus or rebuilding the tree. Semantic generation confirms the current
resolved UI, not that pixels have finished presentation. Node and retained payload
budgets match hosted-window semantics; budget overflow denies the projection.

Desktop, panel, launcher/run, control center and volume OSD are the initial ordinary
roles. The lock screen, trusted indication, permission UI and unprojected roles
remain unavailable. Shell semantic mutation and external application accessibility
are separate unfinished work; an inspection response does not authorize either.
