use crate::{
    Align, AnyView, Border, Button, Color, Column, Component, ComponentBuilderExt, Container,
    Dropdown, Grid, HorizontalRule, Insets, Justify, NavigationScope, Overflow, ReadingDirection,
    Row, SemanticRole, SemanticTheme, Slider, Spacer, Text, TextAlign, TextField, Track, UiId,
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
                .background(theme.surfaces.window)
                .child(
                    Column::new()
                        .gap(theme.spacing.compact)
                        .child(
                            Text::new(title)
                                .scale(1.75)
                                .color(theme.text.primary)
                                .wrap(true),
                        )
                        .child(
                            Text::new(description)
                                .color(theme.text.secondary)
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
pub struct NavigationItem<Message = String>(super::Element<Message>);

impl<Message> NavigationItem<Message> {
    pub fn unavailable(
        theme: SemanticTheme,
        label: impl Into<String>,
        unavailable_label: impl Into<String>,
    ) -> Self {
        let label = label.into();
        Self(
            Container::new()
                .fill_width()
                .min_height(40.0)
                .padding(Insets::symmetric(
                    theme.spacing.control,
                    theme.spacing.content,
                ))
                .radius(theme.radii.control)
                .background(theme.surfaces.sidebar)
                .semantic_role(SemanticRole::NavigationItem)
                .accessibility_label(&label)
                .accessibility_state("unavailable")
                .child(
                    Row::new()
                        .gap(theme.spacing.control)
                        .child(Text::new(label).color(theme.text.disabled).wrap(true))
                        .child(
                            Text::new(unavailable_label)
                                .color(theme.text.disabled)
                                .scale(0.85)
                                .wrap(true),
                        ),
                )
                .into_element(),
        )
    }

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
        Self::with_leading_direction(
            theme,
            message,
            label,
            selected,
            leading,
            ReadingDirection::LeftToRight,
        )
    }

    pub fn with_leading_direction(
        theme: SemanticTheme,
        message: Message,
        label: impl Into<String>,
        selected: bool,
        leading: impl Component<Message>,
        direction: ReadingDirection,
    ) -> Self {
        let label = label.into();
        let mut item_content = Row::new()
            .fill_width()
            .fill_height()
            .align_items(Align::Center)
            .gap(theme.spacing.control)
            .child(leading)
            .child(
                Text::new(&label)
                    .color(theme.text.primary)
                    .wrap(true)
                    .align(if direction == ReadingDirection::RightToLeft {
                        TextAlign::End
                    } else {
                        TextAlign::Start
                    })
                    .grow(1.0),
            );
        if direction == ReadingDirection::RightToLeft {
            item_content = item_content.reverse();
        }
        let rail = Container::new()
            .width(3.0)
            .height(40.0)
            .background(if selected {
                theme.accent.ordinary
            } else {
                theme.surfaces.sidebar
            });
        let action = Container::new()
            .grow(1.0)
            .fill_height()
            .padding(if direction == ReadingDirection::RightToLeft {
                Insets {
                    top: 0.0,
                    right: 0.0,
                    bottom: 0.0,
                    left: 1.0,
                }
            } else {
                Insets {
                    top: 0.0,
                    right: 1.0,
                    bottom: 0.0,
                    left: 0.0,
                }
            })
            .child(
                Container::new()
                    .fill_width()
                    .fill_height()
                    .child(item_content),
            );
        let mut row = Row::new()
            .fill_width()
            .align_items(Align::Center)
            .gap(theme.spacing.control)
            .child(rail)
            .child(action);
        if direction == ReadingDirection::RightToLeft {
            row = row.reverse();
        }
        Self(
            Container::new()
                .fill_width()
                .min_height(40.0)
                .shrink(0.0)
                .radius(theme.radii.control)
                .background(if selected {
                    theme.accent.soft
                } else {
                    theme.surfaces.sidebar
                })
                .controller_focus_border(theme.borders.controller_focus)
                .message(message)
                .child(row)
                .semantic_role(SemanticRole::NavigationItem)
                .accessibility_label(label)
                .accessibility_state(if selected { "selected" } else { "unselected" })
                .into_element(),
        )
    }

    pub fn id(mut self, id: impl Into<UiId>) -> Self {
        self.0 = self.0.id(id);
        self
    }

    pub fn semantic_role(mut self, role: SemanticRole) -> Self {
        self.0 = self.0.semantic_role(role);
        self
    }

    pub fn direction(mut self, direction: ReadingDirection) -> Self {
        if direction == ReadingDirection::RightToLeft {
            self.0.children.reverse();
        }
        self
    }
}

impl<Message> Component<Message> for NavigationItem<Message> {
    fn into_element(self) -> super::Element<Message> {
        self.0.into_element()
    }
}

/// A real text-input filter styled for the Settings navigation rail.
pub struct SettingsSearchField<Message = String>(Container<Message>);

/// One searchable Settings destination. Labels are localized by the owning
/// application; the shared matcher owns ranking and disambiguation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SettingsSearchEntry<Message> {
    pub page: String,
    pub section: String,
    pub control: String,
    pub target: UiId,
    pub message: Message,
    pub available: bool,
}

impl<Message> SettingsSearchEntry<Message> {
    pub fn new(
        page: impl Into<String>,
        section: impl Into<String>,
        control: impl Into<String>,
        target: impl Into<UiId>,
        message: Message,
    ) -> Self {
        Self {
            page: page.into(),
            section: section.into(),
            control: control.into(),
            target: target.into(),
            message,
            available: true,
        }
    }

    pub fn unavailable(mut self) -> Self {
        self.available = false;
        self
    }

    pub fn disambiguated_label(&self) -> String {
        format!("{} — {} · {}", self.control, self.page, self.section)
    }
}

/// Stable, relevance-ranked search over localized destination metadata.
pub fn search_settings<'a, Message>(
    query: &str,
    entries: &'a [SettingsSearchEntry<Message>],
) -> Vec<&'a SettingsSearchEntry<Message>> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Vec::new();
    }
    let mut matches = entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| {
            let control = entry.control.to_lowercase();
            let page = entry.page.to_lowercase();
            let section = entry.section.to_lowercase();
            let score = if control == query {
                0
            } else if control.starts_with(&query) {
                1
            } else if control.contains(&query) {
                2
            } else if section.starts_with(&query) {
                3
            } else if section.contains(&query) {
                4
            } else if page.starts_with(&query) {
                5
            } else if page.contains(&query) {
                6
            } else {
                return None;
            };
            Some((score, index, entry))
        })
        .collect::<Vec<_>>();
    matches.sort_by_key(|(score, index, _)| (*score, *index));
    matches.into_iter().map(|(_, _, entry)| entry).collect()
}

impl<Message> SettingsSearchField<Message> {
    pub fn new(
        theme: SemanticTheme,
        id: impl Into<UiId>,
        value: &str,
        placeholder: impl Into<String>,
        on_change: fn(String) -> Message,
    ) -> Self {
        Self::with_leading(
            theme,
            id,
            value,
            placeholder,
            on_change,
            Text::new("⌕")
                .width(15.0)
                .scale(1.1)
                .align(TextAlign::Center)
                .color(theme.text.secondary),
        )
    }

    pub fn with_leading(
        theme: SemanticTheme,
        id: impl Into<UiId>,
        value: &str,
        placeholder: impl Into<String>,
        on_change: fn(String) -> Message,
        leading: impl Component<Message>,
    ) -> Self {
        let text_color = if value.is_empty() {
            theme.text.secondary
        } else {
            theme.text.primary
        };
        Self(
            Container::new()
                .fill_width()
                .height(40.0)
                .shrink(0.0)
                .padding(Insets::symmetric(9.0, 10.0))
                .radius(theme.radii.control)
                .background(theme.surfaces.raised)
                .border(theme.borders.subtle, theme.sizing.border)
                .child(
                    Row::new()
                        .fill_width()
                        .align_items(Align::Center)
                        .gap(theme.spacing.control)
                        .child(leading)
                        .child(
                            TextField::on_change_with_placeholder(value, placeholder, on_change)
                                .id(id)
                                .grow(1.0)
                                .scale(0.9)
                                .color(text_color)
                                .wrap(false),
                        ),
                ),
        )
    }
}

impl<Message> Component<Message> for SettingsSearchField<Message> {
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
                .color(theme.text.secondary)
                .wrap(true),
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
        Self::build(theme, width, true)
    }

    pub fn embedded_header(theme: SemanticTheme, width: f32) -> Self {
        Self::build(theme, width, false)
    }

    fn build(theme: SemanticTheme, width: f32, owns_pane: bool) -> Self {
        let mut navigation = Container::new()
            .id("settings-navigation-pane")
            .width(width)
            .min_width(width)
            .padding(Insets::all(theme.spacing.content))
            .gap(theme.spacing.compact)
            .background(theme.surfaces.sidebar);
        if owns_pane {
            navigation = navigation
                .fill_height()
                .border(theme.borders.subtle, 1.0)
                .overflow_y(Overflow::Auto)
                .navigation_scope(NavigationScope::pane(false))
                .navigation_scope_highlight(theme.borders.controller_focus);
        }
        Self(navigation)
    }

    pub fn section(mut self, theme: SemanticTheme, label: impl Into<String>) -> Self {
        self.0 = self.0.child(
            Container::new()
                .shrink(0.0)
                .padding(Insets {
                    top: theme.spacing.compact,
                    right: 0.0,
                    bottom: 0.0,
                    left: 0.0,
                })
                .child(NavigationSectionLabel::new(theme, label)),
        );
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
        Self::new_directional(
            theme,
            viewport_width,
            navigation,
            header,
            content,
            ReadingDirection::LeftToRight,
        )
    }

    pub fn new_directional(
        theme: SemanticTheme,
        viewport_width: f32,
        navigation: impl Component<Message>,
        header: impl Component<Message>,
        content: impl Component<Message>,
        direction: ReadingDirection,
    ) -> Self {
        let content = Container::new()
            .id("settings-content-pane")
            .fill_width()
            .fill_height()
            .border(theme.borders.subtle, 1.0)
            .navigation_scope(NavigationScope::pane(true))
            .navigation_scope_highlight(theme.borders.controller_focus)
            .child(content);
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
            let mut layout = Row::new()
                .fill_width()
                .fill_height()
                .child(navigation)
                .child(
                    Column::new()
                        .grow(1.0)
                        .child(header)
                        .child(Container::new().grow(1.0).child(content)),
                );
            if direction == ReadingDirection::RightToLeft {
                layout = layout.reverse();
            }
            Container::new().child(layout)
        };
        Self(
            root.fill_width()
                .fill_height()
                .background(theme.surfaces.window),
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
            SurfaceRole::Window => theme.surfaces.window,
            SurfaceRole::Sidebar => theme.surfaces.sidebar,
            SurfaceRole::Card => theme.surfaces.card,
            SurfaceRole::Raised => theme.surfaces.raised,
            SurfaceRole::Hover => theme.surfaces.hover,
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

/// Semantic state of a binary switch, including honest unavailable states.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SwitchState {
    Off,
    On,
    MixedUnavailable,
    DisabledOff,
    DisabledOn,
}

impl SwitchState {
    fn value(self) -> bool {
        matches!(self, Self::On | Self::DisabledOn)
    }

    fn interactive(self) -> bool {
        matches!(self, Self::Off | Self::On)
    }
}

/// A binary control that emits a request for the opposite value.
pub struct Switch<Message = String>(Container<Message>);

impl<Message> Switch<Message> {
    pub fn new(value: bool, on_change: fn(bool) -> Message, theme: SemanticTheme) -> Self {
        Self::with_state(
            if value {
                SwitchState::On
            } else {
                SwitchState::Off
            },
            Some(on_change),
            theme,
        )
    }

    pub fn with_state(
        state: SwitchState,
        on_change: Option<fn(bool) -> Message>,
        theme: SemanticTheme,
    ) -> Self {
        let value = state.value();
        let thumb = Container::new()
            .width(18.0)
            .height(18.0)
            .radius(9.0)
            .background(if state == SwitchState::MixedUnavailable {
                theme.text.disabled
            } else {
                theme.text.primary
            });
        let mut control = Container::new()
            .width(42.0)
            .height(24.0)
            .semantic_role(SemanticRole::Switch)
            .accessibility_label("Switch")
            .radius(12.0)
            .padding(Insets::all(3.0))
            .background(if state == SwitchState::MixedUnavailable {
                theme.surfaces.selected
            } else if value {
                theme.accent.ordinary
            } else {
                theme.surfaces.raised
            })
            .interaction_backgrounds(theme.surfaces.hover, theme.surfaces.pressed)
            .focus_border(theme.borders.focus)
            .controller_focus_border(theme.borders.controller_focus)
            .accessibility_state(match state {
                SwitchState::Off => "off",
                SwitchState::On => "on",
                SwitchState::MixedUnavailable => "mixed unavailable",
                SwitchState::DisabledOff => "off disabled",
                SwitchState::DisabledOn => "on disabled",
            })
            .child(
                Row::new()
                    .fill_width()
                    .justify_content(if state == SwitchState::MixedUnavailable {
                        Justify::Center
                    } else if value {
                        Justify::End
                    } else {
                        Justify::Start
                    })
                    .child(thumb),
            );
        if state.interactive()
            && let Some(on_change) = on_change
        {
            control = control.message(on_change(!value));
        }
        Self(control)
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
                .background(theme.surfaces.card)
                .border(theme.surfaces.raised, 1.0)
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
            Self::new(theme).child(Text::new(title).color(theme.text.primary).scale(1.2));
        if !description.is_empty() {
            card = card.child(
                Text::new(description)
                    .color(theme.text.secondary)
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

/// A titled, vertically spaced group of related Settings content.
pub struct SettingsSection<Message = String>(Column<Message>);

impl<Message> SettingsSection<Message> {
    pub fn new(theme: SemanticTheme, heading: impl Into<String>) -> Self {
        Self(
            Column::new().fill_width().gap(theme.spacing.content).child(
                Text::new(heading)
                    .scale(1.2)
                    .color(theme.text.primary)
                    .wrap(true),
            ),
        )
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

impl<Message> Component<Message> for SettingsSection<Message> {
    fn into_element(self) -> super::Element<Message> {
        self.0.into_element()
    }
}

/// One bounded card whose rows are separated by semantic dividers.
pub struct SettingsListCard<Message = String> {
    card: SettingsCard<Message>,
    theme: SemanticTheme,
    rows: usize,
}

impl<Message> SettingsListCard<Message> {
    pub fn new(theme: SemanticTheme) -> Self {
        Self {
            card: SettingsCard::new(theme),
            theme,
            rows: 0,
        }
    }

    pub fn row(mut self, row: SettingsRow<Message>) -> Self {
        if self.rows > 0 {
            self.card = self
                .card
                .child(HorizontalRule::new(self.theme.borders.subtle).spacing(0.0, 0.0));
        }
        self.card = self.card.child(row);
        self.rows += 1;
        self
    }

    pub fn id(mut self, id: impl Into<UiId>) -> Self {
        self.card = self.card.id(id);
        self
    }
}

impl<Message> Component<Message> for SettingsListCard<Message> {
    fn into_element(self) -> super::Element<Message> {
        self.card.into_element()
    }
}

impl<Message> Component<Message> for SettingsCard<Message> {
    fn into_element(self) -> super::Element<Message> {
        self.0.into_element()
    }
}

/// A label, supporting text, and trailing Settings control.
pub struct SettingsRow<Message = String>(super::Element<Message>);

impl<Message> SettingsRow<Message> {
    pub fn new(
        theme: SemanticTheme,
        label: impl Into<String>,
        supporting_text: impl Into<String>,
    ) -> Self {
        Self::with_optional_leading(theme, label, supporting_text, None::<Spacer<Message>>)
    }

    pub fn with_leading(
        theme: SemanticTheme,
        label: impl Into<String>,
        supporting_text: impl Into<String>,
        leading: impl Component<Message>,
    ) -> Self {
        Self::with_optional_leading(theme, label, supporting_text, Some(leading))
    }

    fn with_optional_leading(
        theme: SemanticTheme,
        label: impl Into<String>,
        supporting_text: impl Into<String>,
        leading: Option<impl Component<Message>>,
    ) -> Self {
        let label = label.into();
        let supporting_text = supporting_text.into();
        let mut labels = Column::new()
            .grow(1.0)
            .gap(theme.spacing.compact)
            .child(Text::new(&label).color(theme.text.primary).wrap(true));
        if !supporting_text.is_empty() {
            labels = labels.child(
                Text::new(supporting_text)
                    .color(theme.text.secondary)
                    .wrap(true),
            );
        }
        let mut row = Row::new()
            .fill_width()
            .min_height(52.0)
            .gap(theme.spacing.content)
            .padding(Insets::all(theme.spacing.control))
            .align_items(Align::Center);
        if let Some(leading) = leading {
            row = row.child(leading);
        }
        Self(row.child(labels).into_element().accessibility_label(label))
    }

    pub fn trailing(mut self, control: impl Component<Message>) -> Self {
        self.0 = self.0.child(control);
        self
    }

    /// Makes the row itself activatable. Use only when this is semantically
    /// identical to activating its trailing control.
    pub fn activate(self, message: Message) -> AnyView<Message> {
        AnyView::new(self.0.message(message).semantic_role(SemanticRole::Button))
    }

    pub fn id(mut self, id: impl Into<UiId>) -> Self {
        self.0 = self.0.id(id);
        self
    }

    pub fn direction(mut self, direction: ReadingDirection) -> Self {
        if direction == ReadingDirection::RightToLeft {
            self.0.children.reverse();
        }
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
        let label = label.into();
        let control = Row::new()
            .width(420.0)
            .gap(theme.spacing.content)
            .align_items(Align::Center)
            .child(
                Slider::on_change(on_change, value)
                    .colors(
                        theme.surfaces.raised,
                        theme.accent.ordinary,
                        theme.text.primary,
                    )
                    .focus_border(theme.borders.focus)
                    .controller_focus_border(theme.borders.controller_focus)
                    .width(330.0)
                    .accessibility_label(label.clone()),
            )
            .child(
                Text::new(value_label)
                    .width(72.0)
                    .align(TextAlign::End)
                    .color(theme.text.primary),
            );
        Self(SettingsRow::new(theme, label, supporting_text).trailing(control))
    }

    pub fn id(mut self, id: impl Into<UiId>) -> Self {
        self.0 = self.0.id(id);
        self
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
        let label = label.into();
        let dropdown = Dropdown::new(toggle_message, selected, options)
            .expanded(expanded)
            .colors(
                theme.surfaces.raised,
                theme.surfaces.hover,
                theme.text.primary,
            )
            .controller_focus_border(theme.borders.controller_focus)
            .accessibility_label(label.clone());
        Self(
            SettingsRow::new(theme, label, supporting_text)
                .trailing(Container::new().width(180.0).child(dropdown)),
        )
    }

    pub fn id(mut self, id: impl Into<UiId>) -> Self {
        self.0 = self.0.id(id);
        self
    }
}

impl<Message> Component<Message> for SelectField<Message> {
    fn into_element(self) -> super::Element<Message> {
        self.0.into_element()
    }
}

/// Compact horizontal group for related actions. It wraps intrinsically when
/// the caller places it in a responsive parent rather than owning viewport math.
pub struct InlineButtonGroup<Message = String>(Row<Message>);

impl<Message> InlineButtonGroup<Message> {
    pub fn new(theme: SemanticTheme) -> Self {
        Self(
            Row::new()
                .gap(theme.spacing.control)
                .align_items(Align::Center),
        )
    }

    pub fn action(mut self, action: Button<Message>) -> Self {
        self.0 = self.0.child(action);
        self
    }

    pub fn child(mut self, child: impl Component<Message>) -> Self {
        self.0 = self.0.child(child);
        self
    }
}

impl<Message> Component<Message> for InlineButtonGroup<Message> {
    fn into_element(self) -> super::Element<Message> {
        self.0.into_element()
    }
}

/// Vertical form group with consistent semantic spacing.
pub struct FieldGroup<Message = String>(Column<Message>);

impl<Message> FieldGroup<Message> {
    pub fn new(theme: SemanticTheme) -> Self {
        Self(Column::new().fill_width().gap(theme.spacing.content))
    }

    pub fn field(mut self, field: impl Component<Message>) -> Self {
        self.0 = self.0.child(field);
        self
    }
}

impl<Message> Component<Message> for FieldGroup<Message> {
    fn into_element(self) -> super::Element<Message> {
        self.0.into_element()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SettingsStatusKind {
    Unavailable,
    Validation,
    RestartRequired,
    Error,
}

/// Honest inline status used by fields and settings rows.
pub struct SettingsStatus<Message = String>(super::Element<Message>);

impl<Message> SettingsStatus<Message> {
    pub fn new(theme: SemanticTheme, kind: SettingsStatusKind, message: impl Into<String>) -> Self {
        let message = message.into();
        let (mark, color, state) = match kind {
            SettingsStatusKind::Unavailable => ("—", theme.text.disabled, "unavailable"),
            SettingsStatusKind::Validation => ("!", theme.text.warning, "validation"),
            SettingsStatusKind::RestartRequired => ("↻", theme.text.accent, "restart required"),
            SettingsStatusKind::Error => ("!", theme.text.danger, "error"),
        };
        Self(
            Container::new()
                .padding(Insets::all(theme.spacing.control))
                .radius(theme.radii.control)
                .background(theme.surfaces.raised)
                .semantic_role(SemanticRole::Status)
                .accessibility_label(&message)
                .accessibility_state(state)
                .child(
                    Row::new()
                        .gap(theme.spacing.control)
                        .align_items(Align::Center)
                        .child(Text::new(mark).color(color).accessibility_hidden(true))
                        .child(Text::new(message).color(color).wrap(true)),
                )
                .into_element(),
        )
    }
}

impl<Message> Component<Message> for SettingsStatus<Message> {
    fn into_element(self) -> super::Element<Message> {
        self.0.into_element()
    }
}

/// A keyboard-order-preserving horizontal list of tabs.
pub struct TabList<Message = String>(super::Element<Message>);

impl<Message> TabList<Message> {
    pub fn new(
        theme: SemanticTheme,
        tabs: impl IntoIterator<Item = (impl Into<String>, Message, bool)>,
    ) -> Self {
        Self::with_panel(theme, tabs, "settings-tab-panel")
    }

    pub fn with_panel(
        theme: SemanticTheme,
        tabs: impl IntoIterator<Item = (impl Into<String>, Message, bool)>,
        panel_id: impl Into<UiId>,
    ) -> Self {
        let panel_id = panel_id.into();
        let tabs =
            tabs.into_iter().map(|(label, message, selected)| {
                let label = label.into();
                Container::new()
                    .height(38.0)
                    .message(message)
                    .interaction_backgrounds(theme.surfaces.hover, theme.surfaces.pressed)
                    .focus_border(theme.borders.focus)
                    .controller_focus_border(theme.borders.controller_focus)
                    .semantic_role(SemanticRole::Tab)
                    .accessibility_label(&label)
                    .accessibility_state(if selected { "selected" } else { "unselected" })
                    .accessibility_controls(panel_id.clone())
                    .child(
                        Column::new()
                            .gap(6.0)
                            .child(Text::new(label).color(if selected {
                                theme.accent.ordinary
                            } else {
                                theme.text.secondary
                            }))
                            .child(Container::new().height(2.0).fill_width().background(
                                if selected {
                                    theme.accent.ordinary
                                } else {
                                    theme.surfaces.window
                                },
                            )),
                    )
            });
        Self(
            Row::new()
                .gap(theme.spacing.section)
                .shrink(0.0)
                .children(tabs)
                .semantic_role(SemanticRole::TabList)
                .navigation_scope(NavigationScope::group()),
        )
    }

    pub fn id(mut self, id: impl Into<UiId>) -> Self {
        self.0 = self.0.id(id);
        self
    }

    pub fn direction(mut self, direction: ReadingDirection) -> Self {
        if direction == ReadingDirection::RightToLeft {
            self.0.children.reverse();
        }
        self
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
        Self::option(
            theme,
            message,
            label,
            None::<String>,
            selected,
            true,
            preview,
        )
    }

    pub fn option(
        theme: SemanticTheme,
        message: Message,
        label: impl Into<String>,
        description: Option<impl Into<String>>,
        selected: bool,
        enabled: bool,
        preview: impl Component<Message>,
    ) -> Self {
        let label = label.into();
        let description = description.map(Into::into);
        let minimum_height = if description.is_some() { 232.0 } else { 168.0 };
        let indicator = Container::new()
            .width(16.0)
            .height(16.0)
            .shrink(0.0)
            .radius(8.0)
            .border(
                if selected {
                    theme.accent.ordinary
                } else {
                    theme.text.secondary
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
                        theme.accent.ordinary
                    } else {
                        theme.surfaces.card
                    }),
            );
        let mut text = Column::new().gap(theme.spacing.compact).child(
            Text::new(label.clone())
                .color(if enabled {
                    theme.text.primary
                } else {
                    theme.text.disabled
                })
                .wrap(true),
        );
        if let Some(description) = description.as_ref() {
            text = text.child(
                Text::new(description.clone())
                    .color(theme.text.secondary)
                    .wrap(true),
            );
        }
        let state = match (enabled, selected) {
            (false, _) => "disabled",
            (true, true) => "selected",
            (true, false) => "unselected",
        };
        Self(
            Container::new()
                .min_width(180.0)
                .min_height(minimum_height)
                .gap(theme.spacing.content)
                .padding(Insets::all(theme.spacing.content))
                .background(theme.surfaces.card)
                .interaction_backgrounds(theme.surfaces.hover, theme.surfaces.pressed)
                .focus_border(theme.borders.focus)
                .controller_focus_border(theme.borders.controller_focus)
                .border(
                    if selected {
                        theme.accent.ordinary
                    } else {
                        theme.surfaces.raised
                    },
                    if selected { 2.0 } else { 1.0 },
                )
                .radius(theme.radii.card)
                .message(message)
                .enabled(enabled)
                .semantic_role(SemanticRole::Option)
                .accessibility_label(&label)
                .accessibility_state(state)
                .accessibility_description(description.unwrap_or_default())
                .child(preview)
                .child(
                    Row::new()
                        .gap(theme.spacing.control)
                        .align_items(Align::Center)
                        .child(indicator)
                        .child(text),
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
                .children(items)
                .semantic_role(SemanticRole::RadioGroup),
        )
    }

    pub fn fixed(columns: usize, items: impl IntoIterator<Item = ChoiceCard<Message>>) -> Self {
        Self(
            Grid::fixed(columns.max(1))
                .gap(16.0)
                .fill_width()
                .children(items)
                .semantic_role(SemanticRole::RadioGroup),
        )
    }

    pub fn direction(mut self, direction: ReadingDirection) -> Self {
        self.0 = self.0.direction(direction);
        self
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
        let label = label.into();
        let (mark, color) = match state {
            PreviewState::Loading => ("…", theme.text.secondary),
            PreviewState::Unavailable => ("—", theme.text.disabled),
            PreviewState::Error => ("!", theme.text.danger),
        };
        Self(
            Self::frame(theme)
                .semantic_role(SemanticRole::Status)
                .accessibility_label(&label)
                .accessibility_state(match state {
                    PreviewState::Loading => "loading",
                    PreviewState::Unavailable => "unavailable",
                    PreviewState::Error => "error",
                })
                .child(
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
        Self::color_labeled(theme, message, color, format!("#{color:06X}"), selected)
    }

    pub fn color_labeled(
        theme: SemanticTheme,
        message: Message,
        color: Color,
        label: impl Into<String>,
        selected: bool,
    ) -> Self {
        let ring = Container::new()
            .width(38.0)
            .height(38.0)
            .radius(19.0)
            .border(
                if selected {
                    theme.accent.ordinary
                } else {
                    theme.surfaces.raised
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
                .interaction_backgrounds(theme.surfaces.hover, theme.surfaces.pressed)
                .focus_border(theme.borders.focus)
                .controller_focus_border(theme.borders.controller_focus)
                .semantic_role(SemanticRole::Radio)
                .accessibility_label(label)
                .accessibility_state(if selected { "selected" } else { "unselected" })
                .child(ring),
        )
    }

    pub fn custom(theme: SemanticTheme, message: Message) -> Self {
        Self(
            Container::new()
                .width(42.0)
                .height(42.0)
                .radius(21.0)
                .border(theme.surfaces.raised, 1.0)
                .message(message)
                .interaction_backgrounds(theme.surfaces.hover, theme.surfaces.pressed)
                .focus_border(theme.borders.focus)
                .controller_focus_border(theme.borders.controller_focus)
                .semantic_role(SemanticRole::Button)
                .accessibility_label("Choose a custom color")
                .child(
                    Container::new().fill_width().child(
                        Text::new("+")
                            .align(TextAlign::Center)
                            .scale(1.45)
                            .color(theme.text.secondary),
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
    use crate::{
        DiagnosticKind, PaintCommand, Point, Rect, ResolvedNode, SdlComponentRenderer, UiEvent,
        UiFrame, UiStateStore,
    };

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
        Search(String),
    }

    fn toggle(value: bool) -> Message {
        Message::Toggle(value)
    }

    fn slide(value: f32) -> Message {
        Message::Slide(value)
    }

    fn search(value: String) -> Message {
        Message::Search(value)
    }

    fn theme() -> SemanticTheme {
        SemanticTheme::from_tokens(crate::SemanticTokenSet::standard(
            0x101114, 0x15171b, 0x1b1e23, 0x32363e, 0x3c414b, 0xf2f3f5, 0xa8abb2, 0x9b62e8,
            0x45305f, 0x55b982, 0x55b982,
        ))
    }

    #[test]
    fn switch_emits_requested_value() {
        let mut state = UiStateStore::default();
        let tree = UiFrame::layout(
            Switch::new(false, toggle, theme()).id("switch"),
            Rect::new(0.0, 0.0, 100.0, 50.0),
        );
        let bounds = tree
            .semantic_targets_for_message(&Message::Toggle(true))
            .into_iter()
            .next()
            .expect("enabled switch should expose its typed action")
            .bounds;
        let center = Point {
            x: bounds.origin.x + bounds.size.width * 0.5,
            y: bounds.origin.y + bounds.size.height * 0.5,
        };
        tree.handle_event(&mut state, UiEvent::PointerPressed(center));
        let outcome = tree.handle_event(&mut state, UiEvent::PointerReleased(center));
        assert_eq!(outcome.messages, vec![Message::Toggle(true)]);
    }

    #[test]
    fn switch_unavailable_and_disabled_states_cannot_activate() {
        for state in [
            SwitchState::MixedUnavailable,
            SwitchState::DisabledOff,
            SwitchState::DisabledOn,
        ] {
            let tree = UiFrame::layout(
                Switch::with_state(state, Some(toggle), theme()).id("switch"),
                Rect::new(0.0, 0.0, 100.0, 50.0),
            );
            assert!(
                tree.semantic_targets_for_message(&Message::Toggle(true))
                    .is_empty()
            );
            assert!(
                tree.semantic_targets_for_message(&Message::Toggle(false))
                    .is_empty()
            );
            assert!(tree.resolved_layout().nodes().iter().any(|node| {
                node.id.as_str().ends_with("switch") && !node.interaction.interactive
            }));
        }
    }

    #[test]
    fn settings_search_placeholder_keeps_an_empty_edit_value() {
        let mut state = UiStateStore::default();
        let tree = UiFrame::layout(
            SettingsSearchField::new(theme(), "search", "", "Search settings...", search),
            Rect::new(0.0, 0.0, 220.0, 48.0),
        );
        let input = tree
            .resolved_layout()
            .nodes()
            .iter()
            .find(|node| node.id.as_str().ends_with("/search"))
            .unwrap();
        let point = Point {
            x: input.allocated.origin.x + input.allocated.size.width / 2.0,
            y: input.allocated.origin.y + input.allocated.size.height / 2.0,
        };
        tree.handle_event(&mut state, UiEvent::PointerPressed(point));
        let outcome = tree.handle_event(&mut state, UiEvent::TextInput("net".into()));
        assert_eq!(outcome.messages, vec![Message::Search("net".into())]);
    }

    #[test]
    fn settings_search_ranks_controls_and_disambiguates_identical_names() {
        let entries = [
            SettingsSearchEntry::new(
                "Display",
                "Scale",
                "Automatic",
                "display-scale",
                Message::Navigate(0),
            ),
            SettingsSearchEntry::new(
                "Appearance",
                "Theme",
                "Automatic",
                "appearance-mode",
                Message::Navigate(1),
            ),
            SettingsSearchEntry::new(
                "Network",
                "Wireless",
                "Wi-Fi",
                "wifi-power",
                Message::Navigate(2),
            )
            .unavailable(),
        ];
        let automatic = search_settings("automatic", &entries);
        assert_eq!(automatic.len(), 2);
        assert_ne!(
            automatic[0].disambiguated_label(),
            automatic[1].disambiguated_label()
        );
        assert!(automatic[0].disambiguated_label().contains("Display"));
        let wifi = search_settings("wi", &entries);
        assert_eq!(wifi[0].control, "Wi-Fi");
        assert!(!wifi[0].available);
        assert!(search_settings("", &entries).is_empty());
        assert!(search_settings("missing", &entries).is_empty());
    }

    #[test]
    fn tabs_and_swatches_preserve_typed_activation() {
        let mut state = UiStateStore::default();
        let theme = theme();
        let tree = UiFrame::layout(
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
        for message in [Message::Tab(1), Message::Color(0), Message::Color(1)] {
            let bounds = tree
                .semantic_targets_for_message(&message)
                .into_iter()
                .next()
                .expect("enabled tab/swatch should expose its typed action")
                .bounds;
            let center = Point {
                x: bounds.origin.x + bounds.size.width * 0.5,
                y: bounds.origin.y + bounds.size.height * 0.5,
            };
            tree.handle_event(&mut state, UiEvent::PointerPressed(center));
            assert_eq!(
                tree.handle_event(&mut state, UiEvent::PointerReleased(center))
                    .messages,
                vec![message]
            );
        }
    }

    #[test]
    fn visual_choices_expose_one_semantic_target_and_disabled_choices_do_not_activate() {
        let theme = theme();
        let tree = UiFrame::layout(
            ChoiceCardGroup::new([
                ChoiceCard::option(
                    theme,
                    Message::Choice(0),
                    "Light",
                    Some("Bright surfaces"),
                    true,
                    true,
                    PreviewTile::new(theme, Text::new("Light preview")),
                )
                .id("light"),
                ChoiceCard::option(
                    theme,
                    Message::Choice(1),
                    "Unavailable",
                    Some("Not installed"),
                    false,
                    false,
                    PreviewTile::unavailable(theme, "Preview unavailable"),
                )
                .id("unavailable"),
            ]),
            Rect::new(0.0, 0.0, 420.0, 200.0),
        );
        assert!(
            !tree
                .semantic_targets_for_message(&Message::Choice(0))
                .is_empty()
        );
        assert!(
            tree.semantic_targets_for_message(&Message::Choice(1))
                .is_empty()
        );
        let options = tree
            .accessibility_nodes()
            .iter()
            .filter(|node| node.role.as_deref() == Some("option"))
            .collect::<Vec<_>>();
        assert_eq!(options.len(), 2);
        assert_eq!(options[0].state.as_deref(), Some("selected"));
        assert_eq!(options[1].state.as_deref(), Some("disabled"));
    }

    #[test]
    fn choice_hover_focus_and_selection_are_independent_visual_states() {
        let theme = theme();
        let view = || {
            ChoiceCard::new(
                theme,
                Message::Choice(0),
                "Selected choice",
                true,
                PreviewTile::new(theme, Text::new("Preview")).height(80.0),
            )
            .id("choice")
        };
        let bounds = Rect::new(0.0, 0.0, 240.0, 240.0);
        let mut state = UiStateStore::default();
        let initial = UiFrame::layout_with_state(view(), bounds, &mut state);
        let rect = initial
            .semantic_targets_for_message(&Message::Choice(0))
            .into_iter()
            .next()
            .unwrap()
            .bounds;
        let point = Point {
            x: rect.origin.x + rect.size.width / 2.0,
            y: rect.origin.y + rect.size.height / 2.0,
        };
        let _ = initial.handle_event(&mut state, UiEvent::PointerMoved(point));
        let hovered = UiFrame::layout_with_state(view(), bounds, &mut state);
        assert!(hovered.commands().iter().any(|command| matches!(
            command,
            PaintCommand::RoundedFill { color, .. } if *color == theme.surfaces.hover
        )));
        let _ = hovered.handle_event(&mut state, UiEvent::FocusNext);
        let focused = UiFrame::layout_with_state(view(), bounds, &mut state);
        assert!(focused.commands().iter().any(|command| matches!(
            command,
            PaintCommand::Stroke { color, .. } if *color == theme.borders.focus
        )));
        assert!(focused.accessibility_nodes().iter().any(|node| {
            node.role.as_deref() == Some("option") && node.state.as_deref() == Some("selected")
        }));
    }

    #[test]
    fn swatches_distinguish_color_selection_from_custom_activation() {
        let theme = theme();
        let tree = UiFrame::layout(
            Row::new()
                .child(ColorSwatch::color_labeled(
                    theme,
                    Message::Color(0),
                    0x3366ff,
                    "Ocean blue",
                    true,
                ))
                .child(ColorSwatch::custom(theme, Message::Color(1))),
            Rect::new(0.0, 0.0, 100.0, 50.0),
        );
        assert!(tree.accessibility_nodes().iter().any(|node| {
            node.role.as_deref() == Some("radio")
                && node.label.as_deref() == Some("Ocean blue")
                && node.state.as_deref() == Some("selected")
        }));
        assert!(tree.accessibility_nodes().iter().any(|node| {
            node.role.as_deref() == Some("button")
                && node.label.as_deref() == Some("Choose a custom color")
        }));
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
        let wide = UiFrame::layout(
            ChoiceCardGroup::new([make(0), make(1), make(2)]),
            Rect::new(0.0, 0.0, 640.0, 400.0),
        );
        let narrow = UiFrame::layout(
            ChoiceCardGroup::new([make(0), make(1), make(2)]),
            Rect::new(0.0, 0.0, 220.0, 600.0),
        );
        assert_eq!(wide.resolved_grid_columns(), Some(3));
        assert_eq!(narrow.resolved_grid_columns(), Some(1));
        assert!(
            !wide
                .semantic_targets_for_message(&Message::Choice(2))
                .is_empty()
        );
    }

    #[test]
    fn visual_choice_matrix_renders_narrow_wide_scaled_themed_and_directional() {
        let dark = theme();
        let light = SemanticTheme::from_tokens(crate::SemanticTokenSet::standard(
            0xf4f5f7, 0xe8eaf0, 0xffffff, 0xd7dae2, 0xe1e5ef, 0x17191f, 0x555b68, 0x7040bb,
            0xe5d8fa, 0x247a50, 0x247a50,
        ));
        for (width, scale, palette, direction) in [
            (220.0, 1.0, dark, ReadingDirection::LeftToRight),
            (640.0, 1.25, dark, ReadingDirection::RightToLeft),
            (640.0, 2.0, light, ReadingDirection::LeftToRight),
            (220.0, 2.0, light, ReadingDirection::RightToLeft),
        ] {
            let make = |choice| {
                ChoiceCard::option(
                    palette,
                    Message::Choice(choice),
                    format!("A deliberately long visual choice label {choice}"),
                    Some("Supporting text wraps without becoming a second activation target"),
                    choice == 0,
                    true,
                    PreviewTile::new(palette, Text::new(format!("Preview {choice}"))).height(96.0),
                )
            };
            let bounds = Rect::new(0.0, 0.0, width, 1000.0);
            let tree = UiFrame::layout_with_diagnostics(
                ChoiceCardGroup::new([make(0), make(1), make(2)]).direction(direction),
                bounds,
            );
            assert!(
                tree.diagnostics().is_empty(),
                "{width}@{scale}: {:#?}",
                tree.diagnostics()
            );
            let mut renderer = SdlComponentRenderer::new_pixel_buffer(
                (width * scale) as u32,
                (1000.0 * scale) as u32,
                scale,
            );
            renderer.render(tree.commands());
            assert!(renderer.pixels().iter().any(|pixel| pixel.a > 0));
            let physical_width = (width * scale) as u32;
            let physical_height = (1000.0 * scale) as u32;
            let image = image::ImageBuffer::<image::Rgba<u8>, Vec<u8>>::from_fn(
                physical_width,
                physical_height,
                |x, y| {
                    let pixel = renderer.pixels()[(y * physical_width + x) as usize];
                    image::Rgba([pixel.r, pixel.g, pixel.b, pixel.a])
                },
            );
            let mode = if palette.surfaces.window == dark.surfaces.window {
                "dark"
            } else {
                "light"
            };
            let direction_name = if direction == ReadingDirection::RightToLeft {
                "rtl"
            } else {
                "ltr"
            };
            let output = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/nickel-ui-snapshots")
                .join(format!(
                    "choices-{mode}-{direction_name}-{width:.0}-{scale:.2}x.png"
                ));
            std::fs::create_dir_all(output.parent().expect("snapshot parent")).unwrap();
            image.save(output).unwrap();
        }
    }

    #[test]
    fn preview_tile_renders_rounded_content_and_explicit_placeholder_states() {
        let theme = theme();
        let content = UiFrame::<Message>::layout(
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
            let tree = UiFrame::<Message>::layout(
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
        let tree = UiFrame::layout_with_diagnostics(
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
        assert!(
            !tree
                .semantic_targets_for_message(&Message::Toggle(false))
                .is_empty()
        );
        assert!(
            !tree
                .semantic_targets_for_message(&Message::Slide(0.85))
                .is_empty()
        );
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

    fn node_ending<'a>(tree: &'a UiFrame<Message>, suffix: &str) -> &'a ResolvedNode {
        tree.resolved_layout()
            .nodes()
            .iter()
            .find(|node| node.id.as_str().ends_with(suffix))
            .unwrap()
    }

    #[test]
    fn settings_shell_reflows_at_its_own_breakpoint() {
        let ordinary =
            UiFrame::layout_with_diagnostics(shell(1000.0), Rect::new(0.0, 0.0, 1000.0, 640.0));
        let narrow =
            UiFrame::layout_with_diagnostics(shell(560.0), Rect::new(0.0, 0.0, 560.0, 760.0));
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
        assert!(
            !ordinary
                .semantic_targets_for_message(&Message::Navigate(1))
                .is_empty()
        );
        assert!(
            !narrow
                .semantic_targets_for_message(&Message::Navigate(0))
                .is_empty()
        );
    }

    #[test]
    fn select_field_preserves_toggle_and_option_messages() {
        let theme = theme();
        let tree = UiFrame::layout(
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
        assert!(
            !tree
                .semantic_targets_for_message(&Message::ToggleSelect)
                .is_empty()
        );
        assert!(
            !tree
                .semantic_targets_for_message(&Message::Select(0))
                .is_empty()
        );
        assert!(
            !tree
                .semantic_targets_for_message(&Message::Select(1))
                .is_empty()
        );
    }

    #[test]
    fn list_field_and_status_composites_preserve_hierarchy_and_disjoint_actions() {
        let theme = theme();
        let actions = InlineButtonGroup::new(theme)
            .action(Button::semantic(
                theme,
                Message::Navigate(1),
                "Retry",
                crate::ButtonPresentation::Primary,
            ))
            .action(Button::semantic(
                theme,
                Message::Navigate(2),
                "Details",
                crate::ButtonPresentation::Secondary,
            ));
        let tree = UiFrame::layout_with_diagnostics(
            SettingsSection::new(theme, "Connectivity")
                .id("section")
                .child(
                    SettingsListCard::new(theme)
                        .id("list")
                        .row(
                            SettingsRow::with_leading(
                                theme,
                                "Network",
                                "Managed by your organization",
                                Text::new("N").accessibility_hidden(true),
                            )
                            .id("network-row")
                            .trailing(actions),
                        )
                        .row(
                            SettingsRow::new(theme, "Bluetooth", "Adapter unavailable").trailing(
                                SettingsStatus::new(
                                    theme,
                                    SettingsStatusKind::Unavailable,
                                    "No Bluetooth adapter",
                                ),
                            ),
                        ),
                ),
            Rect::new(0.0, 0.0, 720.0, 360.0),
        );
        assert!(tree.diagnostics().is_empty(), "{:?}", tree.diagnostics());
        let retry = tree
            .semantic_targets_for_message(&Message::Navigate(1))
            .into_iter()
            .next()
            .unwrap()
            .bounds;
        let details = tree
            .semantic_targets_for_message(&Message::Navigate(2))
            .into_iter()
            .next()
            .unwrap()
            .bounds;
        assert!(retry.origin.x + retry.size.width <= details.origin.x);
        assert!(tree.accessibility_nodes().iter().any(|node| {
            node.role.as_deref() == Some("status") && node.state.as_deref() == Some("unavailable")
        }));
    }

    #[test]
    fn tabs_expose_one_selection_and_their_controlled_panel() {
        let theme = theme();
        let panel = UiId::from("appearance-panel");
        let tree = UiFrame::layout(
            Column::new()
                .child(TabList::with_panel(
                    theme,
                    [
                        ("General", Message::Tab(0), true),
                        ("Fonts", Message::Tab(1), false),
                    ],
                    panel.clone(),
                ))
                .child(
                    SettingsCard::new(theme)
                        .id(panel.clone())
                        .semantic_role(SemanticRole::TabPanel),
                ),
            Rect::new(0.0, 0.0, 500.0, 180.0),
        );
        let tabs = tree
            .accessibility_nodes()
            .iter()
            .filter(|node| node.role.as_deref() == Some("tab"))
            .collect::<Vec<_>>();
        assert_eq!(tabs.len(), 2);
        assert_eq!(
            tabs.iter()
                .filter(|node| node.state.as_deref() == Some("selected"))
                .count(),
            1
        );
        assert!(
            tabs.iter()
                .all(|node| node.controls.as_ref() == Some(&panel))
        );
    }

    #[test]
    fn tab_list_owns_controller_enter_operate_and_exit_depth() {
        let mut state = UiStateStore::default();
        let tree = UiFrame::layout_with_state(
            TabList::new(
                theme(),
                [
                    ("General", Message::Tab(0), true),
                    ("Theme", Message::Tab(1), false),
                ],
            )
            .id("appearance-tabs"),
            Rect::new(0.0, 0.0, 400.0, 80.0),
            &mut state,
        );
        let tab_ids = tree
            .accessibility_nodes()
            .iter()
            .filter(|node| node.role.as_deref() == Some("tab"))
            .map(|node| node.id.clone())
            .collect::<Vec<_>>();
        assert_eq!(tab_ids.len(), 2);

        tree.handle_event(&mut state, UiEvent::ControllerDown);
        assert_eq!(
            state.navigation().controller_selected().map(UiId::as_str),
            Some("root/appearance-tabs")
        );

        tree.handle_event(&mut state, UiEvent::ControllerActivate);
        assert_eq!(
            state.navigation().controller_scope().map(UiId::as_str),
            Some("root/appearance-tabs")
        );
        assert_eq!(state.navigation().controller_selected(), Some(&tab_ids[0]));

        tree.handle_event(&mut state, UiEvent::ControllerRight);
        assert_eq!(state.navigation().controller_selected(), Some(&tab_ids[1]));
        let activation = tree.handle_event(&mut state, UiEvent::ControllerActivate);
        assert_eq!(activation.messages, vec![Message::Tab(1)]);

        tree.handle_event(&mut state, UiEvent::ControllerBack);
        assert_eq!(state.navigation().controller_scope(), None);
        assert_eq!(
            state.navigation().controller_selected().map(UiId::as_str),
            Some("root/appearance-tabs")
        );
    }

    fn localized_shell(
        width: f32,
        title: &str,
        description: &str,
        row_label: &str,
        direction: ReadingDirection,
    ) -> SettingsShell<Message> {
        let theme = theme();
        let navigation = SettingsNavigation::new(theme, 220.0)
            .id("navigation")
            .section(theme, title)
            .item(
                NavigationItem::with_leading_direction(
                    theme,
                    Message::Navigate(0),
                    row_label,
                    true,
                    Container::new()
                        .width(16.0)
                        .child(Text::new("I").accessibility_hidden(true)),
                    direction,
                )
                .id("destination"),
            );
        let row = SettingsRow::new(theme, row_label, description)
            .trailing(Switch::new(true, toggle, theme))
            .direction(direction);
        let content = SettingsSection::new(theme, title).child(
            SettingsListCard::new(theme)
                .row(row)
                .row(SettingsRow::new(theme, "1234", description).direction(direction)),
        );
        SettingsShell::new_directional(
            theme,
            width,
            navigation,
            PageHeader::new(theme, title, description).id("header"),
            content,
            direction,
        )
    }

    #[test]
    fn localized_directional_settings_fixtures_render_at_supported_sizes_and_scales() {
        let locales = [
            (
                "English",
                "Appearance",
                "Choose how Nickel looks and behaves",
                "Reduce transparency",
                ReadingDirection::LeftToRight,
            ),
            (
                "German",
                "Darstellung",
                "Wählen Sie aus, wie Nickel aussieht und sich verhält",
                "Transparenzeffekte reduzieren",
                ReadingDirection::LeftToRight,
            ),
            (
                "Chinese",
                "外观",
                "选择 Nickel 的外观和行为方式",
                "减少透明效果",
                ReadingDirection::LeftToRight,
            ),
            (
                "Spanish",
                "Apariencia",
                "Elige cómo se ve y se comporta Nickel",
                "Reducir la transparencia",
                ReadingDirection::LeftToRight,
            ),
            (
                "Synthetic",
                "A deliberately extended settings section heading",
                "A deliberately long supporting explanation that must wrap and grow instead of being truncated",
                "A deliberately extended control label",
                ReadingDirection::LeftToRight,
            ),
            (
                "RTL",
                "المظهر",
                "اختر كيفية ظهور نيكل وتصرفه 1234",
                "تقليل الشفافية",
                ReadingDirection::RightToLeft,
            ),
        ];
        for (name, title, description, label, direction) in locales {
            let bounds = Rect::new(0.0, 0.0, 1000.0, 720.0);
            let tree = UiFrame::layout_with_diagnostics(
                localized_shell(bounds.size.width, title, description, label, direction),
                bounds,
            );
            assert!(
                tree.diagnostics().is_empty(),
                "{name}: {:#?}",
                tree.diagnostics()
            );
        }

        for (width, height, scale) in [
            (560.0, 760.0, 1.0),
            (1000.0, 720.0, 1.0),
            (1400.0, 820.0, 1.0),
            (1000.0, 720.0, 1.25),
            (560.0, 760.0, 2.0),
            (1400.0, 820.0, 2.0),
        ] {
            let bounds = Rect::new(0.0, 0.0, width, height);
            let tree = UiFrame::layout_with_diagnostics(
                localized_shell(
                    width,
                    "Appearance",
                    "Choose how Nickel looks and behaves",
                    "Reduce transparency",
                    ReadingDirection::LeftToRight,
                ),
                bounds,
            );
            assert!(
                tree.diagnostics().is_empty(),
                "{width}x{height}: {:#?}",
                tree.diagnostics()
            );
            let mut renderer = SdlComponentRenderer::new_pixel_buffer(
                (width * scale) as u32,
                (height * scale) as u32,
                scale,
            );
            renderer.render(tree.commands());
            assert!(renderer.pixels().iter().any(|pixel| pixel.a > 0));
        }
    }

    #[test]
    fn right_to_left_shell_mirrors_sidebar_and_preserves_mixed_numerals() {
        let bounds = Rect::new(0.0, 0.0, 1000.0, 720.0);
        let ltr = UiFrame::layout(
            localized_shell(
                bounds.size.width,
                "Appearance",
                "Description",
                "Transparency",
                ReadingDirection::LeftToRight,
            ),
            bounds,
        );
        let rtl = UiFrame::layout(
            localized_shell(
                bounds.size.width,
                "المظهر",
                "الوصف 1234",
                "الشفافية",
                ReadingDirection::RightToLeft,
            ),
            bounds,
        );
        assert!(
            node_ending(&ltr, "/navigation").allocated.origin.x
                < node_ending(&ltr, "/header").allocated.origin.x
        );
        assert!(
            node_ending(&rtl, "/navigation").allocated.origin.x
                > node_ending(&rtl, "/header").allocated.origin.x
        );
        assert!(rtl.commands().iter().any(|command| matches!(
            command,
            PaintCommand::Text { text, .. } if text.contains("1234")
        )));
    }

    #[test]
    fn navigation_and_content_scroll_offsets_are_independent_and_clamped_on_rebuild() {
        let theme = theme();
        let make_navigation = |count| {
            (0..count).fold(
                SettingsNavigation::new(theme, 220.0).id("navigation-scroll"),
                |navigation, index| {
                    navigation.item(NavigationItem::new(
                        theme,
                        Message::Navigate(index),
                        format!("Destination {index}"),
                        false,
                    ))
                },
            )
        };
        let make_content = |count| {
            Container::new()
                .id("content-scroll")
                .fill_width()
                .fill_height()
                .overflow_y(Overflow::Auto)
                .children((0..count).map(|index| {
                    SettingsRow::new(theme, format!("Control {index}"), "Supporting text")
                }))
        };
        let bounds = Rect::new(0.0, 0.0, 900.0, 360.0);
        let mut state = UiStateStore::default();
        let tree = UiFrame::layout_with_state(
            SettingsShell::new(
                theme,
                bounds.size.width,
                make_navigation(18),
                PageHeader::new(theme, "Page", "Description"),
                make_content(18),
            ),
            bounds,
            &mut state,
        );
        let navigation = node_ending(&tree, "/navigation-scroll");
        let content = node_ending(&tree, "/content-scroll");
        let center = |rect: Rect| Point {
            x: rect.origin.x + rect.size.width / 2.0,
            y: rect.origin.y + rect.size.height / 2.0,
        };
        tree.handle_event(
            &mut state,
            UiEvent::Scroll {
                point: center(navigation.allocated),
                delta_y: 180.0,
            },
        );
        let navigation_offset = state
            .state(&navigation.id)
            .expect("navigation scroll state")
            .scroll_offset;
        assert!(navigation_offset > 0.0);
        assert_eq!(
            state.state(&content.id).map(|entry| entry.scroll_offset),
            Some(0.0)
        );
        tree.handle_event(
            &mut state,
            UiEvent::Scroll {
                point: center(content.allocated),
                delta_y: 220.0,
            },
        );
        assert_eq!(
            state
                .state(&navigation.id)
                .expect("navigation scroll state")
                .scroll_offset,
            navigation_offset
        );
        assert!(
            state
                .state(&content.id)
                .expect("content scroll state")
                .scroll_offset
                > 0.0
        );

        let rebuilt = UiFrame::layout_with_state(
            SettingsShell::new(
                theme,
                bounds.size.width,
                make_navigation(1),
                PageHeader::new(theme, "Page", "Description"),
                make_content(1),
            ),
            bounds,
            &mut state,
        );
        let rebuilt_navigation = node_ending(&rebuilt, "/navigation-scroll");
        let rebuilt_content = node_ending(&rebuilt, "/content-scroll");
        assert_eq!(
            state.state(&rebuilt_navigation.id).unwrap().scroll_offset,
            0.0
        );
        assert_eq!(state.state(&rebuilt_content.id).unwrap().scroll_offset, 0.0);
    }

    #[test]
    fn settings_navigation_owns_its_controller_pane_glow() {
        let theme = theme();
        let build = || {
            SettingsNavigation::new(theme, 220.0).item(NavigationItem::new(
                theme,
                Message::Navigate(0),
                "Display",
                true,
            ))
        };
        let bounds = Rect::new(0.0, 0.0, 220.0, 360.0);
        let mut state = UiStateStore::default();
        let initial = UiFrame::layout_with_state(build(), bounds, &mut state);
        let sidebar = initial
            .resolved_layout()
            .nodes()
            .iter()
            .find(|node| {
                node.navigation_scope
                    .as_ref()
                    .is_some_and(|scope| scope.pane)
            })
            .expect("SettingsNavigation should be a controller pane")
            .id
            .clone();
        state.navigation_mut().set_controller_pane(Some(sidebar));

        let selected = UiFrame::layout_with_state(build(), bounds, &mut state);
        assert!(selected.commands().iter().any(|command| matches!(
            command,
            PaintCommand::Stroke { color, width, .. }
                if *color == theme.borders.controller_focus && *width == 3.0
        )));
    }

    #[test]
    fn stable_navigation_focus_survives_filtering_and_rebuild() {
        let theme = theme();
        let build = |include_first| {
            let mut navigation = SettingsNavigation::new(theme, 220.0);
            if include_first {
                navigation = navigation.item(
                    NavigationItem::new(theme, Message::Navigate(0), "Display", false)
                        .id("display"),
                );
            }
            navigation.item(
                NavigationItem::new(theme, Message::Navigate(1), "Appearance", true)
                    .id("appearance"),
            )
        };
        let bounds = Rect::new(0.0, 0.0, 220.0, 300.0);
        let mut state = UiStateStore::default();
        let initial = UiFrame::layout_with_state(build(true), bounds, &mut state);
        let focused = node_ending(&initial, "/appearance").id.clone();
        initial.handle_event(&mut state, UiEvent::AccessibilityFocus(focused.clone()));
        let rebuilt = UiFrame::layout_with_state(build(false), bounds, &mut state);
        assert_eq!(state.focused(), Some(&focused));
        assert!(node_ending(&rebuilt, "/appearance").interaction.interactive);
    }
}
