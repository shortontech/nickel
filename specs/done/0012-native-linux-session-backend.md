# Native Linux Session Backend

## Goal

Run `nickel-session` directly on a Linux seat and physical outputs while preserving the nested winit backend for development.

## Scope

- Add a backend abstraction that keeps compositor policy independent of rendering and input sources.
- Implement DRM/KMS output discovery, mode selection, page flips, and rendering through Smithay.
- Acquire and release the seat through libseat/logind; handle VT activation and deactivation without losing client state.
- Read keyboard, pointer, touch, and device hotplug events through libinput.
- Detect output hotplug, expose logical geometry and scale, and choose a deterministic initial layout.
- Shut down cleanly and restore the VT when initialization or rendering fails.
- Retain `backend_winit` behind an explicit development feature or command-line selection.

XWayland, display-manager installation, output configuration UI, and session services are separate slices.

## Verification

- Unit-test backend-neutral output layout and device-state transitions.
- Run nested-backend regression tests.
- Launch Nickel from a spare VT, display two Wayland clients, switch away and back, and exit with the original VT restored.
- Exercise monitor and input-device hotplug where hardware permits.
- Run workspace tests and Clippy with warnings denied.

## Completion

Archive this specification when Nickel can own a seat, render clients on a physical output, process physical input, survive a VT switch, and terminate cleanly without another compositor.
