#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("post-window-discovery supports Windows only");
}

#[cfg(target_os = "windows")]
fn main() -> windows::core::Result<()> {
    use windows::Win32::{
        Foundation::{LPARAM, WPARAM},
        UI::WindowsAndMessaging::{GetShellWindow, PostMessageW},
    };

    let mut arguments = std::env::args().skip(1);
    let wrapper = usize::from_str_radix(
        arguments
            .next()
            .expect("usage: post-window-discovery WRAPPER HWND")
            .trim_start_matches("0x"),
        16,
    )
    .expect("hexadecimal wrapper pointer");
    let hwnd = usize::from_str_radix(
        arguments
            .next()
            .expect("usage: post-window-discovery WRAPPER HWND")
            .trim_start_matches("0x"),
        16,
    )
    .expect("hexadecimal HWND");
    assert!(arguments.next().is_none(), "expected exactly two arguments");
    // SAFETY: The shell window belongs to the running diagnostic host.
    let shell = unsafe { GetShellWindow() };
    if shell.is_invalid() {
        return Err(windows::core::Error::from_hresult(windows::core::HRESULT(
            0x80070006_u32 as i32,
        )));
    }
    // SAFETY: The diagnostic host validates the HWND and function prologue
    // before invoking the build-specific method.
    unsafe { PostMessageW(Some(shell), 0x806d, WPARAM(wrapper), LPARAM(hwnd as isize))? };
    println!(
        "phase=post-window-discovery shell={:?} wrapper={wrapper:#x} hwnd={hwnd:#x}",
        shell.0
    );
    Ok(())
}
