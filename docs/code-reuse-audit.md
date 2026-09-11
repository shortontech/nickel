# Nickel code-reuse disposition ledger

Audit date: 2026-09-04; implementation dispositions and inventory refreshed through 2026-09-10.
Scope: all 344 inventoried Rust sources under `crates/`. The exact per-crate snapshot is checked in at
`assets/code-reuse-source-inventory.tsv`; `reuse_authority` fails whenever a source or crate appears
or disappears without review. Candidates were grouped by behavior, then traced through callers and
tests; same-named trait implementations and platform translations were not treated as duplication.

| Candidate locations | Shared behavior | Intended authority | Disposition | Migration order / evidence | Tests | Status |
|---|---|---|---|---|---|---|
| `nickel-core::{shell_settings,wallpaper_settings,launcher_preferences,optional_features,dpi}` | Resolve Nickel's per-user configuration directory | `nickel-storage::config_path` | consolidate | The storage authority now lives below core, so portable domain state delegates environment and filesystem mechanics without creating a platform dependency cycle. All five consumers are guarded against restoring local path logic. | `reuse_authority`, storage and settings round trips | complete |
| Same five modules | Create parents and replace a complete small settings file | `nickel-storage::atomic_write` | consolidate | Seven preference/runtime writers delegate complete-file replacement to the lower-level storage crate. Core retains schemas and validation, while Windows replacement and directory mechanics stay outside the platform-neutral domain crate. | atomic replacement plus existing round trips | complete |
| `nickel-ui::{input,text_editor,text_context_menu,ui::tree}` and application text fields | Editing shortcuts and context actions | `nickel-ui::TextEditCommand` and retained editor | consolidate | Spec 0178 supplies the shared command authority and universal `TextField` adoption; retaining app-owned validation mappers is intentional. | editor/menu parity and secure-field suites | complete (0178) |
| `nickel-ui::{overlay,state,ui::tree,runtime}` and shell/file consumers | Menu focus, dismissal, placement, event containment | `nickel-ui::OverlayMenu` lifecycle | consolidate | Consumers declare menus; UI owns stack, focus return, collision, and accessibility. Direct native menu ownership has no production caller. | overlay matrix and semantic scenario suites | complete |
| `nickel-input::{lib,winit,windows,global}`; session and UI consumers | Native input normalization and shortcut suppression | `nickel-input` normalized events; `nickel-ui::FocusedInputDispatcher` for widget policy | adapter-only | Native scan codes/COM details remain in adapters. Moving widget focus policy down would create a dependency inversion. | adapter traces and correlated-text tests | complete |
| `nickel-core::{hotkeys,window_input,workspaces}` | Shell shortcut meaning | `nickel-core` reducers | share-primitive | Chord normalization is shared; workspace/window policy remains separate reducers because their state/effects differ. | hotkey and workspace tables | complete |
| `nickel-core::{active_output,output_layout}`; session shell layout; UI overlay geometry | Output selection versus rectangle placement | Core active-output/output-layout policy and UI popup collision | keep-distinct | Similar rectangle operations have different coordinate spaces and owners: compositor global outputs versus application-local popup work areas. Combining them would import compositor semantics into UI. | mixed-output core and overlay placement matrices | verified distinct |
| `nickel-core::dpi`, `nickel-session::shell_layout`, `nickel-shell::platform::linux` | Signed logical-rectangle intersection area | `nickel-core::geometry::LogicalRect` | share-primitive | The refreshed audit found three byte-for-byte-equivalent overlap algorithms. Domain-specific placement and capture policy remain separate, while arithmetic and hostile-coordinate handling now have one portable authority. | core extreme-coordinate tests plus existing DPI, placement, and capture selection tests | complete |
| `nickel-core::{display_projection,dpi}`, Nickel Settings, and session-protocol translations | Fractional display scale representation | `nickel-core::dpi::Scale120`; raw `u32` only at native/wire translations | share-primitive | Projection and Settings presentation policy now store the validated scale type instead of parallel raw-number representations. Session protocol and monitor snapshots retain integer fields as translation boundaries and validate them before policy or compositor application. | projection plan/rollback, Settings scale/apply, scale conversion, session layout validation | complete |
| `nickel-session` DRM/udev backends | Connector discovery | session backend contract | adapter-only | DRM object enumeration and udev lifecycle are native mechanics; portable state consumes typed output facts. | scanner/udev contract tests | complete |
| Nickel Settings and Shell Linux adapters | NetworkManager saved-connection discovery | `nickel-platform::network_manager_saved_wifi_connections` | consolidate | The shared platform layer now performs D-Bus profile discovery once; Settings and Shell retain product-specific presentation and activation policy. | Settings/Shell network suites | complete |
| `nickel-platform::linux` and `nickel-shell::desktop_entries` | Bounded desktop-entry loading and application-type admission | `nickel-platform::{desktop_entry_from_path,desktop_entry_is_application}` | consolidate | The platform crate owns file-type/size limits, parsing, and application-type recognition. Shell applies launcher-specific visibility, executable, desktop, localization, and ranking policy to the admitted record instead of reopening native input. | malformed/oversized desktop-entry tests and launcher discovery tests | complete |
| `nickel-shell::platform::{linux,windows,unsupported}` | Launch, enumerate apps, tray/audio/control effects | shell platform trait | adapter-only | Repeated method names are required trait translations. Policy is in launcher/model/live-shell; adapters contain only OS effects. | synthetic platform and native contract tests | complete |
| `nickel-shell::{launcher,model,places,desktop_entries}` | Application discovery, ranking, places | launcher model | share-primitive | Desktop-entry parsing is Linux-specific input; ranking and presentation consume canonical application records. Places are locations, not applications, so merging indexes would erase semantics. | launcher ranking/discovery tests | complete |
| `nickel-platform::default_apps`, Nickel Settings, File Properties | Default association discovery/change | process-wide `AssociationService` | consolidate | Both UIs consume one generation-bearing service; platform backends remain typed adapters. | re-query/change verification and consumer tests | complete (0167) |
| `nickel-file::platform::open_path` and default association launch | Default application activation | `nickel-platform::open_with_default` | consolidate | Spec 0163 supplied the shared typed launch authority; integration removed Nickel File's duplicate platform launcher and routed desktop activation through it. | 0163 platform contract and activation failures | complete |
| `nickel-core::{shell_settings,wallpaper_settings,optional_features,launcher_preferences}` | Independent settings schemas | Each domain type | keep-distinct | Parsing looks similar (`key=value`) but keys, defaults, boundedness, and recovery contracts differ. A generic map would discard validation and typed ownership. Only storage mechanics were shared. | domain round trips/malformed input | verified distinct |
| `nickel-codex::settings` versus core settings | Private remote credentials/host configuration persistence | `nickel-codex::settings` | keep-distinct | Codex requires TOML validation, mode 0600, fsync, and directory sync. The small public preference writer deliberately cannot satisfy this security contract. | permissions, validation, atomic persistence tests | verified distinct |
| `nickel-file::{operations,desktop,properties}` | Filesystem mutation | Each typed operation domain over `FileIdentity` | share-primitive | Stable identity is shared. Transfer conflict/cancellation, desktop layout, and metadata editing have distinct state machines and failure semantics. | operation, desktop scenario, properties stale-target tests | complete |
| `nickel-file::icons`, `nickel-shell::icons`, `nickel-render-assets` | Artwork resolution and caching | Render assets for admitted bytes; product caches for semantic/native lookup | keep-distinct | File icons are MIME/path/provider and scale keyed; shell icons are application identity keyed. Sharing cache storage would couple unrelated eviction and native dependencies. | cache identity/budget/provider tests | verified distinct |
| `nickel-ui::{theme,components,settings_components,start_menu_components}` | Semantic color/focus presentation | `SemanticTheme` token roles | share-primitive | Components retain different structure but resolve shared tokens. One universal component would generalize incidental layout similarity. | theme sweeps and component state sheets | complete |
| `nickel-ui::{gpu,ui::tree,layout}` and shell presenters | Geometry/render helpers | Declarative UI tree and bounded paint commands | consolidate | Production applications declare components; renderer traversal/hit/display-list ownership remains in UI. Existing authority tests prohibit parallel consumer display lists. | `declarative_authority`, custom-paint bounds | complete |
| `nickel-ui-testkit`, per-product fixtures, workbench inventory | Fixture execution and visual acceptance | testkit contracts; product-owned fixture data | keep-distinct | Metadata implementations repeat trait shape, not behavior. Centralizing product fixtures would reverse dependencies and hide product state coverage. | fixture registry and manifest tests | verified distinct |
| Nickel File application and workbench fixtures | Synthetic fixture state and `nickel-ui-testkit` dependency | `workbench-fixtures` feature boundary | adapter-only | Fixture fields, constructors, and the testkit dependency are compiled only for tests or the explicit workbench feature; normal Nickel File builds no longer ship workbench infrastructure. Product-owned fixture data remains intentionally local. | Nickel File default/all-feature builds and workbench consumer inventory | complete |
| Per-application `Application::{view,update,poll_interval}` | Runtime integration | `nickel-ui::Application` trait | keep-distinct | These are required domain implementations. Their message types, polling sources, and views are intentionally application-owned. | host/application contract suites | verified distinct |
| `nickel-ui` controller/runtime and session-aware applications | Session controller fencing and on-screen-keyboard requests | application `HostAdapter` implementations | adapter-only | Generic UI now exposes controller-fence intent and scheduling only. Nickel File, Settings, and the recipient example perform session protocol I/O at their host boundary, eliminating blocking compositor IPC and the session-protocol dependency from `nickel-ui`. | controller schedule/fence tests and application host-adapter tests | complete |
| Nickel Shell binary and feature-gated library | Shell module ownership and entry point | `nickel-shell` library root | consolidate | The binary is a thin call to `nickel_shell::run`; production and workbench builds now share one module/type graph instead of compiling the same source through parallel roots. | Shell default/all-feature builds and workbench fixtures | complete |
| Nickel Shell, Settings, and File build scripts | Windows icon resource generation | `nickel-build-support::embed_windows_icon` | consolidate | Each build script supplies only its icon path and output name; image conversion, resource compilation, and rerun directives have one build-time authority. | workspace default/all-feature builds | complete |
| `nickel-ui::ui::tree` scrollbar layout/hit helpers | Scrollbar geometry | `nickel-ui::ui::tree::scrollbar` | split-seam | Pure thumb/track geometry and drag mapping moved behind a focused private module while `UiFrame` retains interaction and tree ownership. This reduces coordinator density without introducing a second hit-test authority. | scrollbar unit tests and UI tree interaction suites | complete |
| `nickel-ui::ui::tree` selection helpers | Selection geometry and generation tracking | `nickel-ui::ui::tree::selection` | split-seam | Glyph/run geometry, endpoint hit mapping, builder state, and selection generations moved behind a focused private API; `UiFrame` retains orchestration. | selection and UI tree interaction suites | complete |
| `nickel-ui::ui::tree` layout and display-list traversal | Tree measurement versus rendering emission | `nickel-ui::ui::tree::{layout,emission}` | split-seam | Recursive layout, flex/grid track resolution, and transient state application are isolated from recursive command emission, clipping, hit registration, and custom-paint validation. `UiFrame` retains cross-phase orchestration and the public API. | all-feature Nickel UI unit, integration, Clippy, and authority suites | complete |
| Nickel Shell desktop surface behavior | Desktop application state, input, view, and file effects | `nickel-shell::live_shell::desktop` | split-seam | The 1,460-line desktop domain moved out of the shell coordinator while `LiveShell` retains cross-surface coordination and wallpaper ownership. | desktop semantic scenarios and live-shell suites | complete |
| Nickel Shell panel application and live-shell behavioral tests | Panel state/view policy versus cross-surface coordination | `nickel-shell::live_shell::panel`; domain-grouped `live_shell::tests` | split-seam | Panel actions, drag reduction, menus, clock deadlines, tray normalization, and icon preparation now have one focused production module. The former inline test block is grouped by shell flows, desktop interactions, panel/cache behavior, and wallpaper behavior. | Nickel Shell library suite and strict source inventory | complete |
| Nickel Session output-global lifecycle | Bounded two-phase global retirement | `nickel-session::output_retirement` | split-seam | Capacity, grace transitions, and identity visibility are pure policy in one module; `NickelSession` retains compositor side effects. | retirement churn and session output suites | complete |
| Nickel Session preview caching and authenticated control protocol | Preview resource policy and wire-command handling | `nickel-session::state::{preview,control_protocol}` | split-seam | Capture admission, retry lifecycle, byte accounting, and retirement are isolated from authenticated socket ingestion, command/query dispatch, and protocol projections. The session coordinator retains compositor ownership and delegates through narrow internal APIs. | all-feature Nickel Session tests and strict Clippy | complete |
| Nickel File application and fixture source layout | Application runtime versus workbench fixture declarations | `nickel-file::{app,app::fixtures}` | split-seam | The library now uses a conventional application module, with the feature-gated fixture catalog isolated from runtime behavior. | Nickel File default/all-feature tests | complete |
| Nickel Session move and resize grabs | Pointer-event pass-through required by Smithay's grab trait | `nickel-session::grabs::forward_pointer_grab_events` | consolidate | One local macro implements identical relative-motion, axis, frame, gesture, start-data, and unset forwarding while each grab keeps unique motion/button policy. | Nickel Session grab and input suites | complete |
| Multi-output shell surfaces | Per-surface output identity | `nickel-session-protocol::ShellSurfaceIdentity` | consolidate | Capability-gated registration binds opaque surface IDs, canonical roles, and output names; window titles no longer carry protocol metadata, and reserved identities fail closed for unregistered or unauthenticated clients. | protocol round trips, authorization, and multi-output session tests | complete |
| `nickel-i18n` and `nickel-i18n-lint` | Runtime lookup versus source enforcement | Separate runtime and build-time crates sharing catalog conventions | keep-distinct | The lint performs source analysis and must not enter shipped runtime dependencies; runtime localization must not depend on repository source. | catalog and localization-lint suites | verified distinct |
| `nickel-session-protocol` and session state | Wire types versus compositor ownership | protocol crate for wire schema; session for live state | keep-distinct | Mirroring protocol facts into live handles is translation, not duplicated authority; the protocol crate cannot depend on Smithay. | serialization and session state tests | verified distinct |
| `nickel-gaze::{contract,grid,camera}` | Gaze samples, calibration grid, camera frames | Separate typed stages | keep-distinct | Coordinate conversion is shared through contract types; acquisition and calibration have different timing/lifetime constraints. | contract/grid/camera tests | verified distinct |

## Result

The audit now has lower-level authorities for configuration storage and Windows icon embedding;
shared Linux authorities for NetworkManager, desktop-entry admission, and icon-theme discovery; a
single Nickel Shell module graph; an explicit feature boundary around Nickel File fixtures; and
application host adapters as the boundary for session-specific input fencing. Typed shell-surface
registration replaces title metadata. Focused scrollbar, selection, layout, display emission,
desktop, panel, fixture, preview, control-protocol, and output-retirement modules reduce coordinator
density without creating parallel policy authorities.

The original strict clone scan fell from 15 groups and 352 duplicated lines to 9 groups and approximately
140 duplicated lines; the remaining groups are reviewed trait/fixture shapes or small local
translations rather than competing product authorities. The exact 344-source inventory is current,
and the executable audit guards the storage, geometry, display-list, hit-test, and source-count
boundaries against regression.

## 0220–0225 source additions (2026-09-08)

The previous inventory checkpoint `6c16784` listed 259 sources, including 92 in Nickel.
Six new Nickel modules bring those totals to 265 and 98. Reviewed ownership boundaries:

- `platform/status_mailbox.rs` owns replaceable status delivery and wake coalescing. It does not
  replace ordered command queues or duplicate backend device discovery. Audio activity metadata
  preserves transitions lost by snapshot replacement; the shell retains OSD policy.
- `session/internal_ui/desktop_input.rs` translates native pointer/touch/key identities into the
  existing normalized input contract and retains capture ownership. Desktop/keyboard reducers and
  hit testing remain in their existing owners. Generic hosted-app input still uses its adapter;
  complete native keyboard/clipboard normalization remains unfinished.
- `session/backend/udev/preview.rs` owns one primary-GPU submission and readiness timer, distinct
  from output presentation. Admission/cache/retry policy remains in `state/preview.rs`; the native
  module consumes that authority rather than retaining another frame cache.
- `session/preview_submission.rs` shares finish-before-error ordering between native and nested
  capture. Its caller supplies renderer completion policy: native uses nonblocking `try_finish`,
  nested retains synchronous completion. It does not claim to bound arbitrary driver latency.
- `session/native_clipboard.rs` owns native clipboard selection, asynchronous completion and
  recipient checks. Editor capability and cut admission remain in the shared UI text-command owner.
- `session/clipboard_transfer.rs` owns bounded descriptor I/O and worker permits, not selection or
  editor policy. The native text-size policy remains unconfigured pending the user's choice.

The pinned Smithay vendor is dependency source outside the workspace-crate inventory; its narrow
API patch and upstream provenance are recorded in `vendor/smithay/NICKEL-PATCHES.md`. This source
refresh does not represent a new clone-scan measurement or complete live acceptance of these specs.

## Authority exception baselines

- Display-list exceptions: the compositor renderer adapter has 78 reviewed references,
  the terminal viewport has six, and the custom-paint contract fixture has two. Native shell
  and compositor regression tests have test-only inspection bounds of four and three.
  The executable audit requires each exact reviewed count, rejecting increases, silent
  decreases, unlisted consumers, duplicate rows, and stale exceptions. Test-only exceptions
  cannot admit production display-list authority.
- Parallel consumer hit authority: zero files and zero references. The executable audit rejects the
  first unlisted authority and stale exceptions.
- The UI authority audit recursively scans every crate rather than a hand-maintained consumer list;
  it excludes `nickel-ui` itself because that crate is the intended display-list and hit-test owner.

The 2026-09-05 refresh did not increase either exception baseline.

## 2026-09-10 MCP authority inventory refresh

The source inventory now includes the 19-source `nickel-remote-control` crate,
12 additional Nickel sources, and the logging diagnostic collector. This refresh
records the new owners introduced for Specs 0230/0231; their specifications remain
active and native/platform acceptance is incomplete.

- `nickel-remote-control` owns lease policy, bounded request/admission state,
  typed wire contracts, transport, operational metrics, and payload-free event,
  trace, and audit retention. TLS/HTTP/MCP transport uses rustls, axum, and rmcp;
  capture encoding uses the shared image dependency. Native window policy and
  effects remain at the compositor boundary rather than inside HTTP handlers.
- `session/remote_identity.rs` supplies native process/resource evidence while
  reusing the production desktop-entry index. `remote_indicator.rs` owns trusted
  presentation. Neither establishes a second window registry or lease authority.
- The session `remote_*` modules translate approved typed requests into the
  existing production keyboard, pointer, controller, renderer, launch, settings,
  and diagnostic owners. `native_key_worker.rs` isolates bounded native query
  work. `remote_worker.rs` shares preparation admission and worker-state reporting
  across launch/catalog and settings paths.
- `session/window_capture.rs` shares bounded renderer submission/readback
  mechanics. Capture policy, resource evidence, and final authorization remain
  with the compositor request owner.
- `nickel-logging/diagnostics.rs` collects warning/error source metadata without
  visiting event or span fields. It does not parse or duplicate the file log.
- Shell settings now delegates preparation and replacement to
  `nickel_storage::stage_write` and `StagedWrite::commit`. The domain crate owns
  serialization; the storage crate still owns temporary files and replacement.
  The executable architecture check recognizes this shared staged-write entry
  point and rejects direct writes or renames in the settings modules.

The display-list inventory also records two bounded test-only inspection
exceptions: four variant references in `internal_shell.rs` and three in
`session/state.rs`. All seven are inside the final `cfg(test)` module; production
has zero variant references in those files. They inspect emitted text/image
output and resolve a visible sidebar label for native pointer dispatch. The
executable audit rejects production references under these test-only exceptions.
The existing compositor renderer's reviewed count is 78 (46 production adapter
references and 32 rendering-test references), correcting its stale count of 74.
No new production painting or hit-test owner was added. The hit-test exception
baseline remains empty. Source counts do not constitute a new clone-scan or proof
of completed native acceptance.

The subsequent Windows identity foundation adds one `nickel-platform` source.
`process_identity.rs` owns the limited-rights process handle, OS creation-time
query, package-family query, and nonblocking liveness check. It does not create a
window registry, lease policy, or permission path. The source inventory records
ten platform sources; native Windows execution and desktop-owner wiring remain
pending.

Session identity and fresh process-protection queries extend this same process
evidence owner. They introduce no window or authorization registry; unavailable
protection evidence fails the eligibility check instead of granting access.

Token integrity evidence stays in that module too: one temporary query-only
owned token handle, fixed bounded storage, and checked mandatory-SID decoding.
It adds no token serialization, logging, impersonation, or application-level
permission policy. The Windows owner still needs to consume this evidence.


## MCP connection and Windows transport additions

`nickel-remote-control::{connection_watch,emergency}` own shared logical presence
and atomic authority invalidation. Desktop owners consume those authorities rather
than copying lease policy. `nickel-platform::local_control` owns Windows named-pipe
peer verification and framing; `nickel::windows_remote_control` owns Windows runtime
commands and projections. Those OS-specific transport mechanics stay separate from
Linux Unix-datagram I/O while both consume the shared session protocol.

`nickel-mcp-client` is a separate executable transport adapter: bounded stdio/HTTP
forwarding, TLS configuration and watch maintenance belong on the client, while
approval, lease expiry and resource decisions remain on Nickel's server. Its SSE
framing translates wire data and does not introduce a desktop authority. Client
saturation/cancellation coverage is integrated; Windows native acceptance remains pending;
this source-inventory update records module ownership, not spec completion.

The Windows physical chord recognizer now lives in
`nickel::windows_emergency_chord`: a small atomic adapter consuming normalized
Windows keyboard facts, compiled on Windows and for executable Linux tests. It
owns native-key recognition only; shared lease cancellation remains in
`nickel-remote-control::emergency`. No parallel permission policy is introduced.


## Shared trusted indication and local audio

`nickel::remote_indicator` now owns the existing indicator Application for both
platform hosts; the Linux session re-exports it instead of duplicating its UI.
`nickel-remote-control::local_cues` consumes production lease audit transitions
and deadlines. `nickel::local_cues` owns bounded local playback and consumes that
selector, with no remote payloads or new lease policy. Windows trusted-window
plumbing uses retained winit window identity and the existing platform adapter;
it remains inactive pending the rest of the trusted host and native acceptance.
The two audio sources and subsequent shell semantic adapter bring the inventory to 309.

The trusted Windows accessibility adapter adds one source, bringing the inventory
to 310. It projects the production indicator UiHost into AccessKit and forwards
only the existing local Stop action through a bounded owner mailbox; it introduces
no parallel product semantics or remote authorization route.

`live_shell::remote_semantics` selects existing production UiHost or retained viewport semantics for the authorized shell output without changing focus or creating another UI tree.

`nickel-remote-control::appearance` owns typed wire values; `session::state::remote_appearance` stages and reconciles through the existing ShellSettings writer and LiveShell apply path. The shared commit boundary only rechecks atomic cancellation/deadline while authority is already held.

`nickel-core::shell_settings` owns the stable cross-process transaction lock for
every cooperative ShellSettings save. Remote appearance and shell-behavior
preparation retain the same lock across bounded read, staging and final checked
replacement; their production owners add lease and input authority without a
second persistence protocol.

`nickel-remote-control::file_icons` owns the bounded, path-free provider/theme
schema. `session::state::remote_file_icons` projects the platform-owned installed
theme catalog, retains unavailable configured IDs, stages through ShellSettings,
and requests the production shell/file-manager refresh after the desktop owner
accepts the commit. Platform icon lookup and cache revision remain in
`nickel-platform` and `nickel-file`.

`nickel-remote-control::wallpaper` owns the path-free wire projection, while
`nickel-core::wallpaper_settings::PreparedWallpaperSettings` owns locked staging,
revision comparison and checked replacement for both local and remote callers.
`session::state::remote_wallpaper` adds lease, generation and input policy without
duplicating storage or exposing the configured image path.

`nickel-core::optional_features` now delegates bounded regular-file reads and
cross-process update exclusion to `nickel-storage`. Its prior polling lock and
stale-path deletion were removed; the two domain schemas and atomic serializers
remain distinct because their fields and recovery behavior differ.

The retained Windows executable-evidence adapter adds one platform source (311 total). It shares the existing kernel mapped-file verification with local transport, retains file pins independently of process lifetime, and does not promote executable equality into application membership.

The shared bounded D-Bus transport adds one platform source (312 total). It validates frame lengths before delegating parsing to zbus and centralizes authentication, byte, frame and descriptor limits for typed platform consumers.

The guarded shell launch continuation adds one Nickel source (313 total), reusing installed application preparation and native owner replay. Destination placement retains the invoking output.

The Windows application registry adds two Nickel sources (315 total), reusing installed shortcut discovery and retained process/image evidence. Registry policy and native probing remain separate; native Windows acceptance is outstanding.

The shared output-identification raster adds one Nickel source (316 total). Winit and DRM consume the existing badge; remote ownership reuses the production identification lifetime and exact output identities.

Application scaling adds two platform sources, one remote-control source and one Nickel owner source (320 total). Local Settings and MCP share the typed transaction engine, durable intent and fixed GTK/Qt setters.

Launcher favorites use `nickel-core::launcher_preferences::PreparedLauncherPreferences`
for local favorites, local recent-app recording, and typed remote transactions.
The shared storage revision validates descriptor identity without consuming content
at commit; the same stable sibling lock covers all preference writers. Local
staging and persistence run on one bounded worker with a lifetime and epoch check.
Remote preparation uses the existing bounded settings worker and commits under the
original desktop permit before reconciling the accepted launcher and panel model.
The wire DTOs exclude history and unavailable stored IDs; these adapters do not
duplicate preference serialization or authority.

Windows resource observation adds two Nickel owner modules (327 total). The
platform adapter performs bounded native enumeration and identity revalidation;
the remote owner retains generation, scope, and protected-resource policy.
Native Windows acceptance and UI Automation inspection remain open gates.

Native accessibility adds one bounded remote-control schema source and four
Nickel owner/adapter sources (332 total). The wallpaper transaction adds one
bounded remote-control schema source and one Nickel owner source (334 total).
The terminal presentation transaction adds one bounded remote-control schema
source and one Nickel owner source (336 total); it reuses the core terminal
settings serializer and storage transaction while excluding launch-policy text.
The OSK preference transaction adds one path-free remote-control schema and one
Nickel owner source (338 total). It reuses the optional-feature writer and the
compositor's existing runtime snapshot without exposing controller configuration
or recipient/input state.
The file-icon transaction adds one path-free remote-control schema and one Nickel
owner source (340 total). It reuses ShellSettings persistence, the platform theme
catalog and the production icon-cache refresh path without exposing legacy path-like
theme values. The Codex enablement transaction adds one remote-control schema and
one Nickel owner source (342 total). It reuses optional-feature staging and the
compositor-owned internal Codex host lifecycle; source paths, credentials, backend
payloads and active-chat closure remain outside the remote schema.
The idle preference transaction adds one remote-control schema and one Nickel
owner source (344 total). It reuses ShellSettings persistence and the existing
session `IdleController`; it exposes only dim and suspend intervals. Lock timing,
inhibitor ownership, authentication and system power actions remain excluded.
Application inventory refresh adds no source files. It separates the existing
platform discovery into preparation and publication, reuses the launcher/panel
reconciliation and icon-cache owners, and routes the path-free diagnostic action
through the existing bounded worker and compositor authority. The scan, catalog,
metadata, and Windows recursion limits remain platform discovery policy rather
than client-controlled payloads; the exact source inventory remains 344.
Connectivity, audio, peripheral, maintenance, and default-association diagnostic
refresh add no source files. They reuse the production NetworkManager/BlueZ and PipeWire workers,
`SystemStatusUpdate` compositor reconciliation, and the production peripheral
maintenance, and association services. Linux helper processes have owned process-group
deadlines and bounded retained output; Secret Service reads reuse the bounded
authenticated D-Bus transport. Path- and detail-bearing snapshots are reduced to
coarse status before owner delivery. The wire schema exposes only fixed domains
and coarse outcomes; bounded platform snapshots remain internal. The exact source
inventory remains 344.
GTK-shell association and AT-SPI
observation stay separate: the compositor owns surface identity and delayed
input provenance, while the bounded observer exposes only admitted metadata.
Owned GTK menu dispatch, stale-revocation replacement, Escape and physical
semantic-action acceptance are recorded in `target/mcp-native-2026-09-10/`;
native Windows UI Automation acceptance remains an open gate.
