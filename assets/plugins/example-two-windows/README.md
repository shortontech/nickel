# Two window example

This package declares two ordinary windows. Close the details window while
leaving the home window open, then click **Reopen details**. The button requests
one of its own declared surfaces through `show-plugin-surface`; Nickel creates
a fresh component instance for the reopened window and keeps the home window
running.
Both surfaces use the same JSX `<Window>` component. Nickel assigns its ID from
the host surface, while `ui.css` styles the ordinary window and controls.
The details window also has a **Close details** button that requests
`hide-plugin-surface`; its home window stays open.

Run `nickel-plugin dev assets/plugins/example-two-windows` to try it in an
isolated shell. Generate `main.js` from `main.jsx` with the TypeScript CLI
before packaging it for installation.
