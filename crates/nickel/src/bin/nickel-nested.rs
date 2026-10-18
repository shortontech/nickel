#[cfg(target_os = "linux")]
fn main() -> Result<(), String> {
    nickel_linux_session::session::run_nested().map_err(|error| error.to_string())
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("nickel-nested is available only on Linux");
}
