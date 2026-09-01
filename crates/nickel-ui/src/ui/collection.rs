use std::{collections::HashMap, fmt};

use super::{
    AnyView, Background, Color, Column, Component, ComponentBuilderExt, Container, Element, Grid,
    NavigationScope, ReadingDirection, SemanticRole, Text, Track, UiId, VirtualColumn,
    VirtualWindow,
};
use crate::{InputModality, ViewContext};

/// The data lifecycle represented by a [`Collection`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CollectionState<T> {
    Loading,
    Ready(Vec<T>),
    Error(String),
}

/// Declarative layout policy for a [`Collection`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum CollectionPresentation {
    #[default]
    List,
    UniformGrid {
        columns: usize,
    },
    AdaptiveGrid {
        minimum_item_width: f32,
    },
    /// A fixed-extent list which only constructs the visible window plus overscan.
    VirtualList {
        item_height: f32,
        offset: f32,
        viewport_height: f32,
        overscan: f32,
    },
}

/// A construction error that would make item identity ambiguous.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CollectionError<K> {
    DuplicateKey {
        key: K,
        first: usize,
        duplicate: usize,
    },
    DuplicateId {
        id: String,
        first: usize,
        duplicate: usize,
    },
}

type CollectionAction<K, Message> = Box<dyn Fn(&K) -> Message>;
type CollectionPredicate<K> = Box<dyn Fn(&K) -> bool>;
type CollectionItemLabel<T> = Box<dyn Fn(&T) -> String>;
type CollectionErrorView<Message> = Box<dyn Fn(&str) -> Element<Message>>;

struct CollectionInteractions<K, Message> {
    activate: Option<CollectionAction<K, Message>>,
    context: Option<CollectionAction<K, Message>>,
    selected: Option<CollectionPredicate<K>>,
    disabled: Option<CollectionPredicate<K>>,
}

impl<K, Message> Default for CollectionInteractions<K, Message> {
    fn default() -> Self {
        Self {
            activate: None,
            context: None,
            selected: None,
            disabled: None,
        }
    }
}

impl<K: fmt::Display> fmt::Display for CollectionError<K> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateKey {
                key,
                first,
                duplicate,
            } => write!(
                formatter,
                "duplicate collection key `{key}` at item {duplicate} (first used at item {first})"
            ),
            Self::DuplicateId {
                id,
                first,
                duplicate,
            } => write!(
                formatter,
                "collection keys produce duplicate UI id `{id}` at item {duplicate} (first used at item {first})"
            ),
        }
    }
}

impl<K: fmt::Debug + fmt::Display> std::error::Error for CollectionError<K> {}

/// A keyed, declarative list or grid.
///
/// Source items and the item renderer are retained until `into_element`; rendered
/// `Element` trees are not cached. Item identity is derived exclusively from the
/// caller's key and fed into the normal `UiFrame` identity, semantics, and hit paths.
pub struct Collection<T, K, Message, Render, View> {
    state: CollectionState<T>,
    keyed_items: Vec<(K, T)>,
    render: Render,
    presentation: CollectionPresentation,
    id: UiId,
    gap: f32,
    item_focus_border: Option<Color>,
    item_controller_focus_border: Option<Color>,
    navigation_scope: Option<NavigationScope>,
    controller_scope_background: Option<Background>,
    interactions: CollectionInteractions<K, Message>,
    item_label: Option<CollectionItemLabel<T>>,
    reveal: Option<K>,
    reveal_target: Option<UiId>,
    direction: ReadingDirection,
    empty_label: String,
    loading_label: String,
    error_prefix: String,
    empty_slot: Option<Element<Message>>,
    loading_slot: Option<Element<Message>>,
    error_slot: Option<CollectionErrorView<Message>>,
    _view: std::marker::PhantomData<fn() -> View>,
}

impl<T, K, Message, Render, View> Collection<T, K, Message, Render, View>
where
    K: Clone + Eq + std::hash::Hash + fmt::Display,
    Render: Fn(T) -> View,
    View: Component<Message>,
{
    pub fn try_new(
        state: CollectionState<T>,
        key: impl Fn(&T) -> K,
        render: Render,
    ) -> Result<Self, CollectionError<K>> {
        let mut keyed_items = Vec::new();
        let state = match state {
            CollectionState::Ready(items) => {
                let mut keys = HashMap::<K, usize>::new();
                let mut ids = HashMap::<String, usize>::new();
                for (index, item) in items.into_iter().enumerate() {
                    let item_key = key(&item);
                    if let Some(first) = keys.insert(item_key.clone(), index) {
                        return Err(CollectionError::DuplicateKey {
                            key: item_key,
                            first,
                            duplicate: index,
                        });
                    }
                    let item_id = item_key.to_string();
                    if let Some(first) = ids.insert(item_id.clone(), index) {
                        return Err(CollectionError::DuplicateId {
                            id: item_id,
                            first,
                            duplicate: index,
                        });
                    }
                    keyed_items.push((item_key, item));
                }
                CollectionState::Ready(Vec::new())
            }
            CollectionState::Error(error) => CollectionState::Error(error),
            CollectionState::Loading => CollectionState::Loading,
        };
        Ok(Self {
            state,
            keyed_items,
            render,
            presentation: CollectionPresentation::List,
            id: UiId::from("collection"),
            gap: 0.0,
            item_focus_border: None,
            item_controller_focus_border: None,
            navigation_scope: None,
            controller_scope_background: None,
            interactions: CollectionInteractions::default(),
            item_label: None,
            reveal: None,
            reveal_target: None,
            direction: ReadingDirection::LeftToRight,
            empty_label: "No items".into(),
            loading_label: "Loading".into(),
            error_prefix: "Error: ".into(),
            empty_slot: None,
            loading_slot: None,
            error_slot: None,
            _view: std::marker::PhantomData,
        })
    }

    pub fn id(mut self, id: impl Into<UiId>) -> Self {
        self.id = id.into();
        self
    }

    /// Paints keyboard focus on the keyed item which owns its semantic actions.
    pub fn item_focus_border(mut self, color: Color) -> Self {
        self.item_focus_border = Some(color);
        self
    }

    /// Paints controller selection on the keyed item which owns its semantic actions.
    pub fn item_controller_focus_border(mut self, color: Color) -> Self {
        self.item_controller_focus_border = Some(color);
        self
    }

    pub fn navigation_scope(mut self, scope: NavigationScope) -> Self {
        self.navigation_scope = Some(scope);
        self
    }

    pub fn controller_scope_background(mut self, background: impl Into<Background>) -> Self {
        self.controller_scope_background = Some(background.into());
        self
    }

    pub fn presentation(mut self, presentation: CollectionPresentation) -> Self {
        self.presentation = presentation;
        self
    }

    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = gap.max(0.0);
        self
    }

    pub fn on_activate(mut self, action: impl Fn(&K) -> Message + 'static) -> Self {
        self.interactions.activate = Some(Box::new(action));
        self
    }

    pub fn on_context(mut self, action: impl Fn(&K) -> Message + 'static) -> Self {
        self.interactions.context = Some(Box::new(action));
        self
    }

    /// Declares selected items without coupling collection identity to view state.
    pub fn selected_when(mut self, selected: impl Fn(&K) -> bool + 'static) -> Self {
        self.interactions.selected = Some(Box::new(selected));
        self
    }

    /// Disabled items remain represented to accessibility but expose no actions or hit target.
    pub fn disabled_when(mut self, disabled: impl Fn(&K) -> bool + 'static) -> Self {
        self.interactions.disabled = Some(Box::new(disabled));
        self
    }

    pub fn item_label(mut self, label: impl Fn(&T) -> String + 'static) -> Self {
        self.item_label = Some(Box::new(label));
        self
    }

    /// Ensures a stable key is included in a virtualized window on the next rebuild.
    pub fn reveal(mut self, key: K) -> Self {
        self.reveal = Some(key);
        self
    }

    /// Keeps the host-owned keyboard/accessibility focus or controller selection
    /// inside a virtualized window across declarative rebuilds.
    ///
    /// Collection item identity remains the stable key. Applications pass the
    /// production [`ViewContext`] they already receive; they do not mirror focus
    /// or calculate an index/offset themselves.
    pub fn reveal_on_focus(mut self, context: &ViewContext) -> Self {
        self.reveal_target = if context.modality == InputModality::Controller {
            context
                .controller_target
                .clone()
                .or_else(|| context.focused.clone())
        } else {
            context.focused.clone()
        };
        self
    }

    pub fn direction(mut self, direction: ReadingDirection) -> Self {
        self.direction = direction;
        self
    }

    pub fn empty_label(mut self, label: impl Into<String>) -> Self {
        self.empty_label = label.into();
        self
    }

    pub fn loading_label(mut self, label: impl Into<String>) -> Self {
        self.loading_label = label.into();
        self
    }

    pub fn error_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.error_prefix = prefix.into();
        self
    }

    pub fn empty_slot(mut self, slot: impl Component<Message>) -> Self {
        self.empty_slot = Some(slot.into_element());
        self
    }

    pub fn loading_slot(mut self, slot: impl Component<Message>) -> Self {
        self.loading_slot = Some(slot.into_element());
        self
    }

    pub fn error_slot(mut self, slot: impl Fn(&str) -> Element<Message> + 'static) -> Self {
        self.error_slot = Some(Box::new(slot));
        self
    }

    fn status(self, label: String) -> Element<Message> {
        Container::new()
            .id(self.id)
            .semantic_role(SemanticRole::Status)
            .accessibility_label(label.clone())
            .child(Text::new(label))
            .into_element()
    }
}

impl<T, K, Message, Render, View> Component<Message> for Collection<T, K, Message, Render, View>
where
    K: Clone + Eq + std::hash::Hash + fmt::Display,
    Render: Fn(T) -> View,
    View: Component<Message>,
{
    fn into_element(mut self) -> Element<Message> {
        match &self.state {
            CollectionState::Loading => {
                if let Some(slot) = self.loading_slot.take() {
                    return slot;
                }
                let label = self.loading_label.clone();
                return self.status(label);
            }
            CollectionState::Error(error) => {
                if let Some(slot) = self.error_slot.take() {
                    return slot(error);
                }
                let label = format!("{}{error}", self.error_prefix);
                return self.status(label);
            }
            CollectionState::Ready(_) if self.keyed_items.is_empty() => {
                if let Some(slot) = self.empty_slot.take() {
                    return slot;
                }
                let label = self.empty_label.clone();
                return self.status(label);
            }
            CollectionState::Ready(_) => {}
        }

        let item_role = match self.presentation {
            CollectionPresentation::List | CollectionPresentation::VirtualList { .. } => {
                SemanticRole::ListItem
            }
            CollectionPresentation::UniformGrid { .. }
            | CollectionPresentation::AdaptiveGrid { .. } => SemanticRole::GridCell,
        };
        let total = self.keyed_items.len();
        let mut window = 0..total;
        let mut virtual_window = None;
        if let CollectionPresentation::VirtualList {
            item_height,
            offset,
            viewport_height,
            overscan,
        } = self.presentation
        {
            let heights = vec![item_height.max(1.0); total];
            let mut resolved =
                VirtualWindow::from_heights(&heights, self.gap, offset, viewport_height, overscan);
            let reveal_index = self
                .reveal
                .as_ref()
                .and_then(|key| {
                    self.keyed_items
                        .iter()
                        .position(|(candidate, _)| candidate == key)
                })
                .or_else(|| {
                    let target = self.reveal_target.as_ref()?;
                    self.keyed_items
                        .iter()
                        .position(|(key, _)| target.as_str().ends_with(&format!("/{key}")))
                });
            if let Some(index) = reveal_index
                && !resolved.range.contains(&index)
            {
                let stride = item_height.max(1.0) + self.gap;
                resolved = VirtualWindow::from_heights(
                    &heights,
                    self.gap,
                    index as f32 * stride,
                    viewport_height,
                    overscan,
                );
            }
            window = resolved.range.clone();
            virtual_window = Some(resolved);
        }
        let selected = &self.interactions.selected;
        let disabled = &self.interactions.disabled;
        let item_label = &self.item_label;
        let columns = match self.presentation {
            CollectionPresentation::UniformGrid { columns } => Some(columns.max(1)),
            _ => None,
        };
        let direction = self.direction;
        let children = self
            .keyed_items
            .into_iter()
            .enumerate()
            .filter(|(index, _)| window.contains(index))
            .map(|(index, (key, item))| {
                let accessible_name = item_label
                    .as_ref()
                    .map_or_else(|| key.to_string(), |label| label(&item));
                let is_selected = selected.as_ref().is_some_and(|predicate| predicate(&key));
                let is_disabled = disabled.as_ref().is_some_and(|predicate| predicate(&key));
                let mut item_container = Container::new()
                    .id(key.to_string())
                    .semantic_role(item_role)
                    .accessibility_label(accessible_name)
                    .accessibility_description(if let Some(columns) = columns {
                        let row = index / columns + 1;
                        let logical_column = index % columns;
                        let column = match direction {
                            ReadingDirection::LeftToRight => logical_column + 1,
                            ReadingDirection::RightToLeft => columns - logical_column,
                        };
                        format!("row {row}, column {column}, item {} of {total}", index + 1)
                    } else {
                        format!("item {} of {total}", index + 1)
                    })
                    .accessibility_state(match (is_selected, is_disabled) {
                        (true, true) => "selected, disabled",
                        (true, false) => "selected",
                        (false, true) => "disabled",
                        (false, false) => "unselected",
                    })
                    .child(AnyView::new((self.render)(item)));
                if let Some(color) = self.item_focus_border {
                    item_container = item_container.focus_border(color);
                }
                if let Some(color) = self.item_controller_focus_border {
                    item_container = item_container.controller_focus_border(color);
                }
                let mut element = item_container.into_element();
                if !is_disabled && let Some(action) = &self.interactions.activate {
                    element = element.message(action(&key));
                }
                if !is_disabled && let Some(action) = &self.interactions.context {
                    element = element.context_message(action(&key));
                }
                element
            });

        let mut element = match self.presentation {
            CollectionPresentation::List => Column::new()
                .id(self.id)
                .semantic_role(SemanticRole::List)
                .gap(self.gap)
                .children(children)
                .into_element(),
            CollectionPresentation::UniformGrid { columns } => Grid::fixed(columns.max(1))
                .id(self.id)
                .semantic_role(SemanticRole::Grid)
                .gap(self.gap)
                .children(children)
                .into_element(),
            CollectionPresentation::AdaptiveGrid { minimum_item_width } => Grid::auto_fit(
                Track::minmax(Track::px(minimum_item_width.max(1.0)), Track::fr(1.0)),
            )
            .id(self.id)
            .semantic_role(SemanticRole::Grid)
            .gap(self.gap)
            .children(children)
            .into_element(),
            CollectionPresentation::VirtualList { .. } => VirtualColumn::new()
                .window(virtual_window.expect("virtual presentation constructs a window"))
                .gap(self.gap)
                .children(children)
                .into_element()
                .id(self.id)
                .semantic_role(SemanticRole::List),
        };
        element.navigation_scope = self.navigation_scope;
        element.style.controller_scope_background = self.controller_scope_background;
        element
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ActionKind, Application, Rect, SemanticAction, UiEvent, UiFrame, UiHost, UiStateStore,
    };

    #[derive(Clone, Debug, Eq, PartialEq)]
    enum Message {
        Activate(u32),
        Context(u32),
    }

    type TestCollection = Collection<
        (u32, &'static str),
        u32,
        Message,
        fn((u32, &'static str)) -> Text<Message>,
        Text<Message>,
    >;

    fn render_item(item: (u32, &'static str)) -> Text<Message> {
        Text::new(item.1)
    }

    fn collection(items: Vec<(u32, &'static str)>) -> TestCollection {
        Collection::try_new(
            CollectionState::Ready(items),
            |item| item.0,
            render_item as fn((u32, &'static str)) -> Text<Message>,
        )
        .unwrap()
        .id("people")
        .on_activate(|key| Message::Activate(*key))
        .on_context(|key| Message::Context(*key))
    }

    #[test]
    fn keyed_identity_survives_reorder() {
        let bounds = Rect::new(0.0, 0.0, 240.0, 120.0);
        let first = UiFrame::layout(collection(vec![(1, "One"), (2, "Two")]), bounds);
        let reordered = UiFrame::layout(collection(vec![(2, "Two"), (1, "One")]), bounds);
        for message in [Message::Activate(1), Message::Activate(2)] {
            assert_eq!(
                first
                    .semantic_targets_for_message(&message)
                    .into_iter()
                    .map(|target| target.id)
                    .collect::<Vec<_>>(),
                reordered
                    .semantic_targets_for_message(&message)
                    .into_iter()
                    .map(|target| target.id)
                    .collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn removed_key_disappears_from_semantics_and_actions() {
        let bounds = Rect::new(0.0, 0.0, 240.0, 120.0);
        let before = UiFrame::layout(collection(vec![(1, "One"), (2, "Two")]), bounds);
        let after = UiFrame::layout(collection(vec![(2, "Two")]), bounds);
        assert!(
            !before
                .semantic_targets_for_message(&Message::Activate(1))
                .is_empty()
        );
        assert!(
            after
                .semantic_targets_for_message(&Message::Activate(1))
                .is_empty()
        );
        assert!(
            after
                .accessibility_nodes()
                .iter()
                .all(|node| !node.id.as_str().ends_with("/1"))
        );
    }

    #[test]
    fn duplicate_keys_are_rejected_with_both_positions() {
        let result = Collection::try_new(
            CollectionState::Ready(vec![(7_u32, "first"), (7, "second")]),
            |item| item.0,
            |item| Text::<Message>::new(item.1),
        );
        assert!(matches!(
            result,
            Err(CollectionError::DuplicateKey {
                key: 7,
                first: 0,
                duplicate: 1
            })
        ));
    }

    #[test]
    fn semantic_item_actions_use_typed_messages() {
        let tree = UiFrame::layout(
            collection(vec![(7, "Seven")]),
            Rect::new(0.0, 0.0, 200.0, 80.0),
        );
        let item = tree
            .semantic_targets_for_message(&Message::Activate(7))
            .into_iter()
            .next()
            .unwrap()
            .id;
        assert!(
            tree.accessibility_nodes()
                .iter()
                .any(|node| node.role.as_deref() == Some(SemanticRole::List.as_str()))
        );
        assert!(tree.accessibility_nodes().iter().any(|node| {
            node.id == item && node.role.as_deref() == Some(SemanticRole::ListItem.as_str())
        }));
        assert_eq!(
            tree.perform_semantic_action(&item, SemanticAction::Invoke(ActionKind::Activate))
                .unwrap()
                .messages,
            vec![Message::Activate(7)]
        );
        assert_eq!(
            tree.perform_semantic_action(&item, SemanticAction::Invoke(ActionKind::ContextMenu))
                .unwrap()
                .messages,
            vec![Message::Context(7)]
        );
    }

    #[test]
    fn adaptive_grid_changes_column_count_with_available_width() {
        let view = |width| {
            UiFrame::layout(
                collection(vec![(1, "One"), (2, "Two"), (3, "Three")]).presentation(
                    CollectionPresentation::AdaptiveGrid {
                        minimum_item_width: 100.0,
                    },
                ),
                Rect::new(0.0, 0.0, width, 200.0),
            )
        };
        assert_eq!(view(220.0).resolved_grid_columns(), Some(2));
        assert_eq!(view(340.0).resolved_grid_columns(), Some(3));
    }

    #[test]
    fn virtual_list_constructs_only_window_and_reveal_moves_it() {
        let items = (0..100).map(|id| (id, "row")).collect();
        let tree = UiFrame::layout(
            collection(items)
                .presentation(CollectionPresentation::VirtualList {
                    item_height: 20.0,
                    offset: 0.0,
                    viewport_height: 60.0,
                    overscan: 0.0,
                })
                .reveal(80),
            Rect::new(0.0, 0.0, 200.0, 60.0),
        );
        let nodes = tree.accessibility_nodes();
        assert!(nodes.iter().any(|node| node.id.as_str().ends_with("/80")));
        assert!(!nodes.iter().any(|node| node.id.as_str().ends_with("/0")));
        assert!(
            nodes
                .iter()
                .filter(|node| node.role.as_deref() == Some("listitem"))
                .count()
                < 10,
            "virtualization must not build all items"
        );
    }

    #[test]
    fn virtual_collection_reveals_host_focus_by_stable_key_after_rebuild() {
        struct FocusedCollectionApp {
            offset: f32,
            reversed: bool,
        }

        impl Application for FocusedCollectionApp {
            type Message = bool;

            fn update(&mut self, reset: Self::Message) {
                if reset {
                    self.offset = 0.0;
                    self.reversed = true;
                }
            }

            fn view(&self, context: ViewContext) -> impl crate::View<Self::Message> {
                let mut items = (0..100).collect::<Vec<_>>();
                if self.reversed {
                    items.reverse();
                }
                Collection::try_new(
                    CollectionState::Ready(items),
                    |key| *key,
                    |key| Text::new(format!("Item {key}")),
                )
                .unwrap()
                .id("people")
                .on_activate(|_| true)
                .presentation(CollectionPresentation::VirtualList {
                    item_height: 20.0,
                    offset: self.offset,
                    viewport_height: 60.0,
                    overscan: 0.0,
                })
                .reveal_on_focus(&context)
            }
        }

        let mut host = UiHost::new(
            FocusedCollectionApp {
                offset: 80.0 * 20.0,
                reversed: false,
            },
            200,
            60,
        );
        let initial_nodes = host.semantic_nodes();
        let focused = initial_nodes
            .iter()
            .find(|node| node.name.as_deref() == Some("80"))
            .or_else(|| {
                initial_nodes
                    .iter()
                    .find(|node| node.id.as_str().ends_with("/80"))
            })
            .expect("initial virtual window contains stable key 80")
            .id
            .clone();
        assert!(host.request_focus(focused.clone()).changed);
        assert_eq!(host.inspect().keyboard_focus.as_ref(), Some(&focused));

        let outcome = host.perform_semantic_action(
            focused.clone(),
            SemanticAction::Invoke(ActionKind::Activate),
        );
        assert!(outcome.changed);
        assert_eq!(host.inspect().keyboard_focus.as_ref(), Some(&focused));
        assert!(host.semantic_nodes().iter().any(|node| node.id == focused));
        assert!(
            host.semantic_nodes()
                .iter()
                .filter(|node| node.role == Some(SemanticRole::ListItem))
                .count()
                < 10,
            "focus reveal must preserve bounded virtualization"
        );

        let mut controller_host = UiHost::new(
            FocusedCollectionApp {
                offset: 80.0 * 20.0,
                reversed: false,
            },
            200,
            60,
        );
        assert!(
            controller_host
                .handle_event(UiEvent::ControllerNext)
                .changed
        );
        let selected = controller_host
            .inspect()
            .controller_target
            .expect("production navigation selected a visible stable key");
        assert!(
            controller_host
                .handle_event(UiEvent::ControllerActivate)
                .changed
        );
        assert_eq!(
            controller_host.inspect().controller_target.as_ref(),
            Some(&selected)
        );
        assert!(
            controller_host
                .semantic_nodes()
                .iter()
                .any(|node| node.id == selected),
            "controller-owned selection is automatically revealed after rebuild"
        );
    }

    #[test]
    fn selected_and_disabled_contracts_are_accessible_and_noninteractive() {
        let tree = UiFrame::layout(
            collection(vec![(1, "One"), (2, "Two")])
                .selected_when(|key| *key == 2)
                .disabled_when(|key| *key == 2),
            Rect::new(0.0, 0.0, 200.0, 80.0),
        );
        let disabled = tree
            .accessibility_nodes()
            .iter()
            .find(|node| node.id.as_str().ends_with("/2"))
            .unwrap();
        assert_eq!(disabled.state.as_deref(), Some("selected, disabled"));
        assert_eq!(disabled.description.as_deref(), Some("item 2 of 2"));
        assert!(
            tree.semantic_targets_for_message(&Message::Activate(2))
                .is_empty()
        );
        assert!(
            tree.semantic_targets_for_message(&Message::Context(2))
                .is_empty()
        );
        assert!(
            !tree
                .semantic_targets_for_message(&Message::Activate(1))
                .is_empty()
        );
    }

    #[test]
    fn lifecycle_states_are_semantic_statuses() {
        for (state, expected) in [
            (CollectionState::Loading, "Loading"),
            (CollectionState::Ready(Vec::new()), "No items"),
            (CollectionState::Error("offline".into()), "Error: offline"),
        ] {
            let tree = UiFrame::layout(
                Collection::try_new(
                    state,
                    |item: &(u32, &str)| item.0,
                    |item| Text::<Message>::new(item.1),
                )
                .unwrap()
                .id("state"),
                Rect::new(0.0, 0.0, 200.0, 60.0),
            );
            assert!(tree.accessibility_nodes().iter().any(|node| {
                node.role.as_deref() == Some("status") && node.label.as_deref() == Some(expected)
            }));
        }
    }

    #[test]
    fn lifecycle_slots_accept_arbitrary_declarative_content() {
        let empty = UiFrame::layout(
            Collection::try_new(
                CollectionState::<(u32, &str)>::Ready(Vec::new()),
                |item| item.0,
                |item| Text::<Message>::new(item.1),
            )
            .unwrap()
            .empty_slot(Container::new().accessibility_label("Create the first item")),
            Rect::new(0.0, 0.0, 200.0, 60.0),
        );
        assert!(
            empty
                .accessibility_nodes()
                .iter()
                .any(|node| { node.label.as_deref() == Some("Create the first item") })
        );

        let error = UiFrame::layout(
            Collection::try_new(
                CollectionState::<(u32, &str)>::Error("offline".into()),
                |item| item.0,
                |item| Text::<Message>::new(item.1),
            )
            .unwrap()
            .error_slot(|reason| {
                Container::new()
                    .accessibility_label(format!("Retry after {reason}"))
                    .into_element()
            }),
            Rect::new(0.0, 0.0, 200.0, 60.0),
        );
        assert!(
            error
                .accessibility_nodes()
                .iter()
                .any(|node| { node.label.as_deref() == Some("Retry after offline") })
        );
    }

    #[test]
    fn virtual_scrolling_is_bounded_and_rtl_grid_reports_logical_positions() {
        let end = UiFrame::layout(
            collection((0..20).map(|id| (id, "row")).collect()).presentation(
                CollectionPresentation::VirtualList {
                    item_height: 20.0,
                    offset: f32::MAX,
                    viewport_height: 40.0,
                    overscan: 0.0,
                },
            ),
            Rect::new(0.0, 0.0, 200.0, 40.0),
        );
        assert!(
            end.accessibility_nodes()
                .iter()
                .any(|node| node.id.as_str().ends_with("/19"))
        );
        assert!(
            !end.accessibility_nodes()
                .iter()
                .any(|node| node.id.as_str().ends_with("/0"))
        );

        let rtl = UiFrame::layout(
            collection(vec![(1, "One"), (2, "Two")])
                .presentation(CollectionPresentation::UniformGrid { columns: 2 })
                .direction(ReadingDirection::RightToLeft),
            Rect::new(0.0, 0.0, 200.0, 80.0),
        );
        let first = rtl
            .accessibility_nodes()
            .iter()
            .find(|node| node.id.as_str().ends_with("/1"))
            .unwrap();
        assert_eq!(
            first.description.as_deref(),
            Some("row 1, column 2, item 1 of 2")
        );
    }

    #[test]
    fn grid_uses_production_spatial_controller_navigation() {
        let mut state = UiStateStore::default();
        let tree = UiFrame::layout_with_state(
            collection(vec![(1, "One"), (2, "Two"), (3, "Three"), (4, "Four")])
                .presentation(CollectionPresentation::UniformGrid { columns: 2 }),
            Rect::new(0.0, 0.0, 240.0, 120.0),
            &mut state,
        );
        let item = |message| {
            tree.semantic_targets_for_message(&message)
                .into_iter()
                .next()
                .unwrap()
                .id
        };
        state
            .navigation_mut()
            .set_controller_selected(Some(item(Message::Activate(1))));
        tree.handle_event(&mut state, UiEvent::ControllerRight);
        assert_eq!(
            state.navigation().controller_selected(),
            Some(&item(Message::Activate(2)))
        );
        tree.handle_event(&mut state, UiEvent::ControllerDown);
        assert_eq!(
            state.navigation().controller_selected(),
            Some(&item(Message::Activate(4)))
        );
    }
}
