use std::{collections::HashMap, fmt, hash::Hash};

use crate::{
    AnyView, Column, Component, ComponentBuilderExt, Container, Insets, NavigationItem,
    NavigationScope, Overflow, ReadingDirection, Row, SemanticRole, SemanticTheme, UiId,
};

/// Default width at which navigation and destination content are shown together.
pub const RESPONSIVE_NAVIGATION_BREAKPOINT: f32 = 720.0;

/// The presentation selected by [`ResponsiveNavigation`] for the current width.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResponsiveNavigationPresentation {
    Wide,
    NarrowNavigation,
    NarrowDetail,
}

/// A construction error which would make destination identity ambiguous.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResponsiveNavigationError<K> {
    DuplicateDestinationKey {
        key: K,
        first: usize,
        duplicate: usize,
    },
    DuplicateDestinationId {
        id: String,
        first: usize,
        duplicate: usize,
    },
    UnknownActiveDestination(K),
}

impl<K: fmt::Display> fmt::Display for ResponsiveNavigationError<K> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateDestinationKey {
                key,
                first,
                duplicate,
            } => write!(
                formatter,
                "duplicate navigation destination key `{key}` at item {duplicate} (first used at item {first})"
            ),
            Self::DuplicateDestinationId {
                id,
                first,
                duplicate,
            } => write!(
                formatter,
                "navigation destination keys produce duplicate UI id `{id}` at item {duplicate} (first used at item {first})"
            ),
            Self::UnknownActiveDestination(key) => {
                write!(
                    formatter,
                    "active navigation destination `{key}` does not exist"
                )
            }
        }
    }
}

impl<K: fmt::Debug + fmt::Display> std::error::Error for ResponsiveNavigationError<K> {}

/// One stable, keyed destination and its typed activation message.
pub struct ResponsiveNavigationDestination<K, Message> {
    key: K,
    label: String,
    select: Message,
    header: Option<AnyView<Message>>,
    detail: AnyView<Message>,
    footer: Option<AnyView<Message>>,
    leading: Option<AnyView<Message>>,
    section: Option<String>,
    visible: bool,
}

impl<K, Message> ResponsiveNavigationDestination<K, Message> {
    pub fn new(
        key: K,
        label: impl Into<String>,
        select: Message,
        detail: impl Component<Message>,
    ) -> Self {
        Self {
            key,
            label: label.into(),
            select,
            header: None,
            detail: AnyView::new(detail),
            footer: None,
            leading: None,
            section: None,
            visible: true,
        }
    }

    pub fn header(mut self, header: impl Component<Message>) -> Self {
        self.header = Some(AnyView::new(header));
        self
    }

    pub fn footer(mut self, footer: impl Component<Message>) -> Self {
        self.footer = Some(AnyView::new(footer));
        self
    }

    pub fn leading(mut self, leading: impl Component<Message>) -> Self {
        self.leading = Some(AnyView::new(leading));
        self
    }

    pub fn section(mut self, label: impl Into<String>) -> Self {
        self.section = Some(label.into());
        self
    }

    pub fn visible(mut self, visible: bool) -> Self {
        self.visible = visible;
        self
    }
}

/// Declarative master/detail navigation whose app-owned state is only the
/// active destination key and the typed messages which update it.
///
/// A missing active key means the navigation level on narrow widths. Selecting
/// a destination emits its ordinary UI-tree message; after the app reducer
/// supplies that key, the component presents its detail. On wide widths both
/// levels are present and the same key only controls selection and content.
pub struct ResponsiveNavigation<K, Message> {
    theme: SemanticTheme,
    width: f32,
    breakpoint: f32,
    active: Option<K>,
    destinations: Vec<ResponsiveNavigationDestination<K, Message>>,
    direction: ReadingDirection,
    id: UiId,
    navigation_header: Option<AnyView<Message>>,
    navigation_width: f32,
}

impl<K, Message> ResponsiveNavigation<K, Message>
where
    K: Clone + Eq + Hash + fmt::Display,
{
    pub fn try_new(
        theme: SemanticTheme,
        width: f32,
        active: Option<K>,
        destinations: impl IntoIterator<Item = ResponsiveNavigationDestination<K, Message>>,
    ) -> Result<Self, ResponsiveNavigationError<K>> {
        let destinations = destinations.into_iter().collect::<Vec<_>>();
        let mut keys = HashMap::<K, usize>::new();
        let mut ids = HashMap::<String, usize>::new();
        for (index, destination) in destinations.iter().enumerate() {
            if let Some(first) = keys.insert(destination.key.clone(), index) {
                return Err(ResponsiveNavigationError::DuplicateDestinationKey {
                    key: destination.key.clone(),
                    first,
                    duplicate: index,
                });
            }
            let id = destination.key.to_string();
            if let Some(first) = ids.insert(id.clone(), index) {
                return Err(ResponsiveNavigationError::DuplicateDestinationId {
                    id,
                    first,
                    duplicate: index,
                });
            }
        }
        if let Some(active) = &active
            && !keys.contains_key(active)
        {
            return Err(ResponsiveNavigationError::UnknownActiveDestination(
                active.clone(),
            ));
        }
        Ok(Self {
            theme,
            width,
            breakpoint: RESPONSIVE_NAVIGATION_BREAKPOINT,
            active,
            destinations,
            direction: ReadingDirection::LeftToRight,
            id: UiId::from("responsive-navigation"),
            navigation_header: None,
            navigation_width: 260.0,
        })
    }

    pub fn breakpoint(mut self, breakpoint: f32) -> Self {
        self.breakpoint = breakpoint.max(0.0);
        self
    }

    pub fn direction(mut self, direction: ReadingDirection) -> Self {
        self.direction = direction;
        self
    }

    pub fn id(mut self, id: impl Into<UiId>) -> Self {
        self.id = id.into();
        self
    }

    pub fn navigation_header(mut self, header: impl Component<Message>) -> Self {
        self.navigation_header = Some(AnyView::new(header));
        self
    }

    pub fn navigation_width(mut self, width: f32) -> Self {
        self.navigation_width = width.max(0.0);
        self
    }

    pub fn presentation(&self) -> ResponsiveNavigationPresentation {
        if self.width >= self.breakpoint {
            ResponsiveNavigationPresentation::Wide
        } else if self.active.is_some() {
            ResponsiveNavigationPresentation::NarrowDetail
        } else {
            ResponsiveNavigationPresentation::NarrowNavigation
        }
    }
}

impl<K, Message> Component<Message> for ResponsiveNavigation<K, Message>
where
    K: Clone + Eq + Hash + fmt::Display,
{
    fn into_element(self) -> super::Element<Message> {
        let presentation = self.presentation();
        let root_id = self.id.as_str().to_owned();
        let active = self.active.as_ref();
        let mut navigation = Column::new()
            .id(format!("{root_id}/destinations"))
            .fill_height()
            .gap(self.theme.spacing.compact)
            .padding(Insets::all(self.theme.spacing.content))
            .overflow_y(Overflow::Auto)
            .background(self.theme.surfaces.sidebar)
            .semantic_role(SemanticRole::List)
            .navigation_scope(NavigationScope::pane(active.is_none()))
            .navigation_scope_highlight(self.theme.borders.controller_focus);
        if let Some(header) = self.navigation_header {
            navigation = navigation.child(header);
        }
        let mut selected_detail = None;
        for destination in self.destinations {
            let selected = active == Some(&destination.key);
            let destination_id = format!("{root_id}/destination/{}", destination.key);
            if destination.visible {
                if let Some(section) = destination.section {
                    navigation = navigation.child(
                        Container::new()
                            .min_height(21.0)
                            .child(crate::Text::new(section).color(self.theme.text.secondary)),
                    );
                }
                let item = if let Some(leading) = destination.leading {
                    NavigationItem::with_leading(
                        self.theme,
                        destination.select,
                        &destination.label,
                        selected,
                        leading,
                    )
                } else {
                    NavigationItem::new(
                        self.theme,
                        destination.select,
                        &destination.label,
                        selected,
                    )
                };
                navigation = navigation.child(item.id(destination_id).direction(self.direction));
            }
            if selected {
                selected_detail = Some((
                    destination.key,
                    destination.header,
                    destination.detail,
                    destination.footer,
                ));
            }
        }

        let detail = selected_detail.map(|(key, header, detail, footer)| {
            let mut content = Column::new()
                .id(format!("{root_id}/detail/{key}"))
                .fill_width()
                .fill_height()
                .background(self.theme.surfaces.window)
                .semantic_role(SemanticRole::TabPanel)
                .navigation_scope(NavigationScope::pane(true))
                .navigation_scope_highlight(self.theme.borders.controller_focus);
            if let Some(header) = header {
                content = content.child(header);
            }
            content = content.child(Container::new().grow(1.0).child(detail));
            if let Some(footer) = footer {
                content = content.child(footer);
            }
            content
        });

        let root = match presentation {
            ResponsiveNavigationPresentation::Wide => {
                let navigation = navigation
                    .width(self.navigation_width)
                    .min_width(self.navigation_width);
                let detail = detail
                    .map(AnyView::new)
                    .unwrap_or_else(|| AnyView::new(Container::new().fill_width().fill_height()));
                let row = Row::new()
                    .id("presentation")
                    .fill_width()
                    .fill_height()
                    .child(navigation)
                    .child(detail);
                if self.direction == ReadingDirection::RightToLeft {
                    row.reverse().into_element()
                } else {
                    row.into_element()
                }
            }
            ResponsiveNavigationPresentation::NarrowNavigation => Container::new()
                .id("presentation")
                .fill_width()
                .fill_height()
                .child(navigation)
                .into_element(),
            ResponsiveNavigationPresentation::NarrowDetail => Container::new()
                .id("presentation")
                .fill_width()
                .fill_height()
                .child(detail.expect("validated active destination must have detail"))
                .into_element(),
        };
        Container::new()
            .id(self.id)
            .fill_width()
            .fill_height()
            .child(root)
            .into_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Point, Rect, Text, UiEvent, UiFrame, UiStateStore};

    #[derive(Clone, Debug, Eq, PartialEq)]
    enum Message {
        Select(&'static str),
    }

    fn theme() -> SemanticTheme {
        SemanticTheme::from_tokens(crate::SemanticTokenSet::standard(
            0x101114, 0x15171b, 0x1b1e23, 0x32363e, 0x3c414b, 0xf2f3f5, 0xa8abb2, 0x9b62e8,
            0x45305f, 0x55b982, 0x55b982,
        ))
    }

    fn destinations() -> Vec<ResponsiveNavigationDestination<&'static str, Message>> {
        vec![
            ResponsiveNavigationDestination::new(
                "home",
                "Home",
                Message::Select("home"),
                Text::new("Home detail"),
            )
            .header(Text::new("Home header"))
            .footer(Text::new("Home footer")),
            ResponsiveNavigationDestination::new(
                "search",
                "Search",
                Message::Select("search"),
                Text::new("Search detail"),
            ),
        ]
    }

    fn navigation(
        width: f32,
        active: Option<&'static str>,
    ) -> ResponsiveNavigation<&'static str, Message> {
        ResponsiveNavigation::try_new(theme(), width, active, destinations()).unwrap()
    }

    #[test]
    fn wide_and_narrow_presentations_derive_from_width_and_active_key() {
        assert_eq!(
            navigation(900.0, Some("home")).presentation(),
            ResponsiveNavigationPresentation::Wide
        );
        assert_eq!(
            navigation(480.0, None).presentation(),
            ResponsiveNavigationPresentation::NarrowNavigation
        );
        assert_eq!(
            navigation(480.0, Some("home")).presentation(),
            ResponsiveNavigationPresentation::NarrowDetail
        );
    }

    #[test]
    fn resize_rebuild_preserves_destination_identity_from_app_active_key() {
        let wide = UiFrame::layout(
            navigation(900.0, Some("search")),
            Rect::new(0.0, 0.0, 900.0, 600.0),
        );
        let narrow = UiFrame::layout(
            navigation(480.0, Some("search")),
            Rect::new(0.0, 0.0, 480.0, 600.0),
        );
        let wide_detail = wide
            .semantic_nodes()
            .into_iter()
            .find(|node| node.role == Some(SemanticRole::TabPanel))
            .unwrap();
        let narrow_detail = narrow
            .semantic_nodes()
            .into_iter()
            .find(|node| node.role == Some(SemanticRole::TabPanel))
            .unwrap();
        assert_eq!(wide_detail.id, narrow_detail.id);
    }

    #[test]
    fn keyboard_selection_uses_typed_production_activation() {
        let tree = UiFrame::layout(navigation(480.0, None), Rect::new(0.0, 0.0, 480.0, 600.0));
        let mut state = UiStateStore::default();
        tree.reconcile_state(&mut state);
        tree.handle_event(&mut state, UiEvent::FocusNext);
        assert_eq!(
            tree.handle_event(&mut state, UiEvent::KeyboardActivate)
                .messages,
            vec![Message::Select("home")]
        );

        let bounds = tree
            .semantic_targets_for_message(&Message::Select("search"))
            .into_iter()
            .next()
            .unwrap()
            .bounds;
        let point = Point {
            x: bounds.origin.x + bounds.size.width / 2.0,
            y: bounds.origin.y + bounds.size.height / 2.0,
        };
        tree.handle_event(&mut state, UiEvent::PointerPressed(point));
        assert_eq!(
            tree.handle_event(&mut state, UiEvent::PointerReleased(point))
                .messages,
            vec![Message::Select("search")]
        );
    }

    #[test]
    fn rtl_places_wide_navigation_on_the_trailing_side() {
        let ltr = UiFrame::layout(
            navigation(900.0, Some("home")),
            Rect::new(0.0, 0.0, 900.0, 600.0),
        );
        let rtl = UiFrame::layout(
            navigation(900.0, Some("home")).direction(ReadingDirection::RightToLeft),
            Rect::new(0.0, 0.0, 900.0, 600.0),
        );
        let navigation_bounds = |tree: &UiFrame<Message>| {
            tree.semantic_nodes()
                .into_iter()
                .find(|node| {
                    node.id.as_str().ends_with("/destinations")
                        && node.role == Some(SemanticRole::List)
                })
                .unwrap()
                .bounds
        };
        let ltr_x = navigation_bounds(&ltr).origin.x;
        let rtl_x = navigation_bounds(&rtl).origin.x;
        assert!(rtl_x > ltr_x);
    }

    #[test]
    fn duplicate_destination_keys_are_rejected() {
        let result = ResponsiveNavigation::try_new(
            theme(),
            900.0,
            Some("same"),
            vec![
                ResponsiveNavigationDestination::new(
                    "same",
                    "One",
                    Message::Select("same"),
                    Text::new("One"),
                ),
                ResponsiveNavigationDestination::new(
                    "same",
                    "Two",
                    Message::Select("same"),
                    Text::new("Two"),
                ),
            ],
        );
        assert!(matches!(
            result,
            Err(ResponsiveNavigationError::DuplicateDestinationKey {
                first: 0,
                duplicate: 1,
                ..
            })
        ));
    }
}
