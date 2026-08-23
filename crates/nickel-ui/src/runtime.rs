use std::{error::Error, time::Duration};

use sdl3::{
    event::{Event, WindowEvent},
    keyboard::{Keycode, Mod},
    mouse::MouseButton,
};

use crate::{Point, Rect, SdlCanvasPresenter, UiEvent, UiStateStore, UiTree, View};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shortcut {
    Submit,
    Newline,
    Escape,
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
    let mut running = true;
    let mut dirty = true;

    while running {
        dirty |= application.poll();
        let (logical_width, logical_height) = presenter.window().size();
        let pixel_width = presenter.window().size_in_pixels().0;
        let scale = pixel_width as f32 / logical_width.max(1) as f32;
        let tree = UiTree::layout_with_state(
            application.view(),
            Rect::new(0.0, 0.0, logical_width as f32, logical_height as f32),
            &mut state,
        );
        if dirty {
            presenter.present_accelerated(tree.commands(), scale)?;
            dirty = false;
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
                    dirty = true;
                    continue;
                }
                Event::Window {
                    win_event: WindowEvent::FocusLost,
                    ..
                } => UiEvent::FocusLost,
                Event::MouseMotion { x, y, .. } => {
                    cursor = Point { x, y };
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
                    keycode: Some(Keycode::Return),
                    keymod,
                    ..
                } if keymod.intersects(Mod::LSHIFTMOD | Mod::RSHIFTMOD) => {
                    if application.shortcut(Shortcut::Newline) {
                        dirty = true;
                        continue;
                    }
                    UiEvent::KeyboardActivate
                }
                Event::KeyDown {
                    keycode: Some(Keycode::Return),
                    ..
                } => {
                    if application.shortcut(Shortcut::Submit) {
                        dirty = true;
                        continue;
                    }
                    UiEvent::KeyboardActivate
                }
                Event::KeyDown {
                    keycode: Some(Keycode::Escape),
                    ..
                } => {
                    if application.shortcut(Shortcut::Escape) {
                        dirty = true;
                    }
                    continue;
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
                } if keymod.intersects(Mod::LCTRLMOD | Mod::RCTRLMOD) => UiEvent::TextSelectAll,
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
                } => UiEvent::TextMoveLeft {
                    extend_selection: keymod.intersects(Mod::LSHIFTMOD | Mod::RSHIFTMOD),
                },
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
                } => UiEvent::TextMoveHome {
                    extend_selection: keymod.intersects(Mod::LSHIFTMOD | Mod::RSHIFTMOD),
                },
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
            let outcome = tree.handle_event(&mut state, event);
            for message in outcome.messages {
                application.update(message);
            }
            if outcome.invalidation != crate::Invalidation::None {
                dirty = true;
            }
        }
    }
    text_input.stop(presenter.window());
    state.destroy();
    Ok(())
}
