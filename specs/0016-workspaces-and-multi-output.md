# Workspaces and Multi-Output Shell Behavior

## Goal

Make window management, task switching, and shell placement predictable across virtual workspaces and multiple displays.

## Behavior

- Model workspaces with stable IDs, ordered membership, active output, and deterministic window movement rules.
- Keep application grouping independent of workspace identity while allowing the panel and previews to filter by current workspace.
- Implement create, remove, switch, and move-window operations with keyboard and UI entry points.
- Define focus restoration, minimize/maximize geometry, fullscreen, and Alt-Tab behavior across workspaces.
- Maintain per-output logical geometry, scale, transform, work area, and primary designation.
- Place panels, launchers, menus, previews, notifications, and fullscreen surfaces on the correct output.
- Reflow windows safely when an output disappears and restore a stable layout when it returns.
- Expose the same portable workspace/output model to Windows adapters where the OS permits equivalent behavior.

Persistent user-configurable layouts and elaborate workspace animations are out of scope.

## Verification

- Unit-test workspace ordering, filtering, focus restoration, output removal, and scale-aware shell placement.
- Test Alt-Tab, maximize/restore, fullscreen, and window movement on two workspaces.
- Test mixed-scale two-monitor hotplug and verify no shell surface becomes unreachable.
- Verify grouped previews and window actions target the correct workspace member.

## Completion

Archive this specification after the behavior passes in both the nested harness and a native multi-output session.
