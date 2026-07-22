# Wayland Application Compatibility

## Goal

Implement the protocol and X11 compatibility surface needed for normal daily applications rather than compositor-specific demos.

## Scope

- Complete and verify clipboard, drag-and-drop, and primary-selection behavior.
- Implement XDG activation and startup-token handling without permitting focus stealing.
- Support text-input and input-method protocols for composition and on-screen keyboards.
- Support relative pointer, pointer constraints, idle inhibition, and client/server decoration negotiation.
- Add XWayland lifecycle, display publication, X11 window association, focus, clipboard bridging, and clean restart.
- Preserve application identity and window state in the portable window model for both Wayland and X11 clients.
- Add protocol globals only with documented policy and resource limits.

Screen capture portals, accessibility, color management, and HDR are later compatibility work.

## Verification

- Exercise clipboard and drag-and-drop in both directions between two Wayland clients and between Wayland and XWayland.
- Test Unicode composition/IME, constrained-pointer applications, idle inhibition, activation tokens, and both decoration modes.
- Launch representative native Wayland, Electron/Chromium, toolkit, game, and legacy X11 applications.
- Verify XWayland restart does not terminate `nickel-session` or native Wayland clients.
- Run protocol-focused tests, workspace tests, and Clippy with warnings denied.

## Completion

Archive this specification once the compatibility matrix passes in a display-manager-launched Nickel session and known limitations are documented.
