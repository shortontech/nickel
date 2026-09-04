use std::{collections::HashMap, sync::Arc, time::Duration};

use crate::{DismissReason, FocusReturn, OverlayAnchor, OverlayId};
use crate::{DocumentSelection, SelectionDocument, TextEditor};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum InputModality {
    #[default]
    Keyboard,
    Pointer,
    Controller,
    Accessibility,
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct UiId(String);

impl UiId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn scoped(&self, child: impl AsRef<str>) -> Self {
        Self(format!("{}/{}", self.0, child.as_ref()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for UiId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for UiId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

macro_rules! integer_ids {
    ($($integer:ty),* $(,)?) => {
        $(impl From<$integer> for UiId {
            fn from(value: $integer) -> Self {
                Self::new(value.to_string())
            }
        })*
    };
}

integer_ids!(u8, u16, u32, u64, usize, i8, i16, i32, i64, isize);

#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub enum Invalidation {
    #[default]
    None,
    Paint,
    Layout,
    Scheduled(Duration),
}

impl Invalidation {
    pub fn merge(self, other: Self) -> Self {
        match (self, other) {
            (Self::Layout, _) | (_, Self::Layout) => Self::Layout,
            (Self::Paint, _) | (_, Self::Paint) => Self::Paint,
            (Self::Scheduled(left), Self::Scheduled(right)) => Self::Scheduled(left.min(right)),
            (Self::Scheduled(delay), Self::None) | (Self::None, Self::Scheduled(delay)) => {
                Self::Scheduled(delay)
            }
            (Self::None, Self::None) => Self::None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TransientState {
    pub scroll_offset_x: f32,
    pub scroll_offset: f32,
    pub scroll_velocity: f32,
    /// Whether content growth should keep this scroll region pinned to its end.
    pub scroll_at_end: bool,
    pub dropdown_open: bool,
    pub dropdown_open_generation: u64,
    pub animation_progress: f32,
    pub editor: Option<TextEditor>,
}

impl Default for TransientState {
    fn default() -> Self {
        Self {
            scroll_offset_x: 0.0,
            scroll_offset: 0.0,
            scroll_velocity: 0.0,
            scroll_at_end: true,
            dropdown_open: false,
            dropdown_open_generation: 0,
            animation_progress: 0.0,
            editor: None,
        }
    }
}

#[derive(Clone, Debug)]
struct StateEntry {
    state: TransientState,
    last_seen_frame: u64,
}

#[derive(Clone, Debug)]
struct DurableNodeState {
    entries: HashMap<UiId, StateEntry>,
    frame: u64,
    retention_frames: u64,
}

#[derive(Clone, Debug)]
struct PointerModalityState {
    hovered: Option<UiId>,
    pressed: Option<UiId>,
    captured: Option<UiId>,
    scrollbar_grab_offset: Option<f32>,
    input_modality: InputModality,
    window_focused: bool,
}

#[derive(Clone, Debug, Default)]
pub struct NavigationState {
    focused: Option<UiId>,
    controller_selected: Option<UiId>,
    controller_pane: Option<UiId>,
    controller_scope: Option<UiId>,
    controller_editing: bool,
    controller_retained_focus: HashMap<UiId, UiId>,
}

impl NavigationState {
    pub fn focused(&self) -> Option<&UiId> {
        self.focused.as_ref()
    }
    pub fn controller_selected(&self) -> Option<&UiId> {
        self.controller_selected.as_ref()
    }
    pub fn controller_pane(&self) -> Option<&UiId> {
        self.controller_pane.as_ref()
    }
    pub fn controller_scope(&self) -> Option<&UiId> {
        self.controller_scope.as_ref()
    }
    pub fn controller_editing(&self) -> bool {
        self.controller_editing
    }
    pub fn retained_controller_focus(&self, scope: &UiId) -> Option<&UiId> {
        self.controller_retained_focus.get(scope)
    }
    pub(crate) fn retain_controller_focus(&mut self, scope: UiId, selected: UiId) {
        self.controller_retained_focus.insert(scope, selected);
    }
    pub(crate) fn forget_controller_focus(&mut self, scope: &UiId) {
        self.controller_retained_focus.remove(scope);
    }
    pub(crate) fn set_controller_selected(&mut self, id: Option<UiId>) -> Invalidation {
        replace_if_changed(&mut self.controller_selected, id, Invalidation::Paint)
    }
    pub(crate) fn set_controller_pane(&mut self, id: Option<UiId>) -> Invalidation {
        replace_if_changed(&mut self.controller_pane, id, Invalidation::Paint)
    }
    pub(crate) fn set_controller_scope(&mut self, id: Option<UiId>) -> Invalidation {
        replace_if_changed(&mut self.controller_scope, id, Invalidation::Paint)
    }
    pub(crate) fn set_controller_editing(&mut self, editing: bool) -> Invalidation {
        if self.controller_editing == editing {
            Invalidation::None
        } else {
            self.controller_editing = editing;
            Invalidation::Paint
        }
    }
}

#[derive(Clone, Debug)]
struct TextSelectionState {
    caret_visible: bool,
    selection_owner: Option<UiId>,
    document_selections: HashMap<UiId, DocumentSelection>,
    selection_documents: HashMap<UiId, Arc<SelectionDocument>>,
}

#[derive(Clone, Debug, Default)]
struct OverlayState {
    stack: Vec<(OverlayId, Option<UiId>)>,
    last_focus_return: Option<FocusReturn>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TextContextSession {
    pub editor: UiId,
    pub document_generation: u64,
    pub selection_generation: u64,
    pub secure: bool,
    pub editable: bool,
    pub anchor: OverlayAnchor,
}

#[derive(Clone, Debug)]
pub struct UiStateStore {
    durable: DurableNodeState,
    pointer: PointerModalityState,
    navigation: NavigationState,
    text: TextSelectionState,
    overlays: OverlayState,
    text_context: Option<TextContextSession>,
    clipboard_text: Option<String>,
}

impl Default for UiStateStore {
    fn default() -> Self {
        Self::with_retention_frames(2)
    }
}

impl UiStateStore {
    pub fn with_retention_frames(retention_frames: u64) -> Self {
        Self {
            durable: DurableNodeState {
                entries: HashMap::new(),
                frame: 0,
                retention_frames,
            },
            pointer: PointerModalityState {
                hovered: None,
                pressed: None,
                captured: None,
                scrollbar_grab_offset: None,
                input_modality: InputModality::default(),
                window_focused: true,
            },
            navigation: NavigationState::default(),
            text: TextSelectionState {
                caret_visible: true,
                selection_owner: None,
                document_selections: HashMap::new(),
                selection_documents: HashMap::new(),
            },
            overlays: OverlayState::default(),
            text_context: None,
            clipboard_text: None,
        }
    }

    pub(crate) fn set_clipboard_offer(&mut self, text: Option<&str>) {
        self.clipboard_text = text.map(ToOwned::to_owned);
    }

    pub(crate) fn clipboard_text(&self) -> Option<&str> {
        self.clipboard_text.as_deref()
    }

    pub(crate) fn text_context(&self) -> Option<&TextContextSession> {
        self.text_context.as_ref()
    }

    pub(crate) fn open_text_context(
        &mut self,
        editor: UiId,
        document_generation: u64,
        selection_generation: u64,
        secure: bool,
        anchor: OverlayAnchor,
    ) -> Invalidation {
        let menu = OverlayId::new(editor.scoped("text-context-menu"));
        self.text_context = Some(TextContextSession {
            editor: editor.clone(),
            document_generation,
            selection_generation,
            secure,
            editable: true,
            anchor,
        });
        self.open_overlay(menu, editor)
    }

    pub(crate) fn open_read_only_text_context(
        &mut self,
        target: UiId,
        document_generation: u64,
        selection_generation: u64,
        anchor: OverlayAnchor,
    ) -> Invalidation {
        let menu = OverlayId::new(target.scoped("text-context-menu"));
        self.text_context = Some(TextContextSession {
            editor: target.clone(),
            document_generation,
            selection_generation,
            secure: false,
            editable: false,
            anchor,
        });
        self.open_overlay(menu, target)
    }

    pub(crate) fn clear_text_context(&mut self) {
        self.text_context = None;
    }

    pub(crate) fn open_overlay(&mut self, id: OverlayId, invocation_target: UiId) -> Invalidation {
        let focus_return = self.navigation.focused.clone().or(Some(invocation_target));
        if self
            .overlays
            .stack
            .last()
            .is_some_and(|(open, _)| open == &id)
        {
            return Invalidation::None;
        }
        self.overlays.stack.push((id, focus_return));
        Invalidation::Layout
    }

    pub fn open_overlay_id(&self) -> Option<&OverlayId> {
        self.overlays.stack.last().map(|(id, _)| id)
    }

    pub fn overlay_stack(&self) -> impl DoubleEndedIterator<Item = &OverlayId> {
        self.overlays.stack.iter().map(|(id, _)| id)
    }

    pub(crate) fn dismiss_overlay(&mut self, reason: DismissReason) -> Invalidation {
        let Some((overlay, target)) = self.overlays.stack.pop() else {
            return Invalidation::None;
        };
        self.navigation.focused = target.clone();
        self.overlays.last_focus_return = Some(FocusReturn {
            overlay,
            target,
            reason,
        });
        self.text_context = None;
        Invalidation::Layout
    }

    pub fn take_focus_return(&mut self) -> Option<FocusReturn> {
        self.overlays.last_focus_return.take()
    }

    pub fn begin_frame(&mut self) {
        self.durable.frame = self.durable.frame.saturating_add(1);
    }

    pub fn touch(&mut self, id: impl Into<UiId>) -> &mut TransientState {
        let frame = self.durable.frame;
        &mut self
            .durable
            .entries
            .entry(id.into())
            .and_modify(|entry| entry.last_seen_frame = frame)
            .or_insert_with(|| StateEntry {
                state: TransientState::default(),
                last_seen_frame: frame,
            })
            .state
    }

    pub fn end_frame(&mut self) {
        let oldest = self
            .durable
            .frame
            .saturating_sub(self.durable.retention_frames);
        self.durable
            .entries
            .retain(|_, entry| entry.last_seen_frame >= oldest);
        self.clear_ownership_for_missing_entries();
    }

    /// Reconciles ephemeral interaction ownership against the current semantic
    /// topology. Cached per-component state may outlive a frame, but focus,
    /// capture, and controller ownership may not point at removed nodes.
    pub(crate) fn reconcile_live_targets(&mut self) {
        let live = self
            .durable
            .entries
            .iter()
            .filter(|(_, entry)| entry.last_seen_frame == self.durable.frame)
            .map(|(id, _)| id.clone())
            .collect::<std::collections::HashSet<_>>();
        for owner in [
            &mut self.navigation.focused,
            &mut self.pointer.hovered,
            &mut self.pointer.pressed,
            &mut self.pointer.captured,
            &mut self.navigation.controller_selected,
            &mut self.navigation.controller_pane,
            &mut self.navigation.controller_scope,
            &mut self.text.selection_owner,
        ] {
            if owner.as_ref().is_some_and(|id| !live.contains(id)) {
                *owner = None;
            }
        }
        if self.navigation.controller_selected.is_none() {
            self.navigation.controller_editing = false;
        }
        self.navigation
            .controller_retained_focus
            .retain(|scope, target| live.contains(scope) && live.contains(target));
    }

    pub fn contains(&self, id: &UiId) -> bool {
        self.durable.entries.contains_key(id)
    }

    pub fn state(&self, id: &UiId) -> Option<&TransientState> {
        self.durable.entries.get(id).map(|entry| &entry.state)
    }

    pub fn state_mut(&mut self, id: impl Into<UiId>) -> &mut TransientState {
        self.touch(id)
    }

    pub fn editor(&mut self, id: impl Into<UiId>, initial: impl Into<String>) -> &mut TextEditor {
        self.touch(id)
            .editor
            .get_or_insert_with(|| TextEditor::new(initial))
    }

    pub(crate) fn set_focus(&mut self, id: Option<UiId>) -> Invalidation {
        self.text.caret_visible = true;
        replace_if_changed(&mut self.navigation.focused, id, Invalidation::Paint)
    }

    pub fn set_hovered(&mut self, id: Option<UiId>) -> Invalidation {
        replace_if_changed(&mut self.pointer.hovered, id, Invalidation::Paint)
    }

    pub fn set_pressed(&mut self, id: Option<UiId>) -> Invalidation {
        replace_if_changed(&mut self.pointer.pressed, id, Invalidation::Paint)
    }

    pub fn set_capture(&mut self, id: Option<UiId>) -> Invalidation {
        if id.is_none() {
            self.pointer.scrollbar_grab_offset = None;
        }
        replace_if_changed(&mut self.pointer.captured, id, Invalidation::None)
    }

    pub(crate) fn set_scrollbar_grab_offset(&mut self, offset: f32) {
        self.pointer.scrollbar_grab_offset = Some(offset);
    }

    pub(crate) fn scrollbar_grab_offset(&self) -> Option<f32> {
        self.pointer.scrollbar_grab_offset
    }

    pub fn navigation(&self) -> &NavigationState {
        &self.navigation
    }

    pub(crate) fn navigation_mut(&mut self) -> &mut NavigationState {
        &mut self.navigation
    }

    pub fn focused(&self) -> Option<&UiId> {
        self.navigation.focused.as_ref()
    }

    pub fn hovered(&self) -> Option<&UiId> {
        self.pointer.hovered.as_ref()
    }

    pub fn pressed(&self) -> Option<&UiId> {
        self.pointer.pressed.as_ref()
    }

    pub fn captured(&self) -> Option<&UiId> {
        self.pointer.captured.as_ref()
    }

    pub fn input_modality(&self) -> InputModality {
        self.pointer.input_modality
    }

    pub(crate) fn set_input_modality(&mut self, modality: InputModality) -> Invalidation {
        if self.pointer.input_modality == modality {
            Invalidation::None
        } else {
            self.pointer.input_modality = modality;
            Invalidation::Paint
        }
    }

    pub fn window_focused(&self) -> bool {
        self.pointer.window_focused
    }

    pub(crate) fn set_window_focused(&mut self, focused: bool) -> Invalidation {
        let changed = self.pointer.window_focused != focused;
        self.pointer.window_focused = focused;
        let controller_changed = if focused {
            false
        } else {
            self.navigation.controller_selected.take().is_some()
                | self.navigation.controller_scope.take().is_some()
                | std::mem::take(&mut self.navigation.controller_editing)
        };
        if changed || controller_changed {
            Invalidation::Paint
        } else {
            Invalidation::None
        }
    }

    pub fn selection_owner(&self) -> Option<&UiId> {
        self.text.selection_owner.as_ref()
    }

    pub fn document_selection(&self, id: &UiId) -> Option<&DocumentSelection> {
        self.text.document_selections.get(id)
    }

    pub fn document_selection_mut(&mut self, id: impl Into<UiId>) -> &mut DocumentSelection {
        self.text.document_selections.entry(id.into()).or_default()
    }

    pub(crate) fn reconcile_document_selection(
        &mut self,
        id: UiId,
        document: Arc<SelectionDocument>,
    ) {
        let selection = self.text.document_selections.entry(id.clone()).or_default();
        if let Some(previous) = self.text.selection_documents.get(&id) {
            document.reconcile_from(previous, selection);
        } else {
            document.reconcile(selection);
        }
        self.text.selection_documents.insert(id, document);
    }

    pub fn set_selection_owner(&mut self, id: Option<UiId>) -> Invalidation {
        if self.text.selection_owner == id {
            return Invalidation::None;
        }
        self.text.selection_owner = id;
        Invalidation::Paint
    }

    pub fn clear_document_selection(&mut self) -> Invalidation {
        let changed = self.text.selection_owner.take().is_some()
            || self
                .text
                .document_selections
                .values()
                .any(|selection| selection.anchor.is_some() || selection.focus.is_some());
        self.text.document_selections.clear();
        self.text.selection_documents.clear();
        if changed {
            Invalidation::Paint
        } else {
            Invalidation::None
        }
    }

    pub fn caret_visible(&self) -> bool {
        self.text.caret_visible
    }

    pub fn show_caret(&mut self) -> Invalidation {
        if self.text.caret_visible {
            Invalidation::None
        } else {
            self.text.caret_visible = true;
            Invalidation::Paint
        }
    }

    pub fn toggle_caret(&mut self) -> Invalidation {
        if self.navigation.focused.is_none() {
            return Invalidation::None;
        }
        self.text.caret_visible = !self.text.caret_visible;
        Invalidation::Paint
    }

    pub fn scroll_by(&mut self, id: impl Into<UiId>, delta: f32, maximum: f32) -> Invalidation {
        let state = self.touch(id);
        let next = (state.scroll_offset + delta).clamp(0.0, maximum.max(0.0));
        state.scroll_at_end = next >= (maximum - 1.0).max(0.0);
        if next == state.scroll_offset {
            Invalidation::None
        } else {
            state.scroll_offset = next;
            Invalidation::Layout
        }
    }

    pub fn scroll_by_x(&mut self, id: impl Into<UiId>, delta: f32, maximum: f32) -> Invalidation {
        let state = self.touch(id);
        let next = (state.scroll_offset_x + delta).clamp(0.0, maximum.max(0.0));
        if next == state.scroll_offset_x {
            Invalidation::None
        } else {
            state.scroll_offset_x = next;
            Invalidation::Layout
        }
    }

    pub(crate) fn set_dropdown_open(&mut self, id: impl Into<UiId>, open: bool) -> Invalidation {
        let state = self.touch(id);
        if state.dropdown_open == open {
            Invalidation::None
        } else {
            state.dropdown_open = open;
            Invalidation::Layout
        }
    }

    pub fn schedule_repaint(delay: Duration) -> Invalidation {
        Invalidation::Scheduled(delay)
    }

    pub fn focus_lost(&mut self) -> Invalidation {
        let pressed = self.pointer.pressed.take().is_some();
        let captured = self.pointer.captured.take().is_some();
        let context = self.text_context.take().is_some();
        let overlay = if context {
            self.dismiss_overlay(DismissReason::Cancel)
        } else {
            Invalidation::None
        };
        let changed = pressed || captured;
        if changed {
            Invalidation::Paint.merge(overlay)
        } else {
            overlay
        }
    }

    pub fn suspended(&mut self) -> Invalidation {
        self.focus_lost()
    }

    pub fn device_removed(&mut self) -> Invalidation {
        let changed = self.navigation.controller_selected.take().is_some();
        self.focus_lost().merge(if changed {
            Invalidation::Paint
        } else {
            Invalidation::None
        })
    }

    pub fn destroy(&mut self) {
        self.durable.entries.clear();
        self.navigation.focused = None;
        self.pointer.hovered = None;
        self.pointer.pressed = None;
        self.pointer.captured = None;
        self.navigation.controller_selected = None;
        self.navigation.controller_pane = None;
        self.navigation.controller_scope = None;
        self.navigation.controller_editing = false;
        self.pointer.input_modality = InputModality::default();
        self.navigation.controller_retained_focus.clear();
        self.text.caret_visible = true;
        self.text.selection_owner = None;
        self.text.document_selections.clear();
        self.text.selection_documents.clear();
    }

    fn clear_ownership_for_missing_entries(&mut self) {
        let entries = &self.durable.entries;
        for owner in [
            &mut self.navigation.focused,
            &mut self.pointer.hovered,
            &mut self.pointer.pressed,
            &mut self.pointer.captured,
            &mut self.navigation.controller_selected,
            &mut self.navigation.controller_pane,
            &mut self.navigation.controller_scope,
            &mut self.text.selection_owner,
        ] {
            if owner.as_ref().is_some_and(|id| !entries.contains_key(id)) {
                *owner = None;
            }
        }
        self.text
            .document_selections
            .retain(|id, _| self.durable.entries.contains_key(id));
        self.text
            .selection_documents
            .retain(|id, _| self.durable.entries.contains_key(id));
        self.navigation
            .controller_retained_focus
            .retain(|scope, selected| {
                self.durable.entries.contains_key(scope)
                    && self.durable.entries.contains_key(selected)
            });
    }
}

fn replace_if_changed(
    target: &mut Option<UiId>,
    value: Option<UiId>,
    invalidation: Invalidation,
) -> Invalidation {
    if *target == value {
        Invalidation::None
    } else {
        *target = value;
        invalidation
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyed_state_survives_insertion_reordering_and_short_absence() {
        let mut store = UiStateStore::with_retention_frames(2);
        store.begin_frame();
        store.touch("list/a").scroll_offset = 42.0;
        store.touch("list/b");
        store.end_frame();

        store.begin_frame();
        store.touch("list/new");
        store.touch("list/b");
        store.touch("list/a");
        store.end_frame();
        assert_eq!(
            store.state(&UiId::from("list/a")).unwrap().scroll_offset,
            42.0
        );

        store.begin_frame();
        store.touch("list/b");
        store.end_frame();
        store.begin_frame();
        store.touch("list/a");
        store.end_frame();
        assert_eq!(
            store.state(&UiId::from("list/a")).unwrap().scroll_offset,
            42.0
        );
    }

    #[test]
    fn abandoned_state_is_reclaimed_after_bounded_generations() {
        let mut store = UiStateStore::with_retention_frames(1);
        store.begin_frame();
        store.touch("gone");
        store.end_frame();
        store.begin_frame();
        store.end_frame();
        assert!(store.contains(&UiId::from("gone")));
        store.begin_frame();
        store.end_frame();
        assert!(!store.contains(&UiId::from("gone")));
    }

    #[test]
    fn removed_targets_lose_interaction_ownership_before_cached_state_expires() {
        let mut store = UiStateStore::with_retention_frames(4);
        store.begin_frame();
        store.touch("removed");
        store.set_focus(Some(UiId::from("removed")));
        store
            .navigation_mut()
            .set_controller_selected(Some(UiId::from("removed")));
        store.end_frame();

        store.begin_frame();
        store.touch("retained");
        store.end_frame();
        store.reconcile_live_targets();

        assert!(
            store.contains(&UiId::from("removed")),
            "derived state remains cached"
        );
        assert!(store.focused().is_none());
        assert!(store.navigation().controller_selected().is_none());
    }

    #[test]
    fn focus_loss_device_removal_and_destroy_clear_input_ownership() {
        let mut store = UiStateStore::default();
        store.begin_frame();
        for id in ["field", "button", "controller"] {
            store.touch(id);
        }
        store.set_focus(Some(UiId::from("field")));
        store.set_pressed(Some(UiId::from("button")));
        store.set_capture(Some(UiId::from("button")));
        store
            .navigation_mut()
            .set_controller_selected(Some(UiId::from("controller")));
        assert_eq!(store.focus_lost(), Invalidation::Paint);
        assert!(store.pressed().is_none() && store.captured().is_none());
        assert_eq!(store.device_removed(), Invalidation::Paint);
        assert!(store.navigation().controller_selected().is_none());
        store.destroy();
        assert!(store.focused().is_none() && !store.contains(&UiId::from("field")));
    }

    #[test]
    fn transient_controls_persist_and_return_minimum_invalidations() {
        let mut store = UiStateStore::default();
        store.begin_frame();
        assert_eq!(
            store.set_hovered(Some(UiId::from("button"))),
            Invalidation::Paint
        );
        assert_eq!(
            store.set_hovered(Some(UiId::from("button"))),
            Invalidation::None
        );
        assert_eq!(store.scroll_by("list", 10.0, 100.0), Invalidation::Layout);
        assert_eq!(store.scroll_by("list", 0.0, 100.0), Invalidation::None);
        assert_eq!(store.set_dropdown_open("menu", true), Invalidation::Layout);
        assert_eq!(store.set_dropdown_open("menu", true), Invalidation::None);
        let editor = store.editor("query", "hello");
        editor.move_left(false);
        editor.set_preedit("世界", Some(0..3));
        let snapshot = editor.clone();
        store.end_frame();
        store.begin_frame();
        assert_eq!(store.editor("query", "ignored"), &snapshot);
    }

    #[test]
    fn nested_transients_cancel_from_the_top_and_restore_each_focus_owner() {
        let mut store = UiStateStore::default();
        store.set_focus(Some(UiId::from("page-button")));
        store.open_overlay(OverlayId::new("popover"), UiId::from("page-button"));
        store.set_focus(Some(UiId::from("popover-action")));
        store.open_overlay(OverlayId::new("dialog"), UiId::from("popover-action"));
        assert_eq!(
            store.overlay_stack().cloned().collect::<Vec<_>>(),
            [OverlayId::new("popover"), OverlayId::new("dialog")]
        );
        store.dismiss_overlay(DismissReason::Cancel);
        assert_eq!(store.open_overlay_id(), Some(&OverlayId::new("popover")));
        assert_eq!(store.focused(), Some(&UiId::from("popover-action")));
        store.dismiss_overlay(DismissReason::Cancel);
        assert!(store.open_overlay_id().is_none());
        assert_eq!(store.focused(), Some(&UiId::from("page-button")));
    }
}
