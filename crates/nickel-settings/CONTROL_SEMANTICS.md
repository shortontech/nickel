# Settings control semantics inventory

This inventory records the production control chosen for each Settings value. It is kept beside the
consumer so reviews can distinguish deliberate composites from accidental reimplementations.

| Page | Setting or action | Value and timing | Production control | Disposition |
| --- | --- | --- | --- | --- |
| Display | Select/arrange display | selection plus spatial drag | semantic selectable drag card | custom graphical composite is required for monitor geometry |
| Display | Identify | command | semantic secondary button | retained |
| Display | Make primary | staged exclusive value | semantic secondary button | retained as a command applying to the selected display |
| Display | Enabled | staged binary value | `SettingsRow` plus `Switch` | migrated; Apply remains the explicit commit boundary |
| Display | Apply | commit command | semantic primary button | retained |
| Nickel Bar | Bar output scope | two exclusive values | semantic radio buttons | retained |
| Nickel Bar | Window scope | two exclusive values | semantic radio buttons | retained |
| Nickel Bar | Desktop count | bounded ordered integer | semantic slider | retained |
| Appearance | Theme mode | three exclusive values | `ChoiceCardGroup` | retained |
| Appearance | Accent/appearance hue and intensity | bounded ordered ranges | `SliderField` | retained |
| Appearance | Wallpaper image | choose/remove commands | semantic buttons | retained |
| Appearance | Wallpaper fit | symbolic enumeration | `SelectField` | retained |
| Appearance | Reduce transparency | persistent binary value | `SettingsRow` plus `Switch` | retained |
| Appearance | Animation level | symbolic enumeration | `SelectField` | retained |
| Appearance | File artwork provider | symbolic enumeration | `SelectField` | retained |
| Appearance | File icon theme | validated text value | text field | retained |
| Appearance | Reset | command | semantic secondary button | retained |
| Network | Wi-Fi power | confirmed binary platform state | `SettingsRow` plus `Switch` | migrated; unavailable state is disabled and requests are typed |
| Network | Wi-Fi network | connect/disconnect command with status | semantic network card | custom composite retained for signal/security/connection status |
| Network | Adapter | observed status | noninteractive status card | retained |
| Bluetooth | Adapter power | confirmed binary platform state | `SettingsRow` plus `Switch` | migrated; unavailable state is disabled and requests are typed |
| Bluetooth | Discovery | start/stop command | semantic secondary button | retained because discovery is an operation, not preference state |
| Bluetooth | Device | connect/disconnect command with status | semantic device card | custom composite retained for pairing/battery/connection status |
| Keyboard Shortcuts | Shortcut rows | observed binding | status rows | retained until editing is implemented |
| About Nickel | Product/platform details | observed information | status/text rows | retained |
| Settings shell | Search | editable query | `SettingsSearchField` | retained |
| Settings shell | Destination | exclusive navigation state | `SettingsNavigation` | retained |

Asynchronous platform controls must render the last confirmed value. A control may show a pending
state only after its request has left the reducer, and failure must not overwrite that confirmed
value. Adding a new On/Off button or clickable low-level container requires updating this inventory
with a reason that a shared `Switch` cannot represent the state.
