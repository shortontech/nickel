# Linux Login Session Bootstrap

## Goal

Make Nickel selectable from a display manager and establish the user-session environment required by normal desktop applications.

## Behavior

- Provide a Wayland session desktop entry and a Rust session launcher with development and installed-path support.
- Set `XDG_SESSION_TYPE=wayland`, `XDG_CURRENT_DESKTOP=Nickel`, `XDG_SESSION_DESKTOP=Nickel`, and the compositor-selected `WAYLAND_DISPLAY`.
- Import the environment into the D-Bus and systemd user sessions before starting desktop services or applications.
- Start `nickel-session`, then its supervised `nickel-ui` child, with structured logs and actionable fatal errors.
- Integrate existing user-session D-Bus, PipeWire/WirePlumber, keyring, policy agent, and portal services without reimplementing them in Nickel.
- Support clean logout and distinguish compositor failure from an intentional session exit.
- Document installation, removal, development launch, and recovery from a broken session.

Packaging for a specific distribution and automatic privilege escalation are out of scope.

## Verification

- Validate generated desktop-entry and launcher paths in a temporary install prefix.
- Start Nickel from at least one display manager and confirm the expected environment from a terminal launched inside Nickel.
- Verify D-Bus applications, audio, policy prompts, and portal discovery can reach their user services.
- Log out and log back into another desktop without stale Nickel processes or environment.

## Completion

Archive this specification when a user can select Nickel at login, launch ordinary Wayland applications, use core user services, and log out safely.
