# Nickel

Nickel is an experimental cross-platform desktop shell written entirely in Rust.

Its goal is simple: provide the same desktop interface across Windows, Linux, and eventually macOS, while adapting to each operating system through narrow native integrations.

Nickel is not a theme, a launcher skin, or a web-based desktop. It is a native shell project built around a shared application model, deterministic search and ranking, GPU-rendered UI, and platform-specific adapters.

## Status

Nickel is under active early development.

Currently implemented:

- Installed-application discovery
- Application icons
- Fuzzy application search
- Keyboard navigation
- Scrollable search results
- GTK-style panel components
- Persistent user preferences
- Application pinning

The first usable milestone is a fast launcher and panel that can serve as a daily-use shell surface before Nickel takes responsibility for window management, notifications, file browsing, or full desktop-session ownership.

## Goals

Nickel aims to provide:

- One consistent shell interface across operating systems
- Fast application discovery and search
- Keyboard- and controller-friendly navigation
- Pinned and recent applications
- Running-window management
- A native file browser
- Notifications and status items
- Consistent settings and shell behavior
- A shared rendering and interaction model
- Explicit platform capability detection

The visible product should remain recognizably Nickel everywhere. Platform adapters are responsible for translating native operating-system behavior into Nickel's common domain model.

## Non-Goals

Nickel does not attempt to:

- Emulate Windows Explorer, GNOME Shell, or KDE Plasma exactly
- Hide meaningful operating-system limitations
- Depend on Electron or a browser runtime
- Reimplement every system settings application
- Become a Linux distribution
- Require identical native APIs across platforms

Platform-specific behavior is expected. Platform-specific product behavior should be minimized.

## Architecture

Nickel is organized as a Cargo workspace with small, focused crates:

```text
crates/
├── nickel-core/       Platform-neutral state and domain logic
├── nickel-search/     Indexing, fuzzy matching, and result ranking
├── nickel-ui/         winit event handling, wgpu rendering, and widgets
└── nickel-platform/   Windows, Linux, and macOS adapters

assets/                Fonts, shaders, icons, and test fixtures
tests/                 Workspace-level integration and contract tests
specs/                 Active and completed design specifications
```

The central rule is that search, ranking, navigation, pinning, and task-switcher policy remain independent of native window APIs.

A platform adapter may discover applications differently, but the shared search engine should receive the same normalized model:

```rust
pub struct Application {
    pub id: ApplicationId,
    pub name: String,
    pub executable: PathBuf,
    pub arguments: Vec<String>,
    pub icon: IconSource,
    pub categories: Vec<String>,
}
```

The same principle will apply to windows, notifications, displays, status items, and file-system operations.

## Platform Strategy

### Windows

Nickel can initially run as a normal application and launcher.

Later milestones may allow Nickel to become the login shell while continuing to delegate folder browsing to Windows Explorer until Nickel's own file browser is ready.

Expected Windows integrations include:

- Start Menu and packaged-app discovery
- Native icon extraction
- Window enumeration and activation
- Shell startup configuration
- Notification-area integration
- File associations and launch handling

### Linux

Nickel will support Linux incrementally.

The initial version can run inside an existing desktop session as a launcher and panel. A later version may run as part of, or become, a Wayland compositor session.

Expected Linux integrations include:

- `.desktop` application discovery
- Freedesktop icon themes
- GTK and KDE theme settings
- Wayland and X11 window integration
- StatusNotifierItem support
- D-Bus notifications
- XDG desktop portals

When Nickel becomes responsible for a Wayland session, the compositor and visible shell should remain separate processes so the shell UI can reload without disconnecting client applications.

### macOS

macOS support is architectural rather than immediate.

A future adapter may integrate with Launch Services, application bundles, system icons, and window APIs. Nickel may replace much of the visible desktop experience, but macOS does not provide the same supported shell-replacement model as Windows.

## Search

Search is Nickel's first major subsystem because it is portable, measurable, and deterministic.

Ranking should prefer, in order:

1. Exact application-name matches
2. Prefix matches
3. Word-prefix matches
4. Strong fuzzy matches
5. Weaker fuzzy matches

Usage signals such as pinned state, recent launches, and launch frequency may adjust ranking without making results unpredictable.

Important search behavior includes:

- Stable result ordering
- Duplicate application collapse
- Predictable missing-icon fallbacks
- Keyboard selection that remains visible while scrolling
- No native platform calls in the ranking path
- Identical output for identical normalized input

## Performance

Nickel should remain lightweight enough to run continuously.

The largest current memory and rendering risk is icon handling. Icons should therefore be:

- Decoded outside the render loop
- Loaded at an appropriate display size
- Uploaded to the GPU once
- Reused through stable texture handles
- Stored in a bounded LRU cache
- Rendered only for visible or near-visible rows

Search result lists should be virtualized. Scrolling through hundreds of applications must not cause every result to be laid out, decoded, uploaded, or rendered on every frame.

Planned performance checks include:

- Large synthetic application indexes
- Bounded allocations per query
- Stable idle memory usage
- No growth after repeated search open/close cycles
- Smooth scrolling with cold and warm icon caches
- Predictable startup and indexing time

## User Preferences

User preferences are versioned and intentionally simple.

Initial persisted state includes pinned applications:

```rust
pub struct UserPreferences {
    pub schema_version: u32,
    pub pinned_apps: Vec<ApplicationId>,
}
```

Nickel stores stable logical application IDs rather than raw executable paths whenever possible.

If a pinned application is temporarily unavailable, Nickel should retain the unresolved pin so it can recover automatically when the application returns.

## Building

Nickel uses stable Rust.

From the workspace root:

```bash
cargo run
cargo build --workspace
cargo test --workspace
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

### Linux compositor backends

The default `nickel-session` build uses the nested winit backend and can run
inside an existing desktop:

```bash
cargo run -p nickel-session -- --backend winit --command target/debug/nickel-ui
```

The direct backend requires the standard DRM, GBM, libinput, udev, and libseat
development packages. On Ubuntu these are `libdrm-dev`, `libgbm-dev`,
`libinput-dev`, `libudev-dev`, `libseat-dev`, and `libegl1-mesa-dev`.

Build the direct backend without linking the nested window-system backend:

```bash
cargo build -p nickel-ui
cargo build -p nickel-session --no-default-features --features backend-udev
```

Run it from a text VT owned by the logged-in user:

```bash
RUST_LOG=info target/debug/nickel-session \
  --backend udev --command target/debug/nickel-ui
```

`NICKEL_DRM_DEVICE=/dev/dri/cardN` overrides automatic primary-GPU selection.
Do not launch the direct backend from inside another compositor that already
owns the same DRM device.

## Testing

Unit tests live beside their modules. Cross-crate and platform-contract tests live under `tests/`.

Core logic should be tested with synthetic:

- Applications
- Windows
- Notifications
- Clocks
- Controller events
- Theme data
- Platform capabilities

Platform adapter tests should be gated by target operating system.

On Linux, a deterministic StatusNotifierItem is available for live panel testing:

```bash
cargo run -p nickel-ui --bin nickel-test-tray
```

Its blue-and-yellow checkerboard icon logs an activation message when clicked and disappears when the process stops.

Manual platform coverage should record:

- Focus behavior
- DPI and fractional scaling
- Multiple monitors
- Permission failures
- Missing applications
- Broken icons
- Shell restart and recovery behavior

Behavior-oriented test names are preferred:

```text
exact_match_ranks_above_prefix_match
recent_window_ranks_above_unused_match
missing_icon_uses_deterministic_fallback
selected_result_remains_visible_while_scrolling
unavailable_pin_is_not_deleted
```

## Roadmap

### Milestone 1 — Launcher

- [x] Application discovery
- [x] Native application icons
- [x] Fuzzy search
- [x] Keyboard navigation
- [x] Scrollable results
- [x] Panel components
- [ ] Persistent pinning
- [ ] Pin ordering
- [ ] Launch history
- [ ] Icon-cache optimization
- [ ] Search-result virtualization

### Milestone 2 — Panel

- [ ] Persistent application panel
- [ ] Pinned application launching
- [ ] Running-window association
- [ ] Window activation
- [ ] Clock
- [ ] Session controls
- [ ] Multiple-monitor behavior

### Milestone 3 — Shell Integration

- [ ] Windows login-shell mode
- [ ] Linux desktop-session mode
- [ ] Reloadable shell process
- [ ] Notification integration
- [ ] Status-item integration
- [ ] Theme normalization
- [ ] Controller navigation

### Milestone 4 — Files

- [ ] Native file browser
- [ ] Common file-operation model
- [ ] Platform file metadata
- [ ] Thumbnails with bounded caches
- [ ] Search within the current directory
- [ ] Default file-handler integration

### Milestone 5 — Wayland Session

- [ ] Persistent compositor process
- [ ] Reloadable Nickel shell
- [ ] Window placement and task switching
- [ ] Server-side decoration support
- [ ] Client-side decoration compatibility
- [ ] XWayland support
- [ ] Lock, logout, and recovery surfaces

## Project Principles

### Platform-native capabilities, platform-independent behavior

Nickel should use the strongest native integration available on each operating system while presenting one coherent product model.

### Normalize at the boundary

Native application entries, icons, windows, notifications, and file metadata should be converted into platform-neutral structures before reaching core logic.

### Keep the compositor boring

When Nickel owns a Wayland session, the compositor should be small, stable, and persistent. Rapidly changing UI belongs in a separate reloadable shell process.

### Bound every cache

Icon, thumbnail, search, notification, and history caches must have explicit item or byte limits.

### Make behavior testable

Ranking, navigation, selection, pinning, and shell policy must be deterministic and testable without a live desktop session.

### Remain native

Nickel application code and shipped tooling remain Rust. JavaScript, TypeScript, Lua, Go, C, and C++ application components are intentionally excluded.

## Contributing

Nickel is currently an experimental personal project, but focused issues and pull requests are welcome.

Before submitting a change:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

Use concise imperative commit subjects, such as:

```text
Add fuzzy ranking pipeline
Persist pinned application order
Virtualize search result rows
Bound the GPU icon cache
```

Pull requests should explain user-visible behavior, list tested platforms, reference relevant specifications or issues, and include screenshots or recordings for visual changes.

## License

Nickel is dual-licensed under the [MIT License](LICENSE-MIT) or the
[Apache License, Version 2.0](LICENSE-APACHE), at your option.
