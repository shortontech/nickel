//! Generic presentation primitives shared by application surfaces.
//!
//! These types retain declarative elements, never paint commands, so layout,
//! semantics, hit testing, and controller focus continue through `UiFrame`.

use crate::ui::Element;
use crate::{
    Component, ComponentBuilderExt, Container, Insets, Row, SemanticRole, SemanticTheme, UiId,
};

/// Declares both artwork paths while retaining only the selected declarative element.
pub struct ArtworkPresentation<Message = String> {
    selected: Element<Message>,
    used_fallback: bool,
}

impl<Message> ArtworkPresentation<Message> {
    pub fn new(
        artwork_available: bool,
        artwork: impl Component<Message>,
        fallback: impl Component<Message>,
    ) -> Self {
        let (selected, used_fallback) = if artwork_available {
            (artwork.into_element(), false)
        } else {
            (fallback.into_element(), true)
        };
        Self {
            selected,
            used_fallback,
        }
    }

    pub fn used_fallback(&self) -> bool {
        self.used_fallback
    }
}

/// A reusable item row with optional leading and trailing presentation.
pub struct ItemPresentation<Message = String> {
    id: UiId,
    children: Vec<Element<Message>>,
    selected: bool,
    disabled: bool,
    theme: SemanticTheme,
    activate: Option<Message>,
    context: Option<Message>,
}

impl<Message> ItemPresentation<Message> {
    pub fn new(id: impl Into<UiId>, body: impl Component<Message>, theme: SemanticTheme) -> Self {
        Self {
            id: id.into(),
            children: vec![body.into_element()],
            selected: false,
            disabled: false,
            theme,
            activate: None,
            context: None,
        }
    }

    pub fn leading(mut self, view: impl Component<Message>) -> Self {
        self.children.insert(0, view.into_element());
        self
    }

    pub fn artwork(mut self, artwork: ArtworkPresentation<Message>) -> Self {
        self.children.insert(0, artwork.selected);
        self
    }

    pub fn trailing(mut self, view: impl Component<Message>) -> Self {
        self.children.push(view.into_element());
        self
    }

    pub fn supporting(self, view: impl Component<Message>) -> Self {
        self.trailing(view)
    }

    pub fn badge(self, view: impl Component<Message>) -> Self {
        self.trailing(view)
    }

    pub fn on_activate(mut self, message: Message) -> Self {
        self.activate = Some(message);
        self
    }

    pub fn on_context(mut self, message: Message) -> Self {
        self.context = Some(message);
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

impl<Message> Component<Message> for ItemPresentation<Message> {
    fn into_element(self) -> Element<Message> {
        let mut element = Row::new()
            .id(self.id)
            .semantic_role(SemanticRole::ListItem)
            .accessibility_state(match (self.selected, self.disabled) {
                (true, true) => "selected, disabled",
                (true, false) => "selected",
                (false, true) => "disabled",
                (false, false) => "unselected",
            })
            .background(if self.selected {
                self.theme.surfaces.selected
            } else {
                self.theme.surfaces.card
            })
            .padding(Insets::all(self.theme.spacing.control))
            .gap(self.theme.spacing.control)
            .children(self.children)
            .into_element();
        if !self.disabled {
            if let Some(message) = self.activate {
                element = element.message(message);
            }
            if let Some(message) = self.context {
                element = element.context_message(message);
            }
        }
        element
    }
}

/// Named slots for a conventional application surface.
pub struct SurfaceScaffold<Message = String> {
    id: UiId,
    header: Option<Element<Message>>,
    sidebar: Option<Element<Message>>,
    content: Element<Message>,
    actions: Option<Element<Message>>,
    footer: Option<Element<Message>>,
}

impl<Message> SurfaceScaffold<Message> {
    pub fn new(id: impl Into<UiId>, content: impl Component<Message>) -> Self {
        Self {
            id: id.into(),
            header: None,
            sidebar: None,
            content: content.into_element(),
            actions: None,
            footer: None,
        }
    }

    pub fn header(mut self, header: impl Component<Message>) -> Self {
        self.header = Some(header.into_element().accessibility_role("banner"));
        self
    }

    pub fn sidebar(mut self, sidebar: impl Component<Message>) -> Self {
        self.sidebar = Some(sidebar.into_element().accessibility_role("navigation"));
        self
    }

    pub fn actions(mut self, actions: impl Component<Message>) -> Self {
        self.actions = Some(actions.into_element().accessibility_role("group"));
        self
    }

    pub fn footer(mut self, footer: impl Component<Message>) -> Self {
        self.footer = Some(footer.into_element().accessibility_role("contentinfo"));
        self
    }
}

impl<Message> Component<Message> for SurfaceScaffold<Message> {
    fn into_element(self) -> Element<Message> {
        let body = Row::new().children(
            self.sidebar
                .into_iter()
                .chain(std::iter::once(self.content)),
        );
        Container::new()
            .id(self.id)
            .children(
                self.header
                    .into_iter()
                    .chain(std::iter::once(body.into_element()))
                    .chain(self.actions)
                    .chain(self.footer),
            )
            .into_element()
    }
}

macro_rules! region_primitive {
    ($name:ident, $role:literal, $semantic:expr) => {
        pub struct $name<Message = String>(Element<Message>);
        impl<Message> $name<Message> {
            pub fn new(id: impl Into<UiId>) -> Self {
                let id = id.into();
                Self(
                    Container::new()
                        .id(id.clone())
                        .semantic_role($semantic)
                        .into_element()
                        .accessibility_label(id.as_str())
                        .accessibility_role($role),
                )
            }
            pub fn child(mut self, child: impl Component<Message>) -> Self {
                self.0 = self.0.child(child);
                self
            }
        }
        impl<Message> Component<Message> for $name<Message> {
            fn into_element(self) -> Element<Message> {
                self.0
            }
        }
    };
}

region_primitive!(StatusRegion, "status", SemanticRole::Status);

struct OrderedRegion<Message> {
    id: UiId,
    role: &'static str,
    children: Vec<Element<Message>>,
    direction: crate::ReadingDirection,
    maximum_visible: Option<usize>,
    overflow: Option<Element<Message>>,
}

impl<Message> OrderedRegion<Message> {
    fn new(id: impl Into<UiId>, role: &'static str) -> Self {
        Self {
            id: id.into(),
            role,
            children: Vec::new(),
            direction: crate::ReadingDirection::LeftToRight,
            maximum_visible: None,
            overflow: None,
        }
    }
    fn child(mut self, child: impl Component<Message>) -> Self {
        self.children.push(child.into_element());
        self
    }
    fn direction(mut self, direction: crate::ReadingDirection) -> Self {
        self.direction = direction;
        self
    }
    fn overflow(mut self, maximum_visible: usize, overflow: impl Component<Message>) -> Self {
        self.maximum_visible = Some(maximum_visible);
        self.overflow = Some(overflow.into_element());
        self
    }
    fn into_element(mut self) -> Element<Message> {
        if self.direction == crate::ReadingDirection::RightToLeft {
            self.children.reverse();
        }
        if let Some(maximum) = self.maximum_visible
            && self.children.len() > maximum
        {
            self.children.truncate(maximum.saturating_sub(1));
            if maximum > 0 {
                self.children.extend(self.overflow);
            }
        }
        Row::new()
            .id(self.id.clone())
            .semantic_role(SemanticRole::List)
            .children(self.children)
            .into_element()
            .accessibility_label(self.id.as_str())
            .accessibility_role(self.role)
    }
}

macro_rules! ordered_region {
    ($name:ident, $role:literal) => {
        pub struct $name<Message = String>(OrderedRegion<Message>);
        impl<Message> $name<Message> {
            pub fn new(id: impl Into<UiId>) -> Self {
                Self(OrderedRegion::new(id, $role))
            }
            pub fn child(mut self, child: impl Component<Message>) -> Self {
                self.0 = self.0.child(child);
                self
            }
            pub fn direction(mut self, direction: crate::ReadingDirection) -> Self {
                self.0 = self.0.direction(direction);
                self
            }
            pub fn overflow(
                mut self,
                maximum_visible: usize,
                overflow: impl Component<Message>,
            ) -> Self {
                self.0 = self.0.overflow(maximum_visible, overflow);
                self
            }
        }
        impl<Message> Component<Message> for $name<Message> {
            fn into_element(self) -> Element<Message> {
                self.0.into_element()
            }
        }
    };
}

ordered_region!(ToolRegion, "toolbar");
ordered_region!(ActionRegion, "group");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Rect, Text, UiFrame};

    #[test]
    fn item_presentation_exposes_state_and_named_content_without_custom_painting() {
        let theme = SemanticTheme::from_tokens(crate::SemanticTokenSet::standard(
            1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11,
        ));
        let tree = UiFrame::<()>::layout(
            ItemPresentation::new("item", Text::new("Body"), theme)
                .leading(Text::new("Lead"))
                .trailing(Text::new("Trail"))
                .selected(true)
                .disabled(true),
            Rect::new(0.0, 0.0, 300.0, 60.0),
        );
        let node = tree
            .accessibility_nodes()
            .iter()
            .find(|node| node.id.as_str().ends_with("/item"))
            .unwrap();
        assert_eq!(node.role.as_deref(), Some("listitem"));
        assert_eq!(node.state.as_deref(), Some("selected, disabled"));
        assert_eq!(
            tree.commands()
                .iter()
                .filter(|command| matches!(command, crate::PaintCommand::Text { .. }))
                .count(),
            3
        );
    }

    #[test]
    fn scaffold_and_regions_retain_accessibility_landmarks() {
        let tree = UiFrame::<()>::layout(
            SurfaceScaffold::new("surface", Text::new("content"))
                .header(ToolRegion::new("tools").child(Text::new("tools")))
                .footer(StatusRegion::new("status").child(Text::new("ready"))),
            Rect::new(0.0, 0.0, 300.0, 120.0),
        );
        let roles = tree
            .accessibility_nodes()
            .iter()
            .filter_map(|node| node.role.as_deref())
            .collect::<Vec<_>>();
        assert!(roles.contains(&"list"));
        assert!(roles.contains(&"status"));
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    enum Action {
        Open(&'static str),
        Context(&'static str),
    }

    #[test]
    fn product_fixtures_compose_from_the_same_item_and_surface_primitives() {
        let theme = SemanticTheme::from_tokens(crate::SemanticTokenSet::standard(
            1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11,
        ));
        for (id, label) in [
            ("settings", "Appearance"),
            ("launcher", "Browser"),
            ("file-grid", "Documents"),
            ("chat-list", "General"),
        ] {
            let tree = UiFrame::layout(
                SurfaceScaffold::new(
                    id,
                    ItemPresentation::new("item", Text::new(label), theme)
                        .supporting(Text::new("Supporting text"))
                        .badge(Text::new("New"))
                        .on_activate(Action::Open(id))
                        .on_context(Action::Context(id)),
                )
                .header(ToolRegion::new("tools").child(Text::new("Actions")))
                .actions(ActionRegion::new("legend").child(Text::new("Open · Menu")))
                .footer(StatusRegion::new("status").child(Text::new("Ready"))),
                Rect::new(0.0, 0.0, 480.0, 240.0),
            );
            assert!(
                !tree
                    .semantic_targets_for_message(&Action::Open(id))
                    .is_empty()
            );
            let target = tree
                .semantic_targets_for_message(&Action::Open(id))
                .into_iter()
                .next()
                .unwrap();
            assert_eq!(
                tree.perform_semantic_action(
                    &target.id,
                    crate::SemanticAction::Invoke(crate::ActionKind::ContextMenu),
                )
                .unwrap()
                .messages,
                vec![Action::Context(id)]
            );
            assert!(!tree.commands().is_empty());
        }
    }

    #[test]
    fn artwork_fallback_and_locale_ordered_bounded_actions_are_declarative() {
        let fallback =
            ArtworkPresentation::<()>::new(false, Text::new("image"), Text::new("fallback"));
        assert!(fallback.used_fallback());
        let tree = UiFrame::layout(
            ActionRegion::<()>::new("actions")
                .child(Text::new("first"))
                .child(Text::new("second"))
                .child(Text::new("hidden"))
                .direction(crate::ReadingDirection::RightToLeft)
                .overflow(2, Text::new("more")),
            Rect::new(0.0, 0.0, 300.0, 60.0),
        );
        let texts = tree
            .commands()
            .iter()
            .filter_map(|command| match command {
                crate::PaintCommand::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(texts, ["hidden", "more"]);
    }
}
