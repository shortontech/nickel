# Settings operation parity

Audit baseline: the native Settings application at `509dc0d2^`; replacement:
ordinary `nickel-default` pages and public capability clients. This is an operation
inventory, not a claim of native hardware testing.

| Area | Public page and capability | Linux | Windows |
| --- | --- | --- | --- |
| Theme, hue, intensity | Appearance; `appearance.get/set` | Light/dark/system, hue 0–359, intensity 0–100, system accent inheritance, custom hue, transparency, animation level, reset retained | Same preferences; native support reflected by snapshots |
| Wallpaper | Appearance; `wallpaper` | Position, custom image choice/reset, previews retained; chooser reports unsupported/cancel/failure | Same; native chooser and image persistence |
| Taskbar and desktops | Desktop and taskbar; `preferences` | Display scope, window scope, desktop count retained | Same |
| Idle behavior | Idle behavior; `preferences` | Dim, lock, suspend timeout settings retained | Same preferences; native effects depend on host support |
| Preferred applications | Preferred applications; `preferences` | Terminal/file manager choices, unresolved saved choice and system defaults retained | Same |
| File artwork | File artwork; `preferences` | Icon provider, named installed themes, unavailable saved theme retained | System provider support is reported rather than inventing installed Unix themes |
| Displays | Displays; `displays` | Layout, enable/primary, modes/refresh, scale, identify, preview keep/revert, application scale retained; orientation added | Native supported layout/scale/orientation/identify operations; preview validity is host supplied |
| Default applications | Default applications; `associations` | Target search, family filter, handler selection, protected-target checks, system fallback retained | Native consent is required where Windows protects defaults; no registry bypass |
| Optional features | Optional features; `features` | Keyboard preference/environment override; Codex policy/probe/retry/disable confirmation retained | Host snapshot distinguishes unsupported probes and unavailable runtime counters |
| Keyboard reference | Keyboard shortcuts; `shortcuts` | Production shortcut reference and availability | Unsupported global shortcuts are marked unavailable |
| Plugins | Plugins; `plugins` | Status, grants, enable review with current revision, disable, metadata controls, available memory categories retained | Same; no fabricated JavaScript heap or component memory |
| About | About; `system.get` | Package version, OS and architecture retained | Same |
| Wi-Fi | Wi-Fi; `wifi` | Saved profile connect, radio power, connected network disconnect; adapter names/types/state and optional speed | Saved profile connect and radio power; adapter names/descriptions/state and optional speed. Disconnect is unavailable |
| Bluetooth | Bluetooth; `bluetooth` | Power, discovery, explicit Pair, Connect/Disconnect; adapter name and available battery/type/RSSI | Power and native Pair. Continuous discovery and profile Connect/Disconnect are unavailable; battery/type/RSSI are null |

## Gaps repaired by this audit

- Linux Wi-Fi disconnect was absent from the public client. It now targets the
  current network identity and revision and invokes NetworkManager Device.Disconnect.
- Linux Bluetooth pairing was incorrectly unavailable. An explicit Pair request
  now invokes BlueZ Device1.Pair; connection remains a separate public operation.
- Network adapter details and Bluetooth adapter/device details were dropped from
  the migration. They now come from bounded native inventories and use null for
  unavailable speed/battery/type/RSSI.
- Read-only or locked connectivity callers previously saw writable operation flags.
  Their operation flags and network action availability now report false.
- Default application family filtering is restored in ordinary JSX.

## Deliberate limits

The old Wi-Fi UI supported saved profiles, not new credentials, enterprise setup,
or hidden-network configuration. The public page states that these require system
settings. Windows Bluetooth previously attempted profile actions through a pairing
helper; that helper explicitly rejected disconnect and supplied no safe profile
connect implementation. The replacement reports those operations unavailable.
Bluetooth Pair may still require the platform authentication/consent agent.

The old separate Bluetooth pairing window is represented by discovery and Pair
controls in the ordinary Bluetooth page. No private Rust page host remains.
The old Settings search could focus selected controls; the replacement searches
registered pages/settings and accepts native page navigation. It does not expose a
native page-specific focus adapter.

Derived Settings composition must receive `system`, `navigation`, `features`, and
`shortcuts` through the same owner-resource snapshots as other capabilities.
`navigation` is gated by SettingsRead, preserves revision and selected output, and
must be consumed only when its revision changes. These snapshot propagation fixes
are integrated by the coordinating composition work.
