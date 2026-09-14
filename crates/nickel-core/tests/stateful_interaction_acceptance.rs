//! Bounded generated acceptance sequences over production interaction reducers.

use std::time::Duration;

use nickel_core::geometry_authority::ControlMode;
use nickel_core::{
    acceptance::{BoundedTrace, SyntheticClock},
    focus::{
        FocusRequestPhase, FocusScope, FocusSecurityEpoch, FocusTargetLifetime, FocusTransactions,
    },
    geometry::LogicalRect,
    geometry_authority::{
        GeometryAuthority, GeometryConstraints, GeometryMeaning, NativeRequest, NativeRequestId,
        ObservationCausality, PendingIntentResult, PendingPlacementIntent, Presentation,
        Settlement, SettlementLimits, SettlementStatus, TaggedGeometry,
    },
    window_operation::{
        AcquisitionId, BeginRequest, CancellationReason, CompletionBinding, CompletionGesture,
        Disposition, Effect, FailureReason, MappingGeneration, NativeLifetimeId, OperationKind,
        RejectionReason, ResourceLeaseId, SeatId, Source, SourceGeneration, SourceId,
        TerminalOutcome, WindowId, WindowMapping, WindowOperationReducer,
    },
};

const STEPS_PER_SEED: usize = 24;
const SEEDS: [u64; 4] = [1, 0x5eed, 0xc0ffee, u64::MAX - 1];

fn next(seed: &mut u64) -> u64 {
    *seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
    *seed
}

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

fn mapping(generation: u64) -> WindowMapping {
    WindowMapping {
        window: WindowId::new(1),
        native_lifetime: NativeLifetimeId::new(generation),
        generation: MappingGeneration::new(generation),
    }
}

fn begin(
    reducer: &mut WindowOperationReducer,
    generation: u64,
) -> (nickel_core::window_operation::OperationId, AcquisitionId) {
    let (operation, transition) = reducer.begin(BeginRequest {
        seat: SeatId::new(1),
        subject: mapping(generation),
        kind: OperationKind::Move,
        control: ControlMode::Enforced,
        origin: binding(1),
        optional_update_sources: vec![source(9)],
    });
    let operation = operation.expect("unoccupied fixture seat must admit");
    let Effect::Acquire { request, .. } = transition.effects[0] else {
        panic!("begin did not request production acquisition")
    };
    (operation, request)
}

fn assert_occupied(
    reducer: &mut WindowOperationReducer,
    by: nickel_core::window_operation::OperationId,
) {
    let (other, transition) = reducer.begin(BeginRequest {
        seat: SeatId::new(1),
        subject: mapping(99),
        kind: OperationKind::Move,
        control: ControlMode::Enforced,
        origin: binding(7),
        optional_update_sources: vec![],
    });
    assert_eq!(other, None);
    assert_eq!(
        transition.disposition,
        Disposition::Rejected(RejectionReason::SeatOccupied { by })
    );
}

#[test]
fn generated_operation_lifecycles_release_admission_and_fence_stale_tails() {
    for mut seed in SEEDS {
        let mut reducer = WindowOperationReducer::default();
        let mut trace = BoundedTrace::new(STEPS_PER_SEED * 3);
        let mut generation = 1;

        for _ in 0..STEPS_PER_SEED {
            let (operation, acquisition) = begin(&mut reducer, generation);
            assert_occupied(&mut reducer, operation);

            if next(&mut seed) & 1 == 0 {
                let terminal = reducer.fail(operation, FailureReason::AdmissionTimedOut);
                trace.record(terminal.disposition);
                assert_eq!(
                    reducer.terminal_outcome(operation),
                    Some(TerminalOutcome::Failed(FailureReason::AdmissionTimedOut))
                );
                let late =
                    reducer.acquired(operation, acquisition, ResourceLeaseId::new(generation));
                assert!(matches!(
                    late.effects.as_slice(),
                    [Effect::ReleaseResource { .. }]
                ));
            } else {
                reducer.acquired(operation, acquisition, ResourceLeaseId::new(generation));
                reducer.activate(operation);
                let handoff = reducer.request_handoff(operation, binding(2), vec![source(9)]);
                let Effect::Acquire { request, .. } = handoff.effects[0] else {
                    panic!("handoff did not request acquisition")
                };
                if next(&mut seed).is_multiple_of(3) {
                    reducer.handoff_acquired(
                        operation,
                        request,
                        ResourceLeaseId::new(generation + 100),
                    );
                    assert_eq!(
                        reducer.release(operation, binding(1)).disposition,
                        Disposition::ConsumedTail
                    );
                    reducer.release(operation, binding(2));
                    assert_eq!(
                        reducer.terminal_outcome(operation),
                        Some(TerminalOutcome::Committed)
                    );
                } else if next(&mut seed) & 1 == 0 {
                    reducer.source_disconnected(operation, source(1));
                    assert_eq!(
                        reducer.terminal_outcome(operation),
                        Some(TerminalOutcome::Cancelled(
                            CancellationReason::CompletionSourceLost
                        ))
                    );
                    let late = reducer.handoff_acquired(
                        operation,
                        request,
                        ResourceLeaseId::new(generation + 100),
                    );
                    assert!(matches!(
                        late.effects.as_slice(),
                        [Effect::ReleaseResource { .. }]
                    ));
                } else {
                    reducer.cancel(operation, CancellationReason::TargetUnmapped);
                    assert_eq!(
                        reducer.terminal_outcome(operation),
                        Some(TerminalOutcome::Cancelled(
                            CancellationReason::TargetUnmapped
                        ))
                    );
                    let late = reducer.handoff_acquired(
                        operation,
                        request,
                        ResourceLeaseId::new(generation + 100),
                    );
                    assert!(matches!(
                        late.effects.as_slice(),
                        [Effect::ReleaseResource { .. }]
                    ));
                }
            }
            assert!(reducer.operation(operation).is_none());
            generation += 1;
        }

        assert_eq!(trace.dropped(), 0);
    }
}

fn rect(width: i32) -> LogicalRect {
    LogicalRect {
        x: 0,
        y: 0,
        width,
        height: 60,
    }
}

fn fact(rect: LogicalRect) -> TaggedGeometry {
    TaggedGeometry {
        rect,
        meaning: GeometryMeaning::CanonicalManagedBounds,
        units: nickel_core::geometry_authority::CoordinateUnits::CanonicalLogical,
        topology_version: 1,
    }
}

#[test]
fn no_output_placement_intent_is_bounded_by_time_mapping_and_revision() {
    let authority = GeometryAuthority::new(rect(80), Presentation::Normal);
    let original_revision = authority.revisions().placement;
    let intent = PendingPlacementIntent {
        placement: rect(100),
        mapping_generation: 7,
        placement_revision: original_revision,
        expires_at_tick: 10,
    };

    assert_eq!(
        intent.reconcile(9, 7, original_revision),
        PendingIntentResult::Apply(rect(100))
    );
    assert_eq!(
        intent.reconcile(9, 8, original_revision),
        PendingIntentResult::MappingReplaced
    );
    let mut changed = authority;
    changed.set_placement(
        rect(90),
        GeometryConstraints {
            min_width: 20,
            min_height: 20,
            max_width: None,
            max_height: None,
        },
    );
    assert_eq!(
        intent.reconcile(9, 7, changed.revisions().placement),
        PendingIntentResult::Superseded
    );
    assert_eq!(
        intent.reconcile(10, 7, original_revision),
        PendingIntentResult::Expired
    );
}

#[test]
fn generated_late_geometry_and_focus_events_never_revive_superseded_authority() {
    let constraints = GeometryConstraints {
        min_width: 20,
        min_height: 20,
        max_width: Some(200),
        max_height: None,
    };
    for mut seed in SEEDS {
        let mut authority = GeometryAuthority::new(rect(80), Presentation::Normal);
        let mut focus = FocusTransactions::default();
        let mut clock = SyntheticClock::default();

        for step in 0..STEPS_PER_SEED {
            let old = focus.request_at(
                step,
                FocusTargetLifetime::Generation(step as u64),
                None,
                FocusScope::Ordinary,
                FocusSecurityEpoch(1),
                clock.now(),
                Duration::from_millis(5),
            );
            let current = focus.request_at(
                step + 1,
                FocusTargetLifetime::Generation(step as u64 + 1),
                None,
                FocusScope::Ordinary,
                FocusSecurityEpoch(1),
                clock.now(),
                Duration::from_millis(5),
            );
            assert_eq!(focus.phase(&old), Some(FocusRequestPhase::Superseded));
            assert!(!focus.acknowledge_at(&old, clock.now()));

            let request = NativeRequest {
                id: NativeRequestId(step as u64 + 1),
                mapping_generation: step as u64 + 1,
                desired: authority.revisions(),
                placement: authority.constrained_proposal,
            };
            let mut settlement = Settlement::new(
                request,
                SettlementLimits {
                    deadline_tick: 2,
                    max_corrections: 1,
                },
            );
            settlement.expire(2);
            assert_eq!(settlement.status, SettlementStatus::Unconfirmed);

            authority.set_placement(rect(90 + (next(&mut seed) % 40) as i32), constraints);
            let superseding = authority.base_placement.value;
            settlement.observe(
                fact(request.placement),
                ObservationCausality::Correlated(request.id),
            );
            assert_eq!(settlement.status, SettlementStatus::Unconfirmed);
            assert_eq!(authority.base_placement.value, superseding);

            clock.advance(Duration::from_millis(5));
            assert!(focus.advance_to(clock.now()));
            assert_eq!(focus.phase(&current), Some(FocusRequestPhase::TimedOut));
        }
    }
}
