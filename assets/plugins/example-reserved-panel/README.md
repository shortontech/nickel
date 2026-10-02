# Reserved panel example

This JSX plugin creates a 36-pixel `<FixedWindow>` across each output and reserves desktop
work area. Run `nickel-plugin dev assets/plugins/example-reserved-panel` to test
it in an isolated shell beside the bundled taskbar. Nickel stacks their native
windows on the configured panel edge and releases each panel's reserved strip
when that panel retires.

Floating docks use the same root with `reserveWorkArea` omitted and can
set `bottomOffset` instead.
