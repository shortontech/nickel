# Shared Shell Graphics

## Goal

Remove shell startup and first-use latency caused by creating an independent wgpu stack for every Nickel surface.

## Design

- `nickel-ui` creates one shared wgpu instance, adapter, device, and queue.
- The launcher, panel, and context menu retain independent surfaces, surface configurations, viewports, atlases, and renderers.
- All shell surfaces use the shared device and queue for resource creation, uploads, command submission, and presentation.
- Surface initialization accepts the shared graphics context instead of requesting another adapter/device.
- The context menu is rendered once during initialization so its first visible frame does not trigger deferred pipeline and glyph preparation.
- Renderer-specific buffers and atlases remain isolated until sharing them has a demonstrated memory or latency benefit.

## Constraints

- The graphics abstraction remains platform-neutral and contains no Wayland, Win32, or session transport logic.
- Existing launcher, panel, and context-menu behavior must remain unchanged.
- Device loss or incompatible-surface recovery is deferred; all current shell windows are created on the same event loop and display backend.

## Verification

- Source contains one adapter and device request for `nickel-ui`.
- Context-menu first show is immediate in the live nested session.
- Launcher focus, maximize, task icons, and close-menu action still work.
- `cargo test --workspace` and workspace Clippy pass.
