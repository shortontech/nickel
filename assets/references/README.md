# Visual references

`nickel-start-menu.png` and `nickel-settings-appearance.png` are composition references, not product
artwork. Their admitted dimensions, checksums, provenance, and usage status are recorded in
`../visual-fixtures.toml`.

Appearance review compares the hierarchy, sidebar/content proportion, card rhythm, tab placement,
visual-choice clarity, wallpaper field grouping, and interface-row alignment. The orange host
decoration, exact copy, current values, and font rasterization are excluded from pixel acceptance.

The 2026-08-28 Appearance comparison used FFmpeg 8.0.1. It cropped the reference to
`1424:1062:0:43` (removing host decoration), cropped the deterministic dark English render to
`1424:1062:0:0`, and applied `blend=all_mode=difference`. The ignored aid is written to
`target/nickel-ui-snapshots/appearance-dark-en-difference.png`; the original reference and render,
not this aid, remain the acceptance oracle.

Start Menu review uses the 2026-08-31 controller-launcher reference as a hierarchy oracle: compare
the bounded sidebar and flexible content panes, Home/Applications/Places relationship, favorites
grid, recent-application rows, attached action legend, search placement, row density, taskbar
relationship, icon alignment, and visual hierarchy. Names, counts, wallpaper, date, accent,
controller-family labels, account tier, and installed applications are deliberately excluded from
pixel acceptance.

For a rough image-difference aid, crop both images to the menu bounds, resize the live crop to the
reference menu crop, convert both to grayscale, and inspect an absolute-difference image. Record the
tool version and crop rectangles with the result. This aid can expose drift in pane boundaries and row
rhythm, but it cannot accept the design: the original-resolution live render still requires visual
inspection because theme, font rasterization, content, and translucency legitimately differ.

The previous 2026-08-28 comparison described the superseded project-oriented reference and must not
be reused as acceptance evidence for the controller-first composition. Record fresh crop geometry
and tool versions when a live implementation is ready for comparison. The original live capture and
the admitted reference, not a stretched difference aid, remain the acceptance oracle.
