#![doc = include_str!("../README.md")]

extern crate self as nickel_ui;

pub mod controller;
pub mod document_selection;
pub mod gpu;
pub mod input;
pub mod layout;
pub mod overlay;
pub mod primitives;
mod runtime;
pub mod state;
pub mod text_context_menu;
pub mod text_editor;
pub mod theme;
mod ui;

pub use controller::{ControllerAction, ControllerFamily, ControllerInput};
pub use document_selection::{
    DocumentSelection, SelectionAffinity, SelectionDocument, SelectionEndpoint, SelectionRun,
    TextBoundary,
};
pub use gpu::{
    AggregatePresenterCacheDiagnostics, DamageRegion, Pixel, PresenterCacheDiagnostics,
    SoftwareRenderer,
};
pub use input::{FocusedInputDispatcher, InputCommand, InputContext};
pub use layout::{
    Align, Axis, Constraints, FlexItem, Insets, Justify, Length, Overflow, Point, Rect, Size,
    Track, layout_flex,
};
pub use nickel_core::resource_owner::{
    DependencyOwnerDiagnostics, DependencyOwnerKind, dependency_owner_diagnostics,
};
pub use overlay::{
    CollisionPolicy, DismissPolicy, DismissReason, FocusReturn, OverlayAnchor, OverlayFocusPolicy,
    OverlayId, OverlayMenu, OverlayMenuItem, OverlayPlacement, OverlayStyle, TransientKind,
    TransientSurface, TransientTone, place_transient,
};
pub use primitives::{
    ActionRegion, ArtworkPresentation, ItemPresentation, StatusRegion, SurfaceScaffold, ToolRegion,
};
pub use runtime::{
    AdapterOutcome, Application, Completion, CompletionFailure, CompletionFailureKind,
    ControllerPollSchedule, DefaultHostAdapter, EffectEvidence, FileDragEvent, FrameOverlay,
    GlobalAction, HostAdapter, HostBatch, HostChangeToken, HostEvent, HostEventOutcome,
    HostFailure, HostFailureStage, HostInspection, HostServices, HostTelemetry, MessageEvidence,
    OutboundFileDrag, OverlayDeclarationFailure, Popover, SemanticActionFailure, Shortcut, Tooltip,
    UiHost, ViewContext, run, run_with_adapter,
};
pub use state::{InputModality, Invalidation, NavigationState, TransientState, UiId, UiStateStore};
pub use text_context_menu::{
    TextCommandEffect, TextContextAction, TextContextPolicy, TextEditCommand, execute_text_command,
    text_context_actions, text_context_menu,
};
pub use text_editor::TextEditor;
pub use theme::{
    AccentColors, AccessibilityPreferences, AppearancePreference, BorderColors, ContrastPreference,
    EasingCurve, FontWeight, MotionPreference, MotionScale, PlatformThemePreferences, RadiusScale,
    ResolvedAppearance, ResolvedThemePreferences, ScrollbarPalette, ScrollbarStateColors,
    SemanticTheme, SemanticTokenSet, SizingScale, SpacingScale, SurfaceColors, TextColors,
    TextStyle, ThemePreferences, TransparencyPreference, TypographyScale, focused_surface,
    focused_surface_with_foreground,
};
pub use ui::{
    ACTION_LEGEND_COMPACT_BREAKPOINT, AccessibilityNode, AccountSummaryRow, ActionKind,
    ActionLegend, ActionLegendActions, ActionLegendDensity, ActionLegendEntry, ActionLegendLabel,
    AnyView, Background, Border, Button, ButtonLabel, ButtonPresentation, ChoiceCard,
    ChoiceCardGroup, Collection, CollectionError, CollectionPresentation, CollectionState, Color,
    ColorSwatch, Column, CompactIconTile, Component, ComponentBuilderExt, Container, ContentPane,
    ControllerControlPresentation, ControllerGlyphSource, CustomPaint, DiagnosticKind,
    DiagnosticMode, DragGesture, DragPhase, Dropdown, EffectiveHitRoute, EventOutcome,
    FallbackAvatar, FieldGroup, FileGrid, FileGridItem, FrameRequest, FrameResourceDiagnostics,
    GradientAxis, Grid, GridColumnSpec, Header, HorizontalRule, Icon, Image, ImageAlignment,
    ImageFit, ImagePresentation, InlineButtonGroup, InputSource, InteractionIntent,
    InteractionState, LauncherSearchField, Layer, LayoutDiagnostic, LinearGradient, Menu, MenuBar,
    MenuItem, NavigationDirection, NavigationEntry, NavigationExit, NavigationItem,
    NavigationNeighbors, NavigationScope, NavigationSectionLabel, NavigationTraversal, PageHeader,
    PointerIcon, PreviewState, PreviewTile, ProjectStatusRow, RESPONSIVE_NAVIGATION_BREAKPOINT,
    RadioButton, RadioGroup, RadioOption, ReadingDirection, ResolvedGrid, ResolvedLayout,
    ResolvedNode, ResponsiveNavigation, ResponsiveNavigationDestination, ResponsiveNavigationError,
    ResponsiveNavigationPresentation, Row, SETTINGS_SHELL_NARROW_BREAKPOINT,
    START_MENU_SINGLE_PANE_BREAKPOINT, ScrollExtent, SectionHeader, SelectField,
    SelectionIndicator, SelectionRegion, SemanticAction, SemanticActionError,
    SemanticControllerAction, SemanticNodeSnapshot, SemanticQueryError, SemanticRole,
    SemanticSelector, SemanticTarget, SemanticValueInput, SemanticValueSnapshot, SessionActionRow,
    SettingsCard, SettingsListCard, SettingsNavigation, SettingsRow, SettingsSearchEntry,
    SettingsSearchField, SettingsSection, SettingsShell, SettingsStatus, SettingsStatusKind,
    ShortcutRow, ShortcutState, ShoulderHints, Sidebar, SidebarFolder, SidebarItem, SidebarSection,
    Slider, SliderField, SourceLocation, Spacer, StartMenuNarrowPane, StartMenuShell, StyledText,
    StyledTextSpan, Surface, SurfaceRole, Switch, SwitchState, TabList, Text, TextAlign, TextField,
    TextMeasureCacheMode, Tone, UiEvent, UiFrame, VerticalScroll, VirtualColumn, VirtualWindow,
    search_settings, with_text_measure_cache_mode,
};
pub use ui_declarative_macros::{component, id, ui};

// Internal implementation shorthand. The application-facing root deliberately
// does not export this type; graphical exceptions and presenters use `backend`.
pub(crate) use ui::PaintCommand;

/// Renderer-facing command stream. Application UI should use declarative
/// components or [`CustomPaint`]; platform presenters consume this module.
pub mod backend {
    pub use crate::ui::PaintCommand;
}

pub type Fragment<Message = String> = Column<Message>;

pub trait View<Message>: Component<Message> {}

impl<Message, T: Component<Message>> View<Message> for T {}

pub mod prelude {
    pub use crate::{
        AccentColors, AccessibilityPreferences, AccountSummaryRow, ActionRegion, Align, AnyView,
        AppearancePreference, Application, ArtworkPresentation, Background, Border, BorderColors,
        Button, ButtonPresentation, ChoiceCard, ChoiceCardGroup, Collection, CollectionError,
        CollectionPresentation, CollectionState, CollisionPolicy, Color, ColorSwatch, Column,
        CompactIconTile, Component, ComponentBuilderExt, Container, ContrastPreference,
        CustomPaint, DiagnosticKind, DismissPolicy, DismissReason, Dropdown, EasingCurve,
        FallbackAvatar, FieldGroup, FontWeight, Fragment, Grid, GridColumnSpec, Icon, Image,
        ImageAlignment, ImageFit, ImagePresentation, InlineButtonGroup, Insets, ItemPresentation,
        Justify, LauncherSearchField, Length, Menu, MenuBar, MenuItem, MotionPreference,
        MotionScale, NavigationDirection, NavigationEntry, NavigationExit, NavigationItem,
        NavigationNeighbors, NavigationScope, NavigationSectionLabel, NavigationTraversal,
        Overflow, OverlayAnchor, OverlayFocusPolicy, OverlayId, OverlayMenu, OverlayMenuItem,
        OverlayPlacement, OverlayStyle, PageHeader, PlatformThemePreferences, PointerIcon, Popover,
        PreviewState, PreviewTile, ProjectStatusRow, RESPONSIVE_NAVIGATION_BREAKPOINT, RadioButton,
        RadioGroup, RadioOption, RadiusScale, ReadingDirection, ResolvedAppearance,
        ResolvedThemePreferences, ResponsiveNavigation, ResponsiveNavigationDestination,
        ResponsiveNavigationError, ResponsiveNavigationPresentation, Row,
        SETTINGS_SHELL_NARROW_BREAKPOINT, START_MENU_SINGLE_PANE_BREAKPOINT, SectionHeader,
        SelectField, SelectionIndicator, SelectionRegion, SemanticTheme, SemanticTokenSet,
        SessionActionRow, SettingsCard, SettingsListCard, SettingsNavigation, SettingsRow,
        SettingsSearchEntry, SettingsSection, SettingsShell, SettingsStatus, SettingsStatusKind,
        Shortcut, ShortcutRow, ShortcutState, SizingScale, Slider, SliderField, Spacer,
        SpacingScale, StartMenuNarrowPane, StartMenuShell, StatusRegion, Surface, SurfaceColors,
        SurfaceRole, SurfaceScaffold, Switch, SwitchState, TabList, Text, TextAlign, TextBoundary,
        TextColors, TextField, TextStyle, ThemePreferences, Tone, ToolRegion, Tooltip, Track,
        TransientKind, TransientSurface, TransientTone, TransparencyPreference, TypographyScale,
        UiEvent, UiId, UiStateStore, View, VirtualColumn, VirtualWindow, component, id,
        place_transient, run, search_settings, ui,
    };
}

#[cfg(test)]
mod declarative_tests {
    use super::prelude::*;
    use super::{Rect, UiFrame};

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
        let tree = UiFrame::layout(
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
        assert!(
            !tree
                .semantic_targets_for_message(&Message::ToggleMenu)
                .is_empty()
        );
    }

    fn volume(value: f32) -> Message {
        Message::Volume(value)
    }

    fn query(value: String) -> Message {
        Message::Query(value)
    }

    #[test]
    fn declarative_controller_panes_groups_and_adjustment_share_one_state_machine() {
        let view = ui! {
            <Row>
                <Container id={"sidebar"} navigation_scope={NavigationScope::pane(false)}
                    controller_scope_background={0x24203a}>
                    <Button on_press={Message::Save}>{"Save"}</Button>
                </Container>
                <Container id={"content"} navigation_scope={NavigationScope::pane(true)}
                    controller_scope_background={0x24203a}>
                    <Container id={"audio"} navigation_scope={NavigationScope::group()}
                        controller_focus_background_tint={0x55d98b}>
                        <Slider id={"volume"} value={0.5} on_change={volume}
                            adjustment_step={0.1} controller_focus_background_tint={0x55d98b} />
                    </Container>
                </Container>
            </Row>
        };
        let mut state = UiStateStore::default();
        let tree = UiFrame::layout_with_state(view, Rect::new(0.0, 0.0, 480.0, 240.0), &mut state);

        tree.handle_event(&mut state, UiEvent::ControllerNext);
        assert!(
            state
                .navigation()
                .controller_selected()
                .is_some_and(|id| id.as_str().ends_with("/audio"))
        );
        tree.handle_event(&mut state, UiEvent::ControllerActivate);
        assert!(state.navigation().controller_scope().is_some());
        tree.handle_event(&mut state, UiEvent::ControllerActivate);
        assert!(state.navigation().controller_editing());
        let adjusted = tree.handle_event(&mut state, UiEvent::ControllerAdjust(1.0));
        assert_eq!(adjusted.messages, vec![Message::Volume(0.6)]);
        tree.handle_event(&mut state, UiEvent::ControllerBack);
        assert!(!state.navigation().controller_editing());

        tree.handle_event(&mut state, UiEvent::ControllerPreviousPane);
        assert!(
            state
                .navigation()
                .controller_selected()
                .is_some_and(|id| id.as_str().contains("sidebar"))
        );

        tree.handle_event(&mut state, UiEvent::FocusLost);
        assert!(!state.window_focused());
        assert!(state.navigation().controller_selected().is_none());
        tree.handle_event(&mut state, UiEvent::ControllerNext);
        assert!(state.navigation().controller_selected().is_none());

        tree.handle_event(&mut state, UiEvent::FocusGained);
        tree.handle_event(&mut state, UiEvent::ControllerNext);
        assert!(state.navigation().controller_selected().is_some());
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
        let tree = UiFrame::layout(view, Rect::new(0.0, 0.0, 480.0, 320.0));
        let save_id = tree
            .semantic_targets_for_message(&Message::Save)
            .into_iter()
            .next()
            .expect("declarative save target")
            .id;
        assert!(save_id.as_str().ends_with("/save"));
        let save = tree
            .resolved_layout()
            .find(&save_id)
            .expect("resolved save");
        assert_eq!(save.source.expect("declarative source").component, "Button");
        assert!(tree.commands().iter().any(|command| matches!(
            command,
            super::backend::PaintCommand::Text { text, .. } if text == "Nickel"
        )));
        assert!(tree.commands().iter().any(|command| matches!(
            command,
            super::backend::PaintCommand::RoundedFill { radius, .. } if *radius == 6.0
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
        let tree: UiFrame<Message> = UiFrame::layout(view, Rect::new(0.0, 0.0, 200.0, 80.0));
        assert_eq!(
            tree.commands()
                .iter()
                .filter(|command| matches!(command, super::backend::PaintCommand::Text { .. }))
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
        let tree: UiFrame<Message> = UiFrame::layout(view, Rect::new(0.0, 0.0, 200.0, 80.0));
        assert_eq!(
            tree.commands()
                .iter()
                .filter(|command| matches!(command, super::backend::PaintCommand::Text { .. }))
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
        let tree: UiFrame<Message> = UiFrame::layout(view, Rect::new(0.0, 0.0, 480.0, 160.0));
        assert!(tree.commands().iter().any(
            |command| matches!(command, super::backend::PaintCommand::Text { text, .. } if text == "Settings")
        ));
    }

    #[test]
    fn declarative_source_locations_flow_into_layout_diagnostics() {
        let tree = UiFrame::<Message>::layout_with_diagnostics(
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
