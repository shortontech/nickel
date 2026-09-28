#[cfg(target_os = "windows")]
fn main() {
    nickel_build_support::embed_windows_icon("../../assets/icons/nickel-panel.png", "nickel.ico");
    reserve_windows_shell_stack();
}

#[cfg(not(target_os = "windows"))]
fn main() {
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
