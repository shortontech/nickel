# Grouped application windows

Nickel groups taskbar windows by reconciled application ID while retaining each native window ID,
title, active state, and stacking order. Hovering a multi-window group requests bounded live frames
from `nickel-session`; the shell renders one titled card per window. Hovering a card asks the
compositor to peek that window above a dimmed workspace without changing keyboard focus or persistent
stacking. Close, activation, minimize, maximize/restore, and context-menu requests retain the exact
window ID across the shell/session boundary.

`nickel-test-input scenario grouped-windows APPLICATION_ID` turns this into an unattended live
regression once two disposable fixture windows with that ID are mapped. It discovers window IDs from
compositor snapshots, uses only renderer-resolved semantic targets for the interaction sequence, and
polls compositor state after each operation. It contains no panel, preview-card, or menu geometry.

Preview targets are renderer-owned. In an explicitly test-controlled nested session,
`nickel-test-input semantic` asks the live shell frame to resolve a panel group, preview card, close
button, or menu row. The compositor converts that surface-local result using its current mapped
surface geometry and dispatches ordinary pointer events. Tests and clients do not reproduce panel,
card, or menu coordinates.

## Recorded nested acceptance

On 2026-08-29, two native Wayland Konsole windows with application ID `org.kde.konsole` produced one
taskbar group and two side-by-side titled live thumbnails. Semantic panel and preview hover preserved
window 11 as the active application while peeking window 20 over the dimmed workspace. The same
renderer-resolved path activated a specific card, minimized window 10, maximized and restored window
11, and closed only window 10. Authoritative window snapshots were checked after every transition.

This is compositor-integration evidence, not a claim that every toolkit supplies useful titles or
damage at the same cadence. Missing frames retain a bounded placeholder, empty titles follow the
documented application-name/untitled fallback, and platform-native acceptance remains separate from
the nested semantic suite.
