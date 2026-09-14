//! Deterministic desired-geometry authority and native settlement primitives.
//!
//! This module deliberately contains no platform objects. Adapters translate native geometry into
//! tagged facts, execute [`NativeRequest`]s, and return causal observations.

use crate::geometry::LogicalRect;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct GeometryRevision(u64);

impl GeometryRevision {
    pub const INITIAL: Self = Self(1);

    pub const fn get(self) -> u64 {
        self.0
    }

    fn next(self) -> Self {
        Self(self.0.checked_add(1).expect("geometry revision exhausted"))
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum GeometryField {
    Placement,
    Presentation,
    RestorePlacement,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GeometryMeaning {
    CanonicalManagedBounds,
    WaylandWindowGeometry,
    SurfaceBufferBounds,
    ServerDecorationBounds,
    Win32OuterBounds,
    ContentBounds,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CoordinateUnits {
    CanonicalLogical,
    OutputLogical { scale_120: u32 },
    NativePhysical,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TaggedGeometry {
    pub rect: LogicalRect,
    pub meaning: GeometryMeaning,
    pub units: CoordinateUnits,
    pub topology_version: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControlMode {
    Enforced,
    Cooperative,
    ExternallyContested,
    Delegated,
}

impl ControlMode {
    pub const fn permits_nickel_write(self) -> bool {
        !matches!(self, Self::Delegated)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FieldOwner {
    Nickel,
    External,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Presentation {
    Normal,
    Maximized,
    Tiled,
    Fullscreen,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Revisioned<T> {
    pub value: T,
    pub revision: GeometryRevision,
    pub owner: FieldOwner,
    pub control: ControlMode,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DesiredRevisions {
    pub placement: GeometryRevision,
    pub presentation: GeometryRevision,
    pub restore_placement: GeometryRevision,
}

/// Capability returned by the sole desired-state writer before an adapter issues an effect.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AuthorizedPlacement {
    pub desired: LogicalRect,
    pub revision: GeometryRevision,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GeometryConstraints {
    pub min_width: i32,
    pub min_height: i32,
    pub max_width: Option<i32>,
    pub max_height: Option<i32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConstraintError {
    NonPositiveMinimum,
    MaximumBelowMinimum,
}

impl GeometryConstraints {
    pub fn validate(self) -> Result<Self, ConstraintError> {
        if self.min_width <= 0 || self.min_height <= 0 {
            return Err(ConstraintError::NonPositiveMinimum);
        }
        if self.max_width.is_some_and(|max| max < self.min_width)
            || self.max_height.is_some_and(|max| max < self.min_height)
        {
            return Err(ConstraintError::MaximumBelowMinimum);
        }
        Ok(self)
    }

    pub fn constrain(self, mut rect: LogicalRect) -> LogicalRect {
        rect.width = rect.width.max(self.min_width);
        rect.height = rect.height.max(self.min_height);
        if let Some(max) = self.max_width {
            rect.width = rect.width.min(max);
        }
        if let Some(max) = self.max_height {
            rect.height = rect.height.min(max);
        }
        rect
    }
}

/// Absolute, unconstrained intent for one anchor epoch. Constraints never feed back into it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GeometryIntent {
    pub anchor: LogicalRect,
    pub delta_x: i64,
    pub delta_y: i64,
    pub delta_width: i64,
    pub delta_height: i64,
}

impl GeometryIntent {
    pub const fn new(anchor: LogicalRect) -> Self {
        Self {
            anchor,
            delta_x: 0,
            delta_y: 0,
            delta_width: 0,
            delta_height: 0,
        }
    }

    pub fn add_resize_delta(&mut self, width: i64, height: i64) {
        self.delta_width = self.delta_width.saturating_add(width);
        self.delta_height = self.delta_height.saturating_add(height);
    }

    pub fn unconstrained(self) -> Result<LogicalRect, IntentError> {
        fn add(value: i32, delta: i64) -> Result<i32, IntentError> {
            i64::from(value)
                .checked_add(delta)
                .and_then(|value| i32::try_from(value).ok())
                .ok_or(IntentError::Overflow)
        }
        Ok(LogicalRect {
            x: add(self.anchor.x, self.delta_x)?,
            y: add(self.anchor.y, self.delta_y)?,
            width: add(self.anchor.width, self.delta_width)?,
            height: add(self.anchor.height, self.delta_height)?,
        })
    }

    pub fn proposal(self, constraints: GeometryConstraints) -> Result<LogicalRect, IntentError> {
        Ok(constraints.constrain(self.unconstrained()?))
    }

    /// Starts a new source/transform epoch at the current effective proposal.
    pub const fn rebase(self, effective: LogicalRect) -> Self {
        Self::new(effective)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IntentError {
    Overflow,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LogicalPointF64 {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CoordinateTransform {
    pub origin_x: f64,
    pub origin_y: f64,
    pub scale_x: f64,
    pub scale_y: f64,
    pub version: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransformError {
    NonFinite,
    ZeroScale,
}

impl CoordinateTransform {
    pub fn validate(self) -> Result<Self, TransformError> {
        if !self.origin_x.is_finite()
            || !self.origin_y.is_finite()
            || !self.scale_x.is_finite()
            || !self.scale_y.is_finite()
        {
            return Err(TransformError::NonFinite);
        }
        if self.scale_x == 0.0 || self.scale_y == 0.0 {
            return Err(TransformError::ZeroScale);
        }
        Ok(self)
    }

    pub fn to_canonical(self, point: LogicalPointF64) -> Result<LogicalPointF64, TransformError> {
        self.validate()?;
        if !point.x.is_finite() || !point.y.is_finite() {
            return Err(TransformError::NonFinite);
        }
        Ok(LogicalPointF64 {
            x: self.origin_x + point.x / self.scale_x,
            y: self.origin_y + point.y / self.scale_y,
        })
    }
}

/// Converts the last source position through its recorded transform, then establishes one new epoch.
pub fn rebase_source_position(
    position: LogicalPointF64,
    old: CoordinateTransform,
    new: CoordinateTransform,
) -> Result<LogicalPointF64, TransformError> {
    let canonical = old.to_canonical(position)?;
    new.validate()?;
    Ok(LogicalPointF64 {
        x: (canonical.x - new.origin_x) * new.scale_x,
        y: (canonical.y - new.origin_y) * new.scale_y,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CompensationBaseline {
    pub placement: LogicalRect,
    pub placement_last_owned: GeometryRevision,
    pub presentation: Presentation,
    pub presentation_last_owned: GeometryRevision,
    pub restore_placement: Option<LogicalRect>,
    pub restore_last_owned: GeometryRevision,
}

impl CompensationBaseline {
    /// Records the latest revision produced by the operation without replacing its initial value.
    pub fn note_owned(&mut self, field: GeometryField, revision: GeometryRevision) {
        match field {
            GeometryField::Placement => self.placement_last_owned = revision,
            GeometryField::Presentation => self.presentation_last_owned = revision,
            GeometryField::RestorePlacement => self.restore_last_owned = revision,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompensationResult {
    Exact,
    Adjusted,
    SkippedSuperseded,
    SkippedNoWriteAuthority,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CompensationReport {
    pub placement: CompensationResult,
    pub presentation: CompensationResult,
    pub restore_placement: CompensationResult,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GeometryAuthority {
    pub base_placement: Revisioned<LogicalRect>,
    pub constrained_proposal: LogicalRect,
    pub presentation: Revisioned<Presentation>,
    pub restore_placement: Revisioned<Option<LogicalRect>>,
    /// Latest native fact, which is never silently promoted to desired geometry.
    pub observed_geometry: Option<(TaggedGeometry, ObservationCausality)>,
    pub topology_version: u64,
    pub constraint_version: u64,
}

impl GeometryAuthority {
    pub fn new(placement: LogicalRect, presentation: Presentation) -> Self {
        Self {
            base_placement: Revisioned {
                value: placement,
                revision: GeometryRevision::INITIAL,
                owner: FieldOwner::Nickel,
                control: ControlMode::Enforced,
            },
            constrained_proposal: placement,
            presentation: Revisioned {
                value: presentation,
                revision: GeometryRevision::INITIAL,
                owner: FieldOwner::Nickel,
                control: ControlMode::Enforced,
            },
            restore_placement: Revisioned {
                value: None,
                revision: GeometryRevision::INITIAL,
                owner: FieldOwner::Nickel,
                control: ControlMode::Enforced,
            },
            observed_geometry: None,
            topology_version: 1,
            constraint_version: 1,
        }
    }

    pub const fn revisions(&self) -> DesiredRevisions {
        DesiredRevisions {
            placement: self.base_placement.revision,
            presentation: self.presentation.revision,
            restore_placement: self.restore_placement.revision,
        }
    }

    pub fn set_placement(&mut self, placement: LogicalRect, constraints: GeometryConstraints) {
        self.base_placement.value = placement;
        self.base_placement.revision = self.base_placement.revision.next();
        self.base_placement.owner = FieldOwner::Nickel;
        self.constrained_proposal = constraints.constrain(placement);
    }

    /// Publishes a desired revision before native realization, including equal-valued writes.
    pub fn authorize_placement(
        &mut self,
        placement: LogicalRect,
        constraints: GeometryConstraints,
    ) -> AuthorizedPlacement {
        self.set_placement(placement, constraints);
        AuthorizedPlacement {
            desired: self.constrained_proposal,
            revision: self.base_placement.revision,
        }
    }

    /// Recomputes a temporary effective placement without replacing the user's base placement.
    pub fn constrain_placement(&mut self, constraints: GeometryConstraints) -> LogicalRect {
        self.constrained_proposal = constraints.constrain(self.base_placement.value);
        self.constrained_proposal
    }

    /// Installs a policy-computed effective placement while retaining the current base placement.
    pub fn set_constrained_proposal(&mut self, proposal: LogicalRect) {
        self.constrained_proposal = proposal;
    }

    pub fn clear_placement_constraint(&mut self) -> LogicalRect {
        self.constrained_proposal = self.base_placement.value;
        self.constrained_proposal
    }

    pub fn set_presentation(&mut self, presentation: Presentation) {
        self.presentation.value = presentation;
        self.presentation.revision = self.presentation.revision.next();
        self.presentation.owner = FieldOwner::Nickel;
    }

    pub fn set_restore_placement(&mut self, placement: Option<LogicalRect>) {
        self.restore_placement.value = placement;
        self.restore_placement.revision = self.restore_placement.revision.next();
        self.restore_placement.owner = FieldOwner::Nickel;
    }

    pub fn observe(&mut self, fact: TaggedGeometry, causality: ObservationCausality) {
        self.observed_geometry = Some((fact, causality));
        if causality == ObservationCausality::Independent {
            self.base_placement.owner = FieldOwner::External;
        } else if causality == ObservationCausality::Unknown {
            self.base_placement.owner = FieldOwner::Unknown;
        }
    }

    pub fn baseline(&self) -> CompensationBaseline {
        CompensationBaseline {
            placement: self.base_placement.value,
            placement_last_owned: self.base_placement.revision,
            presentation: self.presentation.value,
            presentation_last_owned: self.presentation.revision,
            restore_placement: self.restore_placement.value,
            restore_last_owned: self.restore_placement.revision,
        }
    }

    pub fn compensate(
        &mut self,
        baseline: CompensationBaseline,
        constraints: GeometryConstraints,
    ) -> CompensationReport {
        let placement = compensate_field(
            &mut self.base_placement,
            baseline.placement,
            baseline.placement_last_owned,
            |value| constraints.constrain(value),
        );
        self.constrained_proposal = constraints.constrain(self.base_placement.value);
        let presentation = compensate_field(
            &mut self.presentation,
            baseline.presentation,
            baseline.presentation_last_owned,
            |value| value,
        );
        let restore_placement = compensate_field(
            &mut self.restore_placement,
            baseline.restore_placement,
            baseline.restore_last_owned,
            |value| value,
        );
        CompensationReport {
            placement,
            presentation,
            restore_placement,
        }
    }
}

fn compensate_field<T: Copy + PartialEq>(
    field: &mut Revisioned<T>,
    baseline: T,
    last_owned: GeometryRevision,
    constrain: impl FnOnce(T) -> T,
) -> CompensationResult {
    if field.owner != FieldOwner::Nickel || !field.control.permits_nickel_write() {
        return CompensationResult::SkippedNoWriteAuthority;
    }
    if field.revision != last_owned {
        return CompensationResult::SkippedSuperseded;
    }
    let adjusted = constrain(baseline);
    field.value = adjusted;
    field.revision = field.revision.next();
    if adjusted == baseline {
        CompensationResult::Exact
    } else {
        CompensationResult::Adjusted
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct NativeRequestId(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeRequest {
    pub id: NativeRequestId,
    pub mapping_generation: u64,
    pub desired: DesiredRevisions,
    pub placement: LogicalRect,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObservationCausality {
    Correlated(NativeRequestId),
    Independent,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SettlementStatus {
    Pending,
    Applied,
    AppliedWithAdjustment,
    Superseded,
    Failed,
    Unconfirmed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SettlementLimits {
    pub deadline_tick: u64,
    pub max_corrections: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Settlement {
    pub request: NativeRequest,
    pub status: SettlementStatus,
    pub limits: SettlementLimits,
    pub corrections: u8,
    pub observed: Option<TaggedGeometry>,
}

impl Settlement {
    pub const fn new(request: NativeRequest, limits: SettlementLimits) -> Self {
        Self {
            request,
            status: SettlementStatus::Pending,
            limits,
            corrections: 0,
            observed: None,
        }
    }

    pub fn observe(&mut self, fact: TaggedGeometry, causality: ObservationCausality) {
        self.observed = Some(fact);
        if self.status != SettlementStatus::Pending {
            return;
        }
        self.status = match causality {
            ObservationCausality::Correlated(id) if id == self.request.id => {
                if fact.rect == self.request.placement {
                    SettlementStatus::Applied
                } else {
                    SettlementStatus::AppliedWithAdjustment
                }
            }
            ObservationCausality::Independent => SettlementStatus::Superseded,
            ObservationCausality::Unknown | ObservationCausality::Correlated(_) => {
                SettlementStatus::Pending
            }
        };
    }

    pub fn fail(&mut self) {
        if self.status == SettlementStatus::Pending {
            self.status = SettlementStatus::Failed;
        }
    }

    pub fn expire(&mut self, now_tick: u64) {
        if self.status == SettlementStatus::Pending && now_tick >= self.limits.deadline_tick {
            self.status = SettlementStatus::Unconfirmed;
        }
    }

    pub fn reserve_correction(&mut self, now_tick: u64) -> bool {
        if self.status != SettlementStatus::Pending
            || now_tick >= self.limits.deadline_tick
            || self.corrections >= self.limits.max_corrections
        {
            return false;
        }
        self.corrections += 1;
        true
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PendingPlacementIntent {
    pub placement: LogicalRect,
    pub mapping_generation: u64,
    pub placement_revision: GeometryRevision,
    pub expires_at_tick: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PendingIntentResult {
    Apply(LogicalRect),
    Superseded,
    MappingReplaced,
    Expired,
}

impl PendingPlacementIntent {
    pub fn reconcile(
        self,
        now_tick: u64,
        mapping_generation: u64,
        current_revision: GeometryRevision,
    ) -> PendingIntentResult {
        if now_tick >= self.expires_at_tick {
            PendingIntentResult::Expired
        } else if mapping_generation != self.mapping_generation {
            PendingIntentResult::MappingReplaced
        } else if current_revision != self.placement_revision {
            PendingIntentResult::Superseded
        } else {
            PendingIntentResult::Apply(self.placement)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(width: i32) -> LogicalRect {
        LogicalRect {
            x: 0,
            y: 0,
            width,
            height: 60,
        }
    }

    fn constraints(max_width: i32) -> GeometryConstraints {
        GeometryConstraints {
            min_width: 20,
            min_height: 20,
            max_width: Some(max_width),
            max_height: None,
        }
    }

    #[test]
    fn constraints_preserve_overshoot_across_samples() {
        let mut intent = GeometryIntent::new(rect(90));
        intent.add_resize_delta(20, 0);
        assert_eq!(intent.proposal(constraints(100)).unwrap().width, 100);
        intent.add_resize_delta(-5, 0);
        assert_eq!(intent.proposal(constraints(100)).unwrap().width, 100);
        assert_eq!(intent.unconstrained().unwrap().width, 105);
    }

    #[test]
    fn conditional_compensation_cannot_overwrite_a_newer_revision() {
        let mut authority = GeometryAuthority::new(rect(90), Presentation::Normal);
        let baseline = authority.baseline();
        authority.set_placement(rect(110), constraints(200));
        let report = authority.compensate(baseline, constraints(200));
        assert_eq!(report.placement, CompensationResult::SkippedSuperseded);
        assert_eq!(authority.base_placement.value, rect(110));
    }

    #[test]
    fn equal_valued_newer_write_supersedes_compensation_revision() {
        let mut authority = GeometryAuthority::new(rect(90), Presentation::Normal);
        let baseline = authority.baseline();
        authority.authorize_placement(rect(90), constraints(200));

        let report = authority.compensate(baseline, constraints(200));
        assert_eq!(report.placement, CompensationResult::SkippedSuperseded);
        assert_eq!(authority.base_placement.value, rect(90));
    }

    #[test]
    fn operation_can_advance_its_guard_without_replacing_initial_baseline() {
        let mut authority = GeometryAuthority::new(rect(90), Presentation::Normal);
        let mut baseline = authority.baseline();
        authority.set_placement(rect(110), constraints(200));
        baseline.note_owned(GeometryField::Placement, authority.revisions().placement);
        let report = authority.compensate(baseline, constraints(200));
        assert_eq!(report.placement, CompensationResult::Exact);
        assert_eq!(authority.base_placement.value, rect(90));
    }

    #[test]
    fn observed_geometry_remains_a_fact_and_withdraws_unknown_authority() {
        let mut authority = GeometryAuthority::new(rect(90), Presentation::Normal);
        authority.observe(
            TaggedGeometry {
                rect: rect(120),
                meaning: GeometryMeaning::Win32OuterBounds,
                units: CoordinateUnits::NativePhysical,
                topology_version: 1,
            },
            ObservationCausality::Unknown,
        );
        assert_eq!(authority.base_placement.value, rect(90));
        assert_eq!(authority.base_placement.owner, FieldOwner::Unknown);
    }

    #[test]
    fn compensation_reports_current_constraint_adjustment() {
        let mut authority = GeometryAuthority::new(rect(190), Presentation::Normal);
        let baseline = authority.baseline();
        let report = authority.compensate(baseline, constraints(100));
        assert_eq!(report.placement, CompensationResult::Adjusted);
        assert_eq!(authority.base_placement.value, rect(100));
    }

    #[test]
    fn delegated_field_withdraws_compensation_permission() {
        let mut authority = GeometryAuthority::new(rect(90), Presentation::Normal);
        let baseline = authority.baseline();
        authority.base_placement.control = ControlMode::Delegated;
        let report = authority.compensate(baseline, constraints(200));
        assert_eq!(
            report.placement,
            CompensationResult::SkippedNoWriteAuthority
        );
    }

    #[test]
    fn settlement_is_bounded_and_late_facts_do_not_revive_it() {
        let request = NativeRequest {
            id: NativeRequestId(7),
            mapping_generation: 2,
            desired: DesiredRevisions {
                placement: GeometryRevision::INITIAL,
                presentation: GeometryRevision::INITIAL,
                restore_placement: GeometryRevision::INITIAL,
            },
            placement: rect(100),
        };
        let mut settlement = Settlement::new(
            request,
            SettlementLimits {
                deadline_tick: 10,
                max_corrections: 2,
            },
        );
        assert!(settlement.reserve_correction(8));
        assert!(settlement.reserve_correction(9));
        assert!(!settlement.reserve_correction(9));
        settlement.expire(10);
        assert_eq!(settlement.status, SettlementStatus::Unconfirmed);
        settlement.observe(
            TaggedGeometry {
                rect: rect(100),
                meaning: GeometryMeaning::CanonicalManagedBounds,
                units: CoordinateUnits::CanonicalLogical,
                topology_version: 1,
            },
            ObservationCausality::Correlated(NativeRequestId(7)),
        );
        assert_eq!(settlement.status, SettlementStatus::Unconfirmed);
        assert_eq!(settlement.observed.unwrap().rect, rect(100));
    }

    #[test]
    fn source_transform_rebase_preserves_canonical_position() {
        let old = CoordinateTransform {
            origin_x: -100.0,
            origin_y: 20.0,
            scale_x: 2.0,
            scale_y: 2.0,
            version: 1,
        };
        let new = CoordinateTransform {
            origin_x: 0.0,
            origin_y: 0.0,
            scale_x: 1.0,
            scale_y: 1.0,
            version: 2,
        };
        assert_eq!(
            rebase_source_position(LogicalPointF64 { x: 40.0, y: 60.0 }, old, new).unwrap(),
            LogicalPointF64 { x: -80.0, y: 50.0 }
        );
    }

    #[test]
    fn no_output_intent_is_bounded_and_revision_guarded() {
        let intent = PendingPlacementIntent {
            placement: rect(90),
            mapping_generation: 4,
            placement_revision: GeometryRevision::INITIAL,
            expires_at_tick: 20,
        };
        assert_eq!(
            intent.reconcile(10, 4, GeometryRevision::INITIAL),
            PendingIntentResult::Apply(rect(90))
        );
        assert_eq!(
            intent.reconcile(10, 4, GeometryRevision(2)),
            PendingIntentResult::Superseded
        );
        assert_eq!(
            intent.reconcile(20, 4, GeometryRevision::INITIAL),
            PendingIntentResult::Expired
        );
    }

    #[test]
    fn removing_temporary_constraint_uses_newer_user_base_placement() {
        let mut authority = GeometryAuthority::new(rect(90), Presentation::Normal);
        authority.set_constrained_proposal(rect(70));
        authority.set_placement(rect(120), constraints(200));
        authority.set_constrained_proposal(rect(80));

        assert_eq!(authority.clear_placement_constraint(), rect(120));
        assert_eq!(authority.constrained_proposal, rect(120));
    }
}
