# Native acceptance queue for the interaction backlog

Updated 2026-09-04. This is the remaining acceptance queue for Specifications 0121, 0122, 0136,
0149–0181, and 0188–0193. Terminal and Run work in Specifications 0183–0187 is intentionally
excluded. A specification stays active until every applicable item below has recorded evidence.
Cross-compilation, unit tests, and a nested compositor do not substitute for a criterion that names
an installed native session, physical hardware, or visual inspection.

## Explicitly excluded Run dependency

Specifications 0183–0187 and their Run/terminal deliverables are outside this wave. That exclusion
also applies narrowly to Spec 0136's `Super+R` behavior and Spec 0178's Run-field adoption and native
acceptance. Those clauses are **unimplemented**, not complete: `GlobalShortcut::ShowRun` currently
logs that Nickel Run is unavailable and performs no action. Specs 0136 and 0178 therefore remain
active for those clauses after this wave unless the specifications are split; no automated or native
result below is evidence for Run.

Nickel currently targets Linux and Windows. Scoped specifications that predated removal of the macOS
target have been amended accordingly; macOS is not a missing acceptance platform for this queue.

## Automated gates before native acceptance

The post-integration completeness audit on 2026-09-04 rechecked every scoped implementation against
its specification rather than treating existing tests as sufficient. It fixed concurrent and
repeated editing-shortcut suppression, current-folder Properties, rejected Codex-model recovery,
winit launcher placement on the active display, bounded deterministic MPRIS dispatch, and Windows
association-query translation. The combined tree passes the full Linux workspace test suite and
strict all-target/all-feature Clippy. The affected cross-platform package set also passes native
Windows tests and strict all-target Clippy after these fixes.

On 2026-09-05, the final Linux workspace run completed 82 suites with 1,805 passed, zero failed,
and 26 ignored release/manual benchmarks. Both full-provider reachability traversals completed with
all 4,285 advertised paths reachable across 99 fixture variants and zero issues. A subsequent
installed-session Alt+Tab check reproduced a compositor panic twice: a fitted `210 x 135` preview
was incorrectly required to occupy the maximum `240 x 135` byte count. Commit `6dc426c` validates
the frame's actual dimensions instead; its session suite passed 216 tests with one ignored
benchmark, strict session Clippy passed, and the user confirmed Alt+Tab no longer terminates the
installed two-output session. This closes the crash regression only; the visual and reverse-cycle
checks below remain native acceptance work.

Later on 2026-09-05, commit `c336c62` completed the shared file-plane presentation authority:
Nickel File, desktop entries, launcher search results, and launcher application tiles now use the
same component for hit regions, semantics, icon containment, bounded two-line labels, and
hover/selection/focus presentation. Idle tiles are colorless and borderless unless a host supplies
an active state, while desktop supplies only its label backing and layout policy. The relevant UI,
File, and Shell suites passed, strict workspace all-target/all-feature Clippy passed on the combined
tree, and the combined Shell suite passed 258 tests with eight explicitly ignored live/release
checks. Visual label containment and interaction on installed Linux and Windows shells remain native
acceptance rather than being inferred from those tests.

An SDDM restart later exposed a separate lifecycle defect: the prior shell survived its compositor
as a PID-1-owned orphan and retried a terminal winit pump about 1.18 million times per second while
querying its retired session socket. The replacement shell remained healthy at roughly 67 scheduled
iterations per second. Commit `a83a3ce` preserves ordinary Linux surface-close and headless-output
policy but turns winit's terminal `PumpStatus::Exit` into a distinct process-terminal event that
supersedes buffered input. Its focused regression and strict all-target/all-feature Shell Clippy
pass, and the release binary is installed. A later SDDM/compositor restart must still confirm that
the old shell is reaped and exactly one replacement starts before this native lifecycle criterion is
closed.

Subsequent installed-desktop testing on 2026-09-05 exposed three more interaction defects. Commit
`86cb5f0` coalesces consecutive native pointer motion per surface/device without crossing input
boundaries and routes desktop secondary input through the overlay host; its focused tests and strict
Shell Clippy pass. Commit `0792c6e` makes repeated secondary presses atomically replace the current
desktop menu and changes shared UI focus loss to dismiss every host-owned overlay, not only text
context menus. Repeated right-click, right-click/outside-left-click/right-click, release/motion, and
blur regressions pass, but installed visual confirmation remains queued. Commit `8db2855` adds
bounded native launcher icon resolution for Linux `.desktop` and Windows `.lnk`, `.url`, and `.exe`
entries, with Linux and native Windows tests and strict Clippy passing. Commit `667c9e9` additionally
caches locale-appropriate freedesktop `Name` metadata in each directory snapshot instead of
displaying opaque launcher filenames or reparsing files during paint. Its localized/fallback tests,
full 145-test File suite, and strict Platform/File/Shell Clippy pass. The user confirmed the installed
Linux desktop now shows human launcher names; icon appearance and menu behavior remain queued.

The same live pass found that `.desktop` activation opened the launcher source in VS Code. Commit
`238448c` classifies Linux desktop entries as launch targets and delegates execution to `gio launch`
without shell parsing; ordinary documents retain default-association behavior. Commit `5ce483c`
makes the desktop overlay own complete pointer transactions so a Rename-row click cannot reach an
icon geometrically beneath it; the real rendered-row regression, full 263-test Shell suite, and
strict Shell Clippy pass. Commit `ef3c0ad` fixes repeated artwork disappearance by retaining cached
icons across metadata-only directory invalidations and evicting only removed or meaningfully changed
entries. Installed confirmation of launcher execution, click containment, and stable artwork remains
queued.

Commit `b5c3f53` replaces the oversized flat desktop background menu with shared recursive `View`
and `Sort By` submenus. Pointer hover/click, keyboard left/right scope traversal, controller
activation, accessibility relationships, blur/cancel dismissal, RTL flipping, and output-bounded
placement share the UI overlay authority. Production-input regressions prove that Hide then Show
updates the persisted desktop-layout visibility state and that selecting an icon preserves its
nontransparent rendered artwork. The combined UI and Shell suites passed 582 tests with nine
ignored live/release checks, strict all-target/all-feature Clippy passed, and the declarative display-
list/hit-authority gates remained at their zero consumer exception baseline. Installed pointer,
keyboard, controller, placement, and persistence confirmation remains queued.

Chrome Wayland upload and VS Code dirty-tab close then exposed invisible modal children: window
grouping admitted a second identity and Alt+` listed it without a preview, while the taskbar preview
path saw only the presentable parent. Commit `c28a262` fixes initial commits for registered-but-yet-
unmapped xdg toplevels, advertises `xdg-dialog-v1`, centers/constrains children relative to parents,
and raises/focuses modal children. The complete Session suite passed 204 tests with one ignored
release benchmark and strict all-target/all-feature Clippy passed. Native Chrome portal chooser and
VS Code confirmation acceptance require a compositor/session restart with the installed binary.

The Spec 0136 completeness audit found that `Super+P` still opened the generic Control Center rather
than the required projection chooser. Commit `70a5976` adds a dedicated keyboard/controller-
focusable chooser, filters choices against the live output topology, preserves preview Keep/Revert
and 15-second rollback, and clears false confirmation state when native preview dispatch fails. The
Shell suite passed 266 tests with eight ignored live/release checks, the core shell and workspace
scenario suites passed 22 tests, strict Shell Clippy passed, and the Windows GNU cross-check passed.
The physical multi-output, client, controller-parity, focus-restoration, and native Windows items
below remain required.

- **0136 — conventional shortcuts:** integrate and pass the complete non-terminal shortcut suite,
  including snap/restore, Show Desktop, notification history, display projection, controller/menu
  equivalents, suppression, and focus restoration.

Specification 0162 is complete and archived. Its refreshed source inventory, reuse-authority tests,
declarative display-list audit, parallel-hit-authority audit, and bounded custom-paint audit all pass
without increasing either exception baseline. Its platform-neutral consolidations retain no native
acceptance dependency.

## Installed Nickel Linux session

- **0121:** inspect grouped previews and Alt+Tab with wide, square, and portrait windows; verify
  proportions, letterboxing, titles, controls, selection, and activation.
- **0154:** with a realistically large application catalog, reach and activate the final entry by
  mouse wheel and keyboard.
- **0159:** verify consumed editing shortcuts never leak correlated text through native IME/input
  adapters; repeat on native Windows/Proton paths.
- **0161:** restart the session after changing workspace count, then exercise Ctrl+Alt and
  Super+Ctrl aliases with focused Wayland and XWayland clients.
- **0163, 0164, 0166:** open representative associated files; verify stationary truthful context
  menus, list/grid Ctrl-click and Shift-click selection, keyboard selection, and files-only,
  folders-only, mixed, and incomplete-metadata summaries.
- **0169:** acquire and drag shared scrollbars and exercise wheel, keyboard, and controller scrolling
  at representative scales and themes.
- **0170:** verify Alt+Shift+Tab direction, focus, repeat, suppression, and modifier-release behavior.
- **0175:** traverse Settings, launcher, task switcher, Codex, and Nickel File with keyboard and
  controller; repeat the applicable visual pass on Windows.
- **0178:** exercise the shared text menu in Settings, launcher/search, Codex, File rename, and lock
  fields, including multiline input, IME, clipboard, secure-field restrictions, blur, and dismissal.
- **0193:** verify Super+L and Ctrl+Alt+L from focused Wayland and XWayland clients with no competing
  shortcut daemon; inspect password alignment at supported scales. Verify Super+L on Windows.

## Physical two-monitor session

- **0149:** verify one independently sized bar and wallpaper per monitor across reconnect/re-enable.
- **0150, 0151:** invoke the launcher and create ordinary windows on each active monitor; dialogs stay
  with their parent and all geometry remains inside the selected work area.
- **0152:** open Dolphin context menus and nested submenus at the pointer on both monitors.
- **0153:** verify Codex, clock, and quick-settings popovers align with their invoking controls on
  both bars.
- **0172:** change every bar/window scope and workspace count, reconnect a monitor, restart the
  shell, then log out/in and verify effective persistence.
- **0176:** inspect idle, hover, selected, and menu surfaces on both monitors; menus remain visually
  distinct from tiles.
- **0181:** open the empty-desktop menu and each direct Settings destination on Linux and Windows,
  including secondary-output placement.

## Native media and audio

- **0122:** test physical Linux keyboard/headset controls against PipeWire and at least two MPRIS
  applications, including held repeat, lock, USB/Bluetooth output changes, and Wayland/XWayland.
  Repeat available physical media and volume keys on Windows.
  Do not inject these keys in a supposedly isolated nested session: Nickel currently controls the
  host PipeWire and MPRIS buses.

## Live Codex app-server

- **0155:** commit a model selection, blur/reopen it, and prove the next live turn uses that model.
- **0156:** resume identifiable persisted conversations from previews in the same window.
- **0157:** inspect and change the effective approval policy without performing an unsafe operation.
- **0158:** paste a real screenshot and send it with text to an image-capable model.
- **0189, 0190:** on Linux and Windows, cover Codex absent, disabled, signed out, failed/recoverable,
  and ready states; disable/re-enable it and confirm workers, subscriptions, caches, and non-Settings
  UI are actually retired and restored.

## Native file operations and desktop

- **0165:** on Linux and Windows, test rename, copy, cut, paste, and multi-item drag
  with external file managers, cross-filesystem moves, conflicts, cancellation, and external drops.
- **0167:** change a harmless association, re-query the effective handler, launch through it, and
  restore it on every claimed platform.
- **0180:** on Linux and Windows, test real desktop files and launchers, group selection, movement,
  sorting, drops, persistence, and monitor reconnect.
- **0188:** while Nickel File remains open, create, rename, move, delete, and change metadata from an
  external process on Linux and Windows; verify stable selection and bounded reconciliation.

## Native Windows input and shell integration

Automated native Windows evidence recorded on 2026-09-04: the cross-platform application/library
set (`nickel-core`, `nickel-input`, `nickel-platform`, `nickel-ui`, `nickel-file`,
`nickel-settings`, `nickel-codex-ui`, and `nickel-shell`) passed `cargo check --all-targets` on the
Windows MSVC host. Native tests then passed for core, input, platform, File, Settings, Codex UI,
shared UI, and shell, including the Windows raw-input and filesystem-watcher paths. Those runs found
and corrected portability defects involving the `.exe` suffix, Linux-only toolkit-scale variables,
CRLF source auditing, Windows hidden-file attributes, coalesced native watcher events, and truthful
non-Linux Wi-Fi controls. This is build and automated contract evidence only; it does not replace
the interactive acceptance below. The same cross-platform package set also passes native Windows
Clippy across all targets with warnings denied.

On 2026-09-05, an isolated native Windows release shell launched in interactive session 1 and
remained responsive. Its replacement, built from wallpaper commit `ad8e0cf` (integrated into master
as `1b4b052`), reached first shell in 1.59 seconds; the wallpaper adapter logged an unavailable COM
class and then successfully loaded the Windows `TranscodedWallpaper` fallback at 1920 by 1200. The
focused native wallpaper suite passed six tests and production-binary Clippy passed. This proves the
fallback and runtime path, but the replacement predates the shared file-plane commit and visual
wallpaper presence still requires observation on the attached display. A separate all-feature
fixture-only configuration defect found by the broader Clippy command was corrected in `a3c9228`
without broadening the Linux-only notification service API: the cross-platform fixture now builds a
deterministic `DesktopNotification` directly. Native Windows all-target/all-feature release Clippy
then passed, as did 14 focused native notification tests; the corresponding Linux notification
tests and strict workspace Clippy also pass on the integrated tree.

The isolated Windows shell was then rebuilt from exact master `6a0a572`, including the shared
file-plane, wallpaper fallback, and fixture-portability changes. Its native serial release build
passed in 2 minutes 18 seconds; the exact-master binary remained responsive in interactive session
1 at about 81 MiB and reached first shell in 1.57 seconds. Its log again recorded the unavailable COM
class followed by a successful 1920 by 1200 `TranscodedWallpaper` fallback. Temporary launch-task
registration was removed and the user's dirty primary Windows checkout was not modified. Wallpaper,
label containment, and icon selection/navigation still require direct visual observation.

- **0136:** test every owned non-terminal shortcut and conflict/failure path, including controller
  equivalents and exact focus restoration. Unsupported virtual-desktop operations must remain
  truthful no-ops.
- **0168:** test focused/global keyboard input, layouts and IME, pointer/wheel input, focus loss,
  registered ownership, suppression, and registration failures.
- **0177:** test the supported taskbar window-action subset; pair it with Linux KDE/Qt and
  Wayland/XWayland menu acceptance.
- **0179:** test pin identity, launch, grouping, reorder, persistence, and recovery for desktop,
  packaged, and conventional executable applications on Linux and Windows.
- **0180:** verify the shared file-plane item visually contains long one- and two-line labels and
  exposes selection, keyboard focus, activation, and host-specific context menus on Windows; idle
  launcher, File, and desktop icon tiles must not regain unconditional borders or backgrounds.

## Native visual, Settings, and default handlers

- **0171:** inspect Codex selector and task switcher under every supported theme, including live theme
  changes and forward/reverse switching.
- **0173:** audit Linux and Windows at low/high DPI under dark, light, high-contrast, and custom
  themes; pure black/white remains confined to the independently configurable terminal viewport.
- **0174:** exercise the real Wi-Fi switch and representative migrated Settings controls on Linux
  and Windows without presenting unsupported operations as available. The checked-in
  `docs/settings-control-dispositions.tsv` is the machine-checked control inventory; native testing
  remains required for the accepted production components and documented custom composites.
- **0191:** change and restore Linux MIME/URI and Nickel terminal/file-manager handlers; exercise and
  verify supported Windows consent workflows and effective handlers.

## Physical mixed-DPI session

- **0192:** on Linux, move GTK 3/4, Qt 5/6, native Wayland, and XWayland windows across two physical
  mixed-DPI displays and exercise spanning, hysteresis, hotplug, toolkit compatibility, and restart
  persistence. On Windows, repeat with per-monitor-aware and legacy applications and verify
  sign-out/restart behavior.

## Safe nested coverage before physical runs

An explicitly test-controlled nested Smithay session may automate output hotplug, fractional scale,
surface cardinality, active-output placement, workspaces, lock surfaces, readiness, and bounded
resource diagnostics. It necessarily creates a visible host window today; Nickel has no supported
start-hidden/minimized nested mode. Test credentials must be read from the supervised shell child's
environment without printing or persisting them, and the disposable session/process group and
temporary sockets must be removed after the run.

### Recorded nested evidence

On 2026-09-04, an isolated `backend-winit --test-control` run reached a healthy one-output baseline,
added a rotated `1024 x 768` output at `180/120` scale, and converged to exactly two desktops, two
panels, two lock surfaces, and one launcher. It remained ready while locked, then converged
immediately back to the exact one-output role set when the secondary output was disconnected.
Reconnecting while locked restored the two-output set. The workspace command created a fifth stable
workspace, cache diagnostics stayed within their declared zero-preview baseline, and all disposable
processes, sockets, logs, and the synthetic output were removed afterward. A defect found during the
run—readiness counting dormant reconnect-grace surfaces—was corrected before this evidence was
recorded. Registered hidden surfaces now remain visible in diagnostic output without being counted
as active-topology readiness roles.

This closes only nested structural coverage. It does not close any physical-display, native toolkit,
hardware-input, visual, PAM, file-manager interoperability, live Codex, or Windows item above.
