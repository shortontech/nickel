//! Platform-neutral lifecycle for interactive window move and resize operations.
//!
//! Native adapters validate platform objects and execute [`Effect`]s. This
//! module deliberately contains no native handles, protocol serials, or event
//! types.

use std::collections::{HashMap, HashSet};

use crate::geometry_authority::ControlMode;

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

#[derive(Default)]
pub struct WindowOperationReducer {
    next_operation: u64,
    next_acquisition: u64,
    operations: HashMap<OperationId, Operation>,
    seats: HashMap<SeatId, OperationId>,
    windows: HashMap<WindowId, OperationId>,
    invalid_acquisitions: HashSet<AcquisitionId>,
    terminal: HashMap<OperationId, TerminalOutcome>,
}

impl WindowOperationReducer {
    #[must_use]
    pub fn operation(&self, id: OperationId) -> Option<&Operation> {
        self.operations.get(&id)
    }

    #[must_use]
    pub fn terminal_outcome(&self, id: OperationId) -> Option<TerminalOutcome> {
        self.terminal.get(&id).copied()
    }

    pub fn begin(&mut self, request: BeginRequest) -> (Option<OperationId>, Transition) {
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
        self.invalid_acquisitions.insert(current.acquisition);
        if let Some(handoff) = current.handoff.take() {
            self.invalid_acquisitions.insert(handoff.request);
        }
        self.terminal.insert(operation, outcome);

        let mut effects = Vec::new();
        if let Some(lease) = current.resource {
            effects.push(Effect::ReleaseResource { lease });
        }
        match outcome {
            TerminalOutcome::Committed => {
                effects.push(Effect::SubmitFinalDesiredState { operation });
            }
            TerminalOutcome::Cancelled(_) | TerminalOutcome::Failed(_) => {
                effects.push(Effect::RequestCompensation {
                    operation,
                    decision: compensation.unwrap_or(CompensationDecision::Conditional),
                });
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
        if self.invalid_acquisitions.remove(&request) {
            Transition::applied(vec![Effect::ReleaseResource { lease }])
        } else {
            Transition::disposition(Disposition::Rejected(RejectionReason::StaleAcquisition))
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
        CompletionBinding {
            source: source(id),
            gesture: CompletionGesture::Button(1),
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

    #[test]
    fn unrelated_release_does_not_complete_operation() {
        let mut reducer = WindowOperationReducer::default();
        let (operation, acquisition) = begin(&mut reducer, 1, 1);
        activate(&mut reducer, operation, acquisition);
        let unrelated_button = CompletionBinding {
            source: source(1),
            gesture: CompletionGesture::Button(2),
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
    }
}
