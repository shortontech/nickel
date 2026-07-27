//! Platform-neutral interaction components shared by Nickel surfaces.

pub mod controller;
pub mod gpu;
pub mod layout;
pub mod text_editor;
pub mod ui;

pub use controller::{ControllerAction, ControllerInput, NavigationPane, PaneNavigation};
pub use gpu::ComponentGpu;
pub use layout::{Axis, Constraints, FlexItem, Insets, Point, Rect, Size, layout_flex};
pub use text_editor::TextEditor;
pub use ui::{
    Background, Button, ButtonLabel, Color, Column, Component, Container, ContentPane, Dropdown,
    FileGrid, FileGridItem, GradientAxis, Grid, Header, HorizontalRule, Image, LinearGradient,
    PaintCommand, RadioButton, Row, ShoulderHints, Sidebar, SidebarFolder, SidebarItem,
    SidebarSection, Slider, Text, TextAlign, TextField, UiTree,
};
