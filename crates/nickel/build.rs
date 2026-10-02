#[cfg(target_os = "windows")]
fn main() {
    embed_package_catalog();
    nickel_build_support::embed_windows_icon("../../assets/icons/nickel-panel.png", "nickel.ico");
    reserve_windows_shell_stack();
}

#[cfg(not(target_os = "windows"))]
fn main() {
    embed_package_catalog();
    reserve_windows_shell_stack();
}

fn reserve_windows_shell_stack() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    // JSX evaluation can exceed the Windows PE default main-thread reserve.
    let argument = if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        "/STACK:8388608"
    } else {
        "-Wl,--stack,8388608"
    };
    println!("cargo:rustc-link-arg-bin=nickel={argument}");
}

// Embed checked-in modules verbatim. No compiler or package manager runs at build
// or runtime; authors regenerate JavaScript explicitly when editing JSX.
fn embed_package_catalog() {
    use std::{fs, path::Path};
    fn collect(root: &Path, directory: &Path, files: &mut Vec<(String, std::path::PathBuf)>) {
        let mut entries = fs::read_dir(directory)
            .expect("read bundled package directory")
            .map(|entry| entry.expect("read bundled package entry"))
            .collect::<Vec<_>>();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let kind = entry.file_type().expect("inspect bundled asset");
            assert!(!kind.is_symlink(), "bundled assets cannot be symlinks");
            if kind.is_dir() {
                collect(root, &entry.path(), files);
            } else if kind.is_file() {
                let path = entry.path();
                let relative = path
                    .strip_prefix(root)
                    .unwrap()
                    .components()
                    .map(|part| part.as_os_str().to_str().unwrap())
                    .collect::<Vec<_>>()
                    .join("/");
                files.push((relative, path));
            }
        }
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/plugins");
    println!("cargo:rerun-if-changed={}", root.display());
    let mut packages = fs::read_dir(&root)
        .unwrap()
        .map(|entry| entry.unwrap())
        .collect::<Vec<_>>();
    packages.sort_by_key(|entry| entry.file_name());
    let mut output = String::from(
        "type EmbeddedFiles = &'static [(&'static str, &'static [u8])];\nconst BUNDLED_PACKAGES: &[EmbeddedFiles] = &[\n",
    );
    for package in packages {
        if !package.path().join("plugin.json").is_file() {
            continue;
        }
        let mut files = Vec::new();
        collect(&package.path(), &package.path(), &mut files);
        output.push_str("&[\n");
        for (relative, path) in files {
            output.push_str(&format!("({relative:?}, include_bytes!({:?})),\n", path));
        }
        output.push_str("],\n");
    }
    output.push_str("];\n");
    let path =
        std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("bundled_packages.rs");
    fs::write(path, output).unwrap();
}
