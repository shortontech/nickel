#![doc = include_str!("../README.md")]

extern crate self as nickel_ui;

pub mod controller;
pub mod document_selection;
pub mod gpu;
pub mod layout;
mod runtime;
pub mod state;
pub mod text_editor;
pub mod theme;
pub mod ui;

pub use controller::{ControllerAction, ControllerInput, NavigationPane, PaneNavigation};
pub use document_selection::{
    DocumentSelection, SelectionAffinity, SelectionDocument, SelectionEndpoint, SelectionRun,
    TextBoundary,
};
pub use gpu::{ComponentGpu, DamageRegion, Pixel, SdlCanvasPresenter, SdlComponentRenderer};
pub use layout::{
    Align, Axis, Constraints, FlexItem, Insets, Justify, Length, Overflow, Point, Rect, Size,
    Track, layout_flex,
};
pub use runtime::{Application, ApplicationHost, HostEventOutcome, Shortcut, run};
pub use state::{Invalidation, TransientState, UiId, UiStateStore};
pub use text_editor::TextEditor;
pub use theme::{
    AccentColors, AccessibilityPreferences, AppearancePreference, BorderColors, ContrastPreference,
    EasingCurve, FontWeight, MotionPreference, MotionScale, PlatformThemePreferences, RadiusScale,
    ResolvedAppearance, ResolvedThemePreferences, SemanticColors, SemanticTheme, SemanticTokenSet,
    SizingScale, SpacingScale, SurfaceColors, TextColors, TextStyle, ThemePreferences,
    TransparencyPreference, TypographyScale,
};
pub use ui::{
    AccessibilityNode, AccountSummaryRow, AnyView, Background, Border, Button, ButtonLabel,
    ButtonPresentation, ChoiceCard, ChoiceCardGroup, Color, ColorSwatch, Column, CompactIconTile,
    Component, ComponentBuilderExt, Container, ContentPane, DiagnosticKind, Dropdown, EventOutcome,
    FallbackAvatar, FieldGroup, FileGrid, FileGridItem, GradientAxis, Grid, GridColumnSpec, Header,
    HorizontalRule, Icon, Image, ImageAlignment, ImageFit, ImagePresentation, InlineButtonGroup,
    InteractionState, LauncherSearchField, LayoutDiagnostic, LinearGradient, Menu, MenuBar,
    MenuItem, NavigationItem, NavigationSectionLabel, PageHeader, PaintCommand, PointerIcon,
    PreviewState, PreviewTile, ProjectStatusRow, RadioButton, ReadingDirection, ResolvedGrid,
    ResolvedLayout, ResolvedNode, Row, SETTINGS_SHELL_NARROW_BREAKPOINT,
    START_MENU_SINGLE_PANE_BREAKPOINT, ScrollExtent, SectionHeader, SelectField,
    SelectionIndicator, SelectionRegion, SessionActionRow, SettingsCard, SettingsListCard,
    SettingsNarrowPane, SettingsNavigation, SettingsRow, SettingsSearchEntry, SettingsSearchField,
    SettingsSection, SettingsShell, SettingsStatus, SettingsStatusKind, ShortcutRow, ShortcutState,
    ShoulderHints, Sidebar, SidebarFolder, SidebarItem, SidebarSection, Slider, SliderField,
    SourceLocation, Spacer, StartMenuNarrowPane, StartMenuShell, StyledText, StyledTextSpan,
    Surface, SurfaceRole, Switch, SwitchState, TabList, Text, TextAlign, TextField, Tone, UiEvent,
    UiTree, VerticalScroll, VirtualColumn, VirtualWindow, search_settings,
};
pub use ui_declarative_macros::{component, id, ui};

pub type Fragment<Message = String> = Column<Message>;

pub trait View<Message>: Component<Message> {}

impl<Message, T: Component<Message>> View<Message> for T {}

pub mod prelude {
    pub use crate::{
        AccentColors, AccessibilityPreferences, AccountSummaryRow, Align, AnyView,
        AppearancePreference, Application, Background, Border, BorderColors, Button,
        ButtonPresentation, ChoiceCard, ChoiceCardGroup, Color, ColorSwatch, Column,
        CompactIconTile, Component, ComponentBuilderExt, Container, ContrastPreference,
        DiagnosticKind, Dropdown, EasingCurve, FallbackAvatar, FieldGroup, FontWeight, Fragment,
        Grid, GridColumnSpec, Icon, Image, ImageAlignment, ImageFit, ImagePresentation,
        InlineButtonGroup, Insets, Justify, LauncherSearchField, Length, Menu, MenuBar, MenuItem,
        MotionPreference, MotionScale, NavigationItem, NavigationSectionLabel, Overflow,
        PageHeader, PlatformThemePreferences, PointerIcon, PreviewState, PreviewTile,
        ProjectStatusRow, RadioButton, RadiusScale, ReadingDirection, ResolvedAppearance,
        ResolvedThemePreferences, Row, SETTINGS_SHELL_NARROW_BREAKPOINT,
        START_MENU_SINGLE_PANE_BREAKPOINT, SectionHeader, SelectField, SelectionIndicator,
        SelectionRegion, SemanticColors, SemanticTheme, SemanticTokenSet, SessionActionRow,
        SettingsCard, SettingsListCard, SettingsNarrowPane, SettingsNavigation, SettingsRow,
        SettingsSearchEntry, SettingsSection, SettingsShell, SettingsStatus, SettingsStatusKind,
        Shortcut, ShortcutRow, ShortcutState, SizingScale, Slider, SliderField, Spacer,
        SpacingScale, StartMenuNarrowPane, StartMenuShell, Surface, SurfaceColors, SurfaceRole,
        Switch, SwitchState, TabList, Text, TextAlign, TextBoundary, TextColors, TextField,
        TextStyle, ThemePreferences, Tone, Track, TransparencyPreference, TypographyScale, UiEvent,
        UiId, UiStateStore, View, VirtualColumn, VirtualWindow, component, id, run,
        search_settings, ui,
    };
}

#[cfg(test)]
mod declarative_tests {
    use super::prelude::*;
    use super::{Rect, UiTree};

    #[derive(Clone, Debug, PartialEq)]
    enum Message {
        Save,
        ToggleMenu,
        Select(u32),
        Volume(f32),
        Query(String),
    }

    #[test]
    fn declarative_menu_preserves_typed_item_messages() {
        let tree = UiTree::layout(
            ui! {
                <MenuBar>
                    <Menu id={id!(file_menu)} on_toggle={Message::ToggleMenu} label={"File"}>
                        <MenuItem label={"Save"} on_press={Message::Save} />
                        <MenuItem label={"Unavailable"} disabled />
                    </Menu>
                </MenuBar>
            },
            Rect::new(0.0, 0.0, 240.0, 120.0),
        );
        assert!(tree.message_rect(&Message::ToggleMenu).is_some());
    }

    fn volume(value: f32) -> Message {
        Message::Volume(value)
    }

    fn query(value: String) -> Message {
        Message::Query(value)
    }

    #[component]
    fn ItemCard(label: &str, message: Message, tone: Option<Color>) -> impl View<Message> {
        ui! {
            <Button on_press={message} color={tone.unwrap_or(0xffffff)}>{label}</Button>
        }
    }

    #[component]
    fn Frame(child: impl Component<Message>, tone: Color) -> impl View<Message> {
        ui! { <Container background={tone}>{child}</Container> }
    }

    #[component]
    fn Stack(
        children: impl IntoIterator<Item = impl Component<Message>>,
        gap: f32,
    ) -> impl View<Message> {
        ui! { <Column gap={gap}>{children.into_iter()}</Column> }
    }

    #[test]
    fn declarative_components_expressions_lists_keys_and_messages_share_the_ui_tree() {
        let title = String::from("Nickel");
        let items = [(7_u32, "Seven"), (9, "Nine")];
        let show_slider = true;
        let view = ui! {
            <Column id={id!(settings_root)} gap={8.0} padding={Insets::all(12.0)}
                background={0x101010} border={Border::new(0x334455, 1.0)} radius={6.0} fill_width>
                <Text>{&title}</Text>
                <Row align_items={Align::Center}>
                    <Button id={id!(save)} on_press={Message::Save}>{"Save"}</Button>
                    <Spacer fill />
                </Row>
                {if show_slider {
                    ui! { <Slider id={id!(volume)} value={0.5} on_change={volume} /> }
                } else {
                    ui! { <Text>{"Volume unavailable"}</Text> }
                }}
                <TextField id={id!(query)} value={"nickel"} on_change={query} />
                <Grid>
                    {items.iter().map(|(key, label)| ui! {
                        <ItemCard key={*key} message={Message::Select(*key)} label={label} />
                    })}
                </Grid>
            </Column>
        };
        let tree = UiTree::layout(view, Rect::new(0.0, 0.0, 480.0, 320.0));
        let save_id = tree
            .id_for_message(&Message::Save)
            .expect("declarative save id");
        assert!(save_id.as_str().ends_with("/save"));
        let save = tree.resolved_layout().find(save_id).expect("resolved save");
        assert_eq!(save.source.expect("declarative source").component, "Button");
        assert!(tree.commands().iter().any(|command| matches!(
            command,
            super::PaintCommand::Text { text, .. } if text == "Nickel"
        )));
        assert!(tree.commands().iter().any(|command| matches!(
            command,
            super::PaintCommand::RoundedFill { radius, .. } if *radius == 6.0
        )));
        let query_id = tree
            .resolved_layout()
            .nodes()
            .iter()
            .find(|node| node.id.as_str().ends_with("/query"))
            .expect("query field")
            .id
            .clone();
        let mut state = UiStateStore::default();
        state.set_focus(Some(query_id));
        assert_eq!(
            tree.handle_event(&mut state, UiEvent::TextInput("!".into()))
                .messages,
            vec![Message::Query("nickel!".into())]
        );
    }

    #[test]
    fn fragments_and_match_are_ordinary_rust_values() {
        let value = 2;
        let view = ui! {
            <>
                <Text>{"First"}</Text>
                {match value {
                    2 => ui! { <Text>{"Second"}</Text> },
                    _ => ui! { <Text>{"Other"}</Text> },
                }}
            </>
        };
        let tree: UiTree<Message> = UiTree::layout(view, Rect::new(0.0, 0.0, 200.0, 80.0));
        assert_eq!(
            tree.commands()
                .iter()
                .filter(|command| matches!(command, super::PaintCommand::Text { .. }))
                .count(),
            2
        );
    }

    #[test]
    fn function_components_accept_declared_child_and_children_slots() {
        let view = ui! {
            <Stack gap={3.0}>
                <Frame tone={0x101010}><Text>{"one"}</Text></Frame>
                <Frame tone={0x202020}><Text>{"two"}</Text></Frame>
            </Stack>
        };
        let tree: UiTree<Message> = UiTree::layout(view, Rect::new(0.0, 0.0, 200.0, 80.0));
        assert_eq!(
            tree.commands()
                .iter()
                .filter(|command| matches!(command, super::PaintCommand::Text { .. }))
                .count(),
            2
        );
    }

    #[test]
    fn public_react_style_contract_accepts_header_alignment_tone_and_typed_style_values() {
        let title = String::from("Settings");
        let view = ui! {
            <Container fill_width min_width={320.0} padding={16.0} gap={8.0}
                background={0x101820} border={Border::new(0x334455, 1.0)} radius={8.0}
                overflow_y={Overflow::Auto}>
                <Header title={&title} />
                <Row align={Align::Center}>
                    <Text tone={Tone::Muted}>{"Declarative"}</Text>
                    <Spacer fill />
                </Row>
            </Container>
        };
        let tree: UiTree<Message> = UiTree::layout(view, Rect::new(0.0, 0.0, 480.0, 160.0));
        assert!(tree.commands().iter().any(
            |command| matches!(command, super::PaintCommand::Text { text, .. } if text == "Settings")
        ));
    }

    #[test]
    fn declarative_source_locations_flow_into_layout_diagnostics() {
        let tree = UiTree::<Message>::layout_with_diagnostics(
            ui! { <Column id={id!(broken)} min_width={200.0} max_width={10.0} /> },
            Rect::new(0.0, 0.0, 100.0, 40.0),
        );
        let diagnostic = tree
            .diagnostics()
            .iter()
            .find(|diagnostic| diagnostic.kind == DiagnosticKind::ContradictoryConstraints)
            .expect("contradictory diagnostic");
        let source = diagnostic.source.expect("declarative diagnostic source");
        assert_eq!(source.component, "Column");
        assert!(source.file.ends_with("lib.rs"));
    }
}
