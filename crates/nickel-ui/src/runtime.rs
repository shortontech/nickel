use std::{
    error::Error,
    time::{Duration, Instant},
};

use sdl3::{
    event::{Event, WindowEvent},
    keyboard::{Keycode, Mod},
    mouse::{Cursor as MouseCursor, MouseButton, SystemCursor},
};

use crate::{Point, PointerIcon, Rect, SdlCanvasPresenter, UiEvent, UiStateStore, UiTree, View};

#[derive(Debug, Default)]
struct FrameScheduler {
    dirty: bool,
}

impl FrameScheduler {
    fn invalidate(&mut self) {
        self.dirty = true;
    }

    fn take_rebuild(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shortcut {
    Submit,
    Newline,
    Escape,
    Reload,
    Back,
    Forward,
    DocumentStart,
    DocumentEnd,
}

pub trait Application: Sized {
    type Message: Clone;

    fn update(&mut self, message: Self::Message);

    fn view(&self) -> impl View<Self::Message>;

    /// Poll application-owned background work without introducing another UI runtime.
    /// Return `true` when new state requires a redraw.
    fn poll(&mut self) -> bool {
        false
    }

    /// Handle application-level keyboard semantics before ordinary component activation.
    fn shortcut(&mut self, _shortcut: Shortcut) -> bool {
        false
    }

    fn title(&self) -> &str {
        "Nickel UI"
    }

    fn initial_size(&self) -> (u32, u32) {
        (800, 600)
    }
}

pub fn run<A: Application>(mut application: A) -> Result<(), Box<dyn Error>> {
    let sdl = sdl3::init()?;
    let video = sdl.video()?;
    let clipboard = video.clipboard();
    let mut events = sdl.event_pump()?;
    let (width, height) = application.initial_size();
    let window = video
        .window(application.title(), width, height)
        .position_centered()
        .resizable()
        .high_pixel_density()
        .build()?;
    let mut presenter = SdlCanvasPresenter::new(window)?;
    let text_input = video.text_input();
    text_input.start(presenter.window());
    let mut state = UiStateStore::default();
    let mut cursor = Point::default();
    let default_cursor = MouseCursor::from_system(SystemCursor::Arrow).ok();
    let hand_cursor = MouseCursor::from_system(SystemCursor::Hand).ok();
    let text_cursor = MouseCursor::from_system(SystemCursor::IBeam).ok();
    let mut pointer_icon = PointerIcon::Default;
    let mut running = true;
    let (logical_width, logical_height) = presenter.window().size();
    let pixel_width = presenter.window().size_in_pixels().0;
    let mut scale = pixel_width as f32 / logical_width.max(1) as f32;
    let mut tree = UiTree::layout_with_state(
        application.view(),
        Rect::new(0.0, 0.0, logical_width as f32, logical_height as f32),
        &mut state,
    );
    presenter.present_accelerated(tree.commands(), scale)?;
    let mut scheduler = FrameScheduler::default();
    let mut next_caret_blink = Instant::now() + Duration::from_millis(500);

    while running {
        let caret_tick = Instant::now() >= next_caret_blink;
        if caret_tick {
            next_caret_blink = Instant::now() + Duration::from_millis(500);
            if state.toggle_caret() != crate::Invalidation::None {
                scheduler.invalidate();
            }
        }
        if application.poll() {
            scheduler.invalidate();
        }
        if scheduler.take_rebuild() {
            let (logical_width, logical_height) = presenter.window().size();
            let pixel_width = presenter.window().size_in_pixels().0;
            scale = pixel_width as f32 / logical_width.max(1) as f32;
            tree = UiTree::layout_with_state(
                application.view(),
                Rect::new(0.0, 0.0, logical_width as f32, logical_height as f32),
                &mut state,
            );
            presenter.present_accelerated(tree.commands(), scale)?;
        }

        let Some(event) = events.wait_event_timeout(Duration::from_millis(16)) else {
            continue;
        };
        let mut pending = vec![event];
        pending.extend(events.poll_iter());
        for event in pending {
            let event = match event {
                Event::Quit { .. }
                | Event::Window {
                    win_event: WindowEvent::CloseRequested,
                    ..
                } => {
                    running = false;
                    continue;
                }
                Event::Window {
                    win_event: WindowEvent::Resized(_, _) | WindowEvent::PixelSizeChanged(_, _),
                    ..
                } => {
                    scheduler.invalidate();
                    continue;
                }
                Event::Window {
                    win_event: WindowEvent::FocusLost,
                    ..
                } => UiEvent::FocusLost,
                Event::MouseMotion { x, y, .. } => {
                    cursor = Point { x, y };
                    let next_icon = tree.pointer_icon_at(cursor);
                    if next_icon != pointer_icon {
                        if let Some(cursor) = match next_icon {
                            PointerIcon::Default => default_cursor.as_ref(),
                            PointerIcon::Hand => hand_cursor.as_ref(),
                            PointerIcon::Text => text_cursor.as_ref(),
                        } {
                            cursor.set();
                        }
                        pointer_icon = next_icon;
                    }
                    UiEvent::PointerMoved(cursor)
                }
                Event::MouseButtonDown {
                    mouse_btn: MouseButton::Left,
                    x,
                    y,
                    ..
                } => {
                    cursor = Point { x, y };
                    UiEvent::PointerPressed(cursor)
                }
                Event::MouseButtonUp {
                    mouse_btn: MouseButton::Left,
                    x,
                    y,
                    ..
                } => {
                    cursor = Point { x, y };
                    UiEvent::PointerReleased(cursor)
                }
                Event::MouseWheel { x, y, .. } if x.abs() > y.abs() => UiEvent::ScrollHorizontal {
                    point: cursor,
                    delta_x: -x * 42.0,
                },
                Event::MouseWheel { y, .. } => UiEvent::Scroll {
                    point: cursor,
                    delta_y: -y * 42.0,
                },
                Event::KeyDown {
                    keycode: Some(Keycode::R),
                    keymod,
                    ..
                } if command_modifier(keymod) && application.shortcut(Shortcut::Reload) => {
                    scheduler.invalidate();
                    continue;
                }
                Event::KeyDown {
                    keycode: Some(Keycode::Left),
                    keymod,
                    ..
                } if keymod.intersects(Mod::LALTMOD | Mod::RALTMOD)
                    && application.shortcut(Shortcut::Back) =>
                {
                    scheduler.invalidate();
                    continue;
                }
                Event::KeyDown {
                    keycode: Some(Keycode::Right),
                    keymod,
                    ..
                } if keymod.intersects(Mod::LALTMOD | Mod::RALTMOD)
                    && application.shortcut(Shortcut::Forward) =>
                {
                    scheduler.invalidate();
                    continue;
                }
                Event::KeyDown {
                    keycode: Some(Keycode::Home),
                    ..
                } if application.shortcut(Shortcut::DocumentStart) => {
                    scheduler.invalidate();
                    continue;
                }
                Event::KeyDown {
                    keycode: Some(Keycode::End),
                    ..
                } if application.shortcut(Shortcut::DocumentEnd) => {
                    scheduler.invalidate();
                    continue;
                }
                Event::KeyDown {
                    keycode: Some(Keycode::Return),
                    keymod,
                    ..
                } if keymod.intersects(Mod::LSHIFTMOD | Mod::RSHIFTMOD) => {
                    if state.focused().is_some() {
                        UiEvent::TextInput("\n".into())
                    } else if application.shortcut(Shortcut::Newline) {
                        scheduler.invalidate();
                        continue;
                    } else {
                        UiEvent::KeyboardActivate
                    }
                }
                Event::KeyDown {
                    keycode: Some(Keycode::Return),
                    ..
                } => {
                    if application.shortcut(Shortcut::Submit) {
                        scheduler.invalidate();
                        continue;
                    }
                    UiEvent::KeyboardActivate
                }
                Event::KeyDown {
                    keycode: Some(Keycode::Escape),
                    ..
                } => {
                    if state.selection_owner().is_some() {
                        UiEvent::SelectionClear
                    } else {
                        if application.shortcut(Shortcut::Escape) {
                            scheduler.invalidate();
                            continue;
                        }
                        UiEvent::Dismiss
                    }
                }
                Event::KeyDown {
                    keycode: Some(Keycode::Tab),
                    keymod,
                    ..
                } if keymod.intersects(Mod::LSHIFTMOD | Mod::RSHIFTMOD) => UiEvent::FocusPrevious,
                Event::KeyDown {
                    keycode: Some(Keycode::Tab),
                    ..
                } => UiEvent::FocusNext,
                Event::KeyDown {
                    keycode: Some(Keycode::A),
                    keymod,
                    ..
                } if command_modifier(keymod) => UiEvent::TextSelectAll,
                Event::KeyDown {
                    keycode: Some(Keycode::C),
                    keymod,
                    ..
                } if command_modifier(keymod) => UiEvent::TextCopy,
                Event::KeyDown {
                    keycode: Some(Keycode::X),
                    keymod,
                    ..
                } if command_modifier(keymod) => {
                    let Some(selected) = tree.selected_text(&state) else {
                        continue;
                    };
                    if clipboard.set_clipboard_text(&selected).is_err() {
                        continue;
                    }
                    UiEvent::TextCut
                }
                Event::KeyDown {
                    keycode: Some(Keycode::V),
                    keymod,
                    ..
                } if command_modifier(keymod) => {
                    let Ok(text) = clipboard.clipboard_text() else {
                        continue;
                    };
                    UiEvent::TextPaste(text)
                }
                Event::KeyDown {
                    keycode: Some(Keycode::Backspace),
                    keymod,
                    ..
                } if keymod.intersects(Mod::LCTRLMOD | Mod::RCTRLMOD) => UiEvent::TextBackspaceWord,
                Event::KeyDown {
                    keycode: Some(Keycode::Backspace),
                    ..
                } => UiEvent::TextBackspace,
                Event::KeyDown {
                    keycode: Some(Keycode::Delete),
                    ..
                } => UiEvent::TextDelete,
                Event::KeyDown {
                    keycode: Some(Keycode::Left),
                    keymod,
                    ..
                } if keymod.intersects(Mod::LCTRLMOD | Mod::RCTRLMOD) => {
                    UiEvent::TextMoveWordLeft {
                        extend_selection: keymod.intersects(Mod::LSHIFTMOD | Mod::RSHIFTMOD),
                    }
                }
                Event::KeyDown {
                    keycode: Some(Keycode::Left),
                    keymod,
                    ..
                } => UiEvent::TextMoveLeft {
                    extend_selection: keymod.intersects(Mod::LSHIFTMOD | Mod::RSHIFTMOD),
                },
                Event::KeyDown {
                    keycode: Some(Keycode::Right),
                    keymod,
                    ..
                } if keymod.intersects(Mod::LCTRLMOD | Mod::RCTRLMOD) => {
                    UiEvent::TextMoveWordRight {
                        extend_selection: keymod.intersects(Mod::LSHIFTMOD | Mod::RSHIFTMOD),
                    }
                }
                Event::KeyDown {
                    keycode: Some(Keycode::Right),
                    keymod,
                    ..
                } => UiEvent::TextMoveRight {
                    extend_selection: keymod.intersects(Mod::LSHIFTMOD | Mod::RSHIFTMOD),
                },
                Event::KeyDown {
                    keycode: Some(Keycode::Home),
                    keymod,
                    ..
                } if keymod.intersects(Mod::LCTRLMOD | Mod::RCTRLMOD) => {
                    UiEvent::TextMoveDocumentHome {
                        extend_selection: keymod.intersects(Mod::LSHIFTMOD | Mod::RSHIFTMOD),
                    }
                }
                Event::KeyDown {
                    keycode: Some(Keycode::Home),
                    keymod,
                    ..
                } => UiEvent::TextMoveHome {
                    extend_selection: keymod.intersects(Mod::LSHIFTMOD | Mod::RSHIFTMOD),
                },
                Event::KeyDown {
                    keycode: Some(Keycode::End),
                    keymod,
                    ..
                } if keymod.intersects(Mod::LCTRLMOD | Mod::RCTRLMOD) => {
                    UiEvent::TextMoveDocumentEnd {
                        extend_selection: keymod.intersects(Mod::LSHIFTMOD | Mod::RSHIFTMOD),
                    }
                }
                Event::KeyDown {
                    keycode: Some(Keycode::End),
                    keymod,
                    ..
                } => UiEvent::TextMoveEnd {
                    extend_selection: keymod.intersects(Mod::LSHIFTMOD | Mod::RSHIFTMOD),
                },
                Event::KeyDown {
                    keycode: Some(Keycode::Space),
                    ..
                } => UiEvent::KeyboardActivate,
                Event::TextInput { text, .. } => UiEvent::TextInput(text),
                Event::TextEditing { text, .. } => UiEvent::ImePreedit(text),
                _ => continue,
            };
            let resets_caret = matches!(
                &event,
                UiEvent::PointerPressed(_)
                    | UiEvent::PointerMoved(_)
                    | UiEvent::TextInput(_)
                    | UiEvent::TextBackspace
                    | UiEvent::TextBackspaceWord
                    | UiEvent::TextDelete
                    | UiEvent::TextMoveLeft { .. }
                    | UiEvent::TextMoveRight { .. }
                    | UiEvent::TextMoveWordLeft { .. }
                    | UiEvent::TextMoveWordRight { .. }
                    | UiEvent::TextMoveHome { .. }
                    | UiEvent::TextMoveEnd { .. }
                    | UiEvent::TextMoveDocumentHome { .. }
                    | UiEvent::TextMoveDocumentEnd { .. }
                    | UiEvent::TextSelectAll
                    | UiEvent::TextCut
                    | UiEvent::TextPaste(_)
            );
            let outcome = tree.handle_event(&mut state, event);
            if resets_caret {
                next_caret_blink = Instant::now() + Duration::from_millis(500);
            }
            for message in outcome.messages {
                application.update(message);
            }
            if let Some(text) = outcome.clipboard_text {
                let _ = clipboard.set_clipboard_text(&text);
            }
            if outcome.invalidation != crate::Invalidation::None {
                scheduler.invalidate();
            }
        }
    }
    text_input.stop(presenter.window());
    state.destroy();
    Ok(())
}

fn command_modifier(keymod: Mod) -> bool {
    keymod.intersects(Mod::LCTRLMOD | Mod::RCTRLMOD | Mod::LGUIMOD | Mod::RGUIMOD)
}

#[cfg(test)]
mod tests {
    use super::FrameScheduler;
    use crate::{Invalidation, UiStateStore};

    #[test]
    fn idle_frames_do_not_rebuild_and_event_batches_coalesce() {
        let mut scheduler = FrameScheduler::default();
        assert!(!scheduler.take_rebuild());
        scheduler.invalidate();
        scheduler.invalidate();
        scheduler.invalidate();
        assert!(scheduler.take_rebuild());
        assert!(!scheduler.take_rebuild());
    }

    #[test]
    fn an_unfocused_caret_tick_does_not_invalidate_the_window() {
        let mut state = UiStateStore::default();

        assert_eq!(state.toggle_caret(), Invalidation::None);
    }
}
