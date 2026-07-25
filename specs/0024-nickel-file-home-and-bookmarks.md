# Nickel File Home and Bookmarks

## Goal

Make Nickel Home the useful starting point for finding recent and frequently used content without
copying one platform's Quick Access implementation.

## Nickel Home

Nickel Home contains:

- Pinned locations in user-defined order.
- Frequent folders ranked by recent successful visits with repeat visits decayed over time.
- Recent files ranked by last successful open.
- Available local and removable volumes.

Sections disappear when empty. Nickel Home must remain usable when history storage is unavailable,
disabled, or empty.

## Bookmarks

- Users can pin filesystem directories and browsable virtual locations.
- Pins have a stable identity, editable label, and optional platform icon.
- Pins can be reordered and removed without affecting their targets.
- Missing targets remain visible with an unavailable state until removed by the user.
- Platform-provided defaults may seed a new profile but do not reappear after explicit removal.

## History and Privacy

- Record successful Nickel File navigation and file activation, not search keystrokes or failed
  attempts.
- Store history locally with bounded retention and deterministic pruning.
- Provide controls to clear recent files, clear frequent folders, and disable either history.
- Never synchronize history unless a future synchronization feature is explicitly enabled.
- Do not expose hidden items in Home while hidden-file visibility is disabled.

## Cross-Platform Sources

Nickel's own history and pins are authoritative. Platform recent-item sources may supplement them
when available but must be deduplicated by canonical target identity.

## Verification

- Test ranking, decay, deduplication, retention, and explicit clearing with a synthetic clock.
- Test pin ordering, relabeling, unavailable targets, and persistence failures.
- Test hidden-item filtering and history-disabled operation.
- Test that opening a file once cannot create duplicate recent entries through multiple aliases.

## Completion

Archive this specification when Nickel Home, persistent pins, frequent folders, recent files, and
privacy controls work on Windows and Linux.
