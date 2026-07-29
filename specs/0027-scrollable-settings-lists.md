# Scrollable Settings Lists

## Goal

Restore live Linux network information in Nickel Settings and give reusable SDL components an
explicit overflow contract so variable-length settings data remains visible, bounded, and
interactive.

## Requirements

- Read Linux Wi-Fi state from NetworkManager over system D-Bus without adding KDE or shell-command
  dependencies.
- Show whether Wi-Fi is available and powered, the active connection, visible networks ordered by
  connection and signal, and whether each network has a saved profile.
- Permit enabling or disabling Wi-Fi and activating saved profiles. New-network credential entry is
  outside this specification.
- Refresh network state without blocking rendering for sustained periods.
- Render all Bluetooth and Wi-Fi entries through vertically scrollable lists rather than truncating
  them with fixed item-count limits.
- Give scroll containers an explicit viewport, content height, and offset.
- Clip descendant painting and hit regions to the scroll viewport. Content outside the viewport
  must neither paint nor receive pointer actions.
- Clamp offsets when content or viewport sizes change.
- Preserve dropdown overlays and existing layouts by making clipping explicit rather than changing
  every container's default behavior.
- Use the resolved Nickel theme for hover and active states.

## Verification

- Unit-test clip command emission, clipped hit regions, scroll translation, and offset clamping.
- Unit-test deterministic Wi-Fi ordering and saved-profile identity parsing where practical.
- Run formatting, workspace tests, strict Clippy, and `git diff --check`.
- In the live KDE development session, confirm NetworkManager and BlueZ return non-empty snapshots
  where the host exposes them.
- Manually confirm mouse-wheel scrolling, hover transitions, and that off-screen rows cannot be
  clicked before archiving this specification.
