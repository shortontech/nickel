#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("post-window-visible supports Windows only");
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
            .expect("usage: post-window-visible WRAPPER HWND")
            .trim_start_matches("0x"),
        16,
    )
    .expect("hexadecimal wrapper pointer");
    let hwnd = usize::from_str_radix(
        arguments
            .next()
            .expect("usage: post-window-visible WRAPPER HWND")
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
    // SAFETY: The host verifies the target HWND and function prologue.
    unsafe { PostMessageW(Some(shell), 0x806e, WPARAM(wrapper), LPARAM(hwnd as isize))? };
    println!(
        "phase=post-window-visible shell={:?} wrapper={wrapper:#x} hwnd={hwnd:#x}",
        shell.0
    );
    Ok(())
}
