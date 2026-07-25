# Nickel File Foundation

## Goal

Provide a native cross-platform file manager that shares Nickel's component system and remains
usable without Windows Explorer or a Linux desktop file manager.

## First Slice

- Launch as an independent `nickel-file` process.
- Open the user's home directory by default and accept an optional path argument.
- Use the familiar file-manager structure: navigation toolbar, places sidebar, and labeled icon grid.
- List directories before files with stable case-insensitive ordering.
- Navigate into directories and support Back and Up.
- Select entries with the mouse or keyboard.
- Open files through the platform's registered handler.
- Keep filesystem enumeration and navigation policy independent of native launch APIs.
- Render clear empty-directory and read-error states.

Copy, move, rename, trash, conflict handling, thumbnails, search, tabs, removable locations, network
locations, file previews, and shell association registration are later slices.

## Verification

- Unit-test entry ordering, parent navigation, and history behavior with temporary directories.
- Build on Windows and Linux without leaking native types into the directory model.
- Manually verify directories, files, Unicode names, inaccessible paths, scrolling, double-click,
  Back, Up, Enter, and platform file activation.

## Completion

Archive this specification after Nickel File can replace the basic browse-and-open workflow on both
Windows and Linux.
