# Nickel code-reuse disposition ledger

Audit date: 2026-09-04; refreshed after the backlog integration wave on 2026-09-05. Scope: every
checked-in Rust source under `crates/` (217 files after the consolidation below). The exact per-crate
inventory is checked in at `assets/code-reuse-source-inventory.tsv`; `reuse_authority` fails whenever
a source or crate appears or disappears without an audit refresh. Candidates were grouped by
behavior, then traced through callers and tests; same-named trait implementations and platform
translations were not treated as duplication.

| Candidate locations | Shared behavior | Intended authority | Disposition | Migration order / evidence | Tests | Status |
|---|---|---|---|---|---|---|
| `nickel-core::{shell_settings,wallpaper_settings,launcher_preferences,optional_features,dpi}` | Resolve Nickel's per-user configuration directory | `nickel-core::persistence::config_path` | consolidate | Introduce authority, migrate all five callers, and prohibit new environment/path copies. The refreshed audit added DPI persistence to the enforcement inventory. | `reuse_authority`, each settings round trip | complete |
| Same five modules | Create parents and replace a complete small settings file | `nickel-core::persistence::atomic_write` | consolidate | Introduce atomic primitive, migrate seven preference/runtime writers, remove direct non-atomic writes, and enumerate every consumer in the authority guard. | atomic replacement plus existing round trips | complete |
| `nickel-ui::{input,text_editor,text_context_menu,ui::tree}` and application text fields | Editing shortcuts and context actions | `nickel-ui::TextEditCommand` and retained editor | consolidate | Spec 0178 supplies the shared command authority and universal `TextField` adoption; retaining app-owned validation mappers is intentional. | editor/menu parity and secure-field suites | complete (0178) |
| `nickel-ui::{overlay,state,ui::tree,runtime}` and shell/file consumers | Menu focus, dismissal, placement, event containment | `nickel-ui::OverlayMenu` lifecycle | consolidate | Consumers declare menus; UI owns stack, focus return, collision, and accessibility. Direct native menu ownership has no production caller. | overlay matrix and semantic scenario suites | complete |
| `nickel-input::{lib,winit,windows,global}`; session and UI consumers | Native input normalization and shortcut suppression | `nickel-input` normalized events; `nickel-ui::FocusedInputDispatcher` for widget policy | adapter-only | Native scan codes/COM details remain in adapters. Moving widget focus policy down would create a dependency inversion. | adapter traces and correlated-text tests | complete |
| `nickel-core::{hotkeys,window_input,workspaces}` | Shell shortcut meaning | `nickel-core` reducers | share-primitive | Chord normalization is shared; workspace/window policy remains separate reducers because their state/effects differ. | hotkey and workspace tables | complete |
| `nickel-core::{active_output,output_layout}`; session shell layout; UI overlay geometry | Output selection versus rectangle placement | Core active-output/output-layout policy and UI popup collision | keep-distinct | Similar rectangle operations have different coordinate spaces and owners: compositor global outputs versus application-local popup work areas. Combining them would import compositor semantics into UI. | mixed-output core and overlay placement matrices | verified distinct |
| `nickel-core::dpi`, `nickel-session::shell_layout`, `nickel-shell::platform::linux` | Signed logical-rectangle intersection area | `nickel-core::geometry::LogicalRect` | share-primitive | The refreshed audit found three byte-for-byte-equivalent overlap algorithms. Domain-specific placement and capture policy remain separate, while arithmetic and hostile-coordinate handling now have one portable authority. | core extreme-coordinate tests plus existing DPI, placement, and capture selection tests | complete |
| `nickel-core::{display_projection,dpi}`, Nickel Settings, and session-protocol translations | Fractional display scale representation | `nickel-core::dpi::Scale120`; raw `u32` only at native/wire translations | share-primitive | Projection and Settings presentation policy now store the validated scale type instead of parallel raw-number representations. Session protocol and monitor snapshots retain integer fields as translation boundaries and validate them before policy or compositor application. | projection plan/rollback, Settings scale/apply, scale conversion, session layout validation | complete |
| `nickel-session` DRM/udev backends | Connector discovery | session backend contract | adapter-only | DRM object enumeration and udev lifecycle are native mechanics; portable state consumes typed output facts. | scanner/udev contract tests | complete |
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
| Per-application `Application::{view,update,poll_interval}` | Runtime integration | `nickel-ui::Application` trait | keep-distinct | These are required domain implementations. Their message types, polling sources, and views are intentionally application-owned. | host/application contract suites | verified distinct |
| `nickel-i18n` and `nickel-i18n-lint` | Runtime lookup versus source enforcement | Separate runtime and build-time crates sharing catalog conventions | keep-distinct | The lint performs source analysis and must not enter shipped runtime dependencies; runtime localization must not depend on repository source. | catalog and localization-lint suites | verified distinct |
| `nickel-session-protocol` and session state | Wire types versus compositor ownership | protocol crate for wire schema; session for live state | keep-distinct | Mirroring protocol facts into live handles is translation, not duplicated authority; the protocol crate cannot depend on Smithay. | serialization and session state tests | verified distinct |
| `nickel-gaze::{contract,grid,camera}` | Gaze samples, calibration grid, camera frames | Separate typed stages | keep-distinct | Coordinate conversion is shared through contract types; acquisition and calibration have different timing/lifetime constraints. | contract/grid/camera tests | verified distinct |

## Result

The audit eliminated four platform-directory implementations and five direct small-settings writers,
replacing them with two narrow primitives. The consolidation removes 140 production lines and adds
121 (including the cross-platform authority and its Windows atomic-replace implementation), a net
reduction of 19 production lines. Its more important effect is one configuration-root authority,
atomic replacement for every migrated public preference file and removal of prior path duplication
disagreement. The enforcement test makes both authorities non-regressive.

The refresh consolidates three overlap algorithms and two logical-rectangle definitions into one
primitive, and replaces the projection policy's two raw scale fields with the existing validated
scale type. The portable overflow-safe primitive makes the executable implementation nine lines
larger after removing the copies; its focused inline tests add another 37 lines. Combined with the
original slice, the audit's executable production-code impact remains a net reduction of ten lines.

No compatibility shims or deferred consolidation candidates remain. Native adapter implementations
and product fixtures remain intentionally distinct for the evidence stated above.

## Authority exception baselines

- Consumer display-list authority: one reviewed file,
  `nickel-ui-workbench/src/fixture_inventory.rs`, bounded to two `PaintCommand` references for the
  custom-paint contract fixture. The executable audit requires the exact reviewed count, rejecting
  increases, silent decreases, unlisted consumers, duplicate rows, and stale exceptions.
- Parallel consumer hit authority: zero files and zero references. The executable audit rejects the
  first unlisted authority and stale exceptions.
- The UI authority audit recursively scans every crate rather than a hand-maintained consumer list;
  it excludes `nickel-ui` itself because that crate is the intended display-list and hit-test owner.

The 2026-09-05 refresh did not increase either exception baseline.
