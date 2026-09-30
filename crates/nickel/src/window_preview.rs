use crate::model::{OpenWindow, WindowId};
use nickel_core::theme::ThemePalette;
use nickel_ui::SemanticTheme;

pub const CARD_WIDTH: f32 = 276.0;
pub const TASK_SWITCHER_CARD_WIDTH: f32 = 220.0;
pub const PREVIEW_HEIGHT: f32 = 214.0;
const GAP: f32 = 10.0;
const PADDING: f32 = 12.0;

pub(crate) fn semantic_theme_from_palette(palette: ThemePalette) -> SemanticTheme {
    SemanticTheme::from_tokens(nickel_ui::SemanticTokenSet::standard(
        palette.background,
        palette.panel,
        palette.surface,
        palette.surface_hover,
        palette.surface_hover,
        palette.text,
        palette.muted,
        palette.accent,
        palette.accent_soft,
        palette.complement,
        palette.complement,
    ))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PreviewAction {
    Activate(WindowId),
    Close(WindowId),
    OpenMenu(WindowId),
}

pub(crate) fn same_window_identity(captured: &OpenWindow, current: &OpenWindow) -> bool {
    captured.id == current.id && captured.application_id == current.application_id
}

pub fn preview_dimensions(window_count: usize) -> (u32, u32) {
    let count = window_count.max(1) as f32;
    (
        (PADDING * 2.0 + count * CARD_WIDTH + (count - 1.0) * GAP).round() as u32,
        PREVIEW_HEIGHT.round() as u32,
    )
}

pub fn task_switcher_dimensions(window_count: usize) -> (u32, u32) {
    let count = window_count.clamp(1, 5) as f32;
    (
        (PADDING * 2.0 + count * TASK_SWITCHER_CARD_WIDTH + (count - 1.0) * GAP).round() as u32,
        PREVIEW_HEIGHT.round() as u32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ApplicationId;
    #[test]
    fn five_task_switcher_cards_fit_a_1280_pixel_output() {
        let (width, _) = task_switcher_dimensions(5);
        assert!(width <= 1_200);
    }
    #[test]
    fn native_window_identity_rejects_reused_ids_from_another_application() {
        let captured = OpenWindow {
            id: WindowId(9),
            application_id: Some(ApplicationId::new("org.nickel.Editor")),
            active: true,
            title: "Original".into(),
            state: Default::default(),
        };
        let mut current = captured.clone();
        current.title = "Renamed".into();
        assert!(same_window_identity(&captured, &current));
        current.application_id = Some(ApplicationId::new("org.nickel.Other"));
        assert!(!same_window_identity(&captured, &current));
        current = captured.clone();
        current.id = WindowId(10);
        assert!(!same_window_identity(&captured, &current));
    }
}
