#[cfg(target_os = "windows")]
#[path = "../wrapper_inspect.rs"]
mod wrapper_inspect;

#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("find-window-wrapper supports Windows only");
}

#[cfg(target_os = "windows")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let frame = usize::from_str_radix(
        std::env::args()
            .nth(1)
            .ok_or("usage: find-window-wrapper FRAME_HWND_HEX")?
            .trim_start_matches("0x"),
        16,
    )?;
    let found = wrapper_inspect::find_wrapper(frame)?;
    println!(
        "interface={:#x} client={:#x} wait_flags={:#x}",
        found.interface, found.client, found.wait_flags
    );
    Ok(())
}
