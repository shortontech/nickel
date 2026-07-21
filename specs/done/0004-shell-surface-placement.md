# Shell Surface Placement

## Objective

Give `nickel-session` explicit geometry policy for Nickel-owned shell surfaces instead of treating the panel and launcher like cascaded application windows.

## Behavior

- The panel occupies the full logical output width and is 56 logical pixels high.
- The panel is anchored to the bottom edge at `x = 0`.
- Nickel-owned panel and launcher surfaces do not request client-side window decorations.
- The usable application work area ends at the panel's top edge.
- The launcher retains its requested size and is centered within the usable work area.
- Showing an already-warm launcher recalculates its position before mapping it.
- Output resize events reconfigure and reposition both shell surfaces.
- Ordinary new windows remain cascaded but are clamped to the usable work area.

## Boundaries

Shell roles are recognized internally from the bootstrap window titles `Nickel Panel` and `Nickel Launcher`. A dedicated private protocol can replace this temporary identification mechanism later. This slice does not add multiple-output policy, user-configurable panel sizes, layer-shell compatibility, maximizing, or tiling.

## Verification

- Unit tests cover panel geometry, work-area calculation, launcher centering, and undersized outputs.
- Workspace formatting, clippy, and tests pass.
- Manual nested-session testing confirms full-width bottom placement, centered launcher placement, and stable positions across host-window resizing.
