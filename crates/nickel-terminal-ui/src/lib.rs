//! Declarative Nickel UI projection and normalized input policy for one terminal viewport.

#[cfg(test)]
mod allocation_probe {
    use std::{
        alloc::{GlobalAlloc, Layout, System},
        sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    };

    pub(super) struct CountingAllocator;
    static ENABLED: AtomicBool = AtomicBool::new(false);
    static CALLS: AtomicUsize = AtomicUsize::new(0);
    static BYTES: AtomicUsize = AtomicUsize::new(0);

    unsafe impl GlobalAlloc for CountingAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            if ENABLED.load(Ordering::Relaxed) {
                CALLS.fetch_add(1, Ordering::Relaxed);
                BYTES.fetch_add(layout.size(), Ordering::Relaxed);
            }
            // SAFETY: Forwarded unchanged to the system allocator.
            unsafe { System.alloc(layout) }
        }

        unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
            // SAFETY: The pointer and layout came from the system allocator above.
            unsafe { System.dealloc(pointer, layout) }
        }

        unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
            if ENABLED.load(Ordering::Relaxed) {
                CALLS.fetch_add(1, Ordering::Relaxed);
                BYTES.fetch_add(size, Ordering::Relaxed);
            }
            // SAFETY: The pointer/layout came from System and the new size is forwarded unchanged.
            unsafe { System.realloc(pointer, layout, size) }
        }
    }

    pub(super) fn measure<T>(operation: impl FnOnce() -> T) -> (T, usize, usize) {
        CALLS.store(0, Ordering::Relaxed);
        BYTES.store(0, Ordering::Relaxed);
        ENABLED.store(true, Ordering::SeqCst);
        let result = operation();
        ENABLED.store(false, Ordering::SeqCst);
        (
            result,
            CALLS.load(Ordering::Relaxed),
            BYTES.load(Ordering::Relaxed),
        )
    }
}

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: allocation_probe::CountingAllocator = allocation_probe::CountingAllocator;

use nickel_input::{
    AggregateModifier, InputEvent, KeyCode, KeyEdge, ModifierState, PhysicalKey, PointerButton,
    PointerEvent, TextEvent,
};
use nickel_terminal::{
    TerminalCell, TerminalColor, TerminalNamedColor, TerminalPoint, TerminalScroll,
    TerminalSelectionKind, TerminalSnapshot, TerminalUnderline,
};
use nickel_ui::{
    CustomPaint, Rect, SemanticRole, StyledTextSpan, TextAlign, View, backend::PaintCommand,
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
    BeginSelection(TerminalSelectionKind, TerminalPoint),
    UpdateSelection(TerminalPoint),
    UpdateSelectionAndScroll(TerminalPoint, i32),
    ClearSelection,
    Focus(bool),
}

#[derive(Clone, Debug, Default)]
pub struct TerminalPointerTranslator {
    modifiers: ModifierState,
    selecting: bool,
    pressed_button: Option<PointerButton>,
    last_click: Option<(TerminalPoint, u64)>,
    click_count: u8,
    last_point: Option<TerminalPoint>,
}

impl TerminalPointerTranslator {
    pub fn translate(
        &mut self,
        input: &InputEvent,
        snapshot: &TerminalSnapshot,
        metrics: CellMetrics,
        now_millis: u64,
    ) -> Option<TerminalInputCommand> {
        if let InputEvent::Key(key) = input {
            self.modifiers = key.modifiers.clone();
            return None;
        }
        let selection_override = self.modifiers.aggregate(AggregateModifier::Shift);
        let mouse_reporting = snapshot.mouse_reporting && !selection_override;
        match input {
            InputEvent::Pointer(PointerEvent::Button {
                button,
                edge,
                position: Some(position),
                ..
            }) => {
                let point = terminal_point(position.x, position.y, snapshot, metrics);
                self.last_point = Some(point);
                if mouse_reporting {
                    if *edge == KeyEdge::Pressed {
                        self.pressed_button = Some(button.clone());
                    } else {
                        self.pressed_button = None;
                    }
                    return mouse_button_bytes(button, *edge, point, snapshot.sgr_mouse)
                        .map(TerminalInputCommand::Write);
                }
                if *button != PointerButton::Primary {
                    return None;
                }
                if *edge == KeyEdge::Released {
                    self.selecting = false;
                    return None;
                }
                self.selecting = true;
                if self
                    .last_click
                    .is_some_and(|(last, at)| last == point && now_millis.saturating_sub(at) <= 500)
                {
                    self.click_count = self.click_count.saturating_add(1).min(3);
                } else {
                    self.click_count = 1;
                }
                self.last_click = Some((point, now_millis));
                let kind = match self.click_count {
                    2 => TerminalSelectionKind::Semantic,
                    3 => TerminalSelectionKind::Lines,
                    _ => TerminalSelectionKind::Simple,
                };
                Some(TerminalInputCommand::BeginSelection(kind, point))
            }
            InputEvent::Pointer(PointerEvent::Motion { position, .. }) => {
                let point = terminal_point(position.x, position.y, snapshot, metrics);
                self.last_point = Some(point);
                if mouse_reporting {
                    let button = self.pressed_button.as_ref()?;
                    mouse_motion_bytes(button, point, snapshot.sgr_mouse)
                        .map(TerminalInputCommand::Write)
                } else if self.selecting {
                    let viewport_height = snapshot.lines as f64 * f64::from(metrics.height);
                    let scroll = if position.y < 0.0 {
                        1
                    } else if position.y >= viewport_height {
                        -1
                    } else {
                        0
                    };
                    if scroll == 0 {
                        Some(TerminalInputCommand::UpdateSelection(point))
                    } else {
                        Some(TerminalInputCommand::UpdateSelectionAndScroll(
                            point, scroll,
                        ))
                    }
                } else {
                    None
                }
            }
            InputEvent::Pointer(PointerEvent::Axis {
                delta,
                discrete,
                position,
                ..
            }) => {
                let lines = discrete.map_or_else(
                    || match delta.y.total_cmp(&0.0) {
                        std::cmp::Ordering::Greater => -3,
                        std::cmp::Ordering::Less => 3,
                        std::cmp::Ordering::Equal => 0,
                    },
                    |(_, vertical)| -vertical,
                );
                if lines == 0 {
                    return None;
                }
                if mouse_reporting {
                    let point = position
                        .map(|position| terminal_point(position.x, position.y, snapshot, metrics))
                        .or(self.last_point)
                        .unwrap_or(TerminalPoint { line: 0, column: 0 });
                    self.last_point = Some(point);
                    mouse_wheel_bytes(lines, point, snapshot.sgr_mouse)
                        .map(TerminalInputCommand::Write)
                } else {
                    Some(TerminalInputCommand::Scroll(TerminalScroll::Lines(lines)))
                }
            }
            InputEvent::FocusLost { .. } => {
                self.selecting = false;
                self.pressed_button = None;
                None
            }
            _ => None,
        }
    }
}

fn terminal_point(
    x: f64,
    y: f64,
    snapshot: &TerminalSnapshot,
    metrics: CellMetrics,
) -> TerminalPoint {
    TerminalPoint {
        line: (y.max(0.0) / f64::from(metrics.height))
            .floor()
            .min(snapshot.lines.saturating_sub(1) as f64) as i32,
        column: (x.max(0.0) / f64::from(metrics.width))
            .floor()
            .min(snapshot.columns.saturating_sub(1) as f64) as usize,
    }
}

fn mouse_button_code(button: &PointerButton) -> Option<u8> {
    match button {
        PointerButton::Primary => Some(0),
        PointerButton::Middle => Some(1),
        PointerButton::Secondary => Some(2),
        _ => None,
    }
}

fn sgr_mouse(code: u8, point: TerminalPoint, release: bool) -> Vec<u8> {
    format!(
        "\x1b[<{code};{};{}{}",
        point.column + 1,
        point.line + 1,
        if release { 'm' } else { 'M' }
    )
    .into_bytes()
}

fn legacy_mouse(code: u8, point: TerminalPoint) -> Option<Vec<u8>> {
    let column = u8::try_from(point.column + 33).ok()?;
    let line = u8::try_from(point.line + 33).ok()?;
    Some(vec![
        0x1b,
        b'[',
        b'M',
        code.saturating_add(32),
        column,
        line,
    ])
}

fn mouse_button_bytes(
    button: &PointerButton,
    edge: KeyEdge,
    point: TerminalPoint,
    sgr: bool,
) -> Option<Vec<u8>> {
    let code = if edge == KeyEdge::Released {
        3
    } else {
        mouse_button_code(button)?
    };
    Some(if sgr {
        sgr_mouse(code, point, edge == KeyEdge::Released)
    } else {
        legacy_mouse(code, point)?
    })
}

fn mouse_motion_bytes(button: &PointerButton, point: TerminalPoint, sgr: bool) -> Option<Vec<u8>> {
    let code = mouse_button_code(button)?.saturating_add(32);
    Some(if sgr {
        sgr_mouse(code, point, false)
    } else {
        legacy_mouse(code, point)?
    })
}

fn mouse_wheel_bytes(lines: i32, point: TerminalPoint, sgr: bool) -> Option<Vec<u8>> {
    let code = if lines > 0 { 64 } else { 65 };
    let one = if sgr {
        sgr_mouse(code, point, false)
    } else {
        legacy_mouse(code, point)?
    };
    Some(one.repeat(lines.unsigned_abs().clamp(1, 32) as usize))
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
    translate_input_with_modes(input, false, false)
}

pub fn translate_input_with_application_cursor(
    input: &InputEvent,
    application_cursor: bool,
) -> Option<TerminalInputCommand> {
    translate_input_with_modes(input, application_cursor, false)
}

pub fn translate_input_with_modes(
    input: &InputEvent,
    application_cursor: bool,
    application_keypad: bool,
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
                    _ => key_bytes(
                        code,
                        shift,
                        alt,
                        control,
                        application_cursor,
                        application_keypad,
                    )
                    .map(TerminalInputCommand::Write),
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
                    _ => key_bytes(
                        code,
                        shift,
                        alt,
                        control,
                        application_cursor,
                        application_keypad,
                    )
                    .map(TerminalInputCommand::Write),
                };
            }
            // Ctrl+Alt is the normalized fallback representation of AltGr on backends which
            // cannot expose a distinct modifier. Printable data must arrive through Text::Commit.
            if control
                && !alt
                && let Some(byte) = control_byte(code)
            {
                return Some(TerminalInputCommand::Write(vec![byte]));
            }
            key_bytes(
                code,
                shift,
                alt,
                control,
                application_cursor,
                application_keypad,
            )
            .map(TerminalInputCommand::Write)
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

fn key_bytes(
    code: KeyCode,
    shift: bool,
    alt: bool,
    control: bool,
    application_cursor: bool,
    application_keypad: bool,
) -> Option<Vec<u8>> {
    if let Some(sequence) = modified_special_key(code, shift, alt, control) {
        return Some(sequence);
    }
    let sequence: &[u8] = match code {
        KeyCode::Numpad0 if application_keypad => b"\x1bOp",
        KeyCode::Numpad1 if application_keypad => b"\x1bOq",
        KeyCode::Numpad2 if application_keypad => b"\x1bOr",
        KeyCode::Numpad3 if application_keypad => b"\x1bOs",
        KeyCode::Numpad4 if application_keypad => b"\x1bOt",
        KeyCode::Numpad5 if application_keypad => b"\x1bOu",
        KeyCode::Numpad6 if application_keypad => b"\x1bOv",
        KeyCode::Numpad7 if application_keypad => b"\x1bOw",
        KeyCode::Numpad8 if application_keypad => b"\x1bOx",
        KeyCode::Numpad9 if application_keypad => b"\x1bOy",
        KeyCode::NumpadDecimal if application_keypad => b"\x1bOn",
        KeyCode::NumpadAdd if application_keypad => b"\x1bOk",
        KeyCode::NumpadSubtract if application_keypad => b"\x1bOm",
        KeyCode::NumpadMultiply if application_keypad => b"\x1bOj",
        KeyCode::NumpadDivide if application_keypad => b"\x1bOo",
        KeyCode::NumpadEnter if application_keypad => b"\x1bOM",
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

fn modified_special_key(code: KeyCode, shift: bool, alt: bool, control: bool) -> Option<Vec<u8>> {
    if shift && !alt && !control && code == KeyCode::Tab {
        return Some(b"\x1b[Z".to_vec());
    }
    let modifier = 1 + u8::from(shift) + 2 * u8::from(alt) + 4 * u8::from(control);
    if modifier == 1 {
        return None;
    }
    let sequence = match code {
        KeyCode::ArrowUp => format!("\x1b[1;{modifier}A"),
        KeyCode::ArrowDown => format!("\x1b[1;{modifier}B"),
        KeyCode::ArrowRight => format!("\x1b[1;{modifier}C"),
        KeyCode::ArrowLeft => format!("\x1b[1;{modifier}D"),
        KeyCode::Home => format!("\x1b[1;{modifier}H"),
        KeyCode::End => format!("\x1b[1;{modifier}F"),
        KeyCode::Insert => format!("\x1b[2;{modifier}~"),
        KeyCode::Delete => format!("\x1b[3;{modifier}~"),
        KeyCode::PageUp => format!("\x1b[5;{modifier}~"),
        KeyCode::PageDown => format!("\x1b[6;{modifier}~"),
        KeyCode::F1 => format!("\x1b[1;{modifier}P"),
        KeyCode::F2 => format!("\x1b[1;{modifier}Q"),
        KeyCode::F3 => format!("\x1b[1;{modifier}R"),
        KeyCode::F4 => format!("\x1b[1;{modifier}S"),
        KeyCode::F5 => format!("\x1b[15;{modifier}~"),
        KeyCode::F6 => format!("\x1b[17;{modifier}~"),
        KeyCode::F7 => format!("\x1b[18;{modifier}~"),
        KeyCode::F8 => format!("\x1b[19;{modifier}~"),
        KeyCode::F9 => format!("\x1b[20;{modifier}~"),
        KeyCode::F10 => format!("\x1b[21;{modifier}~"),
        KeyCode::F11 => format!("\x1b[23;{modifier}~"),
        KeyCode::F12 => format!("\x1b[24;{modifier}~"),
        _ => return None,
    };
    Some(sequence.into_bytes())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerminalPalette {
    pub foreground: u32,
    pub background: u32,
    pub cursor: u32,
    pub selection: u32,
    pub indexed: [u32; 16],
    pub cursor_style: nickel_core::terminal_settings::TerminalCursorStyle,
    pub font_family: std::sync::Arc<str>,
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
            cursor_style: nickel_core::terminal_settings::TerminalCursorStyle::Block,
            font_family: std::sync::Arc::from("monospace"),
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

    pub fn resolved(font_family: &str, font_size: f32, scale: f32) -> Self {
        let (width, height) =
            nickel_render_assets::monospace_cell_geometry(font_family, font_size, scale);
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
        let visible = visible_text(self.snapshot);
        CustomPaint::commands(self.paint_commands())
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
    }

    fn paint_commands(&self) -> Vec<PaintCommand> {
        let width = self.snapshot.columns as f32 * self.metrics.width;
        let height = self.snapshot.lines as f32 * self.metrics.height;
        let mut commands = Vec::with_capacity(self.snapshot.lines.saturating_mul(4) + 1);
        commands.push(PaintCommand::Fill {
            rect: Rect::new(0.0, 0.0, width, height),
            color: self.palette.background,
        });
        for (index, cell) in self.snapshot.cells.iter().enumerate() {
            self.paint_cell(cell, index, &mut commands);
        }
        commands
    }

    fn paint_cell(&self, cell: &TerminalCell, index: usize, commands: &mut Vec<PaintCommand>) {
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
        if cursor
            && self.palette.cursor_style
                == nickel_core::terminal_settings::TerminalCursorStyle::Block
        {
            background = self.palette.cursor;
            foreground = self.palette.background;
        }
        let column = index % self.snapshot.columns;
        let line = index / self.snapshot.columns;
        let rect = Rect::new(
            column as f32 * self.metrics.width,
            line as f32 * self.metrics.height,
            self.metrics.width,
            self.metrics.height,
        );
        if background != self.palette.background {
            commands.push(PaintCommand::Fill {
                rect,
                color: background,
            });
        }
        let paints_text = !cell.concealed
            && !cell.wide_spacer
            && (cell.character != ' '
                || !cell.combining.is_empty()
                || cell.underline != TerminalUnderline::None);
        if paints_text {
            let mut value = cell.character.to_string();
            value.extend(cell.combining.iter());
            let end = value.len();
            commands.push(PaintCommand::StyledText {
                bounds: rect,
                text: value,
                spans: vec![StyledTextSpan {
                    range: 0..end,
                    bold: cell.bold,
                    italic: cell.italic,
                    monospace: true,
                    font_family: Some(std::sync::Arc::clone(&self.palette.font_family)),
                    strikethrough: false,
                    underline: match cell.underline {
                        TerminalUnderline::None => nickel_ui::TextUnderlineStyle::None,
                        TerminalUnderline::Single => nickel_ui::TextUnderlineStyle::Single,
                        TerminalUnderline::Double => nickel_ui::TextUnderlineStyle::Double,
                        TerminalUnderline::Curly => nickel_ui::TextUnderlineStyle::Curly,
                        TerminalUnderline::Dotted => nickel_ui::TextUnderlineStyle::Dotted,
                        TerminalUnderline::Dashed => nickel_ui::TextUnderlineStyle::Dashed,
                    },
                    color: Some(foreground),
                    background: None,
                }],
                scale: self.metrics.text_scale,
                font_size: Some(self.metrics.text_scale * 8.0),
                color: foreground,
                align: TextAlign::Start,
            });
        }
        if cursor
            && self.palette.cursor_style
                == nickel_core::terminal_settings::TerminalCursorStyle::Beam
        {
            commands.push(PaintCommand::Stroke {
                rect: Rect::new(rect.origin.x, rect.origin.y, 1.0, rect.size.height),
                color: self.palette.cursor,
                width: 1.0,
            });
        } else if cursor
            && self.palette.cursor_style
                == nickel_core::terminal_settings::TerminalCursorStyle::Underline
        {
            commands.push(PaintCommand::Stroke {
                rect: Rect::new(
                    rect.origin.x,
                    rect.origin.y + rect.size.height - 1.0,
                    rect.size.width,
                    1.0,
                ),
                color: self.palette.cursor,
                width: 1.0,
            });
        }
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
        TerminalColor::Named(
            TerminalNamedColor::Foreground | TerminalNamedColor::BrightForeground,
        ) => palette.foreground,
        TerminalColor::Named(TerminalNamedColor::Background) => palette.background,
        TerminalColor::Named(name) => named_index(*name).map_or_else(
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

fn named_index(name: TerminalNamedColor) -> Option<usize> {
    Some(match name {
        TerminalNamedColor::Black | TerminalNamedColor::DimBlack => 0,
        TerminalNamedColor::Red | TerminalNamedColor::DimRed => 1,
        TerminalNamedColor::Green | TerminalNamedColor::DimGreen => 2,
        TerminalNamedColor::Yellow | TerminalNamedColor::DimYellow => 3,
        TerminalNamedColor::Blue | TerminalNamedColor::DimBlue => 4,
        TerminalNamedColor::Magenta | TerminalNamedColor::DimMagenta => 5,
        TerminalNamedColor::Cyan | TerminalNamedColor::DimCyan => 6,
        TerminalNamedColor::White | TerminalNamedColor::DimWhite => 7,
        TerminalNamedColor::BrightBlack => 8,
        TerminalNamedColor::BrightRed => 9,
        TerminalNamedColor::BrightGreen => 10,
        TerminalNamedColor::BrightYellow => 11,
        TerminalNamedColor::BrightBlue => 12,
        TerminalNamedColor::BrightMagenta => 13,
        TerminalNamedColor::BrightCyan => 14,
        TerminalNamedColor::BrightWhite => 15,
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
    use nickel_input::{
        DeviceId, EventOrder, KeyEvent, KeyLocation, LogicalKey, Modifier, ModifierState,
        Point as InputPoint,
    };
    use nickel_terminal::{TerminalDimensions, TerminalEngine};
    use nickel_ui::{Rect, SoftwareRenderer, UiFrame};

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
        let palette = TerminalPalette {
            font_family: std::sync::Arc::from("Nickel Fixture Mono"),
            ..TerminalPalette::default()
        };
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
        let commands = format!("{:?}", frame.commands());
        assert!(commands.contains("bold: true"));
        assert!(commands.contains("italic: true"));
        assert!(commands.contains("underline: Single"));
        assert!(commands.contains("Nickel Fixture Mono"));
        assert!(commands.contains("font_size: Some(14.0)"));
        assert!(
            frame.commands().len() <= 100,
            "visible work stays bounded by the grid"
        );
    }

    #[test]
    #[ignore = "explicit release-mode full-frame allocation probe"]
    fn release_full_frame_allocation_budget() {
        if cfg!(debug_assertions) {
            panic!("frame allocation budgets require --release");
        }
        let dimensions = TerminalDimensions::new(160, 60, 8, 16).unwrap();
        let mut engine = TerminalEngine::new(dimensions, 10_000).unwrap();
        let dense_output = format!("{}\r\n", "X".repeat(159)).repeat(60);
        engine.process(dense_output.as_bytes());
        let snapshot = engine.snapshot();
        let palette = TerminalPalette::default();
        let started = std::time::Instant::now();
        let (frame, allocation_calls, allocation_bytes) = crate::allocation_probe::measure(|| {
            UiFrame::<()>::layout(
                TerminalViewport::new(
                    &snapshot,
                    &palette,
                    CellMetrics::integral(14.0, 1.0),
                    "Resource probe",
                )
                .view(),
                Rect::new(0.0, 0.0, 1600.0, 1140.0),
            )
        });
        let elapsed = started.elapsed();
        assert!(
            elapsed <= std::time::Duration::from_millis(20),
            "{elapsed:?}"
        );
        assert!(allocation_calls <= 50_000, "{allocation_calls}");
        assert!(allocation_bytes <= 8 * 1024 * 1024, "{allocation_bytes}");
        let mut renderer = SoftwareRenderer::new_pixel_buffer(1600, 1140, 1.0);
        let cold_render_started = std::time::Instant::now();
        renderer.render(frame.commands());
        let cold_render_elapsed = cold_render_started.elapsed();
        assert!(
            cold_render_elapsed <= std::time::Duration::from_millis(750),
            "{cold_render_elapsed:?}"
        );
        let warm_render_started = std::time::Instant::now();
        renderer.render(frame.commands());
        let warm_render_elapsed = warm_render_started.elapsed();
        assert!(
            warm_render_elapsed <= std::time::Duration::from_millis(10),
            "{warm_render_elapsed:?}"
        );
        eprintln!(
            "terminal full-frame probe: elapsed={elapsed:?}, allocations={allocation_calls}, \
             allocated_bytes={allocation_bytes}, commands={}, cold_render={cold_render_elapsed:?}, \
             warm_render={warm_render_elapsed:?}",
            frame.commands().len()
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
        let alt_graph = ModifierState::from_sides([Modifier::ControlLeft, Modifier::AltRight]);
        assert_eq!(translate_input(&key(KeyCode::KeyQ, alt_graph)), None);
        let alt_graph_text = InputEvent::Text(TextEvent::Commit {
            device: DeviceId(1),
            order: EventOrder(3),
            text: "@".into(),
        });
        assert_eq!(
            translate_input(&alt_graph_text),
            Some(TerminalInputCommand::Write(vec![b'@']))
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
    fn application_keypad_emits_vt_keypad_sequences() {
        let numpad_seven = key(KeyCode::Numpad7, ModifierState::default());
        assert_eq!(
            translate_input_with_modes(&numpad_seven, false, false),
            None
        );
        assert_eq!(
            translate_input_with_modes(&numpad_seven, false, true),
            Some(TerminalInputCommand::Write(b"\x1bOw".to_vec()))
        );
        assert_eq!(
            translate_input_with_modes(
                &key(KeyCode::NumpadEnter, ModifierState::default()),
                false,
                true,
            ),
            Some(TerminalInputCommand::Write(b"\x1bOM".to_vec()))
        );
    }

    #[test]
    fn navigation_and_function_keys_preserve_xterm_modifiers() {
        let control = ModifierState::from_sides_and_unsided([], [AggregateModifier::Control]);
        let alt = ModifierState::from_sides_and_unsided([], [AggregateModifier::Alt]);
        let shift = ModifierState::from_sides_and_unsided([], [AggregateModifier::Shift]);
        let control_shift = ModifierState::from_sides_and_unsided(
            [],
            [AggregateModifier::Control, AggregateModifier::Shift],
        );
        assert_eq!(
            translate_input(&key(KeyCode::ArrowRight, control)),
            Some(TerminalInputCommand::Write(b"\x1b[1;5C".to_vec()))
        );
        assert_eq!(
            translate_input(&key(KeyCode::F5, alt)),
            Some(TerminalInputCommand::Write(b"\x1b[15;3~".to_vec()))
        );
        assert_eq!(
            translate_input(&key(KeyCode::Tab, shift)),
            Some(TerminalInputCommand::Write(b"\x1b[Z".to_vec()))
        );
        assert_eq!(
            translate_input(&key(KeyCode::F12, control_shift)),
            Some(TerminalInputCommand::Write(b"\x1b[24;6~".to_vec()))
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

    fn pointer_button(edge: KeyEdge, x: f64, y: f64) -> InputEvent {
        InputEvent::Pointer(PointerEvent::Button {
            device: DeviceId(1),
            order: EventOrder(1),
            button: PointerButton::Primary,
            edge,
            position: Some(InputPoint { x, y }),
        })
    }

    #[test]
    fn pointer_selection_supports_drag_word_line_and_shift_override() {
        let selection_snapshot = snapshot(b"one two");
        let metrics = CellMetrics {
            width: 10.0,
            height: 20.0,
            text_scale: 1.0,
        };
        let mut pointer = TerminalPointerTranslator::default();
        let press = pointer_button(KeyEdge::Pressed, 25.0, 5.0);
        assert_eq!(
            pointer.translate(&press, &selection_snapshot, metrics, 10),
            Some(TerminalInputCommand::BeginSelection(
                TerminalSelectionKind::Simple,
                TerminalPoint { line: 0, column: 2 }
            ))
        );
        assert_eq!(
            pointer.translate(&press, &selection_snapshot, metrics, 200),
            Some(TerminalInputCommand::BeginSelection(
                TerminalSelectionKind::Semantic,
                TerminalPoint { line: 0, column: 2 }
            ))
        );
        assert_eq!(
            pointer.translate(&press, &selection_snapshot, metrics, 300),
            Some(TerminalInputCommand::BeginSelection(
                TerminalSelectionKind::Lines,
                TerminalPoint { line: 0, column: 2 }
            ))
        );
        let motion = InputEvent::Pointer(PointerEvent::Motion {
            device: DeviceId(1),
            order: EventOrder(2),
            position: InputPoint { x: 55.0, y: 25.0 },
            delta: None,
        });
        assert_eq!(
            pointer.translate(&motion, &selection_snapshot, metrics, 310),
            Some(TerminalInputCommand::UpdateSelection(TerminalPoint {
                line: 1,
                column: 5,
            }))
        );
        let beyond_bottom = InputEvent::Pointer(PointerEvent::Motion {
            device: DeviceId(1),
            order: EventOrder(3),
            position: InputPoint { x: 55.0, y: 80.0 },
            delta: None,
        });
        assert_eq!(
            pointer.translate(&beyond_bottom, &selection_snapshot, metrics, 320),
            Some(TerminalInputCommand::UpdateSelectionAndScroll(
                TerminalPoint { line: 2, column: 5 },
                -1,
            ))
        );

        let reporting = snapshot(b"\x1b[?1000h\x1b[?1006h");
        let shift = ModifierState::from_sides_and_unsided([], [AggregateModifier::Shift]);
        pointer.translate(&key(KeyCode::ShiftLeft, shift), &reporting, metrics, 400);
        assert!(matches!(
            pointer.translate(&press, &reporting, metrics, 900),
            Some(TerminalInputCommand::BeginSelection(..))
        ));
    }

    #[test]
    fn terminal_mouse_reporting_uses_mode_aware_sgr_coordinates() {
        let reporting = snapshot(b"\x1b[?1000h\x1b[?1006h");
        assert!(reporting.mouse_reporting);
        assert!(reporting.sgr_mouse);
        let metrics = CellMetrics {
            width: 10.0,
            height: 20.0,
            text_scale: 1.0,
        };
        let mut pointer = TerminalPointerTranslator::default();
        assert_eq!(
            pointer.translate(
                &pointer_button(KeyEdge::Pressed, 25.0, 25.0),
                &reporting,
                metrics,
                0,
            ),
            Some(TerminalInputCommand::Write(b"\x1b[<0;3;2M".to_vec()))
        );
        assert_eq!(
            pointer.translate(
                &pointer_button(KeyEdge::Released, 25.0, 25.0),
                &reporting,
                metrics,
                1,
            ),
            Some(TerminalInputCommand::Write(b"\x1b[<3;3;2m".to_vec()))
        );
        let wheel = InputEvent::Pointer(PointerEvent::Axis {
            device: DeviceId(1),
            order: EventOrder(3),
            delta: nickel_input::Vector { x: 0.0, y: -1.0 },
            discrete: Some((0, -2)),
            position: Some(InputPoint { x: 45.0, y: 5.0 }),
        });
        assert_eq!(
            pointer.translate(&wheel, &reporting, metrics, 2),
            Some(TerminalInputCommand::Write(
                b"\x1b[<64;5;1M\x1b[<64;5;1M".to_vec()
            ))
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
        for scale in [1.0, 1.25, 2.0] {
            let resolved = CellMetrics::resolved("monospace", 13.0, scale);
            assert_eq!((resolved.width * scale).fract(), 0.0);
            assert_eq!((resolved.height * scale).fract(), 0.0);
        }
    }
}
