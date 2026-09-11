# Native transient identity boundary

Nickel resolves an ordinary native window's application from OS process evidence.
A transient whose shared runtime has no standalone application identity may fall
back to its production parent only when both windows still resolve to the exact
same current OS process incarnation. Wayland uses the compositor-owned
`xdg_toplevel` parent graph. Xwayland uses `WM_TRANSIENT_FOR`, but the same-process
check prevents that forgeable property from crossing a process boundary.

The focused policy coverage exercises the production inheritance predicate. A
current same-process helper can inherit its parent's application. Pending,
unavailable, stale and protected identities cannot inherit, and a live unrelated
process cannot inherit even if a native parent relationship claims otherwise.
Window registry coverage removes a child and verifies its replacement receives a
higher generation, while the retired ID is no longer live. Existing owner
retirement coverage removes the corresponding identity before deferred lease
cleanup and ignores identity-worker replies whose monotonic window ID is gone.

No live transient acceptance is claimed by this increment. The checked
`nickel-ui` examples each create one winit toplevel. Winit's parent-window API is
unsupported on Wayland and creates an embedded X11 child rather than a managed
`WM_TRANSIENT_FOR` toplevel on X11. Zenity's `--attach` option is a deprecated
no-op on this host. A normal ELF process also gives every one of its windows the
same direct executable identity, so such a fixture would bypass rather than test
shared-runtime fallback. There is no installed Flatpak client available to
exercise a parent with corroborated application identity and a same-process
helper transient without direct identity.

Live Wayland and Xwayland coverage still needs a checked client capable of
creating managed parented toplevels from one shared-runtime process while varying
the child surface claim. Auth, credential and broker dialogs also need their real
host process and portal relationships exercised; the deterministic protected and
cross-process cases here do not establish native presentation or toolkit behavior.
