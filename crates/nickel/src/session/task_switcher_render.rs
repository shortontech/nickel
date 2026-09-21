use smithay::{
    backend::{allocator::Fourcc, renderer::element::memory::MemoryRenderBuffer},
    utils::{Physical, Size, Transform},
};

use crate::session::{NickelSession, window_registry::WindowId};

const MAX_CARDS: usize = 5;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BufferKey {
    pub(crate) candidates: Vec<WindowId>,
    pub(crate) selected: usize,
    pub(crate) output_size: (i32, i32),
    pub(crate) preview_generation: u64,
    pub(crate) theme_colors: [u32; 6],
}

pub(crate) struct BufferCache {
    pub(crate) key: BufferKey,
    pub(crate) buffer: MemoryRenderBuffer,
    pub(crate) size: Size<i32, Physical>,
}

pub(crate) fn key(state: &NickelSession, output_size: Size<i32, Physical>) -> BufferKey {
    BufferKey {
        candidates: state.task_switcher.candidates().to_vec(),
        selected: state.task_switcher.selected_index(),
        output_size: (output_size.w, output_size.h),
        preview_generation: state.preview_generation(),
        theme_colors: theme_colors(state),
    }
}

pub(crate) fn buffer(
    state: &NickelSession,
    output_size: Size<i32, Physical>,
) -> Option<(MemoryRenderBuffer, Size<i32, Physical>)> {
    let theme = theme(state);
    let candidates = state.task_switcher.candidates();
    let selected_index = state.task_switcher.selected_index();
    if candidates.len() < 2 {
        return None;
    }
    let range = visible_range(candidates.len(), selected_index);
    let count = range.len();
    let gap = 14_u32;
    let padding = 20_u32;
    let available_width = u32::try_from(output_size.w.saturating_sub(80))
        .unwrap_or_default()
        .min(1160);
    let card_width = ((available_width
        .saturating_sub(padding * 2 + gap * count.saturating_sub(1) as u32))
        / count as u32)
        .clamp(140, 220);
    let card_height = 204_u32;
    let label_height = 24_u32;
    let width = padding * 2 + card_width * count as u32 + gap * count.saturating_sub(1) as u32;
    let height = card_height + padding * 2;
    let size = (width as i32, height as i32);
    let buffer = draw_memory_render_buffer(width, height, |pixels| {
        let mut image =
            image::ImageBuffer::<image::Rgba<u8>, &mut [u8]>::from_raw(width, height, pixels)
                .expect("memory render buffer has the requested RGBA dimensions");
        image
            .pixels_mut()
            .for_each(|pixel| *pixel = rgba(theme.surfaces.sidebar, 244));

        for (slot, index) in range.enumerate() {
            let x = padding + slot as u32 * (card_width + gap);
            let selected = index == selected_index;
            let border = if selected {
                rgba(theme.accent.ordinary, 255)
            } else {
                rgba(theme.borders.ordinary, 255)
            };
            fill_rgba_rect(&mut image, x, padding, card_width, card_height, border);
            fill_rgba_rect(
                &mut image,
                x + 4,
                padding + 4,
                card_width - 8,
                card_height - 8,
                rgba(theme.surfaces.card, 255),
            );
            let id = candidates[index];
            if let Some(frame) = state.preview_frames.get(&id)
                && let Some(source) = image::ImageBuffer::<image::Rgba<u8>, &[u8]>::from_raw(
                    u32::from(frame.width),
                    u32::from(frame.height),
                    frame.rgba.as_slice(),
                )
            {
                draw_contained_preview(
                    &mut image,
                    &source,
                    (
                        x + 8,
                        padding + 8,
                        card_width - 16,
                        card_height - label_height - 20,
                    ),
                );
            }
            let title = state
                .windows
                .title(id)
                .filter(|title| !title.trim().is_empty())
                .or_else(|| {
                    state
                        .windows
                        .app_id(id)
                        .filter(|application| !application.trim().is_empty())
                })
                .unwrap_or("Untitled window");
            if let Some(label) = crate::session::window_frame::render_task_switcher_label(
                card_width - 16,
                label_height,
                title,
                theme.surfaces.card,
                theme.text.primary,
            ) && let Some(label) = image::ImageBuffer::<image::Rgba<u8>, &[u8]>::from_raw(
                card_width - 16,
                label_height,
                label.as_slice(),
            ) {
                image::imageops::overlay(
                    &mut image,
                    &label,
                    i64::from(x + 8),
                    i64::from(padding + card_height - label_height - 6),
                );
            }
        }
    });
    Some((buffer, size.into()))
}

pub(crate) fn visible_range(count: usize, selected: usize) -> std::ops::Range<usize> {
    let visible = count.min(MAX_CARDS);
    let start = selected
        .saturating_sub(visible / 2)
        .min(count.saturating_sub(visible));
    start..start + visible
}

pub(crate) fn contained_preview_bounds(
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    source_width: u32,
    source_height: u32,
) -> (u32, u32, u32, u32) {
    let bounds = nickel_ui::ImagePresentation::default().bounds(
        nickel_ui::Rect::new(x as f32, y as f32, width as f32, height as f32),
        nickel_ui::Size::new(source_width as f32, source_height as f32),
    );
    let fitted_width = bounds.size.width.round().clamp(0.0, width as f32) as u32;
    let fitted_height = bounds.size.height.round().clamp(0.0, height as f32) as u32;
    (
        x + (width - fitted_width) / 2,
        y + (height - fitted_height) / 2,
        fitted_width,
        fitted_height,
    )
}

pub(crate) fn draw_contained_preview<D, S>(
    destination: &mut D,
    source: &S,
    viewport: (u32, u32, u32, u32),
) where
    D: image::GenericImage<Pixel = image::Rgba<u8>>,
    S: image::GenericImageView<Pixel = image::Rgba<u8>>,
{
    let (x, y, width, height) = contained_preview_bounds(
        viewport.0,
        viewport.1,
        viewport.2,
        viewport.3,
        source.width(),
        source.height(),
    );
    if width == 0 || height == 0 {
        return;
    }
    let thumbnail =
        image::imageops::resize(source, width, height, image::imageops::FilterType::Triangle);
    image::imageops::overlay(destination, &thumbnail, i64::from(x), i64::from(y));
}

fn theme(state: &NickelSession) -> nickel_ui::SemanticTheme {
    state
        .internal_shell
        .as_ref()
        .map(crate::internal_shell::InternalShellCoordinator::semantic_theme)
        .unwrap_or_else(|| {
            crate::window_preview::semantic_theme_from_palette(
                nickel_core::theme::ThemePalette::from_appearance(Default::default()),
            )
        })
}

fn theme_colors(state: &NickelSession) -> [u32; 6] {
    let theme = theme(state);
    [
        theme.surfaces.sidebar,
        theme.surfaces.card,
        theme.surfaces.hover,
        theme.borders.ordinary,
        theme.accent.ordinary,
        theme.text.primary,
    ]
}

fn rgba(color: u32, alpha: u8) -> image::Rgba<u8> {
    image::Rgba([
        ((color >> 16) & 0xff) as u8,
        ((color >> 8) & 0xff) as u8,
        (color & 0xff) as u8,
        alpha,
    ])
}

pub(crate) fn draw_memory_render_buffer(
    width: u32,
    height: u32,
    draw: impl FnOnce(&mut [u8]),
) -> MemoryRenderBuffer {
    let size = (width as i32, height as i32);
    let mut buffer = MemoryRenderBuffer::new(Fourcc::Abgr8888, size, 1, Transform::Normal, None);
    buffer
        .render()
        .draw(|pixels| {
            draw(pixels);
            Ok::<_, std::convert::Infallible>(vec![smithay::utils::Rectangle::from_size(
                size.into(),
            )])
        })
        .expect("infallible task switcher drawing");
    buffer
}

fn fill_rgba_rect<I>(image: &mut I, x: u32, y: u32, width: u32, height: u32, color: image::Rgba<u8>)
where
    I: image::GenericImage<Pixel = image::Rgba<u8>>,
{
    for row in y..y.saturating_add(height).min(image.height()) {
        for column in x..x.saturating_add(width).min(image.width()) {
            image.put_pixel(column, row, color);
        }
    }
}
