# Generic surface draft: taskbar and launcher

`taskbar/main.next.jsx` is a review source, with visual styling in
`taskbar/main.next.css`. It is not selected by a manifest or loaded by Nickel.
The launcher draft was retired after its shared CSS grid was incorporated into
the shipped `launcher/main.jsx` and `launcher/ui.css`.

The draft deliberately keeps ordinary JavaScript callbacks and `useRef` /
`useState`. An author should be able to write `onClick={() => ...}` without
registering a named export for every button. Named exported functions are an
optional ABI for actions that another plugin invokes. Rust still validates
every `nickel.request` against the owning plugin's capabilities.

## Root contract needed

- `<Window>` is the single host surface root. `<FixedWindow>` is a normal JSX
  helper that returns `<Window placement="fixed" ... />`; it has no separate
  Rust rendering or lifecycle path. `id` matches a manifest-authorized
  surface; JSX chooses its placement and children. The host applies one generic
  creation, input, rendering, and retirement path.
- Taskbar needs `output="all"`, `edge="bottom"`, `width="100%"`, `height={56}`,
  and `reserveWorkArea={true}`. For a dock, CSS background alpha must not imply
  that text, icons, or input become transparent.
- Launcher needs `output="active"`, `anchor="bottom-start"`,
  `avoid="taskbar"`, width and height, and output/work-area maximums. The
  current native placement is bottom-left of the selected output, above the
  bottom panel; those host-resolved semantics need one general anchor rule.
- `className` must work on roots and descendants. Buttons and text fields keep
  input, focus, editing, accessibility, and sizing behavior but have no forced
  visual theme. CSS supplies their backgrounds, borders, padding, radii, text
  size, and line height. The selectors in these examples stay within each
  plugin's stylesheet.

## Missing generic UI contracts

- The shipped taskbar receives badge contributions through
  `nickel.data.slots["task-badge"]` and maps them into JSX beside each task.
  The separate taskbar context menu now reads `nickel.data.slots["task-action"]`.
  Its activation still uses taskbar-specific host routing.
- `<Dialog>` and `<Menu>` should own child surfaces through the same generic
  surface lifecycle, with anchor IDs resolved from their parent tree. The
  launcher keeps its existing logout and app menu behavior in the draft.
- CSS examples use flex alignment, gaps, absolute badge position, hover/focus
  selectors, box shadow, and text styles. The first CSS implementation may
  support only a subset; unsupported declarations should be explicit compiler
  diagnostics, not silently replaced by Rust component defaults.
- `Spacer grow={1}` needs flexible width, preserving the GNOME 2 style panel
  composition use case. This is the simplest useful test of generic layout.
- Projected `nickel.data` and the current typed request names remain intact
  for this draft. A future service API can improve their names without tying
  the generic surface path to taskbar or launcher IDs.

## Visual review and performance observation

Compare the draft with master at normal and high DPI, on multiple outputs,
with menus and dialogs open. Check keyboard focus and drag pinning.
Record stock process memory alongside the aspirational 200 MiB Windows and
280 MiB Linux figures, and record stock presentation-tree memory alongside
the 12 MiB figure. Memory is not a promotion gate. These source files alone
prove neither appearance nor memory use.

Controller navigation parity across new JSX layouts is a later epic. Existing
native control behavior stays in place during this migration.
