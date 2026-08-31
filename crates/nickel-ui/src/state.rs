use std::{collections::HashMap, sync::Arc, time::Duration};

use crate::{DocumentSelection, SelectionDocument, TextEditor};

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
pub struct UiStateStore {
    entries: HashMap<UiId, StateEntry>,
    frame: u64,
    retention_frames: u64,
    focused: Option<UiId>,
    hovered: Option<UiId>,
    pressed: Option<UiId>,
    captured: Option<UiId>,
    controller_selected: Option<UiId>,
    controller_pane: Option<UiId>,
    controller_scope: Option<UiId>,
    controller_editing: bool,
    window_focused: bool,
    caret_visible: bool,
    selection_owner: Option<UiId>,
    document_selections: HashMap<UiId, DocumentSelection>,
    selection_documents: HashMap<UiId, Arc<SelectionDocument>>,
}

impl Default for UiStateStore {
    fn default() -> Self {
        Self::with_retention_frames(2)
    }
}

impl UiStateStore {
    pub fn with_retention_frames(retention_frames: u64) -> Self {
        Self {
            entries: HashMap::new(),
            frame: 0,
            retention_frames,
            focused: None,
            hovered: None,
            pressed: None,
            captured: None,
            controller_selected: None,
            controller_pane: None,
            controller_scope: None,
            controller_editing: false,
            window_focused: true,
            caret_visible: true,
            selection_owner: None,
            document_selections: HashMap::new(),
            selection_documents: HashMap::new(),
        }
    }

    pub fn begin_frame(&mut self) {
        self.frame = self.frame.saturating_add(1);
    }

    pub fn touch(&mut self, id: impl Into<UiId>) -> &mut TransientState {
        let frame = self.frame;
        &mut self
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
        let oldest = self.frame.saturating_sub(self.retention_frames);
        self.entries
            .retain(|_, entry| entry.last_seen_frame >= oldest);
        self.clear_ownership_for_missing_entries();
    }

    pub fn contains(&self, id: &UiId) -> bool {
        self.entries.contains_key(id)
    }

    pub fn state(&self, id: &UiId) -> Option<&TransientState> {
        self.entries.get(id).map(|entry| &entry.state)
    }

    pub fn state_mut(&mut self, id: impl Into<UiId>) -> &mut TransientState {
        self.touch(id)
    }

    pub fn editor(&mut self, id: impl Into<UiId>, initial: impl Into<String>) -> &mut TextEditor {
        self.touch(id)
            .editor
            .get_or_insert_with(|| TextEditor::new(initial))
    }

    pub fn set_focus(&mut self, id: Option<UiId>) -> Invalidation {
        self.caret_visible = true;
        replace_if_changed(&mut self.focused, id, Invalidation::Paint)
    }

    pub fn set_hovered(&mut self, id: Option<UiId>) -> Invalidation {
        replace_if_changed(&mut self.hovered, id, Invalidation::Paint)
    }

    pub fn set_pressed(&mut self, id: Option<UiId>) -> Invalidation {
        replace_if_changed(&mut self.pressed, id, Invalidation::Paint)
    }

    pub fn set_capture(&mut self, id: Option<UiId>) -> Invalidation {
        replace_if_changed(&mut self.captured, id, Invalidation::None)
    }

    pub fn set_controller_selected(&mut self, id: Option<UiId>) -> Invalidation {
        replace_if_changed(&mut self.controller_selected, id, Invalidation::Paint)
    }

    pub fn focused(&self) -> Option<&UiId> {
        self.focused.as_ref()
    }

    pub fn hovered(&self) -> Option<&UiId> {
        self.hovered.as_ref()
    }

    pub fn pressed(&self) -> Option<&UiId> {
        self.pressed.as_ref()
    }

    pub fn captured(&self) -> Option<&UiId> {
        self.captured.as_ref()
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

    pub fn window_focused(&self) -> bool {
        self.window_focused
    }

    pub fn set_window_focused(&mut self, focused: bool) -> Invalidation {
        let changed = self.window_focused != focused;
        self.window_focused = focused;
        let controller_changed = if focused {
            false
        } else {
            self.controller_selected.take().is_some()
                | self.controller_scope.take().is_some()
                | std::mem::take(&mut self.controller_editing)
        };
        if changed || controller_changed {
            Invalidation::Paint
        } else {
            Invalidation::None
        }
    }

    pub fn set_controller_pane(&mut self, id: Option<UiId>) -> Invalidation {
        replace_if_changed(&mut self.controller_pane, id, Invalidation::Paint)
    }

    pub fn set_controller_scope(&mut self, id: Option<UiId>) -> Invalidation {
        replace_if_changed(&mut self.controller_scope, id, Invalidation::Paint)
    }

    pub fn set_controller_editing(&mut self, editing: bool) -> Invalidation {
        if self.controller_editing == editing {
            Invalidation::None
        } else {
            self.controller_editing = editing;
            Invalidation::Paint
        }
    }

    pub fn selection_owner(&self) -> Option<&UiId> {
        self.selection_owner.as_ref()
    }

    pub fn document_selection(&self, id: &UiId) -> Option<&DocumentSelection> {
        self.document_selections.get(id)
    }

    pub fn document_selection_mut(&mut self, id: impl Into<UiId>) -> &mut DocumentSelection {
        self.document_selections.entry(id.into()).or_default()
    }

    pub(crate) fn reconcile_document_selection(
        &mut self,
        id: UiId,
        document: Arc<SelectionDocument>,
    ) {
        let selection = self.document_selections.entry(id.clone()).or_default();
        if let Some(previous) = self.selection_documents.get(&id) {
            document.reconcile_from(previous, selection);
        } else {
            document.reconcile(selection);
        }
        self.selection_documents.insert(id, document);
    }

    pub fn set_selection_owner(&mut self, id: Option<UiId>) -> Invalidation {
        if self.selection_owner == id {
            return Invalidation::None;
        }
        self.selection_owner = id;
        Invalidation::Paint
    }

    pub fn clear_document_selection(&mut self) -> Invalidation {
        let changed = self.selection_owner.take().is_some()
            || self
                .document_selections
                .values()
                .any(|selection| selection.anchor.is_some() || selection.focus.is_some());
        self.document_selections.clear();
        self.selection_documents.clear();
        if changed {
            Invalidation::Paint
        } else {
            Invalidation::None
        }
    }

    pub fn caret_visible(&self) -> bool {
        self.caret_visible
    }

    pub fn show_caret(&mut self) -> Invalidation {
        if self.caret_visible {
            Invalidation::None
        } else {
            self.caret_visible = true;
            Invalidation::Paint
        }
    }

    pub fn toggle_caret(&mut self) -> Invalidation {
        if self.focused.is_none() {
            return Invalidation::None;
        }
        self.caret_visible = !self.caret_visible;
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

    pub fn set_dropdown_open(&mut self, id: impl Into<UiId>, open: bool) -> Invalidation {
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
        let pressed = self.pressed.take().is_some();
        let captured = self.captured.take().is_some();
        let changed = pressed || captured;
        if changed {
            Invalidation::Paint
        } else {
            Invalidation::None
        }
    }

    pub fn suspended(&mut self) -> Invalidation {
        self.focus_lost()
    }

    pub fn device_removed(&mut self) -> Invalidation {
        let changed = self.controller_selected.take().is_some();
        self.focus_lost().merge(if changed {
            Invalidation::Paint
        } else {
            Invalidation::None
        })
    }

    pub fn destroy(&mut self) {
        self.entries.clear();
        self.focused = None;
        self.hovered = None;
        self.pressed = None;
        self.captured = None;
        self.controller_selected = None;
        self.controller_pane = None;
        self.controller_scope = None;
        self.controller_editing = false;
        self.caret_visible = true;
        self.selection_owner = None;
        self.document_selections.clear();
        self.selection_documents.clear();
    }

    fn clear_ownership_for_missing_entries(&mut self) {
        let entries = &self.entries;
        for owner in [
            &mut self.focused,
            &mut self.hovered,
            &mut self.pressed,
            &mut self.captured,
            &mut self.controller_selected,
            &mut self.controller_pane,
            &mut self.controller_scope,
            &mut self.selection_owner,
        ] {
            if owner.as_ref().is_some_and(|id| !entries.contains_key(id)) {
                *owner = None;
            }
        }
        self.document_selections
            .retain(|id, _| self.entries.contains_key(id));
        self.selection_documents
            .retain(|id, _| self.entries.contains_key(id));
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
    fn focus_loss_device_removal_and_destroy_clear_input_ownership() {
        let mut store = UiStateStore::default();
        store.begin_frame();
        for id in ["field", "button", "controller"] {
            store.touch(id);
        }
        store.set_focus(Some(UiId::from("field")));
        store.set_pressed(Some(UiId::from("button")));
        store.set_capture(Some(UiId::from("button")));
        store.set_controller_selected(Some(UiId::from("controller")));
        assert_eq!(store.focus_lost(), Invalidation::Paint);
        assert!(store.pressed().is_none() && store.captured().is_none());
        assert_eq!(store.device_removed(), Invalidation::Paint);
        assert!(store.controller_selected().is_none());
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
}
