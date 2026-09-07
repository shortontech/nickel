#[cfg(target_os = "linux")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    nickel_shell::session::run()
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("nickel-session is only available on Linux");
    std::process::exit(1);
}
