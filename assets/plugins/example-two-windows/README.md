# Two window example

This package declares two ordinary windows. Close the details window while
leaving the home window open, then click **Reopen details**. The button requests
one of its own declared surfaces through `show-plugin-surface`; Nickel creates
a fresh component instance for the reopened window and keeps the home window
running.

Run `nickel-plugin dev assets/plugins/example-two-windows` to try it in an
isolated shell. Generate `main.js` from `main.jsx` with the TypeScript CLI
before packaging it for installation.
