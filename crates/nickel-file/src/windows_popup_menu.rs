//! Nickel-rendered context menus in a dedicated Windows popup window.

use std::{sync::mpsc, time::Duration};

use nickel_ui::{
    AdapterOutcome, Application, Container, FrameOverlay, HostAdapter, HostServices, OverlayAnchor,
    OverlayMenu, OverlayMenuItem, Point, UiHost, UiId, View, ViewContext,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::{
    Foundation::{HWND, POINT},
    UI::WindowsAndMessaging::{
        GWL_EXSTYLE, GWLP_HWNDPARENT, GetCursorPos, GetWindowLongPtrW, SetWindowLongPtrW,
        WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
    },
};
use winit::{
    dpi::PhysicalPosition,
    event::WindowEvent,
    window::{Window, WindowAttributes, WindowLevel},
};

#[derive(Clone)]
enum Choice<Message> {
    Action(Message),
    Submenu(Vec<OverlayMenuItem<Message>>),
}

struct PopupApplication<Message> {
    menu: OverlayMenu<usize>,
    choices: Vec<Choice<Message>>,
    selected: Option<usize>,
    size: (u32, u32),
}

impl<Message: Clone> PopupApplication<Message> {
    fn new(source: OverlayMenu<Message>) -> Self {
        let source = source.fit_width_to_content(320.0);
        let menu_width = source.width
            + if source.items.iter().any(|item| !item.children.is_empty()) {
                18.0
            } else {
                0.0
            };
        let width = menu_width.ceil() as u32 + 4;
        let height = (source.padding.top
            + source.row_height * source.items.len() as f32
            + source.row_gap * source.items.len().saturating_sub(1) as f32
            + source.padding.bottom)
            .ceil() as u32
            + 4;
        let mut menu = OverlayMenu::new(
            "windows-detached-context",
            OverlayAnchor::Point {
                invocation_target: UiId::from("popup-anchor"),
                point: Point { x: 2.0, y: 2.0 },
            },
        );
        menu.width = menu_width;
        menu.row_height = source.row_height;
        menu.row_gap = source.row_gap;
        menu.padding = source.padding;
        menu.radius = source.radius;
        menu.background = source.background;
        menu.border = source.border;
        menu.border_width = source.border_width;
        menu.foreground = source.foreground;
        menu.text_scale = source.text_scale;
        menu.text_align = source.text_align;
        menu.item_hover = source.item_hover;
        menu.item_pressed = source.item_pressed;
        menu.item_selected = source.item_selected;
        menu.item_radius = source.item_radius;
        menu.direction = source.direction;
        let mut choices = Vec::new();
        for item in source.items {
            let choice = if item.children.is_empty() {
                item.action.clone().map(Choice::Action)
            } else {
                Some(Choice::Submenu(item.children.clone()))
            };
            let label = if item.children.is_empty() {
                item.label
            } else {
                format!("{}  ›", item.label)
            };
            let mut mapped = if let Some(choice) = choice {
                choices.push(choice);
                OverlayMenuItem::action(item.id, label, choices.len() - 1)
            } else {
                OverlayMenuItem::disabled(item.id, label)
            };
            mapped.shortcut = item.shortcut;
            mapped.separator_before = item.separator_before;
            mapped.disabled_reason = item.disabled_reason;
            mapped.tone = item.tone;
            menu.items.push(mapped);
        }
        Self {
            menu,
            choices,
            selected: None,
            size: (width, height),
        }
    }

    fn take_choice(&mut self) -> Option<Choice<Message>> {
        self.selected
            .take()
            .and_then(|index| self.choices.get(index).cloned())
    }
}

impl<Message: Clone> Application for PopupApplication<Message> {
    type Message = usize;

    fn update(&mut self, message: usize) {
        self.selected = Some(message);
    }

    fn view(&self, _: ViewContext) -> impl View<Self::Message> {
        Container::new()
            .id("popup-anchor")
            .width(self.size.0 as f32)
            .height(self.size.1 as f32)
    }

    fn frame_overlays(&self, _: ViewContext) -> Vec<FrameOverlay<Self::Message>> {
        vec![FrameOverlay::Menu(self.menu.clone())]
    }

    fn title(&self) -> &str {
        "Nickel Context Menu"
    }

    fn initial_size(&self) -> (u32, u32) {
        self.size
    }
}

struct PopupAdapter<Message> {
    sender: mpsc::Sender<Option<Choice<Message>>>,
    owner: HWND,
    position: POINT,
    focused: bool,
}

impl<Message: Clone + Send + 'static> HostAdapter<PopupApplication<Message>>
    for PopupAdapter<Message>
{
    fn window_attributes(
        &self,
        _application: &PopupApplication<Message>,
        attributes: WindowAttributes,
    ) -> WindowAttributes {
        attributes
            .with_decorations(false)
            .with_resizable(false)
            .with_window_level(WindowLevel::AlwaysOnTop)
            .with_position(PhysicalPosition::new(self.position.x, self.position.y))
    }

    fn started(
        &mut self,
        host: &mut UiHost<PopupApplication<Message>>,
        services: HostServices<'_>,
    ) -> Result<AdapterOutcome, Box<dyn std::error::Error>> {
        if let Ok(handle) = services.window().window_handle()
            && let RawWindowHandle::Win32(handle) = handle.as_raw()
        {
            let hwnd = HWND(handle.hwnd.get() as *mut _);
            // SAFETY: Both HWNDs refer to live windows for this modal popup.
            // The owner relationship keeps the popup out of Alt+Tab, and the
            // style update removes its taskbar button.
            unsafe {
                let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
                SetWindowLongPtrW(
                    hwnd,
                    GWL_EXSTYLE,
                    ((style | WS_EX_TOOLWINDOW.0) & !WS_EX_APPWINDOW.0) as isize,
                );
                SetWindowLongPtrW(hwnd, GWLP_HWNDPARENT, self.owner.0 as isize);
            }
        }
        host.open_transient(
            nickel_ui::OverlayId::new("windows-detached-context"),
            UiId::from("popup-anchor"),
        );
        services.window().focus_window();
        Ok(AdapterOutcome {
            changed: true,
            ..AdapterOutcome::default()
        })
    }

    fn event(
        &mut self,
        _host: &mut UiHost<PopupApplication<Message>>,
        event: &WindowEvent,
        _services: HostServices<'_>,
    ) -> Result<AdapterOutcome, Box<dyn std::error::Error>> {
        match event {
            WindowEvent::Focused(true) => self.focused = true,
            WindowEvent::Focused(false) if self.focused => {
                let _ = self.sender.send(None);
                return Ok(AdapterOutcome::exit());
            }
            _ => {}
        }
        Ok(AdapterOutcome::default())
    }

    fn poll_interval(&self) -> Option<Duration> {
        Some(Duration::from_millis(16))
    }

    fn poll(
        &mut self,
        host: &mut UiHost<PopupApplication<Message>>,
        _services: HostServices<'_>,
    ) -> Result<AdapterOutcome, Box<dyn std::error::Error>> {
        if let Some(choice) = host.application_mut().take_choice() {
            let _ = self.sender.send(Some(choice));
            return Ok(AdapterOutcome::exit());
        }
        if host.inspect().open_overlay.is_none() {
            let _ = self.sender.send(None);
            return Ok(AdapterOutcome::exit());
        }
        Ok(AdapterOutcome::default())
    }
}

/// Show Nickel's existing menu component in its own ephemeral window.
/// Nested entries open another tightly sized Nickel popup after selection.
pub fn start<Message: Clone + Send + 'static>(
    menu: OverlayMenu<Message>,
    window: &Window,
) -> Option<mpsc::Receiver<Option<Message>>> {
    let RawWindowHandle::Win32(handle) = window.window_handle().ok()?.as_raw() else {
        return None;
    };
    let owner = handle.hwnd.get() as usize;
    let scale = window.scale_factor();
    let mut position = POINT::default();
    // SAFETY: `position` is a valid output buffer for the current cursor.
    if unsafe { GetCursorPos(&mut position) }.is_err() {
        return None;
    }
    let (sender, receiver) = mpsc::channel();
    std::thread::Builder::new()
        .name("nickel-context-popup-owner".into())
        .spawn(move || {
            let action = run(menu, owner, scale, position);
            let _ = sender.send(action);
        })
        .ok()?;
    Some(receiver)
}

fn run<Message: Clone + Send + 'static>(
    mut menu: OverlayMenu<Message>,
    owner: usize,
    scale: f64,
    mut position: POINT,
) -> Option<Message> {
    loop {
        let application = PopupApplication::new(menu.clone());
        let width = application.size.0 as i32;
        let (sender, receiver) = mpsc::channel();
        let popup = std::thread::Builder::new()
            .name("nickel-context-popup".into())
            .spawn(move || {
                let adapter = PopupAdapter {
                    sender,
                    owner: HWND(owner as *mut _),
                    position,
                    focused: false,
                };
                nickel_ui::run_with_adapter_on_any_thread(application, adapter)
                    .map_err(|error| error.to_string())
            })
            .ok()?;
        let choice = receiver.recv().ok().flatten();
        if let Err(error) = popup.join().ok()? {
            tracing::warn!(%error, "Nickel context popup failed");
            return None;
        }
        match choice? {
            Choice::Action(action) => return Some(action),
            Choice::Submenu(children) => {
                position.x += ((width - 4) as f64 * scale).round() as i32;
                menu.items = children;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nickel_popup_maps_leaf_actions_and_nested_items() {
        let menu = OverlayMenu::new("source", OverlayAnchor::Node(UiId::from("source")))
            .item(OverlayMenuItem::action("open", "Open", 10))
            .item(OverlayMenuItem::submenu(
                "sort",
                "Sort by",
                [OverlayMenuItem::action("name", "Name", 20)],
            ));
        let mut popup = PopupApplication::new(menu);
        assert_eq!(popup.initial_size().0, popup.size.0);
        popup.update(0);
        assert!(matches!(popup.take_choice(), Some(Choice::Action(10))));
        popup.update(1);
        assert!(matches!(
            popup.take_choice(),
            Some(Choice::Submenu(children)) if children[0].action == Some(20)
        ));
    }
}
