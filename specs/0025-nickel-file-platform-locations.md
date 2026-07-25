# Nickel File Platform Locations

## Goal

Expose volumes, cloud roots, libraries, and network locations through native platform discovery
while presenting consistent Nickel File behavior.

## Windows

### Known Folders and Cloud Roots

- Resolve user folders with the Known Folders API.
- Discover OneDrive and other registered sync-provider roots without assuming folder names.
- Preserve provider status overlays and availability states when the platform exposes them.

### This PC

- Present fixed, removable, optical, and network volumes.
- Show the native volume label, filesystem icon, capacity, and free space where available.
- Refresh on device arrival, removal, mount, and label changes.
- Offer eject only when the platform reports that the device supports it.

### Libraries

- Enumerate registered Windows Libraries rather than hard-coding Documents, Music, Pictures, and
  Videos.
- Display a library as an aggregate collection while retaining each item's real target path.
- Respect library membership and default-save-location changes made outside Nickel.

### Network

- Browse Windows network locations and saved mappings without requiring Explorer.
- Treat discovery, authentication, and offline failures as recoverable view states.
- Never initiate broad network discovery until the user opens Network.

Windows shell namespace identifiers and COM interfaces remain inside `nickel-platform`.

## Linux

- Discover volumes and removable media from the active session's storage services.
- Honor XDG user directories and update when their configuration changes.
- Represent mounted network locations and bookmarks when the desktop portal or storage service
  exposes them.
- Keep optional desktop-service integrations behind narrow adapters.

## macOS

- Discover mounted volumes, user favorites, iCloud Drive, and network locations through native
  workspace and file-manager services when macOS support begins.

## Sidebar

The sidebar groups locations in this order when present:

1. Nickel Home
2. User pins
3. Cloud and user locations
4. This Computer and volumes
5. Network

Empty groups are omitted. Native locations use the shared location model and must not be converted
to fake filesystem paths.

## Verification

- Contract-test discovery with synthetic platform adapters.
- Manually test drive insertion/removal, offline network paths, cloud placeholders, redirected
  folders, and library membership changes.
- Verify Nickel File remains responsive while network and removable providers are slow.
- Verify duplicate paths from libraries, pins, and cloud providers retain one target identity.

## Completion

Archive this specification when Windows This PC, Libraries, cloud roots, and Network are usable
without Explorer and Linux volumes and XDG locations satisfy the same platform contracts.
