use crate::{Color, Insets, Point, ReadingDirection, Rect, SemanticTheme, Size, TextAlign, UiId};

/// Semantic purpose of a transient surface. This is intentionally closed so a
/// host can apply focus, dismissal, and accessibility policy consistently.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransientKind {
    Popover,
    Dialog,
    Tooltip,
    ContextMenu,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransientTone {
    Ordinary,
    Status,
    Destructive,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DismissPolicy {
    pub cancel: bool,
    pub outside_pointer: bool,
    pub action: bool,
}

impl Default for DismissPolicy {
    fn default() -> Self {
        Self {
            cancel: true,
            outside_pointer: true,
            action: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OverlayStyle {
    pub background: Color,
    pub foreground: Color,
    pub border: Color,
    pub selected: Color,
    pub radius: u16,
}

/// Host-consumable declaration shared by non-menu transient surfaces.
#[derive(Clone, Debug, PartialEq)]
pub struct TransientSurface {
    pub id: OverlayId,
    pub kind: TransientKind,
    pub anchor: OverlayAnchor,
    pub placement: OverlayPlacement,
    pub collision: CollisionPolicy,
    pub focus: OverlayFocusPolicy,
    pub dismiss: DismissPolicy,
    pub focus_return: UiId,
    pub logical_size: Size,
    pub style: OverlayStyle,
    pub accessible_name: Option<String>,
    pub direction: ReadingDirection,
    pub scale: f32,
}

impl TransientSurface {
    pub fn popover(
        id: impl Into<UiId>,
        anchor: OverlayAnchor,
        size: Size,
        style: OverlayStyle,
    ) -> Self {
        Self::new(id, TransientKind::Popover, anchor, size, style)
    }
    pub fn dialog(
        id: impl Into<UiId>,
        anchor: OverlayAnchor,
        size: Size,
        style: OverlayStyle,
    ) -> Self {
        Self::new(id, TransientKind::Dialog, anchor, size, style)
    }
    pub fn tooltip(
        id: impl Into<UiId>,
        anchor: OverlayAnchor,
        size: Size,
        style: OverlayStyle,
    ) -> Self {
        let mut surface = Self::new(id, TransientKind::Tooltip, anchor, size, style);
        surface.focus = OverlayFocusPolicy::Preserve;
        surface
    }
    fn new(
        id: impl Into<UiId>,
        kind: TransientKind,
        anchor: OverlayAnchor,
        logical_size: Size,
        style: OverlayStyle,
    ) -> Self {
        let focus_return = anchor.id().clone();
        Self {
            id: OverlayId::new(id),
            kind,
            anchor,
            placement: OverlayPlacement::Below,
            collision: CollisionPolicy::FlipThenClamp,
            focus: OverlayFocusPolicy::FirstItem,
            dismiss: DismissPolicy::default(),
            focus_return,
            logical_size,
            style,
            accessible_name: None,
            direction: ReadingDirection::LeftToRight,
            scale: 1.0,
        }
    }

    pub fn accessible_name(mut self, name: impl Into<String>) -> Self {
        self.accessible_name = Some(name.into());
        self
    }

    pub fn placement(mut self, placement: OverlayPlacement) -> Self {
        self.placement = placement;
        self
    }

    pub fn collision(mut self, collision: CollisionPolicy) -> Self {
        self.collision = collision;
        self
    }

    pub fn focus(mut self, focus: OverlayFocusPolicy) -> Self {
        self.focus = focus;
        self
    }

    pub fn dismiss(mut self, dismiss: DismissPolicy) -> Self {
        self.dismiss = dismiss;
        self
    }

    pub fn focus_return(mut self, target: impl Into<UiId>) -> Self {
        self.focus_return = target.into();
        self
    }

    pub fn direction(mut self, direction: ReadingDirection) -> Self {
        self.direction = direction;
        self
    }

    pub fn scale(mut self, scale: f32) -> Self {
        self.scale = scale.max(f32::EPSILON);
        self
    }
}

impl OverlayStyle {
    pub fn from_theme(theme: &SemanticTheme) -> Self {
        Self {
            background: theme.surfaces.raised,
            foreground: theme.text.primary,
            border: theme.borders.ordinary,
            selected: theme.surfaces.selected,
            radius: theme.radii.overlay.max(0.0).round() as u16,
        }
    }
}

/// Stable identity for an overlay across frame rebuilds.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct OverlayId(UiId);

impl OverlayId {
    pub fn new(id: impl Into<UiId>) -> Self {
        Self(id.into())
    }
    pub fn as_ui_id(&self) -> &UiId {
        &self.0
    }
    pub(crate) fn item_id(&self, item: &UiId) -> UiId {
        self.0.scoped(item.as_str())
    }
}

/// The node whose geometry and focus ownership invoke an overlay.
#[derive(Clone, Debug, PartialEq)]
pub enum OverlayAnchor {
    Node(UiId),
    InvocationTarget(UiId),
    InvocationTargetCenter(UiId),
    /// A pointer-positioned surface that still belongs to a semantic invocation
    /// target for controller, accessibility, dismissal, and focus return.
    Point {
        invocation_target: UiId,
        point: Point,
    },
}

impl OverlayAnchor {
    pub fn id(&self) -> &UiId {
        match self {
            Self::Node(id) | Self::InvocationTarget(id) | Self::InvocationTargetCenter(id) => id,
            Self::Point {
                invocation_target, ..
            } => invocation_target,
        }
    }

    pub(crate) fn rect(&self, node: Rect) -> Rect {
        match self {
            Self::Node(_) | Self::InvocationTarget(_) => node,
            Self::InvocationTargetCenter(_) => Rect::new(
                node.origin.x + node.size.width / 2.0,
                node.origin.y + node.size.height / 2.0,
                0.0,
                0.0,
            ),
            Self::Point { point, .. } => Rect::new(point.x, point.y, 0.0, 0.0),
        }
    }

    pub(crate) fn with_resolved_target(&self, target: UiId) -> Self {
        match self {
            Self::Node(_) => Self::Node(target),
            Self::InvocationTarget(_) => Self::InvocationTarget(target),
            Self::InvocationTargetCenter(_) => Self::InvocationTargetCenter(target),
            Self::Point { point, .. } => Self::Point {
                invocation_target: target,
                point: *point,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum OverlayPlacement {
    Above,
    #[default]
    Below,
    Before,
    After,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CollisionPolicy {
    None,
    #[default]
    FlipThenClamp,
    Clamp,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum OverlayFocusPolicy {
    Preserve,
    #[default]
    FirstItem,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DismissReason {
    Action,
    Cancel,
    OutsidePointer,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FocusReturn {
    pub overlay: OverlayId,
    pub target: Option<UiId>,
    pub reason: DismissReason,
}

#[derive(Clone, Debug)]
pub struct OverlayMenuItem<Message> {
    pub id: UiId,
    pub label: String,
    pub action: Option<Message>,
    pub disabled_reason: Option<String>,
    pub shortcut: Option<String>,
    pub accessible_name: Option<String>,
    pub tone: TransientTone,
    pub separator_before: bool,
}

impl<Message> OverlayMenuItem<Message> {
    pub fn action(id: impl Into<UiId>, label: impl Into<String>, action: Message) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            action: Some(action),
            disabled_reason: None,
            shortcut: None,
            accessible_name: None,
            tone: TransientTone::Ordinary,
            separator_before: false,
        }
    }
    pub fn disabled(id: impl Into<UiId>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            action: None,
            disabled_reason: None,
            shortcut: None,
            accessible_name: None,
            tone: TransientTone::Ordinary,
            separator_before: false,
        }
    }

    pub fn disabled_with_reason(
        id: impl Into<UiId>,
        label: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self::disabled(id, label).reason(reason)
    }

    pub fn reason(mut self, reason: impl Into<String>) -> Self {
        self.disabled_reason = Some(reason.into());
        self
    }

    pub fn shortcut(mut self, shortcut: impl Into<String>) -> Self {
        self.shortcut = Some(shortcut.into());
        self
    }

    pub fn accessible_name(mut self, name: impl Into<String>) -> Self {
        self.accessible_name = Some(name.into());
        self
    }

    pub fn tone(mut self, tone: TransientTone) -> Self {
        self.tone = tone;
        self
    }

    pub fn separator_before(mut self, separated: bool) -> Self {
        self.separator_before = separated;
        self
    }
}

#[derive(Clone, Debug)]
pub struct OverlayMenu<Message> {
    pub kind: TransientKind,
    pub id: OverlayId,
    pub anchor: OverlayAnchor,
    pub placement: OverlayPlacement,
    pub collision: CollisionPolicy,
    pub focus: OverlayFocusPolicy,
    pub width: f32,
    pub row_height: f32,
    pub row_gap: f32,
    pub padding: Insets,
    pub radius: f32,
    pub items: Vec<OverlayMenuItem<Message>>,
    pub background: Color,
    pub foreground: Color,
    pub text_scale: f32,
    pub text_align: TextAlign,
    pub item_hover: Option<Color>,
    pub item_pressed: Option<Color>,
    pub item_selected: Option<Color>,
    pub item_radius: f32,
    pub initial_controller_item: Option<UiId>,
    pub direction: ReadingDirection,
}

impl<Message> OverlayMenu<Message> {
    pub fn new(id: impl Into<UiId>, anchor: OverlayAnchor) -> Self {
        Self {
            id: OverlayId::new(id),
            kind: TransientKind::ContextMenu,
            anchor,
            placement: OverlayPlacement::Below,
            collision: CollisionPolicy::FlipThenClamp,
            focus: OverlayFocusPolicy::FirstItem,
            width: 200.0,
            row_height: 34.0,
            row_gap: 0.0,
            padding: Insets::all(0.0),
            radius: 0.0,
            items: Vec::new(),
            background: 0x202630,
            foreground: 0xe8edf4,
            text_scale: 2.0,
            text_align: TextAlign::Start,
            item_hover: None,
            item_pressed: None,
            item_selected: None,
            item_radius: 0.0,
            initial_controller_item: None,
            direction: ReadingDirection::LeftToRight,
        }
    }

    /// Selects a stable item key when this menu is presented as an already-open surface.
    pub fn initial_controller_item(mut self, item: impl Into<UiId>) -> Self {
        self.initial_controller_item = Some(item.into());
        self
    }
    pub fn item(mut self, item: OverlayMenuItem<Message>) -> Self {
        self.items.push(item);
        self
    }

    pub fn kind(mut self, kind: TransientKind) -> Self {
        self.kind = kind;
        self
    }

    pub fn semantic_style(mut self, style: OverlayStyle) -> Self {
        self.background = style.background;
        self.foreground = style.foreground;
        self.item_selected = Some(style.selected);
        self.radius = f32::from(style.radius);
        self
    }

    /// Resolves logical before/after placement using the menu's reading direction.
    pub fn direction(mut self, direction: ReadingDirection) -> Self {
        self.direction = direction;
        self
    }
}

/// Resolves logical before/after placement, scale, and work-area collision in
/// one deterministic operation shared by popovers, dialogs, tooltips, and menus.
pub fn place_transient(
    anchor: Rect,
    logical_size: Size,
    work_area: Rect,
    placement: OverlayPlacement,
    collision: CollisionPolicy,
    direction: ReadingDirection,
    scale: f32,
) -> Rect {
    let placement = match (direction, placement) {
        (ReadingDirection::RightToLeft, OverlayPlacement::Before) => OverlayPlacement::After,
        (ReadingDirection::RightToLeft, OverlayPlacement::After) => OverlayPlacement::Before,
        _ => placement,
    };
    let scale = scale.max(f32::EPSILON);
    place(
        anchor,
        Size::new(logical_size.width * scale, logical_size.height * scale),
        work_area,
        placement,
        collision,
    )
}

pub(crate) fn place(
    anchor: Rect,
    desired: Size,
    viewport: Rect,
    placement: OverlayPlacement,
    collision: CollisionPolicy,
) -> Rect {
    let desired = if collision == CollisionPolicy::None {
        desired
    } else {
        Size::new(
            desired.width.min(viewport.size.width.max(0.0)),
            desired.height.min(viewport.size.height.max(0.0)),
        )
    };
    let candidate = |side| match side {
        OverlayPlacement::Below => Rect::new(
            anchor.origin.x,
            anchor.origin.y + anchor.size.height,
            desired.width,
            desired.height,
        ),
        OverlayPlacement::Above => Rect::new(
            anchor.origin.x,
            anchor.origin.y - desired.height,
            desired.width,
            desired.height,
        ),
        OverlayPlacement::After => Rect::new(
            anchor.origin.x + anchor.size.width,
            anchor.origin.y,
            desired.width,
            desired.height,
        ),
        OverlayPlacement::Before => Rect::new(
            anchor.origin.x - desired.width,
            anchor.origin.y,
            desired.width,
            desired.height,
        ),
    };
    let fits = |rect: Rect| {
        rect.origin.x >= viewport.origin.x
            && rect.origin.y >= viewport.origin.y
            && rect.origin.x + rect.size.width <= viewport.origin.x + viewport.size.width
            && rect.origin.y + rect.size.height <= viewport.origin.y + viewport.size.height
    };
    let mut rect = candidate(placement);
    if collision == CollisionPolicy::FlipThenClamp && !fits(rect) {
        rect = candidate(match placement {
            OverlayPlacement::Below => OverlayPlacement::Above,
            OverlayPlacement::Above => OverlayPlacement::Below,
            OverlayPlacement::Before => OverlayPlacement::After,
            OverlayPlacement::After => OverlayPlacement::Before,
        });
    }
    if collision != CollisionPolicy::None {
        rect.origin.x = rect.origin.x.clamp(
            viewport.origin.x,
            (viewport.origin.x + viewport.size.width - rect.size.width).max(viewport.origin.x),
        );
        rect.origin.y = rect.origin.y.clamp(
            viewport.origin.y,
            (viewport.origin.y + viewport.size.height - rect.size.height).max(viewport.origin.y),
        );
    }
    rect
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ActionKind, Button, Point, SemanticRole, SemanticSelector, UiEvent, UiFrame, UiStateStore,
    };

    #[derive(Clone, Debug, Eq, PartialEq)]
    enum Message {
        Anchor,
        Choose,
    }

    fn frame(state: &mut UiStateStore) -> UiFrame<Message> {
        UiFrame::layout_with_state(
            Button::new(Message::Anchor, "Anchor").id("anchor"),
            Rect::new(0.0, 0.0, 240.0, 120.0),
            state,
        )
    }

    fn anchor_id(frame: &UiFrame<Message>) -> UiId {
        frame
            .resolved_layout()
            .nodes()
            .iter()
            .find(|node| node.semantic_role == Some(crate::SemanticRole::Button))
            .unwrap()
            .id
            .clone()
    }

    fn menu(anchor: UiId) -> OverlayMenu<Message> {
        OverlayMenu::new("context", OverlayAnchor::InvocationTarget(anchor))
            .item(OverlayMenuItem::action("choose", "Choose", Message::Choose))
    }

    #[test]
    fn context_channels_open_the_same_registered_overlay() {
        for channel in 0..4 {
            let mut state = UiStateStore::default();
            let mut frame = frame(&mut state);
            let anchor = anchor_id(&frame);
            frame
                .present_menu(&mut state, menu(anchor.clone()))
                .unwrap();
            state.set_focus(Some(anchor.clone()));
            state
                .navigation_mut()
                .set_controller_selected(Some(anchor.clone()));
            let bounds = frame.resolved_layout().find(&anchor).unwrap().allocated;
            let point = Point {
                x: bounds.origin.x + 1.0,
                y: bounds.origin.y + 1.0,
            };
            let event = match channel {
                0 => UiEvent::PointerContext(point),
                1 => UiEvent::KeyboardContextMenu,
                2 => UiEvent::ControllerContextMenu,
                _ => UiEvent::AccessibilityContextMenu(anchor),
            };
            assert_eq!(
                frame.handle_event(&mut state, event).invalidation,
                crate::Invalidation::Layout
            );
            assert_eq!(state.open_overlay_id(), Some(&OverlayId::new("context")));
        }
    }

    #[test]
    fn direct_semantic_context_action_uses_the_registered_overlay_transition() {
        let mut state = UiStateStore::default();
        let mut frame = frame(&mut state);
        let anchor = anchor_id(&frame);
        frame
            .present_menu(&mut state, menu(anchor.clone()))
            .unwrap();

        let outcome = frame
            .transition(
                &mut state,
                crate::InputSource::Programmatic,
                crate::InteractionIntent::Invoke {
                    target: anchor,
                    action: crate::SemanticAction::Invoke(crate::ActionKind::ContextMenu),
                },
            )
            .unwrap();

        assert_eq!(outcome.invalidation, crate::Invalidation::Layout);
        assert_eq!(state.open_overlay_id(), Some(&OverlayId::new("context")));
    }

    #[test]
    fn always_open_menu_surface_uses_the_same_transition_and_stable_item_focus() {
        let mut state = UiStateStore::default();
        let mut frame = frame(&mut state);
        let anchor = anchor_id(&frame);
        let menu = menu(anchor).initial_controller_item("choose");

        let outcome = frame.present_open_menu(&mut state, menu).unwrap();

        assert_eq!(outcome.invalidation, crate::Invalidation::Layout);
        assert_eq!(state.open_overlay_id(), Some(&OverlayId::new("context")));
        assert_eq!(
            state.navigation().controller_selected(),
            Some(&OverlayId::new("context").item_id(&UiId::from("choose")))
        );
        assert_eq!(
            frame
                .query(&SemanticSelector::Role(SemanticRole::MenuItem))
                .len(),
            1
        );
    }

    #[test]
    fn open_overlay_traps_controller_navigation_on_menu_items() {
        let mut state = UiStateStore::default();
        let mut closed = frame(&mut state);
        let anchor = anchor_id(&closed);
        state
            .navigation_mut()
            .set_controller_selected(Some(anchor.clone()));
        closed
            .present_menu(&mut state, menu(anchor.clone()))
            .unwrap();
        closed.handle_event(&mut state, UiEvent::ControllerContextMenu);

        let mut open = frame(&mut state);
        open.present_menu(&mut state, menu(anchor)).unwrap();
        let item = OverlayId::new("context").item_id(&UiId::from("choose"));
        assert_eq!(state.navigation().controller_selected(), Some(&item));

        let outcome = open.handle_event(&mut state, UiEvent::ControllerActivate);
        assert_eq!(outcome.messages, vec![Message::Choose]);
        assert!(state.open_overlay_id().is_none());
    }

    #[test]
    fn open_overlay_menu_and_items_share_semantic_and_accessibility_authority() {
        let mut state = UiStateStore::default();
        let closed = frame(&mut state);
        let anchor = anchor_id(&closed);
        state.open_overlay(OverlayId::new("context"), anchor.clone());

        let mut open = frame(&mut state);
        open.present_menu(&mut state, menu(anchor.clone())).unwrap();
        let menu_id = OverlayId::new("context").as_ui_id().clone();
        let item_id = OverlayId::new("context").item_id(&UiId::from("choose"));
        let menus = open.query(&SemanticSelector::Role(SemanticRole::Menu));
        assert!(menus.iter().any(|node| node.id == menu_id));
        let item = open
            .query_unique(&SemanticSelector::Id(item_id.clone()))
            .expect("open menu item must be semantically queryable");
        assert!(
            open.semantic_nodes().iter().all(|node| node.id != anchor),
            "an open menu makes background semantics inert"
        );
        assert!(
            open.accessibility_nodes()
                .iter()
                .all(|node| node.id != anchor),
            "an open menu makes the background accessibility tree inert"
        );
        assert_eq!(item.parent, Some(menu_id));
        assert_eq!(item.role, Some(SemanticRole::MenuItem));
        assert_eq!(item.actions, vec![ActionKind::Activate]);

        let accessible = open
            .accessibility_nodes()
            .iter()
            .find(|node| node.id == item_id)
            .expect("semantic menu item must have accessibility parity");
        assert_eq!(accessible.label.as_deref(), item.name.as_deref());
        assert_eq!(
            accessible.role.as_deref(),
            Some(SemanticRole::MenuItem.as_str())
        );
        assert_eq!(accessible.interactive, item.enabled);
    }

    #[test]
    fn overlay_action_dismissal_returns_focus() {
        let mut state = UiStateStore::default();
        let mut closed = frame(&mut state);
        let anchor = anchor_id(&closed);
        closed
            .present_menu(&mut state, menu(anchor.clone()))
            .unwrap();
        state.set_focus(Some(anchor.clone()));
        closed.handle_event(&mut state, UiEvent::KeyboardContextMenu);

        let mut open = frame(&mut state);
        open.present_menu(&mut state, menu(anchor.clone())).unwrap();
        let item = OverlayId::new("context").item_id(&UiId::from("choose"));
        let rect = open
            .accessibility_nodes()
            .iter()
            .find(|node| node.id == item)
            .unwrap()
            .rect;
        let point = Point {
            x: rect.origin.x + 2.0,
            y: rect.origin.y + 2.0,
        };
        open.handle_event(&mut state, UiEvent::PointerPressed(point));
        let outcome = open.handle_event(&mut state, UiEvent::PointerReleased(point));
        assert_eq!(outcome.messages, vec![Message::Choose]);
        assert_eq!(state.focused(), Some(&anchor));
        assert_eq!(
            state.take_focus_return().unwrap().reason,
            DismissReason::Action
        );
    }

    #[test]
    fn escape_dismisses_and_collision_flips_inside_viewport() {
        let placed = place(
            Rect::new(80.0, 90.0, 20.0, 10.0),
            Size {
                width: 60.0,
                height: 40.0,
            },
            Rect::new(0.0, 0.0, 100.0, 100.0),
            OverlayPlacement::Below,
            CollisionPolicy::FlipThenClamp,
        );
        assert_eq!(placed, Rect::new(40.0, 50.0, 60.0, 40.0));

        let mut state = UiStateStore::default();
        let mut frame = frame(&mut state);
        let anchor = anchor_id(&frame);
        state.set_focus(Some(anchor.clone()));
        state.open_overlay(OverlayId::new("context"), anchor.clone());
        frame
            .present_menu(&mut state, menu(anchor.clone()))
            .unwrap();
        frame.handle_event(&mut state, UiEvent::Dismiss);
        assert_eq!(state.focused(), Some(&anchor));
        assert_eq!(
            state.take_focus_return().unwrap().reason,
            DismissReason::Cancel
        );
    }

    #[test]
    fn logical_placement_honors_rtl_scale_and_work_area_edges() {
        let anchor = Rect::new(80.0, 10.0, 10.0, 10.0);
        let area = Rect::new(5.0, 5.0, 95.0, 80.0);
        let rtl = place_transient(
            anchor,
            Size::new(20.0, 10.0),
            area,
            OverlayPlacement::Before,
            CollisionPolicy::FlipThenClamp,
            ReadingDirection::RightToLeft,
            2.0,
        );
        assert_eq!(rtl.size, Size::new(40.0, 20.0));
        assert_eq!(rtl.origin.x, 40.0, "after must flip before overflowing");
        assert!(rtl.origin.x >= area.origin.x && rtl.origin.y >= area.origin.y);
        assert!(rtl.origin.x + rtl.size.width <= area.origin.x + area.size.width);
    }

    #[test]
    fn menu_reconstruction_preserves_item_focus_and_removed_origin_closes_recoverably() {
        let mut state = UiStateStore::default();
        let mut closed = frame(&mut state);
        let anchor = anchor_id(&closed);
        state
            .navigation_mut()
            .set_controller_selected(Some(anchor.clone()));
        closed
            .present_menu(&mut state, menu(anchor.clone()))
            .unwrap();
        closed.handle_event(&mut state, UiEvent::ControllerContextMenu);

        let mut rebuilt = frame(&mut state);
        rebuilt
            .present_menu(&mut state, menu(anchor.clone()))
            .unwrap();
        let selected = OverlayId::new("context").item_id(&UiId::from("choose"));
        assert_eq!(state.navigation().controller_selected(), Some(&selected));

        let mut without_origin = UiFrame::layout_with_state(
            crate::Text::<Message>::new("Origin removed"),
            Rect::new(0.0, 0.0, 240.0, 120.0),
            &mut state,
        );
        assert_eq!(
            without_origin.present_menu(&mut state, menu(anchor)),
            Err(crate::SemanticActionError::MissingTarget)
        );
        assert!(state.open_overlay_id().is_none());
        let returned = state.take_focus_return().expect("recoverable focus return");
        assert_eq!(returned.reason, DismissReason::Cancel);
    }

    #[test]
    fn menu_render_matrix_is_bounded_directional_themed_and_deterministic() {
        let palettes = [
            crate::SemanticTokenSet::standard(
                0x090b10, 0x121722, 0x1b2230, 0x263149, 0x31405e, 0xf7f9ff, 0xa8b0c0, 0x7c5cff,
                0x352968, 0x52d6a4, 0xe05252,
            ),
            crate::SemanticTokenSet::standard(
                0xf8f8fa, 0xffffff, 0xf0f1f4, 0xe1e4ea, 0xd1d6df, 0x111318, 0x555b66, 0x006ad4,
                0xcce5ff, 0x006b4f, 0xb00020,
            ),
            crate::SemanticTokenSet::standard(
                0x000000, 0x000000, 0x000000, 0xffffff, 0xffffff, 0xffffff, 0xffffff, 0x00ffff,
                0x003333, 0xffff00, 0xff00ff,
            ),
        ];
        for (width, direction) in [
            (176.0, ReadingDirection::LeftToRight),
            (320.0, ReadingDirection::RightToLeft),
        ] {
            for tokens in palettes {
                for scale in [1.0, 2.0] {
                    let render = || {
                        let mut state = UiStateStore::default();
                        let mut frame = UiFrame::layout_with_state(
                            Button::new(Message::Anchor, "Anchor at output edge")
                                .id("anchor")
                                .width(width),
                            Rect::new(0.0, 0.0, width, 112.0),
                            &mut state,
                        );
                        let anchor = anchor_id(&frame);
                        state.open_overlay(OverlayId::new("matrix-menu"), anchor.clone());
                        let theme = SemanticTheme::from_tokens(tokens);
                        let mut menu = OverlayMenu::new(
                            "matrix-menu",
                            OverlayAnchor::InvocationTarget(anchor),
                        )
                        .direction(direction)
                        .semantic_style(OverlayStyle::from_theme(&theme))
                        .item(OverlayMenuItem::action(
                            "localized",
                            "Eine sehr lange lokalisierte Anwendungsaktion",
                            Message::Choose,
                        ));
                        menu.row_height *= scale;
                        menu.text_scale *= scale;
                        frame.present_menu(&mut state, menu).unwrap();
                        (
                            frame.commands().to_vec(),
                            frame.accessibility_nodes().to_vec(),
                        )
                    };
                    let first = render();
                    let second = render();
                    assert_eq!(first.0, second.0);
                    assert_eq!(first.1, second.1);
                    for node in &first.1 {
                        assert!(node.rect.origin.x >= 0.0);
                        assert!(node.rect.origin.y >= 0.0);
                        assert!(node.rect.origin.x + node.rect.size.width <= width);
                        assert!(node.rect.origin.y + node.rect.size.height <= 112.0);
                    }
                }
            }
        }
    }

    #[test]
    fn transient_kind_and_semantic_style_are_typed_declarations() {
        let theme = crate::SemanticTheme::from_tokens(crate::SemanticTokenSet::standard(
            1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11,
        ));
        let style = OverlayStyle::from_theme(&theme);
        let menu = OverlayMenu::<Message>::new("tip", OverlayAnchor::Node("anchor".into()))
            .kind(TransientKind::Tooltip)
            .semantic_style(style);
        assert_eq!(menu.kind, TransientKind::Tooltip);
        assert_eq!(menu.background, theme.surfaces.raised);
        assert_eq!(menu.item_selected, Some(theme.surfaces.selected));
        let tooltip = TransientSurface::tooltip(
            "help",
            OverlayAnchor::Node("anchor".into()),
            Size::new(80.0, 24.0),
            style,
        );
        assert_eq!(tooltip.kind, TransientKind::Tooltip);
        assert_eq!(tooltip.focus, OverlayFocusPolicy::Preserve);
        assert_eq!(
            TransientSurface::dialog(
                "dialog",
                OverlayAnchor::Node("anchor".into()),
                Size::new(300.0, 200.0),
                style,
            )
            .kind,
            TransientKind::Dialog
        );
        let destructive = OverlayMenuItem::<Message>::disabled_with_reason(
            "delete",
            "Delete",
            "Administrator policy",
        )
        .shortcut("Shift+Delete")
        .accessible_name("Permanently delete")
        .tone(TransientTone::Destructive)
        .separator_before(true);
        assert_eq!(
            destructive.disabled_reason.as_deref(),
            Some("Administrator policy")
        );
        assert_eq!(destructive.tone, TransientTone::Destructive);
        assert!(destructive.separator_before);
    }

    #[test]
    fn removed_anchor_is_a_deterministic_declaration_error() {
        let mut state = UiStateStore::default();
        let mut frame = frame(&mut state);
        let result = frame.present_menu(&mut state, menu(UiId::from("anchor-that-was-removed")));
        assert!(result.is_err());
        assert!(state.open_overlay_id().is_none());
    }
}
