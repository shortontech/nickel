# Linux Application Compatibility

Nickel's Linux session supports native Wayland clients and starts one rootless XWayland server for
legacy X11 clients. This document records the compositor policies, resource bounds, test matrix,
and known limitations for that compatibility layer.

## Protocol policy

| Capability | Nickel policy | Resource lifetime or bound |
| --- | --- | --- |
| Clipboard and drag-and-drop | Standard `wl_data_device` focus follows keyboard focus. Smithay owns client and server drag grabs. XWayland clipboard requests are bridged through the same seat. | Selection advertisements bridged by Nickel are deduplicated and limited to 64 MIME types of at most 256 bytes each. Offers and drag grabs otherwise live no longer than their Wayland resources. |
| Primary selection | `zwp_primary_selection_device_manager_v1` follows keyboard focus and is bridged to XWayland PRIMARY. | The same 64-type and 256-byte MIME limits apply to the XWayland bridge. |
| Activation | `xdg_activation_v1` accepts a token only while it is younger than 10 seconds, its requester still owns keyboard focus, and its serial belongs to Nickel's seat and is no older than that focus entry. A token is consumed after one request, accepted or rejected. | At most 256 unconsumed tokens are retained. Expired tokens are removed when another token is created. |
| Text input and input method | `zwp_text_input_manager_v3` follows the focused Wayland surface. `zwp_input_method_manager_v2` is session-local and supports composition popups. | Smithay permits one active input method per seat. Popup resources are tracked by `PopupManager` and released with their clients. |
| Relative pointer and constraints | Relative motion uses the physical backend delta. Locked pointers retain their logical position; confined pointers remain inside the production surface/input region. | The protocol permits one constraint for a pointer/surface pair. Constraints end with their protocol objects. |
| Idle inhibition | A live `zwp_idle_inhibitor_v1` prevents future session-idle actions while its surface remains alive. Multiple inhibitors on one surface are reference-counted. | At most 256 distinct surfaces are recorded as idle-inhibited; further distinct surfaces are ignored and logged. |
| Decorations | Nickel prefers server-side decorations for ordinary Wayland and managed X11 windows. Authenticated Nickel shell surfaces stay client-decorated and borderless. A client request for client-side decoration is honored. | Decoration state is one mode and one surface identifier per live toplevel. |
| XWayland | One rootless server receives a stable `DISPLAY`. Managed X11 windows enter the same registry, stacking, focus, preview, minimize, maximize, fullscreen, move, resize, and close paths as Wayland windows. | X11 state is retained only for live managed windows. A failed XWayland server is restarted after one second on the same display number; native Wayland clients are not restarted. |

All globals are available only through the per-user Wayland socket. Nickel does not expose a
cross-user input method, clipboard, or test-control endpoint. Test input is separately capability
authenticated, restricted to the nested backend, and disabled unless `--test-control` is explicit.
Toplevels from the authenticated shell client remain outside the application registry while their
app ID is still pending, preventing recreated previews and menus from briefly becoming taskbar
applications. A non-shell identity such as a Codex project window is admitted once its app ID is
known.

## Compatibility matrix

The final column is intentionally not considered complete until it is repeated from the Nickel
entry selected in SDDM. Nested runs are development evidence, not display-manager acceptance.

| Class | Representative client | Nested result | SDDM result |
| --- | --- | --- | --- |
| Protocol discovery | `wayland-info` | Required public globals advertised and client exits normally; `xwayland_shell_v1` remains correctly private to the supervised XWayland client | Pending |
| Native toolkit | KCalc with `QT_QPA_PLATFORM=wayland` | Native Wayland mapping and survival across XWayland restart verified | Konsole remained alive, mapped, and active while the supervised XWayland process was replaced |
| Legacy X11 | XTerm | Mapping, identity, focus, semantic keyboard input, and same-display XWayland restart verified | A new XTerm mapped, focused, retained `StartupWMClass` identity, and closed cleanly through Nickel after XWayland replacement |
| Chromium | Google Chrome with `--ozone-platform=wayland` | Native Wayland mapping and stable `google-chrome` identity verified | Native Wayland Chrome mapped and remained usable in the SDDM session |
| Electron | Visual Studio Code with `--ozone-platform=wayland` | Native Wayland mapping and stable `code` identity verified | Discord/Chromium-class Electron clients mapped and remained usable in the SDDM session |
| Activation | Two SCTK native Wayland clients | A focused source's fresh pointer serial produced a token that activated the mapped target; a token without a seat serial was consumed and rejected while source focus remained unchanged | Pending |
| Game/relative pointer | `vkcube --wsi wayland`, Chromium pointer lock, and the SCTK relative-pointer probe | Native Vulkan client mapped and remained alive across forced XWayland restart; Chromium entered pointer lock; persistent confinement, region confinement, lock activation, and exact accelerated/unaccelerated relative deltas verified through production input dispatch | Pending |
| Idle inhibition | Raw Wayland idle-inhibit client | Authoritative inhibited-surface count transitioned `0 → 1 → 0` across inhibitor creation and destruction | The supervised shell was replaced with the release build while Konsole remained mapped; the authoritative count fell from 12 shell-created inhibitors to `0`. A native client `0 → 1 → 0` transition remains pending. |
| Clipboard | Two Wayland clients, then Wayland and XWayland | Exact clipboard and primary-selection payloads verified Wayland-to-Wayland and in both Wayland/XWayland directions | Clipboard paste and primary-selection paste were observed between native clients; the explicit Wayland/XWayland direction matrix remains pending |
| Drag-and-drop | Two Wayland clients, then Wayland and XWayland | Copy negotiation, drop completion, and exact payloads verified Wayland-to-Wayland and in both Wayland/XWayland directions | Dolphin-to-Konsole payload delivery and the compositor-rendered drag icon were observed; the reverse and explicit Wayland/XWayland matrix remains pending |
| Composition | Chromium text area plus a raw `zwp_input_method_v2` client | The input method received activation and surrounding-text state, sent a live `é漢` preedit, and committed the exact payload; Chromium reported `compositionend:é漢` | Pending |
| Decorations | SCTK server-decoration client and raw client-decoration client | Wire-level configure events verified server-side mode `2`, then an explicit transition from the server default to client-side mode | Pending |

## Known limitations

- Cursor-position hints are accepted for locked pointers, but Nickel does not currently warp the
  pointer to the hint when a lock ends.
- XWayland clipboard integration covers CLIPBOARD and PRIMARY. Clipboard-manager persistence after
  the owning client exits is not provided by the compositor.
- Screen-cast portals, accessibility protocols, color management, and HDR are outside this
  compatibility milestone.
- Native DRM behavior, PAM wallet handoff, and complete application startup environment must be
  judged from an SDDM-launched Nickel session; a nested compositor cannot certify them.
