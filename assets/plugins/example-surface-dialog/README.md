# Separate dialog surface

This package declares an ordinary window and a dialog. Enabling it creates the
window only. Its button requests `show-plugin-surface` for the declared dialog;
the dialog's own button requests `hide-plugin-surface` to retire it. The other
button requests `show-settings` under the declared grant.
The dialog declares `home` as its owner. Closing that window retires the dialog;
on Windows the host also creates it as a native owned window.

Run `nickel-plugin dev assets/plugins/example-surface-dialog` to try it in an
isolated Nickel session. The `.jsx` source is compiled before validation and
reload; `main.js` is the packaged runtime entry.
