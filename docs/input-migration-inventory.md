# Input migration inventory

Updated: 2026-08-30

This inventory records the backend-to-normalized input boundaries covered by private Specifications
0099–0104. Application reducers must consume `nickel_input::InputEvent` or typed controller/global
shortcut outcomes. Native event types are allowed only at the listed adapters and test boundaries.

| Boundary | Native source | Normalized owner | Consumers | Status |
| --- | --- | --- | --- | --- |
| winit focused input | `winit::event::WindowEvent` | `nickel_input::winit::Adapter` | shell, launcher, screenshot, lock, overlays, embedded Codex surfaces, `nickel-ui` runtime, Settings, Nickel File, Shapes test | migrated; per-surface scale and focus reset preserved |
| Declarative UI dispatch | normalized focused events | `nickel_ui::input::FocusedInputDispatcher` and `UiHost::handle_input` | typed widget and application messages, standalone and embedded hosts | migrated |
| Gilrs controllers | `gilrs::Event` | `nickel_input::gilrs` plus `ControllerNormalizer` | `nickel-ui` controller feed | migrated |
| Smithay compositor shortcuts | XKB keysyms | `CompositorShortcutAdapter` | launcher, task switching, workspaces, screenshot actions | migrated boundary |
| Windows global shortcuts | `RegisterHotKey` and low-level hook messages | `RegistrationTable` and shared key vocabulary | launcher, Run, task switching, screenshot actions | migrated boundary |
| Nested semantic test input | authenticated test protocol | compositor production input and hit testing | scenario and live acceptance tools | intentional test boundary |

The remaining native names found by the inventory search are intentional adapter boundaries:

- Smithay XKB keysyms for compositor-owned shortcuts, virtual-terminal switching, and compositor
  recovery controls.
- Win32 `RegisterHotKey` and `KBDLLHOOKSTRUCT` conversion inside the Windows operating-system
  shortcut adapter and its Windows-only harness.
- Winit event imports inside the shared adapter or runtime binaries that immediately pass the whole
  event to that adapter.
- Gilrs event polling inside the shared controller feed before normalization.

No application-local SDL, winit, Smithay, Win32, or Gilrs key table remains in the inventoried
focused consumers. Re-run the following boundary audit when adding a consumer:

```sh
rg -n 'winit::keyboard::|gilrs::|KBDLLHOOKSTRUCT|RegisterHotKey|Keysym' \
  crates/nickel-shell crates/nickel-settings crates/nickel-codex-ui crates/nickel-ui \
  crates/nickel-file crates/nickel-gaze crates/nickel-shapes-test crates/nickel-session
```
