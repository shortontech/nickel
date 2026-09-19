# Codex UI 0241–0247 acceptance record

This separates automated and nested evidence from native user-session acceptance. The active
specifications remain in `specs/` until their own completion gates are met. No observation below
claims that either native platform has passed.

## Verified in the current Linux checkout (2026-09-18)

- `cargo clippy -p nickel-codex-ui -p nickel-codex -p nickel-ui --all-targets --all-features -- -D warnings`: passed.
- `cargo test -p nickel-codex-ui -p nickel-codex -p nickel-ui --all-targets -q`: passed, including the asset-provenance and declarative-authority audits. The Codex UI unit suite reported 165 passed, 3 ignored; the UI unit suite reported 414 passed, 2 ignored.
- `cargo check --target x86_64-pc-windows-gnu -p nickel -q`: passed with warnings; this is a cross-target compile check, not Windows runtime evidence.
- Focused shell tests for Codex approval notification revision/overflow, simultaneous Codex and MCP requests, and persistent MCP notifications passed.
- `cargo test -p nickel --lib -q`: 1,021 passed, 28 ignored, one failure in the unrelated `session::state::remote_settings::tests::prepared_settings_reject_changed_configuration_and_expiry_without_overwriting`. That test passed when run alone. The full shell result is **not** green; the cause of its full-suite-only failure has not been established.
- `cargo build --release -p nickel -q`: passed. The prepared `target/release/nickel` is 90,458,296 bytes with `cksum` `3955444581 90458296`.
- After the repository owner restarted SDDM, Nickel launched on tty7 as PID 1165536. `/proc/1165536/exe` and the prepared release binary both had inode 20897158; the Wayland and session sockets were present. This establishes that the new binary started, not that its UI or input passed native acceptance.
- A bordered nested Smithay/Winit session was run at a 960×540 requested output; the host window was 1440×810 physical pixels and the embedded Codex chat occupied 1120×704 logical units. The launcher opened a recent project, the compact chat rendered, and expanded run settings remained inside the bordered window. Local screenshots (ignored build artifacts): `target/acceptance/codex-final-chat-1120x704.png` and `target/acceptance/codex-final-chat-settings-1120x704.png`. The nested session was stopped after inspection.

## Native Linux observations (2026-09-18)

- The owner opened a Codex project and typed successfully in the live shell. A blue fill still appeared behind text in the editor, as in other Nickel text fields. The transparent-editor fallback focus fill has since been removed in source with a regression test; the live binary has not been rebuilt or restarted with this fix.
- A file-listing result appeared in chat. Inspection of the matching Codex session event showed the list in an assistant `AgentMessage`, not an automatically displayed tool result, so no suppression change was made.
- QR scanning was rejected by the phone, but the manual pairing code succeeded. The phone access panel showed a connected host and one paired iPhone. The owner supplied a zeroed, same-length decode of Nickel's QR and confirmed it contained only the long opaque code. Nickel now constructs `https://chatgpt.com/codex/pair?pairing_code=…` before QR encoding, matching the pairing URL shape observed by the owner. The new QR has automated coverage but has not yet been tested in the live shell; no live code was captured in this record.
- The phone panel repeated connection and pairing state, exposed an installation ID, and used excessive explanatory text and padding. Its source view has been compacted, but the change is not yet in the live binary and has not had native visual acceptance.
- After SDDM restart, Codex phone access reported `Errored` while the phone could still show a connection. Codex's local structured logs showed the second Nickel-owned app-server repeatedly receiving HTTP 409 `Remote app server already online`; the project menu and chat had each spawned an app-server under the same installation. Source now makes only the project-menu app-server own remote access and routes chat phone-management calls through it. The fix passed focused tests and lint but has not yet had native acceptance.
- The phone panel's only close action was below its main content, so the owner could not find a way back without closing the window. Source now puts Back at the top of phone and remote-host panels and gives Escape priority over interrupting an active turn. The command picker also clipped long labels because of a forced 220-unit width; source now uses content sizing. Its disabled slash-command entries were preserved. The owner expects those commands to work; implementing their Codex-backed behavior remains an open integration task, not a UI-label issue.

## Still required before archiving 0241–0247

- Linux native acceptance in the main Nickel session with the prepared release binary: opening and switching projects/threads, compact and maximized layout, run settings, streaming and long content, IME/selection across resize, approval arrival while typing, notification review/decision/expiry, and focus restoration. Process launch is verified; none of these visual/input observations has yet been recorded.
- Windows native visual and input acceptance at its actual display scale, including notification action routing and focus behavior. The Windows host is reachable, but its `C:\projects\nickel` checkout was observed at `b311992e` with modified `crates/nickel/Cargo.toml` and `crates/nickel/src/lib.rs` plus an untracked session log. It was not changed or used for acceptance.
- Record actual usable 200%-scale client geometry and separate Linux/Windows results for the 0246 and 0247 matrices. The 960×540 logical fixture and a cross-compile do not substitute for those native observations.
- Resolve or explicitly classify the full-shell remote-settings test failure before claiming an entirely green workspace. The full workspace run also encountered the running shell's fixed localhost test port; that conflict is not a Codex UI failure but does not count as a passing workspace run.
