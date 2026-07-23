# Display Settings

## Goal

Provide an all-Rust `nickel-settings` application for inspecting, arranging, and selecting the primary display while dogfooding the native Linux session.

## Scope

- Add a backend-neutral output model with stable identifiers, connector names, logical geometry, current mode, scale, and primary state.
- Extend the Nickel session protocol with request/reply output enumeration and commands to apply a complete layout and primary-output selection atomically.
- Render a Display page that represents connected monitors as labeled rectangles and supports dragging them into position.
- Allow selecting exactly one primary output. The panel, launcher, context menus, and default new-window placement follow it without forcing that output to the left.
- Normalize applied coordinates so the combined desktop has a non-negative origin, reject overlapping layouts, and keep at least one output enabled.
- Persist the accepted layout by stable output identity and restore matching entries on later session starts. Newly discovered outputs receive a deterministic fallback placement.
- Apply changes live without restarting `nickel-session`; preserve windows where possible and rescue windows stranded outside the new desktop bounds.

The first slice targets Linux DRM outputs. Resolution, refresh-rate, rotation, fractional scale, color management, Windows display APIs, and macOS display APIs remain follow-up work.

## Verification

- Unit-test normalization, overlap rejection, stable identity matching, fallback placement, and primary selection.
- Test session-protocol parsing and atomic rejection of invalid layouts.
- Run the settings application under the nested backend and exercise live apply in a native two-monitor session.
- Repeatedly restart Nickel and verify the saved primary output and left/right arrangement remain stable regardless of connector discovery order.
- Run workspace tests and Clippy with warnings denied.

## Completion

Archive this specification when a user can launch `nickel-settings`, drag two connected outputs into position, select the primary output, apply the configuration live, and observe the same arrangement after restarting Nickel.
