//! Persistent, bounded state for dense graphical surfaces.
//!
//! [`RetainedSurfaceHost`] is deliberately application-owned. Replacing a
//! declarative [`crate::UiFrame`] therefore does not replace the surface's
//! state, renderer resources, or accepted output. The host validates every
//! update before publishing it and exposes the accepted paint as an ordinary
//! [`crate::CustomPaint`] component, so it does not introduce another event
//! loop, renderer, or focus authority.

use std::{fmt, hash::Hasher, time::Duration};

use crate::{
    ActionKind, Component, CustomPaint, Layer, Point, Rect, SemanticRole, Size, UiId,
    backend::PaintCommand, ui::Element,
};

/// Revisions that can invalidate retained graphical output without changing
/// the surface's logical content revision.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RetainedSurfaceRevisions {
    pub content: u64,
    pub theme: u64,
    pub font: u64,
    pub assets: u64,
}

/// Complete, deterministic input to one retained-surface update.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RetainedSurfaceInput {
    pub bounds: Rect,
    pub scale_factor: f32,
    pub time: Duration,
    pub revisions: RetainedSurfaceRevisions,
}

impl RetainedSurfaceInput {
    pub fn new(bounds: Rect, scale_factor: f32, revisions: RetainedSurfaceRevisions) -> Self {
        Self {
            bounds,
            scale_factor,
            time: Duration::ZERO,
            revisions,
        }
    }

    pub const fn at_time(mut self, time: Duration) -> Self {
        self.time = time;
        self
    }
}

/// Work classes changed since the last admitted update.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RetainedSurfaceChanges(u16);

impl RetainedSurfaceChanges {
    pub const BOUNDS: Self = Self(1 << 0);
    pub const SCALE: Self = Self(1 << 1);
    pub const THEME: Self = Self(1 << 2);
    pub const FONT: Self = Self(1 << 3);
    pub const ASSETS: Self = Self(1 << 4);
    pub const TIME: Self = Self(1 << 5);
    pub const CONTENT: Self = Self(1 << 6);
    pub const ALL: Self = Self((1 << 7) - 1);

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    const fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }
}

/// Host lifecycle transitions that invalidate resources outside ordinary
/// content updates.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RetainedSurfaceLifecycle {
    Attached,
    Detached,
    ScaleChanged { old: u32, new: u32 },
    RendererLost,
    Destroyed,
}

/// A bounded hit-test region. The surrounding Nickel UI remains the event and
/// focus authority; a surface can only describe semantic actions for its own
/// allocation.
#[derive(Clone, Debug, PartialEq)]
pub struct RetainedSurfaceInteraction {
    pub id: UiId,
    pub bounds: Rect,
    pub action: ActionKind,
    pub focusable: bool,
}

/// A bounded accessibility record owned by the surrounding native surface.
#[derive(Clone, Debug, PartialEq)]
pub struct RetainedSurfaceSemantic {
    pub id: UiId,
    pub bounds: Rect,
    pub role: SemanticRole,
    pub label: Option<String>,
    pub state: Option<String>,
    pub protected: bool,
}

/// Explicit results of one model update. Paint commands and damage use the
/// same absolute logical coordinate space as `input.bounds`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RetainedSurfaceOutput {
    pub measured: Size,
    pub paint: Vec<PaintCommand>,
    pub interactions: Vec<RetainedSurfaceInteraction>,
    pub semantics: Vec<RetainedSurfaceSemantic>,
    pub damage: Vec<Rect>,
}

/// Hard admission bounds for model-owned output and persistent state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RetainedSurfaceLimits {
    pub max_paint_commands: usize,
    pub max_interactions: usize,
    pub max_semantics: usize,
    pub max_damage_rects: usize,
    pub max_retained_bytes: usize,
}

impl Default for RetainedSurfaceLimits {
    fn default() -> Self {
        Self {
            max_paint_commands: 65_536,
            max_interactions: 16_384,
            max_semantics: 16_384,
            max_damage_rects: 1_024,
            max_retained_bytes: 64 * 1024 * 1024,
        }
    }
}

/// A dense graphical surface with application-owned persistent state.
pub trait RetainedSurfaceModel {
    /// Produce a complete candidate for `input`. `changes` identifies the
    /// narrowest inputs changed since the previously accepted candidate.
    fn update(
        &mut self,
        input: RetainedSurfaceInput,
        changes: RetainedSurfaceChanges,
    ) -> Result<RetainedSurfaceOutput, String>;

    /// Notify the model of lifecycle transitions. Implementations must release
    /// renderer-dependent resources on `RendererLost`, and all live resources
    /// on `Detached` and `Destroyed` as appropriate.
    fn lifecycle(&mut self, event: RetainedSurfaceLifecycle);

    /// Bytes retained by the model, excluding the accepted output accounted by
    /// the host. Admission rejects a candidate above its configured bound.
    fn retained_bytes(&self) -> usize;
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RetainedSurfaceDiagnostics {
    pub update_calls: u64,
    pub unchanged_reuses: u64,
    pub rejected_updates: u64,
    pub lifecycle_events: u64,
    pub paint_commands: usize,
    pub interaction_records: usize,
    pub semantic_records: usize,
    pub damage_rects: usize,
    pub model_retained_bytes: usize,
    pub output_retained_bytes: usize,
}

/// An accepted immutable surface result suitable for inspection, headless
/// testing, or emission as a native custom-paint component.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RetainedSurfaceSnapshot {
    pub input: Option<RetainedSurfaceInput>,
    pub output: RetainedSurfaceOutput,
}

impl RetainedSurfaceSnapshot {
    pub fn component<Message>(&self) -> CustomPaint<Message> {
        let origin = self
            .input
            .map_or(Point::default(), |input| input.bounds.origin);
        CustomPaint::commands(
            self.output
                .paint
                .iter()
                .cloned()
                .map(|command| translate_command(command, -origin.x, -origin.y))
                .collect(),
        )
        .width(self.output.measured.width)
        .height(self.output.measured.height)
    }

    /// Bridge the admitted specialized surface into Nickel's ordinary native
    /// frame authority. Paint stays a bounded custom-paint child; supported
    /// action/semantic regions become normal keyed UI nodes, so focus,
    /// controller activation, hit testing, and accessibility continue through
    /// [`crate::UiFrame`] rather than a second event loop.
    ///
    /// The bridge intentionally rejects contract shapes the general component
    /// vocabulary cannot represent faithfully yet: non-focusable pointer-only
    /// actions, value actions, and protected semantic records.
    pub fn try_element<Message: Clone>(
        &self,
        id: impl Into<UiId>,
        mut message: impl FnMut(&RetainedSurfaceInteraction) -> Message,
    ) -> Result<Element<Message>, String> {
        let input = self
            .input
            .ok_or_else(|| "retained surface has no accepted output".to_owned())?;
        if self.output.semantics.iter().any(|record| record.protected) {
            return Err(
                "protected retained semantics require a protected native projection".into(),
            );
        }
        if self.output.interactions.iter().any(|record| {
            !record.focusable
                || !matches!(
                    record.action,
                    ActionKind::Activate | ActionKind::ContextMenu
                )
        }) {
            return Err(
                "retained interaction cannot be represented by a native action node".into(),
            );
        }

        let root_id = id.into();
        let mut layer = Layer::new()
            .id(root_id.clone())
            .width(input.bounds.size.width)
            .height(input.bounds.size.height)
            .child(self.component());
        for interaction in &self.output.interactions {
            let local = local_rect(interaction.bounds, input.bounds.origin);
            let semantic = self
                .output
                .semantics
                .iter()
                .find(|semantic| semantic.id == interaction.id);
            let mut node = Layer::new()
                .id(interaction.id.clone())
                .width(local.size.width)
                .height(local.size.height)
                .into_element()
                .position(local.origin);
            node = match interaction.action {
                ActionKind::Activate => node.message(message(interaction)),
                ActionKind::ContextMenu => node.context_message(message(interaction)),
                _ => unreachable!("unsupported actions were rejected above"),
            };
            if let Some(semantic) = semantic {
                node = node.semantic_role(semantic.role);
                if let Some(label) = &semantic.label {
                    node = node.accessibility_label(label.clone());
                }
                if let Some(state) = &semantic.state {
                    node = node.accessibility_state(state.clone());
                }
            }
            layer = layer.child(node);
        }
        for semantic in self.output.semantics.iter().filter(|semantic| {
            !self
                .output
                .interactions
                .iter()
                .any(|interaction| interaction.id == semantic.id)
        }) {
            let local = local_rect(semantic.bounds, input.bounds.origin);
            let mut node = Layer::new()
                .id(semantic.id.clone())
                .width(local.size.width)
                .height(local.size.height)
                .into_element()
                .position(local.origin)
                .semantic_role(semantic.role);
            if let Some(label) = &semantic.label {
                node = node.accessibility_label(label.clone());
            }
            if let Some(state) = &semantic.state {
                node = node.accessibility_state(state.clone());
            }
            layer = layer.child(node);
        }
        Ok(layer
            .into_element()
            .content_revision(input_content_revision(input)))
    }

    pub fn interaction_at(&self, point: Point) -> Option<&RetainedSurfaceInteraction> {
        self.output
            .interactions
            .iter()
            .rev()
            .find(|region| contains(region.bounds, point))
    }
}

fn input_content_revision(input: RetainedSurfaceInput) -> u64 {
    let mut revision = std::collections::hash_map::DefaultHasher::new();
    for value in [
        input.bounds.origin.x.to_bits(),
        input.bounds.origin.y.to_bits(),
        input.bounds.size.width.to_bits(),
        input.bounds.size.height.to_bits(),
        input.scale_factor.to_bits(),
    ] {
        revision.write_u32(value);
    }
    revision.write_u128(input.time.as_nanos());
    for value in [
        input.revisions.content,
        input.revisions.theme,
        input.revisions.font,
        input.revisions.assets,
    ] {
        revision.write_u64(value);
    }
    revision.finish()
}

fn local_rect(rect: Rect, origin: Point) -> Rect {
    Rect::new(
        rect.origin.x - origin.x,
        rect.origin.y - origin.y,
        rect.size.width,
        rect.size.height,
    )
}

fn translate_command(mut command: PaintCommand, dx: f32, dy: f32) -> PaintCommand {
    let translate = |rect: &mut Rect| {
        rect.origin.x += dx;
        rect.origin.y += dy;
    };
    match &mut command {
        PaintCommand::BackdropBlur { rect, .. }
        | PaintCommand::Fill { rect, .. }
        | PaintCommand::TopRoundedFill { rect, .. }
        | PaintCommand::RoundedFill { rect, .. }
        | PaintCommand::Gradient { rect, .. }
        | PaintCommand::RoundedStroke { rect, .. }
        | PaintCommand::Stroke { rect, .. }
        | PaintCommand::OverlayFill { rect, .. }
        | PaintCommand::OverlayStroke { rect, .. } => translate(rect),
        PaintCommand::Text { bounds, .. }
        | PaintCommand::StyledText { bounds, .. }
        | PaintCommand::Image { bounds, .. } => translate(bounds),
        PaintCommand::PushClip(_) | PaintCommand::PopClip => {}
    }
    command
}

/// Persistent owner and transactional admission boundary for one specialized
/// retained surface.
pub struct RetainedSurfaceHost<M: RetainedSurfaceModel> {
    model: M,
    limits: RetainedSurfaceLimits,
    accepted: RetainedSurfaceSnapshot,
    diagnostics: RetainedSurfaceDiagnostics,
    attached: bool,
    destroyed: bool,
}

impl<M: RetainedSurfaceModel> fmt::Debug for RetainedSurfaceHost<M> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RetainedSurfaceHost")
            .field("limits", &self.limits)
            .field("accepted", &self.accepted)
            .field("diagnostics", &self.diagnostics)
            .field("attached", &self.attached)
            .field("destroyed", &self.destroyed)
            .finish_non_exhaustive()
    }
}

impl<M: RetainedSurfaceModel> RetainedSurfaceHost<M> {
    pub fn new(model: M, limits: RetainedSurfaceLimits) -> Result<Self, String> {
        let retained_bytes = model.retained_bytes();
        if retained_bytes > limits.max_retained_bytes {
            return Err("retained surface model exceeds its byte budget".into());
        }
        Ok(Self {
            model,
            limits,
            accepted: RetainedSurfaceSnapshot::default(),
            diagnostics: RetainedSurfaceDiagnostics {
                model_retained_bytes: retained_bytes,
                ..RetainedSurfaceDiagnostics::default()
            },
            attached: false,
            destroyed: false,
        })
    }

    /// Update and transactionally publish a complete candidate. An unchanged
    /// input returns the previous immutable snapshot without executing model
    /// code. Failed admission leaves the previous snapshot authoritative.
    pub fn update(
        &mut self,
        input: RetainedSurfaceInput,
    ) -> Result<&RetainedSurfaceSnapshot, String> {
        if self.destroyed {
            return Err("retained surface was destroyed".into());
        }
        validate_input(input)?;
        if !self.attached {
            self.notify(RetainedSurfaceLifecycle::Attached);
            self.attached = true;
        }
        let changes = self
            .accepted
            .input
            .map_or(RetainedSurfaceChanges::ALL, |previous| {
                changes(previous, input)
            });
        if changes.is_empty() {
            self.diagnostics.unchanged_reuses = self.diagnostics.unchanged_reuses.saturating_add(1);
            return Ok(&self.accepted);
        }
        if let Some(previous) = self.accepted.input
            && changes.contains(RetainedSurfaceChanges::SCALE)
        {
            self.notify(RetainedSurfaceLifecycle::ScaleChanged {
                old: scale_key(previous.scale_factor),
                new: scale_key(input.scale_factor),
            });
        }
        self.diagnostics.update_calls = self.diagnostics.update_calls.saturating_add(1);
        let candidate = match self.model.update(input, changes) {
            Ok(candidate) => candidate,
            Err(error) => {
                self.diagnostics.rejected_updates =
                    self.diagnostics.rejected_updates.saturating_add(1);
                return Err(error);
            }
        };
        let model_bytes = self.model.retained_bytes();
        if let Err(error) = validate_output(&candidate, input.bounds, self.limits, model_bytes) {
            self.diagnostics.rejected_updates = self.diagnostics.rejected_updates.saturating_add(1);
            return Err(error);
        }
        let output_bytes = output_bytes(&candidate);
        self.accepted = RetainedSurfaceSnapshot {
            input: Some(input),
            output: candidate,
        };
        self.diagnostics.paint_commands = self.accepted.output.paint.len();
        self.diagnostics.interaction_records = self.accepted.output.interactions.len();
        self.diagnostics.semantic_records = self.accepted.output.semantics.len();
        self.diagnostics.damage_rects = self.accepted.output.damage.len();
        self.diagnostics.model_retained_bytes = model_bytes;
        self.diagnostics.output_retained_bytes = output_bytes;
        Ok(&self.accepted)
    }

    pub fn snapshot(&self) -> &RetainedSurfaceSnapshot {
        &self.accepted
    }

    pub const fn diagnostics(&self) -> RetainedSurfaceDiagnostics {
        self.diagnostics
    }

    pub fn detach(&mut self) {
        if self.attached && !self.destroyed {
            self.notify(RetainedSurfaceLifecycle::Detached);
            self.attached = false;
            self.accepted = RetainedSurfaceSnapshot::default();
            self.clear_output_diagnostics();
        }
    }

    pub fn renderer_lost(&mut self) {
        if !self.destroyed {
            self.notify(RetainedSurfaceLifecycle::RendererLost);
            self.accepted = RetainedSurfaceSnapshot::default();
            self.clear_output_diagnostics();
        }
    }

    pub fn destroy(&mut self) {
        if !self.destroyed {
            if self.attached {
                self.notify(RetainedSurfaceLifecycle::Detached);
                self.attached = false;
            }
            self.notify(RetainedSurfaceLifecycle::Destroyed);
            self.destroyed = true;
            self.accepted = RetainedSurfaceSnapshot::default();
            self.clear_output_diagnostics();
        }
    }

    fn notify(&mut self, event: RetainedSurfaceLifecycle) {
        self.model.lifecycle(event);
        self.diagnostics.lifecycle_events = self.diagnostics.lifecycle_events.saturating_add(1);
        self.diagnostics.model_retained_bytes = self.model.retained_bytes();
    }

    fn clear_output_diagnostics(&mut self) {
        self.diagnostics.paint_commands = 0;
        self.diagnostics.interaction_records = 0;
        self.diagnostics.semantic_records = 0;
        self.diagnostics.damage_rects = 0;
        self.diagnostics.output_retained_bytes = 0;
        self.diagnostics.model_retained_bytes = self.model.retained_bytes();
    }
}

impl<M: RetainedSurfaceModel> Drop for RetainedSurfaceHost<M> {
    fn drop(&mut self) {
        self.destroy();
    }
}

fn changes(previous: RetainedSurfaceInput, next: RetainedSurfaceInput) -> RetainedSurfaceChanges {
    let mut changed = RetainedSurfaceChanges::default();
    if previous.bounds != next.bounds {
        changed.insert(RetainedSurfaceChanges::BOUNDS);
    }
    if scale_key(previous.scale_factor) != scale_key(next.scale_factor) {
        changed.insert(RetainedSurfaceChanges::SCALE);
    }
    if previous.time != next.time {
        changed.insert(RetainedSurfaceChanges::TIME);
    }
    if previous.revisions.content != next.revisions.content {
        changed.insert(RetainedSurfaceChanges::CONTENT);
    }
    if previous.revisions.theme != next.revisions.theme {
        changed.insert(RetainedSurfaceChanges::THEME);
    }
    if previous.revisions.font != next.revisions.font {
        changed.insert(RetainedSurfaceChanges::FONT);
    }
    if previous.revisions.assets != next.revisions.assets {
        changed.insert(RetainedSurfaceChanges::ASSETS);
    }
    changed
}

fn scale_key(scale: f32) -> u32 {
    scale.to_bits()
}

fn validate_input(input: RetainedSurfaceInput) -> Result<(), String> {
    if !input.scale_factor.is_finite() || input.scale_factor <= 0.0 {
        return Err("retained surface scale factor must be positive and finite".into());
    }
    if !rect_is_finite(input.bounds)
        || input.bounds.size.width < 0.0
        || input.bounds.size.height < 0.0
    {
        return Err("retained surface bounds must be finite and non-negative".into());
    }
    Ok(())
}

fn validate_output(
    output: &RetainedSurfaceOutput,
    allocation: Rect,
    limits: RetainedSurfaceLimits,
    retained_bytes: usize,
) -> Result<(), String> {
    if retained_bytes > limits.max_retained_bytes {
        return Err("retained surface model exceeds its byte budget".into());
    }
    for (actual, limit, name) in [
        (
            output.paint.len(),
            limits.max_paint_commands,
            "paint commands",
        ),
        (
            output.interactions.len(),
            limits.max_interactions,
            "interaction records",
        ),
        (
            output.semantics.len(),
            limits.max_semantics,
            "semantic records",
        ),
        (
            output.damage.len(),
            limits.max_damage_rects,
            "damage rectangles",
        ),
    ] {
        if actual > limit {
            return Err(format!("retained surface exceeds its {name} budget"));
        }
    }
    if !output.measured.width.is_finite()
        || !output.measured.height.is_finite()
        || output.measured.width < 0.0
        || output.measured.height < 0.0
    {
        return Err("retained surface measurement must be finite and non-negative".into());
    }
    for command in &output.paint {
        if matches!(
            command,
            PaintCommand::BackdropBlur { .. }
                | PaintCommand::OverlayFill { .. }
                | PaintCommand::OverlayStroke { .. }
                | PaintCommand::PushClip(_)
                | PaintCommand::PopClip
        ) {
            return Err(
                "retained surfaces may not own overlays, clip stacks, or backdrop commands".into(),
            );
        }
        let Some(bounds) = paint_bounds(command) else {
            return Err("retained surface paint command has no bounded geometry".into());
        };
        if !rect_is_finite(bounds) || !rect_contains(allocation, bounds) {
            return Err("retained surface paint escaped its allocation".into());
        }
    }
    for bounds in output
        .interactions
        .iter()
        .map(|record| record.bounds)
        .chain(output.semantics.iter().map(|record| record.bounds))
        .chain(output.damage.iter().copied())
    {
        if !rect_is_finite(bounds) || !rect_contains(allocation, bounds) {
            return Err("retained surface result escaped its allocation".into());
        }
    }
    Ok(())
}

fn output_bytes(output: &RetainedSurfaceOutput) -> usize {
    output.paint.capacity() * std::mem::size_of::<PaintCommand>()
        + output.interactions.capacity() * std::mem::size_of::<RetainedSurfaceInteraction>()
        + output.semantics.capacity() * std::mem::size_of::<RetainedSurfaceSemantic>()
        + output.damage.capacity() * std::mem::size_of::<Rect>()
        + output
            .semantics
            .iter()
            .map(|record| {
                record.label.as_ref().map_or(0, String::capacity)
                    + record.state.as_ref().map_or(0, String::capacity)
            })
            .sum::<usize>()
}

fn paint_bounds(command: &PaintCommand) -> Option<Rect> {
    match command {
        PaintCommand::Fill { rect, .. }
        | PaintCommand::TopRoundedFill { rect, .. }
        | PaintCommand::RoundedFill { rect, .. }
        | PaintCommand::Gradient { rect, .. }
        | PaintCommand::RoundedStroke { rect, .. }
        | PaintCommand::Stroke { rect, .. }
        | PaintCommand::OverlayFill { rect, .. }
        | PaintCommand::OverlayStroke { rect, .. } => Some(*rect),
        PaintCommand::Text { bounds, .. }
        | PaintCommand::StyledText { bounds, .. }
        | PaintCommand::Image { bounds, .. } => Some(*bounds),
        PaintCommand::BackdropBlur { .. } | PaintCommand::PushClip(_) | PaintCommand::PopClip => {
            None
        }
    }
}

fn rect_is_finite(rect: Rect) -> bool {
    rect.origin.x.is_finite()
        && rect.origin.y.is_finite()
        && rect.size.width.is_finite()
        && rect.size.height.is_finite()
}

fn rect_contains(outer: Rect, inner: Rect) -> bool {
    inner.origin.x >= outer.origin.x
        && inner.origin.y >= outer.origin.y
        && inner.origin.x + inner.size.width <= outer.origin.x + outer.size.width
        && inner.origin.y + inner.size.height <= outer.origin.y + outer.size.height
}

fn contains(rect: Rect, point: Point) -> bool {
    point.x >= rect.origin.x
        && point.y >= rect.origin.y
        && point.x <= rect.origin.x + rect.size.width
        && point.y <= rect.origin.y + rect.size.height
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc, time::Duration};

    use super::*;
    use crate::{
        FrameRequest, SoftwareRenderer, UiFrame, UiStateStore,
        backend::{FrameRenderer, RenderFrame},
    };

    #[derive(Default)]
    struct Evidence {
        updates: usize,
        events: Vec<RetainedSurfaceLifecycle>,
    }

    struct Fixture {
        evidence: Rc<RefCell<Evidence>>,
        bytes: usize,
    }

    impl RetainedSurfaceModel for Fixture {
        fn update(
            &mut self,
            input: RetainedSurfaceInput,
            _changes: RetainedSurfaceChanges,
        ) -> Result<RetainedSurfaceOutput, String> {
            self.evidence.borrow_mut().updates += 1;
            let split = (input.bounds.size.width / 2.0).max(1.0);
            let changed = Rect::new(
                input.bounds.origin.x,
                input.bounds.origin.y,
                split,
                input.bounds.size.height,
            );
            Ok(RetainedSurfaceOutput {
                measured: input.bounds.size,
                paint: vec![
                    PaintCommand::Fill {
                        rect: changed,
                        color: if input.revisions.content.is_multiple_of(2) {
                            0xff11_2233
                        } else {
                            0xff66_4422
                        },
                    },
                    PaintCommand::Fill {
                        rect: Rect::new(
                            input.bounds.origin.x + split,
                            input.bounds.origin.y,
                            input.bounds.size.width - split,
                            input.bounds.size.height,
                        ),
                        color: 0xff22_8844,
                    },
                ],
                interactions: vec![RetainedSurfaceInteraction {
                    id: UiId::from("surface/action"),
                    bounds: changed,
                    action: ActionKind::Activate,
                    focusable: true,
                }],
                semantics: vec![RetainedSurfaceSemantic {
                    id: UiId::from("surface/action"),
                    bounds: changed,
                    role: SemanticRole::Button,
                    label: Some("Retained action".into()),
                    state: None,
                    protected: false,
                }],
                damage: vec![changed],
            })
        }

        fn lifecycle(&mut self, event: RetainedSurfaceLifecycle) {
            self.evidence.borrow_mut().events.push(event);
            if matches!(
                event,
                RetainedSurfaceLifecycle::Detached | RetainedSurfaceLifecycle::Destroyed
            ) {
                self.bytes = 0;
            }
        }

        fn retained_bytes(&self) -> usize {
            self.bytes
        }
    }

    fn input(revision: u64) -> RetainedSurfaceInput {
        RetainedSurfaceInput::new(
            Rect::new(0.0, 0.0, 40.0, 20.0),
            1.0,
            RetainedSurfaceRevisions {
                content: revision,
                ..RetainedSurfaceRevisions::default()
            },
        )
    }

    #[test]
    fn unchanged_updates_reuse_the_accepted_snapshot_and_admission_is_transactional() {
        let evidence = Rc::new(RefCell::new(Evidence::default()));
        let mut host = RetainedSurfaceHost::new(
            Fixture {
                evidence: evidence.clone(),
                bytes: 64,
            },
            RetainedSurfaceLimits::default(),
        )
        .unwrap();
        host.update(input(1)).unwrap();
        let accepted = host.snapshot().clone();
        host.update(input(1)).unwrap();
        assert_eq!(host.snapshot(), &accepted);
        assert_eq!(evidence.borrow().updates, 1);
        assert_eq!(host.diagnostics().unchanged_reuses, 1);

        host.limits.max_paint_commands = 1;
        assert!(host.update(input(2)).is_err());
        assert_eq!(host.snapshot(), &accepted);
        assert_eq!(host.diagnostics().rejected_updates, 1);
    }

    #[test]
    fn lifecycle_covers_attach_scale_loss_detach_reattach_and_destruction() {
        let evidence = Rc::new(RefCell::new(Evidence::default()));
        {
            let mut host = RetainedSurfaceHost::new(
                Fixture {
                    evidence: evidence.clone(),
                    bytes: 64,
                },
                RetainedSurfaceLimits::default(),
            )
            .unwrap();
            host.update(input(0)).unwrap();
            host.update(RetainedSurfaceInput {
                scale_factor: 2.0,
                ..input(0)
            })
            .unwrap();
            host.renderer_lost();
            assert!(host.snapshot().input.is_none());
            host.update(input(1).at_time(Duration::from_millis(16)))
                .unwrap();
            host.detach();
            assert_eq!(host.diagnostics().model_retained_bytes, 0);
            assert!(host.snapshot().try_element("detached", |_| ()).is_err());
            host.update(input(2)).unwrap();
        }
        assert_eq!(
            evidence.borrow().events,
            [
                RetainedSurfaceLifecycle::Attached,
                RetainedSurfaceLifecycle::ScaleChanged {
                    old: 1.0_f32.to_bits(),
                    new: 2.0_f32.to_bits(),
                },
                RetainedSurfaceLifecycle::RendererLost,
                RetainedSurfaceLifecycle::Detached,
                RetainedSurfaceLifecycle::Attached,
                RetainedSurfaceLifecycle::Detached,
                RetainedSurfaceLifecycle::Destroyed,
            ]
        );
    }

    #[test]
    fn incremental_damage_covers_every_pixel_changed_from_the_full_oracle() {
        let evidence = Rc::new(RefCell::new(Evidence::default()));
        let mut host = RetainedSurfaceHost::new(
            Fixture { evidence, bytes: 0 },
            RetainedSurfaceLimits::default(),
        )
        .unwrap();
        let old = host.update(input(0)).unwrap().clone();
        let next = host.update(input(1)).unwrap().clone();

        let mut old_renderer = SoftwareRenderer::new(40, 20, 1.0);
        old_renderer.render(&old.output.paint);
        let old_pixels = old_renderer.pixels().to_vec();
        let mut full_renderer = SoftwareRenderer::new(40, 20, 1.0);
        full_renderer.render(&next.output.paint);
        let full_pixels = full_renderer.pixels();
        assert_ne!(old_pixels, full_pixels);
        for (index, (before, after)) in old_pixels.iter().zip(full_pixels).enumerate() {
            if before == after {
                continue;
            }
            let point = Point {
                x: (index % 40) as f32 + 0.5,
                y: (index / 40) as f32 + 0.5,
            };
            assert!(next.output.damage.iter().any(|rect| contains(*rect, point)));
        }

        let mut incremental = SoftwareRenderer::new(40, 20, 1.0);
        incremental.render(&old.output.paint);
        incremental
            .render_frame_with_damage(
                RenderFrame {
                    commands: &next.output.paint,
                    logical_size: (40, 20),
                    scale_factor: 1.0,
                    generation: 2,
                },
                Some(&next.output.damage),
            )
            .unwrap();
        assert_eq!(incremental.pixels(), full_pixels);
        assert_eq!(
            next.interaction_at(Point { x: 2.0, y: 2.0 })
                .map(|record| &record.id),
            Some(&UiId::from("surface/action"))
        );
    }

    #[test]
    fn output_cannot_escape_allocation_or_resource_budgets() {
        let evidence = Rc::new(RefCell::new(Evidence::default()));
        let mut host = RetainedSurfaceHost::new(
            Fixture { evidence, bytes: 0 },
            RetainedSurfaceLimits {
                max_damage_rects: 0,
                ..RetainedSurfaceLimits::default()
            },
        )
        .unwrap();
        assert!(host.update(input(0)).is_err());
        assert!(host.snapshot().input.is_none());
    }

    #[test]
    fn accepted_surface_bridges_into_native_paint_hit_focus_and_accessibility_authority() {
        let evidence = Rc::new(RefCell::new(Evidence::default()));
        let mut host = RetainedSurfaceHost::new(
            Fixture { evidence, bytes: 0 },
            RetainedSurfaceLimits::default(),
        )
        .unwrap();
        let accepted = host
            .update(RetainedSurfaceInput::new(
                Rect::new(40.0, 30.0, 40.0, 20.0),
                1.0,
                RetainedSurfaceRevisions {
                    content: 7,
                    ..RetainedSurfaceRevisions::default()
                },
            ))
            .unwrap()
            .clone();
        let element = accepted
            .try_element("retained", |_| 17_u8)
            .expect("supported retained records bridge to native nodes");
        let mut state = UiStateStore::default();
        let frame = UiFrame::resolve(
            element,
            FrameRequest::new(Rect::new(0.0, 0.0, 40.0, 20.0), &mut state),
        );

        assert_eq!(frame.message_at(Point { x: 5.0, y: 5.0 }), Some(&17));
        assert!(frame.semantic_nodes().iter().any(|node| {
            node.name.as_deref() == Some("Retained action")
                && node.role == Some(SemanticRole::Button)
                && node.actions.contains(&ActionKind::Activate)
        }));
        assert!(frame.commands().iter().any(|command| {
            matches!(command, PaintCommand::Fill { rect, .. } if rect.origin == Point::default())
        }));
    }

    #[test]
    fn bridge_matches_cold_resolution_and_rejects_unrepresentable_authority() {
        let evidence = Rc::new(RefCell::new(Evidence::default()));
        let mut host = RetainedSurfaceHost::new(
            Fixture { evidence, bytes: 0 },
            RetainedSurfaceLimits::default(),
        )
        .unwrap();
        let bounds = Rect::new(0.0, 0.0, 40.0, 20.0);
        let first = host.update(input(0)).unwrap().clone();
        let mut incremental_state = UiStateStore::default();
        let first_frame = UiFrame::resolve(
            first.try_element("retained", |_| 1_u8).unwrap(),
            FrameRequest::new(bounds, &mut incremental_state),
        );
        let next = host.update(input(1)).unwrap().clone();
        let incremental = UiFrame::resolve_against(
            next.try_element("retained", |_| 1_u8).unwrap(),
            FrameRequest::new(bounds, &mut incremental_state),
            &first_frame,
        );
        let mut cold_state = UiStateStore::default();
        let cold = UiFrame::resolve(
            next.try_element("retained", |_| 1_u8).unwrap(),
            FrameRequest::new(bounds, &mut cold_state),
        );
        assert_eq!(incremental.resolved_layout(), cold.resolved_layout());
        assert_eq!(incremental.commands(), cold.commands());
        assert_eq!(incremental.semantic_nodes(), cold.semantic_nodes());
        assert_eq!(
            incremental.accessibility_nodes(),
            cold.accessibility_nodes()
        );

        let mut protected = next.clone();
        protected.output.semantics[0].protected = true;
        assert!(protected.try_element("retained", |_| 1_u8).is_err());
        let mut pointer_only = next;
        pointer_only.output.interactions[0].focusable = false;
        assert!(pointer_only.try_element("retained", |_| 1_u8).is_err());
    }
}
