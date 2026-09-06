//! Render the production keyboard component without opening a native window.
//! This is visual component evidence, not native delivery acceptance.
use nickel_core::theme::ThemePalette;
use nickel_ui::{
    Application, SoftwareRenderer, UiHost,
    on_screen_keyboard::{KeyboardApp, KeyboardMessage},
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args()
        .nth(1)
        .ok_or("usage: keyboard_layout OUTPUT.png")?;
    let mut app = KeyboardApp::new(ThemePalette::from_appearance(Default::default()));
    app.recipient_changed(true);
    let arguments = std::env::args().collect::<Vec<_>>();
    let compact = arguments.iter().any(|argument| argument == "--compact");
    let (width, height) = if compact { (640, 360) } else { (1280, 420) };
    if arguments.iter().any(|argument| argument == "--navigation") {
        app.update(KeyboardMessage::Key(
            nickel_core::on_screen_keyboard::KeyboardKey::Panel(
                nickel_core::on_screen_keyboard::KeyboardPanel::Navigation,
            ),
        ));
    }
    if arguments.iter().any(|argument| argument == "--symbols") {
        app.update(KeyboardMessage::Key(
            nickel_core::on_screen_keyboard::KeyboardKey::Panel(
                nickel_core::on_screen_keyboard::KeyboardPanel::Symbols,
            ),
        ));
    }
    let host = UiHost::new(app, width, height);
    if std::env::args().any(|arg| arg == "--inspect") {
        for command in host.commands() {
            println!("{command:?}");
        }
    }
    let mut renderer = SoftwareRenderer::new_pixel_buffer(width, height, 1.0);
    host.render_software(&mut renderer);
    let mut image = image::RgbaImage::new(width, height);
    for (output, input) in image.pixels_mut().zip(renderer.pixels()) {
        *output = image::Rgba([input.r, input.g, input.b, input.a]);
    }
    image.save(output)?;
    Ok(())
}
