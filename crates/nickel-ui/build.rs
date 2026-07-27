#[cfg(target_os = "windows")]
fn main() {
    embed_icon("../../assets/icons/nickel-panel.png", "nickel-ui.ico");
}

#[cfg(not(target_os = "windows"))]
fn main() {}

#[cfg(target_os = "windows")]
fn embed_icon(source: &str, output_name: &str) {
    use std::{env, fs::File, path::PathBuf};

    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest directory"));
    let source = manifest.join(source);
    println!("cargo:rerun-if-changed={}", source.display());

    let original = image::open(&source)
        .unwrap_or_else(|error| panic!("load {}: {error}", source.display()))
        .into_rgba8();
    let output =
        PathBuf::from(env::var_os("OUT_DIR").expect("build output directory")).join(output_name);
    let mut directory = ico::IconDir::new(ico::ResourceType::Icon);
    for size in [16, 24, 32, 48, 64, 128, 256] {
        let resized =
            image::imageops::resize(&original, size, size, image::imageops::FilterType::Lanczos3);
        let image = ico::IconImage::from_rgba_data(size, size, resized.into_raw());
        directory.add_entry(ico::IconDirEntry::encode(&image).expect("encode icon frame"));
    }
    directory
        .write(File::create(&output).expect("create generated icon"))
        .expect("write generated icon");

    winresource::WindowsResource::new()
        .set_icon(output.to_str().expect("UTF-8 icon path"))
        .compile()
        .expect("embed Nickel Bar icon");
}
