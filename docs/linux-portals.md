# Linux desktop portals

Nickel delegates sandbox-sensitive desktop requests to the XDG desktop portal service on the user
session bus. It does not launch desktop-specific helper programs.

## Current integration

- Linux file dialogs call `org.freedesktop.portal.FileChooser` directly through the Rust platform
  adapter. A missing portal is an explicit dialog failure; Nickel cannot fall back to Zenity or
  another desktop helper. Windows continues to use SDL's native dialog adapter.
- HTTP and HTTPS links are submitted directly to `org.freedesktop.portal.OpenURI` on the session bus.
- The session installer supplies `nickel-portals.conf`. It selects the GTK implementation by default,
  retains KWallet as the Secret portal implementation where it is installed, and selects
  `xdg-desktop-portal-wlr` for ScreenCast and Screenshot.
- Nickel advertises the standard output-capture and image-copy-capture Wayland protocols. Portal
  frames come from the final composited output rather than raw client buffers, so lock and recovery
  overlays remain authoritative. The globals are visible only to the dedicated
  `xdg-desktop-portal-wlr` process, not ordinary Wayland clients.

These choices keep the request authority with the user's portal frontend and avoid depending on a
KDE, GNOME, or other shell process being present in a Nickel session.

## Recorded acceptance

On 2026-08-29, Google Chrome's standard `getDisplayMedia` flow ran as a Wayland client in the nested
Nickel compositor with an isolated `xdg-desktop-portal` session configured for
`xdg-desktop-portal-wlr`. The browser selected the Nickel output, negotiated a 1280x800 PipeWire
video stream, and rendered recursively captured frames at the same resolution. While that stream
remained active, crossing Nickel's authenticated nested lock boundary changed a sampled captured
client pixel from white to the compositor's opaque lock-cover color. This proves the portal receives
the final locked composition rather than readable client content.

On the same date, Nickel Settings ran in a read-only Bubblewrap mount namespace against an isolated
session bus containing only `xdg-desktop-portal` and its GTK backend. Its production image-picker
action opened `org.freedesktop.portal.FileChooser` inside Nickel, exposed only the configured image
patterns, returned the selected file URI, and rendered the decoded image preview. Process inspection
and the portal log confirmed that no Zenity or other desktop helper was launched. A forged Flatpak
identity was separately rejected by the portal and is not counted as package-sandbox acceptance.

## Outstanding compatibility

The session image must provide `xdg-desktop-portal`, `xdg-desktop-portal-gtk`,
`xdg-desktop-portal-wlr`, and PipeWire. RemoteDesktop input injection is not provided. A portal
implementation discovered from the host desktop is not acceptance evidence: for example, the KDE
screen-capture backend depends on KWin and cannot serve a standalone Nickel compositor. Native
completion remains outstanding until the same application-level flow receives PipeWire
frames in an SDDM-launched Nickel session.
