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

Start Menu review uses the reference as a hierarchy oracle: compare pane proportions, section order,
footer and search attachment, row density, taskbar relationship, icon alignment, and visual hierarchy.
Names, counts, wallpaper, date, accent, and installed applications are deliberately excluded from
pixel acceptance.

For a rough image-difference aid, crop both images to the menu bounds, resize the live crop to the
reference menu crop, convert both to grayscale, and inspect an absolute-difference image. Record the
tool version and crop rectangles with the result. This aid can expose drift in pane boundaries and row
rhythm, but it cannot accept the design: the original-resolution live render still requires visual
inspection because theme, font rasterization, content, and translucency legitimately differ.

The 2026-08-28 live Start Menu comparison used FFmpeg 8.0.1. It cropped the admitted reference to
`1092:980:56:24`, cropped the 1280x800 nested-session capture to `920:680:18:56`, scaled that live
crop to the reference crop, converted both to grayscale, and applied `blend=all_mode=difference`.
The ignored inputs and aid are `target/nickel-ui-snapshots/start-menu-live-1280x800.png` and
`target/nickel-ui-snapshots/start-menu-live-difference.png`. The original live capture was inspected
at its native resolution; the stretched difference aid is not an acceptance oracle.
