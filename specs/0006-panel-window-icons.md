# Panel Window Icons

## Goal

Show open application windows in the Nickel panel as a compact, icon-first task strip similar to Windows 11 and KDE Plasma.

## Behavior

- `nickel-session` remains the authority for window identity, stacking order, application ID, and active state.
- `nickel-ui` receives bounded window snapshots through the existing local session-control channel.
- Nickel-owned launcher and panel surfaces are omitted from the task strip.
- Each application window receives a 48 px task button containing a centered 32 px icon.
- Hovered buttons receive a visible background. The active window receives a short accent indicator along the panel edge.
- Icons are resolved from existing desktop-entry metadata. An unresolved application receives a neutral fallback mark.
- This slice represents individual windows. Grouping multiple windows by application is deferred.
- Window-list changes should become visible within 250 ms without rebuilding the launcher index or recreating either shell window.

## Portability

The window snapshot and panel task model must remain platform-neutral. The initial transport may use Unix datagrams because `nickel-session` currently targets the Linux compositor path; platform-specific providers can populate the same model later.

## Verification

- Registry serialization excludes shell-owned windows and preserves active state.
- Panel layout and hit testing remain correct with zero, one, and several tasks.
- Opening, focusing, and closing a real application updates the running panel.
- `cargo test --workspace` and workspace Clippy pass.
