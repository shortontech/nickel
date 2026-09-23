#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("window-inspect-hwnd supports Windows only");
}

#[cfg(target_os = "windows")]
fn main() {
    use std::ffi::c_void;
    use windows::Win32::{
        Foundation::{HWND, RECT},
        UI::{
            HiDpi::{GetAwarenessFromDpiAwarenessContext, GetWindowDpiAwarenessContext},
            WindowsAndMessaging::{
                GA_PARENT, GWL_EXSTYLE, GWL_STYLE, GetAncestor, GetClassNameW, GetForegroundWindow,
                GetParent, GetWindowLongPtrW, GetWindowRect, GetWindowThreadProcessId, IsWindow,
                IsWindowVisible,
            },
        },
    };

    let argument = std::env::args()
        .nth(1)
        .expect("usage: window-inspect-hwnd HEX_HWND");
    let value =
        usize::from_str_radix(argument.trim_start_matches("0x"), 16).expect("hexadecimal HWND");
    let hwnd = HWND(value as *mut c_void);
    let mut process_id = 0;
    let mut class = [0u16; 256];
    // SAFETY: All calls only inspect the supplied HWND; Windows validates it.
    let valid = unsafe { IsWindow(Some(hwnd)) }.as_bool();
    let thread_id = unsafe { GetWindowThreadProcessId(hwnd, Some(&mut process_id)) };
    let class_len = unsafe { GetClassNameW(hwnd, &mut class) }.max(0) as usize;
    let parent = unsafe { GetParent(hwnd) };
    let ancestor_parent = unsafe { GetAncestor(hwnd, GA_PARENT) };
    let style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) };
    let ex_style = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) };
    let dpi_context = unsafe { GetWindowDpiAwarenessContext(hwnd) };
    let dpi_awareness = unsafe { GetAwarenessFromDpiAwarenessContext(dpi_context) };
    let visible = unsafe { IsWindowVisible(hwnd) }.as_bool();
    let foreground = unsafe { GetForegroundWindow() } == hwnd;
    let mut rect = RECT::default();
    let rect_result = unsafe { GetWindowRect(hwnd, &mut rect) };
    println!(
        "hwnd={:?} valid={valid} visible={visible} foreground={foreground} pid={process_id} tid={thread_id} class={:?} parent={parent:?} ancestor_parent={:?} rect={rect:?} rect_result={rect_result:?} style={style:#x} ex_style={ex_style:#x} dpi_context={:?} dpi_awareness={:?}",
        hwnd.0,
        String::from_utf16_lossy(&class[..class_len]),
        ancestor_parent.0,
        dpi_context.0,
        dpi_awareness.0,
    );
}
