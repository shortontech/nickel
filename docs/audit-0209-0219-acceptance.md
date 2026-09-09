# Audit 0209–0219 acceptance

Implementation and automated regression coverage are integrated from five isolated worktrees.
The checklist below records the remaining user-facing acceptance pass; an unchecked item is not a
claim of success. On 2026-09-07 the user requested archival of specs 0209–0219 after implementation
and initial live testing. They now reside in `specs/done/`; archival does not mark the remaining
acceptance checks or cache measurements complete.

## Initial live observations and follow-up

The release build of `6c16784` succeeded and the user restarted their session into that binary.
Process executable identity was checked against `target/release/nickel`. Initial snapshots showed
approximately 129 MiB PSS / 161 MiB RSS with two monitors, and 104 MiB PSS / 136 MiB RSS after the
user unplugged the DisplayLink monitor and restarted. These are Nickel process snapshots, not
whole-session totals, sustained workload measurements, or controlled attribution to DisplayLink.
The remaining display's resolution was not verified.

Two follow-ups remain open in [the regression notes](audit-0209-0219-regressions.md): the user reports
the on-screen keyboard is unavailable, and terminal typing transiently emitted repeated `@`
characters after reconnecting the second monitor and Alt-Tabbing. Neither cause is confirmed.

## Automated evidence

Implementation commits run through `61f58f3`; the integration bookkeeping commit containing this
record also adds backward-compatible presentation diagnostics and updates the source, authority,
and cache/lifecycle inventories. Release workload details are recorded in
[software raster evidence](software-raster-audit-evidence.md),
[presentation memory validation](presentation-memory-validation.md),
[Codex projection memory](../crates/nickel-codex-ui/MEMORY.md), and
[controller delivery](../crates/nickel-codex/DELIVERY.md).

Integrated Linux checks on 2026-09-07:

- `CARGO_BUILD_JOBS=4 cargo test --workspace --quiet`: passed, 1,900 tests passed and 26 ignored.
  Ignored native, environment-specific, and opt-in measurement tests are not implied to have run.
- `cargo fmt --all --check`: passed.
- `CARGO_BUILD_JOBS=4 cargo clippy --workspace --all-targets --all-features -- -D warnings`: passed.
- `CARGO_BUILD_JOBS=4 cargo run -p nickel-ui-workbench -- validate`: passed; 28 fixtures, 44 cache
  records, 44 lifecycle records, 22 consumers, and 22 live acceptance records. Validation checks the
  ledger structure and consistency, not that pending live acceptance has been performed.

Focused renderer import/damage, lazy offscreen selection, sidebar observation, and bounded delivery
regressions are included in the workspace pass. Opt-in release workloads were run separately as
recorded in the evidence documents above; their counters do not substitute for live RSS/VRAM.

The routine cache validator is distinct from its final-completion gate. The ledger deliberately
keeps unmeasured derived caches at `pending_measure`; synthetic validator tests exercise admitted
statuses without changing that ledger or claiming native acceptance.

Memory reports must distinguish payload length, retained allocation capacity, temporary peaks, and
opaque dependency/driver storage. Process RSS and GPU memory measurements are separate evidence;
passing a configured budget test does not measure either one.

Baseline at `077183c`: formatting and strict all-target/all-feature workspace Clippy passed.
Workspace tests reached the declarative-authority gate and failed because `internal_ui.rs` had 64
display-list references against the admitted 63. The extra baseline reference was the existing
pixel-alignment regression. The integrated declaration is reviewed against renderer regression
fixtures; it does not authorize a new application-owned display list or hit-test system.

The source inventory also accounts for four reviewed modules: shared bounded controller delivery,
Codex projection accounting, the bounded sidebar controller, and a test-only Smithay memory-import
renderer. The last module supplies instrumentation missing from Smithay's dummy renderer; it does
not introduce another production renderer.

The Windows GNU cross-check reached the shared UI, Codex UI, and file crates but the shell
failed in the unchanged Windows adapter: `crates/nickel/src/platform/windows.rs` does not match
`HotkeyAction::CancelSwitch`. This pre-existing shell build blocker is outside these eleven
specifications. No Windows runtime acceptance is claimed.

## Interactive acceptance

| Specs | Exercise | Expected behavior |
| --- | --- | --- |
| 0209–0211 | Stream a long response and verbose command output; scroll, select, and copy while streaming. | Input remains responsive, selection/copy matches displayed text, any retention limit is visible, and completion remains truthful. |
| 0209–0211 | Switch conversations, cancel a turn, and handle an approval while output is arriving. | Lifecycle events remain ordered and actionable; stale generations cannot update the new conversation. |
| 0212, 0217 | Render plain and styled prose, Unicode, emoji, colored text, and strikethrough at normal and fractional DPI. | Text, clipping, alpha, and decorations remain correct with bounded retained glyph/raster data. |
| 0213 | Exercise explicit software presentation with small updates, then resize and hide/show. | Partial changes repaint correctly without stale pixels; unchanged frames avoid conversion and buffer creation. |
| 0213 | Exercise a supported GPU/fallback/recovery sequence using the existing controlled harness. | Each transition presents a complete frame and retires obsolete resources. |
| 0214, 0218 | Hover the panel, type in Launcher, open a task menu, and change a real task/tray state. | The correct surface changes; unrelated desktops do not rebuild their shell scenes. |
| 0215 | Show the same wallpaper/icon source on different-DPI outputs, including a high-density variant. | Sampling remains sharp/correct and compatible sources share storage; distinct source pixels remain distinct. |
| 0216 | Repeatedly open and hide the Codex project menu after rendering text. | Private text scratch retires while hidden; project/controller state survives and reopening shows current content. |
| 0218 | Run Nickel Settings/File with builtin icon fallback and move the pointer over the panel. | Icons remain stable and warm updates do not repeatedly decode embedded PNGs. |
| 0219 | Put different windows on two outputs; alternate panel interactions with per-output filtering on and off. | Each panel renders and activates the correct windows before and after input on the other output. |
| 0219 | Change output scale, including a same-physical-size transition where supported. | Software content repaints at the new scale without old-scale pixels. |
| 0219 | Expand/collapse folders, modify children externally, and visit a very large directory. | Listings refresh or report their bounded partial/error state; collapsed results do not reappear from late work. |

## Platform and rollout record

- [x] Integrated automated checks and available synthetic release evidence recorded; remaining live measurements are explicitly pending.
- [ ] Linux native GPU session tested with the user.
- [ ] Linux nested/controlled software presentation coverage recorded.
- [ ] Multiple outputs and fractional DPI checked.
- [x] Windows runtime not exercised; cross-build blocker recorded above.
- [ ] Repeated workload memory returns to the documented bounded steady state.

No test above requires restarting the user's display manager as a routine verification step.
Build, installation, and session replacement are separate from the source implementation record.
