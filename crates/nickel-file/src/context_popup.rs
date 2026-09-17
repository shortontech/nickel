//! Compositor-hosted presentation of Nickel File's shared context menu.

use nickel_ui::{
    Application, Container, FrameOverlay, HostedApplication, OverlayAnchor, OverlayMenu,
    OverlayMenuItem, Point, Shortcut, ShortcutOutcome, UiId, View, ViewContext,
};

use crate::app::{FileContextPopupSpec, FileMessage};

pub struct FileContextPopup {
    menu: OverlayMenu<usize>,
    actions: Vec<Option<FileMessage>>,
    chosen: Option<usize>,
    size: (u32, u32),
}

impl FileContextPopup {
    pub fn host(spec: FileContextPopupSpec) -> HostedApplication<Self> {
        let source = spec.menu.fit_width_to_content(320.0);
        let actions = source
            .items
            .iter()
            .map(|item| item.action.clone())
            .collect();
        let width = source.width.ceil() as u32 + 4;
        let height = (source.padding.top
            + source.row_height * source.items.len() as f32
            + source.row_gap * source.items.len().saturating_sub(1) as f32
            + source.padding.bottom)
            .ceil() as u32
            + 4;
        let mut menu = OverlayMenu::new(
            "file-detached-context",
            OverlayAnchor::Point {
                invocation_target: UiId::from("popup-anchor"),
                point: Point { x: 2.0, y: 2.0 },
            },
        );
        menu.width = source.width;
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
        for (index, item) in source.items.into_iter().enumerate() {
            let mut mapped = if item.action.is_some() {
                OverlayMenuItem::action(item.id, item.label, index)
            } else {
                OverlayMenuItem::disabled(item.id, item.label)
            };
            mapped.shortcut = item.shortcut;
            mapped.separator_before = item.separator_before;
            mapped.disabled_reason = item.disabled_reason;
            menu.items.push(mapped);
        }
        let mut host = HostedApplication::new(
            Self {
                menu,
                actions,
                chosen: None,
                size: (width, height),
            },
            width,
            height,
        );
        host.host_mut().open_transient(
            nickel_ui::OverlayId::new("file-detached-context"),
            UiId::from("popup-anchor"),
        );
        host
    }

    pub fn take_action(&mut self) -> Option<FileMessage> {
        self.chosen
            .take()
            .and_then(|index| self.actions[index].clone())
    }
}

impl Application for FileContextPopup {
    type Message = usize;

    fn update(&mut self, message: usize) {
        self.chosen = Some(message);
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

    fn shortcut_outcome(&mut self, _: Shortcut) -> ShortcutOutcome {
        ShortcutOutcome::from_changed(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detached_host_opens_shared_menu_with_compact_content_width() {
        let menu = OverlayMenu::new("source", OverlayAnchor::Node(UiId::from("source"))).item(
            OverlayMenuItem::action("rename", "Rename", FileMessage::ContextRename),
        );
        let mut host = FileContextPopup::host(FileContextPopupSpec {
            anchor: Point { x: 0.0, y: 0.0 },
            menu,
        });
        assert!(host.host().inspect().open_overlay.is_some());
        assert!(host.host().application().size.0 < 200);
        assert!(
            host.host()
                .semantic_nodes()
                .iter()
                .any(|node| { node.name.as_deref() == Some("Rename") })
        );
        host.host_mut().handle_event(nickel_ui::UiEvent::Dismiss);
        assert!(host.host().inspect().open_overlay.is_none());
    }
}
