fn main() -> Result<(), Box<dyn std::error::Error>> {
    let app = twinkle_workbench::jsx_preview::JsxPreview::new()?;
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    match args.as_slice() {
        [] => twinkle::run(app),
        [command, path] if command == "render" => {
            let host = twinkle::UiHost::new(app, 720, 480);
            let raster = twinkle_testkit::render_host(&host, 720, 480, 1.0);
            image::RgbaImage::from_raw(raster.width, raster.height, raster.rgba)
                .ok_or("invalid preview raster")?
                .save(path)?;
            Ok(())
        }
        _ => Err("usage: twinkle-jsx-preview [render OUTPUT.png]".into()),
    }
}
