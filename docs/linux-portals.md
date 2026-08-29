# Linux desktop portals

Nickel delegates sandbox-sensitive desktop requests to the XDG desktop portal service on the user
session bus. It does not launch desktop-specific helper programs.

## Current integration

- SDL file dialogs are forced through SDL's `portal` backend. A missing portal is an explicit dialog
  failure rather than permission to fall back to Zenity or another desktop helper.
- HTTP and HTTPS links are submitted directly to `org.freedesktop.portal.OpenURI` on the session bus.
- The session installer supplies `nickel-portals.conf`. It selects the GTK implementation by default,
  while retaining KWallet as the Secret portal implementation where it is installed.

These choices keep the request authority with the user's portal frontend and avoid depending on a
KDE, GNOME, or other shell process being present in a Nickel session.

## Outstanding compatibility

Nickel does not yet provide the compositor-side ScreenCast, Screenshot, or RemoteDesktop integration
needed for reliable screen sharing. A portal implementation discovered from the host desktop is not
acceptance evidence: for example, the KDE screen-capture backend depends on KWin and cannot serve a
standalone Nickel compositor. Screen sharing remains incomplete until a representative sandboxed
application negotiates a portal session, selects a Nickel-owned source, and receives PipeWire frames.
