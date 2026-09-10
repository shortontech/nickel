//! The existing compositor output-number raster, shared by native and nested backends.
use smithay::{
    backend::{allocator::Fourcc, renderer::element::memory::MemoryRenderBuffer},
    utils::Transform,
};

pub(crate) fn identify_badge(number: usize) -> MemoryRenderBuffer {
    const SIZE: usize = 180;
    const THICKNESS: usize = 18;
    let mut rgba = vec![0_u8; SIZE * SIZE * 4];
    for pixel in rgba.chunks_exact_mut(4) {
        pixel.copy_from_slice(&[38, 45, 59, 238]);
    }
    let segments = match number {
        1 => [false, true, true, false, false, false, false],
        2 => [true, true, false, true, true, false, true],
        3 => [true, true, true, true, false, false, true],
        4 => [false, true, true, false, false, true, true],
        5 => [true, false, true, true, false, true, true],
        6 => [true, false, true, true, true, true, true],
        7 => [true, true, true, false, false, false, false],
        8 => [true; 7],
        _ => [true, true, true, true, false, true, true],
    };
    let rectangles = [
        (55, 25, 70, THICKNESS),
        (120, 35, THICKNESS, 55),
        (120, 90, THICKNESS, 55),
        (55, 137, 70, THICKNESS),
        (42, 90, THICKNESS, 55),
        (42, 35, THICKNESS, 55),
        (55, 81, 70, THICKNESS),
    ];
    for ((x, y, width, height), enabled) in rectangles.into_iter().zip(segments) {
        if !enabled {
            continue;
        }
        for row in y..y + height {
            for column in x..x + width {
                let index = (row * SIZE + column) * 4;
                rgba[index..index + 4].copy_from_slice(&[245, 247, 252, 255]);
            }
        }
    }
    MemoryRenderBuffer::from_slice(
        &rgba,
        Fourcc::Abgr8888,
        (SIZE as i32, SIZE as i32),
        1,
        Transform::Normal,
        None,
    )
}
