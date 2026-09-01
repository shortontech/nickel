use std::{
    alloc::{GlobalAlloc, Layout, System},
    env,
    error::Error,
    fmt, fs,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

use nickel_markdown::{MarkdownDocument, MarkdownPalette, markdown_view};
use nickel_ui::{
    ActionKind, AnyView, Application, Button, ButtonLabel, Collection, CollectionPresentation,
    CollectionState, Column, ComponentBuilderExt, Container, Dropdown, Grid, Image, ImageFit,
    Insets, Menu, MenuBar, MenuItem, RadioButton, ReadingDirection, ResponsiveNavigation,
    ResponsiveNavigationDestination, Row, SdlComponentRenderer, SemanticRole, SemanticTheme,
    Slider, Surface, SurfaceRole, Switch, Text, TextField, UiHost, VerticalScroll, ViewContext,
};
use nickel_ui_testkit::{
    AccessibilityPreset, ActivationVia, ErasedFixtureSession, Fixture, FixtureAsset,
    FixtureDirection, FixtureMetadata, FixtureProvider, FixtureRegistry, FixtureRegistryEntry,
    FixtureSource, FixtureTheme, FixtureVariant, LocalePreset, ReachabilityPolicy, ScalePreset,
    Selector, SimulatedEffectKind, TraceDocument, ViewportPreset, audit_registry_reachability,
    open, render_host, validate_host,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};

mod fixture_inventory;

struct CountingAllocator;

static ALLOCATION_OPERATIONS: AtomicU64 = AtomicU64::new(0);

// SAFETY: Every operation delegates to `System` with the original pointer and
// layout. The counter is observational and does not alter allocator behavior.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATION_OPERATIONS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: Delegating the caller-provided layout to the system allocator.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        ALLOCATION_OPERATIONS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: Delegating the caller-provided layout to the system allocator.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: Delegating the allocation's original pointer and layout.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCATION_OPERATIONS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: Delegating the allocation's original pointer/layout and requested size.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL_ALLOCATOR: CountingAllocator = CountingAllocator;

fn allocation_operations() -> u64 {
    ALLOCATION_OPERATIONS.load(Ordering::Relaxed)
}

const COUNTER_ID: &str = "core.counter";
const MARKDOWN_ID: &str = "markdown.core";
const MARKDOWN_LINK: &str = "https://example.com/guide";
const PRIMITIVES_ID: &str = "shared.primitives";
const COLLECTION_STATES_ID: &str = "shared.collection-states";
const SETTINGS_WIDE_ID: &str = "settings.wide";
const SETTINGS_NARROW_RTL_ID: &str = "settings.narrow-rtl";
const LAUNCHER_ID: &str = "launcher.dashboard";
const CODEX_ID: &str = "codex.chat";
const MENU_ID: &str = "shared.menus";
#[cfg(any(
    not(feature = "file-provider"),
    not(feature = "markdown-viewer-provider"),
    not(feature = "gaze-provider"),
    not(feature = "shell-provider")
))]
use nickel_ui_testkit::ExternalFixtureProvider;

#[cfg(not(feature = "file-provider"))]
const FILE_ID: &str = "file.browser";
#[cfg(not(feature = "file-provider"))]
const FILE_VARIANTS: &[FixtureVariant] = &[
    FixtureVariant {
        id: "wide",
        title: "Wide",
        viewport: ViewportPreset {
            id: "wide",
            width: 960,
            height: 640,
        },
        theme: FixtureTheme::Dark,
        locale: nickel_ui_testkit::DEFAULT_LOCALE,
        scale: nickel_ui_testkit::DEFAULT_SCALE,
        controller_family: nickel_ui::ControllerFamily::Generic,
        accessibility: nickel_ui_testkit::DEFAULT_ACCESSIBILITY,
    },
    FixtureVariant {
        id: "narrow-200",
        title: "Narrow 200%",
        viewport: ViewportPreset {
            id: "narrow",
            width: 540,
            height: 420,
        },
        theme: FixtureTheme::Dark,
        locale: nickel_ui_testkit::DEFAULT_LOCALE,
        scale: ScalePreset {
            id: "2x",
            factor: 2.0,
        },
        controller_family: nickel_ui::ControllerFamily::Generic,
        accessibility: nickel_ui_testkit::DEFAULT_ACCESSIBILITY,
    },
];
#[cfg(not(feature = "file-provider"))]
static FILE_METADATA: FixtureMetadata = FixtureMetadata {
    id: FILE_ID,
    title: "Nickel File",
    description: "Production Nickel File browser surface",
    tags: &["file", "browser", "collection", "context-menu"],
    source: FixtureSource {
        crate_name: "nickel-file",
        file: "src/main.rs",
        line: 107,
    },
    variants: FILE_VARIANTS,
    assets: &[],
    simulated_effects: &[],
};
#[cfg(not(feature = "file-provider"))]
const FILE_PROVIDER: ExternalFixtureProvider = ExternalFixtureProvider {
    protocol_version: 1,
    cargo_package: "nickel-file",
    workbench_feature: "file-provider",
};
#[cfg(not(feature = "markdown-viewer-provider"))]
const MARKDOWN_VIEWER_VARIANTS: &[FixtureVariant] = &[
    external_variant("loaded", "Loaded document", "markdown-viewer", 960, 720),
    external_variant("loading", "Loading document", "markdown-viewer", 960, 720),
    external_variant("error", "Load error", "markdown-viewer", 960, 720),
    external_variant(
        "selection",
        "Selectable document",
        "markdown-viewer",
        620,
        720,
    ),
];
#[cfg(not(feature = "markdown-viewer-provider"))]
static MARKDOWN_VIEWER_METADATA: FixtureMetadata = FixtureMetadata {
    id: "markdown.viewer",
    title: "Nickel Markdown Viewer",
    description: "Production Markdown viewer with deterministic load and selection states",
    tags: &["markdown", "viewer", "document", "selection", "error"],
    source: FixtureSource {
        crate_name: "nickel-markdown-ui",
        file: "src/lib.rs",
        line: 467,
    },
    variants: MARKDOWN_VIEWER_VARIANTS,
    assets: &[],
    simulated_effects: &[],
};
#[cfg(not(feature = "markdown-viewer-provider"))]
const MARKDOWN_VIEWER_PROVIDER: ExternalFixtureProvider = ExternalFixtureProvider {
    protocol_version: 1,
    cargo_package: "nickel-markdown-ui",
    workbench_feature: "markdown-viewer-provider",
};

#[cfg(not(feature = "gaze-provider"))]
const GAZE_VARIANTS: &[FixtureVariant] = &[
    external_variant("disconnected", "Disconnected", "gaze-grid", 1120, 720),
    external_variant("connected", "Connected", "gaze-grid", 1120, 720),
    external_variant("empty", "No face detected", "gaze-grid", 1120, 720),
    external_variant("populated", "Tracking gaze", "gaze-grid", 1120, 720),
];
#[cfg(not(feature = "gaze-provider"))]
static GAZE_METADATA: FixtureMetadata = FixtureMetadata {
    id: "gaze.grid",
    title: "Gaze calibration grid",
    description: "Production gaze-grid UI with deterministic, simulated tracking states.",
    tags: &["gaze", "grid", "accessibility", "controller"],
    source: FixtureSource {
        crate_name: "nickel-gaze",
        file: "src/bin/nickel-gaze-grid.rs",
        line: 156,
    },
    variants: GAZE_VARIANTS,
    assets: &[],
    simulated_effects: &[],
};
#[cfg(not(feature = "gaze-provider"))]
const GAZE_PROVIDER: ExternalFixtureProvider = ExternalFixtureProvider {
    protocol_version: 1,
    cargo_package: "nickel-gaze",
    workbench_feature: "gaze-provider",
};

#[cfg(not(feature = "shell-provider"))]
macro_rules! shell_external_fixture {
    ($metadata:ident, $provider:ident, $id:literal, $title:literal, $description:literal, $variants:expr) => {
        static $metadata: FixtureMetadata = FixtureMetadata {
            id: $id,
            title: $title,
            description: $description,
            tags: &["shell", "production", "external-provider"],
            source: FixtureSource {
                crate_name: "nickel-shell",
                file: "src/workbench_fixtures.rs",
                line: 1,
            },
            variants: $variants,
            assets: &[],
            simulated_effects: &[],
        };
        const $provider: ExternalFixtureProvider = ExternalFixtureProvider {
            protocol_version: 1,
            cargo_package: "nickel-shell",
            workbench_feature: "shell-provider",
        };
    };
}

#[cfg(not(feature = "shell-provider"))]
const SHELL_RUNTIME_VARIANTS: &[FixtureVariant] = &[
    external_variant("multi-output", "Multi-output", "shell-runtime", 960, 540),
    external_variant(
        "surface-lifecycle",
        "Surface lifecycle",
        "shell-runtime",
        800,
        450,
    ),
];
#[cfg(not(feature = "shell-provider"))]
const SHELL_DESKTOP_VARIANTS: &[FixtureVariant] = &[
    external_variant("solid", "Solid background", "shell-desktop", 960, 540),
    external_variant("wallpaper", "Wallpaper", "shell-desktop", 960, 540),
];
#[cfg(not(feature = "shell-provider"))]
const SHELL_PANEL_VARIANTS: &[FixtureVariant] = &[
    external_variant("wide", "Wide", "shell-panel", 1200, 56),
    external_variant("narrow", "Narrow", "shell-panel", 640, 56),
    external_variant("fullscreen", "Fullscreen", "shell-panel", 960, 56),
];
#[cfg(not(feature = "shell-provider"))]
const SHELL_NOTIFICATION_VARIANTS: &[FixtureVariant] = &[
    external_variant("no-actions", "No actions", "shell-notification", 420, 180),
    external_variant("actions", "Actions", "shell-notification", 420, 180),
    external_variant("long-body", "Long body", "shell-notification", 420, 240),
];
#[cfg(not(feature = "shell-provider"))]
const SHELL_LOCK_VARIANTS: &[FixtureVariant] = &[
    external_variant("empty", "Empty", "shell-lock", 960, 540),
    external_variant("password", "Password", "shell-lock", 960, 540),
    external_variant("error", "Error", "shell-lock", 960, 540),
];
#[cfg(not(feature = "shell-provider"))]
const SHELL_SCREENSHOT_VARIANTS: &[FixtureVariant] = &[
    external_variant("idle", "Idle", "shell-screenshot", 960, 540),
    external_variant("selecting", "Selecting", "shell-screenshot", 960, 540),
    external_variant("confirmed", "Confirmed", "shell-screenshot", 960, 540),
    external_variant("error", "Error", "shell-screenshot", 960, 540),
];
#[cfg(not(feature = "shell-provider"))]
const SHELL_PREVIEW_VARIANTS: &[FixtureVariant] = &[
    external_variant("empty", "Empty", "shell-window-preview", 300, 214),
    external_variant("one", "One window", "shell-window-preview", 300, 214),
    external_variant("many", "Many windows", "shell-window-preview", 882, 214),
    external_variant(
        "missing-preview",
        "Missing preview",
        "shell-window-preview",
        300,
        214,
    ),
];
#[cfg(not(feature = "shell-provider"))]
const SHELL_CONTROL_VARIANTS: &[FixtureVariant] = &[
    external_variant("available", "Available", "shell-control-center", 380, 650),
    external_variant(
        "unavailable",
        "Unavailable",
        "shell-control-center",
        380,
        650,
    ),
    external_variant(
        "confirmation",
        "Confirmation",
        "shell-control-center",
        380,
        650,
    ),
    external_variant("scroll", "Scroll", "shell-control-center", 380, 420),
];
#[cfg(not(feature = "shell-provider"))]
const SHELL_PROJECT_VARIANTS: &[FixtureVariant] = &[
    external_variant("open", "Open", "shell-codex-project-menu", 920, 680),
    external_variant("search", "Search", "shell-codex-project-menu", 920, 680),
    external_variant("empty", "Empty", "shell-codex-project-menu", 920, 680),
];
#[cfg(not(feature = "shell-provider"))]
const SHELL_SEARCH_VARIANTS: &[FixtureVariant] = &[
    external_variant(
        "empty-query",
        "Empty query",
        "shell-launcher-search",
        920,
        680,
    ),
    external_variant("results", "Results", "shell-launcher-search", 920, 680),
    external_variant(
        "no-results",
        "No results",
        "shell-launcher-search",
        920,
        680,
    ),
    external_variant("scroll", "Scroll", "shell-launcher-search", 920, 680),
];

#[cfg(not(feature = "shell-provider"))]
shell_external_fixture!(
    SHELL_RUNTIME_METADATA,
    SHELL_RUNTIME_PROVIDER,
    "shell.runtime",
    "Shell runtime",
    "Production shell-owned UiHost lifecycle surface",
    SHELL_RUNTIME_VARIANTS
);
#[cfg(not(feature = "shell-provider"))]
shell_external_fixture!(
    SHELL_DESKTOP_METADATA,
    SHELL_DESKTOP_PROVIDER,
    "shell.desktop",
    "Desktop",
    "Production desktop application",
    SHELL_DESKTOP_VARIANTS
);
#[cfg(not(feature = "shell-provider"))]
shell_external_fixture!(
    SHELL_PANEL_METADATA,
    SHELL_PANEL_PROVIDER,
    "shell.panel",
    "Panel",
    "Production panel application",
    SHELL_PANEL_VARIANTS
);
#[cfg(not(feature = "shell-provider"))]
shell_external_fixture!(
    SHELL_NOTIFICATION_METADATA,
    SHELL_NOTIFICATION_PROVIDER,
    "shell.notification",
    "Notification",
    "Production notification application",
    SHELL_NOTIFICATION_VARIANTS
);
#[cfg(not(feature = "shell-provider"))]
shell_external_fixture!(
    SHELL_LOCK_METADATA,
    SHELL_LOCK_PROVIDER,
    "shell.lock",
    "Lock screen",
    "Production lock application",
    SHELL_LOCK_VARIANTS
);
#[cfg(not(feature = "shell-provider"))]
shell_external_fixture!(
    SHELL_SCREENSHOT_METADATA,
    SHELL_SCREENSHOT_PROVIDER,
    "shell.screenshot",
    "Screenshot",
    "Production screenshot application",
    SHELL_SCREENSHOT_VARIANTS
);
#[cfg(not(feature = "shell-provider"))]
shell_external_fixture!(
    SHELL_PREVIEW_METADATA,
    SHELL_PREVIEW_PROVIDER,
    "shell.window-preview",
    "Window preview",
    "Production window preview application",
    SHELL_PREVIEW_VARIANTS
);
#[cfg(not(feature = "shell-provider"))]
shell_external_fixture!(
    SHELL_CONTROL_METADATA,
    SHELL_CONTROL_PROVIDER,
    "shell.control-center",
    "Control Center",
    "Production control center application",
    SHELL_CONTROL_VARIANTS
);
#[cfg(not(feature = "shell-provider"))]
shell_external_fixture!(
    SHELL_PROJECT_METADATA,
    SHELL_PROJECT_PROVIDER,
    "shell.codex-project-menu",
    "Codex project menu",
    "Production launcher project surface",
    SHELL_PROJECT_VARIANTS
);
#[cfg(not(feature = "shell-provider"))]
shell_external_fixture!(
    SHELL_SEARCH_METADATA,
    SHELL_SEARCH_PROVIDER,
    "shell.launcher-search",
    "Launcher search",
    "Production launcher search surface",
    SHELL_SEARCH_VARIANTS
);

#[cfg(any(
    not(feature = "markdown-viewer-provider"),
    not(feature = "gaze-provider"),
    not(feature = "shell-provider")
))]
const fn external_variant(
    id: &'static str,
    title: &'static str,
    viewport_id: &'static str,
    width: u32,
    height: u32,
) -> FixtureVariant {
    FixtureVariant {
        id,
        title,
        viewport: ViewportPreset {
            id: viewport_id,
            width,
            height,
        },
        theme: FixtureTheme::Dark,
        locale: nickel_ui_testkit::DEFAULT_LOCALE,
        scale: nickel_ui_testkit::DEFAULT_SCALE,
        controller_family: nickel_ui::ControllerFamily::Generic,
        accessibility: nickel_ui_testkit::DEFAULT_ACCESSIBILITY,
    }
}
const MARKDOWN_SOURCE: &str = r#"# Markdown core

Representative **prose** with [the guide](https://example.com/guide) and `inline_code()`.

```rust
fn deterministic() -> bool { true }
```

| Surface | Authority |
| :--- | ---: |
| Semantics | Nickel UI |
| Raster | Headless |
"#;
const FEEDBACK_BUDGETS: &str = include_str!("../../../assets/ui-feedback-budgets.toml");
const CACHE_INVENTORY: &str = include_str!("../../../assets/ui-caches.tsv");
const CACHE_LIFECYCLE_MATRIX: &str = include_str!("../../../assets/ui-cache-lifecycle.tsv");
const CONSUMER_INVENTORY: &str = include_str!("../../../assets/ui-consumers.tsv");
const VISUAL_FIXTURES: &str = include_str!("../../../assets/visual-fixtures.toml");
const UI_EVIDENCE: &str = include_str!("../../../assets/evidence/ui-evidence.json");
const NESTED_RUNTIME_EVIDENCE: &str =
    include_str!("../../../assets/evidence/nested-runtime-performance.json");
const UI_REACHABILITY_REPORT: &[u8] =
    include_bytes!("../../../assets/evidence/ui-reachability-report.json");
const SOFTWARE_PRESENTER_CACHE_DIAGNOSTICS: &str =
    "unavailable (software preview does not instantiate a GPU presenter)";

#[derive(Debug, Deserialize)]
struct VisualFixtureManifest {
    reference: Vec<VisualReferenceRecord>,
}

#[derive(Debug, Deserialize)]
struct VisualReferenceRecord {
    id: String,
    path: String,
    sha256: String,
    authorship: String,
    usage_status: String,
    source: String,
}

#[derive(Clone, Debug, Deserialize)]
struct FeedbackBudgets {
    version: u32,
    metadata: BenchmarkMetadata,
    fast: FastBudgets,
    focused: FocusedBudgets,
    pre_commit: PreCommitBudgets,
    full_visual: FullVisualBudgets,
    live: LiveBudgets,
    lifecycle: LifecycleBudgets,
}

#[derive(Clone, Debug, Deserialize)]
struct BenchmarkMetadata {
    build_profile: String,
    toolchain: String,
    cpu: String,
    gpu: String,
    scale: f64,
    fixture: String,
    execution: String,
}

#[derive(Clone, Debug, Deserialize)]
struct FastBudgets {
    incremental_compile_ms: f64,
    selected_unit_test_ms: f64,
}

#[derive(Clone, Debug, Deserialize)]
struct FocusedBudgets {
    semantic_scenario_p95_ms: f64,
    software_render_p95_ms: f64,
    hard_command_ms: f64,
    samples: usize,
    workbench_open_p95_ms: f64,
    warm_frame_p95_ms: f64,
    input_to_visible_p95_ms: f64,
    frame_allocations: usize,
    retained_frame_bytes: usize,
    cache_growth_bytes: usize,
}

#[derive(Clone, Debug, Deserialize)]
struct PreCommitBudgets {
    strict_clippy_ms: f64,
    workspace_test_ms: f64,
}

#[derive(Clone, Debug, Deserialize)]
struct FullVisualBudgets {
    full_matrix_ms: f64,
    shardable: bool,
}

#[derive(Clone, Debug, Deserialize)]
struct LiveBudgets {
    input_to_visible_p95_ms: f64,
    warm_frame_p95_ms: f64,
    samples: usize,
}

#[derive(Clone, Debug, Deserialize)]
struct LifecycleBudgets {
    idle_frames: u64,
    retained_build_scratch_bytes: usize,
}

#[derive(Debug, Deserialize)]
struct NestedRuntimeEvidence {
    environment: NestedRuntimeEnvironment,
    samples: NestedRuntimeSamples,
    summary: NestedRuntimeSummary,
    result: NestedRuntimeResult,
}

#[derive(Debug, Deserialize)]
struct NestedRuntimeEnvironment {
    hardware_claim: bool,
}

#[derive(Debug, Deserialize)]
struct NestedRuntimeSamples {
    warm_present_us: Vec<u64>,
    input_to_visible_us: Vec<u64>,
}

#[derive(Debug, Deserialize)]
struct NestedRuntimeSummary {
    warm_present_p95_us: u64,
    input_to_visible_p95_us: u64,
    retained_presenter_bytes: usize,
    frame_allocations: NestedAllocationEvidence,
}

#[derive(Debug, Deserialize)]
struct NestedAllocationEvidence {
    count: Option<u64>,
    sample_count: usize,
    scope: String,
    unavailable_reason: Option<String>,
    measurement: Option<String>,
}

#[derive(Debug, Deserialize)]
struct NestedRuntimeResult {
    frame_allocations: bool,
}

fn validate_nested_allocation_evidence(
    evidence: &NestedAllocationEvidence,
) -> Result<(), Box<dyn Error>> {
    match evidence.scope.as_str() {
        "unavailable"
            if evidence.count.is_none()
                && evidence.sample_count == 0
                && evidence
                    .unavailable_reason
                    .as_deref()
                    .is_some_and(|reason| !reason.trim().is_empty()) =>
        {
            Ok(())
        }
        "process" | "thread"
            if evidence.count.is_some()
                && evidence.sample_count > 0
                && evidence.unavailable_reason.is_none() =>
        {
            Ok(())
        }
        "presenter"
            if evidence.count.is_some()
                && evidence.sample_count > 0
                && evidence.unavailable_reason.is_none()
                && evidence
                    .measurement
                    .as_deref()
                    .is_some_and(|measurement| !measurement.trim().is_empty()) =>
        {
            Ok(())
        }
        _ => Err(Box::new(UsageError(
            "nested allocation evidence has an invalid scope or measurement shape".into(),
        ))),
    }
}

fn nested_runtime_evidence(
    budgets: &FeedbackBudgets,
) -> Result<NestedRuntimeEvidence, Box<dyn Error>> {
    let evidence: NestedRuntimeEvidence = serde_json::from_str(NESTED_RUNTIME_EVIDENCE)?;
    validate_nested_allocation_evidence(&evidence.summary.frame_allocations)?;
    let mut warm = evidence.samples.warm_present_us.clone();
    let mut input = evidence.samples.input_to_visible_us.clone();
    if evidence.environment.hardware_claim
        || warm.len() < budgets.live.samples
        || input.len() < budgets.live.samples
        || p95_u64(&mut warm) != evidence.summary.warm_present_p95_us
        || p95_u64(&mut input) != evidence.summary.input_to_visible_p95_us
        || evidence.summary.warm_present_p95_us as f64 > budgets.live.warm_frame_p95_ms * 1_000.0
        || evidence.summary.input_to_visible_p95_us as f64
            > budgets.live.input_to_visible_p95_ms * 1_000.0
        || evidence.summary.retained_presenter_bytes > budgets.focused.retained_frame_bytes
        || evidence.summary.frame_allocations.scope == "unavailable"
        || evidence.summary.frame_allocations.sample_count < budgets.live.samples
        || evidence
            .summary
            .frame_allocations
            .count
            .is_none_or(|count| count > budgets.focused.frame_allocations as u64)
        || !evidence.result.frame_allocations
    {
        return Err(Box::new(UsageError(
            "nested runtime evidence is inconsistent, insufficient, or exceeds its budget".into(),
        )));
    }
    Ok(evidence)
}

fn budgets() -> Result<FeedbackBudgets, Box<dyn Error>> {
    let budgets: FeedbackBudgets = toml::from_str(FEEDBACK_BUDGETS)?;
    let metadata = &budgets.metadata;
    let metadata_complete = [
        &metadata.build_profile,
        &metadata.toolchain,
        &metadata.cpu,
        &metadata.gpu,
        &metadata.fixture,
        &metadata.execution,
    ]
    .into_iter()
    .all(|value| !value.trim().is_empty())
        && metadata.scale > 0.0;
    let tiers_complete = budgets.fast.incremental_compile_ms > 0.0
        && budgets.fast.selected_unit_test_ms > 0.0
        && budgets.focused.workbench_open_p95_ms > 0.0
        && budgets.focused.warm_frame_p95_ms > 0.0
        && budgets.focused.input_to_visible_p95_ms > 0.0
        && budgets.focused.frame_allocations == 0
        && budgets.focused.retained_frame_bytes > 0
        && budgets.focused.cache_growth_bytes > 0
        && budgets.pre_commit.strict_clippy_ms > 0.0
        && budgets.pre_commit.workspace_test_ms > 0.0
        && budgets.full_visual.full_matrix_ms > 0.0
        && budgets.full_visual.shardable
        && budgets.focused.hard_command_ms * 4.0 < budgets.full_visual.full_matrix_ms
        && budgets.live.input_to_visible_p95_ms > 0.0
        && budgets.live.warm_frame_p95_ms > 0.0
        && budgets.live.samples >= 2;
    if budgets.version != 1 || budgets.focused.samples < 2 || !metadata_complete || !tiers_complete
    {
        return Err(Box::new(UsageError(
            "invalid feedback budget manifest".into(),
        )));
    }
    Ok(budgets)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CacheInventoryValidation {
    Routine,
    FinalCompletion,
}

const REQUIRED_UI_CACHE_IDS: &[&str] = &[
    "compositor_cursor_buffers",
    "compositor_frame_action_icons",
    "compositor_identify_badges",
    "compositor_output_backgrounds",
    "cosmic_text_font_systems",
    "native_glyph_atlas",
    "native_image_textures",
    "shell_presenter_pixels",
    "smithay_renderer_internal_caches",
    "software_glyph_raster",
    "window_titlebar_rasters",
];

fn validate_cache_inventory_with(
    inventory: &str,
    validation: CacheInventoryValidation,
) -> Result<usize, Box<dyn Error>> {
    let mut lines = inventory.lines();
    if lines.next()
        != Some(
            "id\towner\tcategory\tkey_type\tvalue_type\tsource_truth\tgeneration\tmax_entries\tmax_bytes\teviction\tinvalidation\tlifetime\tfallback\tstatus\tevidence",
        )
    {
        return Err(Box::new(UsageError(
            "invalid cache inventory header".into(),
        )));
    }
    let mut ids = std::collections::BTreeSet::new();
    let mut count = 0;
    for (line_number, line) in lines.enumerate() {
        let columns = line.split('\t').collect::<Vec<_>>();
        if columns.len() != 15 || columns.iter().any(|column| column.trim().is_empty()) {
            return Err(Box::new(UsageError(format!(
                "invalid cache inventory row {}",
                line_number + 2
            ))));
        }
        if !ids.insert(columns[0]) {
            return Err(Box::new(UsageError(format!(
                "duplicate cache inventory id `{}`",
                columns[0]
            ))));
        }
        if !matches!(
            columns[13],
            "removed"
                | "remove"
                | "pending_measure"
                | "admitted_measured"
                | "measured_admitted"
                | "admitted_opaque"
                | "resource_admitted"
                | "lifecycle_fixed"
                | "lifecycle_fix_required"
        ) {
            return Err(Box::new(UsageError(format!(
                "invalid cache inventory status `{}`",
                columns[13]
            ))));
        }
        if matches!(columns[13], "admitted_measured" | "measured_admitted")
            && columns[7] == "0"
            && columns[8] == "0"
        {
            return Err(Box::new(UsageError(format!(
                "admitted cache `{}` has no entry or byte bound",
                columns[0]
            ))));
        }
        if columns[13] == "resource_admitted" {
            if !matches!(columns[2], "resource_reuse" | "speculative_background_work") {
                return Err(Box::new(UsageError(format!(
                    "resource-owned admission `{}` has performance-derived category `{}`",
                    columns[0], columns[2]
                ))));
            }
            if columns[7] == "0" || columns[8] == "0" {
                return Err(Box::new(UsageError(format!(
                    "resource-owned admission `{}` has no retained entry or byte bound",
                    columns[0]
                ))));
            }
            for field in [
                "admission=resource_ownership",
                "bounded=",
                "release=",
                "authority=",
            ] {
                if !columns[14].contains(field) {
                    return Err(Box::new(UsageError(format!(
                        "resource-owned admission `{}` is missing structured `{field}` evidence",
                        columns[0]
                    ))));
                }
            }
        }
        if columns[8] == "opaque_dependency" && !columns[14].contains("opaque") {
            return Err(Box::new(UsageError(format!(
                "dependency-owned cache `{}` must describe its opaque accounting",
                columns[0]
            ))));
        }
        if columns[0] == "smithay_renderer_internal_caches"
            && (!columns[14].contains("active and peak safe Rust owner-instance diagnostics")
                || !columns[14].contains("ordinary owner drop")
                || !columns[14].contains("remain opaque"))
        {
            return Err(Box::new(UsageError(format!(
                "dependency owner `{}` lacks executable owner lifecycle evidence or explicit opacity",
                columns[0]
            ))));
        }
        if columns[0] == "smithay_renderer_internal_caches"
            && columns[13] == "admitted_opaque"
            && (!columns[14].contains("hard admission=1")
                || !columns[14].contains("active DRM render nodes"))
        {
            return Err(Box::new(UsageError(
                "Smithay opaque admission lacks its enforced top-level bound or native child resource bound"
                    .into(),
            )));
        }
        if columns[0] == "cosmic_text_font_systems"
            && (columns[7] != "1"
                || columns[13] != "admitted_opaque"
                || !columns[14].contains("hard process-wide owner cardinality of 1")
                || !columns[14].contains("zero-state handle")
                || !columns[14].contains("process teardown")
                || !columns[14].contains("remain opaque"))
        {
            return Err(Box::new(UsageError(
                "cosmic-text owner lacks a hard process-wide singleton bound or honest opacity"
                    .into(),
            )));
        }
        if columns[13] == "admitted_opaque" {
            if columns[8] != "opaque_dependency" {
                return Err(Box::new(UsageError(format!(
                    "opaque admission `{}` must retain explicit opaque dependency byte accounting",
                    columns[0]
                ))));
            }
            if matches!(columns[7], "0" | "dependency-owned") {
                return Err(Box::new(UsageError(format!(
                    "opaque admission `{}` has no bounded Nickel owner cardinality",
                    columns[0]
                ))));
            }
            if !columns[14].contains("drop") && !columns[14].contains("process teardown") {
                return Err(Box::new(UsageError(format!(
                    "opaque admission `{}` must state its owner release lifecycle",
                    columns[0]
                ))));
            }
        }
        if validation == CacheInventoryValidation::FinalCompletion
            && !matches!(
                columns[13],
                "removed"
                    | "admitted_measured"
                    | "measured_admitted"
                    | "admitted_opaque"
                    | "resource_admitted"
            )
        {
            return Err(Box::new(UsageError(format!(
                "cache `{}` is not final-completion ready: status `{}`",
                columns[0], columns[13]
            ))));
        }
        count += 1;
    }
    if count == 0 {
        return Err(Box::new(UsageError("cache inventory is empty".into())));
    }
    let missing = REQUIRED_UI_CACHE_IDS
        .iter()
        .filter(|id| !ids.contains(**id))
        .copied()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(Box::new(UsageError(format!(
            "cache inventory omits required UI retained resources: {}",
            missing.join(", ")
        ))));
    }
    Ok(count)
}

fn validate_cache_inventory() -> Result<usize, Box<dyn Error>> {
    validate_cache_inventory_with(CACHE_INVENTORY, CacheInventoryValidation::Routine)
}

fn validate_cache_inventory_for_final_completion() -> Result<usize, Box<dyn Error>> {
    validate_cache_inventory_with(CACHE_INVENTORY, CacheInventoryValidation::FinalCompletion)
}

fn validate_cache_lifecycle_matrix(inventory: &str, matrix: &str) -> Result<usize, Box<dyn Error>> {
    const HEADER: &str = "id\thide\tsuspend\tclose\toutput_reconnect\ttopology_shrink\ttheme\tlocale\tfont\tapplication_replace\tfixture_teardown";
    const ACTIONS: &[&str] = &[
        "na",
        "retain",
        "clear",
        "drop_owner",
        "replace",
        "rebuild",
        "reconcile",
    ];

    let inventory_ids = inventory
        .lines()
        .skip(1)
        .filter_map(|line| line.split('\t').next())
        .collect::<std::collections::BTreeSet<_>>();
    let mut lines = matrix.lines();
    if lines.next() != Some(HEADER) {
        return Err(Box::new(UsageError(
            "invalid cache lifecycle matrix header".into(),
        )));
    }
    let mut matrix_ids = std::collections::BTreeSet::new();
    for (line_number, line) in lines.enumerate() {
        let columns = line.split('\t').collect::<Vec<_>>();
        if columns.len() != 11 || columns.iter().any(|column| column.trim().is_empty()) {
            return Err(Box::new(UsageError(format!(
                "invalid cache lifecycle row {}",
                line_number + 2
            ))));
        }
        if !matrix_ids.insert(columns[0]) {
            return Err(Box::new(UsageError(format!(
                "duplicate cache lifecycle id `{}`",
                columns[0]
            ))));
        }
        for (boundary, action) in HEADER.split('\t').skip(1).zip(columns.iter().skip(1)) {
            if !ACTIONS.contains(action) {
                return Err(Box::new(UsageError(format!(
                    "cache `{}` has invalid `{}` lifecycle action `{}`",
                    columns[0], boundary, action
                ))));
            }
        }
    }
    let missing = inventory_ids
        .difference(&matrix_ids)
        .copied()
        .collect::<Vec<_>>();
    let unknown = matrix_ids
        .difference(&inventory_ids)
        .copied()
        .collect::<Vec<_>>();
    if !missing.is_empty() || !unknown.is_empty() {
        return Err(Box::new(UsageError(format!(
            "cache lifecycle matrix differs from inventory: missing [{}], unknown [{}]",
            missing.join(", "),
            unknown.join(", ")
        ))));
    }
    Ok(matrix_ids.len())
}

fn validate_consumer_inventory() -> Result<usize, Box<dyn Error>> {
    let mut lines = CONSUMER_INVENTORY.lines();
    if lines.next()
        != Some(
            "order\tsurface\tcrate\tarchitecture_state\tmigration_status\towner\tplatform_scope\tsemantic_roles\tsemantic_actions\tpaint_authority\thost_authority\tcustom_paint_exception\tworkbench_fixtures\tscenario_evidence\tvisual_variants\taccessibility_evidence\tcontroller_evidence\tlive_acceptance\tresource_evidence\tdepends_on",
        )
    {
        return Err(Box::new(UsageError(
            "invalid consumer inventory header".into(),
        )));
    }
    let mut surfaces = std::collections::BTreeSet::new();
    let mut expected_order = 1usize;
    for (line_number, line) in lines.enumerate() {
        let columns = line.split('\t').collect::<Vec<_>>();
        if columns.len() != 20 || columns.iter().any(|column| column.trim().is_empty()) {
            return Err(Box::new(UsageError(format!(
                "invalid consumer inventory row {}",
                line_number + 2
            ))));
        }
        let order = columns[0].parse::<usize>()?;
        if order != expected_order {
            return Err(Box::new(UsageError(format!(
                "consumer order {} does not follow {}",
                order, expected_order
            ))));
        }
        if !matches!(
            columns[4],
            "architecture_verified_acceptance_pending"
                | "headless_verified_live_not_applicable"
                | "headless_verified_live_pending"
                | "external_migration_pending"
        ) {
            return Err(Box::new(UsageError(format!(
                "invalid consumer migration status `{}`",
                columns[4]
            ))));
        }
        if !matches!(columns[9], "frame" | "mixed") {
            return Err(Box::new(UsageError(format!(
                "invalid consumer paint authority `{}`",
                columns[9]
            ))));
        }
        if !surfaces.insert(columns[1]) {
            return Err(Box::new(UsageError(format!(
                "duplicate consumer surface `{}`",
                columns[1]
            ))));
        }
        expected_order += 1;
    }
    Ok(expected_order - 1)
}

fn validate_fixture_asset(asset: &FixtureAsset) -> Result<(), Box<dyn Error>> {
    let manifest: VisualFixtureManifest = toml::from_str(VISUAL_FIXTURES)?;
    let record = manifest
        .reference
        .iter()
        .find(|record| record.id == asset.id)
        .ok_or_else(|| {
            UsageError(format!(
                "asset `{}` is absent from visual-fixtures.toml",
                asset.id
            ))
        })?;
    let expected_path = format!("assets/{}", record.path);
    if expected_path != asset.path
        || record.sha256 != asset.sha256
        || record.authorship.trim().is_empty()
        || record.usage_status.trim().is_empty()
        || record.source.trim().is_empty()
        || asset.license.trim().is_empty()
    {
        return Err(Box::new(UsageError(format!(
            "asset `{}` provenance or license metadata does not match its manifest record",
            asset.id
        ))));
    }
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let bytes = fs::read(repository.join(asset.path))?;
    let digest = format!("{:x}", Sha256::digest(bytes));
    if digest != asset.sha256 {
        return Err(Box::new(UsageError(format!(
            "asset `{}` checksum mismatch: expected {}, got {digest}",
            asset.id, asset.sha256
        ))));
    }
    Ok(())
}

fn p95(samples: &mut [f64]) -> f64 {
    samples.sort_by(f64::total_cmp);
    let index = ((samples.len() as f64 * 0.95).ceil() as usize)
        .saturating_sub(1)
        .min(samples.len().saturating_sub(1));
    samples[index]
}

fn p95_u64(samples: &mut [u64]) -> u64 {
    samples.sort_unstable();
    let index = ((samples.len() as f64 * 0.95).ceil() as usize)
        .saturating_sub(1)
        .min(samples.len().saturating_sub(1));
    samples[index]
}

struct ExecutionMetadata {
    rustc: String,
    cpu: String,
    renderer: &'static str,
}

fn execution_metadata() -> ExecutionMetadata {
    let rustc = std::process::Command::new("rustc")
        .arg("-Vv")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| {
            String::from_utf8_lossy(&output.stdout)
                .trim()
                .replace('\n', " | ")
        })
        .unwrap_or_else(|| "rustc unavailable".into());
    let cpu = fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|contents| {
            contents.lines().find_map(|line| {
                line.strip_prefix("model name\t:")
                    .map(|value| value.trim().to_owned())
            })
        })
        .unwrap_or_else(|| format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH));
    ExecutionMetadata {
        rustc,
        cpu,
        renderer: "nickel-ui deterministic software raster; GPU presenter not instantiated",
    }
}

fn linux_rss_bytes() -> Option<usize> {
    fs::read_to_string("/proc/self/status")
        .ok()?
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))?
        .split_whitespace()
        .next()?
        .parse::<usize>()
        .ok()
        .map(|kib| kib.saturating_mul(1024))
}

#[derive(Clone, Debug, PartialEq)]
enum CounterMessage {
    Increment,
    Query(String),
}

#[derive(Default)]
struct CounterApp {
    count: usize,
    query: String,
}

impl Application for CounterApp {
    type Message = CounterMessage;

    fn update(&mut self, message: Self::Message) {
        match message {
            CounterMessage::Increment => self.count += 1,
            CounterMessage::Query(query) => self.query = query,
        }
    }

    fn view(&self, _context: ViewContext) -> impl nickel_ui::View<Self::Message> {
        Column::new()
            .gap(12.0)
            .child(Button::new(CounterMessage::Increment, "Increment").id("increment"))
            .child(Text::new(format!("Count: {}", self.count)))
            .child(
                TextField::on_change(&self.query, CounterMessage::Query)
                    .id("query")
                    .accessibility_label("Counter filter"),
            )
    }
}

struct CounterFixture;

impl Fixture for CounterFixture {
    type App = CounterApp;

    fn metadata() -> &'static FixtureMetadata {
        static METADATA: FixtureMetadata = FixtureMetadata {
            id: COUNTER_ID,
            title: "Counter",
            description: "Button and text-field semantic interaction",
            tags: &["core", "button", "text-field"],
            source: FixtureSource {
                crate_name: "nickel-ui-workbench",
                file: file!(),
                line: line!(),
            },
            variants: &[FixtureVariant {
                id: "default",
                title: "Default",
                viewport: ViewportPreset {
                    id: "default",
                    width: 480,
                    height: 240,
                },
                theme: FixtureTheme::Dark,
                locale: LocalePreset {
                    id: "en-US",
                    direction: FixtureDirection::LeftToRight,
                },
                scale: ScalePreset {
                    id: "1x",
                    factor: 1.0,
                },
                controller_family: nickel_ui::ControllerFamily::Generic,
                accessibility: AccessibilityPreset {
                    id: "default",
                    high_contrast: false,
                    reduced_motion: false,
                    reduced_transparency: false,
                },
            }],
            assets: &[],
            simulated_effects: &[],
        };
        &METADATA
    }

    fn create() -> Self::App {
        CounterApp::default()
    }

    fn surface_size() -> (u32, u32) {
        (480, 240)
    }

    fn default_activation() -> Option<Selector> {
        Some(Selector::role_name(SemanticRole::Button, "Increment"))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum MarkdownMessage {
    OpenLink(String),
}

struct MarkdownApp {
    document: MarkdownDocument,
    opened_link: Option<String>,
}

impl Default for MarkdownApp {
    fn default() -> Self {
        Self {
            document: MarkdownDocument::parse(MARKDOWN_SOURCE),
            opened_link: None,
        }
    }
}

impl Application for MarkdownApp {
    type Message = MarkdownMessage;

    fn update(&mut self, message: Self::Message) {
        match message {
            MarkdownMessage::OpenLink(destination) => self.opened_link = Some(destination),
        }
    }

    fn view(&self, _context: ViewContext) -> impl nickel_ui::View<Self::Message> {
        AnyView::new(markdown_view(
            &self.document,
            MarkdownPalette::default(),
            |destination| MarkdownMessage::OpenLink(destination.to_owned()),
        ))
    }
}

struct MarkdownFixture;

impl Fixture for MarkdownFixture {
    type App = MarkdownApp;

    fn metadata() -> &'static FixtureMetadata {
        static METADATA: FixtureMetadata = FixtureMetadata {
            id: MARKDOWN_ID,
            title: "Markdown core",
            description: "Production declarative Markdown prose, link, code, and table",
            tags: &["markdown", "link", "code", "table"],
            source: FixtureSource {
                crate_name: "nickel-markdown",
                file: file!(),
                line: line!(),
            },
            variants: &[FixtureVariant {
                id: "default",
                title: "Default",
                viewport: ViewportPreset {
                    id: "default",
                    width: 720,
                    height: 640,
                },
                theme: FixtureTheme::Dark,
                locale: LocalePreset {
                    id: "en-US",
                    direction: FixtureDirection::LeftToRight,
                },
                scale: ScalePreset {
                    id: "1x",
                    factor: 1.0,
                },
                controller_family: nickel_ui::ControllerFamily::Generic,
                accessibility: AccessibilityPreset {
                    id: "default",
                    high_contrast: false,
                    reduced_motion: false,
                    reduced_transparency: false,
                },
            }],
            assets: &[],
            simulated_effects: &[SimulatedEffectKind::OpenUrl],
        };
        &METADATA
    }

    fn create() -> Self::App {
        MarkdownApp::default()
    }

    fn surface_size() -> (u32, u32) {
        (720, 640)
    }

    fn default_activation() -> Option<Selector> {
        Some(Selector::role_name(SemanticRole::Button, "the guide  ↗"))
    }
}

#[derive(Clone, Debug, PartialEq)]
enum GalleryMessage {
    Activate(&'static str),
    Text(String),
    Adjust(f32),
    Toggle(bool),
    Select(&'static str),
}

#[derive(Clone, Copy)]
enum GalleryKind {
    Primitives,
    CollectionStates,
    Settings { rtl: bool },
    Launcher,
    Codex,
    Menus,
}

struct GalleryApp {
    kind: GalleryKind,
    fixture_theme: FixtureTheme,
    fixture_direction: FixtureDirection,
    controller_family: nickel_ui::ControllerFamily,
    accessibility: AccessibilityPreset,
    text: String,
    selected: &'static str,
    enabled: bool,
    value: f32,
}

impl GalleryApp {
    fn new(kind: GalleryKind) -> Self {
        Self {
            kind,
            fixture_theme: FixtureTheme::Dark,
            fixture_direction: FixtureDirection::LeftToRight,
            controller_family: nickel_ui::ControllerFamily::Generic,
            accessibility: AccessibilityPreset {
                id: "default",
                high_contrast: false,
                reduced_motion: false,
                reduced_transparency: false,
            },
            text: String::new(),
            selected: "general",
            enabled: true,
            value: 0.42,
        }
    }

    fn from_variant(kind: GalleryKind, variant: &FixtureVariant) -> Self {
        let mut app = Self::new(kind);
        app.fixture_theme = variant.theme;
        app.fixture_direction = variant.locale.direction;
        app.controller_family = variant.controller_family;
        app.accessibility = variant.accessibility;
        app
    }

    fn theme(&self) -> SemanticTheme {
        match self.fixture_theme {
            FixtureTheme::Light => {
                SemanticTheme::from_tokens(nickel_ui::SemanticTokenSet::standard(
                    0xf5f7fb, 0xe8edf5, 0xffffff, 0xdce5f1, 0xcbd7e6, 0x17202e, 0x52647a, 0x5641d8,
                    0xded9ff, 0x156db8, 0x217a50,
                ))
            }
            FixtureTheme::HighContrast => {
                SemanticTheme::from_tokens(nickel_ui::SemanticTokenSet::standard(
                    0x000000, 0x090909, 0x111111, 0x1c1c1c, 0x292929, 0xffffff, 0xe8e8e8, 0xffd400,
                    0x4a3d00, 0x00e5ff, 0x7dff8a,
                ))
            }
            FixtureTheme::Dark => workbench_theme(),
        }
    }

    fn primitives(&self) -> AnyView<GalleryMessage> {
        AnyView::new(
            Surface::new(self.theme(), SurfaceRole::Window)
                .child(Text::new("Shared primitive gallery").bold(true).scale(1.35))
                .child(
                    Row::new()
                        .gap(10.0)
                        .child(
                            Button::new(GalleryMessage::Activate("primary"), "Primary action")
                                .id("primary"),
                        )
                        .child(
                            RadioButton::new(
                                GalleryMessage::Select("general"),
                                "General",
                                self.selected == "general",
                            )
                            .accessibility_label("General"),
                        )
                        .child(
                            Switch::new(self.enabled, GalleryMessage::Toggle, self.theme())
                                .accessibility_label("Enable shared primitive"),
                        ),
                )
                .child(
                    TextField::on_change_with_placeholder(
                        &self.text,
                        "Search",
                        GalleryMessage::Text,
                    )
                    .id("search"),
                )
                .child(
                    Slider::on_change(GalleryMessage::Adjust, self.value)
                        .id("volume")
                        .accessibility_label("Volume"),
                )
                .child(
                    Dropdown::new(
                        GalleryMessage::Activate("dropdown"),
                        "Comfortable",
                        [
                            ("Compact", GalleryMessage::Select("compact")),
                            ("Comfortable", GalleryMessage::Select("comfortable")),
                        ],
                    )
                    .id("density")
                    .accessibility_label("Density"),
                )
                .child(Grid::fixed(3).gap(8.0).children(
                    ["Container", "Row / Column", "Grid"].map(|label| {
                        Container::new()
                            .padding(Insets::all(10.0))
                            .background(0x202b3b)
                            .child(Text::new(label))
                    }),
                )),
        )
    }

    fn collections(&self) -> AnyView<GalleryMessage> {
        fn tile(item: (&'static str, &'static str)) -> Container<GalleryMessage> {
            Container::new()
                .padding(Insets::all(8.0))
                .background(0x202b3b)
                .accessibility_label(item.1)
                .child(Text::new(item.1))
        }
        let ready = Collection::try_new(
            CollectionState::Ready(vec![("one", "One"), ("two", "Two"), ("three", "Three")]),
            |item| item.0,
            tile,
        )
        .expect("unique gallery keys")
        .id("ready")
        .item_label(|item| item.1.to_owned())
        .gap(8.0)
        .presentation(CollectionPresentation::AdaptiveGrid {
            minimum_item_width: 110.0,
        })
        .on_activate(|key| GalleryMessage::Select(key));
        let empty = Collection::try_new(
            CollectionState::<(&str, &str)>::Ready(vec![]),
            |item| item.0,
            tile,
        )
        .expect("empty collection")
        .id("empty")
        .empty_label("Nothing pinned yet");
        let loading = Collection::try_new(
            CollectionState::<(&str, &str)>::Loading,
            |item| item.0,
            tile,
        )
        .expect("loading collection")
        .id("loading")
        .loading_label("Loading recent items…");
        let error = Collection::try_new(
            CollectionState::<(&str, &str)>::Error("Offline".into()),
            |item| item.0,
            tile,
        )
        .expect("error collection")
        .id("error");
        AnyView::new(
            Column::new()
                .gap(14.0)
                .padding(Insets::all(18.0))
                .child(Text::new("Collection lifecycle").bold(true).scale(1.3))
                .child(ready)
                .child(empty)
                .child(loading)
                .child(error),
        )
    }

    fn settings(&self, width: f32, rtl: bool) -> AnyView<GalleryMessage> {
        let destinations = [
            ResponsiveNavigationDestination::new(
                "general",
                "General",
                GalleryMessage::Select("general"),
                Column::new()
                    .gap(10.0)
                    .padding(Insets::all(18.0))
                    .child(Text::new("General settings").bold(true))
                    .child(self.primitives()),
            ),
            ResponsiveNavigationDestination::new(
                "appearance",
                "Appearance",
                GalleryMessage::Select("appearance"),
                Column::new()
                    .padding(Insets::all(18.0))
                    .child(Text::new("Theme and accessibility preferences")),
            )
            .section("Personalization"),
        ];
        AnyView::new(
            ResponsiveNavigation::try_new(self.theme(), width, Some(self.selected), destinations)
                .expect("known active setting")
                .id("settings-navigation")
                .direction(
                    if rtl || self.fixture_direction == FixtureDirection::RightToLeft {
                        ReadingDirection::RightToLeft
                    } else {
                        ReadingDirection::LeftToRight
                    },
                )
                .navigation_header(
                    Text::new(if rtl {
                        "الإعدادات"
                    } else {
                        "Settings"
                    })
                    .bold(true),
                ),
        )
    }

    fn product_surface(
        &self,
        title: &'static str,
        items: &[&'static str],
    ) -> AnyView<GalleryMessage> {
        let cards = items.iter().enumerate().map(|(index, label)| {
            Button::new(GalleryMessage::Activate(label), *label)
                .id(format!("item-{index}"))
                .height(72.0)
        });
        AnyView::new(
            Surface::new(self.theme(), SurfaceRole::Window).child(
                Column::new()
                    .gap(12.0)
                    .padding(Insets::all(18.0))
                    .child(Text::new(title).bold(true).scale(1.35))
                    .child(
                        TextField::on_change_with_placeholder(
                            &self.text,
                            "Search…",
                            GalleryMessage::Text,
                        )
                        .id("product-search"),
                    )
                    .child(
                        Grid::fixed(3)
                            .gap(10.0)
                            .direction(match self.fixture_direction {
                                FixtureDirection::LeftToRight => ReadingDirection::LeftToRight,
                                FixtureDirection::RightToLeft => ReadingDirection::RightToLeft,
                            })
                            .children(cards),
                    ),
            ),
        )
    }

    fn menus(&self) -> AnyView<GalleryMessage> {
        AnyView::new(
            Column::new()
                .gap(14.0)
                .padding(Insets::all(18.0))
                .child(Text::new("Menus and typed actions").bold(true))
                .child(
                    MenuBar::new().child(
                        Menu::new(
                            GalleryMessage::Activate("menu"),
                            "Actions",
                            [
                                MenuItem::new("Pin", GalleryMessage::Activate("pin")),
                                MenuItem::new("Log out", GalleryMessage::Activate("logout")),
                                MenuItem::disabled("Unavailable"),
                            ],
                        )
                        .id("actions-menu")
                        .accessibility_label("Actions"),
                    ),
                )
                .child(
                    Button::new(GalleryMessage::Activate("context"), "Context menu target")
                        .id("context-target"),
                ),
        )
    }
}

impl Application for GalleryApp {
    type Message = GalleryMessage;
    fn update(&mut self, message: Self::Message) {
        match message {
            GalleryMessage::Text(value) => self.text = value,
            GalleryMessage::Adjust(value) => self.value = value,
            GalleryMessage::Toggle(value) => self.enabled = value,
            GalleryMessage::Select(value) => self.selected = value,
            GalleryMessage::Activate(_) => {}
        }
    }
    fn view(&self, context: ViewContext) -> impl nickel_ui::View<Self::Message> {
        match self.kind {
            GalleryKind::Primitives => self.primitives(),
            GalleryKind::CollectionStates => self.collections(),
            GalleryKind::Settings { rtl } => self.settings(context.viewport.size.width, rtl),
            GalleryKind::Launcher => self.product_surface(
                "Launcher",
                &[
                    "Files",
                    "Terminal",
                    "Browser",
                    "Settings",
                    "Software Center",
                    "Log out",
                    "Nickel project",
                    "See all projects",
                ],
            ),
            GalleryKind::Codex => AnyView::new(
                Column::new()
                    .fill_width()
                    .fill_height()
                    .gap(10.0)
                    .padding(Insets::all(20.0))
                    .background(0x101722)
                    .child(Text::new("Codex chat").bold(true).scale(5.0))
                    .child(
                        Container::new()
                            .fill_width()
                            .min_height(80.0)
                            .padding(Insets::all(14.0))
                            .background(0x202b3b)
                            .child(
                                Text::new("Assistant response with semantic Markdown and code.")
                                    .scale(3.5)
                                    .wrap(true)
                                    .max_lines(2),
                            ),
                    )
                    .child(
                        VerticalScroll::new(GalleryMessage::Activate("scroll"), 0.0)
                            .fill_width()
                            .height(140.0)
                            .background(0x0b111a)
                            .padding(Insets::all(14.0))
                            .child(
                                Text::new("Tool output\nBuild passed\nTests passed")
                                    .scale(3.2)
                                    .color(0xb8c5d6),
                            ),
                    )
                    .child(
                        TextField::on_change_with_placeholder(
                            &self.text,
                            "Message Codex",
                            GalleryMessage::Text,
                        )
                        .scale(3.8)
                        .id("composer")
                        .fill_width(),
                    )
                    .child(
                        Button::with_label(
                            GalleryMessage::Activate("send"),
                            ButtonLabel::new("Send").scale(3.8),
                        )
                        .id("send")
                        .height(44.0)
                        .width(160.0)
                        .accessibility_label("Send"),
                    ),
            ),
            GalleryKind::Menus => self.menus(),
        }
    }
}

fn workbench_theme() -> SemanticTheme {
    SemanticTheme::from_tokens(nickel_ui::SemanticTokenSet::standard(
        0x101722, 0x151f2d, 0x1c2838, 0x243247, 0x2d405a, 0xf2f6fb, 0x9cacc0, 0x7b61ff, 0x332c61,
        0x67b7ff, 0x61c993,
    ))
}

macro_rules! gallery_fixture {
    ($name:ident, $id:expr, $title:expr, $description:expr, $tags:expr, $kind:expr, $size:expr, $selector:expr, $variants:expr, $effects:expr) => {
        gallery_fixture!(@impl $name, $id, $title, $description, $tags, $kind, $size, $selector, $variants, $effects, &[]);
    };
    ($name:ident, $id:expr, $title:expr, $description:expr, $tags:expr, $kind:expr, $size:expr, $selector:expr, $variants:expr, $effects:expr, $assets:expr) => {
        gallery_fixture!(@impl $name, $id, $title, $description, $tags, $kind, $size, $selector, $variants, $effects, $assets);
    };
    (@impl $name:ident, $id:expr, $title:expr, $description:expr, $tags:expr, $kind:expr, $size:expr, $selector:expr, $variants:expr, $effects:expr, $assets:expr) => {
        struct $name;
        impl Fixture for $name {
            type App = GalleryApp;
            fn metadata() -> &'static FixtureMetadata {
                static METADATA: FixtureMetadata = FixtureMetadata {
                    id: $id,
                    title: $title,
                    description: $description,
                    tags: $tags,
                    source: FixtureSource {
                        crate_name: "nickel-ui-workbench",
                        file: file!(),
                        line: line!(),
                    },
                    variants: $variants,
                    assets: $assets,
                    simulated_effects: $effects,
                };
                &METADATA
            }
            fn create() -> Self::App {
                GalleryApp::new($kind)
            }
            fn create_variant(variant: &FixtureVariant) -> Self::App {
                GalleryApp::from_variant($kind, variant)
            }
            fn surface_size() -> (u32, u32) {
                $size
            }
            fn default_activation() -> Option<Selector> {
                Some($selector)
            }
        }
    };
}

macro_rules! fixture_variant {
    ($id:expr, $title:expr, $width:expr, $height:expr, $theme:expr, $direction:expr, $family:expr, $contrast:expr, $motion:expr) => {
        FixtureVariant {
            id: $id,
            title: $title,
            viewport: ViewportPreset {
                id: $id,
                width: $width,
                height: $height,
            },
            theme: $theme,
            locale: LocalePreset {
                id: if matches!($direction, FixtureDirection::RightToLeft) {
                    "ar"
                } else {
                    "en-US"
                },
                direction: $direction,
            },
            scale: ScalePreset {
                id: "1x",
                factor: 1.0,
            },
            controller_family: $family,
            accessibility: AccessibilityPreset {
                id: if $contrast {
                    "high-contrast"
                } else if $motion {
                    "reduced-motion"
                } else {
                    "default"
                },
                high_contrast: $contrast,
                reduced_motion: $motion,
                reduced_transparency: $motion,
            },
        }
    };
}

gallery_fixture!(
    PrimitivesFixture,
    PRIMITIVES_ID,
    "Shared primitives",
    "Public controls, layout, input, selection, and image-free composition",
    &["shared", "controls", "high-contrast", "reduced-effects"],
    GalleryKind::Primitives,
    (760, 520),
    Selector::role_name(SemanticRole::Button, "Primary action"),
    &[
        fixture_variant!(
            "dark",
            "Dark",
            760,
            520,
            FixtureTheme::Dark,
            FixtureDirection::LeftToRight,
            nickel_ui::ControllerFamily::Generic,
            false,
            false
        ),
        fixture_variant!(
            "high-contrast",
            "High contrast",
            760,
            520,
            FixtureTheme::HighContrast,
            FixtureDirection::LeftToRight,
            nickel_ui::ControllerFamily::Generic,
            true,
            true
        ),
        FixtureVariant {
            id: "scale-150",
            title: "150% scale",
            viewport: ViewportPreset {
                id: "wide",
                width: 760,
                height: 520,
            },
            theme: FixtureTheme::Dark,
            locale: LocalePreset {
                id: "en-US",
                direction: FixtureDirection::LeftToRight,
            },
            scale: ScalePreset {
                id: "150-percent",
                factor: 1.5,
            },
            controller_family: nickel_ui::ControllerFamily::Generic,
            accessibility: AccessibilityPreset {
                id: "default",
                high_contrast: false,
                reduced_motion: false,
                reduced_transparency: false,
            },
        },
    ],
    &[]
);
gallery_fixture!(
    CollectionStatesFixture,
    COLLECTION_STATES_ID,
    "Collection states",
    "Keyed grid plus empty, loading, and error states",
    &["collection", "grid", "empty", "loading", "error"],
    GalleryKind::CollectionStates,
    (700, 520),
    Selector::id("root/ready/ready/one"),
    &[fixture_variant!(
        "lifecycle",
        "Lifecycle states",
        700,
        520,
        FixtureTheme::Dark,
        FixtureDirection::LeftToRight,
        nickel_ui::ControllerFamily::Generic,
        false,
        false
    )],
    &[]
);
gallery_fixture!(
    SettingsWideFixture,
    SETTINGS_WIDE_ID,
    "Settings wide",
    "Representative wide Settings navigation and controls",
    &["settings", "wide", "theme-dark"],
    GalleryKind::Settings { rtl: false },
    (900, 620),
    Selector::role_name(SemanticRole::Button, "Primary action"),
    &[fixture_variant!(
        "wide",
        "Wide",
        900,
        620,
        FixtureTheme::Dark,
        FixtureDirection::LeftToRight,
        nickel_ui::ControllerFamily::Xbox,
        false,
        false
    )],
    &[]
);
gallery_fixture!(
    SettingsNarrowRtlFixture,
    SETTINGS_NARROW_RTL_ID,
    "Settings narrow RTL",
    "Narrow right-to-left responsive navigation",
    &["settings", "narrow", "rtl", "scale"],
    GalleryKind::Settings { rtl: true },
    (420, 720),
    Selector::role_name(SemanticRole::Button, "Primary action"),
    &[fixture_variant!(
        "narrow-rtl",
        "Narrow RTL",
        420,
        720,
        FixtureTheme::Dark,
        FixtureDirection::RightToLeft,
        nickel_ui::ControllerFamily::PlayStation,
        false,
        false
    )],
    &[]
);
gallery_fixture!(
    LauncherFixture,
    LAUNCHER_ID,
    "Launcher dashboard",
    "Representative controller-reachable launcher applications and session action",
    &["launcher", "controller", "dashboard"],
    GalleryKind::Launcher,
    (820, 620),
    Selector::role_name(SemanticRole::Button, "Files"),
    &[
        fixture_variant!(
            "wide-xbox",
            "Wide Xbox",
            820,
            620,
            FixtureTheme::Dark,
            FixtureDirection::LeftToRight,
            nickel_ui::ControllerFamily::Xbox,
            false,
            false
        ),
        fixture_variant!(
            "narrow-playstation",
            "Narrow PlayStation",
            520,
            720,
            FixtureTheme::Dark,
            FixtureDirection::LeftToRight,
            nickel_ui::ControllerFamily::PlayStation,
            false,
            false
        ),
        fixture_variant!(
            "wide-switch-rtl",
            "Wide Switch RTL",
            820,
            620,
            FixtureTheme::Light,
            FixtureDirection::RightToLeft,
            nickel_ui::ControllerFamily::Switch,
            false,
            false
        ),
        fixture_variant!(
            "high-contrast-menu",
            "High contrast session menu",
            820,
            620,
            FixtureTheme::HighContrast,
            FixtureDirection::LeftToRight,
            nickel_ui::ControllerFamily::Generic,
            true,
            true
        ),
    ],
    &[
        SimulatedEffectKind::Logout,
        SimulatedEffectKind::PackageMutation
    ],
    &[FixtureAsset {
        id: "nickel-controller-launcher-2026-08-31",
        path: "assets/references/nickel-start-menu.png",
        license: "Repository-owner-authorized visual composition reference; not shipped artwork",
        sha256: "e8da9f1045b2045a1bd5e51812319ee159603dc56db7c6b7de1647f209cadebf",
    }]
);
gallery_fixture!(
    CodexFixture,
    CODEX_ID,
    "Codex chat",
    "Representative chat, tool output, scrolling, composer, and send action",
    &["codex", "chat", "scroll", "composer"],
    GalleryKind::Codex,
    (760, 420),
    Selector::id("root/send"),
    &[fixture_variant!(
        "dark",
        "Dark chat",
        760,
        420,
        FixtureTheme::Dark,
        FixtureDirection::LeftToRight,
        nickel_ui::ControllerFamily::Generic,
        false,
        false
    )],
    &[SimulatedEffectKind::ExternalAccount]
);
gallery_fixture!(
    MenusFixture,
    MENU_ID,
    "Menus",
    "Menu bar, disabled item, typed pin and logout actions, and context target",
    &["menu", "context-menu", "pin", "logout"],
    GalleryKind::Menus,
    (620, 360),
    Selector::id("root/context-target"),
    &[fixture_variant!(
        "context",
        "Context and session actions",
        620,
        360,
        FixtureTheme::Dark,
        FixtureDirection::LeftToRight,
        nickel_ui::ControllerFamily::Generic,
        false,
        false
    )],
    &[SimulatedEffectKind::Logout]
);

#[derive(Clone, Debug, PartialEq)]
enum WorkbenchMessage {
    Query(String),
    Select(String),
    SelectVariant(String),
    SetTheme(FixtureTheme),
    SetDirection(FixtureDirection),
    SetScale(f32),
    SetViewport(u32, u32),
    SetControllerFamily(nickel_ui::ControllerFamily),
    ToggleHighContrast(bool),
    ToggleReducedEffects(bool),
    CompareVariant(String),
    CompareReference(String),
    SetComparisonMode(ComparisonMode),
    ClearComparison,
    SetModality(ActivationVia),
    Activate,
    Reset,
    CatalogScroll(f32),
    MainScroll(f32),
    InspectorScroll(f32),
    SelectSemanticNode(String),
}

#[derive(Clone, Debug, PartialEq)]
struct RecordedEffect {
    fixture: String,
    variant: String,
    modality: ActivationVia,
    effect: SimulatedEffectKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ComparisonMode {
    SideBySide,
    OverlayDifference,
}

struct ReferenceComparison {
    reference: Arc<image::RgbaImage>,
    difference: Arc<image::RgbaImage>,
}

struct WorkbenchApp {
    catalog: Vec<FixtureRegistryEntry>,
    query: String,
    selected: String,
    selected_variant: String,
    comparison_variant: Option<String>,
    comparison_session: Option<Box<dyn ErasedFixtureSession>>,
    reference_comparison: Option<ReferenceComparison>,
    comparison_mode: ComparisonMode,
    modality: ActivationVia,
    session: Box<dyn ErasedFixtureSession>,
    status: String,
    catalog_scroll: f32,
    main_scroll: f32,
    inspector_scroll: f32,
    recorded_effects: Vec<RecordedEffect>,
    selected_semantic_node: Option<String>,
    last_render_ms: f64,
}

impl WorkbenchApp {
    fn new() -> Result<Self, Box<dyn Error>> {
        let catalog = registry()?;
        let selected = catalog
            .first()
            .ok_or_else(|| UsageError("fixture catalog is empty".into()))?
            .metadata
            .id
            .to_owned();
        let session = catalog[0].open();
        let selected_variant = session.variant().id.to_owned();
        Ok(Self {
            catalog,
            query: String::new(),
            selected,
            selected_variant,
            comparison_variant: None,
            comparison_session: None,
            reference_comparison: None,
            comparison_mode: ComparisonMode::SideBySide,
            modality: ActivationVia::Semantic,
            session,
            status: "Ready".into(),
            catalog_scroll: 0.0,
            main_scroll: 0.0,
            inspector_scroll: 0.0,
            recorded_effects: Vec::new(),
            selected_semantic_node: None,
            last_render_ms: 0.0,
        })
    }

    fn visible_fixture_ids(&self) -> Vec<&'static str> {
        let query = self.query.trim().to_ascii_lowercase();
        self.catalog
            .iter()
            .filter(|f| {
                let metadata = f.metadata;
                query.is_empty()
                    || metadata.id.to_ascii_lowercase().contains(&query)
                    || metadata.title.to_ascii_lowercase().contains(&query)
                    || metadata.description.to_ascii_lowercase().contains(&query)
                    || metadata.tags.iter().any(|tag| tag.contains(&query))
            })
            .map(|f| f.metadata.id)
            .collect()
    }

    fn select(&mut self, id: &str) {
        if let Some(entry) = self.catalog.iter().find(|entry| entry.metadata.id == id) {
            if entry.is_external() {
                let feature = entry
                    .external_provider
                    .expect("external entry has provider metadata")
                    .workbench_feature;
                match spawn_external_workbench(feature, &["native", id]) {
                    Ok(()) => self.status = "Started external fixture provider".into(),
                    Err(error) => self.status = format!("External provider failed: {error}"),
                }
                return;
            }
            self.selected = id.to_owned();
            self.session = entry.open();
            self.selected_variant = self.session.variant().id.to_owned();
            self.comparison_variant = None;
            self.comparison_session = None;
            self.reference_comparison = None;
            self.status = "Fixture opened from registry".into();
            self.inspector_scroll = 0.0;
            self.selected_semantic_node = None;
            self.measure_render();
        }
    }

    fn apply_configuration(&mut self, configure: impl FnOnce(&mut FixtureVariant)) {
        let Some(entry) = self
            .catalog
            .iter()
            .find(|entry| entry.metadata.id == self.selected)
        else {
            return;
        };
        let mut configuration = *self.session.variant();
        configuration.id = "custom";
        configuration.title = "Custom controls";
        configure(&mut configuration);
        self.session = entry.open_configuration(configuration);
        self.selected_variant = "custom".into();
        self.status = "Independent fixture configuration applied".into();
        self.measure_render();
    }

    fn measure_render(&mut self) {
        let started = Instant::now();
        let _ = self.session.render(1.0);
        self.last_render_ms = started.elapsed().as_secs_f64() * 1000.0;
    }
}

fn external_workbench_command(features: &str, args: &[&str]) -> std::process::Command {
    let mut command =
        std::process::Command::new(env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    command
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .args([
            "run",
            "-p",
            "nickel-ui-workbench",
            "--features",
            features,
            "--",
        ])
        .args(args);
    command
}

fn spawn_external_workbench(features: &str, args: &[&str]) -> Result<(), Box<dyn Error>> {
    external_workbench_command(features, args).spawn()?;
    Ok(())
}

fn run_external_workbench(features: &str, args: &[String]) -> Result<(), Box<dyn Error>> {
    let status = external_workbench_command(
        features,
        &args.iter().map(String::as_str).collect::<Vec<_>>(),
    )
    .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(Box::new(UsageError(format!(
            "external fixture provider exited with {status}"
        ))))
    }
}

impl Application for WorkbenchApp {
    type Message = WorkbenchMessage;

    fn update(&mut self, message: Self::Message) {
        match message {
            WorkbenchMessage::Query(query) => self.query = query,
            WorkbenchMessage::Select(id) => self.select(&id),
            WorkbenchMessage::SelectVariant(variant) => {
                if let Some(entry) = self
                    .catalog
                    .iter()
                    .find(|entry| entry.metadata.id == self.selected)
                    && let Ok(session) = entry.open_variant(&variant)
                {
                    self.session = session;
                    self.selected_variant = variant;
                    self.status = "Variant opened from production fixture factory".into();
                    self.measure_render();
                }
            }
            WorkbenchMessage::SetTheme(theme) => {
                self.apply_configuration(|variant| variant.theme = theme)
            }
            WorkbenchMessage::SetDirection(direction) => self.apply_configuration(|variant| {
                variant.locale.direction = direction;
                variant.locale.id = if direction == FixtureDirection::RightToLeft {
                    "ar"
                } else {
                    "en-US"
                };
            }),
            WorkbenchMessage::SetScale(factor) => self.apply_configuration(|variant| {
                variant.scale = ScalePreset {
                    id: if factor > 1.0 {
                        "150-percent"
                    } else {
                        "100-percent"
                    },
                    factor,
                }
            }),
            WorkbenchMessage::SetControllerFamily(family) => {
                self.apply_configuration(|variant| variant.controller_family = family)
            }
            WorkbenchMessage::SetViewport(width, height) => self.apply_configuration(|variant| {
                variant.viewport = ViewportPreset {
                    id: if width < 600 { "narrow" } else { "wide" },
                    width,
                    height,
                }
            }),
            WorkbenchMessage::ToggleHighContrast(enabled) => self.apply_configuration(|variant| {
                variant.accessibility.high_contrast = enabled;
                variant.theme = if enabled {
                    FixtureTheme::HighContrast
                } else {
                    FixtureTheme::Dark
                };
            }),
            WorkbenchMessage::ToggleReducedEffects(enabled) => {
                self.apply_configuration(|variant| {
                    variant.accessibility.reduced_motion = enabled;
                    variant.accessibility.reduced_transparency = enabled;
                })
            }
            WorkbenchMessage::CompareVariant(variant) => {
                if let Some(entry) = self
                    .catalog
                    .iter()
                    .find(|entry| entry.metadata.id == self.selected)
                    && let Ok(session) = entry.open_variant(&variant)
                {
                    self.comparison_variant = Some(variant);
                    self.comparison_session = Some(session);
                    self.reference_comparison = None;
                    self.status = "Side-by-side deterministic comparison enabled".into();
                }
            }
            WorkbenchMessage::CompareReference(id) => {
                if let Some(asset) = self
                    .session
                    .metadata()
                    .assets
                    .iter()
                    .find(|asset| asset.id == id)
                {
                    match reference_comparison(self.session.as_ref(), asset) {
                        Ok(comparison) => {
                            self.reference_comparison = Some(comparison);
                            self.comparison_session = None;
                            self.comparison_variant = None;
                            self.status = "Admitted visual reference comparison enabled".into();
                        }
                        Err(error) => self.status = format!("Reference comparison failed: {error}"),
                    }
                }
            }
            WorkbenchMessage::SetComparisonMode(mode) => self.comparison_mode = mode,
            WorkbenchMessage::ClearComparison => {
                self.comparison_variant = None;
                self.comparison_session = None;
                self.reference_comparison = None;
                self.status = "Comparison cleared".into();
            }
            WorkbenchMessage::SetModality(modality) => self.modality = modality,
            WorkbenchMessage::Activate => {
                self.status = match self.session.activate(self.modality) {
                    Ok(()) => {
                        let effects = self.session.metadata().simulated_effects;
                        if effects.is_empty() {
                            format!(
                                "Activated via {:?}; no external effect declared",
                                self.modality
                            )
                        } else {
                            self.recorded_effects
                                .extend(effects.iter().copied().map(|effect| RecordedEffect {
                                    fixture: self.selected.clone(),
                                    variant: self.selected_variant.clone(),
                                    modality: self.modality,
                                    effect,
                                }));
                            format!(
                                "Simulated only; declared {:?}; no platform action executed",
                                effects
                            )
                        }
                    }
                    Err(error) => format!("Activation failed: {error}"),
                }
            }
            WorkbenchMessage::Reset => {
                self.session.reset();
                self.measure_render();
                self.status = "Fixture reset".into();
            }
            WorkbenchMessage::CatalogScroll(offset) => self.catalog_scroll = offset,
            WorkbenchMessage::MainScroll(offset) => self.main_scroll = offset,
            WorkbenchMessage::InspectorScroll(offset) => self.inspector_scroll = offset,
            WorkbenchMessage::SelectSemanticNode(id) => {
                self.selected_semantic_node = Some(id);
                self.status = "Selected production semantic geometry".into();
            }
        }
    }

    fn view(&self, context: ViewContext) -> impl nickel_ui::View<Self::Message> {
        let viewport_width = context.viewport.size.width.max(640.0);
        let viewport_height = context.viewport.size.height.max(420.0);
        let compact = viewport_width < 960.0 || viewport_height < 700.0;
        let outer_padding = if compact { 10.0 } else { 16.0 };
        let content_height = (viewport_height - outer_padding * 2.0).max(360.0);
        let sidebar_width = if compact { 250.0 } else { 268.0 };
        let main_width = (viewport_width - outer_padding * 2.0 - sidebar_width - 14.0).max(340.0);
        let catalog = Column::new()
            .gap(8.0)
            .children(self.visible_fixture_ids().into_iter().map(|id| {
                let fixture = self
                    .catalog
                    .iter()
                    .find(|fixture| fixture.metadata.id == id)
                    .expect("visible fixture comes from catalog");
                let metadata = fixture.metadata;
                let selected = id == self.selected;
                Button::new(
                    WorkbenchMessage::Select(id.to_owned()),
                    format!("{}\n{}", metadata.title, metadata.id),
                )
                .id(format!("fixture-{id}"))
                .width(sidebar_width - 32.0)
                .height(52.0)
                .max_lines(2)
                .background(if selected { 0x243957 } else { 0x182131 })
                .border(if selected { 0x69a7ff } else { 0x2b3a50 }, 1.0)
                .focus_border(0x8fc1ff)
                .controller_focus_border(0xffd166)
                .radius(8.0)
                .color(if selected { 0xf5f9ff } else { 0xc8d2e1 })
            }));
        let modes = [
            ActivationVia::Semantic,
            ActivationVia::Pointer,
            ActivationVia::Touch,
            ActivationVia::Keyboard,
            ActivationVia::Controller,
            ActivationVia::Accessibility,
        ];
        let modality_controls = Row::new().gap(6.0).children(modes.into_iter().map(|mode| {
            let selected = mode == self.modality;
            Button::new(WorkbenchMessage::SetModality(mode), modality_label(mode))
                .height(34.0)
                .padding(Insets::symmetric(10.0, 6.0))
                .background(if selected { 0x24528a } else { 0x202c3d })
                .border(if selected { 0x76b5ff } else { 0x34445b }, 1.0)
                .focus_border(0x8fc1ff)
                .controller_focus_border(0xffd166)
                .radius(7.0)
                .color(0xf1f6fc)
        }));
        let inspector =
            Column::new()
                .gap(6.0)
                .children(self.session.semantic_nodes().into_iter().map(|node| {
                    let name = node.name.as_deref().unwrap_or("Unnamed node");
                    let selected = self.selected_semantic_node.as_deref() == Some(node.id.as_str());
                    Button::new(
                        WorkbenchMessage::SelectSemanticNode(node.id.as_str().to_owned()),
                        format!(
                            "{} · {}\n{} • {}\ngeometry=({:.0},{:.0}) {:.0}×{:.0}",
                            role_label(node.role),
                            name,
                            action_labels(&node.actions),
                            node.id.as_str(),
                            node.bounds.origin.x,
                            node.bounds.origin.y,
                            node.bounds.size.width,
                            node.bounds.size.height,
                        ),
                    )
                    .max_lines(3)
                    .padding(Insets::symmetric(10.0, 7.0))
                    .background(if selected { 0x49333a } else { 0x17202e })
                    .border(if selected { 0xff6b6b } else { 0x29384d }, 1.0)
                    .radius(7.0)
                }));
        let metadata = self.session.metadata();
        let variant = self.session.variant();
        let variant_controls =
            Row::new()
                .gap(6.0)
                .children(metadata.variants.iter().map(|preset| {
                    let selected = preset.id == self.selected_variant;
                    Button::new(
                        WorkbenchMessage::SelectVariant(preset.id.to_owned()),
                        preset.title,
                    )
                    .height(32.0)
                    .background(if selected { 0x24528a } else { 0x202c3d })
                    .border(if selected { 0x76b5ff } else { 0x34445b }, 1.0)
                    .color(0xf1f6fc)
                }));
        let configuration_controls = Column::new()
            .gap(6.0)
            .child(
                Row::new()
                    .gap(6.0)
                    .child(Button::new(
                        WorkbenchMessage::SetViewport(420, 720),
                        "Narrow 420×720",
                    ))
                    .child(Button::new(
                        WorkbenchMessage::SetViewport(900, 620),
                        "Wide 900×620",
                    ))
                    .child(Button::new(WorkbenchMessage::SetScale(1.0), "Scale 100%"))
                    .child(Button::new(WorkbenchMessage::SetScale(1.5), "Scale 150%")),
            )
            .child(
                Row::new()
                    .gap(6.0)
                    .child(Button::new(
                        WorkbenchMessage::SetTheme(FixtureTheme::Light),
                        "Light",
                    ))
                    .child(Button::new(
                        WorkbenchMessage::SetTheme(FixtureTheme::Dark),
                        "Dark",
                    ))
                    .child(Button::new(
                        WorkbenchMessage::SetDirection(FixtureDirection::LeftToRight),
                        "LTR / en-US",
                    ))
                    .child(Button::new(
                        WorkbenchMessage::SetDirection(FixtureDirection::RightToLeft),
                        "RTL / ar",
                    )),
            )
            .child(
                Row::new()
                    .gap(6.0)
                    .child(Button::new(
                        WorkbenchMessage::SetControllerFamily(nickel_ui::ControllerFamily::Generic),
                        "Generic controller",
                    ))
                    .child(Button::new(
                        WorkbenchMessage::SetControllerFamily(nickel_ui::ControllerFamily::Xbox),
                        "Xbox",
                    ))
                    .child(Button::new(
                        WorkbenchMessage::ToggleHighContrast(!variant.accessibility.high_contrast),
                        "Toggle contrast",
                    ))
                    .child(Button::new(
                        WorkbenchMessage::ToggleReducedEffects(
                            !(variant.accessibility.reduced_motion
                                && variant.accessibility.reduced_transparency),
                        ),
                        "Toggle reduced effects",
                    )),
            );
        let comparison_controls = Row::new().gap(6.0).children(
            metadata
                .variants
                .iter()
                .filter(|preset| preset.id != self.selected_variant)
                .map(|preset| {
                    Button::new(
                        WorkbenchMessage::CompareVariant(preset.id.to_owned()),
                        format!("Compare {}", preset.title),
                    )
                    .height(32.0)
                })
                .chain(self.comparison_variant.as_ref().map(|_| {
                    Button::new(WorkbenchMessage::ClearComparison, "Clear comparison").height(32.0)
                }))
                .chain(metadata.assets.iter().map(|asset| {
                    Button::new(
                        WorkbenchMessage::CompareReference(asset.id.to_owned()),
                        "Compare admitted reference",
                    )
                    .height(32.0)
                }))
                .chain(
                    self.reference_comparison
                        .as_ref()
                        .into_iter()
                        .flat_map(|_| {
                            [
                                Button::new(
                                    WorkbenchMessage::SetComparisonMode(ComparisonMode::SideBySide),
                                    "Side by side",
                                )
                                .height(32.0),
                                Button::new(
                                    WorkbenchMessage::SetComparisonMode(
                                        ComparisonMode::OverlayDifference,
                                    ),
                                    "Difference overlay",
                                )
                                .height(32.0),
                                Button::new(WorkbenchMessage::ClearComparison, "Clear comparison")
                                    .height(32.0),
                            ]
                        }),
                ),
        );
        let (preview_width, preview_height) = self.session.surface_size();
        let preview_max_width = (main_width - 48.0).max(240.0);
        let preview_max_height = if compact { 190.0 } else { 300.0 };
        let preview_scale = (preview_max_width / preview_width as f32)
            .min(preview_max_height / preview_height as f32)
            .min(1.0);
        let primary_preview = Image::new(
            1,
            session_raster_highlight(
                self.session.as_ref(),
                self.selected_semantic_node.as_deref(),
            ),
        )
        .fit(ImageFit::Contain)
        .width(preview_width as f32 * preview_scale)
        .height(preview_height as f32 * preview_scale);
        let preview_content: AnyView<WorkbenchMessage> =
            if let Some(reference) = &self.reference_comparison {
                let image = match self.comparison_mode {
                    ComparisonMode::SideBySide => reference.reference.clone(),
                    ComparisonMode::OverlayDifference => reference.difference.clone(),
                };
                let width = image.width();
                let height = image.height();
                let scale = ((preview_max_width / 2.0) / width as f32)
                    .min(preview_max_height / height as f32)
                    .min(1.0);
                AnyView::new(
                    Row::new().gap(10.0).child(primary_preview).child(
                        Image::new(3, image)
                            .fit(ImageFit::Contain)
                            .width(width as f32 * scale)
                            .height(height as f32 * scale),
                    ),
                )
            } else if let Some(comparison) = &self.comparison_session {
                let (width, height) = comparison.surface_size();
                let scale = ((preview_max_width / 2.0) / width as f32)
                    .min(preview_max_height / height as f32)
                    .min(1.0);
                AnyView::new(
                    Row::new().gap(10.0).child(primary_preview).child(
                        Image::new(2, session_raster(comparison.as_ref()))
                            .fit(ImageFit::Contain)
                            .width(width as f32 * scale)
                            .height(height as f32 * scale),
                    ),
                )
            } else {
                AnyView::new(primary_preview)
            };
        let inspection = self.session.inspect();
        let accessibility_count = self.session.accessibility_nodes().len();
        let semantic_nodes = self.session.semantic_nodes();
        let clipped_nodes = semantic_nodes
            .iter()
            .filter(|node| {
                node.bounds.origin.x < 0.0
                    || node.bounds.origin.y < 0.0
                    || node.bounds.origin.x + node.bounds.size.width > preview_width as f32
                    || node.bounds.origin.y + node.bounds.size.height > preview_height as f32
            })
            .count();
        let scroll_owners = semantic_nodes
            .iter()
            .filter(|node| node.actions.contains(&ActionKind::Scroll))
            .map(|node| node.id.as_str())
            .collect::<Vec<_>>();
        let inspection_summary = format!(
            "focus={:?} · capture={:?} · controller={:?} · scope={:?} · editing={} · modality={:?} · overlay={:?}",
            inspection.keyboard_focus,
            inspection.pointer_capture,
            inspection.controller_target,
            inspection.controller_scope,
            inspection.controller_editing,
            inspection.modality,
            inspection.open_overlay,
        );
        Container::new()
            .width(viewport_width)
            .height(viewport_height)
            .background(0x0d131d)
            .padding(Insets::all(outer_padding))
            .child(
                Row::new()
                    .gap(14.0)
                    .child(
                        Container::new()
                            .width(268.0)
                            .width(sidebar_width)
                            .height(content_height)
                            .padding(Insets::all(16.0))
                            .gap(12.0)
                            .background(0x131c29)
                            .border(0x26364b, 1.0)
                            .radius(12.0)
                            .child(
                                Text::new("UI Workbench")
                                    .scale(2.4)
                                    .bold(true)
                                    .color(0xf2f6fb),
                            )
                            .child(Text::new("Fixture catalog").color(0x93a7bf))
                            .child(
                                Container::new()
                                    .padding(Insets::symmetric(10.0, 9.0))
                                    .background(0x0c121b)
                                    .border(0x40536c, 1.0)
                                    .radius(7.0)
                                    .child(
                                        TextField::on_change_with_placeholder(
                                            &self.query,
                                            "Search fixtures…",
                                            WorkbenchMessage::Query,
                                        )
                                        .id("catalog-search")
                                        .color(0xdce5f1),
                                    ),
                            )
                            .child(
                                VerticalScroll::new(
                                    WorkbenchMessage::CatalogScroll(self.catalog_scroll),
                                    self.catalog_scroll,
                                )
                                .on_scroll(WorkbenchMessage::CatalogScroll)
                                .controlled(true)
                                .height((content_height - 132.0).max(180.0))
                                .child(catalog),
                            ),
                    )
                    .child(
                        VerticalScroll::new(
                            WorkbenchMessage::MainScroll(self.main_scroll),
                            self.main_scroll,
                        )
                        .on_scroll(WorkbenchMessage::MainScroll)
                        .controlled(true)
                        .width(main_width)
                        .height(content_height)
                        .child(Column::new()
                            .width(main_width)
                            .gap(12.0)
                            .child(
                                Container::new()
                                    .min_height(82.0)
                                    .padding(Insets::all(14.0))
                                    .gap(8.0)
                                    .background(0x131c29)
                                    .border(0x26364b, 1.0)
                                    .radius(12.0)
                                    .child(
                                        Text::new(metadata.title)
                                            .scale(2.2)
                                            .bold(true)
                                            .color(0xf4f7fb),
                                    )
                                    .child(Text::new(metadata.id).color(0x77b4ff))
                                    .child(Text::new(metadata.description).color(0xa8b6c8))
                                    .child(
                                        Text::new(format!(
                                            "{}:{} · {} · {:?} · {} · {:.1}x · {:?}",
                                            metadata.source.file,
                                            metadata.source.line,
                                            variant.viewport.id,
                                            variant.theme,
                                            variant.locale.id,
                                            variant.scale.factor,
                                            variant.controller_family,
                                        ))
                                        .color(0x91a2b8)
                                        .scale(1.45),
                                    )
                                    .child(variant_controls)
                                    .child(configuration_controls)
                                    .child(comparison_controls),
                            )
                            .child(
                                Container::new()
                                    .padding(Insets::all(12.0))
                                    .gap(8.0)
                                    .background(0x111925)
                                    .border(0x33465f, 1.0)
                                    .radius(12.0)
                                    .child(
                                        Text::new(format!(
                                            "Preview  ·  {} × {}",
                                            preview_width, preview_height
                                        ))
                                        .bold(true)
                                        .color(0xdce5f1),
                                    )
                                    .child(
                                        Container::new()
                                            .padding(Insets::all(10.0))
                                            .background(0x080c12)
                                            .border(0x3c4d63, 1.0)
                                            .radius(8.0)
                                            .child(preview_content),
                                    ),
                            )
                            .child(
                                Container::new()
                                    .padding(Insets::all(10.0))
                                    .gap(8.0)
                                    .background(0x131c29)
                                    .border(0x26364b, 1.0)
                                    .radius(10.0)
                                    .child(
                                        Text::new("Interaction route").bold(true).color(0xdce5f1),
                                    )
                                    .child(
                                        Text::new(if metadata.simulated_effects.is_empty() {
                                            "Fixture effects: in-process typed messages only".to_owned()
                                        } else {
                                            format!("SIMULATION ONLY · real platform effects disabled · {:?}", metadata.simulated_effects)
                                        })
                                        .color(if metadata.simulated_effects.is_empty() { 0x9fb0c4 } else { 0xffc56b })
                                        .scale(1.55),
                                    )
                                    .child(modality_controls)
                                    .child(
                                        Row::new()
                                            .gap(8.0)
                                            .child(
                                                Button::new(
                                                    WorkbenchMessage::Activate,
                                                    "Activate primary",
                                                )
                                                .background(0x2f78c4)
                                                .border(0x75b8ff, 1.0)
                                                .focus_border(0x8fc1ff)
                                                .controller_focus_border(0xffd166)
                                                .radius(7.0)
                                                .color(0xffffff),
                                            )
                                            .child(
                                                Button::new(
                                                    WorkbenchMessage::Reset,
                                                    "Reset fixture",
                                                )
                                                .background(0x202c3d)
                                                .border(0x40536c, 1.0)
                                                .focus_border(0x8fc1ff)
                                                .controller_focus_border(0xffd166)
                                                .radius(7.0)
                                                .color(0xe7edf5),
                                            ),
                                    )
                                    .child(
                                        Container::new()
                                            .width((main_width - 20.0).max(160.0))
                                            .padding(Insets::symmetric(10.0, 10.0))
                                            .background(0x172b26)
                                            .border(0x2d5c4d, 1.0)
                                            .radius(7.0)
                                            .child(
                                                Text::new(&self.status)
                                                    .color(0x9fe0c1)
                                                    .wrap(true),
                                            ),
                                    )
                                    .child(
                                        Text::new(format!(
                                            "Simulation audit records: {}",
                                            self.recorded_effects.len()
                                        ))
                                        .color(0xaec5df)
                                        .scale(1.5),
                                    )
                                    .child(
                                        Row::new()
                                            .gap(8.0)
                                            .child(metric_badge(
                                                "Frame",
                                                inspection.frame_generation,
                                            ))
                                            .child(metric_badge(
                                                "Nodes",
                                                inspection.resources.node_count as u64,
                                            ))
                                            .child(metric_badge(
                                                "Paint",
                                                inspection.resources.paint_primitive_count as u64,
                                            ))
                                            .child(metric_badge(
                                                "Hits",
                                                inspection.resources.hit_target_count as u64,
                                            )),
                                    )
                                    .child(
                                        Container::new()
                                            .padding(Insets::all(10.0))
                                            .gap(7.0)
                                            .background(0x131c29)
                                            .border(0x26364b, 1.0)
                                            .radius(10.0)
                                            .child(
                                                Text::new("Semantic inspector")
                                                    .bold(true)
                                                    .color(0xdce5f1),
                                            )
                                            .child(
                                                Text::new(format!(
                                                    "{} semantic · {} accessibility · {} retained bytes · {} diagnostics",
                                                    inspection.resources.node_count,
                                                    accessibility_count,
                                                    inspection.resources.estimated_retained_bytes,
                                                    inspection.diagnostics.len() + inspection.overlay_failures.len(),
                                                ))
                                                .color(0x899bb1)
                                                .scale(1.5),
                                            )
                                            .child(
                                                Text::new(inspection_summary)
                                                    .color(0x9fb4ce)
                                                    .scale(1.45)
                                                    .wrap(true),
                                            )
                                            .child(
                                                Text::new(format!(
                                                    "layout={:?} · overlay_failures={:?} · resources={:?}",
                                                    inspection.diagnostics,
                                                    inspection.overlay_failures,
                                                    inspection.resources,
                                                ))
                                                .color(0x8397b2)
                                                .scale(1.3)
                                                .wrap(true),
                                            )
                                            .child(
                                                Text::new(format!(
                                                    "resolved_layout: {} nodes · clipped={} · scroll_owners={:?}",
                                                    semantic_nodes.len(), clipped_nodes, scroll_owners
                                                ))
                                                .color(0x9fb4ce)
                                                .scale(1.3)
                                                .wrap(true),
                                            )
                                            .child(
                                                Text::new(format!(
                                                    "frame_timing: software_render={:.3}ms · presenter_cache={} · retained={} bytes · scratch={} bytes",
                                                    self.last_render_ms,
                                                    SOFTWARE_PRESENTER_CACHE_DIAGNOSTICS,
                                                    inspection.resources.estimated_retained_bytes,
                                                    inspection.resources.retained_build_scratch_bytes,
                                                ))
                                                .color(0x9fb4ce)
                                                .scale(1.3)
                                                .wrap(true),
                                            )
                                            .child(
                                                VerticalScroll::new(
                                                    WorkbenchMessage::InspectorScroll(
                                                        self.inspector_scroll,
                                                    ),
                                                    self.inspector_scroll,
                                                )
                                                .on_scroll(WorkbenchMessage::InspectorScroll)
                                                .controlled(true)
                                                .height(150.0)
                                                .child(inspector),
                                            ),
                                    ),
                            )),
                    ),
            )
    }

    fn title(&self) -> &str {
        "Nickel UI Workbench"
    }
    fn initial_size(&self) -> (u32, u32) {
        (1120, 820)
    }
}

fn session_raster(session: &dyn ErasedFixtureSession) -> Arc<image::RgbaImage> {
    let raster = session.render(1.0);
    Arc::new(
        image::RgbaImage::from_raw(raster.width, raster.height, raster.rgba)
            .expect("host raster dimensions are internally consistent"),
    )
}

fn session_raster_highlight(
    session: &dyn ErasedFixtureSession,
    selected_id: Option<&str>,
) -> Arc<image::RgbaImage> {
    let mut image = Arc::unwrap_or_clone(session_raster(session));
    let Some(bounds) = selected_id.and_then(|id| {
        session
            .semantic_nodes()
            .into_iter()
            .find(|node| node.id.as_str() == id)
            .map(|node| node.bounds)
    }) else {
        return Arc::new(image);
    };
    let left = bounds.origin.x.max(0.0) as u32;
    let top = bounds.origin.y.max(0.0) as u32;
    let right = (bounds.origin.x + bounds.size.width)
        .min(image.width() as f32)
        .ceil() as u32;
    let bottom = (bounds.origin.y + bounds.size.height)
        .min(image.height() as f32)
        .ceil() as u32;
    if right <= left || bottom <= top {
        return Arc::new(image);
    }
    let highlight = image::Rgba([0xff, 0x3b, 0x5c, 0xff]);
    for x in left..right {
        image.put_pixel(x, top, highlight);
        image.put_pixel(x, bottom - 1, highlight);
    }
    for y in top..bottom {
        image.put_pixel(left, y, highlight);
        image.put_pixel(right - 1, y, highlight);
    }
    Arc::new(image)
}

fn reference_comparison(
    session: &dyn ErasedFixtureSession,
    asset: &FixtureAsset,
) -> Result<ReferenceComparison, Box<dyn Error>> {
    validate_fixture_asset(asset)?;
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let reference = image::open(repository.join(asset.path))?.to_rgba8();
    let current = Arc::unwrap_or_clone(session_raster(session));
    let resized = image::imageops::resize(
        &reference,
        current.width(),
        current.height(),
        image::imageops::FilterType::Triangle,
    );
    let mut difference = image::RgbaImage::new(current.width(), current.height());
    for (x, y, pixel) in difference.enumerate_pixels_mut() {
        let left = current.get_pixel(x, y);
        let right = resized.get_pixel(x, y);
        *pixel = image::Rgba([
            left[0].abs_diff(right[0]),
            left[1].abs_diff(right[1]),
            left[2].abs_diff(right[2]),
            0xff,
        ]);
    }
    Ok(ReferenceComparison {
        reference: Arc::new(reference),
        difference: Arc::new(difference),
    })
}

fn modality_label(modality: ActivationVia) -> &'static str {
    match modality {
        ActivationVia::Semantic => "Semantic",
        ActivationVia::Pointer => "Pointer",
        ActivationVia::Touch => "Touch",
        ActivationVia::Keyboard => "Keyboard",
        ActivationVia::Controller => "Controller",
        ActivationVia::Accessibility => "Accessibility",
    }
}

fn role_label(role: Option<SemanticRole>) -> String {
    role.map_or_else(|| "Layout".to_owned(), |role| format!("{role:?}"))
}

fn action_labels(actions: &[ActionKind]) -> String {
    if actions.is_empty() {
        "No actions".to_owned()
    } else {
        actions
            .iter()
            .map(|action| format!("{action:?}"))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn metric_badge(label: &str, value: u64) -> Container<WorkbenchMessage> {
    Container::new()
        .padding(Insets::symmetric(12.0, 7.0))
        .gap(2.0)
        .background(0x17202e)
        .border(0x2c3d53, 1.0)
        .radius(8.0)
        .child(Text::new(label).color(0xa7b5c8).scale(1.45))
        .child(
            Text::new(value.to_string())
                .color(0xf3f7fc)
                .bold(true)
                .scale(2.1),
        )
}

#[derive(Debug)]
struct UsageError(String);

impl fmt::Display for UsageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for UsageError {}

struct CoreFixtureProvider;
impl FixtureProvider for CoreFixtureProvider {
    fn register(
        &self,
        registry: &mut FixtureRegistry,
    ) -> Result<(), nickel_ui_testkit::RegistryError> {
        registry.register::<CounterFixture>()?;
        registry.register::<PrimitivesFixture>()?;
        registry.register::<CollectionStatesFixture>()
    }
}

struct ProductFixtureProvider;
impl FixtureProvider for ProductFixtureProvider {
    fn register(
        &self,
        registry: &mut FixtureRegistry,
    ) -> Result<(), nickel_ui_testkit::RegistryError> {
        registry.register::<MarkdownFixture>()?;
        registry.register::<SettingsWideFixture>()?;
        registry.register::<SettingsNarrowRtlFixture>()?;
        registry.register::<LauncherFixture>()?;
        registry.register::<CodexFixture>()?;
        registry.register::<MenusFixture>()
    }
}

fn registry() -> Result<Vec<FixtureRegistryEntry>, Box<dyn Error>> {
    let mut registry = FixtureRegistry::new();
    registry.register_provider(&CoreFixtureProvider)?;
    registry.register_provider(&ProductFixtureProvider)?;
    #[cfg(feature = "file-provider")]
    registry.register_provider(&nickel_file::FileFixtureProvider)?;
    #[cfg(not(feature = "file-provider"))]
    registry.register_external(&FILE_METADATA, FILE_PROVIDER)?;
    #[cfg(feature = "markdown-viewer-provider")]
    registry.register_provider(&nickel_markdown_ui::MarkdownViewerFixtureProvider)?;
    #[cfg(not(feature = "markdown-viewer-provider"))]
    registry.register_external(&MARKDOWN_VIEWER_METADATA, MARKDOWN_VIEWER_PROVIDER)?;
    #[cfg(feature = "gaze-provider")]
    registry.register_provider(&nickel_gaze::GazeGridFixtureProvider)?;
    #[cfg(not(feature = "gaze-provider"))]
    registry.register_external(&GAZE_METADATA, GAZE_PROVIDER)?;
    #[cfg(feature = "shell-provider")]
    registry.register_provider(&nickel_shell::ShellFixtureProvider)?;
    #[cfg(not(feature = "shell-provider"))]
    {
        registry.register_external(&SHELL_RUNTIME_METADATA, SHELL_RUNTIME_PROVIDER)?;
        registry.register_external(&SHELL_DESKTOP_METADATA, SHELL_DESKTOP_PROVIDER)?;
        registry.register_external(&SHELL_PANEL_METADATA, SHELL_PANEL_PROVIDER)?;
        registry.register_external(&SHELL_NOTIFICATION_METADATA, SHELL_NOTIFICATION_PROVIDER)?;
        registry.register_external(&SHELL_LOCK_METADATA, SHELL_LOCK_PROVIDER)?;
        registry.register_external(&SHELL_SCREENSHOT_METADATA, SHELL_SCREENSHOT_PROVIDER)?;
        registry.register_external(&SHELL_PREVIEW_METADATA, SHELL_PREVIEW_PROVIDER)?;
        registry.register_external(&SHELL_CONTROL_METADATA, SHELL_CONTROL_PROVIDER)?;
        registry.register_external(&SHELL_PROJECT_METADATA, SHELL_PROJECT_PROVIDER)?;
        registry.register_external(&SHELL_SEARCH_METADATA, SHELL_SEARCH_PROVIDER)?;
    }
    fixture_inventory::register(&mut registry)?;
    Ok(registry.finish())
}

fn fixture_entry(id: &str) -> Result<FixtureRegistryEntry, Box<dyn Error>> {
    registry()?
        .into_iter()
        .find(|entry| entry.metadata.id == id)
        .ok_or_else(|| Box::new(UsageError(format!("unknown fixture `{id}`"))) as Box<dyn Error>)
}

fn validate_session(session: &dyn ErasedFixtureSession) -> Result<(), Box<dyn Error>> {
    let metadata = session.metadata();
    let nodes = session.semantic_nodes();
    let accessibility_nodes = session.accessibility_nodes();
    let allows_no_semantics =
        metadata.tags.contains(&"noninteractive") || metadata.tags.contains(&"variant-interactive");
    if nodes.is_empty() && !allows_no_semantics {
        return Err(Box::new(UsageError(format!(
            "fixture `{}` has no semantic nodes",
            metadata.id
        ))));
    }
    if nodes.is_empty()
        && (accessibility_nodes.is_empty()
            || accessibility_nodes.iter().any(|node| node.interactive))
    {
        return Err(Box::new(UsageError(format!(
            "non-action fixture `{}` must expose descriptive accessibility without actions",
            metadata.id
        ))));
    }
    let mut ids = std::collections::BTreeSet::new();
    for node in &nodes {
        if !ids.insert(node.id.as_str()) {
            return Err(Box::new(UsageError(format!(
                "fixture `{}` repeats semantic id `{}`",
                metadata.id,
                node.id.as_str()
            ))));
        }
        if node.role.is_some() && node.name.as_deref().is_some_and(str::is_empty) {
            return Err(Box::new(UsageError(format!(
                "fixture `{}` has an empty accessible name at `{}` ({:?})",
                metadata.id,
                node.id.as_str(),
                node.role
            ))));
        }
    }
    let inspection = session.inspect();
    if !inspection.diagnostics.is_empty() || !inspection.overlay_failures.is_empty() {
        return Err(Box::new(UsageError(format!(
            "fixture `{}` reports diagnostics: layout={:?} overlay={:?}",
            metadata.id, inspection.diagnostics, inspection.overlay_failures
        ))));
    }
    for node in accessibility_nodes {
        if node.interactive && node.label.as_deref().is_none_or(str::is_empty) {
            return Err(Box::new(UsageError(format!(
                "fixture `{}` has unnamed interactive accessibility node `{}`",
                metadata.id,
                node.id.as_str()
            ))));
        }
    }
    let first = session.render(1.0);
    let second = session.render(1.0);
    if first != second || first.rgba.is_empty() {
        return Err(Box::new(UsageError(format!(
            "fixture `{}` is not deterministic",
            metadata.id
        ))));
    }
    Ok(())
}

fn list() -> Result<(), Box<dyn Error>> {
    for fixture in registry()? {
        let metadata = fixture.metadata;
        println!(
            "{}\t{}\t{}",
            metadata.id,
            metadata.title,
            metadata.tags.join(",")
        );
    }
    Ok(())
}

fn metadata_json(id: Option<&str>) -> Result<(), Box<dyn Error>> {
    let entries = registry()?
        .into_iter()
        .filter(|entry| id.is_none_or(|id| entry.metadata.id == id))
        .map(|entry| {
            let metadata = entry.metadata;
            serde_json::json!({
                "id": metadata.id,
                "title": metadata.title,
                "description": metadata.description,
                "tags": metadata.tags,
                "source": { "crate": metadata.source.crate_name, "file": metadata.source.file, "line": metadata.source.line },
                "assets": metadata.assets.iter().map(|asset| serde_json::json!({
                    "id": asset.id, "path": asset.path, "license": asset.license, "sha256": asset.sha256,
                })).collect::<Vec<_>>(),
                "simulated_effects": metadata.simulated_effects.iter().map(|effect| format!("{effect:?}")).collect::<Vec<_>>(),
                "variants": metadata.variants.iter().map(|variant| serde_json::json!({
                    "id": variant.id, "title": variant.title,
                    "viewport": { "id": variant.viewport.id, "width": variant.viewport.width, "height": variant.viewport.height },
                    "theme": format!("{:?}", variant.theme),
                    "locale": { "id": variant.locale.id, "direction": format!("{:?}", variant.locale.direction) },
                    "scale": { "id": variant.scale.id, "factor": variant.scale.factor },
                    "controller_family": format!("{:?}", variant.controller_family),
                    "accessibility": {
                        "id": variant.accessibility.id,
                        "high_contrast": variant.accessibility.high_contrast,
                        "reduced_motion": variant.accessibility.reduced_motion,
                        "reduced_transparency": variant.accessibility.reduced_transparency,
                    },
                })).collect::<Vec<_>>(),
            })
        })
        .collect::<Vec<_>>();
    if let Some(id) = id
        && entries.is_empty()
    {
        return Err(Box::new(UsageError(format!("unknown fixture `{id}`"))));
    }
    println!("{}", serde_json::to_string_pretty(&entries)?);
    Ok(())
}

fn semantic_json(id: &str, variant: Option<&str>) -> Result<(), Box<dyn Error>> {
    let entry = fixture_entry(id)?;
    let session = match variant {
        Some(variant) => entry.open_variant(variant)?,
        None => entry.open(),
    };
    let inspection = session.inspect();
    let document = serde_json::json!({
        "fixture": id,
        "variant": session.variant().id,
        "inspection": {
            "frame_generation": inspection.frame_generation,
            "semantic_generation": inspection.semantic_generation,
            "modality": format!("{:?}", inspection.modality),
            "keyboard_focus": inspection.keyboard_focus.as_ref().map(|id| id.as_str()),
            "controller_target": inspection.controller_target.as_ref().map(|id| id.as_str()),
            "controller_scope": inspection.controller_scope.as_ref().map(|id| id.as_str()),
            "diagnostics": inspection.diagnostics.iter().map(|item| format!("{item:?}")).collect::<Vec<_>>(),
        },
        "semantic": session.semantic_nodes().iter().map(|node| serde_json::json!({
            "id": node.id.as_str(), "parent": node.parent.as_ref().map(|id| id.as_str()),
            "role": node.role.map(|role| format!("{role:?}")), "name": node.name,
            "description": node.description, "enabled": node.enabled, "focused": node.focused,
            "controller_selected": node.controller_selected,
            "bounds": [node.bounds.origin.x, node.bounds.origin.y, node.bounds.size.width, node.bounds.size.height],
            "actions": node.actions.iter().map(|action| format!("{action:?}")).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "accessibility": session.accessibility_nodes().iter().map(|node| serde_json::json!({
            "id": node.id.as_str(), "component": node.component, "label": node.label,
            "description": node.description, "role": node.role, "state": node.state,
            "interactive": node.interactive,
            "bounds": [node.rect.origin.x, node.rect.origin.y, node.rect.size.width, node.rect.size.height],
        })).collect::<Vec<_>>(),
    });
    println!("{}", serde_json::to_string_pretty(&document)?);
    Ok(())
}

fn validate_durable_evidence() -> Result<(), Box<dyn Error>> {
    let evidence: serde_json::Value = serde_json::from_str(UI_EVIDENCE)?;
    if evidence["schema"] != 1 {
        return Err(Box::new(UsageError(
            "unsupported UI evidence schema".into(),
        )));
    }
    let report_hash = format!("{:x}", Sha256::digest(UI_REACHABILITY_REPORT));
    if evidence["reachability"]["sha256"].as_str() != Some(report_hash.as_str()) {
        return Err(Box::new(UsageError(
            "retained reachability report hash drifted".into(),
        )));
    }
    let report: serde_json::Value = serde_json::from_slice(UI_REACHABILITY_REPORT)?;
    let variant_count = report["variants"].as_array().map_or(0, Vec::len);
    if report["schema"] != evidence["reachability"]["report_schema"]
        || Some(variant_count as u64) != evidence["reachability"]["fixture_variants"].as_u64()
        || report["path_count"] != evidence["reachability"]["path_count"]
        || report["reached_count"] != report["path_count"]
        || report["issue_count"] != 0
    {
        return Err(Box::new(UsageError(
            "retained reachability report is incomplete or inconsistent".into(),
        )));
    }
    let performance = &evidence["performance_comparison"];
    let incremental = performance["clean_incremental_ms"]
        .as_f64()
        .ok_or_else(|| UsageError("missing clean incremental evidence".into()))?;
    let matrix = performance["old_launcher_matrix_ms"]
        .as_f64()
        .ok_or_else(|| UsageError("missing old matrix evidence".into()))?;
    if incremental > performance["incremental_budget_ms"].as_f64().unwrap_or(0.0)
        || matrix > performance["full_matrix_budget_ms"].as_f64().unwrap_or(0.0)
        || matrix / incremental
            < performance["minimum_speedup"]
                .as_f64()
                .unwrap_or(f64::INFINITY)
    {
        return Err(Box::new(UsageError(
            "retained performance evidence exceeds its declared budget".into(),
        )));
    }
    for modality in ["pointer", "keyboard", "controller", "accessibility"] {
        let item = &evidence["workbench_live_acceptance"]["modalities"][modality];
        if item["result"] != "passed"
            || item["sha256"].as_str().is_none_or(|hash| hash.len() != 64)
            || item["bytes"].as_u64().unwrap_or(0) == 0
        {
            return Err(Box::new(UsageError(format!(
                "incomplete durable live evidence for {modality}"
            ))));
        }
    }
    Ok(())
}

fn validate() -> Result<(), Box<dyn Error>> {
    let fixtures = registry()?;
    let feedback_budgets = budgets()?;
    nested_runtime_evidence(&feedback_budgets)?;
    if fixtures.is_empty() {
        return Err(Box::new(UsageError("fixture catalog is empty".into())));
    }
    let counter = open::<CounterFixture>();
    let markdown = open::<MarkdownFixture>();
    for (id, issues) in [
        (COUNTER_ID, validate_host(counter.host())),
        (MARKDOWN_ID, validate_host(markdown.host())),
    ] {
        if !issues.is_empty() {
            return Err(Box::new(UsageError(format!(
                "fixture `{id}` validation failed: {issues:?}"
            ))));
        }
    }
    for fixture in &fixtures {
        let metadata = fixture.metadata;
        if metadata.source.crate_name.is_empty()
            || metadata.source.file.is_empty()
            || metadata.source.line == 0
        {
            return Err(Box::new(UsageError(format!(
                "fixture `{}` has incomplete source metadata",
                metadata.id
            ))));
        }
        for asset in metadata.assets {
            if asset.id.is_empty()
                || asset.path.is_empty()
                || asset.license.is_empty()
                || asset.sha256.len() != 64
            {
                return Err(Box::new(UsageError(format!(
                    "fixture `{}` has incomplete deterministic asset metadata",
                    metadata.id
                ))));
            }
            validate_fixture_asset(asset)?;
        }
        if fixture.is_external() {
            continue;
        }
        for variant in metadata.variants {
            let session = fixture.open_variant(variant.id)?;
            validate_session(session.as_ref())?;
        }
    }
    let budgets = budgets()?;
    validate_durable_evidence()?;
    let cache_count = validate_cache_inventory()?;
    let lifecycle_count = validate_cache_lifecycle_matrix(CACHE_INVENTORY, CACHE_LIFECYCLE_MATRIX)?;
    let consumer_count = validate_consumer_inventory()?;
    let inspection = counter.host().inspect();
    if inspection.resources.retained_build_scratch_bytes
        != budgets.lifecycle.retained_build_scratch_bytes
    {
        return Err(Box::new(UsageError(
            "fixture retains build scratch after resolution".into(),
        )));
    }
    println!(
        "validated {} fixture(s), {} cache record(s), {} lifecycle record(s), and {} consumer record(s)",
        fixtures.len(),
        cache_count,
        lifecycle_count,
        consumer_count
    );
    Ok(())
}

fn validate_final_completion() -> Result<(), Box<dyn Error>> {
    validate()?;
    let cache_count = validate_cache_inventory_for_final_completion()?;
    println!("validated {cache_count} final-completion cache record(s)");
    Ok(())
}

fn benchmark(id: &str) -> Result<(), Box<dyn Error>> {
    let command_started = Instant::now();
    let entry = fixture_entry(id)?;
    let budgets = budgets()?;
    let nested = nested_runtime_evidence(&budgets)?;
    let mut semantic = Vec::with_capacity(budgets.focused.samples);
    let mut render = Vec::with_capacity(budgets.focused.samples);
    let mut workbench_open = Vec::with_capacity(budgets.focused.samples);
    let mut warm_frame = Vec::with_capacity(budgets.focused.samples);
    let mut warm_frame_allocations = Vec::with_capacity(budgets.focused.samples);
    let mut input_visible = Vec::with_capacity(budgets.focused.samples);
    let rss_before = linux_rss_bytes();
    for _ in 0..budgets.focused.samples {
        let started = Instant::now();
        let _ = WorkbenchApp::new()?;
        workbench_open.push(started.elapsed().as_secs_f64() * 1000.0);

        let mut session = entry.open();
        let started = Instant::now();
        session.activate(ActivationVia::Semantic)?;
        semantic.push(started.elapsed().as_secs_f64() * 1000.0);

        let started = Instant::now();
        let raster = session.render(1.0);
        if raster.rgba.is_empty() {
            return Err(Box::new(UsageError("headless raster is empty".into())));
        }
        render.push(started.elapsed().as_secs_f64() * 1000.0);

        let (width, height) = session.surface_size();
        let scale = session.variant().scale.factor;
        let mut persistent_renderer = SdlComponentRenderer::new_pixel_buffer(width, height, scale);
        session.render_persistent(&mut persistent_renderer);
        let started = Instant::now();
        let allocations_before = allocation_operations();
        session.render_persistent(&mut persistent_renderer);
        warm_frame_allocations.push(allocation_operations().saturating_sub(allocations_before));
        warm_frame.push(started.elapsed().as_secs_f64() * 1000.0);

        let mut visible_session = entry.open();
        let started = Instant::now();
        visible_session.activate(ActivationVia::Semantic)?;
        let _ = visible_session.render(1.0);
        input_visible.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    let semantic_p95 = p95(&mut semantic);
    let render_p95 = p95(&mut render);
    let workbench_open_p95 = p95(&mut workbench_open);
    let warm_frame_p95 = p95(&mut warm_frame);
    let warm_frame_allocations_p95 = p95_u64(&mut warm_frame_allocations);
    let input_visible_p95 = p95(&mut input_visible);
    let retained_bytes = entry.open().inspect().resources.estimated_retained_bytes;
    let rss_after = linux_rss_bytes();
    let execution = execution_metadata();
    if semantic_p95 > budgets.focused.semantic_scenario_p95_ms
        || render_p95 > budgets.focused.software_render_p95_ms
        || workbench_open_p95 > budgets.focused.workbench_open_p95_ms
        || warm_frame_allocations_p95 > budgets.focused.frame_allocations as u64
        || retained_bytes > budgets.focused.retained_frame_bytes
        || command_started.elapsed().as_secs_f64() * 1000.0 > budgets.focused.hard_command_ms
    {
        return Err(Box::new(UsageError(format!(
            "feedback budget exceeded: semantic_p95={semantic_p95:.3}ms render_p95={render_p95:.3}ms warm_frame_allocations_p95={warm_frame_allocations_p95}"
        ))));
    }
    println!(
        "fixture={} samples={} semantic_p95_ms={:.3} render_p95_ms={:.3} workbench_open_p95_ms={:.3} warm_software_frame_p95_ms={:.3} software_input_to_visible_p95_ms={:.3} retained_frame_bytes={} presenter_cache_growth_bytes=unavailable presenter_cache_growth_reason=software_preview_has_no_gpu_presenter rss_before_bytes={:?} rss_after_bytes={:?} warm_frame_allocations_p95={} allocator_scope=process allocator_metric=allocation_operations nested_warm_present_p95_us={} nested_input_to_visible_p95_us={} nested_retained_presenter_bytes={} nested_hardware_claim=false nested_frame_allocations_scope={} nested_live_samples={} hard_command_ms={:.1} idle_frames={} profile={} rustc={} cpu={} renderer={} scale={}",
        id,
        budgets.focused.samples,
        semantic_p95,
        render_p95,
        workbench_open_p95,
        warm_frame_p95,
        input_visible_p95,
        retained_bytes,
        rss_before,
        rss_after,
        warm_frame_allocations_p95,
        nested.summary.warm_present_p95_us,
        nested.summary.input_to_visible_p95_us,
        nested.summary.retained_presenter_bytes,
        nested.summary.frame_allocations.scope,
        nested
            .samples
            .warm_present_us
            .len()
            .min(nested.samples.input_to_visible_us.len()),
        budgets.focused.hard_command_ms,
        budgets.lifecycle.idle_frames,
        budgets.metadata.build_profile,
        execution.rustc,
        execution.cpu,
        execution.renderer,
        budgets.metadata.scale,
    );
    Ok(())
}

fn feedback_evidence() -> Result<(), Box<dyn Error>> {
    let budgets = budgets()?;
    let measure = |args: &[&str]| -> Result<f64, Box<dyn Error>> {
        let started = Instant::now();
        let status = std::process::Command::new("cargo").args(args).status()?;
        if !status.success() {
            return Err(Box::new(UsageError(format!(
                "feedback command `cargo {}` failed",
                args.join(" ")
            ))));
        }
        Ok(started.elapsed().as_secs_f64() * 1000.0)
    };
    let incremental = measure(&["check", "-p", "nickel-ui-workbench"])?;
    let selected = measure(&[
        "test",
        "-p",
        "nickel-ui-workbench",
        "core_fixture_runs_through_semantic_host_path",
    ])?;
    if incremental > budgets.fast.incremental_compile_ms
        || selected > budgets.fast.selected_unit_test_ms
    {
        return Err(Box::new(UsageError(format!(
            "fast feedback budget exceeded: incremental={incremental:.1}ms selected_test={selected:.1}ms"
        ))));
    }
    let execution = execution_metadata();
    println!(
        "incremental_compile_ms={incremental:.1} selected_unit_test_ms={selected:.1} old_full_matrix_budget_ms={:.1} speedup_floor={:.1}x profile={} rustc={} cpu={} renderer={}",
        budgets.full_visual.full_matrix_ms,
        budgets.full_visual.full_matrix_ms / incremental.max(selected),
        budgets.metadata.build_profile,
        execution.rustc,
        execution.cpu,
        execution.renderer,
    );
    Ok(())
}

fn feedback_full_comparison() -> Result<(), Box<dyn Error>> {
    let budgets = budgets()?;
    let unique = format!(
        "nickel-workbench-clean-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    );
    let isolated_target = std::env::temp_dir().join(unique);
    fs::create_dir(&isolated_target)?;
    let measure =
        |args: &[&str], target: Option<&std::path::Path>| -> Result<f64, Box<dyn Error>> {
            let started = Instant::now();
            let mut command = std::process::Command::new("cargo");
            command.args(args);
            if let Some(target) = target {
                command.env("CARGO_TARGET_DIR", target);
            }
            let status = command.status()?;
            if !status.success() {
                return Err(Box::new(UsageError(format!(
                    "feedback command `cargo {}` failed",
                    args.join(" ")
                ))));
            }
            Ok(started.elapsed().as_secs_f64() * 1000.0)
        };
    let result = (|| {
        let bootstrap_ms = measure(
            &["check", "-p", "nickel-ui-workbench"],
            Some(&isolated_target),
        )?;
        let clean_incremental_ms = measure(
            &["check", "-p", "nickel-ui-workbench"],
            Some(&isolated_target),
        )?;
        let old_matrix_ms = measure(
            &[
                "test",
                "-p",
                "nickel-shell",
                "dashboard_fixture_matrix_rasterizes_without_empty_or_nonfinite_frames",
                "--",
                "--nocapture",
            ],
            None,
        )?;
        if clean_incremental_ms > budgets.fast.incremental_compile_ms {
            return Err(Box::new(UsageError(format!(
                "clean incremental feedback exceeded {:.1}ms: {clean_incremental_ms:.1}ms",
                budgets.fast.incremental_compile_ms
            ))) as Box<dyn Error>);
        }
        if old_matrix_ms > budgets.full_visual.full_matrix_ms {
            return Err(Box::new(UsageError(format!(
                "old launcher matrix exceeded {:.1}ms: {old_matrix_ms:.1}ms",
                budgets.full_visual.full_matrix_ms
            ))) as Box<dyn Error>);
        }
        if clean_incremental_ms * 4.0 >= old_matrix_ms {
            return Err(Box::new(UsageError(format!(
                "focused clean incremental loop is not materially faster: {clean_incremental_ms:.1}ms vs {old_matrix_ms:.1}ms"
            ))) as Box<dyn Error>);
        }
        let execution = execution_metadata();
        println!(
            "clean_bootstrap_ms={bootstrap_ms:.1} clean_incremental_ms={clean_incremental_ms:.1} old_launcher_matrix_ms={old_matrix_ms:.1} measured_speedup={:.1}x rustc={} cpu={} renderer={}",
            old_matrix_ms / clean_incremental_ms,
            execution.rustc,
            execution.cpu,
            execution.renderer,
        );
        Ok(())
    })();
    if isolated_target.starts_with(std::env::temp_dir())
        && isolated_target.file_name().is_some_and(|name| {
            name.to_string_lossy()
                .starts_with("nickel-workbench-clean-")
        })
    {
        fs::remove_dir_all(&isolated_target)?;
    }
    result
}

fn headless_run(id: &str) -> Result<(), Box<dyn Error>> {
    let (inspection, trace_steps) = match id {
        COUNTER_ID => {
            let mut scenario = open::<CounterFixture>();
            scenario.activate(&Selector::role_name(SemanticRole::Button, "Increment"))?;
            (scenario.host().inspect(), scenario.trace().len())
        }
        MARKDOWN_ID => {
            let mut scenario = open::<MarkdownFixture>();
            scenario.activate(&Selector::role_name(SemanticRole::Button, "the guide  ↗"))?;
            if scenario.host_mut().application_mut().opened_link.as_deref() != Some(MARKDOWN_LINK) {
                return Err(Box::new(UsageError(
                    "Markdown link activation changed its destination".into(),
                )));
            }
            (scenario.host().inspect(), scenario.trace().len())
        }
        _ => {
            let mut session = fixture_entry(id)?.open();
            session.activate(ActivationVia::Semantic)?;
            (session.inspect(), 1)
        }
    };
    println!(
        "fixture={} frames={} nodes={} paint={} hits={} retained_bytes={} trace_steps={}",
        id,
        inspection.frame_generation,
        inspection.resources.node_count,
        inspection.resources.paint_primitive_count,
        inspection.resources.hit_target_count,
        inspection.resources.estimated_retained_bytes,
        trace_steps,
    );
    Ok(())
}

fn headless_render(id: &str, output: &str) -> Result<(), Box<dyn Error>> {
    let entry = registry()?
        .into_iter()
        .find(|entry| entry.metadata.id == id)
        .ok_or_else(|| UsageError(format!("unknown fixture `{id}`")))?;
    let session = entry.open();
    let (width, height) = session.surface_size();
    let raster = session.render(1.0);
    image::save_buffer(
        output,
        &raster.rgba,
        raster.width,
        raster.height,
        image::ColorType::Rgba8,
    )?;
    println!("rendered {} to {} ({}x{})", id, output, width, height);
    Ok(())
}

fn headless_render_variant(id: &str, variant: &str, output: &str) -> Result<(), Box<dyn Error>> {
    let session = fixture_entry(id)?.open_variant(variant)?;
    let raster = session.render(1.0);
    image::save_buffer(
        output,
        &raster.rgba,
        raster.width,
        raster.height,
        image::ColorType::Rgba8,
    )?;
    println!(
        "rendered {}:{} to {} ({}x{})",
        id, variant, output, raster.width, raster.height
    );
    Ok(())
}

fn headless_render_workbench(
    output: &str,
    requested_size: Option<(u32, u32)>,
) -> Result<(), Box<dyn Error>> {
    let app = WorkbenchApp::new()?;
    let (width, height) = requested_size.unwrap_or_else(|| app.initial_size());
    let host = UiHost::new(app, width, height);
    let raster = render_host(&host, width, height, 1.0);
    image::save_buffer(
        output,
        &raster.rgba,
        raster.width,
        raster.height,
        image::ColorType::Rgba8,
    )?;
    println!("rendered workbench to {} ({}x{})", output, width, height);
    Ok(())
}

fn headless_trace(id: &str, output: &str) -> Result<(), Box<dyn Error>> {
    let mut session = fixture_entry(id)?.open();
    session.activate(ActivationVia::Semantic)?;
    let document = session.trace_document();
    fs::write(output, serde_json::to_vec_pretty(&document)?)?;
    println!(
        "recorded {} trace step(s) to {}",
        document.steps.len(),
        output
    );
    Ok(())
}

fn replay_trace(input: &str) -> Result<(), Box<dyn Error>> {
    let document: TraceDocument = serde_json::from_slice(&fs::read(input)?)?;
    let mut session = fixture_entry(&document.fixture)?.open();
    session.replay(&document)?;
    println!(
        "replayed fixture={} steps={} frames={}",
        document.fixture,
        document.steps.len(),
        session.inspect().frame_generation,
    );
    Ok(())
}

fn reachability(id: &str, modality: &str) -> Result<(), Box<dyn Error>> {
    let mut session = fixture_entry(id)?.open();
    let via = match modality {
        "controller" => ActivationVia::Controller,
        "pointer" => ActivationVia::Pointer,
        "touch" => ActivationVia::Touch,
        "keyboard" => ActivationVia::Keyboard,
        "accessibility" => ActivationVia::Accessibility,
        "semantic" => ActivationVia::Semantic,
        _ => {
            return Err(Box::new(UsageError(format!(
                "unsupported reachability modality `{modality}`"
            ))));
        }
    };
    session.activate(via)?;
    let detail = format!(
        "default_target=true frames={}",
        session.inspect().frame_generation
    );
    println!("fixture={} modality={} {}", id, modality, detail);
    Ok(())
}

fn reachability_report(output: &str) -> Result<(), Box<dyn Error>> {
    let entries = registry()?;
    let report = audit_registry_reachability(&entries, &ReachabilityPolicy::default())?;
    fs::write(output, report.to_json()?)?;
    if !report.is_complete() {
        return Err(Box::new(UsageError(format!(
            "registry reachability is incomplete; report retained at {output}"
        ))));
    }
    println!(
        "retained reachability for {} fixture variant(s) at {}",
        report.variants.len(),
        output
    );
    Ok(())
}

fn run() -> Result<(), Box<dyn Error>> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    let catalog = registry()?;
    let full_provider_command = matches!(args.as_slice(), [command, flag] if command == "validate" && flag == "--full-providers")
        || matches!(args.as_slice(), [command, ..] if command == "reachability-report");
    let mut provider_features = catalog
        .iter()
        .filter(|entry| {
            entry.is_external()
                && (full_provider_command
                    || args.iter().any(|argument| argument == entry.metadata.id))
        })
        .filter_map(|entry| {
            entry
                .external_provider
                .map(|provider| provider.workbench_feature)
        })
        .collect::<Vec<_>>();
    provider_features.sort_unstable();
    provider_features.dedup();
    if !provider_features.is_empty() {
        return run_external_workbench(&provider_features.join(","), &args);
    }
    match args.as_slice() {
        [] => nickel_ui::run(WorkbenchApp::new()?),
        [command] if command == "native" => nickel_ui::run(WorkbenchApp::new()?),
        [command, fixture] if command == "native" => {
            let mut app = WorkbenchApp::new()?;
            if fixture_entry(fixture).is_err() {
                return Err(Box::new(UsageError(format!("unknown fixture `{fixture}`"))));
            }
            app.select(fixture);
            nickel_ui::run(app)
        }
        [command, flag, query] if command == "native" && flag == "--filter" => {
            let mut app = WorkbenchApp::new()?;
            app.query = query.clone();
            nickel_ui::run(app)
        }
        [command] if command == "list" => list(),
        [command] if command == "validate" => validate(),
        [command, flag] if command == "validate" && flag == "--final-completion" => {
            validate_final_completion()
        }
        [command, flag] if command == "validate" && flag == "--full-providers" => validate(),
        [command] if command == "feedback-evidence" => feedback_evidence(),
        [command, flag] if command == "feedback-evidence" && flag == "--full-comparison" => {
            feedback_full_comparison()
        }
        [command] if command == "metadata-json" => metadata_json(None),
        [command, fixture] if command == "metadata-json" => metadata_json(Some(fixture)),
        [command, fixture] if command == "semantic-json" => semantic_json(fixture, None),
        [command, fixture, variant] if command == "semantic-json" => {
            semantic_json(fixture, Some(variant))
        }
        [command, fixture] if command == "bench" => benchmark(fixture),
        [headless, run, fixture] if headless == "headless" && run == "run" => headless_run(fixture),
        [headless, render, fixture, output] if headless == "headless" && render == "render" => {
            headless_render(fixture, output)
        }
        [headless, render_variant, fixture, variant, output]
            if headless == "headless" && render_variant == "render-variant" =>
        {
            headless_render_variant(fixture, variant, output)
        }
        [headless, render_workbench, output]
            if headless == "headless" && render_workbench == "render-workbench" =>
        {
            headless_render_workbench(output, None)
        }
        [headless, render_workbench, output, width, height]
            if headless == "headless" && render_workbench == "render-workbench" =>
        {
            headless_render_workbench(output, Some((width.parse()?, height.parse()?)))
        }
        [headless, trace, fixture, output] if headless == "headless" && trace == "trace" => {
            headless_trace(fixture, output)
        }
        [replay, input] if replay == "replay" => replay_trace(input),
        [command, output] if command == "reachability-report" => reachability_report(output),
        [command, fixture, flag, modality]
            if command == "reachability" && flag == "--modality" =>
        {
            reachability(fixture, modality)
        }
        _ => Err(Box::new(UsageError(
            "usage: nickel-ui-workbench [native [FIXTURE]|native --filter QUERY|list|validate|feedback-evidence [--full-comparison]|metadata-json [FIXTURE]|semantic-json FIXTURE [VARIANT]|bench FIXTURE|headless run FIXTURE|headless render FIXTURE OUTPUT.png|headless render-variant FIXTURE VARIANT OUTPUT.png|headless render-workbench OUTPUT.png [WIDTH HEIGHT]|headless trace FIXTURE TRACE.json|replay TRACE.json|reachability-report OUTPUT.json|reachability FIXTURE --modality pointer|touch|keyboard|controller|accessibility|semantic]".into(),
        ))),
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("nickel-ui-workbench: {error}");
        std::process::exit(2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(feature = "file-provider"))]
    #[test]
    fn file_fixture_is_discoverable_but_external_in_default_build() {
        let entry = fixture_entry(FILE_ID).expect("external File metadata");
        assert!(entry.is_external());
        assert_eq!(entry.metadata.source.crate_name, "nickel-file");
        assert_eq!(entry.external_provider, Some(FILE_PROVIDER));
        let manifest = include_str!("../Cargo.toml");
        assert!(manifest.contains("nickel-file = { path = \"../nickel-file\", optional = true }"));
        assert!(manifest.contains("file-provider = [\"dep:nickel-file\"]"));
    }

    #[cfg(feature = "file-provider")]
    #[test]
    fn file_fixture_is_linked_only_with_provider_feature() {
        let entry = fixture_entry("file.browser").expect("linked File fixture");
        assert!(!entry.is_external());
        assert_eq!(entry.metadata.source.crate_name, "nickel-file");
    }

    #[cfg(not(feature = "markdown-viewer-provider"))]
    #[test]
    fn markdown_viewer_is_discoverable_but_external_by_default() {
        let entry = fixture_entry("markdown.viewer").expect("external Markdown Viewer metadata");
        assert!(entry.is_external());
        assert_eq!(entry.metadata.source.crate_name, "nickel-markdown-ui");
        assert_eq!(entry.external_provider, Some(MARKDOWN_VIEWER_PROVIDER));
    }

    #[cfg(feature = "markdown-viewer-provider")]
    #[test]
    fn markdown_viewer_links_only_with_provider_feature() {
        let entry = fixture_entry("markdown.viewer").expect("linked Markdown Viewer fixture");
        assert!(!entry.is_external());
        assert_eq!(entry.metadata.source.crate_name, "nickel-markdown-ui");
    }

    #[cfg(not(feature = "gaze-provider"))]
    #[test]
    fn gaze_grid_is_discoverable_but_external_by_default() {
        let entry = fixture_entry("gaze.grid").expect("external Gaze metadata");
        assert!(entry.is_external());
        assert_eq!(entry.metadata.source.crate_name, "nickel-gaze");
        assert_eq!(entry.external_provider, Some(GAZE_PROVIDER));
    }

    #[cfg(feature = "gaze-provider")]
    #[test]
    fn gaze_grid_links_only_with_provider_feature() {
        let entry = fixture_entry("gaze.grid").expect("linked Gaze fixture");
        assert!(!entry.is_external());
        assert_eq!(entry.metadata.source.crate_name, "nickel-gaze");
    }

    #[cfg(not(feature = "shell-provider"))]
    #[test]
    fn shell_fixtures_are_discoverable_but_external_by_default() {
        for id in [
            "shell.runtime",
            "shell.desktop",
            "shell.panel",
            "shell.notification",
            "shell.lock",
            "shell.screenshot",
            "shell.window-preview",
            "shell.control-center",
            "shell.codex-project-menu",
            "shell.launcher-search",
        ] {
            let entry = fixture_entry(id).expect("external shell metadata");
            assert!(entry.is_external(), "{id} must stay lazy by default");
            assert_eq!(entry.metadata.source.crate_name, "nickel-shell");
            assert_eq!(
                entry.external_provider.unwrap().workbench_feature,
                "shell-provider"
            );
        }
    }

    #[cfg(feature = "shell-provider")]
    #[test]
    fn shell_fixtures_link_only_with_provider_feature() {
        for id in ["shell.runtime", "shell.desktop", "shell.launcher-search"] {
            let entry = fixture_entry(id).expect("linked shell fixture");
            assert!(!entry.is_external(), "{id} must be production-backed");
            assert_eq!(entry.metadata.source.crate_name, "nickel-shell");
        }
    }

    #[test]
    fn core_catalog_is_valid_and_stably_identified() {
        let entries = registry().expect("valid catalog");
        assert_eq!(entries.len(), 27);
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.metadata.id)
                .collect::<Vec<_>>(),
            vec![
                CODEX_ID,
                COUNTER_ID,
                "file.browser",
                "gaze.grid",
                "launcher.component-inventory",
                LAUNCHER_ID,
                MARKDOWN_ID,
                "markdown.viewer",
                "settings.component-inventory",
                SETTINGS_NARROW_RTL_ID,
                SETTINGS_WIDE_ID,
                COLLECTION_STATES_ID,
                "shared.custom-paint",
                MENU_ID,
                PRIMITIVES_ID,
                "shared.public-components",
                "shared.semantic-dialog",
                "shell.codex-project-menu",
                "shell.control-center",
                "shell.desktop",
                "shell.launcher-search",
                "shell.lock",
                "shell.notification",
                "shell.panel",
                "shell.runtime",
                "shell.screenshot",
                "shell.window-preview",
            ]
        );
    }

    #[test]
    fn core_fixture_runs_through_semantic_host_path() {
        let mut scenario = open::<CounterFixture>();
        scenario
            .activate(&Selector::id("root/increment"))
            .expect("semantic activation");
        assert_eq!(scenario.host_mut().application_mut().count, 1);
        assert_eq!(scenario.trace().len(), 1);
    }

    #[test]
    fn markdown_fixture_has_semantic_accessibility_parity_and_activates_link() {
        let mut scenario = open::<MarkdownFixture>();
        assert!(validate_host(scenario.host()).is_empty());
        scenario
            .activate(&Selector::role_name(SemanticRole::Button, "the guide  ↗"))
            .expect("semantic link activation");
        assert_eq!(
            scenario.host_mut().application_mut().opened_link.as_deref(),
            Some(MARKDOWN_LINK)
        );
        assert_eq!(scenario.trace().len(), 1);
    }

    #[test]
    fn markdown_fixture_exercises_headless_raster_path() {
        let scenario = open::<MarkdownFixture>();
        let (width, height) = MarkdownFixture::surface_size();
        let raster = render_host(scenario.host(), width, height, 1.0);
        assert_eq!(raster.rgba.len(), width as usize * height as usize * 4);
        assert!(
            raster
                .rgba
                .chunks_exact(4)
                .any(|pixel| pixel != [0, 0, 0, 0])
        );
        assert!(scenario.host().inspect().resources.paint_primitive_count > 0);
    }

    #[test]
    fn feedback_budget_manifest_is_versioned_and_positive() {
        let budgets = budgets().expect("valid feedback budgets");
        assert_eq!(budgets.version, 1);
        assert!(budgets.focused.semantic_scenario_p95_ms > 0.0);
        assert!(budgets.focused.software_render_p95_ms > 0.0);
        assert_eq!(budgets.lifecycle.idle_frames, 0);
    }

    #[test]
    fn nested_presenter_allocation_evidence_is_explicit_and_completion_ready() {
        let evidence: NestedRuntimeEvidence =
            serde_json::from_str(NESTED_RUNTIME_EVIDENCE).expect("nested evidence schema");
        validate_nested_allocation_evidence(&evidence.summary.frame_allocations)
            .expect("measured presenter evidence is structurally valid");
        assert_eq!(evidence.summary.frame_allocations.scope, "presenter");
        assert_eq!(evidence.summary.frame_allocations.count, Some(0));
        assert_eq!(evidence.summary.frame_allocations.sample_count, 64);
        assert!(evidence.result.frame_allocations);
        nested_runtime_evidence(&budgets().unwrap())
            .expect("nested runtime evidence meets budgets");
    }

    #[test]
    fn cache_inventory_is_machine_readable_unique_and_statused() {
        assert!(validate_cache_inventory().expect("valid cache inventory") >= 10);
    }

    #[test]
    fn every_inventoried_resource_declares_each_lifecycle_boundary() {
        assert_eq!(
            validate_cache_lifecycle_matrix(CACHE_INVENTORY, CACHE_LIFECYCLE_MATRIX)
                .expect("valid cache lifecycle matrix"),
            CACHE_INVENTORY.lines().skip(1).count()
        );
    }

    #[test]
    fn lifecycle_matrix_covers_required_replacement_and_surface_transitions() {
        let rows = CACHE_LIFECYCLE_MATRIX
            .lines()
            .skip(1)
            .map(|line| {
                let columns = line.split('\t').collect::<Vec<_>>();
                (columns[0], columns)
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        let action = |id: &str, boundary: usize| rows[id][boundary];

        assert_eq!(action("native_text_layout", 1), "clear"); // hide
        assert_eq!(action("software_glyph_raster", 2), "clear"); // suspend
        assert_eq!(action("native_image_textures", 3), "drop_owner"); // close
        assert_eq!(action("shell_presenter_pixels", 4), "rebuild"); // output reconnect
        assert_eq!(action("wallpaper_pixels", 5), "clear"); // topology shrink
        assert_eq!(action("launcher_icons", 6), "clear"); // theme
        assert_eq!(action("plain_text_measure", 7), "clear"); // locale
        assert_eq!(action("native_glyph_atlas", 8), "clear"); // font
        assert_eq!(action("shared_decoded_images", 9), "replace"); // application
        assert_eq!(action("markdown_load_workers", 10), "drop_owner"); // fixture
    }

    #[test]
    fn lifecycle_matrix_fails_closed_for_missing_or_unknown_resources() {
        let incomplete = CACHE_LIFECYCLE_MATRIX
            .lines()
            .filter(|line| !line.starts_with("native_image_textures\t"))
            .collect::<Vec<_>>()
            .join("\n");
        let error = validate_cache_lifecycle_matrix(CACHE_INVENTORY, &incomplete)
            .expect_err("missing lifecycle owner must fail validation");
        assert!(error.to_string().contains("native_image_textures"));
    }

    #[test]
    fn cache_inventory_completeness_is_fail_closed() {
        let incomplete = CACHE_INVENTORY
            .lines()
            .filter(|line| !line.starts_with("compositor_cursor_buffers\t"))
            .collect::<Vec<_>>()
            .join("\n");
        let error = validate_cache_inventory_with(&incomplete, CacheInventoryValidation::Routine)
            .expect_err("omitted retained resource must fail validation");
        assert!(error.to_string().contains("compositor_cursor_buffers"));
    }

    #[test]
    fn opaque_dependency_accounting_must_remain_explicit() {
        let dishonest = CACHE_INVENTORY.replacen("opaque_dependency", "0", 1);
        let error = validate_cache_inventory_with(&dishonest, CacheInventoryValidation::Routine)
            .expect_err("opaque dependency accounting must be stated");
        assert!(error.to_string().contains("explicit opaque dependency"));
    }

    #[test]
    fn opaque_admission_fails_closed_without_cardinality_bytes_or_drop_semantics() {
        let admitted = CACHE_INVENTORY
            .lines()
            .find(|line| line.starts_with("software_glyph_raster\t"))
            .expect("opaque admitted row");

        for dishonest in [
            admitted.replacen(
                "one SwashCache per SdlComponentRenderer owner",
                "dependency-owned",
                1,
            ),
            admitted.replacen("opaque_dependency", "0", 1),
            admitted.replace("drop", "release"),
        ] {
            let inventory = CACHE_INVENTORY.replace(admitted, &dishonest);
            assert!(
                validate_cache_inventory_with(&inventory, CacheInventoryValidation::Routine)
                    .is_err(),
                "dishonest opaque admission must fail validation"
            );
        }
    }

    #[test]
    fn resource_admission_requires_ownership_bounds_release_and_authority() {
        let missing_release =
            CACHE_INVENTORY.replacen("release=remove and clear", "lifecycle=remove and clear", 1);
        let error =
            validate_cache_inventory_with(&missing_release, CacheInventoryValidation::Routine)
                .expect_err("resource admission without release evidence must fail");
        assert!(error.to_string().contains("release="));

        let performance_laundering = CACHE_INVENTORY.replacen(
            "shared_decoded_images\tnickel-render-assets/ImageAssetCache\tresource_reuse",
            "shared_decoded_images\tnickel-render-assets/ImageAssetCache\tderived_performance",
            1,
        );
        let error = validate_cache_inventory_with(
            &performance_laundering,
            CacheInventoryValidation::Routine,
        )
        .expect_err("performance-derived retention needs measured admission");
        assert!(error.to_string().contains("performance-derived category"));
    }

    #[test]
    fn cosmic_text_row_requires_process_wide_singleton_evidence() {
        for evidence in [
            "hard process-wide owner cardinality of 1",
            "zero-state handle",
            "process teardown",
            "remain opaque",
        ] {
            let row = CACHE_INVENTORY
                .lines()
                .find(|line| line.starts_with("cosmic_text_font_systems\t"))
                .expect("cosmic-text dependency owner row");
            let dishonest_row = row.replacen(evidence, "unverified", 1);
            let dishonest = CACHE_INVENTORY.replace(row, &dishonest_row);
            validate_cache_inventory_with(&dishonest, CacheInventoryValidation::Routine)
                .expect_err("dependency owner evidence must fail closed");
        }
    }

    #[test]
    fn smithay_opaque_admission_requires_enforced_and_resource_bounds() {
        let row = CACHE_INVENTORY
            .lines()
            .find(|line| line.starts_with("smithay_renderer_internal_caches\t"))
            .expect("Smithay dependency owner row");
        for evidence in [
            "hard admission=1",
            "resource-bounded by active DRM render nodes",
        ] {
            let dishonest_row = row.replacen(evidence, "unverified", 1);
            let dishonest = CACHE_INVENTORY.replace(row, &dishonest_row);
            validate_cache_inventory_with(&dishonest, CacheInventoryValidation::Routine)
                .expect_err("Smithay admission bounds must fail closed");
        }
    }

    #[test]
    fn final_completion_rejects_pending_performance_and_lifecycle_statuses() {
        for status in ["pending_measure", "lifecycle_fixed"] {
            let inventory = CACHE_INVENTORY.replacen("measured_admitted", status, 1);
            let error = validate_cache_inventory_with(
                &inventory,
                CacheInventoryValidation::FinalCompletion,
            )
            .expect_err("provisional status must not satisfy final completion");
            assert!(error.to_string().contains(status));
        }
        assert_eq!(
            validate_cache_inventory_for_final_completion()
                .expect("the checked-in inventory is final-completion ready"),
            31
        );
    }

    #[test]
    fn consumer_inventory_is_dependency_ordered_and_unique() {
        assert_eq!(
            validate_consumer_inventory().expect("valid consumer inventory"),
            21
        );
    }

    #[test]
    fn fixture_application_replacement_stays_within_resource_lifecycle_budgets() {
        let budgets = budgets().expect("valid feedback budgets");
        for entry in registry().expect("valid catalog") {
            if entry.is_external() {
                continue;
            }
            for variant in entry.metadata.variants {
                let mut session = entry
                    .open_variant(variant.id)
                    .expect("advertised variant opens");
                for _generation in 0..4 {
                    let inspection = session.inspect();
                    assert_eq!(
                        inspection.resources.retained_build_scratch_bytes,
                        budgets.lifecycle.retained_build_scratch_bytes,
                        "{}:{} retained frame-build scratch",
                        entry.metadata.id,
                        variant.id
                    );
                    assert!(
                        inspection.resources.estimated_retained_bytes
                            <= budgets.focused.retained_frame_bytes,
                        "{}:{} retained {} frame bytes above the {} byte budget",
                        entry.metadata.id,
                        variant.id,
                        inspection.resources.estimated_retained_bytes,
                        budgets.focused.retained_frame_bytes
                    );
                    session.reset();
                }
                drop(session);
            }
        }
    }

    #[test]
    fn workbench_catalog_search_and_selection_are_deterministic() {
        let mut app = WorkbenchApp::new().expect("workbench app");
        assert_eq!(app.selected, CODEX_ID);
        app.update(WorkbenchMessage::Query("table".into()));
        assert_eq!(app.visible_fixture_ids(), vec![MARKDOWN_ID]);
        app.update(WorkbenchMessage::Select(MARKDOWN_ID.into()));
        assert_eq!(app.selected, MARKDOWN_ID);
        assert!(
            app.session
                .semantic_nodes()
                .iter()
                .any(|node| node.role == Some(SemanticRole::Button)
                    && node.name.as_deref() == Some("the guide  ↗"))
        );
    }

    #[test]
    fn every_advertised_fixture_supports_registry_driven_execution_paths() {
        for entry in registry().expect("valid catalog") {
            let id = entry.metadata.id;
            fixture_entry(id).expect("listed fixture can be looked up");
            if entry.is_external() {
                continue;
            }
            let mut session = entry.open();
            validate_session(session.as_ref()).expect("semantic and deterministic validation");
            let activated = match session.activate(ActivationVia::Semantic) {
                Ok(()) => true,
                Err(nickel_ui_testkit::FixtureSessionError::NoDefaultActivation { .. }) => {
                    let nodes = session.semantic_nodes();
                    if entry.metadata.tags.contains(&"noninteractive") {
                        assert!(
                            nodes.iter().all(|node| node.actions.is_empty()),
                            "noninteractive fixture {id} must not hide actionable semantics"
                        );
                    } else if entry.metadata.tags.contains(&"input-only") {
                        assert!(
                            nodes
                                .iter()
                                .any(|node| node.actions.contains(&ActionKind::SetValue)),
                            "input-only fixture {id} must expose SetValue"
                        );
                        assert!(
                            nodes
                                .iter()
                                .all(|node| !node.actions.contains(&ActionKind::Activate)),
                            "input-only fixture {id} must not omit an available Activate path"
                        );
                    } else if entry.metadata.tags.contains(&"variant-interactive") {
                        assert!(
                            nodes.iter().all(|node| node.actions.is_empty()),
                            "variant-interactive fixture {id} default must not omit an available action"
                        );
                    } else {
                        panic!("interactive fixture {id} must expose a focused execution path");
                    }
                    false
                }
                Err(error) => {
                    panic!("listed fixture has a focused semantic execution path: {error}")
                }
            };
            assert!(session.inspect().frame_generation > 0);
            let first = session.render(1.0);
            let second = session.render(1.0);
            assert!(!first.rgba.is_empty());
            assert_eq!(
                first.rgba, second.rgba,
                "fixture {id} renders deterministically"
            );
            let document = session.trace_document();
            assert_eq!(document.fixture, id);
            assert_eq!(document.steps.is_empty(), !activated);
            let mut replay = entry.open();
            replay
                .replay(&document)
                .expect("advertised fixture supports source-free replay");
        }
    }

    #[test]
    fn representative_controls_are_reachable_across_all_supported_modalities() {
        for via in [
            ActivationVia::Semantic,
            ActivationVia::Pointer,
            ActivationVia::Touch,
            ActivationVia::Keyboard,
            ActivationVia::Controller,
            ActivationVia::Accessibility,
        ] {
            let mut session = fixture_entry(PRIMITIVES_ID)
                .expect("primitive fixture")
                .open();
            session
                .activate(via)
                .expect("modality reaches primary action");
        }
    }

    #[test]
    fn registry_reachability_report_retains_every_advertised_variant() {
        let entries = registry()
            .expect("registry")
            .into_iter()
            .filter(|entry| !entry.is_external())
            .collect::<Vec<_>>();
        let expected = entries
            .iter()
            .map(|entry| entry.metadata.variants.len())
            .sum::<usize>();
        let report = audit_registry_reachability(&entries, &ReachabilityPolicy::default())
            .expect("registry reachability audit");
        assert_eq!(report.variants.len(), expected);
        let json = report.to_json().expect("machine-readable report");
        assert!(json.contains("\"path_count\""));
        assert!(json.contains("\"issue_count\""));
    }

    #[test]
    fn codex_fixture_occupies_readable_preview_bounds() {
        let session = fixture_entry(CODEX_ID).expect("Codex fixture").open();
        let nodes = session.semantic_nodes();
        let composer = nodes
            .iter()
            .find(|node| node.id.as_str().ends_with("/composer"))
            .expect("semantic composer");
        let send = nodes
            .iter()
            .find(|node| node.id.as_str().ends_with("/send"))
            .expect("semantic send button");
        assert!(
            composer.bounds.size.width >= 680.0,
            "composer must span the preview"
        );
        assert!(
            composer.bounds.origin.y >= 280.0,
            "composer must use the lower preview"
        );
        assert!(send.bounds.size.width >= 150.0 && send.bounds.size.height >= 40.0);

        let raster = session.render(1.0);
        let background = [0x10, 0x17, 0x22, 0xff];
        let mut occupied = None::<(u32, u32, u32, u32)>;
        for (index, pixel) in raster.rgba.chunks_exact(4).enumerate() {
            if pixel == background || pixel[3] == 0 {
                continue;
            }
            let x = index as u32 % raster.width;
            let y = index as u32 / raster.width;
            occupied = Some(occupied.map_or((x, y, x, y), |(left, top, right, bottom)| {
                (left.min(x), top.min(y), right.max(x), bottom.max(y))
            }));
        }
        let (left, top, right, bottom) = occupied.expect("visible fixture content");
        assert!(
            right - left >= 680,
            "content must occupy meaningful width: {left}..{right}"
        );
        assert!(
            bottom - top >= 350,
            "content must occupy meaningful height: {top}..{bottom}"
        );
    }

    #[test]
    fn launcher_high_contrast_preserves_project_semantics_and_visible_text() {
        let entry = fixture_entry(LAUNCHER_ID).expect("launcher fixture");
        let wide = entry.open_variant("wide-xbox").expect("wide variant");
        let contrast = entry
            .open_variant("high-contrast-menu")
            .expect("high contrast variant");
        let actionable_names = |session: &dyn ErasedFixtureSession| {
            session
                .semantic_nodes()
                .into_iter()
                .filter(|node| !node.actions.is_empty())
                .filter_map(|node| node.name)
                .collect::<std::collections::BTreeSet<_>>()
        };
        assert_eq!(
            actionable_names(wide.as_ref()),
            actionable_names(contrast.as_ref())
        );

        let raster = contrast.render(1.0);
        for label in ["Nickel project", "See all projects"] {
            let bounds = contrast
                .semantic_nodes()
                .into_iter()
                .find(|node| node.name.as_deref() == Some(label))
                .expect("project action remains semantic")
                .bounds;
            let left = bounds.origin.x.max(0.0) as u32;
            let top = bounds.origin.y.max(0.0) as u32;
            let right = (bounds.origin.x + bounds.size.width).min(raster.width as f32) as u32;
            let bottom = (bounds.origin.y + bounds.size.height).min(raster.height as f32) as u32;
            let visible_pixels = (top..bottom)
                .flat_map(|y| (left..right).map(move |x| (x, y)))
                .filter(|(x, y)| {
                    let offset = ((*y * raster.width + *x) * 4) as usize;
                    raster.rgba[offset..offset + 3] != [0, 0, 0]
                })
                .count();
            assert!(
                visible_pixels >= 20,
                "{label} must remain visibly contrasted, got {visible_pixels} pixels"
            );
        }
    }

    #[test]
    fn workbench_actions_and_reset_refresh_host_owned_inspection() {
        let mut app = WorkbenchApp::new().expect("workbench app");
        let initial_frame = app.session.inspect().frame_generation;
        app.update(WorkbenchMessage::Activate);
        let activated_frame = app.session.inspect().frame_generation;
        assert!(activated_frame > initial_frame);
        assert!(app.session.inspect().resources.node_count > 0);
        app.update(WorkbenchMessage::Reset);
        let reset_frame = app.session.inspect().frame_generation;
        assert_eq!(reset_frame, initial_frame);
        assert_eq!(app.status, "Fixture reset");
    }

    #[test]
    fn independent_configuration_typed_effects_highlight_and_reference_are_real() {
        let mut app = WorkbenchApp::new().expect("workbench app");
        app.select(PRIMITIVES_ID);
        app.update(WorkbenchMessage::SetScale(1.5));
        app.update(WorkbenchMessage::SetDirection(
            FixtureDirection::RightToLeft,
        ));
        assert_eq!(app.session.variant().scale.factor, 1.5);
        assert_eq!(
            app.session.variant().locale.direction,
            FixtureDirection::RightToLeft
        );
        assert_eq!(app.selected_variant, "custom");

        let node = app
            .session
            .semantic_nodes()
            .into_iter()
            .find(|node| node.name.as_deref() == Some("Primary action"))
            .expect("primary semantic node");
        let plain = session_raster_highlight(app.session.as_ref(), None);
        let highlighted = session_raster_highlight(app.session.as_ref(), Some(node.id.as_str()));
        assert_ne!(
            plain, highlighted,
            "geometry selection changes preview only"
        );
        assert!(
            highlighted
                .pixels()
                .any(|pixel| pixel.0 == [0xff, 0x3b, 0x5c, 0xff])
        );

        app.select(LAUNCHER_ID);
        app.update(WorkbenchMessage::Activate);
        assert!(app.recorded_effects.iter().any(|record| {
            record.fixture == LAUNCHER_ID && record.effect == SimulatedEffectKind::Logout
        }));
        let asset = app.session.metadata().assets[0];
        validate_fixture_asset(&asset).expect("manifested reference");
        app.update(WorkbenchMessage::CompareReference(asset.id.into()));
        assert!(app.reference_comparison.is_some());
        app.update(WorkbenchMessage::SetComparisonMode(
            ComparisonMode::OverlayDifference,
        ));
        assert_eq!(app.comparison_mode, ComparisonMode::OverlayDifference);
    }

    #[test]
    fn workbench_headless_render_is_deterministic_and_nonempty() {
        fn render() -> nickel_ui_testkit::HeadlessRaster {
            let app = WorkbenchApp::new().expect("workbench app");
            let (width, height) = app.initial_size();
            render_host(&UiHost::new(app, width, height), width, height, 1.0)
        }

        let first = render();
        let second = render();
        assert_eq!((first.width, first.height), (1120, 820));
        assert_eq!(first, second);
        let colors = first
            .rgba
            .chunks_exact(4)
            .map(|pixel| [pixel[0], pixel[1], pixel[2], pixel[3]])
            .collect::<std::collections::BTreeSet<_>>();
        assert!(
            colors.len() > 12,
            "workbench should render layered surfaces"
        );
        assert!(colors.contains(&[13, 19, 29, 255]), "window background");
        assert!(colors.contains(&[19, 28, 41, 255]), "panel background");
    }

    #[test]
    fn controller_navigation_has_a_visible_focus_affordance() {
        let app = WorkbenchApp::new().expect("workbench app");
        let (width, height) = app.initial_size();
        let mut host = UiHost::new(app, width, height);
        let before = render_host(&host, width, height, 1.0);

        let mut after = before.clone();
        for _ in 0..32 {
            let outcome = host.handle_controller_action(nickel_ui::ControllerAction::Down);
            assert!(outcome.semantic_failures.is_empty());
            after = render_host(&host, width, height, 1.0);
            if after
                .rgba
                .chunks_exact(4)
                .any(|pixel| pixel == [255, 209, 102, 255])
            {
                break;
            }
        }
        assert!(host.inspect().controller_target.is_some());
        assert_ne!(
            after, before,
            "controller focus must change the rendered frame"
        );
        assert!(
            after
                .rgba
                .chunks_exact(4)
                .any(|pixel| pixel == [255, 209, 102, 255]),
            "the controller focus ring must be visibly distinct from selected-state blue"
        );
    }

    #[test]
    fn compact_workbench_viewport_keeps_rows_separate_and_scrollable() {
        fn named<'a>(
            nodes: &'a [nickel_ui::AccessibilityNode],
            name: &str,
        ) -> &'a nickel_ui::AccessibilityNode {
            nodes
                .iter()
                .find(|node| node.label.as_deref() == Some(name))
                .unwrap_or_else(|| panic!("missing semantic node `{name}`"))
        }
        fn bottom(node: &nickel_ui::AccessibilityNode) -> f32 {
            node.rect.origin.y + node.rect.size.height
        }

        let host = UiHost::new(WorkbenchApp::new().expect("workbench app"), 900, 534);
        let nodes = host.accessibility_nodes();
        let title = named(nodes, "UI Workbench");
        let catalog_label = named(nodes, "Fixture catalog");
        let search = named(nodes, "Search fixtures…");
        let selected = nodes
            .iter()
            .find(|node| node.id.as_str().ends_with("/fixture-codex.chat"))
            .expect("selected catalog row");
        assert!(bottom(title) <= catalog_label.rect.origin.y);
        assert!(bottom(catalog_label) <= search.rect.origin.y);
        assert!(bottom(search) <= selected.rect.origin.y);
        assert!(selected.rect.size.height >= 50.0);

        let mut live_scroll_host =
            UiHost::new(WorkbenchApp::new().expect("workbench app"), 900, 534);
        let outcome = live_scroll_host.handle_event(nickel_ui::UiEvent::Scroll {
            point: nickel_ui::Point { x: 650.0, y: 300.0 },
            delta_y: 420.0,
        });
        assert!(
            outcome.changed,
            "wheel input must update the controlled main scroll"
        );
        let live_scrolled_nodes = live_scroll_host.accessibility_nodes();
        let accessibility = named(live_scrolled_nodes, "Accessibility");
        assert!(
            accessibility.rect.origin.y < 534.0 && bottom(accessibility) > 0.0,
            "live scrolling must expose the accessibility interaction route"
        );

        let raster = render_host(&host, 900, 534, 1.0);
        assert_eq!((raster.width, raster.height), (900, 534));
        assert!(
            raster
                .rgba
                .chunks_exact(4)
                .any(|pixel| pixel == [36, 57, 87, 255])
        );

        let mut route_scrolled = WorkbenchApp::new().expect("workbench app");
        route_scrolled.update(WorkbenchMessage::MainScroll(420.0));
        route_scrolled.update(WorkbenchMessage::SetModality(ActivationVia::Accessibility));
        route_scrolled.update(WorkbenchMessage::Activate);
        let route_host = UiHost::new(route_scrolled, 900, 534);
        let route_nodes = route_host.accessibility_nodes();
        let route = named(route_nodes, "Interaction route");
        let semantic = named(route_nodes, "Semantic");
        let reset = named(route_nodes, "Reset fixture");
        assert!(bottom(route) <= semantic.rect.origin.y);
        assert!(semantic.rect.origin.y < 534.0);
        assert!(reset.rect.origin.x + reset.rect.size.width <= 900.0);
        let route_raster = render_host(&route_host, 900, 534, 1.0);
        assert!(
            route_raster
                .rgba
                .chunks_exact(4)
                .skip(895)
                .step_by(900)
                .all(|pixel| !(pixel[1] > 120 && pixel[1] > pixel[0] + 20)),
            "wrapped status must not paint green text through the right viewport edge"
        );

        let mut scrolled = WorkbenchApp::new().expect("workbench app");
        scrolled.update(WorkbenchMessage::MainScroll(640.0));
        let scrolled_host = UiHost::new(scrolled, 900, 534);
        let scrolled = scrolled_host.accessibility_nodes();
        let inspector = named(scrolled, "Semantic inspector");
        let subtitle = scrolled
            .iter()
            .find(|node| {
                node.label
                    .as_deref()
                    .is_some_and(|name| name.contains("retained bytes"))
            })
            .expect("inspector subtitle");
        assert!(bottom(inspector) <= subtitle.rect.origin.y);
    }
}
