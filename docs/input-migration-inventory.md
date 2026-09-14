# Input and interaction migration inventory

Updated: 2026-09-14

This is a source inventory, not a native acceptance record. “Migrated” means the production path
uses the shared normalized-input, controller-broker, window-operation, or geometry-authority model.
It does not mean that the path has been exercised in an installed Linux or Windows session.

## Producers and normalization boundaries

| Producer | Production boundary | Identity and order carried forward | Routing / cancellation capability | Status |
| --- | --- | --- | --- | --- |
| Focused winit keyboard, text/IME, pointer, wheel, touch and focus events | `nickel_input::winit::Adapter` | generated `DeviceId`, monotonic `EventOrder`, key identity, pointer/touch device and contact | focus loss and device removal reset held state; host dispatch returns an explicit disposition | migrated |
| Smithay libinput keyboard, pointer and touch | `session::input`, Smithay seat, then `InputEvent` for compositor UI | seat/device lifetime, serial/time, button or contact | seat-wide cancel, focus/device/target cancellation; completed-frame touch cancel is forwarded unconditionally | migrated; vendor fix carried in `vendor/smithay` |
| Smithay compositor shortcuts | `CompositorShortcutAdapter` | normalized key and modifier state | typed shell action; lock/focus teardown cancels repeats and interactions | migrated boundary |
| Win32 focused winit input | shared winit adapter and `UiHost` | normalized identity as on other winit hosts | focus/device reset and explicit host disposition | migrated |
| Win32 `RegisterHotKey` and keyboard hook | `WindowsInputAdapter` in `platform::windows` | physical key/scan code and shared modifiers | synchronous hook suppression; typed outcomes delivered once | migrated boundary |
| Win32 low-level pointer hook | `WindowDragCoordinator` plus `WindowOperationReducer` | window lifetime/generation, source generation, initiating button, operation/acquisition | source tests present; foreign geometry admission and native effects are disabled because no bounded identity-bearing completion mechanism is available | unavailable; source fail-closed, Windows tests/native unrun |
| Gilrs controller reader | `nickel_input::gilrs`, `ControllerNormalizer`, `ControllerInput` | controller lifetime, backend/native identity, fingerprint, family, edge, time | focus fence suppresses held input; disconnect/neutral reset; drain bounded to 256 events/poll | migrated |
| Unix session controller worker | `ControllerBroker` through local transport | host connection, lease, stream generation and event ID | revoke cutoff, transfer deadline, overflow reset, neutral barrier | migrated |
| Windows session controller worker | `server_windows::WindowsPipeServer`, `client_windows::AsyncControllerConnection`, and `ControllerBroker` source | correlated bounded frames plus broker host/connection/lease/stream/event identity | adapter source exists, but no production `WindowsPipeServer` construction publishes the broker feed and the direct shell reader is disabled on Windows | unavailable; production integration missing |
| Authenticated semantic/test input | session protocol, `session::test_input`, production hit testing/reducers | authenticated scope and semantic target | explicit press/release/cancel and lease teardown; no alternate reducer | intentional test boundary |
| Remote pointer/keyboard/controller | remote-control admission and session adapters | lease/capability, source generation and execution identity | timeout, focus, disconnect, emergency stop, lock and revocation fence/cancel | migrated |
| On-screen keyboard / input-method protocol | `session::on_screen_keyboard` and focused Smithay keyboard source | focus-bound auxiliary keyboard source and frame order | source cancellation releases only its own keys; unsupported text fails before partial delivery | migrated boundary |
| Accessibility semantic invocation | trusted/native accessibility adapters -> semantic UI action | resource/semantic node identity and action execution identity | stale/revoked nodes fail closed; bounded observation and dispatch cancellation | migrated boundary |

Native types are expected only in these adapters. Application hosts consume `InputEvent`, typed
`ControllerAction`, `HostEvent`, or typed shell/global outcomes.

Normalized ingress is admitted against authority owned by the live producer/runtime registry, not
authority reconstructed from the envelope. Internal surfaces bind renderer runtimes to coordinator
surface lifetimes and separate opaque nonzero recipient leases; routed desktop, keyboard,
clipboard, screenshot, overlay and notification batches preserve that authority end to end.

## Consumers

| Consumer | Routing authority | Cancellation behavior |
| --- | --- | --- |
| `FocusedInputDispatcher` / `UiHost` | active widget identity, semantic-tree hit testing and a shared default-activation prefix for pointer, keyboard and controller activation | focus loss clears capture/preedit/held state; handled disposition prevents duplicate message/fallback activation |
| Shell, launcher, lock, screenshot, overlays, notifications, task switcher | shell-surface identity and production geometry | overlay dismissal, focus transfer, lock and teardown cancel owned gestures |
| Settings, File, Gaze, Shapes, embedded Codex | per-host active widget and semantic target | focus loss/device removal; stale widget identity is not retargeted |
| Wayland/XWayland clients | mapped window, native lifetime and mapping generation | source/resource/seat loss, unmap/destroy, lock, suspend and supersession terminate |
| Internal surfaces/titlebars | internal generation, hit-test kind/subject, initiating button; normalized keyboard and admitted controller events enter the same host identity boundary | removal, grab loss, lock/suspend and matching release terminate; stale controller binding is rejected before host effects |
| Windows foreign windows | `HWND` mapping lifetime and initiating source/button | takeover, failed apply, missing release or source loss terminate; no exclusive-native claim |
| Controller hosts | live compositor route + surface generation + connection + lease + stream generation | revoke/reset invalidates queued and held/repeat state; execution is rechecked against current route and focus before effects |

## Window and geometry writers

`WindowOperationReducer` admits at most one operation per seat and logical window. Each operation
binds `WindowMapping` (`WindowId`, `NativeLifetimeId`, `MappingGeneration`), kind, control mode,
origin/current completion binding, acquisition, resource lease and binding epoch. It distinguishes
applied input, consumed tails, unrelated input and rejected transitions.

| Writer/path | Shared lifecycle | Control / settlement | Status |
| --- | --- | --- | --- |
| XDG client move | `handlers::xdg_shell` -> `WindowPointerOperation` | `Enforced`; compositor placement | migrated |
| XDG client resize | XDG handler -> operation/resize grab | `Cooperative`; configure acknowledgement protocol-owned | migrated |
| Titlebar and Super+pointer move | `session::input` -> move grab | `Enforced` | migrated |
| Internal compositor-surface move | input -> internal move grab | `Enforced`; surface generation bound | migrated |
| XWayland move/resize | XWayland handler -> shared grabs and per-mapping `Settlement` | `Enforced`; client request is independent, tokenless configure notification has `Unknown` causality, and equal unknown evidence remains pending until its 750-tick deadline becomes `Unconfirmed` | migrated; native untested |
| Windows foreign move/resize | `WindowDragCoordinator` | `ExternallyContested`; admission and native writes fail closed without bounded request-identified completion | unavailable; source gates present, Windows tests/native unrun |
| Temporary/no-output placement, internal/maximize restore, presentation | `GeometryAuthority` revisioned desired fields | owner/control/topology/revision-tagged requests and observations; internal restore checks the captured revision before write | migrated |
| Renderer/layout projections and configure emission | platform/session adapters consume authority | projections, not independent desired-state writers | retained boundary |

No inventoried production move/resize path owns a second ad-hoc admission state machine. Native grab
objects, coordinate conversion, `SetWindowPos`, and protocol configure delivery remain adapters.

## Cancellation and stale tails

The reducer names user cancel, completion source/resource/seat loss, lock, suspend, target unmap or
destroy, supersession, native takeover, unknown authority, acquisition failure and release before
activation. Handoff retires the old binding, so its later release/disconnect is consumed. Security
teardown uses `cancel_all`; lock/suspend also cancel Smithay and internal-UI touches, controller
repeats, remote pointer and remote keyboard. Window teardown cancels before identity reuse.

Cancellation effects now retain their `CompensationDecision`. Adapters execute `Conditional`
compensation only when the recorded field revision is still operation-owned; `SkipSuperseded`,
`SkipTargetGone`, and `SkipAuthorityLost` do not restore. Move, resize, internal move, XDG and
XWayland wrappers carry the decision rather than inferring unconditional rollback.

The vendor touch boundary handles `down -> frame -> cancel`: cancellation walks targets Smithay
retains even when compositor pending-slot accounting is empty. Contacts changed in the completed
frame and contacts unchanged in it receive cancel; later motion/up cannot use the retired target.

## Bounded traces and diagnostics

| Facility | Bound / loss signal | Content |
| --- | --- | --- |
| `BoundedTrace` | default 128; oldest dropped; `dropped()` exposed | opaque test observations of production effects |
| Stateful generated traces | finite sequences; synthetic default deadline 2 s | operation/focus/geometry lifecycle and forbidden tails |
| Controller native drain | 256 events/poll; `backlog` on saturation | payload-free availability/activity/held observation |
| `ControllerBroker` | queue 256; transfer deadline 750 ms; overflow resets stream | event/connection/lease/stream identity and barriers |
| Operation, trace, lease, permission audits | 128 events each; oldest evicted | opaque IDs and lifecycle transitions |
| Desktop events | 128 plus eviction count | semantic desktop diagnostics |
| Frame trace | 256 records; requested duration 1–60 s; eviction count | frame category/generation/timing |
| Inventories | 512 windows, 32 outputs, 128 shell surfaces, 32 workspaces, 128 shortcuts | bounded structural observations |
| Diagnostic response | maximum 1 MiB | bounded serialized snapshot |

## Reproducible source audit

Review every match; native boundary matches are intentional.

```sh
rg -n 'winit::keyboard::|gilrs::|KBDLLHOOKSTRUCT|RegisterHotKey|SetWindowPos|Keysym' crates
rg -n 'WindowOperationReducer|WindowPointerOperation|ControlMode::' crates/nickel-core crates/nickel
rg -n 'cancel_all|CancellationReason::|cancel_normalized_touches|\.cancel\(self\)' \
  crates/nickel-core crates/nickel/src/session crates/nickel/src/platform/windows.rs
rg -n 'MAX_.*(TRACE|EVENT|DIAGNOSTIC)|DEFAULT_.*(LIMIT|DEADLINE)|VecDeque' \
  crates/nickel-core crates/nickel-ui crates/nickel-session-protocol crates/nickel-remote-control
```
