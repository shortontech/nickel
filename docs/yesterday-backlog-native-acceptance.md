# Native acceptance queue for the interaction backlog

Updated 2026-09-04. This is the remaining acceptance queue for Specifications 0121, 0122, 0136,
0149–0181, and 0188–0193. Terminal and Run work in Specifications 0183–0187 is intentionally
excluded. A specification stays active until every applicable item below has recorded evidence.
Cross-compilation, unit tests, and a nested compositor do not substitute for a criterion that names
an installed native session, physical hardware, or visual inspection.

## Automated gates before native acceptance

- **0136 — conventional shortcuts:** integrate and pass the complete non-terminal shortcut suite,
  including snap/restore, Show Desktop, notification history, display projection, controller/menu
  equivalents, suppression, and focus restoration.
- **0162 — reuse audit:** make the complete workspace green, including the declarative display-list
  and hit-authority audit, without increasing exception baselines.

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
  Repeat available physical media and volume keys on Windows and record macOS ownership support.
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

- **0165:** on Linux, Windows, and supported macOS, test rename, copy, cut, paste, and multi-item drag
  with external file managers, cross-filesystem moves, conflicts, cancellation, and external drops.
- **0167:** change a harmless association, re-query the effective handler, launch through it, and
  restore it on every claimed platform.
- **0180:** on Linux and Windows, test real desktop files and launchers, group selection, movement,
  sorting, drops, persistence, and monitor reconnect.
- **0188:** while Nickel File remains open, create, rename, move, delete, and change metadata from an
  external process on Linux and Windows; verify stable selection and bounded reconciliation.

## Native Windows input and shell integration

- **0136:** test every owned non-terminal shortcut and conflict/failure path, including controller
  equivalents and exact focus restoration. Unsupported virtual-desktop operations must remain
  truthful no-ops.
- **0168:** test focused/global keyboard input, layouts and IME, pointer/wheel input, focus loss,
  registered ownership, suppression, and registration failures.
- **0177:** test the supported taskbar window-action subset; pair it with Linux KDE/Qt and
  Wayland/XWayland menu acceptance.
- **0179:** test pin identity, launch, grouping, reorder, persistence, and recovery for desktop,
  packaged, and conventional executable applications on Linux and Windows.

## Native visual, Settings, and default handlers

- **0171:** inspect Codex selector and task switcher under every supported theme, including live theme
  changes and forward/reverse switching.
- **0173:** audit Linux and Windows at low/high DPI under dark, light, high-contrast, and custom
  themes; pure black/white remains confined to the independently configurable terminal viewport.
- **0174:** exercise the real Wi-Fi switch and representative migrated Settings controls on Linux
  and Windows without presenting unsupported operations as available.
- **0191:** change and restore Linux MIME/URI and Nickel terminal/file-manager handlers; exercise and
  verify supported Windows and macOS consent workflows and effective handlers.

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
hardware-input, visual, PAM, file-manager interoperability, live Codex, Windows, or macOS item above.
