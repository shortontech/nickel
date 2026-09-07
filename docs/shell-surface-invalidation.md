# Shell surface invalidation and panel ownership

The compositor adapter preserves changed internal surface IDs across deadline polls,
session snapshots, secure-storage updates and batches of input. A synchronization pass
always reconciles visibility and placement. It copies a scene only for a changed or
newly visible surface. Client-window output damage remains independently scheduled by
the compositor. Explicit global synchronization continues to cover theme, locale,
wallpaper, topology, scale and application replacement.

Window membership, ordering and active-state updates affect panels, launcher status,
previews and context menus. Geometry-only moves change the portable window state's
output membership, which participates in equality without copying native geometry. Clock deadlines invalidate
panels. Secure-storage status affects the launcher. Tray updates affect panels;
notifications affect their own surface; audio affects control center and volume OSD;
network and Bluetooth affect control center. Only settings transactions retain global
synchronization. Local panel hover and launcher text input do
not copy desktop or sibling scenes. Visibility transitions update the affected surface
and panel affordances. Panel actions additionally invalidate task previews and menus;
control interactions invalidate panel, volume OSD and keyboard consumers; preview and
context-menu actions invalidate each other's content and panel task state.

Each coordinator slot stores saturating `scene_generation` and `commands_copied`
counters, released with that slot. Trace events record their surface identity plus the
adapter's global/content invalidation reason. Renderer preparation counters remain in
the renderer diagnostics. These counters describe logical scene builds and command
counts, not allocator measurements or VRAM usage.

The panel application holds shared task groups and small presentation state. The
authoritative launcher stays in `LiveShell`. A task projection is reused when its
catalog/pin revision and ordered output-filtered window inputs match. Rendering, task
activation, preview grouping, drag/drop, overlay menus and semantic hit targets consume
that projection. Builtin Settings and File icons are two process-lifetime immutable
PNG decodes, shared by `Arc`; their fixed embedded assets cannot change at runtime.

Each output has its own renderer-free `UiHost<PanelApplication>`, preserving pointer
capture, hover, keyboard focus and overlay state while sibling outputs render.
Rendering restores the input output and host before returning; popover authority is
not reassigned. Deadline scheduling considers every host. Hosts and projections are
released when outputs retire; catalog revisions discard old task projections. A hard
limit of 32 cached hosts plus one active host, and 32 task projections, prevents
unbounded history from arbitrary output names. Crossing the limit clears cached
entries and rebuilds them on demand. Retained bytes scale with current windows and
resolved frames; this change does not claim a measured constant byte limit.

Automated evidence includes a 10,000-entry catalog with repeated semantic hover,
shared builtin image identity, pin invalidation, two-output task filtering, geometry-only
window movement, show-all-windows behavior, output retirement, pointer capture/focus
preservation across sibling rendering, and unchanged scene generations during local
hover and launcher typing. Native multi-output focus/drag/popover tests and release
allocation/frame-time measurements remain follow-up acceptance work; no session
restart or installed binary replacement is required by the implementation.
