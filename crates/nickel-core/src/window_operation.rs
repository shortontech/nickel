//! Platform-neutral lifecycle for interactive window move and resize operations.
//!
//! Native adapters validate platform objects and execute [`Effect`]s. This
//! module deliberately contains no native handles, protocol serials, or event
//! types.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::{
    geometry::LogicalRect,
    geometry_authority::{ControlMode, GeometryConstraints, GeometryIntent, IntentError},
};

macro_rules! opaque_id {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(u64);

        impl $name {
            #[must_use]
            pub const fn new(value: u64) -> Self {
                Self(value)
            }

            #[must_use]
            pub const fn get(self) -> u64 {
                self.0
            }
        }

        impl From<u64> for $name {
            fn from(value: u64) -> Self {
                Self::new(value)
            }
        }
    };
}

opaque_id!(SeatId);
opaque_id!(WindowId);
opaque_id!(NativeLifetimeId);
opaque_id!(MappingGeneration);
opaque_id!(SourceId);
opaque_id!(SourceGeneration);
opaque_id!(OperationId);
opaque_id!(AcquisitionId);
opaque_id!(ResourceLeaseId);
opaque_id!(BindingEpoch);
opaque_id!(PressEpoch);
opaque_id!(AnchorEpoch);
opaque_id!(SourceEpoch);

/// A mapping identity cannot be confused with a reusable native window ID.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct WindowMapping {
    pub window: WindowId,
    pub native_lifetime: NativeLifetimeId,
    pub generation: MappingGeneration,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Source {
    pub id: SourceId,
    pub generation: SourceGeneration,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CompletionGesture {
    Button(u16),
    Contact(u64),
    SemanticGrant(u64),
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct CompletionBinding {
    pub source: Source,
    pub gesture: CompletionGesture,
    /// Incarnation of the press/grant that created this completion authority.
    pub press_epoch: PressEpoch,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum HorizontalEdge {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum VerticalEdge {
    Top,
    Bottom,
}

/// A resize edge is valid by construction: at least one axis is present.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ResizeEdges {
    horizontal: Option<HorizontalEdge>,
    vertical: Option<VerticalEdge>,
}

impl ResizeEdges {
    pub fn new(
        horizontal: Option<HorizontalEdge>,
        vertical: Option<VerticalEdge>,
    ) -> Result<Self, InvalidResizeEdges> {
        if horizontal.is_none() && vertical.is_none() {
            return Err(InvalidResizeEdges);
        }
        Ok(Self {
            horizontal,
            vertical,
        })
    }

    #[must_use]
    pub const fn horizontal(self) -> Option<HorizontalEdge> {
        self.horizontal
    }

    #[must_use]
    pub const fn vertical(self) -> Option<VerticalEdge> {
        self.vertical
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidResizeEdges;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationKind {
    Move,
    Resize(ResizeEdges),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationPhase {
    Admission,
    Armed,
    Active,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CancellationReason {
    UserCancelled,
    CompletionSourceLost,
    RequiredResourceLost,
    Lock,
    Suspend,
    SeatLost,
    TargetUnmapped,
    TargetDestroyed,
    Superseded,
    NativeTakeover,
    AuthorityUnknown,
    AcquisitionFailed,
    ReleasedBeforeActivation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FailureReason {
    AuthorityUnknown,
    NativeApplyFailed,
    AdmissionTimedOut,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalOutcome {
    Committed,
    Cancelled(CancellationReason),
    Failed(FailureReason),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompensationDecision {
    /// Restore only fields whose recorded revisions are still operation-owned.
    Conditional,
    /// A newer desired/native owner must be preserved.
    SkipSuperseded,
    /// The target mapping no longer exists.
    SkipTargetGone,
    /// Native ownership transferred away from Nickel.
    SkipAuthorityLost,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RejectionReason {
    SeatOccupied { by: OperationId },
    WindowOccupied { by: OperationId },
    UnknownOperation,
    WrongPhase,
    StaleAcquisition,
    HandoffAlreadyPending,
    GeometryUnavailable,
    InvalidConstraints,
    GeometryOverflow,
    SourceRebaseRequired,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Disposition {
    Applied,
    ConsumedTail,
    IgnoredUnrelated,
    Rejected(RejectionReason),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Effect {
    Acquire {
        operation: OperationId,
        request: AcquisitionId,
        source: Source,
    },
    ReleaseResource {
        lease: ResourceLeaseId,
    },
    OperationArmed {
        operation: OperationId,
    },
    OperationActivated {
        operation: OperationId,
    },
    BindingChanged {
        operation: OperationId,
        epoch: BindingEpoch,
    },
    ApplyUpdate {
        operation: OperationId,
        source: Source,
    },
    GeometryProposed {
        operation: OperationId,
        anchor_epoch: AnchorEpoch,
        source_epoch: SourceEpoch,
        unconstrained: LogicalRect,
        constrained: LogicalRect,
    },
    GeometryRebased {
        operation: OperationId,
        anchor_epoch: AnchorEpoch,
        source_epoch: SourceEpoch,
        source: Source,
        anchor: LogicalRect,
    },
    SubmitFinalDesiredState {
        operation: OperationId,
    },
    RequestCompensation {
        operation: OperationId,
        decision: CompensationDecision,
    },
    Terminal {
        operation: OperationId,
        outcome: TerminalOutcome,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Transition {
    pub disposition: Disposition,
    pub effects: Vec<Effect>,
}

impl Transition {
    fn applied(effects: Vec<Effect>) -> Self {
        Self {
            disposition: Disposition::Applied,
            effects,
        }
    }

    fn disposition(disposition: Disposition) -> Self {
        Self {
            disposition,
            effects: Vec::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct BeginRequest {
    pub seat: SeatId,
    pub subject: WindowMapping,
    pub kind: OperationKind,
    pub control: ControlMode,
    pub origin: CompletionBinding,
    pub optional_update_sources: Vec<Source>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GeometrySeed {
    pub anchor: LogicalRect,
    pub constraints: GeometryConstraints,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GeometryUpdate {
    /// Absolute displacement from the immutable anchor for this source epoch.
    AbsoluteDisplacement { x: i64, y: i64 },
    /// Admitted semantic delta accumulated without feeding constraints back.
    Delta { x: i64, y: i64 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OperationGeometry {
    pub intent: GeometryIntent,
    pub constraints: GeometryConstraints,
    pub unconstrained: LogicalRect,
    pub constrained: LogicalRect,
    pub anchor_epoch: AnchorEpoch,
    pub source_epoch: SourceEpoch,
    pub update_source: Source,
}

#[derive(Clone, Debug)]
pub struct Operation {
    pub id: OperationId,
    pub seat: SeatId,
    pub subject: WindowMapping,
    pub kind: OperationKind,
    pub control: ControlMode,
    pub phase: OperationPhase,
    /// Immutable provenance. Completion never consults this field.
    pub origin: CompletionBinding,
    pub binding_epoch: BindingEpoch,
    pub completion: CompletionBinding,
    pub optional_update_sources: HashSet<Source>,
    pub geometry: Option<OperationGeometry>,
    resource: Option<ResourceLeaseId>,
    acquisition: AcquisitionId,
    handoff: Option<PendingHandoff>,
    retired_completion: HashSet<CompletionBinding>,
}

#[derive(Clone, Debug)]
struct PendingHandoff {
    request: AcquisitionId,
    completion: CompletionBinding,
    optional_update_sources: HashSet<Source>,
}

pub struct WindowOperationReducer {
    next_operation: u64,
    next_acquisition: u64,
    operations: HashMap<OperationId, Operation>,
    seats: HashMap<SeatId, OperationId>,
    windows: HashMap<WindowId, OperationId>,
    invalid_acquisitions: VecDeque<AcquisitionId>,
    retired_acquisition_watermark: AcquisitionId,
    terminal: HashMap<OperationId, TerminalOutcome>,
    compensation: HashMap<OperationId, CompensationDecision>,
    terminal_order: VecDeque<OperationId>,
    limits: RetentionLimits,
}

pub const DEFAULT_TERMINAL_RETENTION: usize = 256;
pub const DEFAULT_INVALID_ACQUISITION_RETENTION: usize = 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RetentionLimits {
    pub terminal_records: usize,
    pub invalid_acquisitions: usize,
}

impl Default for RetentionLimits {
    fn default() -> Self {
        Self {
            terminal_records: DEFAULT_TERMINAL_RETENTION,
            invalid_acquisitions: DEFAULT_INVALID_ACQUISITION_RETENTION,
        }
    }
}

impl Default for WindowOperationReducer {
    fn default() -> Self {
        Self {
            next_operation: 0,
            next_acquisition: 0,
            operations: HashMap::new(),
            seats: HashMap::new(),
            windows: HashMap::new(),
            invalid_acquisitions: VecDeque::new(),
            retired_acquisition_watermark: AcquisitionId::new(0),
            terminal: HashMap::new(),
            compensation: HashMap::new(),
            terminal_order: VecDeque::new(),
            limits: RetentionLimits::default(),
        }
    }
}

impl WindowOperationReducer {
    #[must_use]
    pub fn with_limits(limits: RetentionLimits) -> Self {
        Self {
            limits,
            ..Self::default()
        }
    }

    #[must_use]
    pub fn operation(&self, id: OperationId) -> Option<&Operation> {
        self.operations.get(&id)
    }

    #[must_use]
    pub fn terminal_outcome(&self, id: OperationId) -> Option<TerminalOutcome> {
        self.terminal.get(&id).copied()
    }

    #[must_use]
    pub fn compensation_decision(&self, id: OperationId) -> Option<CompensationDecision> {
        self.compensation.get(&id).copied()
    }

    #[must_use]
    pub fn retained_terminal_records(&self) -> usize {
        self.terminal_order.len()
    }

    #[must_use]
    pub fn retained_invalid_acquisitions(&self) -> usize {
        self.invalid_acquisitions.len()
    }

    #[must_use]
    pub fn retired_acquisition_watermark(&self) -> AcquisitionId {
        self.retired_acquisition_watermark
    }

    /// Returns the current interactive writer for a stable window identity.
    /// Native adapters use this to revoke a mapping before destroying it.
    #[must_use]
    pub fn operation_for_window(&self, window: WindowId) -> Option<OperationId> {
        self.windows.get(&window).copied()
    }

    /// Terminates every live interaction at a seat-wide security/lifecycle boundary.
    /// Admission is released synchronously; settlement effects remain in the transitions.
    pub fn cancel_all(&mut self, reason: CancellationReason) -> Vec<Transition> {
        let mut operations = self.operations.keys().copied().collect::<Vec<_>>();
        operations.sort_unstable();
        operations
            .into_iter()
            .map(|operation| self.cancel(operation, reason))
            .collect()
    }

    pub fn begin(&mut self, request: BeginRequest) -> (Option<OperationId>, Transition) {
        self.begin_internal(request, None)
    }

    pub fn begin_with_geometry(
        &mut self,
        request: BeginRequest,
        seed: GeometrySeed,
    ) -> (Option<OperationId>, Transition) {
        let Ok(constraints) = seed.constraints.validate() else {
            return (
                None,
                Transition::disposition(Disposition::Rejected(RejectionReason::InvalidConstraints)),
            );
        };
        let intent = GeometryIntent::new(seed.anchor);
        let Ok(unconstrained) = intent.unconstrained() else {
            return (
                None,
                Transition::disposition(Disposition::Rejected(RejectionReason::GeometryOverflow)),
            );
        };
        let mut geometry = OperationGeometry {
            intent,
            constraints,
            unconstrained,
            constrained: constraints.constrain(unconstrained),
            anchor_epoch: AnchorEpoch::new(0),
            source_epoch: SourceEpoch::new(0),
            update_source: request.origin.source,
        };
        let Ok(constrained) = constrain_operation_geometry(request.kind, unconstrained, &geometry)
        else {
            return (
                None,
                Transition::disposition(Disposition::Rejected(RejectionReason::GeometryOverflow)),
            );
        };
        geometry.constrained = constrained;
        self.begin_internal(request, Some(geometry))
    }

    fn begin_internal(
        &mut self,
        request: BeginRequest,
        geometry: Option<OperationGeometry>,
    ) -> (Option<OperationId>, Transition) {
        if let Some(&by) = self.seats.get(&request.seat) {
            return (
                None,
                Transition::disposition(Disposition::Rejected(RejectionReason::SeatOccupied {
                    by,
                })),
            );
        }
        if let Some(&by) = self.windows.get(&request.subject.window) {
            return (
                None,
                Transition::disposition(Disposition::Rejected(RejectionReason::WindowOccupied {
                    by,
                })),
            );
        }

        let id = self.allocate_operation();
        let acquisition = self.allocate_acquisition();
        let source = request.origin.source;
        let operation = Operation {
            id,
            seat: request.seat,
            subject: request.subject,
            kind: request.kind,
            control: request.control,
            phase: OperationPhase::Admission,
            origin: request.origin,
            binding_epoch: BindingEpoch::new(0),
            completion: request.origin,
            optional_update_sources: request.optional_update_sources.into_iter().collect(),
            geometry,
            resource: None,
            acquisition,
            handoff: None,
            retired_completion: HashSet::new(),
        };
        self.seats.insert(operation.seat, id);
        self.windows.insert(operation.subject.window, id);
        self.operations.insert(id, operation);
        (
            Some(id),
            Transition::applied(vec![Effect::Acquire {
                operation: id,
                request: acquisition,
                source,
            }]),
        )
    }

    pub fn acquired(
        &mut self,
        operation: OperationId,
        request: AcquisitionId,
        lease: ResourceLeaseId,
    ) -> Transition {
        let Some(current) = self.operations.get_mut(&operation) else {
            return self.cleanup_late_acquisition(request, lease);
        };
        if current.phase != OperationPhase::Admission || current.acquisition != request {
            return self.cleanup_late_acquisition(request, lease);
        }
        current.resource = Some(lease);
        current.phase = OperationPhase::Armed;
        Transition::applied(vec![Effect::OperationArmed { operation }])
    }

    pub fn activate(&mut self, operation: OperationId) -> Transition {
        let Some(current) = self.operations.get_mut(&operation) else {
            return Transition::disposition(Disposition::Rejected(
                RejectionReason::UnknownOperation,
            ));
        };
        if current.phase != OperationPhase::Armed {
            return Transition::disposition(Disposition::Rejected(RejectionReason::WrongPhase));
        }
        current.phase = OperationPhase::Active;
        Transition::applied(vec![Effect::OperationActivated { operation }])
    }

    pub fn acquisition_failed(
        &mut self,
        operation: OperationId,
        request: AcquisitionId,
    ) -> Transition {
        let Some(current) = self.operations.get(&operation) else {
            return Transition::disposition(Disposition::Rejected(
                RejectionReason::StaleAcquisition,
            ));
        };
        if current.phase != OperationPhase::Admission || current.acquisition != request {
            return Transition::disposition(Disposition::Rejected(
                RejectionReason::StaleAcquisition,
            ));
        }
        self.finish(
            operation,
            TerminalOutcome::Cancelled(CancellationReason::AcquisitionFailed),
            None,
        )
    }

    pub fn update(&mut self, operation: OperationId, source: Source) -> Transition {
        let Some(current) = self.operations.get(&operation) else {
            return Transition::disposition(Disposition::IgnoredUnrelated);
        };
        if current.phase != OperationPhase::Active {
            return Transition::disposition(Disposition::Rejected(RejectionReason::WrongPhase));
        }
        if source != current.completion.source && !current.optional_update_sources.contains(&source)
        {
            return Transition::disposition(Disposition::IgnoredUnrelated);
        }
        Transition::applied(vec![Effect::ApplyUpdate { operation, source }])
    }

    pub fn update_geometry(
        &mut self,
        operation: OperationId,
        source: Source,
        update: GeometryUpdate,
    ) -> Transition {
        let Some(current) = self.operations.get_mut(&operation) else {
            return Transition::disposition(Disposition::IgnoredUnrelated);
        };
        if current.phase != OperationPhase::Active {
            return Transition::disposition(Disposition::Rejected(RejectionReason::WrongPhase));
        }
        if source != current.completion.source && !current.optional_update_sources.contains(&source)
        {
            return Transition::disposition(Disposition::IgnoredUnrelated);
        }
        let kind = current.kind;
        let Some(geometry) = current.geometry.as_mut() else {
            return Transition::disposition(Disposition::Rejected(
                RejectionReason::GeometryUnavailable,
            ));
        };
        if geometry.update_source != source {
            return Transition::disposition(Disposition::Rejected(
                RejectionReason::SourceRebaseRequired,
            ));
        }
        let mut next_intent = geometry.intent;
        if apply_geometry_update(&mut next_intent, kind, update).is_err() {
            return Transition::disposition(Disposition::Rejected(
                RejectionReason::GeometryOverflow,
            ));
        }
        let Ok(unconstrained) = next_intent.unconstrained() else {
            return Transition::disposition(Disposition::Rejected(
                RejectionReason::GeometryOverflow,
            ));
        };
        let Ok(constrained) = constrain_operation_geometry(kind, unconstrained, geometry) else {
            return Transition::disposition(Disposition::Rejected(
                RejectionReason::GeometryOverflow,
            ));
        };
        geometry.intent = next_intent;
        geometry.unconstrained = unconstrained;
        geometry.constrained = constrained;
        Transition::applied(vec![Effect::GeometryProposed {
            operation,
            anchor_epoch: geometry.anchor_epoch,
            source_epoch: geometry.source_epoch,
            unconstrained,
            constrained,
        }])
    }

    pub fn rebase_geometry_source(&mut self, operation: OperationId, source: Source) -> Transition {
        let Some(current) = self.operations.get(&operation) else {
            return Transition::disposition(Disposition::Rejected(
                RejectionReason::UnknownOperation,
            ));
        };
        let Some(anchor) = current.geometry.map(|geometry| geometry.constrained) else {
            return Transition::disposition(Disposition::Rejected(
                RejectionReason::GeometryUnavailable,
            ));
        };
        self.rebase_geometry(operation, source, anchor)
    }

    pub fn rebase_geometry(
        &mut self,
        operation: OperationId,
        source: Source,
        anchor: LogicalRect,
    ) -> Transition {
        let Some(current) = self.operations.get_mut(&operation) else {
            return Transition::disposition(Disposition::Rejected(
                RejectionReason::UnknownOperation,
            ));
        };
        if source != current.completion.source && !current.optional_update_sources.contains(&source)
        {
            return Transition::disposition(Disposition::IgnoredUnrelated);
        }
        let Some(geometry) = current.geometry.as_mut() else {
            return Transition::disposition(Disposition::Rejected(
                RejectionReason::GeometryUnavailable,
            ));
        };
        geometry.intent = geometry.intent.rebase(anchor);
        geometry.unconstrained = anchor;
        geometry.constrained = anchor;
        geometry.anchor_epoch = AnchorEpoch::new(geometry.anchor_epoch.get() + 1);
        geometry.source_epoch = SourceEpoch::new(geometry.source_epoch.get() + 1);
        geometry.update_source = source;
        Transition::applied(vec![Effect::GeometryRebased {
            operation,
            anchor_epoch: geometry.anchor_epoch,
            source_epoch: geometry.source_epoch,
            source,
            anchor,
        }])
    }

    pub fn release(&mut self, operation: OperationId, binding: CompletionBinding) -> Transition {
        let Some(current) = self.operations.get(&operation) else {
            return Transition::disposition(Disposition::IgnoredUnrelated);
        };
        if current.retired_completion.contains(&binding) {
            return Transition::disposition(Disposition::ConsumedTail);
        }
        if binding != current.completion {
            return Transition::disposition(Disposition::IgnoredUnrelated);
        }
        if current.phase != OperationPhase::Active {
            return self.cancel(operation, CancellationReason::ReleasedBeforeActivation);
        }
        self.finish(
            operation,
            TerminalOutcome::Committed,
            Some(CompensationDecision::Conditional),
        )
    }

    pub fn request_handoff(
        &mut self,
        operation: OperationId,
        completion: CompletionBinding,
        optional_update_sources: Vec<Source>,
    ) -> Transition {
        let request = self.allocate_acquisition();
        let Some(current) = self.operations.get_mut(&operation) else {
            return Transition::disposition(Disposition::Rejected(
                RejectionReason::UnknownOperation,
            ));
        };
        if current.phase != OperationPhase::Active {
            return Transition::disposition(Disposition::Rejected(RejectionReason::WrongPhase));
        }
        if current.handoff.is_some() {
            return Transition::disposition(Disposition::Rejected(
                RejectionReason::HandoffAlreadyPending,
            ));
        }
        current.handoff = Some(PendingHandoff {
            request,
            completion,
            optional_update_sources: optional_update_sources.into_iter().collect(),
        });
        Transition::applied(vec![Effect::Acquire {
            operation,
            request,
            source: completion.source,
        }])
    }

    pub fn handoff_acquired(
        &mut self,
        operation: OperationId,
        request: AcquisitionId,
        lease: ResourceLeaseId,
    ) -> Transition {
        let Some(current) = self.operations.get_mut(&operation) else {
            return self.cleanup_late_acquisition(request, lease);
        };
        let Some(pending) = current.handoff.take() else {
            return self.cleanup_late_acquisition(request, lease);
        };
        if pending.request != request {
            current.handoff = Some(pending);
            return self.cleanup_late_acquisition(request, lease);
        }

        current.retired_completion.insert(current.completion);
        current.completion = pending.completion;
        current.optional_update_sources = pending.optional_update_sources;
        current.binding_epoch = BindingEpoch::new(current.binding_epoch.get() + 1);
        let old_lease = current.resource.replace(lease);
        let mut effects = vec![Effect::BindingChanged {
            operation,
            epoch: current.binding_epoch,
        }];
        if let Some(geometry) = current.geometry.as_mut() {
            geometry.intent = geometry.intent.rebase(geometry.constrained);
            geometry.unconstrained = geometry.constrained;
            geometry.anchor_epoch = AnchorEpoch::new(geometry.anchor_epoch.get() + 1);
            geometry.source_epoch = SourceEpoch::new(geometry.source_epoch.get() + 1);
            geometry.update_source = current.completion.source;
            effects.push(Effect::GeometryRebased {
                operation,
                anchor_epoch: geometry.anchor_epoch,
                source_epoch: geometry.source_epoch,
                source: geometry.update_source,
                anchor: geometry.constrained,
            });
        }
        if let Some(lease) = old_lease {
            effects.push(Effect::ReleaseResource { lease });
        }
        Transition::applied(effects)
    }

    pub fn handoff_failed(&mut self, operation: OperationId, request: AcquisitionId) -> Transition {
        let Some(current) = self.operations.get_mut(&operation) else {
            return Transition::disposition(Disposition::Rejected(
                RejectionReason::StaleAcquisition,
            ));
        };
        if current.handoff.as_ref().map(|handoff| handoff.request) != Some(request) {
            return Transition::disposition(Disposition::Rejected(
                RejectionReason::StaleAcquisition,
            ));
        }
        current.handoff = None;
        Transition::applied(Vec::new())
    }

    pub fn source_disconnected(&mut self, operation: OperationId, source: Source) -> Transition {
        let Some(current) = self.operations.get_mut(&operation) else {
            return Transition::disposition(Disposition::IgnoredUnrelated);
        };
        if source == current.completion.source {
            return self.cancel(operation, CancellationReason::CompletionSourceLost);
        }
        if current.optional_update_sources.remove(&source) {
            return Transition::applied(Vec::new());
        }
        Transition::disposition(Disposition::IgnoredUnrelated)
    }

    pub fn cancel(&mut self, operation: OperationId, reason: CancellationReason) -> Transition {
        let decision = match reason {
            CancellationReason::TargetUnmapped | CancellationReason::TargetDestroyed => {
                CompensationDecision::SkipTargetGone
            }
            CancellationReason::Superseded => CompensationDecision::SkipSuperseded,
            CancellationReason::NativeTakeover | CancellationReason::AuthorityUnknown => {
                CompensationDecision::SkipAuthorityLost
            }
            _ => CompensationDecision::Conditional,
        };
        self.finish(
            operation,
            TerminalOutcome::Cancelled(reason),
            Some(decision),
        )
    }

    pub fn fail(&mut self, operation: OperationId, reason: FailureReason) -> Transition {
        let decision = if reason == FailureReason::AuthorityUnknown {
            CompensationDecision::SkipAuthorityLost
        } else {
            CompensationDecision::Conditional
        };
        self.finish(operation, TerminalOutcome::Failed(reason), Some(decision))
    }

    fn finish(
        &mut self,
        operation: OperationId,
        outcome: TerminalOutcome,
        compensation: Option<CompensationDecision>,
    ) -> Transition {
        let Some(mut current) = self.operations.remove(&operation) else {
            return Transition::disposition(Disposition::Rejected(
                RejectionReason::UnknownOperation,
            ));
        };
        self.seats.remove(&current.seat);
        self.windows.remove(&current.subject.window);
        self.remember_invalid_acquisition(current.acquisition);
        if let Some(handoff) = current.handoff.take() {
            self.remember_invalid_acquisition(handoff.request);
        }
        self.terminal.insert(operation, outcome);
        self.terminal_order.push_back(operation);

        let mut effects = Vec::new();
        if let Some(lease) = current.resource {
            effects.push(Effect::ReleaseResource { lease });
        }
        match outcome {
            TerminalOutcome::Committed => {
                effects.push(Effect::SubmitFinalDesiredState { operation });
            }
            TerminalOutcome::Cancelled(_) | TerminalOutcome::Failed(_) => {
                let decision = compensation.unwrap_or(CompensationDecision::Conditional);
                self.compensation.insert(operation, decision);
                effects.push(Effect::RequestCompensation {
                    operation,
                    decision,
                });
            }
        }
        while self.terminal_order.len() > self.limits.terminal_records {
            if let Some(expired) = self.terminal_order.pop_front() {
                self.terminal.remove(&expired);
                self.compensation.remove(&expired);
            }
        }
        effects.push(Effect::Terminal { operation, outcome });
        Transition::applied(effects)
    }

    fn cleanup_late_acquisition(
        &mut self,
        request: AcquisitionId,
        lease: ResourceLeaseId,
    ) -> Transition {
        if request <= self.retired_acquisition_watermark {
            Transition::applied(vec![Effect::ReleaseResource { lease }])
        } else {
            Transition::disposition(Disposition::Rejected(RejectionReason::StaleAcquisition))
        }
    }

    fn remember_invalid_acquisition(&mut self, request: AcquisitionId) {
        self.retired_acquisition_watermark = self.retired_acquisition_watermark.max(request);
        self.invalid_acquisitions.push_back(request);
        while self.invalid_acquisitions.len() > self.limits.invalid_acquisitions {
            self.invalid_acquisitions.pop_front();
        }
    }

    fn allocate_operation(&mut self) -> OperationId {
        self.next_operation += 1;
        OperationId::new(self.next_operation)
    }

    fn allocate_acquisition(&mut self) -> AcquisitionId {
        self.next_acquisition += 1;
        AcquisitionId::new(self.next_acquisition)
    }
}

fn apply_geometry_update(
    intent: &mut GeometryIntent,
    kind: OperationKind,
    update: GeometryUpdate,
) -> Result<(), IntentError> {
    let (x, y, absolute) = match update {
        GeometryUpdate::AbsoluteDisplacement { x, y } => (x, y, true),
        GeometryUpdate::Delta { x, y } => (x, y, false),
    };
    let assign = |slot: &mut i64, value: i64| -> Result<(), IntentError> {
        if absolute {
            *slot = value;
        } else {
            *slot = slot.checked_add(value).ok_or(IntentError::Overflow)?;
        }
        Ok(())
    };
    match kind {
        OperationKind::Move => {
            assign(&mut intent.delta_x, x)?;
            assign(&mut intent.delta_y, y)?;
        }
        OperationKind::Resize(edges) => {
            if let Some(edge) = edges.horizontal() {
                match edge {
                    HorizontalEdge::Left => {
                        assign(&mut intent.delta_x, x)?;
                        assign(
                            &mut intent.delta_width,
                            x.checked_neg().ok_or(IntentError::Overflow)?,
                        )?;
                    }
                    HorizontalEdge::Right => assign(&mut intent.delta_width, x)?,
                }
            }
            if let Some(edge) = edges.vertical() {
                match edge {
                    VerticalEdge::Top => {
                        assign(&mut intent.delta_y, y)?;
                        assign(
                            &mut intent.delta_height,
                            y.checked_neg().ok_or(IntentError::Overflow)?,
                        )?;
                    }
                    VerticalEdge::Bottom => assign(&mut intent.delta_height, y)?,
                }
            }
        }
    }
    Ok(())
}

fn constrain_operation_geometry(
    kind: OperationKind,
    unconstrained: LogicalRect,
    geometry: &OperationGeometry,
) -> Result<LogicalRect, IntentError> {
    let mut constrained = geometry.constraints.constrain(unconstrained);
    if let OperationKind::Resize(edges) = kind {
        let anchor = geometry.intent.anchor;
        if edges.horizontal() == Some(HorizontalEdge::Left) {
            constrained.x = i32::try_from(
                i64::from(anchor.x) + i64::from(anchor.width) - i64::from(constrained.width),
            )
            .map_err(|_| IntentError::Overflow)?;
        }
        if edges.vertical() == Some(VerticalEdge::Top) {
            constrained.y = i32::try_from(
                i64::from(anchor.y) + i64::from(anchor.height) - i64::from(constrained.height),
            )
            .map_err(|_| IntentError::Overflow)?;
        }
    }
    Ok(constrained)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(id: u64) -> Source {
        Source {
            id: SourceId::new(id),
            generation: SourceGeneration::new(1),
        }
    }

    fn binding(id: u64) -> CompletionBinding {
        binding_press(id, 1)
    }

    fn binding_press(id: u64, press: u64) -> CompletionBinding {
        CompletionBinding {
            source: source(id),
            gesture: CompletionGesture::Button(1),
            press_epoch: PressEpoch::new(press),
        }
    }

    fn mapping(id: u64) -> WindowMapping {
        WindowMapping {
            window: WindowId::new(id),
            native_lifetime: NativeLifetimeId::new(id),
            generation: MappingGeneration::new(1),
        }
    }

    fn begin(
        reducer: &mut WindowOperationReducer,
        seat: u64,
        window: u64,
    ) -> (OperationId, AcquisitionId) {
        let (Some(operation), transition) = reducer.begin(BeginRequest {
            seat: SeatId::new(seat),
            subject: mapping(window),
            kind: OperationKind::Move,
            control: ControlMode::Enforced,
            origin: binding(1),
            optional_update_sources: Vec::new(),
        }) else {
            panic!("begin rejected")
        };
        let [Effect::Acquire { request, .. }] = transition.effects.as_slice() else {
            panic!("begin did not request acquisition")
        };
        (operation, *request)
    }

    fn activate(
        reducer: &mut WindowOperationReducer,
        operation: OperationId,
        acquisition: AcquisitionId,
    ) {
        reducer.acquired(
            operation,
            acquisition,
            ResourceLeaseId::new(operation.get()),
        );
        assert_eq!(
            reducer.activate(operation).disposition,
            Disposition::Applied
        );
    }

    fn begin_geometry(
        reducer: &mut WindowOperationReducer,
        seat: u64,
        window: u64,
        kind: OperationKind,
        optional_update_sources: Vec<Source>,
    ) -> (OperationId, AcquisitionId) {
        let (Some(operation), transition) = reducer.begin_with_geometry(
            BeginRequest {
                seat: SeatId::new(seat),
                subject: mapping(window),
                kind,
                control: ControlMode::Enforced,
                origin: binding(1),
                optional_update_sources,
            },
            GeometrySeed {
                anchor: LogicalRect {
                    x: 10,
                    y: 20,
                    width: 90,
                    height: 80,
                },
                constraints: GeometryConstraints {
                    min_width: 40,
                    min_height: 30,
                    max_width: Some(100),
                    max_height: Some(90),
                },
            },
        ) else {
            panic!("geometry begin rejected")
        };
        let [Effect::Acquire { request, .. }] = transition.effects.as_slice() else {
            panic!("begin did not request acquisition")
        };
        (operation, *request)
    }

    fn proposed(transition: &Transition) -> (LogicalRect, LogicalRect) {
        let [
            Effect::GeometryProposed {
                unconstrained,
                constrained,
                ..
            },
        ] = transition.effects.as_slice()
        else {
            panic!("expected geometry proposal: {transition:?}")
        };
        (*unconstrained, *constrained)
    }

    #[test]
    fn unrelated_release_does_not_complete_operation() {
        let mut reducer = WindowOperationReducer::default();
        let (operation, acquisition) = begin(&mut reducer, 1, 1);
        activate(&mut reducer, operation, acquisition);
        let unrelated_button = CompletionBinding {
            source: source(1),
            gesture: CompletionGesture::Button(2),
            press_epoch: PressEpoch::new(1),
        };

        let result = reducer.release(operation, unrelated_button);

        assert_eq!(result.disposition, Disposition::IgnoredUnrelated);
        assert_eq!(
            reducer.operation(operation).unwrap().phase,
            OperationPhase::Active
        );
    }

    #[test]
    fn externally_contested_control_is_retained_as_operation_authority() {
        let mut reducer = WindowOperationReducer::default();
        let (Some(operation), transition) = reducer.begin(BeginRequest {
            seat: SeatId::new(1),
            subject: mapping(1),
            kind: OperationKind::Move,
            control: ControlMode::ExternallyContested,
            origin: binding(1),
            optional_update_sources: Vec::new(),
        }) else {
            panic!("begin rejected")
        };
        assert!(matches!(
            transition.effects.as_slice(),
            [Effect::Acquire { .. }]
        ));
        assert_eq!(
            reducer.operation(operation).unwrap().control,
            ControlMode::ExternallyContested
        );
        assert_eq!(
            reducer
                .cancel(operation, CancellationReason::NativeTakeover)
                .effects,
            vec![
                Effect::RequestCompensation {
                    operation,
                    decision: CompensationDecision::SkipAuthorityLost,
                },
                Effect::Terminal {
                    operation,
                    outcome: TerminalOutcome::Cancelled(CancellationReason::NativeTakeover),
                },
            ]
        );
    }

    #[test]
    fn source_transfer_makes_old_release_and_disconnect_harmless() {
        let mut reducer = WindowOperationReducer::default();
        let (operation, acquisition) = begin(&mut reducer, 1, 1);
        activate(&mut reducer, operation, acquisition);
        let handoff = reducer.request_handoff(operation, binding(2), vec![source(3)]);
        let [Effect::Acquire { request, .. }] = handoff.effects.as_slice() else {
            panic!("handoff did not request acquisition")
        };
        let request = *request;

        reducer.handoff_acquired(operation, request, ResourceLeaseId::new(2));

        assert_eq!(
            reducer.release(operation, binding(1)).disposition,
            Disposition::ConsumedTail
        );
        assert_eq!(
            reducer
                .source_disconnected(operation, source(1))
                .disposition,
            Disposition::IgnoredUnrelated
        );
        assert!(reducer.operation(operation).is_some());
        assert_eq!(
            reducer
                .source_disconnected(operation, source(2))
                .effects
                .last(),
            Some(&Effect::Terminal {
                operation,
                outcome: TerminalOutcome::Cancelled(CancellationReason::CompletionSourceLost),
            })
        );
    }

    #[test]
    fn source_return_accepts_fresh_press_and_consumes_retired_tail() {
        let mut reducer = WindowOperationReducer::default();
        let (operation, acquisition) = begin(&mut reducer, 1, 1);
        activate(&mut reducer, operation, acquisition);

        let to_b = reducer.request_handoff(operation, binding_press(2, 2), Vec::new());
        let [Effect::Acquire { request, .. }] = to_b.effects.as_slice() else {
            panic!("B handoff did not request acquisition")
        };
        reducer.handoff_acquired(operation, *request, ResourceLeaseId::new(2));

        let fresh_a = binding_press(1, 3);
        let to_a = reducer.request_handoff(operation, fresh_a, Vec::new());
        let [Effect::Acquire { request, .. }] = to_a.effects.as_slice() else {
            panic!("fresh A handoff did not request acquisition")
        };
        reducer.handoff_acquired(operation, *request, ResourceLeaseId::new(3));

        assert_eq!(
            reducer.release(operation, binding_press(1, 1)).disposition,
            Disposition::ConsumedTail
        );
        assert_eq!(
            reducer.release(operation, fresh_a).disposition,
            Disposition::Applied
        );
    }

    #[test]
    fn release_during_pending_handoff_wins_and_late_success_is_cleaned_up() {
        let mut reducer = WindowOperationReducer::default();
        let (operation, acquisition) = begin(&mut reducer, 1, 1);
        activate(&mut reducer, operation, acquisition);
        let handoff = reducer.request_handoff(operation, binding(2), Vec::new());
        let [Effect::Acquire { request, .. }] = handoff.effects.as_slice() else {
            panic!("handoff did not request acquisition")
        };
        let request = *request;

        assert_eq!(
            reducer.release(operation, binding(1)).disposition,
            Disposition::Applied
        );
        assert_eq!(
            reducer
                .handoff_acquired(operation, request, ResourceLeaseId::new(99))
                .effects,
            vec![Effect::ReleaseResource {
                lease: ResourceLeaseId::new(99)
            }]
        );
        assert!(reducer.operation(operation).is_none());
    }

    #[test]
    fn cancellation_during_initial_acquisition_cleans_up_late_success() {
        let mut reducer = WindowOperationReducer::default();
        let (operation, acquisition) = begin(&mut reducer, 1, 1);

        reducer.cancel(operation, CancellationReason::Lock);
        let late = reducer.acquired(operation, acquisition, ResourceLeaseId::new(7));

        assert_eq!(
            late.effects,
            vec![Effect::ReleaseResource {
                lease: ResourceLeaseId::new(7)
            }]
        );
        assert!(reducer.operation(operation).is_none());
    }

    #[test]
    fn seat_wide_security_cancel_releases_every_admission_before_settlement() {
        let mut reducer = WindowOperationReducer::default();
        let (first, first_acquisition) = begin(&mut reducer, 2, 20);
        let (second, second_acquisition) = begin(&mut reducer, 1, 10);
        activate(&mut reducer, first, first_acquisition);
        activate(&mut reducer, second, second_acquisition);

        let transitions = reducer.cancel_all(CancellationReason::Suspend);

        assert_eq!(transitions.len(), 2);
        assert_eq!(
            reducer.terminal_outcome(first),
            Some(TerminalOutcome::Cancelled(CancellationReason::Suspend))
        );
        assert_eq!(
            reducer.terminal_outcome(second),
            Some(TerminalOutcome::Cancelled(CancellationReason::Suspend))
        );
        assert!(reducer.operation(first).is_none());
        assert!(reducer.operation(second).is_none());
        assert!(begin(&mut reducer, 1, 30).0 > second);
    }

    #[test]
    fn competing_begin_is_explicitly_rejected() {
        let mut reducer = WindowOperationReducer::default();
        let (operation, _) = begin(&mut reducer, 1, 1);

        let (other, transition) = reducer.begin(BeginRequest {
            seat: SeatId::new(1),
            subject: mapping(2),
            kind: OperationKind::Move,
            control: ControlMode::Enforced,
            origin: binding(2),
            optional_update_sources: Vec::new(),
        });

        assert_eq!(other, None);
        assert_eq!(
            transition.disposition,
            Disposition::Rejected(RejectionReason::SeatOccupied { by: operation })
        );
    }

    #[test]
    fn same_window_cannot_gain_a_second_writer_through_another_mapping_identity() {
        let mut reducer = WindowOperationReducer::default();
        let (operation, _) = begin(&mut reducer, 1, 1);
        let mut replacement = mapping(1);
        replacement.generation = MappingGeneration::new(2);

        let (other, transition) = reducer.begin(BeginRequest {
            seat: SeatId::new(2),
            subject: replacement,
            kind: OperationKind::Move,
            control: ControlMode::Enforced,
            origin: binding(2),
            optional_update_sources: Vec::new(),
        });

        assert_eq!(other, None);
        assert_eq!(
            transition.disposition,
            Disposition::Rejected(RejectionReason::WindowOccupied { by: operation })
        );
    }

    #[test]
    fn release_before_activation_cancels_and_cleans_up_late_acquisition() {
        let mut reducer = WindowOperationReducer::default();
        let (operation, acquisition) = begin(&mut reducer, 1, 1);

        let terminal = reducer.release(operation, binding(1));

        assert_eq!(
            terminal.effects.last(),
            Some(&Effect::Terminal {
                operation,
                outcome: TerminalOutcome::Cancelled(CancellationReason::ReleasedBeforeActivation),
            })
        );
        assert_eq!(
            reducer
                .acquired(operation, acquisition, ResourceLeaseId::new(9))
                .effects,
            vec![Effect::ReleaseResource {
                lease: ResourceLeaseId::new(9)
            }]
        );
    }

    #[test]
    fn resize_requires_at_least_one_edge() {
        assert_eq!(ResizeEdges::new(None, None), Err(InvalidResizeEdges));
        assert!(ResizeEdges::new(Some(HorizontalEdge::Left), None).is_ok());
        assert!(ResizeEdges::new(None, Some(VerticalEdge::Bottom)).is_ok());
        assert!(ResizeEdges::new(Some(HorizontalEdge::Right), Some(VerticalEdge::Top)).is_ok());
    }

    #[test]
    fn terminal_interaction_frees_seat_while_settlement_is_only_an_effect() {
        let mut reducer = WindowOperationReducer::default();
        let (operation, acquisition) = begin(&mut reducer, 1, 1);
        activate(&mut reducer, operation, acquisition);

        let terminal = reducer.release(operation, binding(1));
        assert!(
            terminal
                .effects
                .contains(&Effect::SubmitFinalDesiredState { operation })
        );
        assert_eq!(
            reducer.terminal_outcome(operation),
            Some(TerminalOutcome::Committed)
        );

        let (next, _) = reducer.begin(BeginRequest {
            seat: SeatId::new(1),
            subject: mapping(2),
            kind: OperationKind::Move,
            control: ControlMode::Enforced,
            origin: binding(1),
            optional_update_sources: Vec::new(),
        });
        assert!(next.is_some(), "settlement must not retain the seat lease");
    }

    #[test]
    fn cancellation_decision_is_typed_by_reason() {
        let mut reducer = WindowOperationReducer::default();
        let (operation, acquisition) = begin(&mut reducer, 1, 1);
        activate(&mut reducer, operation, acquisition);

        let transition = reducer.cancel(operation, CancellationReason::NativeTakeover);

        assert!(transition.effects.contains(&Effect::RequestCompensation {
            operation,
            decision: CompensationDecision::SkipAuthorityLost,
        }));
        assert_eq!(
            reducer.compensation_decision(operation),
            Some(CompensationDecision::SkipAuthorityLost)
        );
    }

    #[test]
    fn constraints_do_not_feed_back_into_accumulated_intent() {
        let mut reducer = WindowOperationReducer::default();
        let edges = ResizeEdges::new(Some(HorizontalEdge::Right), None).unwrap();
        let (operation, acquisition) =
            begin_geometry(&mut reducer, 1, 1, OperationKind::Resize(edges), vec![]);
        activate(&mut reducer, operation, acquisition);

        let (unconstrained, constrained) = proposed(&reducer.update_geometry(
            operation,
            source(1),
            GeometryUpdate::Delta { x: 20, y: 0 },
        ));
        assert_eq!(unconstrained.width, 110);
        assert_eq!(constrained.width, 100);

        let (unconstrained, constrained) = proposed(&reducer.update_geometry(
            operation,
            source(1),
            GeometryUpdate::Delta { x: -5, y: 0 },
        ));
        assert_eq!(unconstrained.width, 105);
        assert_eq!(constrained.width, 100);
    }

    #[test]
    fn handoff_rebases_intent_and_source_epochs() {
        let mut reducer = WindowOperationReducer::default();
        let (operation, acquisition) =
            begin_geometry(&mut reducer, 1, 1, OperationKind::Move, vec![]);
        activate(&mut reducer, operation, acquisition);
        let _ =
            reducer.update_geometry(operation, source(1), GeometryUpdate::Delta { x: 20, y: -5 });
        let handoff = reducer.request_handoff(operation, binding(2), vec![]);
        let [Effect::Acquire { request, .. }] = handoff.effects.as_slice() else {
            panic!("handoff acquisition missing")
        };
        let acquired = reducer.handoff_acquired(operation, *request, ResourceLeaseId::new(22));
        assert!(acquired.effects.contains(&Effect::GeometryRebased {
            operation,
            anchor_epoch: AnchorEpoch::new(1),
            source_epoch: SourceEpoch::new(1),
            source: source(2),
            anchor: LogicalRect {
                x: 30,
                y: 15,
                width: 90,
                height: 80,
            },
        }));
        let (_, constrained) = proposed(&reducer.update_geometry(
            operation,
            source(2),
            GeometryUpdate::Delta { x: 3, y: 4 },
        ));
        assert_eq!(
            constrained,
            LogicalRect {
                x: 33,
                y: 19,
                width: 90,
                height: 80,
            }
        );
        assert_eq!(
            reducer.release(operation, binding(1)).disposition,
            Disposition::ConsumedTail
        );
    }

    #[test]
    fn every_resize_edge_preserves_the_opposite_edge() {
        let axes = [
            (Some(HorizontalEdge::Left), None),
            (Some(HorizontalEdge::Right), None),
            (None, Some(VerticalEdge::Top)),
            (None, Some(VerticalEdge::Bottom)),
            (Some(HorizontalEdge::Left), Some(VerticalEdge::Top)),
            (Some(HorizontalEdge::Left), Some(VerticalEdge::Bottom)),
            (Some(HorizontalEdge::Right), Some(VerticalEdge::Top)),
            (Some(HorizontalEdge::Right), Some(VerticalEdge::Bottom)),
        ];
        for (index, (horizontal, vertical)) in axes.into_iter().enumerate() {
            let mut reducer = WindowOperationReducer::default();
            let edges = ResizeEdges::new(horizontal, vertical).unwrap();
            let (operation, acquisition) = begin_geometry(
                &mut reducer,
                1,
                index as u64 + 1,
                OperationKind::Resize(edges),
                vec![],
            );
            activate(&mut reducer, operation, acquisition);
            let (_, rect) = proposed(&reducer.update_geometry(
                operation,
                source(1),
                GeometryUpdate::Delta { x: 7, y: 9 },
            ));
            match horizontal {
                Some(HorizontalEdge::Left) => assert_eq!(rect.x + rect.width, 100),
                Some(HorizontalEdge::Right) => assert_eq!(rect.x, 10),
                None => assert_eq!((rect.x, rect.width), (10, 90)),
            }
            match vertical {
                Some(VerticalEdge::Top) => assert_eq!(rect.y + rect.height, 100),
                Some(VerticalEdge::Bottom) => assert_eq!(rect.y, 20),
                None => assert_eq!((rect.y, rect.height), (20, 80)),
            }
        }
    }

    #[test]
    fn bounded_retention_keeps_late_acquisition_watermark() {
        let mut reducer = WindowOperationReducer::with_limits(RetentionLimits {
            terminal_records: 2,
            invalid_acquisitions: 1,
        });
        let mut finished = Vec::new();
        for id in 1..=3 {
            let (operation, acquisition) = begin(&mut reducer, 1, id);
            reducer.cancel(operation, CancellationReason::UserCancelled);
            finished.push((operation, acquisition));
        }
        assert_eq!(reducer.retained_terminal_records(), 2);
        assert_eq!(reducer.retained_invalid_acquisitions(), 1);
        assert_eq!(reducer.terminal_outcome(finished[0].0), None);
        assert_eq!(reducer.compensation_decision(finished[0].0), None);
        assert_eq!(
            reducer
                .acquired(finished[0].0, finished[0].1, ResourceLeaseId::new(99))
                .effects,
            vec![Effect::ReleaseResource {
                lease: ResourceLeaseId::new(99)
            }]
        );
        assert_eq!(reducer.retired_acquisition_watermark(), finished[2].1);
    }
}
