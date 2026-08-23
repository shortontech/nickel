#![doc = include_str!("../README.md")]

extern crate self as nickel_ui;

mod assets;
pub mod controller;
pub mod gpu;
pub mod layout;
mod runtime;
pub mod state;
pub mod text_editor;
pub mod ui;

pub use controller::{ControllerAction, ControllerInput, NavigationPane, PaneNavigation};
pub use gpu::{ComponentGpu, DamageRegion, Pixel, SdlCanvasPresenter, SdlComponentRenderer};
pub use layout::{
    Align, Axis, Constraints, FlexItem, Insets, Justify, Length, Overflow, Point, Rect, Size,
    Track, layout_flex,
};
pub use runtime::{Application, Shortcut, run};
pub use state::{Invalidation, TransientState, UiId, UiStateStore};
pub use text_editor::TextEditor;
pub use ui::{
    AccessibilityNode, AnyView, Background, Border, Button, ButtonLabel, Color, Column, Component,
    ComponentBuilderExt, Container, ContentPane, DiagnosticKind, Dropdown, EventOutcome, FileGrid,
    FileGridItem, GradientAxis, Grid, GridColumnSpec, Header, HorizontalRule, Image,
    InteractionState, LayoutDiagnostic, LinearGradient, PaintCommand, RadioButton, ResolvedGrid,
    ResolvedLayout, ResolvedNode, Row, ScrollExtent, ShoulderHints, Sidebar, SidebarFolder,
    SidebarItem, SidebarSection, Slider, SourceLocation, Spacer, Text, TextAlign, TextField, Tone,
    UiEvent, UiTree, VerticalScroll,
};
pub use ui_declarative_macros::{component, id, ui};

pub type Fragment<Message = String> = Column<Message>;

pub trait View<Message>: Component<Message> {}

impl<Message, T: Component<Message>> View<Message> for T {}

pub mod prelude {
    pub use crate::{
        Align, AnyView, Application, Background, Border, Button, Color, Column, Component,
        ComponentBuilderExt, Container, DiagnosticKind, Dropdown, Fragment, Grid, GridColumnSpec,
        Insets, Justify, Length, Overflow, RadioButton, Row, Shortcut, Slider, Spacer, Text,
        TextAlign, TextField, Tone, Track, UiEvent, UiId, UiStateStore, View, component, id, run,
        ui,
    };
}

#[cfg(test)]
mod declarative_tests {
    use super::prelude::*;
    use super::{Rect, UiTree};

    #[derive(Clone, Debug, PartialEq)]
    enum Message {
        Save,
        Select(u32),
        Volume(f32),
        Query(String),
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
