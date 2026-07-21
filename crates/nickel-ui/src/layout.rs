pub const RESULT_LEFT: f32 = 40.0;
pub const RESULT_RIGHT_INSET: f32 = 40.0;
pub const RESULT_TOP: f32 = 132.0;
pub const RESULT_HEIGHT: f32 = 48.0;
pub const RESULT_STRIDE: f32 = 52.0;
pub const RESULT_TEXT_LEFT: f32 = 108.0;
pub const RESULT_TEXT_TOP: f32 = RESULT_TOP - 3.0;
pub const ICON_LEFT: f32 = 56.0;
pub const ICON_SIZE: f32 = 36.0;

pub const fn icon_top_offset() -> f32 {
    (RESULT_HEIGHT - ICON_SIZE) / 2.0
}
