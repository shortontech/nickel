# Cargo Workspace Layout

The workspace members are declared in the root [`Cargo.toml`](../Cargo.toml),
which is the authoritative list. `crates/nickel` is the default member.

## Applications and shell

- `nickel` — desktop shell, plus the Linux compositor and session host.
- `nickel-file` — file browser and file manager.
- `nickel-markdown-ui` — standalone Markdown viewer.
- `nickel-terminal-ui` — terminal application UI.
- `nickel-codex-ui` — standalone Codex chat application.
- `twinkle-workbench` — independent native component gallery and shared fixtures.
- `nickel-workbench` — Nickel consumer fixtures, shell providers and acceptance tooling.

## Shared components

- `nickel-core` — platform-neutral shell state and behavior.
- `nickel-ui-host` — Nickel-owned adapters to the Twinkle engine.
- `nickel-jsx-host` — Nickel package composition and provider lifecycle integration.
- `twinkle` — declarative UI, layout, state, and presentation.
- `nickel-platform` — native Windows and Linux adapters.
- `twinkle-input` — input handling.
- `twinkle-render-assets` — rendering assets.
- `twinkle-jsx-runtime` — JSX/TSX execution and hooks.
- `twinkle-presentation` — native tree/CSS presentation; optional `jsx` executor conveniences.
- `twinkle-protocol` — executor-neutral patches, scheduling types and host-provided surface bounds.
- `twinkle-theme` — portable palette types and perceptual color math.
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
- `twinkle-testkit` — UI testing helpers.
- `twinkle-macros` — declarative UI macros.

The Twinkle crates are being prepared for extraction into their own workspace. They
currently retain Nickel dependencies; the local rename does not establish independence.
The active extraction specifications are `specs/0273` through `specs/0276`.
