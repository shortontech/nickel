# Volume overlay plugin

`main.js` renders Nickel's volume overlay with the shared JSX component host.
The shell supplies a bounded label and percentage. It owns audio observation,
the overlay timeout, native placement, and device-name redaction while locked.

The overlay uses `Progress`, a passive component with bounded `percent`,
`width`, and `height` properties. Settings can disable the plugin; Nickel then
uses its native recovery view.
