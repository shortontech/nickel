use crate::{
    Align, Border, Color, Column, Component, ComponentBuilderExt, Container, Dropdown, Grid,
    Insets, Justify, Overflow, Row, SemanticTheme, Slider, Spacer, Text, TextAlign, Track, UiId,
};

/// Width below which [`SettingsShell`] stacks navigation above its content.
pub const SETTINGS_SHELL_NARROW_BREAKPOINT: f32 = 720.0;

/// Localized page title and supporting description.
pub struct PageHeader<Message = String>(Container<Message>);

impl<Message> PageHeader<Message> {
    pub fn new(
        theme: SemanticTheme,
        title: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self(
            Container::new()
                .fill_width()
                .min_height(76.0)
                .padding(Insets::symmetric(
                    theme.spacing.section,
                    theme.spacing.content,
                ))
                .background(theme.colors.window)
                .child(
                    Column::new()
                        .gap(theme.spacing.compact)
                        .child(
                            Text::new(title)
                                .scale(1.75)
                                .color(theme.colors.primary_text)
                                .wrap(true),
                        )
                        .child(
                            Text::new(description)
                                .color(theme.colors.secondary_text)
                                .wrap(true),
                        ),
                ),
        )
    }

    pub fn id(mut self, id: impl Into<UiId>) -> Self {
        self.0 = self.0.id(id);
        self
    }
}

impl<Message> Component<Message> for PageHeader<Message> {
    fn into_element(self) -> super::Element<Message> {
        self.0.into_element()
    }
}

/// One Settings destination with a selected surface and accent rail.
pub struct NavigationItem<Message = String>(Container<Message>);

impl<Message> NavigationItem<Message> {
    pub fn new(
        theme: SemanticTheme,
        message: Message,
        label: impl Into<String>,
        selected: bool,
    ) -> Self {
        Self::with_leading(theme, message, label, selected, Spacer::fixed(0.0))
    }

    pub fn with_leading(
        theme: SemanticTheme,
        message: Message,
        label: impl Into<String>,
        selected: bool,
        leading: impl Component<Message>,
    ) -> Self {
        Self(
            Container::new()
                .fill_width()
                .min_height(40.0)
                .radius(theme.radii.control)
                .background(if selected {
                    theme.colors.accent_soft
                } else {
                    theme.colors.sidebar
                })
                .message(message)
                .child(
                    Row::new()
                        .fill_width()
                        .align_items(Align::Center)
                        .gap(theme.spacing.control)
                        .child(
                            Container::new()
                                .width(3.0)
                                .height(24.0)
                                .radius(1.5)
                                .background(if selected {
                                    theme.colors.accent
                                } else {
                                    theme.colors.sidebar
                                }),
                        )
                        .child(leading)
                        .child(
                            Text::new(label)
                                .color(theme.colors.primary_text)
                                .wrap(true)
                                .grow(1.0),
                        ),
                ),
        )
    }

    pub fn id(mut self, id: impl Into<UiId>) -> Self {
        self.0 = self.0.id(id);
        self
    }
}

impl<Message> Component<Message> for NavigationItem<Message> {
    fn into_element(self) -> super::Element<Message> {
        self.0.into_element()
    }
}

/// A localized label separating a group of Settings destinations.
pub struct NavigationSectionLabel<Message = String>(Text<Message>);

impl<Message> NavigationSectionLabel<Message> {
    pub fn new(theme: SemanticTheme, label: impl Into<String>) -> Self {
        Self(
            Text::new(label)
                .scale(0.85)
                .color(theme.colors.secondary_text),
        )
    }
}

impl<Message> Component<Message> for NavigationSectionLabel<Message> {
    fn into_element(self) -> super::Element<Message> {
        self.0.into_element()
    }
}

/// Grouped, independently scrollable Settings navigation.
pub struct SettingsNavigation<Message = String>(Container<Message>);

impl<Message> SettingsNavigation<Message> {
    pub fn new(theme: SemanticTheme, width: f32) -> Self {
        Self(
            Container::new()
                .width(width)
                .min_width(width)
                .fill_height()
                .padding(Insets::all(theme.spacing.content))
                .gap(theme.spacing.control)
                .overflow_y(Overflow::Auto)
                .background(theme.colors.sidebar),
        )
    }

    pub fn section(mut self, theme: SemanticTheme, label: impl Into<String>) -> Self {
        self.0 = self.0.child(NavigationSectionLabel::new(theme, label));
        self
    }

    pub fn item(mut self, item: NavigationItem<Message>) -> Self {
        self.0 = self.0.child(item);
        self
    }

    pub fn child(mut self, child: impl Component<Message>) -> Self {
        self.0 = self.0.child(child);
        self
    }

    pub fn id(mut self, id: impl Into<UiId>) -> Self {
        self.0 = self.0.id(id);
        self
    }
}

impl<Message> Component<Message> for SettingsNavigation<Message> {
    fn into_element(self) -> super::Element<Message> {
        self.0.into_element()
    }
}

/// Responsive Settings chrome composed from navigation, header, and content.
pub struct SettingsShell<Message = String>(Container<Message>);

impl<Message> SettingsShell<Message> {
    pub fn new(
        theme: SemanticTheme,
        viewport_width: f32,
        navigation: impl Component<Message>,
        header: impl Component<Message>,
        content: impl Component<Message>,
    ) -> Self {
        let root = if viewport_width < SETTINGS_SHELL_NARROW_BREAKPOINT {
            Container::new().child(
                Column::new()
                    .fill_width()
                    .fill_height()
                    .child(header)
                    .child(
                        Container::new()
                            .fill_width()
                            .height(220.0)
                            .shrink(0.0)
                            .child(navigation),
                    )
                    .child(Container::new().grow(1.0).child(content)),
            )
        } else {
            Container::new().child(
                Row::new()
                    .fill_width()
                    .fill_height()
                    .child(navigation)
                    .child(
                        Column::new()
                            .grow(1.0)
                            .child(header)
                            .child(Container::new().grow(1.0).child(content)),
                    ),
            )
        };
        Self(
            root.fill_width()
                .fill_height()
                .background(theme.colors.window),
        )
    }

    pub fn id(mut self, id: impl Into<UiId>) -> Self {
        self.0 = self.0.id(id);
        self
    }
}

impl<Message> Component<Message> for SettingsShell<Message> {
    fn into_element(self) -> super::Element<Message> {
        self.0.into_element()
    }
}

/// The semantic background used by a [`Surface`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SurfaceRole {
    Window,
    Sidebar,
    Card,
    Raised,
    Hover,
}

/// A product-neutral semantic container.
pub struct Surface<Message = String>(Container<Message>);

impl<Message> Surface<Message> {
    pub fn new(theme: SemanticTheme, role: SurfaceRole) -> Self {
        let color = match role {
            SurfaceRole::Window => theme.colors.window,
            SurfaceRole::Sidebar => theme.colors.sidebar,
            SurfaceRole::Card => theme.colors.card,
            SurfaceRole::Raised => theme.colors.raised,
            SurfaceRole::Hover => theme.colors.hover,
        };
        Self(Container::new().background(color))
    }

    pub fn child(mut self, child: impl Component<Message>) -> Self {
        self.0 = self.0.child(child);
        self
    }

    pub fn children(mut self, children: impl IntoIterator<Item = impl Component<Message>>) -> Self {
        self.0 = self.0.children(children);
        self
    }

    pub fn id(mut self, id: impl Into<UiId>) -> Self {
        self.0 = self.0.id(id);
        self
    }

    pub fn border(mut self, border: impl Into<Border>) -> Self {
        self.0 = self.0.border_value(border);
        self
    }

    pub fn radius(mut self, radius: f32) -> Self {
        self.0 = self.0.radius(radius);
        self
    }

    pub fn padding(mut self, padding: impl Into<Insets>) -> Self {
        self.0 = self.0.padding(padding);
        self
    }

    pub fn width(mut self, width: f32) -> Self {
        self.0 = self.0.width(width);
        self
    }

    pub fn height(mut self, height: f32) -> Self {
        self.0 = self.0.height(height);
        self
    }

    pub fn fill_width(mut self) -> Self {
        self.0 = self.0.fill_width();
        self
    }
}

impl<Message> Component<Message> for Surface<Message> {
    fn into_element(self) -> super::Element<Message> {
        self.0.into_element()
    }
}

/// A binary control that emits a request for the opposite value.
pub struct Switch<Message = String>(Container<Message>);

impl<Message> Switch<Message> {
    pub fn new(value: bool, on_change: fn(bool) -> Message, theme: SemanticTheme) -> Self {
        let thumb = Container::new()
            .width(18.0)
            .height(18.0)
            .radius(9.0)
            .background(theme.colors.primary_text);
        Self(
            Container::new()
                .width(42.0)
                .height(24.0)
                .radius(12.0)
                .padding(Insets::all(3.0))
                .background(if value {
                    theme.colors.accent
                } else {
                    theme.colors.raised
                })
                .message(on_change(!value))
                .child(
                    Row::new()
                        .fill_width()
                        .justify_content(if value { Justify::End } else { Justify::Start })
                        .child(thumb),
                ),
        )
    }

    pub fn id(mut self, id: impl Into<UiId>) -> Self {
        self.0 = self.0.id(id);
        self
    }
}

impl<Message> Component<Message> for Switch<Message> {
    fn into_element(self) -> super::Element<Message> {
        self.0.into_element()
    }
}

/// A bordered Settings surface with an optional heading and description.
pub struct SettingsCard<Message = String>(Container<Message>);

impl<Message> SettingsCard<Message> {
    pub fn new(theme: SemanticTheme) -> Self {
        Self(
            Container::new()
                .fill_width()
                .gap(theme.spacing.content)
                .padding(Insets::all(theme.spacing.content))
                .background(theme.colors.card)
                .border(theme.colors.raised, 1.0)
                .radius(theme.radii.card),
        )
    }

    pub fn titled(
        theme: SemanticTheme,
        title: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        let description = description.into();
        let mut card =
            Self::new(theme).child(Text::new(title).color(theme.colors.primary_text).scale(1.2));
        if !description.is_empty() {
            card = card.child(
                Text::new(description)
                    .color(theme.colors.secondary_text)
                    .wrap(true),
            );
        }
        card
    }

    pub fn child(mut self, child: impl Component<Message>) -> Self {
        self.0 = self.0.child(child);
        self
    }

    pub fn children(mut self, children: impl IntoIterator<Item = impl Component<Message>>) -> Self {
        self.0 = self.0.children(children);
        self
    }

    pub fn id(mut self, id: impl Into<UiId>) -> Self {
        self.0 = self.0.id(id);
        self
    }
}

impl<Message> Component<Message> for SettingsCard<Message> {
    fn into_element(self) -> super::Element<Message> {
        self.0.into_element()
    }
}

/// A label, supporting text, and trailing Settings control.
pub struct SettingsRow<Message = String>(Row<Message>);

impl<Message> SettingsRow<Message> {
    pub fn new(
        theme: SemanticTheme,
        label: impl Into<String>,
        supporting_text: impl Into<String>,
    ) -> Self {
        let supporting_text = supporting_text.into();
        let mut labels = Column::new()
            .grow(1.0)
            .gap(theme.spacing.compact)
            .child(Text::new(label).color(theme.colors.primary_text).wrap(true));
        if !supporting_text.is_empty() {
            labels = labels.child(
                Text::new(supporting_text)
                    .color(theme.colors.secondary_text)
                    .wrap(true),
            );
        }
        Self(
            Row::new()
                .fill_width()
                .min_height(52.0)
                .gap(theme.spacing.content)
                .padding(Insets::all(theme.spacing.control))
                .align_items(Align::Center)
                .child(labels),
        )
    }

    pub fn trailing(mut self, control: impl Component<Message>) -> Self {
        self.0 = self.0.child(control);
        self
    }
}

impl<Message> Component<Message> for SettingsRow<Message> {
    fn into_element(self) -> super::Element<Message> {
        self.0.into_element()
    }
}

/// A Settings row containing a slider and formatted value.
pub struct SliderField<Message = String>(SettingsRow<Message>);

impl<Message> SliderField<Message> {
    pub fn new(
        theme: SemanticTheme,
        label: impl Into<String>,
        supporting_text: impl Into<String>,
        value_label: impl Into<String>,
        value: f32,
        on_change: fn(f32) -> Message,
    ) -> Self {
        let control = Row::new()
            .width(420.0)
            .gap(theme.spacing.content)
            .align_items(Align::Center)
            .child(
                Slider::on_change(on_change, value)
                    .colors(
                        theme.colors.raised,
                        theme.colors.accent,
                        theme.colors.primary_text,
                    )
                    .width(330.0),
            )
            .child(
                Text::new(value_label)
                    .width(72.0)
                    .align(TextAlign::End)
                    .color(theme.colors.primary_text),
            );
        Self(SettingsRow::new(theme, label, supporting_text).trailing(control))
    }
}

impl<Message> Component<Message> for SliderField<Message> {
    fn into_element(self) -> super::Element<Message> {
        self.0.into_element()
    }
}

/// A Settings row containing an existing typed [`Dropdown`].
pub struct SelectField<Message = String>(SettingsRow<Message>);

impl<Message> SelectField<Message> {
    pub fn new(
        theme: SemanticTheme,
        label: impl Into<String>,
        supporting_text: impl Into<String>,
        toggle_message: Message,
        selected: impl Into<String>,
        options: impl IntoIterator<Item = (impl Into<String>, Message)>,
        expanded: bool,
    ) -> Self {
        let dropdown = Dropdown::new(toggle_message, selected, options)
            .expanded(expanded)
            .colors(
                theme.colors.raised,
                theme.colors.hover,
                theme.colors.primary_text,
            );
        Self(
            SettingsRow::new(theme, label, supporting_text)
                .trailing(Container::new().width(180.0).child(dropdown)),
        )
    }
}

impl<Message> Component<Message> for SelectField<Message> {
    fn into_element(self) -> super::Element<Message> {
        self.0.into_element()
    }
}

/// A keyboard-order-preserving horizontal list of tabs.
pub struct TabList<Message = String>(Row<Message>);

impl<Message> TabList<Message> {
    pub fn new(
        theme: SemanticTheme,
        tabs: impl IntoIterator<Item = (impl Into<String>, Message, bool)>,
    ) -> Self {
        let tabs = tabs.into_iter().map(|(label, message, selected)| {
            Container::new().height(38.0).message(message).child(
                Column::new()
                    .gap(6.0)
                    .child(Text::new(label).color(if selected {
                        theme.colors.accent
                    } else {
                        theme.colors.secondary_text
                    }))
                    .child(
                        Container::new()
                            .height(2.0)
                            .fill_width()
                            .background(if selected {
                                theme.colors.accent
                            } else {
                                theme.colors.window
                            }),
                    ),
            )
        });
        Self(Row::new().gap(theme.spacing.section).children(tabs))
    }
}

impl<Message> Component<Message> for TabList<Message> {
    fn into_element(self) -> super::Element<Message> {
        self.0.into_element()
    }
}

/// A preview-backed, single activation target for visual choices.
pub struct ChoiceCard<Message = String>(Container<Message>);

impl<Message> ChoiceCard<Message> {
    pub fn new(
        theme: SemanticTheme,
        message: Message,
        label: impl Into<String>,
        selected: bool,
        preview: impl Component<Message>,
    ) -> Self {
        let indicator = Container::new()
            .width(16.0)
            .height(16.0)
            .radius(8.0)
            .border(
                if selected {
                    theme.colors.accent
                } else {
                    theme.colors.secondary_text
                },
                2.0,
            )
            .padding(Insets::all(3.0))
            .child(
                Container::new()
                    .fill_width()
                    .fill_height()
                    .radius(4.0)
                    .background(if selected {
                        theme.colors.accent
                    } else {
                        theme.colors.card
                    }),
            );
        Self(
            Container::new()
                .min_width(180.0)
                .height(168.0)
                .gap(theme.spacing.content)
                .padding(Insets::all(theme.spacing.content))
                .background(theme.colors.card)
                .border(
                    if selected {
                        theme.colors.accent
                    } else {
                        theme.colors.raised
                    },
                    if selected { 2.0 } else { 1.0 },
                )
                .radius(theme.radii.card)
                .message(message)
                .child(preview)
                .child(
                    Row::new()
                        .gap(theme.spacing.control)
                        .align_items(Align::Center)
                        .child(indicator)
                        .child(Text::new(label).color(theme.colors.primary_text).wrap(true)),
                ),
        )
    }

    pub fn id(mut self, id: impl Into<UiId>) -> Self {
        self.0 = self.0.id(id);
        self
    }
}

impl<Message> Component<Message> for ChoiceCard<Message> {
    fn into_element(self) -> super::Element<Message> {
        self.0.into_element()
    }
}

/// A responsive visual-choice layout.
pub struct ChoiceCardGroup<Message = String>(Grid<Message>);

impl<Message> ChoiceCardGroup<Message> {
    pub fn new(items: impl IntoIterator<Item = ChoiceCard<Message>>) -> Self {
        Self(
            Grid::auto_fit(Track::minmax(180.0, Track::fr(1.0)))
                .gap(16.0)
                .fill_width()
                .children(items),
        )
    }
}

impl<Message> Component<Message> for ChoiceCardGroup<Message> {
    fn into_element(self) -> super::Element<Message> {
        self.0.into_element()
    }
}

/// Non-interactive state rendered by a [`PreviewTile`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PreviewState {
    Loading,
    Unavailable,
    Error,
}

/// A bounded, rounded presentation surface for image or code-rendered previews.
pub struct PreviewTile<Message = String>(Container<Message>);

impl<Message> PreviewTile<Message> {
    pub fn new(theme: SemanticTheme, preview: impl Component<Message>) -> Self {
        Self(Self::frame(theme).child(preview))
    }

    pub fn placeholder(
        theme: SemanticTheme,
        state: PreviewState,
        label: impl Into<String>,
    ) -> Self {
        let (mark, color) = match state {
            PreviewState::Loading => ("…", theme.text.secondary),
            PreviewState::Unavailable => ("—", theme.text.disabled),
            PreviewState::Error => ("!", theme.text.danger),
        };
        Self(
            Self::frame(theme).child(
                Column::new()
                    .gap(theme.spacing.compact)
                    .align_items(Align::Center)
                    .justify_content(Justify::Center)
                    .child(Text::new(mark).scale(1.4).color(color))
                    .child(
                        Text::new(label)
                            .color(color)
                            .align(TextAlign::Center)
                            .wrap(true),
                    ),
            ),
        )
    }

    pub fn loading(theme: SemanticTheme, label: impl Into<String>) -> Self {
        Self::placeholder(theme, PreviewState::Loading, label)
    }

    pub fn unavailable(theme: SemanticTheme, label: impl Into<String>) -> Self {
        Self::placeholder(theme, PreviewState::Unavailable, label)
    }

    pub fn error(theme: SemanticTheme, label: impl Into<String>) -> Self {
        Self::placeholder(theme, PreviewState::Error, label)
    }

    fn frame(theme: SemanticTheme) -> Container<Message> {
        Container::new()
            .min_width(96.0)
            .min_height(72.0)
            .fill_width()
            .fill_height()
            .align_items(Align::Center)
            .justify_content(Justify::Center)
            .background(theme.surfaces.raised)
            .border(theme.borders.subtle, theme.sizing.border)
            .radius(theme.radii.card)
            .overflow(Overflow::Clip, Overflow::Clip)
    }

    pub fn id(mut self, id: impl Into<UiId>) -> Self {
        self.0 = self.0.id(id);
        self
    }

    pub fn width(mut self, width: f32) -> Self {
        self.0 = self.0.width(width);
        self
    }

    pub fn height(mut self, height: f32) -> Self {
        self.0 = self.0.height(height);
        self
    }
}

impl<Message> Component<Message> for PreviewTile<Message> {
    fn into_element(self) -> super::Element<Message> {
        self.0.into_element()
    }
}

/// A color value or custom-color action with a distinct add mark.
pub struct ColorSwatch<Message = String>(Container<Message>);

impl<Message> ColorSwatch<Message> {
    pub fn color(theme: SemanticTheme, message: Message, color: Color, selected: bool) -> Self {
        let ring = Container::new()
            .width(38.0)
            .height(38.0)
            .radius(19.0)
            .border(
                if selected {
                    theme.colors.accent
                } else {
                    theme.colors.raised
                },
                if selected { 3.0 } else { 1.0 },
            )
            .padding(Insets::all(4.0))
            .child(
                Container::new()
                    .fill_width()
                    .fill_height()
                    .radius(14.0)
                    .background(color),
            );
        Self(
            Container::new()
                .width(42.0)
                .height(42.0)
                .message(message)
                .child(ring),
        )
    }

    pub fn custom(theme: SemanticTheme, message: Message) -> Self {
        Self(
            Container::new()
                .width(42.0)
                .height(42.0)
                .radius(21.0)
                .border(theme.colors.raised, 1.0)
                .message(message)
                .child(
                    Container::new().fill_width().child(
                        Text::new("+")
                            .align(TextAlign::Center)
                            .scale(1.45)
                            .color(theme.colors.secondary_text),
                    ),
                ),
        )
    }

    pub fn id(mut self, id: impl Into<UiId>) -> Self {
        self.0 = self.0.id(id);
        self
    }
}

impl<Message> Component<Message> for ColorSwatch<Message> {
    fn into_element(self) -> super::Element<Message> {
        self.0.into_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DiagnosticKind, PaintCommand, Rect, ResolvedNode, SemanticColors, UiTree};

    #[derive(Clone, Debug, PartialEq)]
    enum Message {
        Toggle(bool),
        Tab(u8),
        Choice(u8),
        Color(u8),
        Slide(f32),
        Navigate(u8),
        ToggleSelect,
        Select(u8),
    }

    fn toggle(value: bool) -> Message {
        Message::Toggle(value)
    }

    fn slide(value: f32) -> Message {
        Message::Slide(value)
    }

    fn theme() -> SemanticTheme {
        SemanticTheme::new(SemanticColors {
            window: 0x101114,
            sidebar: 0x15171b,
            card: 0x1b1e23,
            raised: 0x32363e,
            hover: 0x3c414b,
            primary_text: 0xf2f3f5,
            secondary_text: 0xa8abb2,
            accent: 0x9b62e8,
            accent_soft: 0x45305f,
            positive: 0x55b982,
        })
    }

    #[test]
    fn switch_emits_requested_value() {
        let tree = UiTree::layout(
            Switch::new(false, toggle, theme()).id("switch"),
            Rect::new(0.0, 0.0, 100.0, 50.0),
        );
        assert!(tree.message_rect(&Message::Toggle(true)).is_some());
    }

    #[test]
    fn tabs_and_swatches_preserve_typed_activation() {
        let theme = theme();
        let tree = UiTree::layout(
            Column::new()
                .child(TabList::new(
                    theme,
                    [
                        ("General", Message::Tab(0), true),
                        ("Theme", Message::Tab(1), false),
                    ],
                ))
                .child(
                    Row::new()
                        .child(ColorSwatch::color(theme, Message::Color(0), 0x3366ff, true))
                        .child(ColorSwatch::custom(theme, Message::Color(1))),
                ),
            Rect::new(0.0, 0.0, 400.0, 120.0),
        );
        assert!(tree.message_rect(&Message::Tab(1)).is_some());
        assert!(tree.message_rect(&Message::Color(0)).is_some());
        assert!(tree.message_rect(&Message::Color(1)).is_some());
    }

    #[test]
    fn choice_group_reflows_deterministically() {
        let theme = theme();
        let make = |choice| {
            ChoiceCard::new(
                theme,
                Message::Choice(choice),
                format!("Choice {choice}"),
                choice == 0,
                Surface::new(theme, SurfaceRole::Raised).height(86.0),
            )
        };
        let wide = UiTree::layout(
            ChoiceCardGroup::new([make(0), make(1), make(2)]),
            Rect::new(0.0, 0.0, 640.0, 400.0),
        );
        let narrow = UiTree::layout(
            ChoiceCardGroup::new([make(0), make(1), make(2)]),
            Rect::new(0.0, 0.0, 220.0, 600.0),
        );
        assert_eq!(wide.resolved_grid_columns(), Some(3));
        assert_eq!(narrow.resolved_grid_columns(), Some(1));
        assert!(wide.message_rect(&Message::Choice(2)).is_some());
    }

    #[test]
    fn preview_tile_renders_rounded_content_and_explicit_placeholder_states() {
        let theme = theme();
        let content = UiTree::<Message>::layout(
            PreviewTile::new(theme, Text::new("Preview"))
                .width(160.0)
                .height(96.0),
            Rect::new(0.0, 0.0, 160.0, 96.0),
        );
        assert!(content.commands().iter().any(|command| matches!(
            command,
            PaintCommand::RoundedFill { radius, .. } if *radius == theme.radii.card
        )));
        assert!(content.commands().iter().any(
            |command| matches!(command, PaintCommand::Text { text, .. } if text == "Preview")
        ));

        for tile in [
            PreviewTile::loading(theme, "Loading preview"),
            PreviewTile::unavailable(theme, "Preview unavailable"),
            PreviewTile::error(theme, "Preview failed"),
        ] {
            let tree = UiTree::<Message>::layout(
                tile.width(160.0).height(96.0),
                Rect::new(0.0, 0.0, 160.0, 96.0),
            );
            assert!(tree.commands().iter().any(|command| matches!(
                command,
                PaintCommand::Text { text, .. }
                    if text.contains("preview") || text.contains("Preview")
            )));
        }
    }

    #[test]
    fn settings_composites_have_finite_deterministic_layout() {
        let theme = theme();
        let tree = UiTree::layout_with_diagnostics(
            SettingsCard::titled(theme, "Interface settings", "Shared visual controls")
                .child(
                    SettingsRow::new(theme, "Transparency", "Use solid surfaces")
                        .trailing(Switch::new(true, toggle, theme)),
                )
                .child(SliderField::new(
                    theme,
                    "Starting hue",
                    "Base hue for the accent color",
                    "305°",
                    0.85,
                    slide,
                )),
            Rect::new(0.0, 0.0, 800.0, 300.0),
        );
        assert!(tree.diagnostics().is_empty(), "{:?}", tree.diagnostics());
        assert!(tree.message_rect(&Message::Toggle(false)).is_some());
        assert!(tree.message_rect(&Message::Slide(0.85)).is_some());
    }

    fn shell(width: f32) -> SettingsShell<Message> {
        let theme = theme();
        let navigation = SettingsNavigation::new(theme, 240.0)
            .id("navigation")
            .section(theme, "System")
            .item(
                NavigationItem::new(theme, Message::Navigate(0), "Display", true)
                    .id("selected-navigation-item"),
            )
            .item(NavigationItem::new(
                theme,
                Message::Navigate(1),
                "Sound",
                false,
            ));
        SettingsShell::new(
            theme,
            width,
            navigation,
            PageHeader::new(theme, "Appearance", "Customize how Nickel looks and feels")
                .id("page-header"),
            Surface::new(theme, SurfaceRole::Window)
                .id("page-content")
                .fill_width()
                .height(300.0),
        )
    }

    fn node_ending<'a>(tree: &'a UiTree<Message>, suffix: &str) -> &'a ResolvedNode {
        tree.resolved_layout()
            .nodes()
            .iter()
            .find(|node| node.id.as_str().ends_with(suffix))
            .unwrap()
    }

    #[test]
    fn settings_shell_reflows_at_its_own_breakpoint() {
        let ordinary =
            UiTree::layout_with_diagnostics(shell(1000.0), Rect::new(0.0, 0.0, 1000.0, 640.0));
        let narrow =
            UiTree::layout_with_diagnostics(shell(560.0), Rect::new(0.0, 0.0, 560.0, 760.0));
        assert!(
            ordinary.diagnostics().is_empty(),
            "{:?}",
            ordinary.diagnostics()
        );
        assert!(
            narrow.diagnostics().iter().all(|diagnostic| !matches!(
                diagnostic.kind,
                DiagnosticKind::InvalidGeometry
                    | DiagnosticKind::ContradictoryConstraints
                    | DiagnosticKind::FlexOverflow
            )),
            "{:?}",
            narrow.diagnostics()
        );

        let ordinary_navigation = node_ending(&ordinary, "/navigation");
        let ordinary_header = node_ending(&ordinary, "/page-header");
        let narrow_navigation = node_ending(&narrow, "/navigation");
        let narrow_header = node_ending(&narrow, "/page-header");
        assert!(ordinary_navigation.allocated.origin.x < ordinary_header.allocated.origin.x);
        assert!(
            narrow_navigation.allocated.origin.y
                >= narrow_header.allocated.origin.y + narrow_header.allocated.size.height
        );
        assert!(ordinary.message_rect(&Message::Navigate(1)).is_some());
        assert!(narrow.message_rect(&Message::Navigate(0)).is_some());
    }

    #[test]
    fn select_field_preserves_toggle_and_option_messages() {
        let theme = theme();
        let tree = UiTree::layout(
            SettingsCard::new(theme).child(SelectField::new(
                theme,
                "Animations",
                "Control the level of interface animations",
                Message::ToggleSelect,
                "Normal",
                [
                    ("Reduced", Message::Select(0)),
                    ("Normal", Message::Select(1)),
                ],
                true,
            )),
            Rect::new(0.0, 0.0, 760.0, 240.0),
        );
        assert!(tree.message_rect(&Message::ToggleSelect).is_some());
        assert!(tree.message_rect(&Message::Select(0)).is_some());
        assert!(tree.message_rect(&Message::Select(1)).is_some());
    }
}
