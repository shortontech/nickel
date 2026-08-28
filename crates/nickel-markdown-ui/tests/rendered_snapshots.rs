#![cfg(feature = "application")]

use std::{fs, path::Path};

use image::{ImageBuffer, Rgba};
use nickel_core::theme::{Appearance, ThemeMode};
use nickel_markdown_ui::{ViewerModel, ViewerPalette, load_document, viewer_view_with_palette};
use nickel_ui::{Rect, SdlComponentRenderer, UiEvent, UiStateStore, UiTree};
use tempfile::tempdir;

fn render_snapshot(
    model: &ViewerModel,
    palette: ViewerPalette,
    width: u32,
    height: u32,
    scale: f32,
    path: &Path,
) {
    let bounds = Rect::new(0.0, 0.0, width as f32 / scale, height as f32 / scale);
    let mut state = UiStateStore::default();
    let initial = UiTree::layout_with_state(
        viewer_view_with_palette(model, None, palette),
        bounds,
        &mut state,
    );
    initial.handle_event(&mut state, UiEvent::FocusNext);
    let tree = UiTree::layout_with_state(
        viewer_view_with_palette(model, None, palette),
        bounds,
        &mut state,
    );
    let reload_id = initial
        .id_for_message(&nickel_markdown_ui::ViewerMessage::Reload)
        .expect("viewer reload action should have a semantic identity")
        .clone();
    assert_eq!(state.focused(), Some(&reload_id));
    assert_eq!(
        tree.resolved_layout()
            .nodes()
            .iter()
            .filter(|node| node.interaction.focused)
            .count(),
        1,
        "FocusNext should focus exactly one viewer control"
    );
    let mut renderer = SdlComponentRenderer::new(width, height, scale);
    renderer.render(tree.commands());
    let pixels = renderer.pixels();
    assert_eq!(pixels.len(), (width * height) as usize);
    let nontransparent = pixels.iter().filter(|pixel| pixel.a > 0).count();
    assert!(nontransparent > pixels.len() / 3);
    let image = ImageBuffer::<Rgba<u8>, Vec<u8>>::from_fn(width, height, |x, y| {
        let pixel = pixels[(y * width + x) as usize];
        Rgba([pixel.r, pixel.g, pixel.b, pixel.a])
    });
    image.save(path).unwrap();
}

#[test]
fn representative_viewer_rasters_are_readable_artifacts() {
    let directory = tempdir().unwrap();
    let markdown = directory.path().join("guide.md");
    fs::write(
        &markdown,
        "# Nickel Markdown\n\nA **safe**, *selectable* document with `inline code` and a [typed link](https://example.com).\n\n> Quotes stay readable.\n\n1. Ordered item\n2. Another item\n\n```rust\nfn main() { println!(\"hello\"); }\n```",
    )
    .unwrap();
    let mut model = ViewerModel::default();
    let request = model.begin_open(&markdown);
    assert!(model.complete(load_document(&request)));

    let output =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/nickel-markdown-snapshots");
    fs::create_dir_all(&output).unwrap();
    for mode in [ThemeMode::Dark, ThemeMode::Light] {
        let mode_name = if mode == ThemeMode::Dark {
            "dark"
        } else {
            "light"
        };
        let palette = ViewerPalette::from_appearance(Appearance {
            mode,
            ..Appearance::default()
        });
        for (name, width, height, scale) in [
            ("narrow", 640, 480, 1.0),
            ("ordinary", 1024, 768, 1.0),
            ("maximized", 1920, 1080, 1.0),
            ("high-dpi", 2048, 1536, 2.0),
        ] {
            render_snapshot(
                &model,
                palette,
                width,
                height,
                scale,
                &output.join(format!("{mode_name}-{name}.png")),
            );
        }
    }
}
