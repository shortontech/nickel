# MCP verification gates: 0230 and 0231

This is an open acceptance checklist, not a completion certificate. Every numbered
verification item in both active specifications is represented below. None is
currently proven across its full required platform and behavior scope. The source
and evidence pointers identify where further work must be verified; a historical
passing test does not prove a later integrated build or a broader native claim.

Evidence logs referenced here live in `target/mcp-native-2026-09-10/` unless an
absolute path is given. `mcp-implementation-status.md` records individual results.
The specifications' non-numbered requirements and Completion sections also remain
binding; this checklist does not replace them.

## Spec 0230

| Gate | Current evidence or implementation | Remaining proof or implementation |
| --- | --- | --- |
| 1 Default listener, zero authority | `settings.rs`, `listener.rs`, server admission; mandatory-watch native fixture rejects requests without ready presence. | Current native default-start and every preapproval observation/action denial on Windows as well as Linux. |
| 2 Scope, clock, renewal, resumption, generation, outcomes | Shared `leases.rs`, `connection_watch.rs`, `lease_requests.rs`; pending-generation and watch-gap regressions; Windows owner projections compile and pass portable policy tests. The Windows owner now samples its input desktop and WTS session before watch activation and revokes runtime authority before queued work on an unlocked-to-protected transition. | Native Windows lifecycle and requested/confirmed outcomes, including HWND reuse and secure-desktop transition execution. |
| 3 Sustained actions in each scope | Linux owner adapters implement window/application/output/full-session operations. The Windows owner now admits local approval/resume only after fresh exact resource validation and trusted-indicator exposure, routes ordinary input/window-manager requests through generation, scope, process, geometry, lifecycle, permit and secure-desktop revalidation, then obtains separate fresh observed outcomes. Windows authorized-window capture, bounded external-window and application-wide UIA observation, and generation-bound UIA Invoke use the same owner identity boundary and final fresh owner revalidation. | Complete the live multi-action matrix for every scope on both platforms; execute approval, input, capture, UIA observation and UIA Invoke natively on Windows. |
| 4 No scope widening | Generation-bearing owner checks, continuous input permits, bounded launch placement and capture workers. | Full transforms/transients/grabs/delayed-effect matrix; ambiguous and broker identities; Windows owners. |
| 5 Native application identity | Linux Wayland peer/XRes process evidence, executable identity, and fail-closed Flatpak identity combine sandbox metadata, unique installed desktop-entry launch identity, and a matching Wayland app ID or XWayland class/alias. Shared-runtime transients inherit only through a live native parent with the same current process. Windows retained process/image evidence, catalog receipts, package and executable membership policy. The Windows owner now projects the same production Start Menu discovery through a stable, freshness-bound catalog generation; application scope filters it by verified identity and full-session scope receives at most 512 entries. Windows launch uses a suspended one-shot broker with an exact inherited-handle allowlist and derives its target from the pinned shortcut rather than accepting a path or command. Output-scoped launch retains exact process-incarnation and bounded ancestry evidence, places the attributed first window through the production owner, and withholds that resource from remote publication until a later contained observation. | Execute Flatpak/shared-runtime and parent inheritance with live native Wayland and XWayland clients; native Windows packaged and unpackaged inventory/launch execution, including broker inheritance, attribution, and visual output placement. |
| 6 Production-owner lifecycle scenarios | Linux owner tests and separate native fixtures cover individual operations/lifecycle transitions; typed shell semantic mutations, guarded device controls and bounded external AT-SPI observation are integrated. GTK delayed-authority revocation, replacement-menu focus/Escape and physical action dispatch pass natively on the current integrated binary. Production-effect events carry a payload-free operation number; a native catalog launch correlated owner completion to the matching server operation. Windows window actions use the production resource owner and fresh native outcome observation. | Full operation-by-transition matrix and native Windows execution. |
| 7 Physical emergency during every operation | Shared atomic stop latch, Linux dispatch, Windows atomic chord recognizer tests. | Actual two physical Control keys during every held/pending operation, both platforms; no late effects. |
| 8 Trusted local indication, remote exclusion | Linux final composition/protected filtering; shared indicator UiHost semantic Stop tests. The Windows indicator projects the same bounded tree and Stop action through a dedicated AccessKit UI Automation adapter. Windows authorized-window capture reads only the target client DC rather than composed desktop pixels and revalidates its protected/resource boundary before publication. | Audit every observation path; native assistive workflow; Windows UIA execution, persistent host validation and native capture-exclusion proof. |
| 9 Request UX and rate behavior | Coalescing/generation, cooldown/blocking/renewal policy and bounded admission tests. | Realistic concurrent native request load with measured interaction responsiveness and full local UX traversal. |
| 10 Native scopes and assistive workflow | Isolated nested Linux fixtures provide partial evidence. | Each scope with actual local assistive input on Linux and Windows; physical backend acceptance. |
| 11 Movement and launch inheritance | Linux output-placement native tests; window/workspace owner paths. | Complete movement matrix and different-app denial with already-applicable lease success; Windows native equivalent. |
| 12 Overlapping input owners | Production arbitration and disconnect cleanup tests; watch native fixture releases held input. Windows low-level hooks exclude injected events from a payload-free physical-input epoch. Bounded keyboard and pointer transactions plus held chords/drags are owner-routed; registered keys/buttons are synchronously released on physical input and owner-observed focus/resource loss, timeout, disconnect, revocation, desktop and emergency transitions. | Execute the Windows native overlap matrix across all key/drag/focus/cancel combinations and prove preservation of physical local input on both platforms. |

## Spec 0231

| Gate | Current evidence or implementation | Remaining proof or implementation |
| --- | --- | --- |
| 1 Exact environment address and Settings | Shared listener parser and transport state; Linux native listener tests. | Full current Windows startup/bind failure/no-fallback/environment-ownership UI matrix. |
| 2 Protected transport and zero authority | TLS listener, explicit-cert/hostname client tests and native TLS fixture. | Real non-loopback two-machine transport and preapproval matrix, with confirmed host identity. |
| 3 Linux–Windows debugging both directions | Rust stdio adapter maintains authenticated watch; Windows control owner incomplete. | Native cross-machine approval, reproduction, correlated diagnosis and verified result in both supported directions. |
| 4 All diagnostic domains/live failures | Bounded snapshots/logs/events/traces, including payload-free trace lifecycle transitions; typed shell behavior, appearance, application scale, launcher favorites, guarded devices and external accessibility; application inventory and all five typed platform re-query domains use bounded production workers and compositor reconciliation; ordinary shell transients use live host protection evidence; protected-safe focus events distinguish ordinary windows, shell generations and cleared focus; external accessibility retains a freshness-bearing summary and operation-correlated event; protected-filtered renderer and shell-cache resource totals are derived from the coherent source records; the snapshot includes the redacted Codex projection and payload-free pending-effect aggregates, and explicitly lists unavailable domains. The Windows owner returns its protected-filtered scoped window/output inventory, focus, held-input state and independent bounded collectors under full-debug authority. It projects bounded production shell-surface incarnation, geometry, output, scale, focus, redraw, visibility, protection, scene-generation and per-surface native presentation evidence while excluding lock, Codex and trusted surfaces. Windows has no compositor-hosted ordinary application client: native Nickel tools are scoped native windows, Winit chrome is scoped shell surfaces, and protected Codex surfaces are excluded, making the internal-application inventory supported and empty. It exposes aggregate shell image-cache counts/bytes with preview totals filtered through the same native-window projection, and publishes DWM preview update state only when every current source remains inside that projection. Typed Windows appearance, file-icon and launcher-favorites transactions use production settings/launcher owners, checked file and catalog generations, staged cooperative writes and fresh protected-input authorization. All five Windows platform refresh domains use a single-flight two-second worker; maintenance PowerShell starts suspended, enters a kill-on-close job before execution and observes the live permit and shared deadline. Application preparation and live retained-child totals expose their existing bounded owner state. Temporary Windows frame traces retain at most 256 production `WinitShell::present` CPU-dispatch records with explicit ingress loss and permit-owned lifecycle. Shared GPU attribution and DWM pixel readback remain unavailable. Repaint, fresh native scene reconciliation and bounded application-catalog refresh route through their production owners and report requested rather than presented or reconciled; event polling and subscriptions retain bounded, coalesced remote-input ownership transitions without key, button, coordinate or timing payloads. | Implement and exercise every remaining unavailable domain/action/settings domain against live failures; execute shell-surface/resource observation, actions, refreshes, tracing, snapshot and event stream natively on Windows. |
| 5 Protected and secret denial | Production protected filtering and bounded payload-free collectors. | Review every new domain/operation plus external sensitive surfaces; native Windows boundary proof. |
| 6 Prometheus under churn/load | Fixed operation labels and bounded counters in `operation_metrics.rs`; the collector test exercises all 46 categories across success/error/cancellation and bounds the exposition, while a router test requires exact coverage of all 45 published tools plus the subscription path. The public ready-connection gauge counts distinct identities through overlap, unready reservation, close and expiry without identity labels. Historical native metrics fixtures provide earlier integration evidence. | Complete current native churn/denial/expiry/input/capture/diagnostic load matrix and responsiveness measurements. |
| 7 Bounds and responsiveness | Bounded queues/workers; adapter saturation tests prove EOF and cancellation bypass saturation. | Sustained integrated load and latency/memory measurements; remaining blocking filesystem/platform boundaries. |
| 8 Physical stop across work types | Atomic authority invalidation and cancellation checks; logical trace/stream cleanup tests. | Physical chord during held input, trace, stream, capture and diagnostic action on Linux and Windows. |
| 9 Indication everywhere, remote nowhere | Linux trusted composition; shared semantic host tests; hidden Windows HWND groundwork. | Full remote-path exclusion audit and native local accessibility/persistence on every output, both platforms. |
| 10 Native lifecycle/mixed DPI | `shell-renderer-native-results.txt` proves simultaneous nested 1.0/1.5 presenter scales and retirement. | Full lifecycle across physical multi-output Linux and Windows, including lock/restart/renewal/reconnection. |
| 11 Delayed subsystem snapshots | Renderer/worker snapshots expose own observations; all five typed Linux platform refreshes and application-catalog refreshes retain their own generation, observation interval, five-second freshness and shared worker state. All five bounded Windows platform providers use a tested single-flight timeout and expose their worker state; maintenance adds cancellable job containment for its production PowerShell processes. Launch preparation and live child totals are bounded; per-surface presentation and DWM preview updates retain separate native generations/failures. Unavailable domains remain explicit. | Controlled delayed GPU work and native Windows provider execution, proving timestamp/staleness semantics and unaffected input/presentation. |
| 12 Public metrics and input privacy | `trace-input-privacy-results.txt` shows real Gtk canary input absent from retained Linux trace logs. Metrics tests cover every fixed method/outcome, reject result/error/resource/title/path/credential/keystroke canaries, enforce a 64 KiB exposition bound and verify public connection counts remain identity-free. | All remaining collection paths, live credential entry and temporary trace categories, including Windows; current native prelease metrics/diagnostic denial. |

Windows shared software-raster cache accounting is now read synchronously from the production
`WinitShell` owner with its own mutation generation and the coherent snapshot timestamp. It is a
process aggregate of bounded counts, retained byte estimates, durable peak bytes, and activity
counters, with no content or per-surface attribution. The remaining resource gaps are shared-cache
attribution to individual surfaces, GPU timing/allocation accounting, and DWM pixel readback.

Windows application-scale policy read/change now uses the production bounded journal
and shared toolkit transaction engine. Preparation holds the stable lock and stages
the whole journal; the owner checks generation, journal revision, protected focus,
physical/shared input, local-input and emergency epochs, permit expiry and one
request deadline at write-through replacement. Existing GTK/Qt ownership and
pending-intent records remain intact while both unavailable native adapters are
reported explicitly. Native Windows policy/UI/physical-input execution remains open.

## Feature requirements outside the numbered gates

The following implementation gaps also prevent completion, even if a narrow test
for one numbered item passes:

- Windows native control, native validation of secure-desktop transitions, stable
  application identity, trusted local UIA and persistent per-output indication
  remain incomplete. The production owner now wires its existing fail-closed
  input-desktop/WTS observation into connection activation and synchronous
  runtime-authority revocation.
- Ordinary shell surface capture, semantic observation and typed semantic
  mutations are integrated, with owned nested Linux tests recorded in the
  implementation log. Bounded external AT-SPI observation is integrated;
  GTK-shell delayed gesture authority, replacement-menu local input and
  collision with trusted indication pass owned native acceptance.
- Snapshot `unavailable_domains` currently includes native GPU timing,
  GPU/shared cache resources and Windows DWM pixel readback,
  and additional event/trace categories. Explicit unavailability is truthful reporting,
  not completion of the requested diagnostic authority.
- Typed settings now cover shell behavior, appearance, application scale,
  launcher favorites and guarded device controls; combined checks and owned
  native transactions are recorded in the implementation log.
  Remaining nonprotected settings and safe production diagnostic actions require
  an explicit inventory and implementation, preserving protected domains.
- Audible lifecycle cues are integrated and native dummy-sink playback was tested.
  This does not establish actual physical volume/mute or Windows audio acceptance.
- Verified application selection before broad approval and remaining native
  broker/daemon launch association validation require further work. The Windows
  broker commit boundary is implemented but cannot be executed on this host.

Neither specification may be archived on this evidence.
