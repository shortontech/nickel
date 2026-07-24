//! Platform-neutral interaction components shared by Nickel surfaces.

pub mod gpu;
pub mod layout;
pub mod text_editor;
pub mod ui;

pub use gpu::ComponentGpu;
pub use layout::{Axis, Constraints, FlexItem, Insets, Point, Rect, Size, layout_flex};
pub use text_editor::TextEditor;
pub use ui::{
    Background, Button, ButtonLabel, Color, Column, Component, Container, GradientAxis, Grid,
    Header, LinearGradient, PaintCommand, Row, Text, TextAlign, TextField, UiTree,
};
