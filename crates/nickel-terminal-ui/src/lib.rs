//! Declarative Nickel UI projection and normalized input policy for one terminal viewport.

use nickel_input::{AggregateModifier, InputEvent, KeyCode, KeyEdge, PhysicalKey, TextEvent};
use nickel_terminal::{TerminalCell, TerminalColor, TerminalScroll, TerminalSnapshot};
use nickel_ui::{
    Component, Container, Grid, SemanticRole, StyledText, StyledTextSpan, Track, View,
};

pub const MAX_PASTE_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TerminalInputCommand {
    Write(Vec<u8>),
    Scroll(TerminalScroll),
    Copy,
    PasteRequested,
    SelectAll,
    ClearScrollback,
    Focus(bool),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PasteDecision {
    Ready(Vec<u8>),
    ConfirmationRequired(String),
    RejectedTooLarge,
}

pub fn prepare_paste(text: String, bracketed: bool) -> PasteDecision {
    if text.len() > MAX_PASTE_BYTES {
        return PasteDecision::RejectedTooLarge;
    }
    if text.contains(['\n', '\r']) {
        return PasteDecision::ConfirmationRequired(text);
    }
    PasteDecision::Ready(encode_paste(text, bracketed))
}

pub fn confirm_paste(text: String, bracketed: bool) -> PasteDecision {
    if text.len() > MAX_PASTE_BYTES {
        PasteDecision::RejectedTooLarge
    } else {
        PasteDecision::Ready(encode_paste(text, bracketed))
    }
}

fn encode_paste(text: String, bracketed: bool) -> Vec<u8> {
    if !bracketed {
        return text.into_bytes();
    }
    let mut bytes = Vec::with_capacity(text.len() + 12);
    bytes.extend_from_slice(b"\x1b[200~");
    bytes.extend_from_slice(text.as_bytes());
    bytes.extend_from_slice(b"\x1b[201~");
    bytes
}

/// Converts normalized input to terminal intent. Printable input is accepted only from committed
/// text events, so key and IME streams can never insert the same text twice.
pub fn translate_input(input: &InputEvent) -> Option<TerminalInputCommand> {
    translate_input_with_application_cursor(input, false)
}

pub fn translate_input_with_application_cursor(
    input: &InputEvent,
    application_cursor: bool,
) -> Option<TerminalInputCommand> {
    match input {
        InputEvent::Text(TextEvent::Commit { text, .. }) if !text.is_empty() => {
            Some(TerminalInputCommand::Write(text.as_bytes().to_vec()))
        }
        InputEvent::Text(TextEvent::Preedit { .. }) => None,
        InputEvent::FocusGained { .. } => Some(TerminalInputCommand::Focus(true)),
        InputEvent::FocusLost { .. } => Some(TerminalInputCommand::Focus(false)),
        InputEvent::Key(event) if event.edge == KeyEdge::Pressed => {
            let control = event.modifiers.aggregate(AggregateModifier::Control);
            let shift = event.modifiers.aggregate(AggregateModifier::Shift);
            let alt = event.modifiers.aggregate(AggregateModifier::Alt);
            let code = match event.physical {
                PhysicalKey::Code(code) => code,
                PhysicalKey::Native(_) => return None,
            };
            if control && shift {
                return match code {
                    KeyCode::KeyC => Some(TerminalInputCommand::Copy),
                    KeyCode::KeyV => Some(TerminalInputCommand::PasteRequested),
                    KeyCode::KeyA => Some(TerminalInputCommand::SelectAll),
                    _ => key_bytes(code, alt, application_cursor).map(TerminalInputCommand::Write),
                };
            }
            if shift {
                return match code {
                    KeyCode::PageUp => Some(TerminalInputCommand::Scroll(TerminalScroll::PageUp)),
                    KeyCode::PageDown => {
                        Some(TerminalInputCommand::Scroll(TerminalScroll::PageDown))
                    }
                    KeyCode::Home => Some(TerminalInputCommand::Scroll(TerminalScroll::Top)),
                    KeyCode::End => Some(TerminalInputCommand::Scroll(TerminalScroll::Bottom)),
                    _ => key_bytes(code, alt, application_cursor).map(TerminalInputCommand::Write),
                };
            }
            if control && let Some(byte) = control_byte(code) {
                return Some(TerminalInputCommand::Write(vec![byte]));
            }
            key_bytes(code, alt, application_cursor).map(TerminalInputCommand::Write)
        }
        _ => None,
    }
}

fn control_byte(code: KeyCode) -> Option<u8> {
    let value = match code {
        KeyCode::KeyA => 1,
        KeyCode::KeyB => 2,
        KeyCode::KeyC => 3,
        KeyCode::KeyD => 4,
        KeyCode::KeyE => 5,
        KeyCode::KeyF => 6,
        KeyCode::KeyG => 7,
        KeyCode::KeyH => 8,
        KeyCode::KeyI => 9,
        KeyCode::KeyJ => 10,
        KeyCode::KeyK => 11,
        KeyCode::KeyL => 12,
        KeyCode::KeyM => 13,
        KeyCode::KeyN => 14,
        KeyCode::KeyO => 15,
        KeyCode::KeyP => 16,
        KeyCode::KeyQ => 17,
        KeyCode::KeyR => 18,
        KeyCode::KeyS => 19,
        KeyCode::KeyT => 20,
        KeyCode::KeyU => 21,
        KeyCode::KeyV => 22,
        KeyCode::KeyW => 23,
        KeyCode::KeyX => 24,
        KeyCode::KeyY => 25,
        KeyCode::KeyZ => 26,
        _ => return None,
    };
    Some(value)
}

fn key_bytes(code: KeyCode, alt: bool, application_cursor: bool) -> Option<Vec<u8>> {
    let sequence: &[u8] = match code {
        KeyCode::Enter | KeyCode::NumpadEnter => b"\r",
        KeyCode::Tab => b"\t",
        KeyCode::Backspace => b"\x7f",
        KeyCode::Escape => b"\x1b",
        KeyCode::ArrowUp if application_cursor => b"\x1bOA",
        KeyCode::ArrowDown if application_cursor => b"\x1bOB",
        KeyCode::ArrowRight if application_cursor => b"\x1bOC",
        KeyCode::ArrowLeft if application_cursor => b"\x1bOD",
        KeyCode::Home if application_cursor => b"\x1bOH",
        KeyCode::End if application_cursor => b"\x1bOF",
        KeyCode::ArrowUp => b"\x1b[A",
        KeyCode::ArrowDown => b"\x1b[B",
        KeyCode::ArrowRight => b"\x1b[C",
        KeyCode::ArrowLeft => b"\x1b[D",
        KeyCode::Home => b"\x1b[H",
        KeyCode::End => b"\x1b[F",
        KeyCode::Insert => b"\x1b[2~",
        KeyCode::Delete => b"\x1b[3~",
        KeyCode::PageUp => b"\x1b[5~",
        KeyCode::PageDown => b"\x1b[6~",
        KeyCode::F1 => b"\x1bOP",
        KeyCode::F2 => b"\x1bOQ",
        KeyCode::F3 => b"\x1bOR",
        KeyCode::F4 => b"\x1bOS",
        KeyCode::F5 => b"\x1b[15~",
        KeyCode::F6 => b"\x1b[17~",
        KeyCode::F7 => b"\x1b[18~",
        KeyCode::F8 => b"\x1b[19~",
        KeyCode::F9 => b"\x1b[20~",
        KeyCode::F10 => b"\x1b[21~",
        KeyCode::F11 => b"\x1b[23~",
        KeyCode::F12 => b"\x1b[24~",
        _ => return None,
    };
    let mut bytes = Vec::with_capacity(sequence.len() + usize::from(alt));
    if alt {
        bytes.push(0x1b);
    }
    bytes.extend_from_slice(sequence);
    Some(bytes)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerminalPalette {
    pub foreground: u32,
    pub background: u32,
    pub cursor: u32,
    pub selection: u32,
    pub indexed: [u32; 16],
}

impl Default for TerminalPalette {
    fn default() -> Self {
        Self {
            foreground: 0xffd8dee9,
            background: 0xff111318,
            cursor: 0xffeceff4,
            selection: 0xff3b526b,
            indexed: [
                0xff000000, 0xffbf616a, 0xffa3be8c, 0xffebcb8b, 0xff81a1c1, 0xffb48ead, 0xff88c0d0,
                0xffe5e9f0, 0xff4c566a, 0xffd08770, 0xffb8d8a8, 0xffffdf9b, 0xff9cc1e6, 0xffd5a5d2,
                0xff8fdbdf, 0xffffffff,
            ],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CellMetrics {
    pub width: f32,
    pub height: f32,
    pub text_scale: f32,
}

impl CellMetrics {
    pub fn integral(font_size: f32, scale: f32) -> Self {
        let physical_scale = scale.max(0.25);
        let height = (font_size.max(6.0) * 1.35 * physical_scale).round() / physical_scale;
        let width = (height * 0.52 * physical_scale).round().max(1.0) / physical_scale;
        Self {
            width,
            height,
            text_scale: font_size.max(6.0) / 8.0,
        }
    }

    pub fn dimensions(&self, width: f32, height: f32) -> (u16, u16) {
        let columns = (width.max(self.width) / self.width)
            .floor()
            .clamp(1.0, 500.0) as u16;
        let lines = (height.max(self.height) / self.height)
            .floor()
            .clamp(1.0, 200.0) as u16;
        (columns, lines)
    }
}

pub struct TerminalViewport<'a> {
    snapshot: &'a TerminalSnapshot,
    palette: &'a TerminalPalette,
    metrics: CellMetrics,
    title: &'a str,
}

impl<'a> TerminalViewport<'a> {
    pub fn new(
        snapshot: &'a TerminalSnapshot,
        palette: &'a TerminalPalette,
        metrics: CellMetrics,
        title: &'a str,
    ) -> Self {
        Self {
            snapshot,
            palette,
            metrics,
            title,
        }
    }

    pub fn view<Message: Clone>(&self) -> impl View<Message> + use<Message> {
        let mut grid = Grid::tracks([Track::repeat(
            self.snapshot.columns,
            Track::px(self.metrics.width),
        )])
        .gap(0.0);
        for (index, cell) in self.snapshot.cells.iter().enumerate() {
            grid = grid.child(self.cell::<Message>(cell, index));
        }
        let visible = visible_text(self.snapshot);
        Container::new()
            .id("terminal-viewport")
            .semantic_role(SemanticRole::GraphicalCustomControl)
            .accessibility_label(format!(
                "Terminal {title}; cursor row {}, column {}; {visible}",
                self.snapshot.cursor.line + 1,
                self.snapshot.cursor.column + 1,
                title = self.title,
            ))
            .width(self.snapshot.columns as f32 * self.metrics.width)
            .height(self.snapshot.lines as f32 * self.metrics.height)
            .background(self.palette.background)
            .child(grid)
    }

    fn cell<Message: Clone>(&self, cell: &TerminalCell, index: usize) -> impl Component<Message> {
        let cursor = self.snapshot.cursor.visible
            && index / self.snapshot.columns == self.snapshot.cursor.line
            && index % self.snapshot.columns == self.snapshot.cursor.column;
        let mut foreground = resolve_color(&cell.foreground, self.palette, true);
        let mut background = resolve_color(&cell.background, self.palette, false);
        if cell.inverse {
            std::mem::swap(&mut foreground, &mut background);
        }
        if cell.dim {
            foreground = blend(foreground, background, 0.55);
        }
        if cell.concealed {
            foreground = background;
        }
        if cell.selected {
            background = self.palette.selection;
        }
        if cursor {
            background = self.palette.cursor;
            foreground = self.palette.background;
        }
        let mut value = if cell.wide_spacer {
            String::new()
        } else {
            cell.character.to_string()
        };
        value.extend(cell.combining.iter());
        let end = value.len();
        let styled = StyledText::new(
            value,
            vec![StyledTextSpan {
                range: 0..end,
                bold: cell.bold,
                italic: cell.italic,
                monospace: true,
                strikethrough: false,
                underline: cell.underline,
                color: Some(foreground),
                background: None,
            }],
        )
        .scale(self.metrics.text_scale)
        .color(foreground);
        Container::new()
            .width(self.metrics.width)
            .height(self.metrics.height)
            .background(background)
            .child(styled)
    }
}

fn visible_text(snapshot: &TerminalSnapshot) -> String {
    snapshot
        .cells
        .chunks(snapshot.columns.max(1))
        .map(|line| {
            line.iter()
                .filter(|cell| !cell.wide_spacer)
                .map(|cell| cell.character)
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn resolve_color(color: &TerminalColor, palette: &TerminalPalette, foreground: bool) -> u32 {
    match color {
        TerminalColor::Rgb(red, green, blue) => {
            0xff00_0000 | (u32::from(*red) << 16) | (u32::from(*green) << 8) | u32::from(*blue)
        }
        TerminalColor::Indexed(index) if *index < 16 => palette.indexed[*index as usize],
        TerminalColor::Indexed(index) if *index < 232 => {
            let index = *index - 16;
            let component = |value: u8| if value == 0 { 0 } else { 55 + value * 40 };
            0xff00_0000
                | (u32::from(component(index / 36)) << 16)
                | (u32::from(component((index / 6) % 6)) << 8)
                | u32::from(component(index % 6))
        }
        TerminalColor::Indexed(index) => {
            let value = 8 + (*index - 232) * 10;
            0xff00_0000 | (u32::from(value) << 16) | (u32::from(value) << 8) | u32::from(value)
        }
        TerminalColor::Named(name) if name == "Foreground" => palette.foreground,
        TerminalColor::Named(name) if name == "Background" => palette.background,
        TerminalColor::Named(name) => named_index(name).map_or_else(
            || {
                if foreground {
                    palette.foreground
                } else {
                    palette.background
                }
            },
            |index| palette.indexed[index],
        ),
    }
}

fn named_index(name: &str) -> Option<usize> {
    Some(match name {
        "Black" => 0,
        "Red" => 1,
        "Green" => 2,
        "Yellow" => 3,
        "Blue" => 4,
        "Magenta" => 5,
        "Cyan" => 6,
        "White" => 7,
        "BrightBlack" => 8,
        "BrightRed" => 9,
        "BrightGreen" => 10,
        "BrightYellow" => 11,
        "BrightBlue" => 12,
        "BrightMagenta" => 13,
        "BrightCyan" => 14,
        "BrightWhite" => 15,
        _ => return None,
    })
}

fn blend(foreground: u32, background: u32, amount: f32) -> u32 {
    let channel = |shift: u32| {
        let foreground = ((foreground >> shift) & 0xff_u32) as f32;
        let background = ((background >> shift) & 0xff_u32) as f32;
        (background + (foreground - background) * amount).round() as u32
    };
    0xff00_0000 | (channel(16) << 16) | (channel(8) << 8) | channel(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nickel_input::{DeviceId, EventOrder, KeyEvent, KeyLocation, LogicalKey, ModifierState};
    use nickel_terminal::{TerminalDimensions, TerminalEngine};
    use nickel_ui::{Rect, UiFrame};

    fn snapshot(bytes: &[u8]) -> TerminalSnapshot {
        let dimensions = TerminalDimensions::new(10, 3, 8, 16).unwrap();
        let mut engine = TerminalEngine::new(dimensions, 100).unwrap();
        engine.process(bytes);
        engine.snapshot()
    }

    #[test]
    fn viewport_uses_bounded_shared_declarative_primitives() {
        let snapshot =
            snapshot(b"plain \x1b[1;2;3;4;31mred\x1b[0m \x1b[8mhidden\x1b[0m \xe7\x95\x8c");
        let palette = TerminalPalette::default();
        let frame = UiFrame::<()>::layout(
            TerminalViewport::new(
                &snapshot,
                &palette,
                CellMetrics::integral(14.0, 1.0),
                "Fixture",
            )
            .view(),
            Rect::new(0.0, 0.0, 300.0, 120.0),
        );
        let viewport = frame
            .semantic_nodes()
            .into_iter()
            .find(|target| target.role == Some(SemanticRole::GraphicalCustomControl))
            .unwrap();
        assert_eq!(viewport.role, Some(SemanticRole::GraphicalCustomControl));
        assert!(viewport.name.as_deref().unwrap().contains("plain red"));
        assert!(frame.commands().iter().any(|command| {
            matches!(
                command,
                nickel_ui::backend::PaintCommand::StyledText { spans, .. }
                    if spans.iter().any(|span| span.bold && span.italic && span.underline)
            )
        }));
        assert!(
            frame.commands().len() <= 100,
            "visible work stays bounded by the grid"
        );
    }

    fn key(code: KeyCode, modifiers: ModifierState) -> InputEvent {
        InputEvent::Key(KeyEvent {
            device: DeviceId(1),
            order: EventOrder(1),
            physical: PhysicalKey::Code(code),
            logical: LogicalKey::Named(nickel_input::NamedKey::Enter),
            location: KeyLocation::Standard,
            edge: KeyEdge::Pressed,
            repeat: false,
            modifiers,
        })
    }

    #[test]
    fn key_text_and_clipboard_commands_do_not_compete() {
        assert_eq!(
            translate_input(&key(KeyCode::Enter, ModifierState::default())),
            Some(TerminalInputCommand::Write(vec![b'\r']))
        );
        let control_shift = ModifierState::from_sides_and_unsided(
            [],
            [AggregateModifier::Control, AggregateModifier::Shift],
        );
        assert_eq!(
            translate_input(&key(KeyCode::KeyC, control_shift)),
            Some(TerminalInputCommand::Copy)
        );
        let text = InputEvent::Text(TextEvent::Commit {
            device: DeviceId(1),
            order: EventOrder(2),
            text: "é".into(),
        });
        assert_eq!(
            translate_input(&text),
            Some(TerminalInputCommand::Write("é".as_bytes().to_vec()))
        );
        assert_eq!(
            translate_input_with_application_cursor(
                &key(KeyCode::ArrowUp, ModifierState::default()),
                true,
            ),
            Some(TerminalInputCommand::Write(b"\x1bOA".to_vec()))
        );
    }

    #[test]
    fn paste_is_exact_bounded_and_mode_aware() {
        assert_eq!(
            prepare_paste("abc".into(), true),
            PasteDecision::Ready(b"\x1b[200~abc\x1b[201~".to_vec())
        );
        assert!(matches!(
            prepare_paste("a\nb".into(), false),
            PasteDecision::ConfirmationRequired(_)
        ));
        assert_eq!(
            prepare_paste("x".repeat(MAX_PASTE_BYTES + 1), false),
            PasteDecision::RejectedTooLarge
        );
    }

    #[test]
    fn resize_changes_only_at_integral_cell_boundaries() {
        let metrics = CellMetrics::integral(13.0, 1.25);
        let width = metrics.width * 80.0;
        let height = metrics.height * 30.0;
        let initial = metrics.dimensions(width, height);
        assert_eq!(
            metrics.dimensions(width + metrics.width * 0.9, height),
            initial
        );
        assert_eq!(
            metrics.dimensions(width + metrics.width, height).0,
            initial.0 + 1
        );
    }
}
