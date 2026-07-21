# Panel Clock

## Objective

Add a lightweight local-time clock to the persistent Nickel panel.

## Behavior

- The panel displays local time as zero-padded 24-hour `HH:MM` text.
- The clock is right-aligned with 24 logical pixels of right padding.
- The launcher button remains confined to the left side; hovering or clicking the clock does not toggle the launcher.
- Timezone and daylight-saving changes are picked up from the operating system.
- The event loop sleeps until the next minute boundary and redraws only when the displayed minute changes.

## Boundaries

The clock is display-only. Calendar UI, seconds, alternate formats, locale preferences, alarms, and click behavior are out of scope.

## Verification

- Unit tests cover `HH:MM` formatting, minute-boundary scheduling, and launcher-button hit testing.
- Workspace formatting, clippy, and tests pass.
- Manual nested-session testing confirms right alignment and a minute update without continuous redraw activity.
