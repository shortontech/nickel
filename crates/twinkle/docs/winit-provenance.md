# Native window dependency

Twinkle currently requires the vendored winit 0.30.13 fork, whose upstream repository is `https://github.com/rust-windowing/winit`. Its Apache-2.0 license is retained in `vendor/winit/LICENSE`. The original manifest and upstream metadata are retained alongside the source.

The workspace declares this fork as a direct path dependency and excludes `vendor/winit` from workspace membership, so workspace tests/features do not implicitly include the fork’s development targets. A consumer cannot rely on a `[patch.crates-io]` declaration in Twinkle's workspace: Cargo applies patches from the consuming workspace root. Native file dragging uses the fork's `WindowExtWayland::start_file_drag`, `WindowEvent::FileDropActionChanged`, and `FileDropAction`; upstream 0.30.13 lacks these APIs.

Relevant Nickel history includes `85ec42bc` (start native Wayland file drags), `da53b778` (receive negotiated native file drops), `e4c3fb77` (native input work), and `3afb751a` (Windows shell input). Extraction must preserve the complete relevant vendor history, including upstream source and licenses, rather than importing only those four commits or silently substituting crates.io winit.

Twinkle's standalone workspace must carry this fork. Downstream Nickel must resolve its direct winit dependency and Twinkle's dependency to the same Cargo source/revision so window/event types retain one identity. This fork does not require importing Smithay or PipeWire into Twinkle; those remain Nickel compositor dependencies.

The renamed-dependency fixture is a separate Cargo workspace, so its compilation verifies that the engine's window dependency resolves without Nickel's root patch configuration. It also verifies the `ui!` and `id!` macros when the Cargo dependency is named `lights`.
