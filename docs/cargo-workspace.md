# Cargo Workspace Layout

The workspace members are declared in the root [`Cargo.toml`](../Cargo.toml),
which is the authoritative list. `crates/nickel` is the default member.

## Applications and shell

- `nickel` — desktop shell, plus the Linux compositor and session host.
- `nickel-settings` — Nickel Plating settings application.
- `nickel-file` — file browser and file manager.
- `nickel-markdown-ui` — standalone Markdown viewer.
- `nickel-terminal-ui` — terminal application UI.
- `nickel-codex-ui` — standalone Codex chat application.
- `nickel-ui-workbench` — UI workbench.

## Shared components

- `nickel-core` — platform-neutral shell state and behavior.
- `nickel-ui` — declarative UI, layout, state, and presentation.
- `nickel-platform` — native Windows and Linux adapters.
- `nickel-input` — input handling.
- `nickel-render-assets` — rendering assets.
- `nickel-storage` — persistent storage.
- `nickel-logging` — native logging.
- `nickel-session-protocol` — session communication types.
- `nickel-remote-control` — remote control protocol and behavior.
- `nickel-mcp-client` — MCP client integration.
- `nickel-gaze` — gaze input support.
- `nickel-i18n` — internationalization.
- `nickel-markdown` — Markdown parsing and presentation.
- `nickel-terminal` — terminal behavior.
- `nickel-codex` — Codex backend integration.

## Windows UWP integration and probes

- `nickel-uwu` — Universal Windows Usher library and frame diagnostics.
- `nickel-windows-app-probe` — packaged-app activation probe.
- `nickel-windows-uwp-target` — packaged test target.

## Development tooling

- `nickel-build-support` — shared build support.
- `nickel-codex-fixture` — offline Codex protocol fixtures and replay.
- `nickel-i18n-lint` — internationalization checks.
- `nickel-ui-testkit` — UI testing helpers.
- `ui-declarative-macros` — declarative UI macros.
