# Nickel File Location Model

## Goal

Let Nickel File navigate filesystem directories and platform-defined locations without pretending
that every location is a path.

## Model

Represent the current location and navigation history with a platform-neutral identity:

- `Directory(PathBuf)` for ordinary filesystem directories.
- `Collection(LocationId)` for generated views such as Nickel Home and Recent Files.
- `Volume(VolumeId)` for mounted filesystems and removable media.
- `Native(LocationId)` for platform namespaces such as Windows This PC and Network.

A location provides a stable identifier, display name, icon, capabilities, optional parent, and an
ordered stream of entries. Entries identify whether they can be opened, browsed, pinned, removed,
ejected, or used as a file-operation destination.

Core navigation, selection, history, and sorting must not contain Windows shell handles, Linux
mount objects, or macOS URLs. Native adapters translate their identifiers at the platform boundary.

## Behavior

- Back and Forward traverse location history, including transitions between virtual and physical
  locations.
- Up follows the location's declared parent and is disabled when no parent exists.
- Refresh reloads the current provider without discarding navigation history.
- The address area renders breadcrumbs when a hierarchy is available and a stable location label
  otherwise.
- Errors appear inside the current view without replacing its identity or corrupting history.
- File operations query destination capabilities before offering an action.

## Verification

- Test mixed directory, collection, volume, and native navigation with synthetic providers.
- Test stable selection across refreshes and removal of vanished entries.
- Test Back, Forward, and Up across virtual-to-physical transitions.
- Test provider errors and unavailable removable or network locations.

## Completion

Archive this specification when Nickel File no longer requires a `PathBuf` for its current location
and both filesystem and synthetic providers use the shared navigation model.
