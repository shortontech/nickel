use std::{collections::HashMap, fmt, ops::Range, sync::Arc};

use super::{
    AnyView, Background, Color, Column, Component, ComponentBuilderExt, Container, Element, Grid,
    NavigationScope, ReadingDirection, SemanticRole, Spacer, Text, Track, UiId, VirtualColumn,
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
    /// A fixed-row-height adaptive grid which constructs only visible complete rows.
    VirtualGrid {
        minimum_item_width: f32,
        row_height: f32,
        offset: f32,
        viewport_width: f32,
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

struct CollectionSourceInner<T, K> {
    items: Vec<(K, Arc<T>)>,
    key_positions: HashMap<K, usize>,
    id_positions: HashMap<String, usize>,
}

/// An immutable, prevalidated authority for a large keyed collection.
///
/// Construction derives every key and rejects duplicate keys and UI identity
/// strings once. Clones share that validated authority. Virtual collections
/// built from a source only clone the keys and item handles in their visible
/// window, so scrolling work is independent of the logical item count.
pub struct CollectionSource<T, K> {
    inner: Arc<CollectionSourceInner<T, K>>,
}

impl<T, K> Clone for CollectionSource<T, K> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<T, K> CollectionSource<T, K>
where
    T: 'static,
    K: Clone + Eq + std::hash::Hash + fmt::Display + 'static,
{
    pub fn try_new(items: Vec<T>, key: impl Fn(&T) -> K) -> Result<Self, CollectionError<K>> {
        let mut keyed_items = Vec::with_capacity(items.len());
        let mut key_positions = HashMap::<K, usize>::with_capacity(items.len());
        let mut id_positions = HashMap::<String, usize>::with_capacity(items.len());
        for (index, item) in items.into_iter().enumerate() {
            let item_key = key(&item);
            if let Some(first) = key_positions.insert(item_key.clone(), index) {
                return Err(CollectionError::DuplicateKey {
                    key: item_key,
                    first,
                    duplicate: index,
                });
            }
            let item_id = item_key.to_string();
            if let Some(first) = id_positions.insert(item_id.clone(), index) {
                return Err(CollectionError::DuplicateId {
                    id: item_id,
                    first,
                    duplicate: index,
                });
            }
            keyed_items.push((item_key, Arc::new(item)));
        }
        Ok(Self {
            inner: Arc::new(CollectionSourceInner {
                items: keyed_items,
                key_positions,
                id_positions,
            }),
        })
    }

    pub fn len(&self) -> usize {
        self.inner.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.items.is_empty()
    }

    /// Creates a collection declaration backed by this validated source.
    pub fn collection<Message, Render, View>(
        &self,
        render: Render,
    ) -> Collection<Arc<T>, K, Message, impl Fn(Arc<T>) -> View, View>
    where
        Render: Fn(&T) -> View,
        View: Component<Message>,
    {
        let window_inner = Arc::clone(&self.inner);
        let key_inner = Arc::clone(&self.inner);
        let id_inner = Arc::clone(&self.inner);
        Collection::from_items(
            CollectionState::Ready(Vec::new()),
            CollectionItems::Windowed {
                len: self.inner.items.len(),
                window: Box::new(move |range| {
                    window_inner.items[range]
                        .iter()
                        .map(|(key, item)| (key.clone(), Arc::clone(item)))
                        .collect()
                }),
                key_index: Box::new(move |key| key_inner.key_positions.get(key).copied()),
                id_index: Box::new(move |id| id_inner.id_positions.get(id).copied()),
            },
            move |item| render(item.as_ref()),
        )
    }
}

type CollectionWindow<T, K> = Box<dyn Fn(Range<usize>) -> Vec<(K, T)>>;
type CollectionKeyIndex<K> = Box<dyn Fn(&K) -> Option<usize>>;
type CollectionIdIndex = Box<dyn Fn(&str) -> Option<usize>>;

enum CollectionItems<T, K> {
    Owned(Vec<(K, T)>),
    Windowed {
        len: usize,
        window: CollectionWindow<T, K>,
        key_index: CollectionKeyIndex<K>,
        id_index: CollectionIdIndex,
    },
}

impl<T, K> CollectionItems<T, K> {
    fn len(&self) -> usize {
        match self {
            Self::Owned(items) => items.len(),
            Self::Windowed { len, .. } => *len,
        }
    }

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn key_index(&self, key: &K) -> Option<usize>
    where
        K: Eq,
    {
        match self {
            Self::Owned(items) => items.iter().position(|(candidate, _)| candidate == key),
            Self::Windowed { key_index, .. } => key_index(key),
        }
    }

    fn id_index(&self, target: &UiId) -> Option<usize>
    where
        K: fmt::Display,
    {
        match self {
            Self::Owned(items) => items
                .iter()
                .position(|(key, _)| target.as_str().ends_with(&format!("/{key}"))),
            Self::Windowed { id_index, .. } => {
                let path = target.as_str();
                id_index(path).or_else(|| {
                    path.match_indices('/')
                        .map(|(index, _)| &path[index + 1..])
                        .find_map(id_index)
                })
            }
        }
    }

    fn into_window(self, range: Range<usize>) -> Vec<(usize, K, T)> {
        let start = range.start;
        match self {
            Self::Owned(mut items) => items
                .drain(range)
                .enumerate()
                .map(|(offset, (key, item))| (start + offset, key, item))
                .collect(),
            Self::Windowed { window, .. } => window(range)
                .into_iter()
                .enumerate()
                .map(|(offset, (key, item))| (start + offset, key, item))
                .collect(),
        }
    }
}

type CollectionAction<K, Message> = Box<dyn Fn(&K) -> Message>;
type CollectionPredicate<K> = Box<dyn Fn(&K) -> bool>;
type CollectionItemLabel<T> = Box<dyn Fn(&T) -> String>;
type CollectionItemRevision<T> = Box<dyn Fn(&T) -> u64>;
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
    keyed_items: CollectionItems<T, K>,
    render: Render,
    presentation: CollectionPresentation,
    id: UiId,
    accessibility_label: Option<String>,
    gap: f32,
    item_focus_background_tint: Option<Color>,
    item_controller_focus_background_tint: Option<Color>,
    item_hover_background: Option<Background>,
    item_pressed_background: Option<Background>,
    item_selected_background: Option<Background>,
    navigation_scope: Option<NavigationScope>,
    controller_scope_background: Option<Background>,
    interactions: CollectionInteractions<K, Message>,
    item_label: Option<CollectionItemLabel<T>>,
    item_revision: Option<CollectionItemRevision<T>>,
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
    fn from_items(
        state: CollectionState<T>,
        keyed_items: CollectionItems<T, K>,
        render: Render,
    ) -> Self {
        Self {
            state,
            keyed_items,
            render,
            presentation: CollectionPresentation::List,
            id: UiId::from("collection"),
            accessibility_label: None,
            gap: 0.0,
            item_focus_background_tint: None,
            item_controller_focus_background_tint: None,
            item_hover_background: None,
            item_pressed_background: None,
            item_selected_background: None,
            navigation_scope: None,
            controller_scope_background: None,
            interactions: CollectionInteractions::default(),
            item_label: None,
            item_revision: None,
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
        }
    }

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
        Ok(Self::from_items(
            state,
            CollectionItems::Owned(keyed_items),
            render,
        ))
    }

    pub fn id(mut self, id: impl Into<UiId>) -> Self {
        self.id = id.into();
        self
    }

    pub fn accessibility_label(mut self, label: impl Into<String>) -> Self {
        self.accessibility_label = Some(label.into());
        self
    }

    /// Paints keyboard focus on the keyed item which owns its semantic actions.
    pub fn item_focus_background_tint(mut self, color: Color) -> Self {
        self.item_focus_background_tint = Some(color);
        self
    }

    /// Paints controller selection on the keyed item which owns its semantic actions.
    pub fn item_controller_focus_background_tint(mut self, color: Color) -> Self {
        self.item_controller_focus_background_tint = Some(color);
        self
    }

    pub fn item_interaction_backgrounds(
        mut self,
        hover: impl Into<Background>,
        pressed: impl Into<Background>,
        selected: impl Into<Background>,
    ) -> Self {
        self.item_hover_background = Some(hover.into());
        self.item_pressed_background = Some(pressed.into());
        self.item_selected_background = Some(selected.into());
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

    /// Declares the exact source-owned revision for each keyed item's visual,
    /// interactive, and semantic content. The revision must change whenever
    /// any declaration produced for that item changes.
    pub fn item_revision(mut self, revision: impl Fn(&T) -> u64 + 'static) -> Self {
        self.item_revision = Some(Box::new(revision));
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
            | CollectionPresentation::AdaptiveGrid { .. }
            | CollectionPresentation::VirtualGrid { .. } => SemanticRole::GridCell,
        };
        let total = self.keyed_items.len();
        let mut window = 0..total;
        let mut virtual_window = None;
        let mut virtual_grid_columns = None;
        let mut virtual_grid_item_width = None;
        if let CollectionPresentation::VirtualList {
            item_height,
            offset,
            viewport_height,
            overscan,
        } = self.presentation
        {
            let item_height = item_height.max(1.0);
            let mut resolved = VirtualWindow::from_uniform(
                total,
                item_height,
                self.gap,
                offset,
                viewport_height,
                overscan,
            );
            let reveal_index = self
                .reveal
                .as_ref()
                .and_then(|key| self.keyed_items.key_index(key))
                .or_else(|| {
                    self.reveal_target
                        .as_ref()
                        .and_then(|target| self.keyed_items.id_index(target))
                });
            if let Some(index) = reveal_index
                && !resolved.range.contains(&index)
            {
                let stride = item_height + self.gap;
                resolved = VirtualWindow::from_uniform(
                    total,
                    item_height,
                    self.gap,
                    index as f32 * stride,
                    viewport_height,
                    overscan,
                );
            }
            window = resolved.range.clone();
            virtual_window = Some(resolved);
        }
        if let CollectionPresentation::VirtualGrid {
            minimum_item_width,
            row_height,
            offset,
            viewport_width,
            viewport_height,
            overscan,
        } = self.presentation
        {
            let minimum = minimum_item_width.max(1.0);
            let gap = self.gap.max(0.0);
            let columns = (((viewport_width.max(0.0) + gap) / (minimum + gap)).floor() as usize)
                .max(1)
                .min(total.max(1));
            let rows = total.div_ceil(columns);
            let row_height = row_height.max(1.0);
            let mut resolved = VirtualWindow::from_uniform(
                rows,
                row_height,
                gap,
                offset,
                viewport_height,
                overscan,
            );
            let reveal_index = self
                .reveal
                .as_ref()
                .and_then(|key| self.keyed_items.key_index(key))
                .or_else(|| {
                    self.reveal_target
                        .as_ref()
                        .and_then(|target| self.keyed_items.id_index(target))
                });
            if let Some(index) = reveal_index {
                let row = index / columns;
                if !resolved.range.contains(&row) {
                    resolved = VirtualWindow::from_uniform(
                        rows,
                        row_height,
                        gap,
                        row as f32 * (row_height + gap),
                        viewport_height,
                        overscan,
                    );
                }
            }
            window = (resolved.range.start * columns)..(resolved.range.end * columns).min(total);
            virtual_window = Some(resolved);
            virtual_grid_columns = Some(columns);
            virtual_grid_item_width = Some(minimum);
        }
        let selected = &self.interactions.selected;
        let disabled = &self.interactions.disabled;
        let item_label = &self.item_label;
        let item_revision = &self.item_revision;
        let columns = match self.presentation {
            CollectionPresentation::UniformGrid { columns } => Some(columns.max(1)),
            CollectionPresentation::VirtualGrid { .. } => virtual_grid_columns,
            _ => None,
        };
        let virtual_row_height = match self.presentation {
            CollectionPresentation::VirtualList { item_height, .. } => Some(item_height.max(1.0)),
            CollectionPresentation::VirtualGrid { row_height, .. } => Some(row_height.max(1.0)),
            _ => None,
        };
        let direction = self.direction;
        let children =
            self.keyed_items
                .into_window(window)
                .into_iter()
                .map(|(index, key, item)| {
                    let content_revision = item_revision.as_ref().map(|revision| revision(&item));
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
                    if let Some(row_height) = virtual_row_height {
                        item_container = item_container.height(row_height);
                    }
                    if let Some(width) = virtual_grid_item_width {
                        item_container = item_container.width(width);
                    }
                    if let Some(color) = self.item_focus_background_tint {
                        item_container = item_container.focus_background_tint(color);
                    }
                    if let Some(color) = self.item_controller_focus_background_tint {
                        item_container = item_container.controller_focus_background_tint(color);
                    }
                    if let (Some(hover), Some(pressed)) =
                        (self.item_hover_background, self.item_pressed_background)
                    {
                        item_container = item_container.interaction_backgrounds(hover, pressed);
                    }
                    if is_selected && let Some(background) = self.item_selected_background {
                        item_container = item_container.background(background);
                    }
                    let mut element = item_container.into_element();
                    if let Some(revision) = content_revision {
                        element = element.content_revision(revision);
                    }
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
            CollectionPresentation::VirtualGrid { .. } => {
                let window = virtual_window.expect("virtual grid constructs a window");
                Column::new()
                    .fill_width()
                    .child(Spacer::vertical(window.leading))
                    .child(
                        Grid::new()
                            .columns(vec![
                                Track::px(
                                    virtual_grid_item_width
                                        .expect("virtual grid resolves item width"),
                                );
                                virtual_grid_columns
                                    .expect("virtual grid resolves columns")
                            ])
                            .gap(self.gap)
                            .children(children),
                    )
                    .child(Spacer::vertical(window.trailing))
                    .into_element()
                    .id(self.id)
                    .semantic_role(SemanticRole::Grid)
            }
        };
        element.navigation_scope = self.navigation_scope;
        element.style.controller_scope_background = self.controller_scope_background;
        if let Some(label) = self.accessibility_label {
            element = element.accessibility_label(label);
        }
        element
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, rc::Rc};

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
    fn collection_source_validates_keys_and_ids_once() {
        let key_calls = Rc::new(Cell::new(0));
        let counted = Rc::clone(&key_calls);
        let source = CollectionSource::try_new((0_u32..100).collect(), move |item| {
            counted.set(counted.get() + 1);
            *item
        })
        .unwrap();
        assert_eq!(key_calls.get(), 100);

        for offset in [0.0, 20.0, 40.0] {
            let tree = UiFrame::layout(
                source
                    .collection(|item| Text::<Message>::new(item.to_string()))
                    .presentation(CollectionPresentation::VirtualList {
                        item_height: 20.0,
                        offset,
                        viewport_height: 20.0,
                        overscan: 0.0,
                    }),
                Rect::new(0.0, 0.0, 200.0, 20.0),
            );
            assert!(
                tree.semantic_nodes()
                    .iter()
                    .filter(|node| node.role == Some(SemanticRole::ListItem))
                    .count()
                    <= 3
            );
        }
        let revealed = UiFrame::layout(
            source
                .collection(|item| Text::<Message>::new(item.to_string()))
                .presentation(CollectionPresentation::VirtualList {
                    item_height: 20.0,
                    offset: 0.0,
                    viewport_height: 20.0,
                    overscan: 0.0,
                })
                .reveal(80),
            Rect::new(0.0, 0.0, 200.0, 20.0),
        );
        assert!(
            revealed
                .semantic_nodes()
                .iter()
                .any(|node| node.id.as_str().ends_with("/80"))
        );
        assert_eq!(
            key_calls.get(),
            100,
            "scrolling must not revalidate the source"
        );
    }

    #[test]
    fn collection_source_bounds_virtual_grid_declaration() {
        let rendered = Rc::new(Cell::new(0));
        let counted = Rc::clone(&rendered);
        let source = CollectionSource::try_new((0_u32..4096).collect(), |item| *item).unwrap();
        let tree = UiFrame::layout(
            source
                .collection(move |item| {
                    counted.set(counted.get() + 1);
                    Text::<Message>::new(item.to_string())
                })
                .gap(10.0)
                .presentation(CollectionPresentation::VirtualGrid {
                    minimum_item_width: 100.0,
                    row_height: 100.0,
                    offset: 2_200.0,
                    viewport_width: 430.0,
                    viewport_height: 250.0,
                    overscan: 100.0,
                }),
            Rect::new(0.0, 0.0, 430.0, 250.0),
        );
        assert!(rendered.get() <= 24, "rendered {} items", rendered.get());
        assert_eq!(rendered.get() % 4, 0);
        assert_eq!(
            tree.semantic_nodes()
                .iter()
                .filter(|node| node.role == Some(SemanticRole::GridCell))
                .count(),
            rendered.get()
        );
    }

    #[test]
    fn collection_source_preserves_duplicate_diagnostics() {
        let duplicate_key = CollectionSource::try_new(vec![(7_u32, "a"), (7, "b")], |item| item.0);
        assert!(matches!(
            duplicate_key,
            Err(CollectionError::DuplicateKey {
                key: 7,
                first: 0,
                duplicate: 1
            })
        ));

        #[derive(Clone, Debug, Eq, PartialEq, Hash)]
        struct SameId(u32);
        impl fmt::Display for SameId {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("same")
            }
        }
        let duplicate_id = CollectionSource::try_new(vec![1_u32, 2], |item| SameId(*item));
        assert!(matches!(
            duplicate_id,
            Err(CollectionError::DuplicateId {
                ref id,
                first: 0,
                duplicate: 1
            }) if id == "same"
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
    fn virtual_grid_constructs_complete_visible_rows_and_preserves_extent() {
        let rendered = Rc::new(Cell::new(0));
        let rendered_items = rendered.clone();
        let collection = Collection::try_new(
            CollectionState::Ready((0_u32..4096).collect()),
            |item| *item,
            move |item| {
                rendered_items.set(rendered_items.get() + 1);
                Text::new(format!("Item {item}"))
            },
        )
        .unwrap()
        .id("files")
        .gap(10.0)
        .presentation(CollectionPresentation::VirtualGrid {
            minimum_item_width: 100.0,
            row_height: 100.0,
            offset: 2_200.0,
            viewport_width: 430.0,
            viewport_height: 250.0,
            overscan: 100.0,
        });
        let tree = UiFrame::layout(
            crate::VerticalScroll::new(Message::Activate(0), 2_200.0)
                .controlled(true)
                .child(collection),
            Rect::new(0.0, 0.0, 430.0, 250.0),
        );

        assert!(rendered.get() <= 24, "rendered {} items", rendered.get());
        assert_eq!(rendered.get() % 4, 0, "windows contain complete rows");
        assert_eq!(
            tree.semantic_nodes()
                .iter()
                .filter(|node| node.role == Some(SemanticRole::GridCell))
                .count(),
            rendered.get()
        );
        let extent = tree
            .scroll_extent(&Message::Activate(0))
            .expect("virtual grid retains a real scroll extent");
        assert_eq!(extent.content.height, 112_630.0);
        assert_eq!(extent.offset, 2_200.0);

        let first_cell_id = tree
            .semantic_nodes()
            .iter()
            .find(|node| node.role == Some(SemanticRole::GridCell))
            .expect("visible grid cell has semantics")
            .id
            .clone();
        let first = tree
            .resolved_layout()
            .nodes()
            .iter()
            .find(|node| node.id == first_cell_id)
            .expect("first visible grid cell has layout");
        assert_eq!(first.allocated.size.width, 100.0);
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

    #[cfg(not(debug_assertions))]
    #[test]
    #[ignore = "release-profile 10,000-item virtual-list admission benchmark"]
    fn virtual_list_10000_release_admission_bounds_native_work() {
        use crate::{FrameRequest, release_admission::AdmissionReport};
        use std::time::Instant;

        const ITEMS: usize = 10_000;
        const ITEM_HEIGHT: f32 = 24.0;
        const VIEWPORT_HEIGHT: f32 = 480.0;
        const OVERSCAN: f32 = 48.0;
        const FIRST_ITEM: usize = 4_000;
        const SAMPLES: usize = 20;
        const ITERATIONS_PER_SAMPLE: usize = 2;

        fn view(
            source: &CollectionSource<usize, usize>,
            offset: f32,
            rendered: Rc<Cell<usize>>,
        ) -> Element<()> {
            source
                .collection(move |item| {
                    rendered.set(rendered.get() + 1);
                    Text::<()>::new(format!("Virtual item {item:05}"))
                })
                .id("virtual-admission")
                .item_revision(|item| **item as u64)
                .presentation(CollectionPresentation::VirtualList {
                    item_height: ITEM_HEIGHT,
                    offset,
                    viewport_height: VIEWPORT_HEIGHT,
                    overscan: OVERSCAN,
                })
                .into_element()
        }

        fn assert_oracle(incremental: &UiFrame<()>, cold: &UiFrame<()>) {
            assert_eq!(incremental.resolved_layout(), cold.resolved_layout());
            assert_eq!(incremental.commands(), cold.commands());
            assert_eq!(incremental.semantic_nodes(), cold.semantic_nodes());
            assert_eq!(
                incremental.accessibility_nodes(),
                cold.accessibility_nodes()
            );
            assert_eq!(
                incremental.interaction_record_counts(),
                cold.interaction_record_counts()
            );
        }

        let bounds = Rect::new(0.0, 0.0, 900.0, VIEWPORT_HEIGHT);
        let offsets = [
            FIRST_ITEM as f32 * ITEM_HEIGHT,
            (FIRST_ITEM + 1) as f32 * ITEM_HEIGHT,
        ];
        let key_derivations = Rc::new(Cell::new(0));
        let counted_derivations = Rc::clone(&key_derivations);
        let source = CollectionSource::try_new((0..ITEMS).collect(), move |item| {
            counted_derivations.set(counted_derivations.get() + 1);
            *item
        })
        .expect("10,000 stable integer keys are unique");
        assert_eq!(key_derivations.get(), ITEMS);
        let initial_rendered = Rc::new(Cell::new(0));
        let mut retained_state = UiStateStore::default();
        let mut retained = UiFrame::resolve(
            view(&source, offsets[0], initial_rendered.clone()),
            FrameRequest::new(bounds, &mut retained_state),
        );
        let visible_items = initial_rendered.get();
        assert!(
            visible_items <= 32,
            "virtual list constructed {visible_items} of {ITEMS} logical items"
        );

        let mut scroll_samples = Vec::with_capacity(SAMPLES * ITERATIONS_PER_SAMPLE);
        let mut cold_samples = Vec::with_capacity(SAMPLES);
        let mut expected_work = None;
        let mut expected_rendered = None;
        for sample in 0..SAMPLES {
            for iteration in 0..ITERATIONS_PER_SAMPLE {
                let offset = offsets[(sample * ITERATIONS_PER_SAMPLE + iteration + 1) % 2];
                let rendered = Rc::new(Cell::new(0));
                let started = Instant::now();
                let next = UiFrame::resolve_against(
                    view(&source, offset, rendered.clone()),
                    FrameRequest::new(bounds, &mut retained_state),
                    &retained,
                );
                scroll_samples.push(started.elapsed());

                let work = next.resource_diagnostics();
                expected_work.get_or_insert(work);
                assert_eq!(expected_work, Some(work));
                expected_rendered.get_or_insert(rendered.get());
                assert_eq!(expected_rendered, Some(rendered.get()));
                assert!(rendered.get() <= 32);
                retained = next;
            }
        }

        for sample in 0..SAMPLES {
            let offset = offsets[sample % offsets.len()];
            let oracle_rendered = Rc::new(Cell::new(0));
            let incremental = UiFrame::resolve_against(
                view(&source, offset, oracle_rendered),
                FrameRequest::new(bounds, &mut retained_state),
                &retained,
            );
            let rendered = Rc::new(Cell::new(0));
            let mut cold_state = UiStateStore::default();
            let started = Instant::now();
            let cold = UiFrame::resolve(
                view(&source, offset, rendered.clone()),
                FrameRequest::new(bounds, &mut cold_state),
            );
            cold_samples.push(started.elapsed());
            assert_eq!(rendered.get(), expected_rendered.expect("retained sample"));
            assert_oracle(&incremental, &cold);
            retained = incremental;
        }
        assert_eq!(
            key_derivations.get(),
            ITEMS,
            "all measured declarations must reuse the validated source authority"
        );

        let work = expected_work.expect("retained work sample");
        assert_eq!(expected_rendered, Some(26));
        assert_eq!(work.retained_node_count, 56);
        assert_eq!(work.retained_nodes_reused, 54);
        assert_eq!(work.retained_nodes_created, 2);
        assert_eq!(work.retained_nodes_removed, 2);
        assert_eq!(work.retained_nodes_moved, 25);
        assert_eq!(work.nodes_measured, 6);
        assert_eq!(work.nodes_placed, 6);
        assert_eq!(work.paint_nodes_executed, 6);
        assert_eq!(work.paint_nodes_reused, 50);
        assert_eq!(work.interaction_nodes_executed, 6);
        assert_eq!(work.interaction_nodes_reused, 50);
        assert_eq!(work.semantic_nodes_executed, 56);
        assert_eq!(work.semantic_nodes_reused, 0);
        AdmissionReport::new("retained_virtual_list", "10000_item_one_row_scroll")
            .metadata("items", ITEMS)
            .metadata("visible_items", expected_rendered.expect("rendered sample"))
            .metadata("samples", SAMPLES)
            .metadata("iterations_per_sample", ITERATIONS_PER_SAMPLE)
            .work("logical_items", ITEMS)
            .work("source_key_derivations_once", key_derivations.get())
            .work("source_key_derivations_per_scroll", 0)
            .work(
                "visible_items_constructed",
                expected_rendered.expect("rendered sample"),
            )
            .work("retained_node_count", work.retained_node_count)
            .work("retained_nodes_reused", work.retained_nodes_reused)
            .work("retained_nodes_created", work.retained_nodes_created)
            .work("retained_nodes_removed", work.retained_nodes_removed)
            .work("retained_nodes_moved", work.retained_nodes_moved)
            .work("nodes_measured", work.nodes_measured)
            .work("nodes_placed", work.nodes_placed)
            .work("paint_nodes_executed", work.paint_nodes_executed)
            .work("paint_nodes_reused", work.paint_nodes_reused)
            .work(
                "interaction_nodes_executed",
                work.interaction_nodes_executed,
            )
            .work("interaction_nodes_reused", work.interaction_nodes_reused)
            .work("semantic_nodes_executed", work.semantic_nodes_executed)
            .work("semantic_nodes_reused", work.semantic_nodes_reused)
            .timings("one_row_scroll", &scroll_samples)
            .timings("cold", &cold_samples)
            .emit();
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
