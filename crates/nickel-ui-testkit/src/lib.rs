use std::{
    any::type_name,
    collections::{BTreeSet, VecDeque},
    error::Error,
    fmt,
    time::{Duration, Instant},
};

use nickel_input::{DeviceId, EventOrder, InputEvent, TouchEvent, TouchId};
use nickel_ui::{
    AccessibilityNode, ActionKind, Application, Completion, ControllerAction, ControllerFamily,
    HostBatch, HostEvent, HostEventOutcome, HostInspection, InputModality, Point, SemanticAction,
    SemanticNodeSnapshot, SemanticQueryError, SemanticRole, SemanticSelector as ProductionSelector,
    SemanticValueInput, SemanticValueSnapshot, SoftwareRenderer, UiEvent, UiHost, UiId,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HeadlessRaster {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub fn render_host<A: Application>(
    host: &UiHost<A>,
    width: u32,
    height: u32,
    scale: f32,
) -> HeadlessRaster {
    let mut renderer = SoftwareRenderer::new_pixel_buffer(width, height, scale);
    host.render_software(&mut renderer);
    let rgba = renderer
        .pixels()
        .iter()
        .flat_map(|pixel| [pixel.r, pixel.g, pixel.b, pixel.a])
        .collect();
    HeadlessRaster {
        width,
        height,
        rgba,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FixtureSource {
    pub crate_name: &'static str,
    pub file: &'static str,
    pub line: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ViewportPreset {
    pub id: &'static str,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FixtureTheme {
    Light,
    Dark,
    HighContrast,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FixtureDirection {
    LeftToRight,
    RightToLeft,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LocalePreset {
    pub id: &'static str,
    pub direction: FixtureDirection,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScalePreset {
    pub id: &'static str,
    pub factor: f32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AccessibilityPreset {
    pub id: &'static str,
    pub high_contrast: bool,
    pub reduced_motion: bool,
    pub reduced_transparency: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FixtureAsset {
    pub id: &'static str,
    pub path: &'static str,
    pub license: &'static str,
    pub sha256: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SimulatedEffectKind {
    Logout,
    Power,
    PackageMutation,
    FileMutation,
    PrivilegedAction,
    ExternalAccount,
    OpenUrl,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FixtureVariant {
    pub id: &'static str,
    pub title: &'static str,
    pub viewport: ViewportPreset,
    pub theme: FixtureTheme,
    pub locale: LocalePreset,
    pub scale: ScalePreset,
    pub controller_family: ControllerFamily,
    pub accessibility: AccessibilityPreset,
}

pub const DEFAULT_VIEWPORT: ViewportPreset = ViewportPreset {
    id: "default",
    width: 800,
    height: 600,
};
pub const DEFAULT_LOCALE: LocalePreset = LocalePreset {
    id: "en-US",
    direction: FixtureDirection::LeftToRight,
};
pub const DEFAULT_SCALE: ScalePreset = ScalePreset {
    id: "1x",
    factor: 1.0,
};
pub const DEFAULT_ACCESSIBILITY: AccessibilityPreset = AccessibilityPreset {
    id: "default",
    high_contrast: false,
    reduced_motion: false,
    reduced_transparency: false,
};
pub const DEFAULT_VARIANT: FixtureVariant = FixtureVariant {
    id: "default",
    title: "Default",
    viewport: DEFAULT_VIEWPORT,
    theme: FixtureTheme::Dark,
    locale: DEFAULT_LOCALE,
    scale: DEFAULT_SCALE,
    controller_family: ControllerFamily::Generic,
    accessibility: DEFAULT_ACCESSIBILITY,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FixtureMetadata {
    pub id: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub tags: &'static [&'static str],
    pub source: FixtureSource,
    pub variants: &'static [FixtureVariant],
    pub assets: &'static [FixtureAsset],
    pub simulated_effects: &'static [SimulatedEffectKind],
}

pub trait Fixture: Send + Sync + 'static {
    type App: Application;

    fn metadata() -> &'static FixtureMetadata;
    fn create() -> Self::App;
    fn create_variant(variant: &FixtureVariant) -> Self::App {
        let _ = variant;
        Self::create()
    }
    fn surface_size() -> (u32, u32) {
        (800, 600)
    }

    fn default_activation() -> Option<Selector> {
        None
    }

    /// Semantic action exercised by the fixture's default interaction.
    fn default_action() -> ActionKind {
        ActionKind::Activate
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FixtureSessionError {
    NoDefaultActivation {
        fixture_id: &'static str,
    },
    Activation {
        fixture_id: &'static str,
        via: ActivationVia,
        source: ScenarioError,
    },
    UnknownVariant {
        fixture_id: &'static str,
        variant: String,
    },
}

impl fmt::Display for FixtureSessionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoDefaultActivation { fixture_id } => {
                write!(
                    formatter,
                    "fixture `{fixture_id}` has no default activation"
                )
            }
            Self::Activation {
                fixture_id,
                via,
                source,
            } => write!(
                formatter,
                "fixture `{fixture_id}` activation via {via:?} failed: {source}"
            ),
            Self::UnknownVariant {
                fixture_id,
                variant,
            } => {
                write!(
                    formatter,
                    "fixture `{fixture_id}` has no variant `{variant}`"
                )
            }
        }
    }
}

impl Error for FixtureSessionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Activation { source, .. } => Some(source),
            Self::NoDefaultActivation { .. } | Self::UnknownVariant { .. } => None,
        }
    }
}

pub trait ErasedFixtureSession {
    fn metadata(&self) -> &'static FixtureMetadata;
    fn surface_size(&self) -> (u32, u32);
    fn variant(&self) -> &FixtureVariant;
    fn reset(&mut self);
    fn inspect(&self) -> HostInspection;
    fn semantic_nodes(&self) -> Vec<SemanticNodeSnapshot>;
    fn accessibility_nodes(&self) -> Vec<AccessibilityNode>;
    fn render(&self, scale: f32) -> HeadlessRaster;
    /// Render into a caller-owned persistent presenter. The caller controls
    /// renderer construction and can therefore measure a true warm frame
    /// without charging allocation of a returned raster to the frame.
    fn render_persistent(&self, renderer: &mut SoftwareRenderer);
    fn activate(&mut self, via: ActivationVia) -> Result<(), FixtureSessionError>;
    fn trace_document(&self) -> TraceDocument;
    fn replay(&mut self, document: &TraceDocument) -> Result<(), ScenarioError>;
    fn reachability_report(&self, policy: &ReachabilityPolicy) -> ReachabilityReport;
}

struct TypedFixtureSession<F: Fixture> {
    scenario: Scenario<F::App>,
    variant: FixtureVariant,
}

impl<F: Fixture> TypedFixtureSession<F> {
    fn new() -> Self {
        Self::new_variant(F::metadata().variants.first().unwrap_or(&DEFAULT_VARIANT))
    }

    fn new_variant(variant: &FixtureVariant) -> Self {
        let (width, height) = logical_fixture_size(variant);
        let mut scenario = Scenario::new(F::create_variant(variant), width, height);
        scenario.host_mut().set_scale_factor(variant.scale.factor);
        Self {
            scenario,
            variant: *variant,
        }
    }
}

impl<F: Fixture> ErasedFixtureSession for TypedFixtureSession<F> {
    fn metadata(&self) -> &'static FixtureMetadata {
        F::metadata()
    }

    fn surface_size(&self) -> (u32, u32) {
        (self.variant.viewport.width, self.variant.viewport.height)
    }

    fn variant(&self) -> &FixtureVariant {
        &self.variant
    }

    fn reset(&mut self) {
        let (width, height) = logical_fixture_size(&self.variant);
        self.scenario = Scenario::new(F::create_variant(&self.variant), width, height);
        self.scenario
            .host_mut()
            .set_scale_factor(self.variant.scale.factor);
    }

    fn inspect(&self) -> HostInspection {
        self.scenario.host().inspect()
    }

    fn semantic_nodes(&self) -> Vec<SemanticNodeSnapshot> {
        self.scenario.semantic_nodes()
    }

    fn accessibility_nodes(&self) -> Vec<AccessibilityNode> {
        self.scenario.host().accessibility_nodes().to_vec()
    }

    fn render(&self, scale: f32) -> HeadlessRaster {
        render_host(
            self.scenario.host(),
            self.variant.viewport.width,
            self.variant.viewport.height,
            scale * self.variant.scale.factor,
        )
    }

    fn render_persistent(&self, renderer: &mut SoftwareRenderer) {
        self.scenario.host().render_software(renderer);
    }

    fn activate(&mut self, via: ActivationVia) -> Result<(), FixtureSessionError> {
        let fixture_id = F::metadata().id;
        let selector = F::default_activation()
            .ok_or(FixtureSessionError::NoDefaultActivation { fixture_id })?;
        self.scenario
            .invoke_via(via, &selector, F::default_action())
            .map(|_| ())
            .map_err(|source| FixtureSessionError::Activation {
                fixture_id,
                via,
                source,
            })
    }

    fn trace_document(&self) -> TraceDocument {
        self.scenario.trace_document(F::metadata().id)
    }

    fn replay(&mut self, document: &TraceDocument) -> Result<(), ScenarioError> {
        self.scenario.replay(document).map(|_| ())
    }

    fn reachability_report(&self, policy: &ReachabilityPolicy) -> ReachabilityReport {
        audit_reachability(
            || {
                Scenario::new(
                    F::create_variant(&self.variant),
                    self.variant.viewport.width,
                    self.variant.viewport.height,
                )
            },
            policy,
        )
    }
}

fn logical_fixture_size(variant: &FixtureVariant) -> (u32, u32) {
    let scale = variant.scale.factor.max(0.25);
    (
        (variant.viewport.width as f32 / scale).round().max(1.0) as u32,
        (variant.viewport.height as f32 / scale).round().max(1.0) as u32,
    )
}

pub type FixtureFactory = fn() -> Box<dyn ErasedFixtureSession>;
pub type FixtureVariantFactory =
    fn(&str) -> Result<Box<dyn ErasedFixtureSession>, FixtureSessionError>;
pub type FixtureConfigurationFactory = fn(FixtureVariant) -> Box<dyn ErasedFixtureSession>;

/// A fixture whose implementation lives in another Cargo package. The workbench can
/// advertise its metadata without linking that package, then restart itself with the
/// provider feature only when execution is requested.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ExternalFixtureProvider {
    pub protocol_version: u16,
    pub cargo_package: &'static str,
    pub workbench_feature: &'static str,
}

#[derive(Clone, Copy)]
pub struct FixtureRegistryEntry {
    pub metadata: &'static FixtureMetadata,
    pub factory: FixtureFactory,
    pub variant_factory: FixtureVariantFactory,
    pub configuration_factory: FixtureConfigurationFactory,
    pub external_provider: Option<ExternalFixtureProvider>,
}

impl FixtureRegistryEntry {
    pub fn is_external(self) -> bool {
        self.external_provider.is_some()
    }
    pub fn open(self) -> Box<dyn ErasedFixtureSession> {
        (self.factory)()
    }

    pub fn open_variant(
        self,
        variant: &str,
    ) -> Result<Box<dyn ErasedFixtureSession>, FixtureSessionError> {
        (self.variant_factory)(variant)
    }

    pub fn open_configuration(self, variant: FixtureVariant) -> Box<dyn ErasedFixtureSession> {
        (self.configuration_factory)(variant)
    }
}

fn fixture_factory<F: Fixture>() -> Box<dyn ErasedFixtureSession> {
    Box::new(TypedFixtureSession::<F>::new())
}

fn fixture_variant_factory<F: Fixture>(
    variant: &str,
) -> Result<Box<dyn ErasedFixtureSession>, FixtureSessionError> {
    let metadata = F::metadata();
    let variant = metadata
        .variants
        .iter()
        .find(|candidate| candidate.id == variant)
        .ok_or_else(|| FixtureSessionError::UnknownVariant {
            fixture_id: metadata.id,
            variant: variant.to_owned(),
        })?;
    Ok(Box::new(TypedFixtureSession::<F>::new_variant(variant)))
}

fn fixture_configuration_factory<F: Fixture>(
    variant: FixtureVariant,
) -> Box<dyn ErasedFixtureSession> {
    Box::new(TypedFixtureSession::<F>::new_variant(&variant))
}

fn external_fixture_factory() -> Box<dyn ErasedFixtureSession> {
    panic!("external fixture must be delegated to its provider before opening")
}

fn external_fixture_variant_factory(
    _: &str,
) -> Result<Box<dyn ErasedFixtureSession>, FixtureSessionError> {
    panic!("external fixture must be delegated to its provider before opening")
}

fn external_fixture_configuration_factory(_: FixtureVariant) -> Box<dyn ErasedFixtureSession> {
    panic!("external fixture must be delegated to its provider before opening")
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RegistryError {
    EmptyId,
    DuplicateId(String),
    NoVariants(String),
    DuplicateVariant { fixture: String, variant: String },
}

impl fmt::Display for RegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyId => formatter.write_str("fixture id must not be empty"),
            Self::DuplicateId(id) => write!(formatter, "duplicate fixture id `{id}`"),
            Self::NoVariants(id) => write!(formatter, "fixture `{id}` declares no variants"),
            Self::DuplicateVariant { fixture, variant } => {
                write!(formatter, "fixture `{fixture}` repeats variant `{variant}`")
            }
        }
    }
}

impl Error for RegistryError {}

/// A component crate can expose one provider function/zero-sized type and
/// register its fixtures without depending on the workbench binary.
pub trait FixtureProvider {
    fn register(&self, registry: &mut FixtureRegistry) -> Result<(), RegistryError>;
}

#[derive(Default)]
pub struct FixtureRegistry {
    entries: Vec<FixtureRegistryEntry>,
}

impl FixtureRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register<F: Fixture>(&mut self) -> Result<(), RegistryError> {
        let metadata = F::metadata();
        self.validate_metadata(metadata)?;
        self.entries.push(FixtureRegistryEntry {
            metadata,
            factory: fixture_factory::<F>,
            variant_factory: fixture_variant_factory::<F>,
            configuration_factory: fixture_configuration_factory::<F>,
            external_provider: None,
        });
        Ok(())
    }

    fn validate_metadata(&self, metadata: &'static FixtureMetadata) -> Result<(), RegistryError> {
        if metadata.id.trim().is_empty() {
            return Err(RegistryError::EmptyId);
        }
        if self
            .entries
            .iter()
            .any(|entry| entry.metadata.id == metadata.id)
        {
            return Err(RegistryError::DuplicateId(metadata.id.to_owned()));
        }
        if metadata.variants.is_empty() {
            return Err(RegistryError::NoVariants(metadata.id.to_owned()));
        }
        let mut variants = std::collections::BTreeSet::new();
        for variant in metadata.variants {
            if variant.id.trim().is_empty() || !variants.insert(variant.id) {
                return Err(RegistryError::DuplicateVariant {
                    fixture: metadata.id.to_owned(),
                    variant: variant.id.to_owned(),
                });
            }
        }
        Ok(())
    }

    pub fn register_external(
        &mut self,
        metadata: &'static FixtureMetadata,
        provider: ExternalFixtureProvider,
    ) -> Result<(), RegistryError> {
        self.validate_metadata(metadata)?;
        self.entries.push(FixtureRegistryEntry {
            metadata,
            factory: external_fixture_factory,
            variant_factory: external_fixture_variant_factory,
            configuration_factory: external_fixture_configuration_factory,
            external_provider: Some(provider),
        });
        Ok(())
    }

    pub fn register_provider(
        &mut self,
        provider: &impl FixtureProvider,
    ) -> Result<(), RegistryError> {
        provider.register(self)
    }

    pub fn finish(mut self) -> Vec<FixtureRegistryEntry> {
        self.entries.sort_unstable_by_key(|entry| entry.metadata.id);
        self.entries
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Selector {
    Id(UiId),
    KeyedItem { collection: UiId, key: String },
    Role(SemanticRole),
    RoleAndName { role: SemanticRole, name: String },
    Action(ActionKind),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivationVia {
    Pointer,
    Touch,
    Keyboard,
    Controller,
    Accessibility,
    Semantic,
}

impl Selector {
    pub fn id(id: impl Into<UiId>) -> Self {
        Self::Id(id.into())
    }

    pub fn role_name(role: SemanticRole, name: impl Into<String>) -> Self {
        Self::RoleAndName {
            role,
            name: name.into(),
        }
    }

    /// Selects one stable item emitted by a declarative keyed collection.
    /// Resolution is semantic-tree based and rejects duplicate suffixes rather
    /// than relying on a collection item's position or message equality.
    pub fn keyed_item(collection: impl Into<UiId>, key: impl Into<String>) -> Self {
        Self::KeyedItem {
            collection: collection.into(),
            key: key.into(),
        }
    }

    fn production(&self) -> ProductionSelector {
        match self {
            Self::Id(id) => ProductionSelector::Id(id.clone()),
            Self::KeyedItem { collection, key } => ProductionSelector::Id(collection.scoped(key)),
            Self::Role(role) => ProductionSelector::Role(*role),
            Self::RoleAndName { role, name } => ProductionSelector::RoleAndName {
                role: *role,
                name: name.clone(),
            },
            Self::Action(action) => ProductionSelector::Action(*action),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScenarioBudget {
    pub operations: usize,
    pub frames: u64,
    pub trace_steps: usize,
}

impl Default for ScenarioBudget {
    fn default() -> Self {
        Self {
            operations: 64,
            frames: 64,
            trace_steps: 64,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScenarioError {
    MissingTarget {
        selector: String,
        suggestions: Vec<String>,
        topology: Vec<String>,
    },
    AmbiguousTarget {
        selector: String,
        matches: usize,
        candidates: Vec<String>,
    },
    OperationBudgetExceeded,
    FrameBudgetExceeded,
    SemanticActionFailed,
    UnsupportedTraceSchema {
        found: u32,
    },
    ReplayDrift {
        step: usize,
        expected: String,
        actual: String,
    },
    ControllerTargetUnreachable,
    KeyboardTargetUnreachable,
    CompletionFailed {
        id: String,
        detail: String,
    },
    AssertionFailed(ScenarioAssertionFailure),
}

impl fmt::Display for ScenarioError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingTarget {
                selector,
                suggestions,
                topology,
            } => {
                write!(formatter, "semantic selector {selector} matched no target")?;
                if !suggestions.is_empty() {
                    write!(formatter, "; suggestions: {}", suggestions.join(", "))?;
                }
                if !topology.is_empty() {
                    write!(formatter, "; topology: {}", topology.join(" -> "))?;
                }
                Ok(())
            }
            Self::AmbiguousTarget {
                selector,
                matches,
                candidates,
            } => {
                write!(
                    formatter,
                    "semantic selector {selector} matched {matches} targets: {}",
                    candidates.join(", ")
                )
            }
            Self::OperationBudgetExceeded => {
                formatter.write_str("scenario operation budget exceeded")
            }
            Self::FrameBudgetExceeded => formatter.write_str("scenario frame budget exceeded"),
            Self::SemanticActionFailed => formatter.write_str("production semantic action failed"),
            Self::UnsupportedTraceSchema { found } => {
                write!(formatter, "unsupported trace schema {found}")
            }
            Self::ReplayDrift {
                step,
                expected,
                actual,
            } => write!(
                formatter,
                "replay drift at step {step}: expected {expected}; actual {actual}"
            ),
            Self::ControllerTargetUnreachable => {
                formatter.write_str("controller could not reach semantic target")
            }
            Self::KeyboardTargetUnreachable => {
                formatter.write_str("keyboard could not reach semantic target")
            }
            Self::CompletionFailed { id, detail } => {
                write!(formatter, "domain completion `{id}` failed: {detail}")
            }
            Self::AssertionFailed(failure) => write!(formatter, "{failure}"),
        }
    }
}

impl Error for ScenarioError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScenarioAssertionFailure {
    pub assertion: String,
    pub detail: String,
    pub topology: Vec<String>,
    pub suggestions: Vec<String>,
}

impl fmt::Display for ScenarioAssertionFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "scenario assertion `{}` failed: {}",
            self.assertion, self.detail
        )?;
        if !self.suggestions.is_empty() {
            write!(
                formatter,
                "; nearby targets: {}",
                self.suggestions.join(", ")
            )?;
        }
        if !self.topology.is_empty() {
            write!(formatter, "; topology: {}", self.topology.join(" -> "))?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FocusDirection {
    Next,
    Previous,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LayoutRelation {
    Above,
    Below,
    LeftOf,
    RightOf,
    Contains,
    NonOverlapping,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ScenarioOperation {
    Resize {
        width: u32,
        height: u32,
        scale: f32,
    },
    AdvanceTime {
        ticks: u32,
    },
    Focus {
        gained: bool,
    },
    Suspend,
    Close,
    PointerMove {
        target: String,
    },
    PointerActivate {
        target: String,
    },
    PointerContext {
        target: String,
    },
    PointerDrag {
        from: String,
        to: String,
    },
    PointerScroll {
        target: String,
        delta_x: f32,
        delta_y: f32,
    },
    TouchActivate {
        target: String,
        contact: u64,
    },
    TouchContext {
        target: String,
    },
    TouchDrag {
        from: String,
        to: String,
        contact: u64,
    },
    TouchCancel {
        target: String,
        contact: u64,
    },
    KeyboardFocus {
        direction: FocusDirection,
    },
    KeyboardActivate,
    KeyboardContext,
    TextInput {
        text: String,
    },
    ImePreedit {
        text: String,
    },
    ClipboardPaste {
        text: String,
    },
    Controller {
        action: String,
    },
    ControllerSemantic {
        target: String,
        action: TraceAction,
    },
    Accessibility {
        target: String,
        action: TraceAction,
    },
    Semantic {
        target: String,
        action: TraceAction,
    },
    PlatformCapabilityChanged {
        capability: String,
        available: bool,
    },
    DomainCompletion {
        id: String,
        payload_type: String,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScenarioStateSnapshot {
    pub frame: u64,
    pub semantic_digest: String,
    pub accessibility_digest: String,
    pub window_focused: bool,
    pub keyboard_focus: Option<String>,
    pub controller_target: Option<String>,
    pub controller_scope: Option<String>,
    pub open_overlay: Option<String>,
    pub modality: String,
    pub diagnostics: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OperationOutcomeSnapshot {
    pub changed: bool,
    pub invalidation: String,
    pub messages: Vec<MessageEvidenceSnapshot>,
    #[serde(default)]
    pub effects: Vec<EffectEvidenceSnapshot>,
    pub completion_failures: Vec<String>,
    #[serde(default)]
    pub pointer_icon: String,
    #[serde(default)]
    pub text_input_active: bool,
    #[serde(default)]
    pub accessibility_generation: u64,
    #[serde(default)]
    pub events_processed: usize,
    #[serde(default)]
    pub completions_processed: usize,
    #[serde(default)]
    pub rebuilt: bool,
    #[serde(default)]
    pub change_frame_generation: u64,
    #[serde(default)]
    pub change_semantic_generation: u64,
    pub clipboard_text: Option<String>,
    pub semantic_failures: Vec<String>,
    pub global_actions: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MessageEvidenceSnapshot {
    pub type_name: String,
    pub label: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EffectEvidenceSnapshot {
    pub type_name: String,
    pub label: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OperationTraceStep {
    pub operation: ScenarioOperation,
    pub resolved_target: Option<String>,
    pub generated_points: Vec<[f32; 2]>,
    pub before: ScenarioStateSnapshot,
    pub after: ScenarioStateSnapshot,
    pub outcome: OperationOutcomeSnapshot,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InteractionCoverageReport {
    pub schema: u32,
    pub semantic_digest: String,
    pub roles: BTreeSet<String>,
    pub actions: BTreeSet<String>,
    pub state_variants: BTreeSet<String>,
    pub input_routes: BTreeSet<String>,
    pub effects: BTreeSet<String>,
    pub adapter_stages: BTreeSet<String>,
}

impl InteractionCoverageReport {
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TraceStep {
    pub target: UiId,
    pub action: SemanticAction,
    pub via: ActivationVia,
    pub frame_before: u64,
    pub frame_after: u64,
    pub changed: bool,
    pub invalidation: String,
    pub messages: Vec<MessageEvidenceSnapshot>,
    pub effects: Vec<EffectEvidenceSnapshot>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum TraceAction {
    Activate,
    ContextMenu,
    Increment,
    Decrement,
    InvokeSetValue,
    SetNumber(f64),
    SetText(String),
    SetBoolean(bool),
    Expand,
    Collapse,
    Select,
    Dismiss,
    Cancel,
    EnterNavigation,
    ExitNavigation,
    Scroll,
}

impl From<&SemanticAction> for TraceAction {
    fn from(action: &SemanticAction) -> Self {
        match action {
            SemanticAction::Invoke(ActionKind::Activate) => Self::Activate,
            SemanticAction::Invoke(ActionKind::ContextMenu) => Self::ContextMenu,
            SemanticAction::Invoke(ActionKind::Increment) => Self::Increment,
            SemanticAction::Invoke(ActionKind::Decrement) => Self::Decrement,
            SemanticAction::Invoke(ActionKind::Expand) => Self::Expand,
            SemanticAction::Invoke(ActionKind::Collapse) => Self::Collapse,
            SemanticAction::Invoke(ActionKind::Select) => Self::Select,
            SemanticAction::Invoke(ActionKind::Dismiss) => Self::Dismiss,
            SemanticAction::Invoke(ActionKind::Cancel) => Self::Cancel,
            SemanticAction::Invoke(ActionKind::EnterNavigation) => Self::EnterNavigation,
            SemanticAction::Invoke(ActionKind::ExitNavigation) => Self::ExitNavigation,
            SemanticAction::Invoke(ActionKind::Scroll) => Self::Scroll,
            SemanticAction::Invoke(ActionKind::SetValue) => Self::InvokeSetValue,
            SemanticAction::SetValue(SemanticValueInput::Number(value)) => Self::SetNumber(*value),
            SemanticAction::SetValue(SemanticValueInput::Text(value)) => {
                Self::SetText(value.clone())
            }
            SemanticAction::SetValue(SemanticValueInput::Boolean(value)) => {
                Self::SetBoolean(*value)
            }
        }
    }
}

impl TraceAction {
    fn to_semantic(&self) -> SemanticAction {
        match self {
            Self::Activate => SemanticAction::Invoke(ActionKind::Activate),
            Self::ContextMenu => SemanticAction::Invoke(ActionKind::ContextMenu),
            Self::Increment => SemanticAction::Invoke(ActionKind::Increment),
            Self::Decrement => SemanticAction::Invoke(ActionKind::Decrement),
            Self::InvokeSetValue => SemanticAction::Invoke(ActionKind::SetValue),
            Self::SetNumber(value) => SemanticAction::SetValue(SemanticValueInput::Number(*value)),
            Self::SetText(value) => {
                SemanticAction::SetValue(SemanticValueInput::Text(value.clone()))
            }
            Self::SetBoolean(value) => {
                SemanticAction::SetValue(SemanticValueInput::Boolean(*value))
            }
            Self::Expand => SemanticAction::Invoke(ActionKind::Expand),
            Self::Collapse => SemanticAction::Invoke(ActionKind::Collapse),
            Self::Select => SemanticAction::Invoke(ActionKind::Select),
            Self::Dismiss => SemanticAction::Invoke(ActionKind::Dismiss),
            Self::Cancel => SemanticAction::Invoke(ActionKind::Cancel),
            Self::EnterNavigation => SemanticAction::Invoke(ActionKind::EnterNavigation),
            Self::ExitNavigation => SemanticAction::Invoke(ActionKind::ExitNavigation),
            Self::Scroll => SemanticAction::Invoke(ActionKind::Scroll),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TraceStepRecord {
    pub target: String,
    pub action: TraceAction,
    pub via: ActivationVia,
    pub frame_delta: u64,
    pub changed: bool,
    #[serde(default)]
    pub invalidation: String,
    #[serde(default)]
    pub messages: Vec<MessageEvidenceSnapshot>,
    #[serde(default)]
    pub effects: Vec<EffectEvidenceSnapshot>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TraceDocument {
    pub schema: u32,
    pub fixture: String,
    pub viewport: [u32; 2],
    pub initial_semantic_digest: String,
    pub final_semantic_digest: String,
    pub steps: Vec<TraceStepRecord>,
    #[serde(default)]
    pub operations: Vec<OperationTraceStep>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ControllerRoute {
    pub target: UiId,
    pub actions: Vec<ControllerAction>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PointerRoute {
    pub target: UiId,
    pub point: Point,
    pub press_frame: u64,
    pub release_frame: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeyboardRoute {
    pub target: UiId,
    pub focus_steps: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ValidationIssue {
    MissingAccessibility { id: UiId },
    MissingAccessibilityName { id: UiId },
    RoleMismatch { id: UiId },
    NameMismatch { id: UiId },
    DescriptionMismatch { id: UiId },
    ControlsMismatch { id: UiId },
    UnreconciledState { state: &'static str, id: UiId },
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReachabilityModality {
    Keyboard,
    Controller,
    Accessibility,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReachabilityPolicy {
    pub modalities: BTreeSet<ReachabilityModality>,
    pub maximum_path_length: usize,
    pub maximum_state_count: usize,
    pub wall_time_ms: u64,
    pub require_semantic_change: bool,
}

impl Default for ReachabilityPolicy {
    fn default() -> Self {
        Self {
            modalities: [
                ReachabilityModality::Keyboard,
                ReachabilityModality::Controller,
                ReachabilityModality::Accessibility,
            ]
            .into_iter()
            .collect(),
            maximum_path_length: 128,
            maximum_state_count: 512,
            wall_time_ms: 3_000,
            require_semantic_change: false,
        }
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReachabilityIssueKind {
    Unreachable,
    ScopeLeak,
    Cycle,
    ExcessivePathLength,
    AdvertisedButIgnored,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReachabilityIssue {
    pub kind: ReachabilityIssueKind,
    pub target: String,
    pub action: String,
    pub modality: ReachabilityModality,
    pub detail: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReachabilityPath {
    pub target: String,
    pub action: String,
    pub modality: ReachabilityModality,
    pub steps: Vec<String>,
    pub reached: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReachabilityObservation {
    pub steps: Vec<String>,
    pub failure: Option<String>,
    pub current_target: Option<String>,
    pub current_semantic_ids: BTreeSet<String>,
    pub semantic_changed: bool,
}

pub fn classify_reachability_observation(
    target: &str,
    action: ActionKind,
    modality: ReachabilityModality,
    _declared_ids: &BTreeSet<String>,
    policy: &ReachabilityPolicy,
    observation: &ReachabilityObservation,
) -> Vec<ReachabilityIssue> {
    let issue = |kind, detail| ReachabilityIssue {
        kind,
        target: target.to_owned(),
        action: format!("{action:?}"),
        modality,
        detail,
    };
    let mut issues = Vec::new();
    if let Some(failure) = &observation.failure {
        issues.push(issue(ReachabilityIssueKind::Unreachable, failure.clone()));
        return issues;
    }
    if observation.steps.len() > policy.maximum_path_length {
        issues.push(issue(
            ReachabilityIssueKind::ExcessivePathLength,
            format!(
                "{} steps exceeds {}",
                observation.steps.len(),
                policy.maximum_path_length
            ),
        ));
    }
    let unique = observation.steps.iter().collect::<BTreeSet<_>>();
    if unique.len() != observation.steps.len() {
        issues.push(issue(
            ReachabilityIssueKind::Cycle,
            "production route revisited an action state".into(),
        ));
    }
    // A successful topology-changing action may intentionally replace the
    // activated node (for example, New Tab rekeys anonymous tab paths). The
    // route has already proven the transition; stale ownership is only a leak
    // when the semantic topology itself did not reconcile.
    if !observation.semantic_changed
        && observation
            .current_target
            .as_ref()
            .is_some_and(|current| !observation.current_semantic_ids.contains(current))
    {
        issues.push(issue(
            ReachabilityIssueKind::ScopeLeak,
            "active target escaped or survived removal from semantic topology".into(),
        ));
    }
    if policy.require_semantic_change && !observation.semantic_changed {
        issues.push(issue(
            ReachabilityIssueKind::AdvertisedButIgnored,
            "route succeeded without changing semantic state".into(),
        ));
    }
    issues
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReachabilityReport {
    pub schema: u32,
    pub semantic_digest: String,
    pub paths: Vec<ReachabilityPath>,
    pub issues: Vec<ReachabilityIssue>,
}

impl ReachabilityReport {
    pub fn is_complete(&self) -> bool {
        self.issues.is_empty() && self.paths.iter().all(|path| path.reached)
    }

    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FixtureVariantReachability {
    pub fixture: String,
    pub variant: String,
    pub report: ReachabilityReport,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RegistryReachabilityReport {
    pub schema: u32,
    pub variants: Vec<FixtureVariantReachability>,
    pub external_provider_count: usize,
    pub path_count: usize,
    pub reached_count: usize,
    pub issue_count: usize,
}

impl RegistryReachabilityReport {
    pub fn is_complete(&self) -> bool {
        self.external_provider_count == 0
            && self.issue_count == 0
            && self.path_count == self.reached_count
    }

    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}

pub fn audit_registry_reachability(
    entries: &[FixtureRegistryEntry],
    policy: &ReachabilityPolicy,
) -> Result<RegistryReachabilityReport, FixtureSessionError> {
    let mut variants = Vec::new();
    let external_provider_count = entries.iter().filter(|entry| entry.is_external()).count();
    for entry in entries {
        if entry.is_external() {
            continue;
        }
        for variant in entry.metadata.variants {
            let session = entry.open_variant(variant.id)?;
            variants.push(FixtureVariantReachability {
                fixture: entry.metadata.id.to_owned(),
                variant: variant.id.to_owned(),
                report: session.reachability_report(policy),
            });
        }
    }
    let path_count = variants
        .iter()
        .map(|variant| variant.report.paths.len())
        .sum();
    let reached_count = variants
        .iter()
        .flat_map(|variant| &variant.report.paths)
        .filter(|path| path.reached)
        .count();
    let issue_count = variants
        .iter()
        .map(|variant| variant.report.issues.len())
        .sum();
    Ok(RegistryReachabilityReport {
        schema: 2,
        variants,
        external_provider_count,
        path_count,
        reached_count,
        issue_count,
    })
}

pub fn audit_reachability<A: Application>(
    factory: impl Fn() -> Scenario<A>,
    policy: &ReachabilityPolicy,
) -> ReachabilityReport {
    let initial = factory();
    let initial_nodes = initial.semantic_nodes();
    let initial_semantic_digest = semantic_digest(&initial_nodes);
    let declared_ids = initial_nodes
        .iter()
        .map(|node| node.id.as_str().to_owned())
        .collect::<BTreeSet<_>>();
    let mut paths = Vec::new();
    let mut issues = Vec::new();
    let mut audited = BTreeSet::new();
    let wall_ceiling = Duration::from_millis(policy.wall_time_ms);
    let controller_targets = initial_nodes
        .iter()
        .filter(|node| {
            node.enabled
                && node.role != Some(SemanticRole::ScrollBar)
                && node.actions.iter().any(|action| {
                    action_supported_by_modality(*action, ReachabilityModality::Controller)
                })
        })
        .map(|node| node.id.as_str().to_owned())
        .collect::<BTreeSet<_>>();
    let controller_routes = policy
        .modalities
        .contains(&ReachabilityModality::Controller)
        .then(|| explore_controller_routes(&factory, &controller_targets, policy));
    for node in initial_nodes
        .iter()
        .filter(|node| node.enabled)
        .filter(|node| !node.actions.is_empty())
    {
        for action in &node.actions {
            for modality in policy
                .modalities
                .iter()
                .filter(|modality| action_supported_by_modality(*action, **modality))
            {
                audited.insert(format!("{}|{action:?}|{modality:?}", node.id.as_str()));
                let mut scenario = factory();
                let selector = Selector::id(node.id.clone());
                let before = semantic_digest(&scenario.semantic_nodes());
                let result = if *modality == ReachabilityModality::Controller {
                    if node.role == Some(SemanticRole::ScrollBar)
                        && matches!(action, ActionKind::Increment | ActionKind::Decrement)
                    {
                        route_controller_scroll(&factory, node, *action, policy.maximum_path_length)
                            .map(|(reached, steps)| {
                                scenario = reached;
                                steps
                            })
                    } else {
                        controller_routes
                            .as_ref()
                            .expect("controller routes exist when modality is enabled")
                            .routes
                            .get(node.id.as_str())
                            .ok_or_else(|| {
                                controller_routes
                                    .as_ref()
                                    .and_then(|routes| routes.failure.clone())
                                    .unwrap_or_else(|| {
                                        format!("controller BFS did not reach {}", node.id.as_str())
                                    })
                            })
                            .and_then(|prefix| {
                                replay_controller_route(&factory, prefix, node, *action).map(
                                    |(reached, steps)| {
                                        scenario = reached;
                                        steps
                                    },
                                )
                            })
                    }
                } else {
                    route_action(
                        &mut scenario,
                        &selector,
                        *action,
                        *modality,
                        policy.maximum_path_length,
                        Instant::now(),
                        wall_ceiling,
                    )
                };
                let steps = result
                    .as_ref()
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .enumerate()
                    .map(|(index, step)| format!("{index}:{step}"))
                    .collect::<Vec<_>>();
                let reached = result.is_ok();
                paths.push(ReachabilityPath {
                    target: node.id.as_str().to_owned(),
                    action: format!("{action:?}"),
                    modality: *modality,
                    steps: steps.clone(),
                    reached,
                });
                let inspection = scenario.host().inspect();
                let current_nodes = scenario.semantic_nodes();
                issues.extend(classify_reachability_observation(
                    node.id.as_str(),
                    *action,
                    *modality,
                    &declared_ids,
                    policy,
                    &ReachabilityObservation {
                        steps,
                        failure: result.err(),
                        current_target: match modality {
                            ReachabilityModality::Keyboard => inspection.keyboard_focus,
                            ReachabilityModality::Controller => inspection.controller_target,
                            ReachabilityModality::Accessibility => None,
                        }
                        .map(|id| id.as_str().to_owned()),
                        current_semantic_ids: current_nodes
                            .iter()
                            .map(|node| node.id.as_str().to_owned())
                            .collect(),
                        semantic_changed: semantic_digest(&current_nodes) != before,
                    },
                ));
            }
        }
    }
    let mut queued = VecDeque::from([(Vec::<(String, ActionKind)>::new(), factory())]);
    let mut expanded_states = BTreeSet::new();
    let mut enqueued_states = BTreeSet::new();
    let dynamic_started = Instant::now();
    while let Some((prefix, state)) = queued.pop_front() {
        if dynamic_started.elapsed() >= wall_ceiling {
            if !issues
                .iter()
                .any(|issue| issue.target == "<reachability-graph>")
            {
                issues.push(reachability_ceiling_issue(wall_ceiling));
            }
            break;
        }
        if expanded_states.len() >= policy.maximum_state_count {
            issues.push(ReachabilityIssue {
                kind: ReachabilityIssueKind::ExcessivePathLength,
                target: "<dynamic-state-graph>".into(),
                action: "Explore".into(),
                modality: ReachabilityModality::Accessibility,
                detail: format!(
                    "dynamic exploration exceeded the {}-state safety ceiling",
                    policy.maximum_state_count
                ),
            });
            break;
        }
        let state_nodes = state.semantic_nodes();
        let state_digest = action_topology_digest(&state_nodes);
        if !expanded_states.insert(state_digest) {
            continue;
        }
        for node in state_nodes
            .iter()
            .filter(|node| node.enabled && !node.actions.is_empty())
        {
            for action in &node.actions {
                if !action_supported_by_modality(*action, ReachabilityModality::Accessibility) {
                    continue;
                }
                let key = format!(
                    "{}|{action:?}|{:?}",
                    node.id.as_str(),
                    ReachabilityModality::Accessibility
                );
                let newly_audited = audited.insert(key);
                let reveals_topology = may_reveal_topology(node, *action);
                if !newly_audited && !reveals_topology {
                    continue;
                }
                let mut next_prefix = prefix.clone();
                next_prefix.push((node.id.as_str().to_owned(), *action));
                let next = replay_accessibility_prefix(
                    &factory,
                    &next_prefix,
                    dynamic_started,
                    wall_ceiling,
                );
                if newly_audited {
                    let steps = next_prefix
                        .iter()
                        .map(|(target, action)| format!("accessibility:{action:?}:{target}"))
                        .collect::<Vec<_>>();
                    let failure = next
                        .as_ref()
                        .is_none()
                        .then(|| "dynamic accessibility path failed to replay".into());
                    let current_nodes = next
                        .as_ref()
                        .map_or_else(Vec::new, Scenario::semantic_nodes);
                    let current_ids = current_nodes
                        .iter()
                        .map(|node| node.id.as_str().to_owned())
                        .collect();
                    paths.push(ReachabilityPath {
                        target: node.id.as_str().to_owned(),
                        action: format!("{action:?}"),
                        modality: ReachabilityModality::Accessibility,
                        steps: steps.clone(),
                        reached: next.is_some(),
                    });
                    issues.extend(classify_reachability_observation(
                        node.id.as_str(),
                        *action,
                        ReachabilityModality::Accessibility,
                        &declared_ids,
                        policy,
                        &ReachabilityObservation {
                            steps,
                            failure,
                            current_target: None,
                            current_semantic_ids: current_ids,
                            semantic_changed: next.as_ref().is_some_and(|scenario| {
                                semantic_digest(&scenario.semantic_nodes())
                                    != semantic_digest(&state_nodes)
                            }),
                        },
                    ));
                }
                if reveals_topology
                    && prefix.len() < policy.maximum_path_length
                    && let Some(next) = next
                {
                    let next_digest = action_topology_digest(&next.semantic_nodes());
                    if next_digest != action_topology_digest(&state_nodes)
                        && !expanded_states.contains(&next_digest)
                        && enqueued_states.insert(next_digest)
                    {
                        queued.push_back((next_prefix, next));
                    }
                }
            }
        }
    }
    ReachabilityReport {
        schema: 1,
        semantic_digest: initial_semantic_digest,
        paths,
        issues,
    }
}

fn reachability_ceiling_issue(ceiling: Duration) -> ReachabilityIssue {
    ReachabilityIssue {
        kind: ReachabilityIssueKind::ExcessivePathLength,
        target: "<reachability-graph>".into(),
        action: "Explore".into(),
        modality: ReachabilityModality::Accessibility,
        detail: format!(
            "reachability exploration exceeded the {}ms wall ceiling",
            ceiling.as_millis()
        ),
    }
}

fn may_reveal_topology(node: &SemanticNodeSnapshot, action: ActionKind) -> bool {
    matches!(
        action,
        ActionKind::ContextMenu | ActionKind::Expand | ActionKind::EnterNavigation
    ) || (action == ActionKind::Activate
        && matches!(node.role, Some(SemanticRole::Menu | SemanticRole::Dialog)))
}

fn replay_accessibility_prefix<A: Application>(
    factory: &impl Fn() -> Scenario<A>,
    prefix: &[(String, ActionKind)],
    audit_started: Instant,
    wall_ceiling: Duration,
) -> Option<Scenario<A>> {
    let mut scenario = factory();
    for (target, action) in prefix {
        if audit_started.elapsed() >= wall_ceiling {
            return None;
        }
        let node = scenario
            .semantic_nodes()
            .into_iter()
            .find(|node| node.id.as_str() == target)?;
        let outcome = scenario.host.perform_accessibility_action(
            node.id.clone(),
            semantic_action_for_target(&node, *action),
        );
        if (!outcome.changed && outcome.effects.is_empty()) || !outcome.semantic_failures.is_empty()
        {
            return None;
        }
    }
    Some(scenario)
}

fn action_topology_digest(nodes: &[SemanticNodeSnapshot]) -> String {
    debug_digest(
        &nodes
            .iter()
            .map(|node| (node.id.as_str(), &node.actions, node.enabled))
            .collect::<Vec<_>>(),
    )
}

fn action_supported_by_modality(action: ActionKind, modality: ReachabilityModality) -> bool {
    match action {
        ActionKind::Activate | ActionKind::ContextMenu => true,
        ActionKind::Increment | ActionKind::Decrement => matches!(
            modality,
            ReachabilityModality::Controller | ReachabilityModality::Accessibility
        ),
        ActionKind::SetValue => modality == ReachabilityModality::Accessibility,
        ActionKind::Cancel
        | ActionKind::Expand
        | ActionKind::Collapse
        | ActionKind::Select
        | ActionKind::Dismiss
        | ActionKind::Scroll
        | ActionKind::EnterNavigation
        | ActionKind::ExitNavigation => modality == ReachabilityModality::Accessibility,
    }
}

pub fn validate_host<A: Application>(host: &UiHost<A>) -> Vec<ValidationIssue> {
    let accessibility = host.accessibility_nodes();
    let mut issues = Vec::new();
    let semantics = host.semantic_nodes();
    let inspection = host.inspect();
    issues.extend(validate_active_state_references(
        &semantics,
        inspection.keyboard_focus.as_ref(),
        inspection.controller_target.as_ref(),
    ));
    for semantic in semantics {
        let Some(node) = accessibility.iter().find(|node| node.id == semantic.id) else {
            issues.push(ValidationIssue::MissingAccessibility { id: semantic.id });
            continue;
        };
        if semantic.role.map(SemanticRole::as_str) != node.role.as_deref() {
            issues.push(ValidationIssue::RoleMismatch {
                id: semantic.id.clone(),
            });
        }
        if semantic.enabled
            && !semantic.actions.is_empty()
            && semantic
                .name
                .as_deref()
                .is_none_or(|name| name.trim().is_empty())
        {
            issues.push(ValidationIssue::MissingAccessibilityName {
                id: semantic.id.clone(),
            });
        }
        if semantic.name != node.label {
            issues.push(ValidationIssue::NameMismatch { id: semantic.id });
            continue;
        }
        if semantic.description != node.description {
            issues.push(ValidationIssue::DescriptionMismatch { id: semantic.id });
            continue;
        }
        if semantic.controls != node.controls {
            issues.push(ValidationIssue::ControlsMismatch { id: semantic.id });
        }
    }
    issues
}

pub fn validate_active_state_references(
    semantics: &[SemanticNodeSnapshot],
    keyboard_focus: Option<&UiId>,
    controller_target: Option<&UiId>,
) -> Vec<ValidationIssue> {
    let ids = semantics
        .iter()
        .map(|node| &node.id)
        .collect::<BTreeSet<_>>();
    [
        ("keyboard_focus", keyboard_focus),
        ("controller_target", controller_target),
    ]
    .into_iter()
    .filter_map(|(state, id)| {
        id.filter(|id| !ids.contains(id))
            .cloned()
            .map(|id| ValidationIssue::UnreconciledState { state, id })
    })
    .collect()
}

pub struct Scenario<A: Application> {
    host: UiHost<A>,
    budget: ScenarioBudget,
    initial_frame: u64,
    initial_viewport: [u32; 2],
    viewport: [u32; 2],
    scale: f32,
    initial_semantic_digest: String,
    operations: usize,
    next_event_order: u64,
    trace: Vec<TraceStep>,
    operation_trace: Vec<OperationTraceStep>,
}

impl<A: Application> Scenario<A> {
    pub fn new(application: A, width: u32, height: u32) -> Self {
        Self::with_budget(application, width, height, ScenarioBudget::default())
    }

    pub fn with_budget(application: A, width: u32, height: u32, budget: ScenarioBudget) -> Self {
        let host = UiHost::new(application, width, height);
        let initial_frame = host.inspect().frame_generation;
        let initial_semantic_digest = semantic_digest(&host.semantic_nodes());
        Self {
            host,
            budget,
            initial_frame,
            initial_viewport: [width, height],
            viewport: [width, height],
            scale: 1.0,
            initial_semantic_digest,
            operations: 0,
            next_event_order: 1,
            trace: Vec::new(),
            operation_trace: Vec::new(),
        }
    }

    pub fn semantic_nodes(&self) -> Vec<SemanticNodeSnapshot> {
        self.host.semantic_nodes()
    }

    pub fn perform(
        &mut self,
        selector: &Selector,
        action: SemanticAction,
    ) -> Result<&mut Self, ScenarioError> {
        self.operations += 1;
        if self.operations > self.budget.operations {
            return Err(ScenarioError::OperationBudgetExceeded);
        }
        let target = self.resolve_target(selector)?;
        let frame_before = self.host.inspect().frame_generation;
        let outcome = self
            .host
            .perform_semantic_action(target.id.clone(), action.clone());
        self.record(
            target.id,
            action,
            ActivationVia::Semantic,
            frame_before,
            &outcome,
        )?;
        Ok(self)
    }

    fn resolve_target(&self, selector: &Selector) -> Result<SemanticNodeSnapshot, ScenarioError> {
        let nodes = self.host.semantic_nodes();
        let selector_label = format!("{selector:?}");
        if let Selector::KeyedItem { collection, key } = selector {
            let suffix = format!("/{}/{}", collection.as_str(), key);
            let matches = nodes
                .iter()
                .filter(|node| node.id.as_str().ends_with(&suffix))
                .cloned()
                .collect::<Vec<_>>();
            return match matches.as_slice() {
                [target] => Ok(target.clone()),
                [] => Err(missing_target_error(selector_label, &suffix, &nodes)),
                _ => Err(ScenarioError::AmbiguousTarget {
                    selector: selector_label,
                    matches: matches.len(),
                    candidates: matches
                        .iter()
                        .map(|node| node.id.as_str().to_owned())
                        .collect(),
                }),
            };
        }
        match self.host.query_unique(&selector.production()) {
            Ok(target) => Ok(target),
            Err(SemanticQueryError::Missing) => Err(missing_target_error(
                selector_label.clone(),
                selector_needle(selector),
                &nodes,
            )),
            Err(SemanticQueryError::Ambiguous { matches }) => {
                let candidates = self
                    .host
                    .query(&selector.production())
                    .into_iter()
                    .map(|node| node.id.as_str().to_owned())
                    .collect();
                Err(ScenarioError::AmbiguousTarget {
                    selector: selector_label,
                    matches,
                    candidates,
                })
            }
        }
    }

    pub fn controller_activate(
        &mut self,
        selector: &Selector,
    ) -> Result<ControllerRoute, ScenarioError> {
        self.begin_operation()?;
        let target = self.resolve_target(selector)?;
        let frame_before = self.host.inspect().frame_generation;
        let mut actions = Vec::new();
        for _ in 0..32 {
            let inspection = self.host.inspect();
            if inspection.controller_target.as_ref() == Some(&target.id) {
                actions.push(ControllerAction::Confirm);
                let outcome = self
                    .host
                    .handle_controller_action(ControllerAction::Confirm);
                if outcome.changed && outcome.semantic_failures.is_empty() {
                    self.record(
                        target.id.clone(),
                        SemanticAction::Invoke(ActionKind::Activate),
                        ActivationVia::Controller,
                        frame_before,
                        &outcome,
                    )?;
                    return Ok(ControllerRoute {
                        target: target.id,
                        actions,
                    });
                }
                return Err(ScenarioError::SemanticActionFailed);
            }
            let action =
                inspection
                    .controller_target
                    .as_ref()
                    .map_or(ControllerAction::Down, |id| {
                        self.host
                            .semantic_nodes()
                            .into_iter()
                            .find(|node| &node.id == id)
                            .map_or(ControllerAction::Confirm, |current| {
                                let current_x =
                                    current.bounds.origin.x + current.bounds.size.width / 2.0;
                                let current_y =
                                    current.bounds.origin.y + current.bounds.size.height / 2.0;
                                let target_x =
                                    target.bounds.origin.x + target.bounds.size.width / 2.0;
                                let target_y =
                                    target.bounds.origin.y + target.bounds.size.height / 2.0;
                                let dx = target_x - current_x;
                                let dy = target_y - current_y;
                                if dx.abs() > dy.abs() {
                                    if dx >= 0.0 {
                                        ControllerAction::Right
                                    } else {
                                        ControllerAction::Left
                                    }
                                } else if dy >= 0.0 {
                                    ControllerAction::Down
                                } else {
                                    ControllerAction::Up
                                }
                            })
                    });
            actions.push(action);
            self.host.handle_controller_action(action);
        }
        Err(ScenarioError::ControllerTargetUnreachable)
    }

    pub fn pointer_activate(&mut self, selector: &Selector) -> Result<PointerRoute, ScenarioError> {
        self.begin_operation()?;
        let target = self.resolve_target(selector)?;
        let frame_before = self.host.inspect().frame_generation;
        let route = self
            .host
            .resolve_effective_target(&target.id, ActionKind::Activate)
            .map_err(|_| ScenarioError::SemanticActionFailed)?;
        let pressed = self.host.handle_event(UiEvent::PointerPressed(route.point));
        if !pressed.changed {
            return Err(ScenarioError::SemanticActionFailed);
        }
        let press_frame = self.host.inspect().frame_generation;
        let released = self
            .host
            .handle_event(UiEvent::PointerReleased(route.point));
        if !released.changed {
            return Err(ScenarioError::SemanticActionFailed);
        }
        self.record(
            target.id,
            SemanticAction::Invoke(ActionKind::Activate),
            ActivationVia::Pointer,
            frame_before,
            &released,
        )?;
        let release_frame = self.host.inspect().frame_generation;
        if release_frame.saturating_sub(self.initial_frame) > self.budget.frames {
            return Err(ScenarioError::FrameBudgetExceeded);
        }
        Ok(PointerRoute {
            target: route.target,
            point: route.point,
            press_frame,
            release_frame,
        })
    }

    pub fn keyboard_activate(
        &mut self,
        selector: &Selector,
    ) -> Result<KeyboardRoute, ScenarioError> {
        self.begin_operation()?;
        let target = self.resolve_target(selector)?;
        let frame_before = self.host.inspect().frame_generation;
        for focus_steps in 0..32 {
            if self.host.inspect().keyboard_focus.as_ref() == Some(&target.id) {
                let outcome = self.host.handle_event(UiEvent::KeyboardActivate);
                if outcome.changed {
                    self.record(
                        target.id.clone(),
                        SemanticAction::Invoke(ActionKind::Activate),
                        ActivationVia::Keyboard,
                        frame_before,
                        &outcome,
                    )?;
                    return Ok(KeyboardRoute {
                        target: target.id,
                        focus_steps,
                    });
                }
                return Err(ScenarioError::SemanticActionFailed);
            }
            self.host.handle_event(UiEvent::FocusNext);
        }
        Err(ScenarioError::KeyboardTargetUnreachable)
    }

    pub fn accessibility_activate(
        &mut self,
        selector: &Selector,
    ) -> Result<&mut Self, ScenarioError> {
        self.begin_operation()?;
        let target = self.resolve_target(selector)?;
        if !target.actions.contains(&ActionKind::Activate) {
            return Err(ScenarioError::SemanticActionFailed);
        }
        let frame_before = self.host.inspect().frame_generation;
        let outcome = self
            .host
            .handle_event(UiEvent::AccessibilityActivate(target.id.clone()));
        if !outcome.changed {
            return Err(ScenarioError::SemanticActionFailed);
        }
        self.record(
            target.id,
            SemanticAction::Invoke(ActionKind::Activate),
            ActivationVia::Accessibility,
            frame_before,
            &outcome,
        )?;
        Ok(self)
    }

    pub fn activate_via(
        &mut self,
        via: ActivationVia,
        selector: &Selector,
    ) -> Result<&mut Self, ScenarioError> {
        match via {
            ActivationVia::Pointer => {
                self.pointer_activate(selector)?;
            }
            ActivationVia::Touch => {
                self.touch_activate(selector, 1)?;
            }
            ActivationVia::Keyboard => {
                self.keyboard_activate(selector)?;
            }
            ActivationVia::Controller => {
                self.controller_activate(selector)?;
            }
            ActivationVia::Accessibility => {
                self.accessibility_activate(selector)?;
            }
            ActivationVia::Semantic => {
                self.activate(selector)?;
            }
        }
        Ok(self)
    }

    pub fn invoke_via(
        &mut self,
        via: ActivationVia,
        selector: &Selector,
        action: ActionKind,
    ) -> Result<&mut Self, ScenarioError> {
        if action == ActionKind::Activate {
            return self.activate_via(via, selector);
        }
        if action != ActionKind::ContextMenu {
            return Err(ScenarioError::SemanticActionFailed);
        }
        match via {
            ActivationVia::Semantic => {
                self.perform(selector, SemanticAction::Invoke(ActionKind::ContextMenu))?;
            }
            ActivationVia::Pointer => {
                self.pointer_context(selector)?;
            }
            ActivationVia::Touch => {
                self.touch_context(selector)?;
            }
            ActivationVia::Keyboard => {
                let target = self.resolve_target(selector)?.id;
                for _ in 0..=self.semantic_nodes().len() {
                    if self.host.inspect().keyboard_focus.as_ref() == Some(&target) {
                        self.keyboard_context_focused()?;
                        return Ok(self);
                    }
                    self.keyboard_focus(FocusDirection::Next)?;
                }
                return Err(ScenarioError::SemanticActionFailed);
            }
            ActivationVia::Controller => {
                let target = self.resolve_target(selector)?.id;
                for _ in 0..=self.semantic_nodes().len() {
                    if self.host.inspect().controller_target.as_ref() == Some(&target) {
                        let outcome = self
                            .host
                            .handle_controller_action(ControllerAction::ContextMenu);
                        if outcome.changed && outcome.semantic_failures.is_empty() {
                            return Ok(self);
                        }
                        return Err(ScenarioError::SemanticActionFailed);
                    }
                    self.host.handle_controller_action(ControllerAction::Down);
                }
                return Err(ScenarioError::SemanticActionFailed);
            }
            ActivationVia::Accessibility => {
                self.accessibility_action(selector, ActionKind::ContextMenu)?;
            }
        }
        Ok(self)
    }

    pub fn activate(&mut self, selector: &Selector) -> Result<&mut Self, ScenarioError> {
        self.perform(selector, SemanticAction::Invoke(ActionKind::Activate))
    }

    pub fn set_value(
        &mut self,
        selector: &Selector,
        value: SemanticValueInput,
    ) -> Result<&mut Self, ScenarioError> {
        self.perform(selector, SemanticAction::SetValue(value))
    }

    fn record(
        &mut self,
        target: UiId,
        action: SemanticAction,
        via: ActivationVia,
        frame_before: u64,
        outcome: &HostEventOutcome,
    ) -> Result<(), ScenarioError> {
        if !outcome.semantic_failures.is_empty() {
            return Err(ScenarioError::SemanticActionFailed);
        }
        let frame_after = self.host.inspect().frame_generation;
        if frame_after.saturating_sub(self.initial_frame) > self.budget.frames {
            return Err(ScenarioError::FrameBudgetExceeded);
        }
        if self.trace.len() < self.budget.trace_steps {
            self.trace.push(TraceStep {
                target,
                action,
                via,
                frame_before,
                frame_after,
                changed: outcome.changed,
                invalidation: format!("{:?}", outcome.invalidation),
                messages: message_evidence_snapshot(outcome),
                effects: effect_evidence_snapshot(outcome),
            });
        }
        Ok(())
    }

    fn begin_operation(&mut self) -> Result<(), ScenarioError> {
        self.operations += 1;
        if self.operations > self.budget.operations {
            return Err(ScenarioError::OperationBudgetExceeded);
        }
        Ok(())
    }

    pub fn resize(
        &mut self,
        width: u32,
        height: u32,
        scale: f32,
    ) -> Result<&mut Self, ScenarioError> {
        self.begin_operation()?;
        let before = self.state_snapshot();
        self.host.resize(width, height);
        self.viewport = [width, height];
        self.scale = scale;
        let changed = self.host.inspect().frame_generation != before.frame;
        self.push_operation(
            ScenarioOperation::Resize {
                width,
                height,
                scale,
            },
            None,
            Vec::new(),
            before,
            HostEventOutcome {
                changed,
                ..HostEventOutcome::default()
            },
        )?;
        Ok(self)
    }

    pub fn advance_time(&mut self, ticks: u32) -> Result<&mut Self, ScenarioError> {
        self.begin_operation()?;
        let before = self.state_snapshot();
        let mut outcome = HostEventOutcome::default();
        for _ in 0..ticks {
            merge_outcome(&mut outcome, self.host.handle_event(UiEvent::CaretBlink));
            if self.host.poll() {
                outcome.changed = true;
            }
        }
        self.push_operation(
            ScenarioOperation::AdvanceTime { ticks },
            None,
            Vec::new(),
            before,
            outcome,
        )?;
        Ok(self)
    }

    pub fn window_focus(&mut self, gained: bool) -> Result<&mut Self, ScenarioError> {
        let event = if gained {
            UiEvent::FocusGained
        } else {
            UiEvent::FocusLost
        };
        self.event_operation(ScenarioOperation::Focus { gained }, None, Vec::new(), event)
    }

    pub fn suspend(&mut self) -> Result<&mut Self, ScenarioError> {
        self.event_operation(
            ScenarioOperation::Suspend,
            None,
            Vec::new(),
            UiEvent::Suspended,
        )
    }

    pub fn close(&mut self) -> Result<&mut Self, ScenarioError> {
        self.event_operation(
            ScenarioOperation::Close,
            None,
            Vec::new(),
            UiEvent::Suspended,
        )
    }

    pub fn pointer_move(&mut self, selector: &Selector) -> Result<Point, ScenarioError> {
        self.begin_operation()?;
        let target = self.resolve_target(selector)?;
        let route = self
            .host
            .resolve_effective_target(&target.id, ActionKind::Activate)
            .map_err(|_| ScenarioError::SemanticActionFailed)?;
        let before = self.state_snapshot();
        let outcome = self.host.handle_event(UiEvent::PointerMoved(route.point));
        self.push_operation(
            ScenarioOperation::PointerMove {
                target: target.id.as_str().into(),
            },
            Some(target.id),
            vec![[route.point.x, route.point.y]],
            before,
            outcome,
        )?;
        Ok(route.point)
    }

    pub fn pointer_context(&mut self, selector: &Selector) -> Result<&mut Self, ScenarioError> {
        self.begin_operation()?;
        let target = self.resolve_target(selector)?;
        let route = self
            .host
            .resolve_effective_target(&target.id, ActionKind::ContextMenu)
            .map_err(|_| ScenarioError::SemanticActionFailed)?;
        let before = self.state_snapshot();
        let outcome = self.host.handle_event(UiEvent::PointerContext(route.point));
        self.push_operation(
            ScenarioOperation::PointerContext {
                target: target.id.as_str().into(),
            },
            Some(target.id),
            vec![[route.point.x, route.point.y]],
            before,
            outcome,
        )?;
        Ok(self)
    }

    pub fn touch_context(&mut self, selector: &Selector) -> Result<&mut Self, ScenarioError> {
        self.begin_operation()?;
        let target = self.resolve_target(selector)?;
        let route = self
            .host
            .resolve_effective_target(&target.id, ActionKind::ContextMenu)
            .map_err(|_| ScenarioError::SemanticActionFailed)?;
        let before = self.state_snapshot();
        let outcome = self.host.handle_event(UiEvent::TouchLongPress(route.point));
        self.push_operation(
            ScenarioOperation::TouchContext {
                target: target.id.as_str().into(),
            },
            Some(target.id),
            vec![[route.point.x, route.point.y]],
            before,
            outcome,
        )?;
        Ok(self)
    }

    pub fn pointer_drag(
        &mut self,
        from: &Selector,
        to: &Selector,
    ) -> Result<&mut Self, ScenarioError> {
        self.begin_operation()?;
        let from = self.resolve_target(from)?;
        let to = self.resolve_target(to)?;
        let from_route = self
            .host
            .resolve_effective_target(&from.id, ActionKind::Activate)
            .map_err(|_| ScenarioError::SemanticActionFailed)?;
        let to_point = center(to.bounds);
        let before = self.state_snapshot();
        let mut outcome = self
            .host
            .handle_event(UiEvent::PointerMoved(from_route.point));
        merge_outcome(
            &mut outcome,
            self.host
                .handle_event(UiEvent::PointerPressed(from_route.point)),
        );
        merge_outcome(
            &mut outcome,
            self.host.handle_event(UiEvent::PointerMoved(to_point)),
        );
        merge_outcome(
            &mut outcome,
            self.host.handle_event(UiEvent::PointerReleased(to_point)),
        );
        self.push_operation(
            ScenarioOperation::PointerDrag {
                from: from.id.as_str().into(),
                to: to.id.as_str().into(),
            },
            Some(to.id),
            vec![
                [from_route.point.x, from_route.point.y],
                [to_point.x, to_point.y],
            ],
            before,
            outcome,
        )?;
        Ok(self)
    }

    pub fn pointer_scroll(
        &mut self,
        selector: &Selector,
        delta_x: f32,
        delta_y: f32,
    ) -> Result<&mut Self, ScenarioError> {
        self.begin_operation()?;
        let target = self.resolve_target(selector)?;
        let point = center(target.bounds);
        let before = self.state_snapshot();
        let mut outcome = self.host.handle_event(UiEvent::PointerMoved(point));
        if delta_x != 0.0 {
            merge_outcome(
                &mut outcome,
                self.host
                    .handle_event(UiEvent::ScrollHorizontal { point, delta_x }),
            );
        }
        if delta_y != 0.0 {
            merge_outcome(
                &mut outcome,
                self.host.handle_event(UiEvent::Scroll { point, delta_y }),
            );
        }
        self.push_operation(
            ScenarioOperation::PointerScroll {
                target: target.id.as_str().into(),
                delta_x,
                delta_y,
            },
            Some(target.id),
            vec![[point.x, point.y]],
            before,
            outcome,
        )?;
        Ok(self)
    }

    pub fn touch_activate(
        &mut self,
        selector: &Selector,
        contact: u64,
    ) -> Result<&mut Self, ScenarioError> {
        self.begin_operation()?;
        let target = self.resolve_target(selector)?;
        let route = self
            .host
            .resolve_effective_target(&target.id, ActionKind::Activate)
            .map_err(|_| ScenarioError::SemanticActionFailed)?;
        let before = self.state_snapshot();
        let order = self.next_order();
        let mut outcome = self.normalized_touch(TouchEvent::Started {
            device: DeviceId(0x4e49),
            order,
            contact: TouchId(contact),
            position: input_point(route.point),
        });
        let order = self.next_order();
        merge_outcome(
            &mut outcome,
            self.normalized_touch(TouchEvent::Ended {
                device: DeviceId(0x4e49),
                order,
                contact: TouchId(contact),
                position: input_point(route.point),
            }),
        );
        self.push_operation(
            ScenarioOperation::TouchActivate {
                target: target.id.as_str().into(),
                contact,
            },
            Some(target.id),
            vec![[route.point.x, route.point.y]],
            before,
            outcome,
        )?;
        Ok(self)
    }

    pub fn touch_drag(
        &mut self,
        from: &Selector,
        to: &Selector,
        contact: u64,
    ) -> Result<&mut Self, ScenarioError> {
        self.begin_operation()?;
        let from = self.resolve_target(from)?;
        let to = self.resolve_target(to)?;
        let start = center(from.bounds);
        let end = center(to.bounds);
        let before = self.state_snapshot();
        let order = self.next_order();
        let mut outcome = self.normalized_touch(TouchEvent::Started {
            device: DeviceId(0x4e49),
            order,
            contact: TouchId(contact),
            position: input_point(start),
        });
        let order = self.next_order();
        merge_outcome(
            &mut outcome,
            self.normalized_touch(TouchEvent::Moved {
                device: DeviceId(0x4e49),
                order,
                contact: TouchId(contact),
                position: input_point(end),
            }),
        );
        let order = self.next_order();
        merge_outcome(
            &mut outcome,
            self.normalized_touch(TouchEvent::Ended {
                device: DeviceId(0x4e49),
                order,
                contact: TouchId(contact),
                position: input_point(end),
            }),
        );
        self.push_operation(
            ScenarioOperation::TouchDrag {
                from: from.id.as_str().into(),
                to: to.id.as_str().into(),
                contact,
            },
            Some(to.id),
            vec![[start.x, start.y], [end.x, end.y]],
            before,
            outcome,
        )?;
        Ok(self)
    }

    pub fn touch_cancel(
        &mut self,
        selector: &Selector,
        contact: u64,
    ) -> Result<&mut Self, ScenarioError> {
        self.begin_operation()?;
        let target = self.resolve_target(selector)?;
        let point = center(target.bounds);
        let before = self.state_snapshot();
        let order = self.next_order();
        let mut outcome = self.normalized_touch(TouchEvent::Started {
            device: DeviceId(0x4e49),
            order,
            contact: TouchId(contact),
            position: input_point(point),
        });
        let order = self.next_order();
        merge_outcome(
            &mut outcome,
            self.normalized_touch(TouchEvent::Cancelled {
                device: DeviceId(0x4e49),
                order,
                contact: TouchId(contact),
            }),
        );
        self.push_operation(
            ScenarioOperation::TouchCancel {
                target: target.id.as_str().into(),
                contact,
            },
            Some(target.id),
            vec![[point.x, point.y]],
            before,
            outcome,
        )?;
        Ok(self)
    }

    pub fn keyboard_focus(
        &mut self,
        direction: FocusDirection,
    ) -> Result<&mut Self, ScenarioError> {
        let event = match direction {
            FocusDirection::Next => UiEvent::FocusNext,
            FocusDirection::Previous => UiEvent::FocusPrevious,
        };
        self.event_operation(
            ScenarioOperation::KeyboardFocus { direction },
            None,
            Vec::new(),
            event,
        )
    }

    pub fn keyboard_activate_focused(&mut self) -> Result<&mut Self, ScenarioError> {
        self.event_operation(
            ScenarioOperation::KeyboardActivate,
            self.host.inspect().keyboard_focus,
            Vec::new(),
            UiEvent::KeyboardActivate,
        )
    }

    pub fn keyboard_context_focused(&mut self) -> Result<&mut Self, ScenarioError> {
        self.event_operation(
            ScenarioOperation::KeyboardContext,
            self.host.inspect().keyboard_focus,
            Vec::new(),
            UiEvent::KeyboardContextMenu,
        )
    }

    pub fn accessibility_action(
        &mut self,
        selector: &Selector,
        action: ActionKind,
    ) -> Result<&mut Self, ScenarioError> {
        self.begin_operation()?;
        let target = self.resolve_target(selector)?;
        let before = self.state_snapshot();
        let outcome = self.host.perform_accessibility_action(
            target.id.clone(),
            semantic_action_for_target(&target, action),
        );
        self.push_operation(
            ScenarioOperation::Accessibility {
                target: target.id.as_str().into(),
                action: TraceAction::from(&SemanticAction::Invoke(action)),
            },
            Some(target.id),
            Vec::new(),
            before,
            outcome,
        )?;
        Ok(self)
    }

    pub fn controller_semantic_action(
        &mut self,
        selector: &Selector,
        action: ActionKind,
    ) -> Result<&mut Self, ScenarioError> {
        self.begin_operation()?;
        let target = self.resolve_target(selector)?;
        let before = self.state_snapshot();
        let outcome = self.host.perform_controller_semantic_action(
            target.id.clone(),
            semantic_action_for_target(&target, action),
        );
        self.push_operation(
            ScenarioOperation::ControllerSemantic {
                target: target.id.as_str().into(),
                action: TraceAction::from(&SemanticAction::Invoke(action)),
            },
            Some(target.id),
            Vec::new(),
            before,
            outcome,
        )?;
        Ok(self)
    }

    pub fn semantic_operation(
        &mut self,
        selector: &Selector,
        action: SemanticAction,
    ) -> Result<&mut Self, ScenarioError> {
        self.begin_operation()?;
        let target = self.resolve_target(selector)?;
        let before = self.state_snapshot();
        let outcome = self
            .host
            .perform_semantic_action(target.id.clone(), action.clone());
        self.push_operation(
            ScenarioOperation::Semantic {
                target: target.id.as_str().into(),
                action: TraceAction::from(&action),
            },
            Some(target.id),
            Vec::new(),
            before,
            outcome,
        )?;
        Ok(self)
    }

    pub fn text_input(&mut self, text: impl Into<String>) -> Result<&mut Self, ScenarioError> {
        let text = text.into();
        self.event_operation(
            ScenarioOperation::TextInput { text: text.clone() },
            None,
            Vec::new(),
            UiEvent::TextInput(text),
        )
    }

    pub fn ime_preedit(&mut self, text: impl Into<String>) -> Result<&mut Self, ScenarioError> {
        let text = text.into();
        self.event_operation(
            ScenarioOperation::ImePreedit { text: text.clone() },
            None,
            Vec::new(),
            UiEvent::ImePreedit(text),
        )
    }

    pub fn clipboard_paste(&mut self, text: impl Into<String>) -> Result<&mut Self, ScenarioError> {
        let text = text.into();
        self.event_operation(
            ScenarioOperation::ClipboardPaste { text: text.clone() },
            None,
            Vec::new(),
            UiEvent::TextPaste(text),
        )
    }

    pub fn controller(&mut self, action: ControllerAction) -> Result<&mut Self, ScenarioError> {
        self.begin_operation()?;
        let before = self.state_snapshot();
        let outcome = self.host.handle_controller_action(action);
        self.push_operation(
            ScenarioOperation::Controller {
                action: format!("{action:?}"),
            },
            self.host.inspect().controller_target,
            Vec::new(),
            before,
            outcome,
        )?;
        Ok(self)
    }

    pub fn platform_capability(
        &mut self,
        capability: impl Into<String>,
        available: bool,
    ) -> Result<&mut Self, ScenarioError> {
        let capability = capability.into();
        let event = (!available).then_some(UiEvent::DeviceRemoved);
        if let Some(event) = event {
            self.event_operation(
                ScenarioOperation::PlatformCapabilityChanged {
                    capability,
                    available,
                },
                None,
                Vec::new(),
                event,
            )
        } else {
            self.begin_operation()?;
            let before = self.state_snapshot();
            self.push_operation(
                ScenarioOperation::PlatformCapabilityChanged {
                    capability,
                    available,
                },
                None,
                Vec::new(),
                before,
                HostEventOutcome::default(),
            )?;
            Ok(self)
        }
    }

    pub fn domain_completion<T: std::any::Any + Send>(
        &mut self,
        id: &'static str,
        payload: T,
    ) -> Result<&mut Self, ScenarioError> {
        self.begin_operation()?;
        let before = self.state_snapshot();
        let outcome = self.host.step(HostBatch {
            completions: vec![Completion::new(id, payload)],
            ..HostBatch::default()
        });
        let failure = outcome.completion_failures.first().cloned();
        self.push_operation(
            ScenarioOperation::DomainCompletion {
                id: id.to_owned(),
                payload_type: type_name::<T>().to_owned(),
            },
            None,
            Vec::new(),
            before,
            outcome,
        )?;
        if let Some(failure) = failure {
            return Err(ScenarioError::CompletionFailed {
                id: failure.id.to_owned(),
                detail: failure.detail,
            });
        }
        Ok(self)
    }

    fn event_operation(
        &mut self,
        operation: ScenarioOperation,
        target: Option<UiId>,
        points: Vec<[f32; 2]>,
        event: UiEvent,
    ) -> Result<&mut Self, ScenarioError> {
        self.begin_operation()?;
        let before = self.state_snapshot();
        let outcome = self.host.handle_event(event);
        self.push_operation(operation, target, points, before, outcome)?;
        Ok(self)
    }

    fn next_order(&mut self) -> EventOrder {
        let order = EventOrder(self.next_event_order);
        self.next_event_order = self.next_event_order.saturating_add(1);
        order
    }

    fn normalized_touch(&mut self, event: TouchEvent) -> HostEventOutcome {
        self.host.step(HostBatch {
            events: vec![HostEvent::Normalized {
                input: InputEvent::Touch(event),
                clipboard_text: None,
            }],
            ..HostBatch::default()
        })
    }

    fn state_snapshot(&self) -> ScenarioStateSnapshot {
        let inspection = self.host.inspect();
        ScenarioStateSnapshot {
            frame: inspection.frame_generation,
            semantic_digest: semantic_digest(&self.host.semantic_nodes()),
            accessibility_digest: debug_digest(&self.host.accessibility_nodes()),
            window_focused: inspection.window_focused,
            keyboard_focus: inspection.keyboard_focus.map(|id| id.as_str().to_owned()),
            controller_target: inspection
                .controller_target
                .map(|id| id.as_str().to_owned()),
            controller_scope: inspection.controller_scope.map(|id| id.as_str().to_owned()),
            open_overlay: inspection
                .open_overlay
                .map(|overlay| format!("{overlay:?}")),
            modality: format!("{:?}", inspection.modality),
            diagnostics: inspection
                .diagnostics
                .into_iter()
                .map(|diagnostic| {
                    format!(
                        "{:?}:{}:{}",
                        diagnostic.kind,
                        diagnostic.id.as_str(),
                        diagnostic.detail
                    )
                })
                .chain(
                    inspection
                        .overlay_failures
                        .into_iter()
                        .map(|failure| format!("overlay:{failure:?}")),
                )
                .collect(),
        }
    }

    fn push_operation(
        &mut self,
        operation: ScenarioOperation,
        target: Option<UiId>,
        points: Vec<[f32; 2]>,
        before: ScenarioStateSnapshot,
        outcome: HostEventOutcome,
    ) -> Result<(), ScenarioError> {
        let after = self.state_snapshot();
        if after.frame.saturating_sub(self.initial_frame) > self.budget.frames {
            return Err(ScenarioError::FrameBudgetExceeded);
        }
        if self.operation_trace.len() < self.budget.trace_steps {
            self.operation_trace.push(OperationTraceStep {
                operation,
                resolved_target: target.map(|id| id.as_str().to_owned()),
                generated_points: points,
                before,
                after,
                outcome: outcome_snapshot(&outcome),
            });
        }
        Ok(())
    }

    pub fn host(&self) -> &UiHost<A> {
        &self.host
    }

    pub fn host_mut(&mut self) -> &mut UiHost<A> {
        &mut self.host
    }

    pub fn trace(&self) -> &[TraceStep] {
        &self.trace
    }

    pub fn operation_trace(&self) -> &[OperationTraceStep] {
        &self.operation_trace
    }

    pub fn coverage_report(&self) -> InteractionCoverageReport {
        let nodes = self.host.semantic_nodes();
        let roles = nodes
            .iter()
            .filter_map(|node| node.role.map(|role| role.as_str().to_owned()))
            .collect();
        let actions = nodes
            .iter()
            .flat_map(|node| node.actions.iter().map(|action| format!("{action:?}")))
            .collect();
        let mut state_variants = BTreeSet::new();
        for node in &nodes {
            state_variants.insert(if node.enabled { "enabled" } else { "disabled" }.into());
            if node.focused {
                state_variants.insert("focused".into());
            }
            if node.controller_selected {
                state_variants.insert("controller_selected".into());
            }
            if node.value.is_some() {
                state_variants.insert("valued".into());
            }
        }
        let input_routes = self
            .operation_trace
            .iter()
            .map(|step| operation_kind(&step.operation).to_owned())
            .collect();
        let mut effects = BTreeSet::new();
        for step in &self.operation_trace {
            effects.extend(step.outcome.effects.iter().map(|effect| {
                effect
                    .label
                    .clone()
                    .unwrap_or_else(|| effect.type_name.clone())
            }));
            effects.extend(step.outcome.global_actions.iter().cloned());
            if step.outcome.clipboard_text.is_some() {
                effects.insert("clipboard".into());
            }
        }
        let mut adapter_stages = BTreeSet::from(["semantic_host".into()]);
        if self
            .operation_trace
            .iter()
            .any(|step| !step.generated_points.is_empty())
        {
            adapter_stages.insert("production_hit_test".into());
        }
        if self.operation_trace.iter().any(|step| {
            matches!(
                step.operation,
                ScenarioOperation::PlatformCapabilityChanged { .. }
            )
        }) {
            adapter_stages.insert("platform_capability_adapter".into());
        }
        InteractionCoverageReport {
            schema: 1,
            semantic_digest: semantic_digest(&nodes),
            roles,
            actions,
            state_variants,
            input_routes,
            effects,
            adapter_stages,
        }
    }

    pub fn assert_no_diagnostics(&self) -> Result<&Self, ScenarioError> {
        let snapshot = self.state_snapshot();
        if snapshot.diagnostics.is_empty() {
            Ok(self)
        } else {
            Err(self.assertion_failure("no diagnostics", snapshot.diagnostics.join("; "), None))
        }
    }

    pub fn assert_focus(&self, selector: &Selector) -> Result<&Self, ScenarioError> {
        let target = self.resolve_target(selector)?;
        let inspection = self.host.inspect();
        if inspection.keyboard_focus.as_ref() == Some(&target.id)
            || inspection.controller_target.as_ref() == Some(&target.id)
        {
            Ok(self)
        } else {
            Err(self.assertion_failure(
                "focus",
                format!(
                    "expected {}, keyboard={:?}, controller={:?}",
                    target.id.as_str(),
                    inspection.keyboard_focus,
                    inspection.controller_target
                ),
                Some(&target.id),
            ))
        }
    }

    pub fn assert_action_available(
        &self,
        selector: &Selector,
        action: ActionKind,
    ) -> Result<&Self, ScenarioError> {
        let target = self.resolve_target(selector)?;
        if target.enabled && target.actions.contains(&action) {
            Ok(self)
        } else {
            Err(self.assertion_failure(
                "action available",
                format!("{} does not expose enabled {action:?}", target.id.as_str()),
                Some(&target.id),
            ))
        }
    }

    pub fn assert_visible(&self, selector: &Selector) -> Result<&Self, ScenarioError> {
        let target = self.resolve_target(selector)?;
        let viewport =
            nickel_ui::Rect::new(0.0, 0.0, self.viewport[0] as f32, self.viewport[1] as f32);
        if rect_intersects(target.bounds, viewport)
            && target.bounds.size.width > 0.0
            && target.bounds.size.height > 0.0
        {
            Ok(self)
        } else {
            Err(self.assertion_failure(
                "visible",
                format!("{} lies outside {:?}", target.id.as_str(), self.viewport),
                Some(&target.id),
            ))
        }
    }

    pub fn assert_layout(
        &self,
        left: &Selector,
        relation: LayoutRelation,
        right: &Selector,
    ) -> Result<&Self, ScenarioError> {
        let left = self.resolve_target(left)?;
        let right = self.resolve_target(right)?;
        let l = left.bounds;
        let r = right.bounds;
        let matches = match relation {
            LayoutRelation::Above => l.origin.y + l.size.height <= r.origin.y,
            LayoutRelation::Below => r.origin.y + r.size.height <= l.origin.y,
            LayoutRelation::LeftOf => l.origin.x + l.size.width <= r.origin.x,
            LayoutRelation::RightOf => r.origin.x + r.size.width <= l.origin.x,
            LayoutRelation::Contains => {
                l.origin.x <= r.origin.x
                    && l.origin.y <= r.origin.y
                    && l.origin.x + l.size.width >= r.origin.x + r.size.width
                    && l.origin.y + l.size.height >= r.origin.y + r.size.height
            }
            LayoutRelation::NonOverlapping => !rect_intersects(l, r),
        };
        if matches {
            Ok(self)
        } else {
            Err(self.assertion_failure(
                "layout relation",
                format!(
                    "{} {relation:?} {} is false: left={l:?}, right={r:?}",
                    left.id.as_str(),
                    right.id.as_str()
                ),
                Some(&left.id),
            ))
        }
    }

    pub fn assert_accessibility(&self) -> Result<&Self, ScenarioError> {
        let issues = validate_host(&self.host);
        if issues.is_empty() {
            Ok(self)
        } else {
            Err(self.assertion_failure("accessibility parity", format!("{issues:?}"), None))
        }
    }

    pub fn assert_modality(&self, expected: InputModality) -> Result<&Self, ScenarioError> {
        let actual = self.host.inspect().modality;
        if actual == expected {
            Ok(self)
        } else {
            Err(self.assertion_failure(
                "input modality",
                format!("expected {expected:?}, actual {actual:?}"),
                None,
            ))
        }
    }

    pub fn assert_overlay_open(&self, expected: bool) -> Result<&Self, ScenarioError> {
        let overlay = self.host.inspect().open_overlay;
        if overlay.is_some() == expected {
            Ok(self)
        } else {
            Err(self.assertion_failure(
                "overlay state",
                format!("expected open={expected}, actual={overlay:?}"),
                None,
            ))
        }
    }

    pub fn assert_value(
        &self,
        selector: &Selector,
        expected: &SemanticValueSnapshot,
    ) -> Result<&Self, ScenarioError> {
        let target = self.resolve_target(selector)?;
        if target.value.as_ref() == Some(expected) {
            Ok(self)
        } else {
            Err(self.assertion_failure(
                "semantic value",
                format!("expected {expected:?}, actual {:?}", target.value),
                Some(&target.id),
            ))
        }
    }

    pub fn assert_last_message_count(&self, expected: usize) -> Result<&Self, ScenarioError> {
        let Some(step) = self.operation_trace.last() else {
            return Err(self.assertion_failure(
                "message count",
                "scenario has no recorded operation",
                None,
            ));
        };
        let actual = step.outcome.messages.len();
        if actual == expected {
            Ok(self)
        } else {
            Err(self.assertion_failure(
                "message count",
                format!(
                    "expected {expected}, actual {actual}: {:?}",
                    step.outcome.messages
                ),
                None,
            ))
        }
    }

    pub fn assert_last_effect_count(&self, expected: usize) -> Result<&Self, ScenarioError> {
        let Some(step) = self.operation_trace.last() else {
            return Err(self.assertion_failure(
                "effect count",
                "scenario has no recorded operation",
                None,
            ));
        };
        let actual = step.outcome.effects.len()
            + step.outcome.global_actions.len()
            + usize::from(step.outcome.clipboard_text.is_some());
        if actual == expected {
            Ok(self)
        } else {
            Err(self.assertion_failure(
                "effect count",
                format!("expected {expected}, actual {actual}"),
                None,
            ))
        }
    }

    pub fn assert_last_scheduled(&self, expected: bool) -> Result<&Self, ScenarioError> {
        let Some(step) = self.operation_trace.last() else {
            return Err(self.assertion_failure(
                "scheduling",
                "scenario has no recorded operation",
                None,
            ));
        };
        let actual = step.outcome.invalidation.starts_with("Scheduled(");
        if actual == expected {
            Ok(self)
        } else {
            Err(self.assertion_failure(
                "scheduling",
                format!(
                    "expected scheduled={expected}, actual invalidation={}",
                    step.outcome.invalidation
                ),
                None,
            ))
        }
    }

    fn assertion_failure(
        &self,
        assertion: impl Into<String>,
        detail: impl Into<String>,
        target: Option<&UiId>,
    ) -> ScenarioError {
        let nodes = self.host.semantic_nodes();
        let topology = nodes
            .iter()
            .take(12)
            .map(|node| {
                format!(
                    "{}({})",
                    node.id.as_str(),
                    node.name.as_deref().unwrap_or("unnamed")
                )
            })
            .collect();
        let suggestions =
            target.map_or_else(Vec::new, |target| suggest_targets(target.as_str(), &nodes));
        ScenarioError::AssertionFailed(ScenarioAssertionFailure {
            assertion: assertion.into(),
            detail: detail.into(),
            topology,
            suggestions,
        })
    }

    pub fn trace_document(&self, fixture: impl Into<String>) -> TraceDocument {
        TraceDocument {
            schema: 5,
            fixture: fixture.into(),
            viewport: self.initial_viewport,
            initial_semantic_digest: self.initial_semantic_digest.clone(),
            final_semantic_digest: semantic_digest(&self.host.semantic_nodes()),
            steps: self
                .trace
                .iter()
                .map(|step| TraceStepRecord {
                    target: step.target.as_str().to_owned(),
                    action: TraceAction::from(&step.action),
                    via: step.via,
                    frame_delta: step.frame_after.saturating_sub(step.frame_before),
                    changed: step.changed,
                    invalidation: step.invalidation.clone(),
                    messages: step.messages.clone(),
                    effects: step.effects.clone(),
                })
                .collect(),
            operations: self.operation_trace.clone(),
        }
    }

    pub fn replay(&mut self, document: &TraceDocument) -> Result<&mut Self, ScenarioError> {
        if !matches!(document.schema, 3..=5) {
            return Err(ScenarioError::UnsupportedTraceSchema {
                found: document.schema,
            });
        }
        if document.viewport != self.initial_viewport
            || document.initial_semantic_digest != self.initial_semantic_digest
        {
            return Err(replay_drift(
                0,
                format!(
                    "viewport={:?}, digest={}",
                    document.viewport, document.initial_semantic_digest
                ),
                format!(
                    "viewport={:?}, digest={}",
                    self.initial_viewport, self.initial_semantic_digest
                ),
            ));
        }
        for (index, expected) in document.steps.iter().enumerate() {
            let trace_index = self.trace.len();
            let selector = Selector::id(expected.target.as_str());
            if expected.action == TraceAction::Activate {
                self.activate_via(expected.via, &selector)?;
            } else if expected.via == ActivationVia::Semantic {
                self.perform(&selector, expected.action.to_semantic())?;
            } else {
                return Err(replay_drift(
                    index,
                    format!("{expected:?}"),
                    "unsupported nonsemantic route",
                ));
            }
            let actual = &self.trace[trace_index];
            if actual.target.as_str() != expected.target
                || TraceAction::from(&actual.action) != expected.action
                || actual.via != expected.via
                || actual.frame_after.saturating_sub(actual.frame_before) != expected.frame_delta
                || actual.changed != expected.changed
                || (document.schema >= 4 && actual.invalidation != expected.invalidation)
                || (document.schema >= 4 && actual.messages != expected.messages)
                || (document.schema >= 5 && actual.effects != expected.effects)
            {
                return Err(replay_drift(
                    index,
                    format!("{expected:?}"),
                    format!("{actual:?}"),
                ));
            }
        }
        for (index, expected) in document.operations.iter().enumerate() {
            let trace_index = self.operation_trace.len();
            self.replay_operation(&expected.operation)
                .map_err(|error| {
                    replay_drift(
                        document.steps.len() + index,
                        format!("{:?}", expected.operation),
                        error.to_string(),
                    )
                })?;
            let actual = &self.operation_trace[trace_index];
            if actual != expected {
                return Err(replay_drift(
                    document.steps.len() + index,
                    format!("{expected:?}"),
                    format!("{actual:?}"),
                ));
            }
        }
        if semantic_digest(&self.host.semantic_nodes()) != document.final_semantic_digest {
            return Err(replay_drift(
                document.steps.len(),
                document.final_semantic_digest.clone(),
                semantic_digest(&self.host.semantic_nodes()),
            ));
        }
        Ok(self)
    }

    fn replay_operation(&mut self, operation: &ScenarioOperation) -> Result<(), ScenarioError> {
        match operation {
            ScenarioOperation::Resize {
                width,
                height,
                scale,
            } => {
                self.resize(*width, *height, *scale)?;
            }
            ScenarioOperation::AdvanceTime { ticks } => {
                self.advance_time(*ticks)?;
            }
            ScenarioOperation::Focus { gained } => {
                self.window_focus(*gained)?;
            }
            ScenarioOperation::Suspend => {
                self.suspend()?;
            }
            ScenarioOperation::Close => {
                self.close()?;
            }
            ScenarioOperation::PointerMove { target } => {
                self.pointer_move(&Selector::id(target.as_str()))?;
            }
            ScenarioOperation::PointerActivate { target } => {
                self.pointer_activate(&Selector::id(target.as_str()))?;
            }
            ScenarioOperation::PointerContext { target } => {
                self.pointer_context(&Selector::id(target.as_str()))?;
            }
            ScenarioOperation::PointerDrag { from, to } => {
                self.pointer_drag(&Selector::id(from.as_str()), &Selector::id(to.as_str()))?;
            }
            ScenarioOperation::PointerScroll {
                target,
                delta_x,
                delta_y,
            } => {
                self.pointer_scroll(&Selector::id(target.as_str()), *delta_x, *delta_y)?;
            }
            ScenarioOperation::TouchActivate { target, contact } => {
                self.touch_activate(&Selector::id(target.as_str()), *contact)?;
            }
            ScenarioOperation::TouchContext { target } => {
                self.touch_context(&Selector::id(target.as_str()))?;
            }
            ScenarioOperation::TouchDrag { from, to, contact } => {
                self.touch_drag(
                    &Selector::id(from.as_str()),
                    &Selector::id(to.as_str()),
                    *contact,
                )?;
            }
            ScenarioOperation::TouchCancel { target, contact } => {
                self.touch_cancel(&Selector::id(target.as_str()), *contact)?;
            }
            ScenarioOperation::KeyboardFocus { direction } => {
                self.keyboard_focus(*direction)?;
            }
            ScenarioOperation::TextInput { text } => {
                self.text_input(text.clone())?;
            }
            ScenarioOperation::ImePreedit { text } => {
                self.ime_preedit(text.clone())?;
            }
            ScenarioOperation::ClipboardPaste { text } => {
                self.clipboard_paste(text.clone())?;
            }
            ScenarioOperation::Controller { action } => {
                let action = parse_controller_action(action).ok_or_else(|| {
                    replay_drift(
                        self.operation_trace.len(),
                        "known controller action",
                        action,
                    )
                })?;
                self.controller(action)?;
            }
            ScenarioOperation::ControllerSemantic { target, action } => {
                let semantic = action.to_semantic();
                let kind = match semantic {
                    SemanticAction::Invoke(kind) => kind,
                    SemanticAction::SetValue(_) => ActionKind::SetValue,
                };
                self.controller_semantic_action(&Selector::id(target.as_str()), kind)?;
            }
            ScenarioOperation::PlatformCapabilityChanged {
                capability,
                available,
            } => {
                self.platform_capability(capability.clone(), *available)?;
            }
            ScenarioOperation::KeyboardActivate => {
                self.keyboard_activate_focused()?;
            }
            ScenarioOperation::KeyboardContext => {
                self.keyboard_context_focused()?;
            }
            ScenarioOperation::Accessibility { target, action } => {
                let SemanticAction::Invoke(kind) = action.to_semantic() else {
                    return Err(replay_drift(
                        self.operation_trace.len(),
                        "accessibility invoke action",
                        format!("{action:?}"),
                    ));
                };
                self.accessibility_action(&Selector::id(target.as_str()), kind)?;
            }
            ScenarioOperation::Semantic { target, action } => {
                self.semantic_operation(&Selector::id(target.as_str()), action.to_semantic())?;
            }
            ScenarioOperation::DomainCompletion { id, payload_type } => {
                return Err(replay_drift(
                    self.operation_trace.len(),
                    format!("registered completion decoder for {id}:{payload_type}"),
                    "no source-free decoder registered",
                ));
            }
        }
        Ok(())
    }
}

fn semantic_digest(nodes: &[SemanticNodeSnapshot]) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    let mut write = |bytes: &[u8]| {
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
    };
    for node in nodes {
        write(node.id.as_str().as_bytes());
        write(&[0]);
        if let Some(parent) = &node.parent {
            write(parent.as_str().as_bytes());
        }
        write(&[0]);
        if let Some(role) = node.role {
            write(role.as_str().as_bytes());
        }
        write(&[0]);
        if let Some(name) = &node.name {
            write(name.as_bytes());
        }
        write(&[u8::from(node.enabled), u8::from(node.focused)]);
        for action in &node.actions {
            write(format!("{action:?}").as_bytes());
            write(&[0]);
        }
    }
    format!("fnv1a64:{hash:016x}")
}

fn replay_drift(
    step: usize,
    expected: impl Into<String>,
    actual: impl Into<String>,
) -> ScenarioError {
    ScenarioError::ReplayDrift {
        step,
        expected: expected.into(),
        actual: actual.into(),
    }
}

fn center(rect: nickel_ui::Rect) -> Point {
    Point {
        x: rect.origin.x + rect.size.width / 2.0,
        y: rect.origin.y + rect.size.height / 2.0,
    }
}

fn input_point(point: Point) -> nickel_input::Point {
    nickel_input::Point {
        x: f64::from(point.x),
        y: f64::from(point.y),
    }
}

fn rect_intersects(left: nickel_ui::Rect, right: nickel_ui::Rect) -> bool {
    left.origin.x < right.origin.x + right.size.width
        && left.origin.x + left.size.width > right.origin.x
        && left.origin.y < right.origin.y + right.size.height
        && left.origin.y + left.size.height > right.origin.y
}

fn merge_outcome(into: &mut HostEventOutcome, next: HostEventOutcome) {
    into.changed |= next.changed;
    into.invalidation = into.invalidation.merge(next.invalidation);
    into.messages.extend(next.messages);
    into.effects.extend(next.effects);
    into.completion_failures.extend(next.completion_failures);
    into.pointer_icon = next.pointer_icon;
    into.text_input_active = next.text_input_active;
    into.accessibility_generation = next.accessibility_generation;
    into.change_token = next.change_token;
    into.telemetry.events_processed += next.telemetry.events_processed;
    into.telemetry.completions_processed += next.telemetry.completions_processed;
    into.telemetry.rebuilt |= next.telemetry.rebuilt;
    into.telemetry.input_to_message_us = into
        .telemetry
        .input_to_message_us
        .saturating_add(next.telemetry.input_to_message_us);
    into.telemetry.input_to_frame_us = into
        .telemetry
        .input_to_frame_us
        .saturating_add(next.telemetry.input_to_frame_us);
    into.telemetry.layout_us = into
        .telemetry
        .layout_us
        .saturating_add(next.telemetry.layout_us);
    into.telemetry.paint_list_us = into
        .telemetry
        .paint_list_us
        .saturating_add(next.telemetry.paint_list_us);
    into.telemetry.scheduled_wakeups = into
        .telemetry
        .scheduled_wakeups
        .saturating_add(next.telemetry.scheduled_wakeups);
    into.telemetry.retained_frame_bytes = into
        .telemetry
        .retained_frame_bytes
        .max(next.telemetry.retained_frame_bytes);
    into.telemetry.allocation_count = match (
        into.telemetry.allocation_count,
        next.telemetry.allocation_count,
    ) {
        (Some(left), Some(right)) => Some(left.saturating_add(right)),
        _ => None,
    };
    if next.clipboard_text.is_some() {
        into.clipboard_text = next.clipboard_text;
    }
    into.semantic_failures.extend(next.semantic_failures);
    into.global_actions.extend(next.global_actions);
}

fn outcome_snapshot(outcome: &HostEventOutcome) -> OperationOutcomeSnapshot {
    OperationOutcomeSnapshot {
        changed: outcome.changed,
        invalidation: format!("{:?}", outcome.invalidation),
        messages: message_evidence_snapshot(outcome),
        effects: effect_evidence_snapshot(outcome),
        completion_failures: outcome
            .completion_failures
            .iter()
            .map(|failure| format!("{}:{}", failure.id, failure.detail))
            .collect(),
        pointer_icon: format!("{:?}", outcome.pointer_icon),
        text_input_active: outcome.text_input_active,
        accessibility_generation: outcome.accessibility_generation,
        events_processed: outcome.telemetry.events_processed,
        completions_processed: outcome.telemetry.completions_processed,
        rebuilt: outcome.telemetry.rebuilt,
        change_frame_generation: outcome.change_token.frame_generation,
        change_semantic_generation: outcome.change_token.semantic_generation,
        clipboard_text: outcome.clipboard_text.clone(),
        semantic_failures: outcome
            .semantic_failures
            .iter()
            .map(|failure| format!("{}:{:?}", failure.target.as_str(), failure.error))
            .collect(),
        global_actions: outcome
            .global_actions
            .iter()
            .map(|action| format!("{action:?}"))
            .collect(),
    }
}

fn message_evidence_snapshot(outcome: &HostEventOutcome) -> Vec<MessageEvidenceSnapshot> {
    outcome
        .messages
        .iter()
        .map(|message| MessageEvidenceSnapshot {
            type_name: message.type_name.to_owned(),
            label: message.label.clone(),
        })
        .collect()
}

fn effect_evidence_snapshot(outcome: &HostEventOutcome) -> Vec<EffectEvidenceSnapshot> {
    outcome
        .effects
        .iter()
        .map(|effect| EffectEvidenceSnapshot {
            type_name: effect.type_name.to_owned(),
            label: effect.label.clone(),
        })
        .collect()
}

fn debug_digest(value: &impl fmt::Debug) -> String {
    let text = format!("{value:?}");
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("fnv1a64:{hash:016x}")
}

fn suggest_targets(needle: &str, nodes: &[SemanticNodeSnapshot]) -> Vec<String> {
    let needle_segments = needle.split('/').collect::<BTreeSet<_>>();
    let mut scored = nodes
        .iter()
        .map(|node| {
            let score = node
                .id
                .as_str()
                .split('/')
                .filter(|segment| needle_segments.contains(segment))
                .count();
            (score, node.id.as_str().to_owned())
        })
        .collect::<Vec<_>>();
    scored.sort_by(|left, right| right.cmp(left));
    scored
        .into_iter()
        .filter(|(score, _)| *score > 0)
        .take(5)
        .map(|(_, id)| id)
        .collect()
}

fn selector_needle(selector: &Selector) -> &str {
    match selector {
        Selector::Id(id) => id.as_str(),
        Selector::KeyedItem { key, .. } => key,
        Selector::Role(_) | Selector::Action(_) => "root",
        Selector::RoleAndName { name, .. } => name,
    }
}

fn missing_target_error(
    selector: String,
    needle: &str,
    nodes: &[SemanticNodeSnapshot],
) -> ScenarioError {
    ScenarioError::MissingTarget {
        selector,
        suggestions: suggest_targets(needle, nodes),
        topology: nodes
            .iter()
            .take(12)
            .map(|node| node.id.as_str().to_owned())
            .collect(),
    }
}

#[derive(Clone, Copy, Debug)]
enum ControllerGraphStep {
    Next,
    Action(ControllerAction),
}

struct ControllerGraphResult {
    routes: std::collections::BTreeMap<String, Vec<ControllerGraphStep>>,
    failure: Option<String>,
}

fn apply_controller_graph_step<A: Application>(
    scenario: &mut Scenario<A>,
    step: ControllerGraphStep,
) {
    match step {
        ControllerGraphStep::Next => {
            scenario.host.handle_event(UiEvent::ControllerNext);
        }
        ControllerGraphStep::Action(action) => {
            scenario.host.handle_controller_action(action);
        }
    }
}

fn explore_controller_routes<A: Application>(
    factory: &impl Fn() -> Scenario<A>,
    targets: &BTreeSet<String>,
    policy: &ReachabilityPolicy,
) -> ControllerGraphResult {
    let mut routes = std::collections::BTreeMap::new();
    // Seed structural traversal from the default pane and nearby shoulder
    // peers. Ordinary pane spines are recorded once before spatial BFS.
    let mut initial_prefixes = vec![Vec::new()];
    for action in [ControllerAction::PreviousPane, ControllerAction::NextPane] {
        for count in 1..=3 {
            initial_prefixes.push(vec![ControllerGraphStep::Action(action); count]);
        }
    }
    for initial_prefix in initial_prefixes {
        let mut seed = factory();
        for step in &initial_prefix {
            apply_controller_graph_step(&mut seed, *step);
        }
        let mut seed_prefix = initial_prefix;
        let mut seed_states = BTreeSet::new();
        let mut repeat_recoveries = 0;
        for _ in seed_prefix.len()..policy.maximum_path_length {
            let inspection = seed.host.inspect();
            if let Some(current) = inspection.controller_target.as_ref()
                && targets.contains(current.as_str())
            {
                routes
                    .entry(current.as_str().to_owned())
                    .or_insert(seed_prefix.clone());
            }
            if routes.len() == targets.len() {
                return ControllerGraphResult {
                    routes,
                    failure: None,
                };
            }
            let state = (
                inspection.controller_target.clone(),
                inspection.controller_scope.clone(),
            );
            let repeated = !seed_states.insert(state);
            if repeated && repeat_recoveries > 0 {
                break;
            }
            if seed_states.len() > policy.maximum_state_count {
                return ControllerGraphResult {
                    routes,
                    failure: Some(format!(
                        "controller graph exploration exceeded the {} state ceiling during structural seeding",
                        policy.maximum_state_count
                    )),
                };
            }
            let current_node = inspection.controller_target.as_ref().and_then(|target| {
                seed.host
                    .semantic_nodes()
                    .into_iter()
                    .find(|node| &node.id == target)
            });
            let step = if repeated {
                repeat_recoveries += 1;
                ControllerGraphStep::Action(ControllerAction::Cancel)
            } else if inspection.controller_target.is_some()
                && current_node
                    .as_ref()
                    .is_none_or(controller_node_reveals_topology)
            {
                ControllerGraphStep::Action(ControllerAction::Confirm)
            } else {
                ControllerGraphStep::Next
            };
            apply_controller_graph_step(&mut seed, step);
            seed_prefix.push(step);
        }
    }

    let mut queue = VecDeque::from([Vec::<ControllerGraphStep>::new()]);
    let mut visited = BTreeSet::new();
    let mut last_trace = Vec::new();

    while let Some(prefix) = queue.pop_front() {
        if prefix.len() > policy.maximum_path_length {
            continue;
        }

        let mut scenario = factory();
        let mut trace = Vec::with_capacity(prefix.len() + 1);
        for controller in &prefix {
            apply_controller_graph_step(&mut scenario, *controller);
            let inspection = scenario.host.inspect();
            trace.push(format!(
                "controller_{controller:?}:{}@{}{}",
                inspection
                    .controller_target
                    .as_ref()
                    .map_or("none", UiId::as_str),
                inspection
                    .controller_scope
                    .as_ref()
                    .map_or("root", UiId::as_str),
                if inspection.controller_editing {
                    ":editing"
                } else {
                    ""
                }
            ));
        }
        last_trace = trace.clone();
        let inspection = scenario.host.inspect();
        if let Some(current) = inspection.controller_target.as_ref() {
            let current = current.as_str();
            if targets.contains(current) {
                routes.entry(current.to_owned()).or_insert(prefix.clone());
                if routes.len() == targets.len() {
                    return ControllerGraphResult {
                        routes,
                        failure: None,
                    };
                }
            }
        }

        let state_key = format!(
            "{}|target={}|scope={}|editing={}|overlay={:?}",
            semantic_digest(&scenario.semantic_nodes()),
            inspection
                .controller_target
                .as_ref()
                .map_or("none", UiId::as_str),
            inspection
                .controller_scope
                .as_ref()
                .map_or("root", UiId::as_str),
            inspection.controller_editing,
            inspection.open_overlay,
        );
        if !visited.insert(state_key) {
            continue;
        }
        if visited.len() >= policy.maximum_state_count {
            return ControllerGraphResult {
                routes,
                failure: Some(format!(
                    "controller BFS exceeded the {} state ceiling; trace={}",
                    policy.maximum_state_count,
                    trace.join(" -> ")
                )),
            };
        }
        if prefix.len() == policy.maximum_path_length {
            continue;
        }
        let semantic_nodes = scenario.host.semantic_nodes();
        let current_node = inspection
            .controller_target
            .as_ref()
            .and_then(|target| semantic_nodes.iter().find(|node| &node.id == target));
        let mut branches = vec![
            ControllerGraphStep::Next,
            ControllerGraphStep::Action(ControllerAction::Up),
            ControllerGraphStep::Action(ControllerAction::Down),
            ControllerGraphStep::Action(ControllerAction::Left),
            ControllerGraphStep::Action(ControllerAction::Right),
            ControllerGraphStep::Action(ControllerAction::PreviousPane),
            ControllerGraphStep::Action(ControllerAction::NextPane),
        ];
        let is_scope_waypoint = inspection.controller_target.is_some() && current_node.is_none();
        let reveals_topology = current_node.is_some_and(controller_node_reveals_topology);
        if is_scope_waypoint || reveals_topology {
            branches.push(ControllerGraphStep::Action(ControllerAction::Confirm));
        }
        if inspection.controller_scope.is_some()
            || inspection.controller_editing
            || inspection.open_overlay.is_some()
        {
            branches.push(ControllerGraphStep::Action(ControllerAction::Cancel));
        }
        for controller in branches {
            let mut next = prefix.clone();
            next.push(controller);
            if matches!(controller, ControllerGraphStep::Next) {
                // Structural traversal is deterministic and usually advances
                // to a new production target. Explore that cheap spine before
                // replaying every spatial branch from the same prefix.
                queue.push_front(next);
            } else {
                queue.push_back(next);
            }
        }
    }

    if routes.len() == targets.len() {
        ControllerGraphResult {
            routes,
            failure: None,
        }
    } else {
        ControllerGraphResult {
            routes,
            failure: Some(format!(
                "controller BFS exhausted {} production-observable states within the {} step path ceiling, reaching fewer than {} targets; trace={}",
                visited.len(),
                policy.maximum_path_length,
                targets.len(),
                last_trace.join(" -> ")
            )),
        }
    }
}

fn controller_node_reveals_topology(node: &SemanticNodeSnapshot) -> bool {
    node.navigation_scope
        || node.actions.iter().any(|action| {
            matches!(
                action,
                ActionKind::Expand | ActionKind::EnterNavigation | ActionKind::ContextMenu
            )
        })
        || matches!(
            node.role,
            Some(SemanticRole::Grid | SemanticRole::Menu | SemanticRole::Dialog)
        )
}

fn replay_controller_route<A: Application>(
    factory: &impl Fn() -> Scenario<A>,
    prefix: &[ControllerGraphStep],
    target: &SemanticNodeSnapshot,
    action: ActionKind,
) -> Result<(Scenario<A>, Vec<String>), String> {
    let mut scenario = factory();
    let mut trace = Vec::with_capacity(prefix.len() + 1);
    for controller in prefix {
        apply_controller_graph_step(&mut scenario, *controller);
        let inspection = scenario.host.inspect();
        trace.push(format!(
            "controller_{controller:?}:{}@{}",
            inspection
                .controller_target
                .as_ref()
                .map_or("none", UiId::as_str),
            inspection
                .controller_scope
                .as_ref()
                .map_or("root", UiId::as_str),
        ));
    }
    let inspection = scenario.host.inspect();
    if inspection.controller_target.as_ref() != Some(&target.id) {
        return Err(format!(
            "controller BFS predecessor drifted before {}",
            target.id.as_str()
        ));
    }
    let (controller, outcome) = match action {
        ActionKind::Activate => (
            ControllerAction::Confirm,
            scenario
                .host
                .handle_controller_action(ControllerAction::Confirm),
        ),
        ActionKind::ContextMenu => (
            ControllerAction::ContextMenu,
            scenario
                .host
                .handle_controller_action(ControllerAction::ContextMenu),
        ),
        ActionKind::Increment | ActionKind::Decrement => {
            if !inspection.controller_editing {
                scenario
                    .host
                    .handle_controller_action(ControllerAction::Confirm);
            }
            let delta = if action == ActionKind::Increment {
                1.0
            } else {
                -1.0
            };
            let controller = if delta > 0.0 {
                ControllerAction::Right
            } else {
                ControllerAction::Left
            };
            (
                controller,
                scenario.host.handle_event(UiEvent::ControllerAdjust(delta)),
            )
        }
        _ => {
            return Err(format!(
                "controller route for {action:?} is not implemented"
            ));
        }
    };
    if !outcome.changed || !outcome.semantic_failures.is_empty() {
        return Err(format!("controller {action:?} produced no transition"));
    }
    trace.push(format!("controller:{controller:?}:{}", target.id.as_str()));
    Ok((scenario, trace))
}

fn route_controller_scroll<A: Application>(
    factory: &impl Fn() -> Scenario<A>,
    target: &SemanticNodeSnapshot,
    action: ActionKind,
    maximum_path_length: usize,
) -> Result<(Scenario<A>, Vec<String>), String> {
    let initial = match target.value {
        Some(SemanticValueSnapshot::Number { value, .. }) => value,
        _ => return Err("controller scrollbar route requires a numeric semantic value".into()),
    };
    let moved_in_direction = |value: f64| match action {
        ActionKind::Increment => value > initial,
        ActionKind::Decrement => value < initial,
        _ => false,
    };
    let mut scenario = factory();
    let mut steps = Vec::new();
    for index in 0..maximum_path_length {
        scenario.host.handle_event(UiEvent::ControllerNext);
        let inspection = scenario.host.inspect();
        steps.push(format!(
            "controller_Next:{}@{}",
            inspection
                .controller_target
                .as_ref()
                .map_or("none", UiId::as_str),
            inspection
                .controller_scope
                .as_ref()
                .map_or("root", UiId::as_str),
        ));
        let current = scenario
            .semantic_nodes()
            .into_iter()
            .find(|node| node.id == target.id)
            .and_then(|node| match node.value {
                Some(SemanticValueSnapshot::Number { value, .. }) => Some(value),
                _ => None,
            })
            .ok_or_else(|| {
                format!(
                    "controller navigation removed scrollbar {}",
                    target.id.as_str()
                )
            })?;
        if moved_in_direction(current) {
            steps.push(format!(
                "controller:{action:?}:{}:{initial}->{current}",
                target.id.as_str()
            ));
            return Ok((scenario, steps));
        }
        if index + 1 == maximum_path_length {
            break;
        }
    }
    Err(format!(
        "controller navigation did not {action:?} scrollbar {} within {maximum_path_length} steps",
        target.id.as_str()
    ))
}

fn route_action<A: Application>(
    scenario: &mut Scenario<A>,
    selector: &Selector,
    action: ActionKind,
    modality: ReachabilityModality,
    maximum_path_length: usize,
    audit_started: Instant,
    wall_ceiling: Duration,
) -> Result<Vec<String>, String> {
    let target = scenario
        .resolve_target(selector)
        .map_err(|error| error.to_string())?;
    match modality {
        ReachabilityModality::Accessibility => {
            let outcome = scenario.host.perform_accessibility_action(
                target.id.clone(),
                semantic_action_for_target(&target, action),
            );
            if outcome.changed && outcome.semantic_failures.is_empty() {
                Ok(vec![format!(
                    "accessibility:{action:?}:{}",
                    target.id.as_str()
                )])
            } else {
                Err(format!(
                    "accessibility {action:?} was advertised but produced no transition"
                ))
            }
        }
        ReachabilityModality::Keyboard => {
            let mut steps = Vec::new();
            for _ in 0..maximum_path_length {
                if audit_started.elapsed() >= wall_ceiling {
                    return Err(format!(
                        "keyboard route exceeded the {}ms wall ceiling",
                        wall_ceiling.as_millis()
                    ));
                }
                let inspection = scenario.host.inspect();
                if inspection.keyboard_focus.as_ref() == Some(&target.id) {
                    let event = match action {
                        ActionKind::Activate => UiEvent::KeyboardActivate,
                        ActionKind::ContextMenu => UiEvent::KeyboardContextMenu,
                        _ => {
                            return Err(format!(
                                "keyboard route for {action:?} is not implemented"
                            ));
                        }
                    };
                    let outcome = scenario.host.handle_event(event);
                    if outcome.changed && outcome.semantic_failures.is_empty() {
                        steps.push(format!("keyboard:{action:?}:{}", target.id.as_str()));
                        return Ok(steps);
                    }
                    return Err(format!("keyboard {action:?} produced no transition"));
                }
                scenario.host.handle_event(UiEvent::FocusNext);
                let reached = scenario
                    .host
                    .inspect()
                    .keyboard_focus
                    .map(|id| id.as_str().to_owned())
                    .unwrap_or_else(|| "none".into());
                steps.push(format!("focus_next:{reached}"));
            }
            Err(format!(
                "keyboard focus path exceeded {maximum_path_length} steps"
            ))
        }
        ReachabilityModality::Controller => {
            let mut steps = Vec::new();
            let mut visited = BTreeSet::new();
            for _ in 0..maximum_path_length {
                if audit_started.elapsed() >= wall_ceiling {
                    return Err(format!(
                        "controller route exceeded the {}ms wall ceiling",
                        wall_ceiling.as_millis()
                    ));
                }
                let inspection = scenario.host.inspect();
                if inspection.controller_target.as_ref() == Some(&target.id) {
                    let (controller, outcome) = match action {
                        ActionKind::Activate => (
                            ControllerAction::Confirm,
                            scenario
                                .host
                                .handle_controller_action(ControllerAction::Confirm),
                        ),
                        ActionKind::ContextMenu => (
                            ControllerAction::ContextMenu,
                            scenario
                                .host
                                .handle_controller_action(ControllerAction::ContextMenu),
                        ),
                        ActionKind::Increment => (ControllerAction::Right, {
                            if !inspection.controller_editing {
                                scenario
                                    .host
                                    .handle_controller_action(ControllerAction::Confirm);
                            }
                            scenario.host.handle_event(UiEvent::ControllerAdjust(1.0))
                        }),
                        ActionKind::Decrement => (ControllerAction::Left, {
                            if !inspection.controller_editing {
                                scenario
                                    .host
                                    .handle_controller_action(ControllerAction::Confirm);
                            }
                            scenario.host.handle_event(UiEvent::ControllerAdjust(-1.0))
                        }),
                        _ => {
                            return Err(format!(
                                "controller route for {action:?} is not implemented"
                            ));
                        }
                    };
                    if outcome.changed && outcome.semantic_failures.is_empty() {
                        steps.push(format!("controller:{controller:?}:{}", target.id.as_str()));
                        return Ok(steps);
                    }
                    return Err(format!("controller {action:?} produced no transition"));
                }
                let state_key = (
                    inspection
                        .controller_target
                        .as_ref()
                        .map(|id| id.as_str().to_owned()),
                    inspection
                        .controller_scope
                        .as_ref()
                        .map(|id| id.as_str().to_owned()),
                );
                if !visited.insert(state_key) {
                    scenario
                        .host
                        .handle_controller_action(ControllerAction::Cancel);
                    let reached = scenario.host.inspect();
                    let reached_target = reached
                        .controller_target
                        .map(|id| id.as_str().to_owned())
                        .unwrap_or_else(|| "none".into());
                    let reached_scope = reached
                        .controller_scope
                        .map(|id| id.as_str().to_owned())
                        .unwrap_or_else(|| "root".into());
                    steps.push(format!("controller_back:{reached_target}@{reached_scope}"));
                    continue;
                }
                let entering_scope = inspection.controller_target.as_ref().is_some_and(|id| {
                    !scenario
                        .host
                        .semantic_nodes()
                        .iter()
                        .any(|node| &node.id == id)
                });
                if entering_scope {
                    scenario
                        .host
                        .handle_controller_action(ControllerAction::Confirm);
                } else {
                    scenario.host.handle_event(UiEvent::ControllerNext);
                }
                let reached = scenario.host.inspect();
                let reached_target = reached
                    .controller_target
                    .map(|id| id.as_str().to_owned())
                    .unwrap_or_else(|| "none".into());
                let reached_scope = reached
                    .controller_scope
                    .map(|id| id.as_str().to_owned())
                    .unwrap_or_else(|| "root".into());
                steps.push(format!(
                    "controller_{}:{reached_target}@{reached_scope}",
                    if entering_scope { "enter" } else { "next" }
                ));
            }
            Err(format!(
                "controller path exceeded {maximum_path_length} steps; trace={}",
                steps.join(" -> ")
            ))
        }
    }
}

fn semantic_action_for_target(target: &SemanticNodeSnapshot, action: ActionKind) -> SemanticAction {
    if action != ActionKind::SetValue {
        return SemanticAction::Invoke(action);
    }
    let value = match target.value.as_ref() {
        Some(SemanticValueSnapshot::Boolean(value)) => SemanticValueInput::Boolean(!value),
        Some(SemanticValueSnapshot::Number {
            value,
            minimum,
            maximum,
            step,
        }) => {
            let next = if value + step <= *maximum {
                value + step
            } else {
                (value - step).max(*minimum)
            };
            SemanticValueInput::Number(next)
        }
        Some(SemanticValueSnapshot::Text(value)) => {
            SemanticValueInput::Text(format!("{value} scenario"))
        }
        Some(SemanticValueSnapshot::ProtectedText { .. }) => {
            SemanticValueInput::Text("scenario protected value".into())
        }
        None => SemanticValueInput::Number(0.5),
    };
    SemanticAction::SetValue(value)
}

fn parse_controller_action(value: &str) -> Option<ControllerAction> {
    Some(match value {
        "Launcher" => ControllerAction::Launcher,
        "Up" => ControllerAction::Up,
        "Down" => ControllerAction::Down,
        "Left" => ControllerAction::Left,
        "Right" => ControllerAction::Right,
        "Confirm" => ControllerAction::Confirm,
        "Cancel" => ControllerAction::Cancel,
        "ContextMenu" => ControllerAction::ContextMenu,
        "PreviousPane" => ControllerAction::PreviousPane,
        "NextPane" => ControllerAction::NextPane,
        _ => return None,
    })
}

fn operation_kind(operation: &ScenarioOperation) -> &'static str {
    match operation {
        ScenarioOperation::Resize { .. } => "resize",
        ScenarioOperation::AdvanceTime { .. } => "advance_time",
        ScenarioOperation::Focus { .. } => "focus",
        ScenarioOperation::Suspend => "suspend",
        ScenarioOperation::Close => "close",
        ScenarioOperation::PointerMove { .. } => "pointer_move",
        ScenarioOperation::PointerActivate { .. } => "pointer_activate",
        ScenarioOperation::PointerContext { .. } => "pointer_context",
        ScenarioOperation::PointerDrag { .. } => "pointer_drag",
        ScenarioOperation::PointerScroll { .. } => "pointer_scroll",
        ScenarioOperation::TouchActivate { .. } => "touch_activate",
        ScenarioOperation::TouchContext { .. } => "touch_context",
        ScenarioOperation::TouchDrag { .. } => "touch_drag",
        ScenarioOperation::TouchCancel { .. } => "touch_cancel",
        ScenarioOperation::KeyboardFocus { .. } => "keyboard_focus",
        ScenarioOperation::KeyboardActivate => "keyboard_activate",
        ScenarioOperation::KeyboardContext => "keyboard_context",
        ScenarioOperation::TextInput { .. } => "text_input",
        ScenarioOperation::ImePreedit { .. } => "ime_preedit",
        ScenarioOperation::ClipboardPaste { .. } => "clipboard_paste",
        ScenarioOperation::Controller { .. } => "controller",
        ScenarioOperation::ControllerSemantic { .. } => "controller_semantic",
        ScenarioOperation::Accessibility { .. } => "accessibility",
        ScenarioOperation::Semantic { .. } => "semantic",
        ScenarioOperation::PlatformCapabilityChanged { .. } => "platform_capability",
        ScenarioOperation::DomainCompletion { .. } => "domain_completion",
    }
}

pub fn open<F: Fixture>() -> Scenario<F::App> {
    let (width, height) = F::surface_size();
    Scenario::new(F::create(), width, height)
}

#[cfg(test)]
mod tests {
    use nickel_ui::{
        Button, Collection, CollectionState, ComponentBuilderExt, Container, DiagnosticKind,
        RadioGroup, RadioOption, SemanticTheme, SemanticTokenSet, Spacer, TabList, Text, TextField,
    };

    use super::*;

    #[test]
    fn scaled_fixture_surfaces_resolve_in_logical_pixels() {
        let variant = FixtureVariant {
            viewport: ViewportPreset {
                id: "physical",
                width: 1025,
                height: 775,
            },
            scale: ScalePreset {
                id: "1.25x",
                factor: 1.25,
            },
            ..DEFAULT_VARIANT
        };
        assert_eq!(logical_fixture_size(&variant), (820, 620));
    }

    #[derive(Clone, Debug, PartialEq)]
    enum Message {
        Increment,
        Query(String),
    }

    #[derive(Default)]
    struct Counter {
        count: usize,
        query: String,
    }

    impl Application for Counter {
        type Message = Message;

        fn update(&mut self, message: Self::Message) {
            match message {
                Message::Increment => self.count += 1,
                Message::Query(query) => self.query = query,
            }
        }

        fn view(&self, _context: nickel_ui::ViewContext) -> impl nickel_ui::View<Self::Message> {
            nickel_ui::Column::new()
                .child(Button::new(Message::Increment, "Increment").id("increment"))
                .child(Text::new(format!("Count: {}", self.count)))
                .child(
                    TextField::on_change(&self.query, Message::Query)
                        .id("query")
                        .accessibility_label("Query"),
                )
        }
    }

    #[test]
    fn scenario_drives_production_semantics_without_geometry_or_state_setters() {
        let mut scenario = Scenario::new(Counter::default(), 320, 160);
        scenario
            .activate(&Selector::role_name(SemanticRole::Button, "Increment"))
            .expect("activate button")
            .set_value(
                &Selector::id("root/query"),
                SemanticValueInput::Text("nickel".into()),
            )
            .expect("set text value");

        assert_eq!(scenario.host_mut().application_mut().count, 1);
        assert_eq!(scenario.host_mut().application_mut().query, "nickel");
        assert_eq!(scenario.trace().len(), 2);
        assert!(scenario.trace().iter().all(|step| step.changed));
        assert!(
            scenario
                .trace()
                .iter()
                .all(|step| !step.messages.is_empty() && !step.invalidation.is_empty())
        );
        assert!(
            scenario.trace()[0].messages[0]
                .type_name
                .contains("Message")
        );
    }

    #[test]
    fn every_activation_modality_reaches_the_same_application_transition() {
        let selector = Selector::role_name(SemanticRole::Button, "Increment");
        for via in [
            ActivationVia::Pointer,
            ActivationVia::Keyboard,
            ActivationVia::Controller,
            ActivationVia::Accessibility,
            ActivationVia::Semantic,
        ] {
            let mut scenario = Scenario::new(Counter::default(), 320, 160);
            scenario
                .activate_via(via, &selector)
                .unwrap_or_else(|error| panic!("{via:?} activation failed: {error}"));
            assert_eq!(scenario.host_mut().application_mut().count, 1, "{via:?}");
            assert_eq!(scenario.trace().len(), 1, "{via:?}");
            assert_eq!(scenario.trace()[0].via, via);
            assert_eq!(
                scenario.trace()[0].action,
                SemanticAction::Invoke(ActionKind::Activate)
            );
        }
    }

    #[test]
    fn touch_operations_use_normalized_production_delivery_and_cancel_without_activation() {
        let selector = Selector::id("root/increment");
        let mut activated = Scenario::new(Counter::default(), 320, 160);
        activated
            .touch_activate(&selector, 7)
            .expect("normalized touch activation");
        assert_eq!(activated.host_mut().application_mut().count, 1);
        activated
            .assert_modality(InputModality::Pointer)
            .expect("touch follows production pointer modality")
            .assert_last_message_count(1)
            .expect("one typed activation message")
            .assert_last_effect_count(0)
            .expect("no platform effects");
        assert_eq!(
            operation_kind(&activated.operation_trace()[0].operation),
            "touch_activate"
        );

        let mut cancelled = Scenario::new(Counter::default(), 320, 160);
        cancelled
            .touch_cancel(&selector, 8)
            .expect("normalized touch cancellation");
        assert_eq!(cancelled.host_mut().application_mut().count, 0);
        assert!(cancelled.host().inspect().pointer_capture.is_none());
        cancelled
            .assert_last_message_count(0)
            .expect("cancel emits no activation message");
    }

    #[test]
    fn typed_domain_completions_flow_through_application_contract_and_trace_failures() {
        struct CompletionApp(u32);
        impl Application for CompletionApp {
            type Message = Message;

            fn update(&mut self, _message: Self::Message) {}

            fn complete(
                &mut self,
                completion: Completion,
            ) -> Result<bool, nickel_ui::CompletionFailure> {
                let id = completion.id;
                let value =
                    completion
                        .downcast::<u32>()
                        .map_err(|_| nickel_ui::CompletionFailure {
                            id,
                            kind: nickel_ui::CompletionFailureKind::TypeMismatch,
                            detail: "expected u32".into(),
                        })?;
                self.0 = value;
                Ok(true)
            }

            fn view(
                &self,
                _context: nickel_ui::ViewContext,
            ) -> impl nickel_ui::View<Self::Message> {
                Text::new(format!("Completed: {}", self.0))
            }
        }

        let mut scenario = Scenario::new(CompletionApp(0), 200, 80);
        scenario
            .domain_completion("count", 42_u32)
            .expect("typed completion");
        assert_eq!(scenario.host_mut().application_mut().0, 42);
        let step = &scenario.operation_trace()[0];
        assert!(matches!(
            step.operation,
            ScenarioOperation::DomainCompletion { ref id, .. } if id == "count"
        ));
        assert!(step.outcome.completion_failures.is_empty());
    }

    #[test]
    fn registry_is_sorted_and_rejects_duplicate_ids() {
        struct CounterFixture;
        impl Fixture for CounterFixture {
            type App = Counter;

            fn metadata() -> &'static FixtureMetadata {
                static METADATA: FixtureMetadata = FixtureMetadata {
                    id: "counter",
                    title: "Counter",
                    description: "Counter semantics",
                    tags: &["core"],
                    source: FixtureSource {
                        crate_name: "nickel-ui-testkit",
                        file: file!(),
                        line: line!(),
                    },
                    variants: &[DEFAULT_VARIANT],
                    assets: &[],
                    simulated_effects: &[],
                };
                &METADATA
            }

            fn create() -> Self::App {
                Counter::default()
            }
        }

        let mut registry = FixtureRegistry::new();
        registry
            .register::<CounterFixture>()
            .expect("first fixture");
        assert_eq!(
            registry.register::<CounterFixture>(),
            Err(RegistryError::DuplicateId("counter".into()))
        );
        let entry = registry.finish()[0];
        assert_eq!(entry.metadata.id, "counter");
        assert_eq!(entry.open().metadata().id, "counter");
        let audit = audit_registry_reachability(&[entry], &ReachabilityPolicy::default())
            .expect("variant reachability audit");
        assert_eq!(audit.variants.len(), 1);
        assert_eq!(audit.variants[0].variant, "default");
        assert_eq!(
            audit.path_count, 4,
            "activation uses all three modalities; SetValue uses accessibility"
        );
        assert_eq!(audit.reached_count + audit.issue_count, audit.path_count);
        assert!(
            audit
                .to_json()
                .expect("registry reachability JSON")
                .contains("counter")
        );
    }

    #[test]
    fn registry_reachability_accounts_for_unloaded_external_providers() {
        static METADATA: FixtureMetadata = FixtureMetadata {
            id: "external.counter",
            title: "External counter",
            description: "External provider accounting fixture",
            tags: &["external"],
            source: FixtureSource {
                crate_name: "counter-provider",
                file: file!(),
                line: line!(),
            },
            variants: &[DEFAULT_VARIANT],
            assets: &[],
            simulated_effects: &[],
        };
        let mut registry = FixtureRegistry::new();
        registry
            .register_external(
                &METADATA,
                ExternalFixtureProvider {
                    protocol_version: 1,
                    cargo_package: "counter-provider",
                    workbench_feature: "counter-provider",
                },
            )
            .expect("external registration");
        let report =
            audit_registry_reachability(&registry.finish(), &ReachabilityPolicy::default())
                .expect("external entries do not execute in-process");
        assert_eq!(report.external_provider_count, 1);
        assert!(report.variants.is_empty());
        assert!(!report.is_complete());
    }

    #[test]
    fn erased_fixture_session_resets_and_activates_all_modalities() {
        struct CounterFixture;
        impl Fixture for CounterFixture {
            type App = Counter;

            fn metadata() -> &'static FixtureMetadata {
                static METADATA: FixtureMetadata = FixtureMetadata {
                    id: "erased-counter",
                    title: "Erased counter",
                    description: "Registry session coverage",
                    tags: &["core"],
                    source: FixtureSource {
                        crate_name: "nickel-ui-testkit",
                        file: file!(),
                        line: line!(),
                    },
                    variants: &[DEFAULT_VARIANT],
                    assets: &[],
                    simulated_effects: &[],
                };
                &METADATA
            }

            fn create() -> Self::App {
                Counter::default()
            }

            fn default_activation() -> Option<Selector> {
                Some(Selector::role_name(SemanticRole::Button, "Increment"))
            }
        }

        let mut registry = FixtureRegistry::new();
        registry.register::<CounterFixture>().expect("registration");
        let entry = registry.finish()[0];
        for via in [
            ActivationVia::Pointer,
            ActivationVia::Keyboard,
            ActivationVia::Controller,
            ActivationVia::Accessibility,
            ActivationVia::Semantic,
        ] {
            let mut session = entry.open();
            let initial_frame = session.inspect().frame_generation;
            session.activate(via).expect("erased activation");
            assert!(
                session.inspect().frame_generation > initial_frame,
                "{via:?}"
            );
            assert!(!session.semantic_nodes().is_empty());
            assert!(!session.render(1.0).rgba.is_empty());
            session.reset();
            assert_eq!(session.inspect().frame_generation, initial_frame);
        }
    }

    #[test]
    fn selectors_reject_ambiguity_and_budgets_fail_without_unbounded_trace_growth() {
        struct DuplicateButtons;
        impl Application for DuplicateButtons {
            type Message = Message;

            fn update(&mut self, _message: Self::Message) {}

            fn view(
                &self,
                _context: nickel_ui::ViewContext,
            ) -> impl nickel_ui::View<Self::Message> {
                nickel_ui::Row::new()
                    .child(Button::new(Message::Increment, "Same").id("first"))
                    .child(Button::new(Message::Increment, "Same").id("second"))
            }
        }

        let mut ambiguous = Scenario::new(DuplicateButtons, 320, 80);
        assert!(matches!(
            ambiguous.activate(&Selector::role_name(SemanticRole::Button, "Same")),
            Err(ScenarioError::AmbiguousTarget { matches: 2, .. })
        ));
        let ambiguous_error =
            match ambiguous.activate(&Selector::role_name(SemanticRole::Button, "Same")) {
                Ok(_) => panic!("duplicate names must be ambiguous"),
                Err(error) => error,
            };
        assert!(ambiguous_error.to_string().contains("root/first"));
        assert!(ambiguous_error.to_string().contains("root/second"));
        assert!(ambiguous.trace().is_empty());

        let missing_error = match ambiguous.activate(&Selector::id("root/firts")) {
            Ok(_) => panic!("misspelled identity must be missing"),
            Err(error) => error,
        };
        assert!(missing_error.to_string().contains("topology:"));

        let mut bounded = Scenario::with_budget(
            Counter::default(),
            320,
            160,
            ScenarioBudget {
                operations: 1,
                frames: 1,
                trace_steps: 1,
            },
        );
        bounded
            .activate(&Selector::id("root/increment"))
            .expect("first operation is within budget");
        assert!(matches!(
            bounded.activate(&Selector::id("root/increment")),
            Err(ScenarioError::OperationBudgetExceeded)
        ));
        assert_eq!(bounded.trace().len(), 1);
    }

    #[test]
    fn fresh_headless_sessions_render_identical_pixels() {
        let first = Scenario::new(Counter::default(), 320, 160);
        let second = Scenario::new(Counter::default(), 320, 160);
        let first = render_host(first.host(), 320, 160, 1.0);
        let second = render_host(second.host(), 320, 160, 1.0);
        assert_eq!(first, second);
        assert!(first.rgba.chunks_exact(4).any(|pixel| pixel[3] != 0));
    }

    #[test]
    fn versioned_source_free_trace_round_trips_and_replays_exactly() {
        let mut original = Scenario::new(Counter::default(), 320, 160);
        original
            .activate(&Selector::id("root/increment"))
            .expect("activate")
            .set_value(
                &Selector::id("root/query"),
                SemanticValueInput::Text("replay".into()),
            )
            .expect("text input");
        let document = original.trace_document("counter");
        let encoded = serde_json::to_string(&document).expect("serialize trace");
        assert!(!encoded.contains("/projects/"));
        let decoded: TraceDocument = serde_json::from_str(&encoded).expect("deserialize trace");

        let mut replay = Scenario::new(Counter::default(), 320, 160);
        replay.replay(&decoded).expect("exact replay");
        assert_eq!(replay.host_mut().application_mut().count, 1);
        assert_eq!(replay.host_mut().application_mut().query, "replay");
        assert_eq!(replay.trace_document("counter"), document);

        let mut wrong_viewport = document.clone();
        wrong_viewport.viewport = [640, 480];
        assert!(matches!(
            Scenario::new(Counter::default(), 320, 160).replay(&wrong_viewport),
            Err(ScenarioError::ReplayDrift { step: 0, .. })
        ));

        let mut wrong_result = document.clone();
        wrong_result.final_semantic_digest = "fnv1a64:0000000000000000".into();
        assert!(matches!(
            Scenario::new(Counter::default(), 320, 160).replay(&wrong_result),
            Err(ScenarioError::ReplayDrift { step: 2, .. })
        ));

        let mut wrong_message_evidence = document.clone();
        wrong_message_evidence.steps[0].messages.clear();
        assert!(matches!(
            Scenario::new(Counter::default(), 320, 160).replay(&wrong_message_evidence),
            Err(ScenarioError::ReplayDrift { step: 0, .. })
        ));
    }

    #[test]
    fn controller_route_uses_production_navigation_and_never_sets_focus_directly() {
        let mut scenario = Scenario::new(Counter::default(), 320, 160);
        let route = scenario
            .controller_activate(&Selector::id("root/increment"))
            .expect("controller reaches increment");
        assert_eq!(route.target, UiId::from("root/increment"));
        assert_eq!(route.actions.last(), Some(&ControllerAction::Confirm));
        assert_eq!(scenario.host_mut().application_mut().count, 1);
        assert_eq!(
            scenario.host().inspect().controller_target,
            Some(UiId::from("root/increment"))
        );
    }

    #[test]
    fn controller_reachability_proves_scroll_through_production_navigation() {
        #[derive(Default)]
        struct ScrollFixture {
            activations: usize,
        }

        impl Application for ScrollFixture {
            type Message = Message;

            fn update(&mut self, message: Self::Message) {
                if message == Message::Increment {
                    self.activations += 1;
                }
            }

            fn view(
                &self,
                _context: nickel_ui::ViewContext,
            ) -> impl nickel_ui::View<Self::Message> {
                nickel_ui::VerticalScroll::new(Message::Increment, 0.0)
                    .on_scroll(|_| Message::Increment)
                    .child(
                        nickel_ui::Column::new()
                            .child(Button::new(Message::Increment, "First").id("first"))
                            .child(Spacer::vertical(400.0))
                            .child(Button::new(Message::Increment, "Last").id("last")),
                    )
            }
        }

        let policy = ReachabilityPolicy {
            modalities: [ReachabilityModality::Controller].into_iter().collect(),
            ..ReachabilityPolicy::default()
        };
        let report = audit_reachability(
            || Scenario::new(ScrollFixture::default(), 200, 100),
            &policy,
        );
        let scroll = report
            .paths
            .iter()
            .find(|path| path.target == "root" && path.action == "Increment")
            .expect("scroll increment is audited");
        assert!(scroll.reached, "{:?}", report.issues);
        assert!(
            scroll
                .steps
                .iter()
                .any(|step| step.contains("controller:Increment:root"))
        );
    }

    #[test]
    fn controller_reachability_enters_semantic_radio_and_tab_scopes() {
        struct NestedDepthApp;

        impl Application for NestedDepthApp {
            type Message = Message;

            fn update(&mut self, _message: Self::Message) {}

            fn view(
                &self,
                _context: nickel_ui::ViewContext,
            ) -> impl nickel_ui::View<Self::Message> {
                let theme = SemanticTheme::from_tokens(SemanticTokenSet::standard(
                    0x101010, 0x181818, 0x202020, 0x242424, 0x303030, 0xf0f0f0, 0xa0a0a0, 0x9050e0,
                    0x402060, 0x50c080, 0x50c080,
                ));
                nickel_ui::Column::new()
                    .child(
                        RadioGroup::new([
                            RadioOption::new(theme, Message::Increment, "Headphones", true)
                                .id("headphones"),
                            RadioOption::new(theme, Message::Increment, "Speakers", false)
                                .id("speakers"),
                        ])
                        .id("outputs"),
                    )
                    .child(
                        TabList::new(
                            theme,
                            [
                                ("General", Message::Increment, true),
                                ("Theme", Message::Increment, false),
                            ],
                        )
                        .id("tabs"),
                    )
            }
        }

        let factory = || Scenario::new(NestedDepthApp, 480, 240);
        let initial = factory();
        let scope_ids = initial
            .semantic_nodes()
            .into_iter()
            .filter(|node| node.navigation_scope)
            .map(|node| node.id)
            .collect::<BTreeSet<_>>();
        assert_eq!(
            scope_ids,
            BTreeSet::from([UiId::from("root/outputs"), UiId::from("root/tabs")])
        );

        let report = audit_reachability(
            factory,
            &ReachabilityPolicy {
                modalities: BTreeSet::from([ReachabilityModality::Controller]),
                ..ReachabilityPolicy::default()
            },
        );
        assert!(report.is_complete(), "{:#?}", report.issues);
        assert_eq!(
            report
                .paths
                .iter()
                .map(|path| &path.target)
                .collect::<BTreeSet<_>>()
                .len(),
            4
        );
        assert!(report.paths.iter().all(|path| path.reached));
    }

    #[test]
    fn controller_graph_reports_state_and_path_ceilings_without_unbounded_search() {
        let factory = || Scenario::new(Counter::default(), 320, 160);
        let controller_only = BTreeSet::from([ReachabilityModality::Controller]);
        let state_limited = audit_reachability(
            factory,
            &ReachabilityPolicy {
                modalities: controller_only.clone(),
                maximum_state_count: 0,
                ..ReachabilityPolicy::default()
            },
        );
        assert!(state_limited.issues.iter().any(|issue| {
            issue.kind == ReachabilityIssueKind::Unreachable
                && issue.detail.contains("state ceiling")
        }));

        let path_limited = audit_reachability(
            factory,
            &ReachabilityPolicy {
                modalities: controller_only,
                maximum_path_length: 0,
                ..ReachabilityPolicy::default()
            },
        );
        assert!(path_limited.issues.iter().any(|issue| {
            issue.kind == ReachabilityIssueKind::Unreachable
                && issue.detail.contains("path ceiling")
        }));
    }

    #[test]
    fn controller_graph_completion_is_independent_of_machine_speed() {
        let factory = || Scenario::new(Counter::default(), 320, 160);
        let targets = factory()
            .semantic_nodes()
            .into_iter()
            .filter(|node| {
                node.actions.iter().any(|action| {
                    action_supported_by_modality(*action, ReachabilityModality::Controller)
                })
            })
            .map(|node| node.id.as_str().to_owned())
            .collect();
        let routes = explore_controller_routes(
            &factory,
            &targets,
            &ReachabilityPolicy {
                wall_time_ms: 0,
                ..ReachabilityPolicy::default()
            },
        );
        assert_eq!(routes.failure, None);
        assert_eq!(routes.routes.len(), targets.len());
    }

    #[test]
    fn semantic_and_accessibility_snapshots_share_role_name_and_identity() {
        let scenario = Scenario::new(Counter::default(), 320, 160);
        assert_eq!(validate_host(scenario.host()), Vec::new());
    }

    #[test]
    fn pointer_route_resolves_once_and_survives_rebuild_before_release() {
        let mut scenario = Scenario::new(Counter::default(), 320, 160);
        let route = scenario
            .pointer_activate(&Selector::id("root/increment"))
            .expect("pointer activation");
        assert_eq!(route.target, UiId::from("root/increment"));
        assert!(route.release_frame > route.press_frame);
        assert_eq!(scenario.host_mut().application_mut().count, 1);
    }

    #[test]
    fn keyboard_and_accessibility_routes_emit_the_same_typed_activation() {
        let mut keyboard = Scenario::new(Counter::default(), 320, 160);
        let route = keyboard
            .keyboard_activate(&Selector::id("root/increment"))
            .expect("keyboard route");
        assert_eq!(route.target, UiId::from("root/increment"));
        assert_eq!(keyboard.host_mut().application_mut().count, 1);

        let mut accessibility = Scenario::new(Counter::default(), 320, 160);
        accessibility
            .accessibility_activate(&Selector::id("root/increment"))
            .expect("accessibility route");
        assert_eq!(accessibility.host_mut().application_mut().count, 1);
    }

    #[test]
    fn keyed_collection_selector_resolves_identity_without_positions_or_messages() {
        struct Keyed;
        impl Application for Keyed {
            type Message = Message;

            fn update(&mut self, _message: Self::Message) {}

            fn view(
                &self,
                _context: nickel_ui::ViewContext,
            ) -> impl nickel_ui::View<Self::Message> {
                Collection::try_new(
                    CollectionState::Ready(vec![("alpha", "Alpha"), ("beta", "Beta")]),
                    |item| item.0,
                    |item| Text::<Message>::new(item.1),
                )
                .expect("unique collection keys")
                .id("items")
                .on_activate(|_| Message::Increment)
            }
        }

        let scenario = Scenario::new(Keyed, 320, 160);
        let target = scenario
            .resolve_target(&Selector::keyed_item("items", "beta"))
            .expect("stable keyed item");
        assert!(target.id.as_str().ends_with("/items/beta"));
        assert_eq!(target.role, Some(SemanticRole::ListItem));
    }

    #[test]
    fn typed_operations_record_resolved_geometry_and_source_free_state() {
        let mut scenario = Scenario::new(Counter::default(), 320, 160);
        scenario
            .pointer_move(&Selector::id("root/increment"))
            .expect("production hit route");
        scenario
            .keyboard_focus(FocusDirection::Next)
            .expect("focus");
        scenario.resize(480, 240, 1.5).expect("resize");
        scenario.window_focus(false).expect("focus loss");
        scenario.window_focus(true).expect("focus return");
        scenario.advance_time(1).expect("deterministic time");
        scenario.assert_no_diagnostics().expect("clean topology");
        scenario
            .assert_accessibility()
            .expect("accessible topology");
        scenario
            .assert_layout(
                &Selector::id("root/increment"),
                LayoutRelation::Above,
                &Selector::id("root/query"),
            )
            .expect("declarative column order");

        let trace = scenario.operation_trace();
        assert_eq!(trace.len(), 6);
        assert_eq!(trace[0].generated_points.len(), 1);
        assert_eq!(trace[0].resolved_target.as_deref(), Some("root/increment"));
        assert_ne!(trace[0].before.frame, trace[0].after.frame);
        let encoded = serde_json::to_string(trace).expect("machine-readable operations");
        assert!(!encoded.contains("/projects/"));
        assert!(!encoded.contains("src/lib.rs"));
        let coverage = scenario.coverage_report();
        assert!(coverage.roles.contains("button"));
        assert!(coverage.actions.contains("Activate"));
        assert!(coverage.input_routes.contains("pointer_move"));
        assert!(coverage.adapter_stages.contains("production_hit_test"));
        assert!(
            coverage
                .to_json()
                .expect("coverage JSON")
                .contains("semantic_host")
        );
    }

    #[test]
    fn operation_trace_replay_detects_structured_state_drift() {
        let mut original = Scenario::new(Counter::default(), 320, 160);
        original
            .keyboard_focus(FocusDirection::Next)
            .expect("keyboard focus")
            .semantic_operation(
                &Selector::id("root/increment"),
                SemanticAction::Invoke(ActionKind::Activate),
            )
            .expect("semantic operation")
            .resize(400, 200, 1.25)
            .expect("resize");
        let document = original.trace_document("counter-operations");
        assert_eq!(document.schema, 5);

        let mut replay = Scenario::new(Counter::default(), 320, 160);
        replay
            .replay(&document)
            .expect("source-free operation replay");
        assert_eq!(replay.trace_document("counter-operations"), document);

        let mut drifted = document.clone();
        drifted.operations[0].after.semantic_digest = "fnv1a64:0000000000000000".into();
        assert!(matches!(
            Scenario::new(Counter::default(), 320, 160).replay(&drifted),
            Err(ScenarioError::ReplayDrift { .. })
        ));

        let mut stale_geometry = document.clone();
        stale_geometry.operations.push(OperationTraceStep {
            operation: ScenarioOperation::PointerMove {
                target: "root/increment".into(),
            },
            resolved_target: Some("root/increment".into()),
            generated_points: vec![[9999.0, 9999.0]],
            before: stale_geometry.operations[0].before.clone(),
            after: stale_geometry.operations[0].after.clone(),
            outcome: stale_geometry.operations[0].outcome.clone(),
        });
        assert!(matches!(
            Scenario::new(Counter::default(), 320, 160).replay(&stale_geometry),
            Err(ScenarioError::ReplayDrift { .. })
        ));
    }

    #[test]
    fn assertion_failures_include_topology_and_matching_suggestions() {
        let scenario = Scenario::new(Counter::default(), 320, 160);
        let error = match scenario.assert_focus(&Selector::id("root/increment")) {
            Ok(_) => panic!("button is not initially focused"),
            Err(error) => error,
        };
        let rendered = error.to_string();
        assert!(rendered.contains("topology:"));
        assert!(rendered.contains("root/increment"));
        assert!(rendered.contains("nearby targets:"));
    }

    #[test]
    fn typed_effect_count_and_replay_drift_are_production_evidence() {
        #[derive(Default)]
        struct EffectApp {
            pending_effect: bool,
        }

        impl Application for EffectApp {
            type Message = Message;

            fn update(&mut self, message: Self::Message) {
                if message == Message::Increment {
                    self.pending_effect = true;
                }
            }

            fn take_effect_evidence(&mut self) -> Vec<nickel_ui::EffectEvidence> {
                std::mem::take(&mut self.pending_effect)
                    .then(|| nickel_ui::EffectEvidence {
                        type_name: "SaveEffect",
                        label: Some("save-counter".into()),
                    })
                    .into_iter()
                    .collect()
            }

            fn view(
                &self,
                _context: nickel_ui::ViewContext,
            ) -> impl nickel_ui::View<Self::Message> {
                Button::new(Message::Increment, "Save").id("save")
            }
        }

        let mut scenario = Scenario::new(EffectApp::default(), 200, 80);
        scenario
            .semantic_operation(
                &Selector::id("root/save"),
                SemanticAction::Invoke(ActionKind::Activate),
            )
            .expect("effectful production activation");
        scenario
            .assert_last_effect_count(1)
            .expect("one typed effect");
        assert_eq!(
            scenario.operation_trace()[0].outcome.effects,
            [EffectEvidenceSnapshot {
                type_name: "SaveEffect".into(),
                label: Some("save-counter".into()),
            }]
        );

        let mut wrong_effect_count = scenario.trace_document("effect-app");
        wrong_effect_count.operations[0].outcome.effects.clear();
        assert!(matches!(
            Scenario::new(EffectApp::default(), 200, 80).replay(&wrong_effect_count),
            Err(ScenarioError::ReplayDrift { .. })
        ));
    }

    #[test]
    fn reachability_report_is_machine_readable_and_flags_ignored_actions() {
        struct ButtonOnly;
        impl Application for ButtonOnly {
            type Message = Message;

            fn update(&mut self, _message: Self::Message) {}

            fn view(
                &self,
                _context: nickel_ui::ViewContext,
            ) -> impl nickel_ui::View<Self::Message> {
                Button::new(Message::Increment, "Ignored").id("ignored")
            }
        }

        let report = audit_reachability(
            || Scenario::new(ButtonOnly, 240, 80),
            &ReachabilityPolicy {
                require_semantic_change: true,
                ..ReachabilityPolicy::default()
            },
        );
        assert_eq!(report.schema, 1);
        assert_eq!(report.paths.len(), 3);
        assert!(report.issues.iter().any(|issue| {
            issue.kind == ReachabilityIssueKind::AdvertisedButIgnored
                && issue.target == "root/ignored"
        }));
        let json = report.to_json().expect("machine-readable report");
        assert!(json.contains("advertised_but_ignored"));
        assert!(json.contains("root/ignored"));
    }

    #[test]
    fn reachability_classifier_rejects_scope_leaks_cycles_and_unreconciled_targets() {
        let declared = BTreeSet::from(["root/inside".to_owned()]);
        let observation = ReachabilityObservation {
            steps: vec!["down:root/inside".into(), "down:root/inside".into()],
            failure: None,
            current_target: Some("root/removed".into()),
            current_semantic_ids: BTreeSet::from(["root/inside".into()]),
            semantic_changed: false,
        };
        let issues = classify_reachability_observation(
            "root/inside",
            ActionKind::Activate,
            ReachabilityModality::Controller,
            &declared,
            &ReachabilityPolicy {
                maximum_path_length: 1,
                require_semantic_change: true,
                ..ReachabilityPolicy::default()
            },
            &observation,
        );
        let kinds = issues
            .iter()
            .map(|issue| issue.kind.clone())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            kinds,
            BTreeSet::from([
                ReachabilityIssueKind::ScopeLeak,
                ReachabilityIssueKind::Cycle,
                ReachabilityIssueKind::ExcessivePathLength,
                ReachabilityIssueKind::AdvertisedButIgnored,
            ])
        );
        assert!(
            issues
                .iter()
                .find(|issue| issue.kind == ReachabilityIssueKind::ScopeLeak)
                .is_some_and(|issue| issue.detail.contains("survived removal"))
        );
    }

    #[test]
    fn validation_rejects_interactive_semantics_without_accessible_names() {
        struct Unnamed;
        impl Application for Unnamed {
            type Message = Message;

            fn update(&mut self, _message: Self::Message) {}

            fn view(
                &self,
                _context: nickel_ui::ViewContext,
            ) -> impl nickel_ui::View<Self::Message> {
                Container::new()
                    .id("unnamed")
                    .semantic_role(SemanticRole::Button)
                    .message(Message::Increment)
                    .child(Spacer::new().width(20.0).height(20.0))
            }
        }

        let frame = nickel_ui::UiFrame::layout_with_diagnostics(
            Unnamed.view(nickel_ui::ViewContext {
                viewport: nickel_ui::Rect::new(0.0, 0.0, 120.0, 80.0),
                modality: nickel_ui::InputModality::Pointer,
                focused: None,
                controller_target: None,
                available_semantic_actions: Vec::new(),
                navigation_depth: 0,
                open_overlay: None,
            }),
            nickel_ui::Rect::new(0.0, 0.0, 120.0, 80.0),
        );
        let id = UiId::from("root/unnamed");
        let diagnostics = frame.diagnostics();
        assert!(
            diagnostics.iter().any(|diagnostic| diagnostic.id == id
                && diagnostic.kind == DiagnosticKind::MissingAccessibleName),
            "missing-name diagnostic was not retained: {diagnostics:?}"
        );
        assert!(frame.semantic_nodes().is_empty());
        assert!(frame.accessibility_nodes().iter().all(|node| node.id != id));
    }

    #[test]
    fn scenario_rejects_duplicate_identity_without_choosing_an_arbitrary_target() {
        struct DuplicateIds;
        impl Application for DuplicateIds {
            type Message = Message;

            fn update(&mut self, _message: Self::Message) {}

            fn view(
                &self,
                _context: nickel_ui::ViewContext,
            ) -> impl nickel_ui::View<Self::Message> {
                nickel_ui::Row::new()
                    .child(Button::new(Message::Increment, "First").id("duplicate"))
                    .child(Button::new(Message::Increment, "Second").id("duplicate"))
            }
        }

        let mut scenario = Scenario::new(DuplicateIds, 240, 80);
        let error = match scenario.activate(&Selector::id("root/duplicate")) {
            Ok(_) => panic!("duplicate identities must not resolve arbitrarily"),
            Err(error) => error,
        };
        assert!(matches!(
            error,
            ScenarioError::AmbiguousTarget { matches: 2, .. }
        ));
        assert!(error.to_string().contains("root/duplicate"));
    }

    #[test]
    fn state_validation_rejects_focus_and_controller_targets_removed_from_topology() {
        let scenario = Scenario::new(Counter::default(), 320, 160);
        let stale_keyboard = UiId::from("root/removed-field");
        let stale_controller = UiId::from("root/removed-button");
        let issues = validate_active_state_references(
            &scenario.semantic_nodes(),
            Some(&stale_keyboard),
            Some(&stale_controller),
        );
        assert_eq!(
            issues,
            vec![
                ValidationIssue::UnreconciledState {
                    state: "keyboard_focus",
                    id: stale_keyboard,
                },
                ValidationIssue::UnreconciledState {
                    state: "controller_target",
                    id: stale_controller,
                },
            ]
        );
    }
}
