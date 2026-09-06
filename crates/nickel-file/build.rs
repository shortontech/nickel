#[cfg(target_os = "windows")]
fn main() {
    nickel_build_support::embed_windows_icon(
        "../../assets/icons/nickel-file.png",
        "nickel-file.ico",
    );
}

#[cfg(not(target_os = "windows"))]
fn main() {}
