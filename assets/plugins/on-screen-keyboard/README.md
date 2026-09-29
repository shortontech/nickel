# On-screen keyboard plugin

`main.jsx` defines Nickel's ordinary keyboard layout. `main.js` is the compiled
entry shipped with the shell. Edit the JSX and use `nickel-plugin dev` with this
directory in a nested session to preview changes.

The host supplies key IDs, labels, widths, and enabled state in `nickel.data`.
The plugin sends a key ID and snapshot generation back through `nickel.request`;
Nickel resolves the current key and delivers text only to the recipient lease
that was current when the input gesture began. The plugin never receives the
recipient identity or a key injection API.

On Linux, Settings enable and disable swaps the JSX overlay and Nickel's
host-owned fallback. Windows still uses the host-owned keyboard surface while
the plugin package and status are available for development.
