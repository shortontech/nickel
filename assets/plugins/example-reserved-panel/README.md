# Reserved panel example

This JSX plugin creates a 36-pixel panel across each output and reserves desktop
work area. Run `nickel-plugin dev assets/plugins/example-reserved-panel` to test
it in an isolated shell beside the bundled taskbar. Nickel stacks their native
windows on the configured panel edge and releases each panel's reserved strip
when that panel retires.

Floating docks use the same panel API with `reserve_work_area` omitted and can
set `bottom_offset` instead.
