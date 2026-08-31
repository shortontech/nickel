use crate::{Align, ControllerFamily, Insets, Justify, SemanticTheme};

use super::{
    AnyView, Button, ButtonPresentation, Column, Component, ComponentBuilderExt, Container,
    Element, Row, Spacer, Text, TextField,
};

pub const START_MENU_SINGLE_PANE_BREAKPOINT: f32 = 620.0;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ReadingDirection {
    #[default]
    LeftToRight,
    RightToLeft,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SemanticControllerAction {
    Confirm,
    Cancel,
    ContextMenu,
    Pin,
    Unpin,
    PreviousSection,
    NextSection,
    ToggleLauncher,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ControllerControlPresentation {
    pub glyph: &'static str,
    pub spoken_name: &'static str,
}

impl ControllerFamily {
    pub fn presentation(self, action: SemanticControllerAction) -> ControllerControlPresentation {
        use ControllerFamily::{Generic, PlayStation, Switch, Xbox};
        use SemanticControllerAction::{
            Cancel, Confirm, ContextMenu, NextSection, Pin, PreviousSection, ToggleLauncher, Unpin,
        };

        match (self, action) {
            (PlayStation, Confirm) => control("×", "Cross"),
            (PlayStation, Cancel) => control("○", "Circle"),
            (PlayStation, ContextMenu) => control("☰", "Options"),
            (PlayStation, Pin | Unpin) => control("□", "Square"),
            (PlayStation, PreviousSection) => control("L1", "L1"),
            (PlayStation, NextSection) => control("R1", "R1"),
            (PlayStation, ToggleLauncher) => control("PS", "PS button"),
            (Xbox, Confirm) => control("A", "A button"),
            (Xbox, Cancel) => control("B", "B button"),
            (Xbox, ContextMenu) => control("☰", "Menu button"),
            (Xbox, Pin | Unpin) => control("X", "X button"),
            (Xbox, PreviousSection) => control("LB", "left bumper"),
            (Xbox, NextSection) => control("RB", "right bumper"),
            (Xbox, ToggleLauncher) => control("X", "Xbox button"),
            (Switch, Confirm) => control("A", "A button"),
            (Switch, Cancel) => control("B", "B button"),
            (Switch, ContextMenu) => control("+", "Plus button"),
            (Switch, Pin | Unpin) => control("Y", "Y button"),
            (Switch, PreviousSection) => control("L", "L button"),
            (Switch, NextSection) => control("R", "R button"),
            (Switch, ToggleLauncher) => control("⌂", "Home button"),
            (Generic, Confirm) => control("OK", "confirm control"),
            (Generic, Cancel) => control("←", "cancel control"),
            (Generic, ContextMenu) => control("…", "menu control"),
            (Generic, Pin | Unpin) => control("◇", "secondary action control"),
            (Generic, PreviousSection) => control("L", "previous section control"),
            (Generic, NextSection) => control("R", "next section control"),
            (Generic, ToggleLauncher) => control("⌂", "launcher control"),
        }
    }
}

const fn control(glyph: &'static str, spoken_name: &'static str) -> ControllerControlPresentation {
    ControllerControlPresentation { glyph, spoken_name }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionLegendEntry {
    pub action: SemanticControllerAction,
    pub label: String,
    pub available: bool,
}

impl ActionLegendEntry {
    pub fn available(action: SemanticControllerAction, label: impl Into<String>) -> Self {
        Self {
            action,
            label: label.into(),
            available: true,
        }
    }

    pub fn unavailable(action: SemanticControllerAction, label: impl Into<String>) -> Self {
        Self {
            action,
            label: label.into(),
            available: false,
        }
    }
}

/// Stable, non-interactive presentation of the semantic actions accepted by
/// the active controller navigation scope.
pub struct ActionLegend<Message = String>(Container<Message>);

impl<Message> ActionLegend<Message> {
    pub fn new(
        theme: SemanticTheme,
        family: ControllerFamily,
        entries: impl IntoIterator<Item = ActionLegendEntry>,
    ) -> Self {
        Self::new_directional(theme, family, entries, ReadingDirection::LeftToRight)
    }

    pub fn new_directional(
        theme: SemanticTheme,
        family: ControllerFamily,
        entries: impl IntoIterator<Item = ActionLegendEntry>,
        direction: ReadingDirection,
    ) -> Self {
        let items = entries
            .into_iter()
            .filter(|entry| entry.available)
            .map(|entry| {
                let presentation = family.presentation(entry.action);
                let accessible = format!("{}: {}", presentation.spoken_name, entry.label);
                Row::new()
                    .min_height(theme.sizing.control_height)
                    .padding(Insets::symmetric(
                        theme.spacing.control,
                        theme.spacing.compact,
                    ))
                    .gap(theme.spacing.compact)
                    .align_items(Align::Center)
                    .accessibility_label(accessible)
                    .child(
                        Container::new()
                            .min_width(26.0)
                            .min_height(26.0)
                            .radius(theme.radii.control)
                            .border(theme.borders.controller_focus, 1.5)
                            .align_items(Align::Center)
                            .justify_content(Justify::Center)
                            .child(
                                Text::new(presentation.glyph)
                                    .color(theme.borders.controller_focus)
                                    .scale(0.82),
                            ),
                    )
                    .child(Text::new(entry.label).color(theme.text.secondary))
            })
            .collect::<Vec<Element<Message>>>();
        let mut row = Row::new()
            .fill_width()
            .min_height(theme.sizing.control_height + theme.spacing.control * 2.0)
            .gap(theme.spacing.content)
            .align_items(Align::Center)
            .children(items);
        if direction == ReadingDirection::RightToLeft {
            row = row.reverse();
        }
        Self(
            Container::new()
                .fill_width()
                .background(theme.surfaces.raised)
                .border(theme.borders.subtle, 1.0)
                .child(row),
        )
    }
}

impl<Message> Component<Message> for ActionLegend<Message> {
    fn into_element(self) -> Element<Message> {
        self.0.into_element()
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum StartMenuNarrowPane {
    #[default]
    Primary,
    Detail,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ShortcutState {
    pub selected: bool,
    pub focused: bool,
    pub hovered: bool,
    pub pressed: bool,
    pub enabled: bool,
}

impl Default for ShortcutState {
    fn default() -> Self {
        Self {
            selected: false,
            focused: false,
            hovered: false,
            pressed: false,
            enabled: true,
        }
    }
}

pub struct StartMenuShell<Message = String> {
    theme: SemanticTheme,
    available_width: f32,
    header: Option<AnyView<Message>>,
    primary: AnyView<Message>,
    detail: AnyView<Message>,
    primary_footer: Option<AnyView<Message>>,
    detail_footer: Option<AnyView<Message>>,
    legend: Option<AnyView<Message>>,
    direction: ReadingDirection,
    narrow_pane: StartMenuNarrowPane,
}

impl<Message> StartMenuShell<Message> {
    pub fn new(
        theme: SemanticTheme,
        available_width: f32,
        primary: impl Component<Message>,
        detail: impl Component<Message>,
    ) -> Self {
        Self {
            theme,
            available_width,
            header: None,
            primary: AnyView::new(primary),
            detail: AnyView::new(detail),
            primary_footer: None,
            detail_footer: None,
            legend: None,
            direction: ReadingDirection::LeftToRight,
            narrow_pane: StartMenuNarrowPane::Primary,
        }
    }

    pub fn header(mut self, header: impl Component<Message>) -> Self {
        self.header = Some(AnyView::new(header));
        self
    }

    pub fn primary_footer(mut self, footer: impl Component<Message>) -> Self {
        self.primary_footer = Some(AnyView::new(footer));
        self
    }

    pub fn detail_footer(mut self, footer: impl Component<Message>) -> Self {
        self.detail_footer = Some(AnyView::new(footer));
        self
    }

    /// Adds a stable surface-level action legend below both navigation panes.
    pub fn legend(mut self, legend: impl Component<Message>) -> Self {
        self.legend = Some(AnyView::new(legend));
        self
    }

    pub fn direction(mut self, direction: ReadingDirection) -> Self {
        self.direction = direction;
        self
    }

    pub fn narrow_pane(mut self, pane: StartMenuNarrowPane) -> Self {
        self.narrow_pane = pane;
        self
    }
}

impl<Message> Component<Message> for StartMenuShell<Message> {
    fn into_element(self) -> super::Element<Message> {
        let Self {
            theme,
            available_width,
            header,
            primary,
            detail,
            primary_footer,
            detail_footer,
            legend,
            direction,
            narrow_pane,
        } = self;
        let pane = |content: AnyView<Message>, footer: Option<AnyView<Message>>| {
            let mut pane = Column::new().fill_width().fill_height().child(content);
            if let Some(footer) = footer {
                pane = pane.child(Spacer::flex()).child(footer);
            }
            pane
        };
        let narrow = available_width < START_MENU_SINGLE_PANE_BREAKPOINT;
        let content = if narrow {
            let (content, footer) = match narrow_pane {
                StartMenuNarrowPane::Primary => (primary, primary_footer),
                StartMenuNarrowPane::Detail => (detail, detail_footer),
            };
            let mut column = Column::new()
                .fill_width()
                .fill_height()
                .grow(1.0)
                .min_height(0.0);
            if let Some(header) = header {
                column = column.child(header);
            }
            column = column.child(content);
            if let Some(footer) = footer {
                column = column.child(Spacer::flex()).child(footer);
            }
            AnyView::new(column)
        } else {
            let primary = pane(primary, primary_footer);
            let mut detail_pane = Column::new()
                .fill_width()
                .fill_height()
                .grow(1.0)
                .min_height(0.0);
            if let Some(header) = header {
                detail_pane = detail_pane.child(header);
            }
            detail_pane = detail_pane.child(detail);
            if let Some(footer) = detail_footer {
                detail_pane = detail_pane.child(Spacer::flex()).child(footer);
            }
            let panes = Row::new()
                .fill_width()
                .fill_height()
                .grow(1.0)
                .min_height(0.0)
                .gap(theme.spacing.content)
                .child(
                    Container::new()
                        .id("start-menu-primary-pane")
                        .width(280.0)
                        .navigation_scope(crate::NavigationScope::pane(false))
                        .navigation_scope_highlight(theme.borders.controller_focus)
                        .child(primary),
                )
                .child(
                    Container::new()
                        .id("start-menu-detail-pane")
                        .grow(1.0)
                        .navigation_scope(crate::NavigationScope::pane(true))
                        .navigation_scope_highlight(theme.borders.controller_focus)
                        .border(theme.borders.subtle, theme.sizing.border)
                        .child(detail_pane),
                );
            AnyView::new(if direction == ReadingDirection::RightToLeft {
                panes.reverse()
            } else {
                panes
            })
        };
        let shell_padding = if narrow {
            theme.spacing.compact
        } else {
            theme.spacing.content
        };
        let mut root = Column::new().fill_width().fill_height().child(content);
        if let Some(legend) = legend {
            root = root.child(legend);
        }
        Container::new()
            .fill_width()
            .fill_height()
            .padding(Insets::all(shell_padding))
            .background(theme.surfaces.window)
            .border(theme.borders.ordinary, theme.sizing.border)
            .radius(theme.radii.card)
            .child(root)
            .into_element()
    }
}

pub struct SectionHeader<Message = String> {
    theme: SemanticTheme,
    title: String,
    action: Option<(String, Message)>,
    direction: ReadingDirection,
}

impl<Message> SectionHeader<Message> {
    pub fn new(theme: SemanticTheme, title: impl Into<String>) -> Self {
        Self {
            theme,
            title: title.into(),
            action: None,
            direction: ReadingDirection::LeftToRight,
        }
    }

    pub fn action(
        mut self,
        _theme: SemanticTheme,
        label: impl Into<String>,
        message: Message,
    ) -> Self {
        self.action = Some((label.into(), message));
        self
    }

    pub fn direction(mut self, direction: ReadingDirection) -> Self {
        self.direction = direction;
        self
    }
}

impl<Message> Component<Message> for SectionHeader<Message> {
    fn into_element(self) -> super::Element<Message> {
        let mut row = Row::new()
            .fill_width()
            .min_height(36.0)
            .align_items(Align::Center)
            .child(
                Text::new(self.title)
                    .color(self.theme.text.secondary)
                    .scale(0.86),
            );
        if let Some((label, message)) = self.action {
            let chevron = if self.direction == ReadingDirection::RightToLeft {
                '‹'
            } else {
                '›'
            };
            row = row.child(Spacer::flex()).child(Button::semantic(
                self.theme,
                message,
                format!("{label}  {chevron}"),
                ButtonPresentation::Quiet,
            ));
        }
        if self.direction == ReadingDirection::RightToLeft {
            row = row.reverse();
        }
        row.into_element()
    }
}

pub struct ShortcutRow<Message = String>(Container<Message>);

impl<Message> ShortcutRow<Message> {
    pub fn new(
        theme: SemanticTheme,
        icon: impl Component<Message>,
        label: impl Into<String>,
        supporting: impl Into<String>,
        message: Option<Message>,
        selected: bool,
    ) -> Self {
        Self::new_directional(
            theme,
            icon,
            label,
            supporting,
            message,
            ShortcutState {
                selected,
                ..ShortcutState::default()
            },
            ReadingDirection::LeftToRight,
        )
    }

    pub fn new_directional(
        theme: SemanticTheme,
        icon: impl Component<Message>,
        label: impl Into<String>,
        supporting: impl Into<String>,
        message: Option<Message>,
        state: ShortcutState,
        direction: ReadingDirection,
    ) -> Self {
        let label = label.into();
        let supporting = supporting.into();
        let unavailable = message.is_none();
        let mut labels = Column::new()
            .grow(1.0)
            .gap(theme.spacing.compact)
            .child(Text::new(&label).color(theme.text.primary).wrap(true));
        if !supporting.is_empty() {
            labels = labels.child(
                Text::new(&supporting)
                    .color(theme.text.secondary)
                    .scale(0.9)
                    .wrap(true),
            );
        }
        let content = Row::new()
            .fill_width()
            .gap(theme.spacing.content)
            .align_items(Align::Center)
            .child(
                Container::new()
                    .width(36.0)
                    .min_height(36.0)
                    .shrink(0.0)
                    .align_items(Align::Center)
                    .justify_content(Justify::Center)
                    .child(icon),
            )
            .child(labels);
        let content = if direction == ReadingDirection::RightToLeft {
            content.reverse()
        } else {
            content
        };
        let background = if state.pressed {
            theme.surfaces.pressed
        } else if state.selected {
            theme.surfaces.selected
        } else if state.hovered {
            theme.surfaces.hover
        } else {
            theme.surfaces.window
        };
        let semantic_state = if !state.enabled {
            "disabled"
        } else if unavailable {
            "unavailable"
        } else if state.pressed {
            "pressed"
        } else if state.focused {
            "focused"
        } else if state.selected {
            "selected"
        } else if state.hovered {
            "hovered"
        } else {
            "available"
        };
        let mut row = Container::new()
            .fill_width()
            .min_height(52.0)
            .padding(Insets::all(theme.spacing.control))
            .radius(theme.radii.control)
            .background(background)
            .accessibility_label(label)
            .accessibility_description(&supporting)
            .accessibility_state(semantic_state)
            .enabled(state.enabled)
            .controller_focus_border(theme.borders.controller_focus)
            .child(content);
        if state.focused {
            row = row.border(theme.borders.focus, 2.0);
        }
        if state.enabled
            && let Some(message) = message
        {
            row = row.message(message);
        }
        Self(row)
    }

    pub fn accessibility_state(mut self, state: impl Into<String>) -> Self {
        self.0 = self.0.accessibility_state(state);
        self
    }

    pub fn context_message(mut self, message: Message) -> Self {
        self.0 = self.0.context_message(message);
        self
    }
}

impl<Message> Component<Message> for ShortcutRow<Message> {
    fn into_element(self) -> super::Element<Message> {
        self.0.into_element()
    }
}

pub struct ProjectStatusRow<Message = String>(ShortcutRow<Message>);

impl<Message> ProjectStatusRow<Message> {
    pub fn new(
        theme: SemanticTheme,
        icon: impl Component<Message>,
        name: impl Into<String>,
        status: impl Into<String>,
        chat_count: Option<usize>,
        message: Option<Message>,
        selected: bool,
    ) -> Self {
        Self::new_directional(
            theme,
            icon,
            name,
            status,
            chat_count,
            message,
            ShortcutState {
                selected,
                ..ShortcutState::default()
            },
            ReadingDirection::LeftToRight,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_directional(
        theme: SemanticTheme,
        icon: impl Component<Message>,
        name: impl Into<String>,
        status: impl Into<String>,
        chat_count: Option<usize>,
        message: Option<Message>,
        state: ShortcutState,
        direction: ReadingDirection,
    ) -> Self {
        let status = status.into();
        let supporting = match chat_count {
            Some(count) => format!(
                "{status} · {count} {}",
                if count == 1 { "chat" } else { "chats" }
            ),
            None => status.clone(),
        };
        Self(
            ShortcutRow::new_directional(theme, icon, name, supporting, message, state, direction)
                .accessibility_state(status),
        )
    }
}

impl<Message> Component<Message> for ProjectStatusRow<Message> {
    fn into_element(self) -> super::Element<Message> {
        self.0.into_element()
    }
}

pub struct AccountSummaryRow<Message = String>(ShortcutRow<Message>);

pub struct FallbackAvatar<Message = String>(Container<Message>);

impl<Message> FallbackAvatar<Message> {
    pub fn new(theme: SemanticTheme, display_name: &str) -> Self {
        let initials = fallback_avatar_initials(display_name);
        Self(
            Container::new()
                .width(36.0)
                .height(36.0)
                .shrink(0.0)
                .radius(18.0)
                .background(theme.surfaces.selected)
                .align_items(Align::Center)
                .justify_content(Justify::Center)
                .accessibility_label(format!("{display_name} avatar"))
                .child(Text::new(initials).color(theme.text.primary).scale(0.9)),
        )
    }
}

impl<Message> Component<Message> for FallbackAvatar<Message> {
    fn into_element(self) -> super::Element<Message> {
        self.0.into_element()
    }
}

fn fallback_avatar_initials(display_name: &str) -> String {
    let initials = display_name
        .split_whitespace()
        .filter_map(|part| part.chars().next())
        .take(2)
        .flat_map(char::to_uppercase)
        .collect::<String>();
    if initials.is_empty() {
        "•".to_owned()
    } else {
        initials
    }
}

impl<Message> AccountSummaryRow<Message> {
    pub fn new(
        theme: SemanticTheme,
        avatar: impl Component<Message>,
        display_name: impl Into<String>,
        supporting: impl Into<String>,
        message: Option<Message>,
        selected: bool,
    ) -> Self {
        Self::new_directional(
            theme,
            avatar,
            display_name,
            supporting,
            message,
            ShortcutState {
                selected,
                ..ShortcutState::default()
            },
            ReadingDirection::LeftToRight,
        )
    }

    pub fn new_directional(
        theme: SemanticTheme,
        avatar: impl Component<Message>,
        display_name: impl Into<String>,
        supporting: impl Into<String>,
        message: Option<Message>,
        state: ShortcutState,
        direction: ReadingDirection,
    ) -> Self {
        Self(ShortcutRow::new_directional(
            theme,
            avatar,
            display_name,
            supporting,
            message,
            state,
            direction,
        ))
    }
}

impl<Message> Component<Message> for AccountSummaryRow<Message> {
    fn into_element(self) -> super::Element<Message> {
        self.0.into_element()
    }
}

pub struct SessionActionRow<Message = String>(ShortcutRow<Message>);

impl<Message> SessionActionRow<Message> {
    pub fn new(
        theme: SemanticTheme,
        icon: impl Component<Message>,
        label: impl Into<String>,
        message: Message,
        _destructive: bool,
        selected: bool,
    ) -> Self {
        Self::new_directional(
            theme,
            icon,
            label,
            message,
            _destructive,
            ShortcutState {
                selected,
                ..ShortcutState::default()
            },
            ReadingDirection::LeftToRight,
        )
    }

    pub fn new_directional(
        theme: SemanticTheme,
        icon: impl Component<Message>,
        label: impl Into<String>,
        message: Message,
        _destructive: bool,
        state: ShortcutState,
        direction: ReadingDirection,
    ) -> Self {
        Self(ShortcutRow::new_directional(
            theme,
            icon,
            label,
            "",
            Some(message),
            state,
            direction,
        ))
    }
}

impl<Message> Component<Message> for SessionActionRow<Message> {
    fn into_element(self) -> super::Element<Message> {
        self.0.into_element()
    }
}

pub struct LauncherSearchField<Message = String>(Container<Message>);

impl<Message> LauncherSearchField<Message> {
    pub fn new(
        theme: SemanticTheme,
        icon: impl Component<Message>,
        query: &str,
        preedit: &str,
        placeholder: impl Into<String>,
        on_change: fn(String) -> Message,
    ) -> Self {
        Self::new_directional(
            theme,
            icon,
            query,
            preedit,
            placeholder,
            on_change,
            ReadingDirection::LeftToRight,
        )
    }

    pub fn new_directional(
        theme: SemanticTheme,
        icon: impl Component<Message>,
        query: &str,
        preedit: &str,
        placeholder: impl Into<String>,
        on_change: fn(String) -> Message,
        direction: ReadingDirection,
    ) -> Self {
        let mut displayed = query.to_owned();
        displayed.push_str(preedit);
        let placeholder = placeholder.into();
        let content = Row::new()
            .fill_width()
            .align_items(Align::Center)
            .gap(theme.spacing.control)
            .child(icon)
            .child(
                TextField::on_change_with_placeholder(&displayed, &placeholder, on_change)
                    .color(theme.text.primary)
                    .grow(1.0),
            );
        let content = if direction == ReadingDirection::RightToLeft {
            content.reverse()
        } else {
            content
        };
        Self(
            Container::new()
                .fill_width()
                .min_height(theme.sizing.control_height)
                .padding(Insets::all(theme.spacing.control))
                .background(theme.surfaces.raised)
                .border(theme.borders.ordinary, theme.sizing.border)
                .radius(theme.radii.control)
                .accessibility_label(&placeholder)
                .accessibility_state(if preedit.is_empty() {
                    "committed"
                } else {
                    "composing"
                })
                .child(content),
        )
    }
}

impl<Message> Component<Message> for LauncherSearchField<Message> {
    fn into_element(self) -> super::Element<Message> {
        self.0.into_element()
    }
}

pub struct CompactIconTile<Message = String>(Container<Message>);

impl<Message> CompactIconTile<Message> {
    pub fn new(
        theme: SemanticTheme,
        icon: impl Component<Message>,
        label: impl Into<String>,
        message: Message,
    ) -> Self {
        Self(
            Container::new()
                .width(48.0)
                .height(48.0)
                .padding(Insets::all(theme.spacing.compact))
                .radius(theme.radii.control)
                .background(theme.surfaces.raised)
                .align_items(Align::Center)
                .justify_content(Justify::Center)
                .accessibility_label(label)
                .message(message)
                .child(icon),
        )
    }
}

impl<Message> Component<Message> for CompactIconTile<Message> {
    fn into_element(self) -> super::Element<Message> {
        self.0.into_element()
    }
}

#[cfg(test)]
mod tests {
    use crate::{PaintCommand, Point, Rect, SemanticColors, UiEvent, UiFrame, UiStateStore};

    use super::*;

    #[derive(Clone, Debug, Eq, PartialEq)]
    enum Message {
        Open,
        SeeAll,
        Query(String),
    }

    fn query(value: String) -> Message {
        Message::Query(value)
    }

    fn actionable_row() -> ShortcutRow<Message> {
        ShortcutRow::new(
            theme(),
            Text::new("□"),
            "Nickel",
            "Desktop shell",
            Some(Message::Open),
            false,
        )
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
            secondary_accent: 0x55b982,
            positive: 0x55b982,
        })
    }

    fn light_theme() -> SemanticTheme {
        SemanticTheme::new(SemanticColors {
            window: 0xf4f5f7,
            sidebar: 0xe7e9ed,
            card: 0xffffff,
            raised: 0xdfe3e8,
            hover: 0xd4d9e0,
            primary_text: 0x17191d,
            secondary_text: 0x555b66,
            accent: 0x7440bd,
            accent_soft: 0xe5d8f7,
            secondary_accent: 0x207a4b,
            positive: 0x207a4b,
        })
    }

    fn component_state_sheet(
        theme: SemanticTheme,
        project_name: &str,
        supporting: &str,
        direction: ReadingDirection,
    ) -> impl Component<Message> {
        let primary = Column::new()
            .gap(theme.spacing.compact)
            .child(SectionHeader::new(theme, "APPLICATIONS").direction(direction))
            .child(ShortcutRow::new_directional(
                theme,
                Text::new("◆"),
                "Selected shortcut",
                supporting,
                Some(Message::Open),
                ShortcutState {
                    selected: true,
                    ..ShortcutState::default()
                },
                direction,
            ))
            .child(ShortcutRow::new_directional(
                theme,
                Text::new("◇"),
                "Unavailable shortcut",
                supporting,
                None::<Message>,
                ShortcutState::default(),
                direction,
            ))
            .child(CompactIconTile::new(
                theme,
                Text::new("N"),
                "Nickel",
                Message::Open,
            ));
        let detail = Column::new()
            .gap(theme.spacing.compact)
            .child(
                SectionHeader::new(theme, "PROJECTS")
                    .action(theme, "See all projects", Message::SeeAll)
                    .direction(direction),
            )
            .child(ProjectStatusRow::new_directional(
                theme,
                Text::new("□"),
                project_name,
                "Active",
                Some(2),
                Some(Message::Open),
                ShortcutState {
                    focused: true,
                    ..ShortcutState::default()
                },
                direction,
            ))
            .child(AccountSummaryRow::new_directional(
                theme,
                FallbackAvatar::new(theme, "Steven Shorton"),
                "Steven Shorton",
                supporting,
                Some(Message::Open),
                ShortcutState::default(),
                direction,
            ));
        let primary_footer = SessionActionRow::new_directional(
            theme,
            Text::new("↪"),
            "Log out",
            Message::Open,
            true,
            ShortcutState::default(),
            direction,
        );
        let detail_footer = LauncherSearchField::new_directional(
            theme,
            Text::new("⌕"),
            "nick",
            "el",
            "Search applications, projects, files…",
            query,
            direction,
        );
        StartMenuShell::new(theme, 900.0, primary, detail)
            .primary_footer(primary_footer)
            .detail_footer(detail_footer)
            .direction(direction)
    }

    #[test]
    fn section_trailing_action_and_row_primary_action_are_disjoint() {
        let view = Column::new()
            .child(SectionHeader::new(theme(), "Projects").action(
                theme(),
                "See all",
                Message::SeeAll,
            ))
            .child(ShortcutRow::new(
                theme(),
                Text::new("□"),
                "Nickel",
                "Desktop shell",
                Some(Message::Open),
                false,
            ));
        let tree = UiFrame::layout(view, Rect::new(0.0, 0.0, 480.0, 160.0));
        let open = tree
            .semantic_targets_for_message(&Message::Open)
            .into_iter()
            .next()
            .expect("row action")
            .bounds;
        let see_all = tree
            .semantic_targets_for_message(&Message::SeeAll)
            .into_iter()
            .next()
            .expect("trailing action")
            .bounds;
        assert!(open.origin.y >= see_all.origin.y + see_all.size.height);
        assert!(
            tree.accessibility_nodes()
                .iter()
                .any(|node| { node.interactive && node.label.as_deref() == Some("Nickel") })
        );
    }

    #[test]
    fn shortcut_activation_converges_across_supported_modalities() {
        let bounds = Rect::new(0.0, 0.0, 420.0, 80.0);

        let mut pointer_state = UiStateStore::default();
        let pointer_tree = UiFrame::layout_with_state(actionable_row(), bounds, &mut pointer_state);
        let target = pointer_tree
            .semantic_targets_for_message(&Message::Open)
            .into_iter()
            .next()
            .expect("row hit")
            .bounds;
        let point = Point {
            x: target.origin.x + target.size.width / 2.0,
            y: target.origin.y + target.size.height / 2.0,
        };
        let _ = pointer_tree.handle_event(&mut pointer_state, UiEvent::PointerPressed(point));
        assert_eq!(
            pointer_tree
                .handle_event(&mut pointer_state, UiEvent::PointerReleased(point))
                .messages,
            [Message::Open]
        );

        let mut keyboard_state = UiStateStore::default();
        let keyboard_tree =
            UiFrame::layout_with_state(actionable_row(), bounds, &mut keyboard_state);
        let _ = keyboard_tree.handle_event(&mut keyboard_state, UiEvent::FocusNext);
        assert_eq!(
            keyboard_tree
                .handle_event(&mut keyboard_state, UiEvent::KeyboardActivate)
                .messages,
            [Message::Open]
        );

        let mut controller_state = UiStateStore::default();
        let controller_tree =
            UiFrame::layout_with_state(actionable_row(), bounds, &mut controller_state);
        let _ = controller_tree.handle_event(&mut controller_state, UiEvent::ControllerNext);
        assert_eq!(
            controller_tree
                .handle_event(&mut controller_state, UiEvent::ControllerActivate)
                .messages,
            [Message::Open]
        );

        let mut accessibility_state = UiStateStore::default();
        let accessibility_tree =
            UiFrame::layout_with_state(actionable_row(), bounds, &mut accessibility_state);
        let id = accessibility_tree
            .accessibility_nodes()
            .iter()
            .find(|node| node.interactive && node.label.as_deref() == Some("Nickel"))
            .expect("accessible row action")
            .id
            .clone();
        assert_eq!(
            accessibility_tree
                .handle_event(&mut accessibility_state, UiEvent::AccessibilityActivate(id),)
                .messages,
            [Message::Open]
        );
    }

    #[test]
    fn unavailable_row_has_no_action_and_project_state_is_textual() {
        let tree = UiFrame::layout(
            ProjectStatusRow::new(
                theme(),
                Text::new("□"),
                "Nickel",
                "Status unknown",
                None,
                None::<Message>,
                false,
            ),
            Rect::new(0.0, 0.0, 420.0, 100.0),
        );
        assert!(tree.semantic_targets_for_message(&Message::Open).is_empty());
        assert!(tree.accessibility_nodes().iter().any(|node| {
            node.label.as_deref() == Some("Nickel")
                && node.state.as_deref() == Some("Status unknown")
        }));
        assert!(tree.commands().iter().any(|command| matches!(
            command,
            PaintCommand::Text { text, .. } if text.contains("Status unknown")
        )));
    }

    #[test]
    fn action_legend_resolves_family_controls_and_omits_unavailable_actions() {
        let entries = [
            ActionLegendEntry::available(SemanticControllerAction::Confirm, "Launch"),
            ActionLegendEntry::available(SemanticControllerAction::ContextMenu, "Actions"),
            ActionLegendEntry::unavailable(SemanticControllerAction::Pin, "Pin"),
            ActionLegendEntry::available(SemanticControllerAction::Cancel, "Close"),
        ];
        let tree = UiFrame::<Message>::layout(
            ActionLegend::new(theme(), ControllerFamily::PlayStation, entries),
            Rect::new(0.0, 0.0, 640.0, 72.0),
        );

        let labels = tree
            .accessibility_nodes()
            .iter()
            .filter_map(|node| node.label.as_deref())
            .collect::<Vec<_>>();
        assert!(labels.contains(&"Cross: Launch"));
        assert!(labels.contains(&"Options: Actions"));
        assert!(labels.contains(&"Circle: Close"));
        assert!(!labels.iter().any(|label| label.contains("Pin")));
        assert!(
            tree.commands()
                .iter()
                .any(|command| matches!(command, PaintCommand::Text { text, .. } if text == "×"))
        );
    }

    #[test]
    fn controller_family_presentations_are_semantic_and_have_truthful_fallbacks() {
        assert_eq!(
            ControllerFamily::Xbox
                .presentation(SemanticControllerAction::PreviousSection)
                .glyph,
            "LB"
        );
        assert_eq!(
            ControllerFamily::Switch
                .presentation(SemanticControllerAction::ContextMenu)
                .spoken_name,
            "Plus button"
        );
        assert_eq!(
            ControllerFamily::Generic
                .presentation(SemanticControllerAction::Confirm)
                .spoken_name,
            "confirm control"
        );
    }

    #[test]
    fn search_field_presents_preedit_without_committing_it() {
        let field =
            LauncherSearchField::new(theme(), Text::new("Search"), "ni", "ck", "Search", query);
        let tree = UiFrame::layout(field, Rect::new(0.0, 0.0, 420.0, 60.0));
        assert!(tree.commands().iter().any(|command| matches!(
            command,
            PaintCommand::Text { text, .. } if text == "nick"
        )));
    }

    #[test]
    fn shell_switches_to_single_pane_below_breakpoint() {
        let narrow = UiFrame::layout(
            StartMenuShell::new(
                theme(),
                619.0,
                Text::<Message>::new("Primary"),
                Text::<Message>::new("Detail"),
            ),
            Rect::new(0.0, 0.0, 600.0, 500.0),
        );
        assert!(narrow.diagnostics().is_empty());
        assert!(narrow.commands().iter().any(|command| matches!(
            command,
            PaintCommand::Text { text, .. } if text == "Primary"
        )));
        assert!(!narrow.commands().iter().any(|command| matches!(
            command,
            PaintCommand::Text { text, .. } if text == "Detail"
        )));

        let detail = UiFrame::layout(
            StartMenuShell::new(
                theme(),
                619.0,
                Text::<Message>::new("Primary"),
                Text::<Message>::new("Detail"),
            )
            .narrow_pane(StartMenuNarrowPane::Detail),
            Rect::new(0.0, 0.0, 600.0, 500.0),
        );
        assert!(!detail.commands().iter().any(|command| matches!(
            command,
            PaintCommand::Text { text, .. } if text == "Primary"
        )));
        assert!(detail.commands().iter().any(|command| matches!(
            command,
            PaintCommand::Text { text, .. } if text == "Detail"
        )));

        let boundary = UiFrame::layout(
            StartMenuShell::new(
                theme(),
                620.0,
                Text::<Message>::new("Primary"),
                Text::<Message>::new("Detail"),
            ),
            Rect::new(0.0, 0.0, 620.0, 500.0),
        );
        for expected in ["Primary", "Detail"] {
            assert!(boundary.commands().iter().any(|command| matches!(
                command,
                PaintCommand::Text { text, .. } if text == expected
            )));
        }
    }

    #[test]
    fn right_to_left_rows_and_wide_shells_mirror_semantic_order() {
        let rtl_row = UiFrame::layout(
            ShortcutRow::new_directional(
                theme(),
                Text::new("ICON"),
                "Label",
                "",
                Some(Message::Open),
                ShortcutState::default(),
                ReadingDirection::RightToLeft,
            ),
            Rect::new(0.0, 0.0, 420.0, 80.0),
        );
        let text_x = |tree: &UiFrame<Message>, wanted: &str| {
            tree.commands()
                .iter()
                .find_map(|command| match command {
                    PaintCommand::Text { bounds, text, .. } if text == wanted => {
                        Some(bounds.origin.x)
                    }
                    _ => None,
                })
                .expect("text command")
        };
        assert!(text_x(&rtl_row, "Label") < text_x(&rtl_row, "ICON"));

        let rtl_shell = UiFrame::layout(
            StartMenuShell::new(
                theme(),
                900.0,
                Text::<Message>::new("Primary"),
                Text::<Message>::new("Detail"),
            )
            .direction(ReadingDirection::RightToLeft),
            Rect::new(0.0, 0.0, 900.0, 500.0),
        );
        assert!(text_x(&rtl_shell, "Detail") < text_x(&rtl_shell, "Primary"));

        let rtl_header = UiFrame::layout(
            SectionHeader::new(theme(), "المشاريع")
                .action(theme(), "عرض الكل", Message::SeeAll)
                .direction(ReadingDirection::RightToLeft),
            Rect::new(0.0, 0.0, 420.0, 60.0),
        );
        assert!(rtl_header.commands().iter().any(|command| matches!(
            command,
            PaintCommand::Text { text, .. } if text.contains('‹')
        )));
        assert!(!rtl_header.commands().iter().any(|command| matches!(
            command,
            PaintCommand::Text { text, .. } if text.contains('›')
        )));
    }

    #[test]
    fn interaction_states_are_visual_textual_and_disable_activation() {
        let focused = UiFrame::layout(
            ShortcutRow::new_directional(
                theme(),
                Text::new("□"),
                "Focused",
                "",
                Some(Message::Open),
                ShortcutState {
                    focused: true,
                    ..ShortcutState::default()
                },
                ReadingDirection::LeftToRight,
            ),
            Rect::new(0.0, 0.0, 420.0, 80.0),
        );
        assert!(focused.commands().iter().any(|command| matches!(
            command,
            PaintCommand::Stroke { color, width, .. }
                if *color == theme().borders.focus && *width == 2.0
        )));
        assert!(focused.accessibility_nodes().iter().any(|node| {
            node.label.as_deref() == Some("Focused") && node.state.as_deref() == Some("focused")
        }));

        let disabled = UiFrame::layout(
            ShortcutRow::new_directional(
                theme(),
                Text::new("□"),
                "Disabled",
                "",
                Some(Message::Open),
                ShortcutState {
                    enabled: false,
                    ..ShortcutState::default()
                },
                ReadingDirection::LeftToRight,
            ),
            Rect::new(0.0, 0.0, 420.0, 80.0),
        );
        assert!(
            disabled
                .semantic_targets_for_message(&Message::Open)
                .is_empty()
        );
        assert!(disabled.accessibility_nodes().iter().any(|node| {
            node.label.as_deref() == Some("Disabled")
                && node.state.as_deref() == Some("disabled")
                && !node.interactive
        }));
    }

    #[test]
    fn fallback_avatar_is_deterministic_for_names_and_empty_identity() {
        assert_eq!(fallback_avatar_initials("Steven Shorton"), "SS");
        assert_eq!(fallback_avatar_initials("nickel"), "N");
        assert_eq!(fallback_avatar_initials("  "), "•");

        let first = UiFrame::layout(
            FallbackAvatar::<Message>::new(theme(), "Steven Shorton"),
            Rect::new(0.0, 0.0, 40.0, 40.0),
        );
        let second = UiFrame::layout(
            FallbackAvatar::<Message>::new(theme(), "Steven Shorton"),
            Rect::new(0.0, 0.0, 40.0, 40.0),
        );
        assert_eq!(first.commands(), second.commands());
    }

    #[test]
    fn start_menu_component_state_sheets_cover_themes_scales_and_text_directions() {
        let dark = theme();
        let high_contrast = SemanticTheme::resolve(
            light_theme().colors,
            dark.colors,
            crate::ResolvedThemePreferences {
                appearance: crate::ResolvedAppearance::Dark,
                high_contrast: true,
                reduced_transparency: false,
                reduced_motion: false,
            },
        );
        let reduced_transparency = SemanticTheme::resolve(
            light_theme().colors,
            dark.colors,
            crate::ResolvedThemePreferences {
                appearance: crate::ResolvedAppearance::Dark,
                high_contrast: false,
                reduced_transparency: true,
                reduced_motion: false,
            },
        );
        let bounds = Rect::new(0.0, 0.0, 900.0, 680.0);
        for (name, theme) in [
            ("dark", dark),
            ("light", light_theme()),
            ("high-contrast", high_contrast),
            ("reduced-transparency", reduced_transparency),
        ] {
            let tree = UiFrame::layout_with_diagnostics(
                component_state_sheet(
                    theme,
                    "Nickel",
                    "Desktop shell and local session",
                    ReadingDirection::LeftToRight,
                ),
                bounds,
            );
            assert!(
                tree.diagnostics().is_empty(),
                "{name}: {:#?}",
                tree.diagnostics()
            );
            for scale in [1.0_f32, 1.25, 2.0] {
                let width = (bounds.size.width * scale) as u32;
                let height = (bounds.size.height * scale) as u32;
                let mut renderer =
                    crate::SdlComponentRenderer::new_pixel_buffer(width, height, scale);
                assert!(!renderer.render(tree.commands()).is_empty());
                assert!(renderer.pixels().iter().any(|pixel| pixel.a > 0));
                let image = image::ImageBuffer::<image::Rgba<u8>, Vec<u8>>::from_fn(
                    width,
                    height,
                    |x, y| {
                        let pixel = renderer.pixels()[(y * width + x) as usize];
                        image::Rgba([pixel.r, pixel.g, pixel.b, pixel.a])
                    },
                );
                let output = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../target/nickel-ui-snapshots")
                    .join(format!("start-menu-components-{name}-{scale:.2}x.png"));
                std::fs::create_dir_all(output.parent().expect("snapshot parent")).unwrap();
                image.save(output).unwrap();
            }
        }

        for (name, project, supporting, direction) in [
            (
                "german",
                "Ausführliche Desktop-Shell-Dokumentation",
                "Lokale Sitzung mit erweitertem beschreibendem Text",
                ReadingDirection::LeftToRight,
            ),
            (
                "chinese",
                "中文桌面外壳项目",
                "本地会话",
                ReadingDirection::LeftToRight,
            ),
            (
                "spanish",
                "Proyecto de escritorio ampliado",
                "Sesión local",
                ReadingDirection::LeftToRight,
            ),
            (
                "expanded",
                "[!! Nickel desktop shell project with synthetic expansion !!]",
                "[!! Local session supporting text expanded for localization testing !!]",
                ReadingDirection::LeftToRight,
            ),
            (
                "arabic",
                "مشروع سطح المكتب",
                "جلسة محلية",
                ReadingDirection::RightToLeft,
            ),
        ] {
            let tree = UiFrame::layout_with_diagnostics(
                component_state_sheet(dark, project, supporting, direction),
                bounds,
            );
            assert!(
                tree.diagnostics().is_empty(),
                "{name}: {:#?}",
                tree.diagnostics()
            );
        }
    }
}
