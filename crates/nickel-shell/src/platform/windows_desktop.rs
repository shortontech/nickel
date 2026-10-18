//! Keep wallpaper windows behind applications through native activation changes.

use windows::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    UI::{
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::{
            HWND_BOTTOM, MA_NOACTIVATE, SWP_NOACTIVATE, SWP_NOZORDER, WINDOWPOS, WM_MOUSEACTIVATE,
            WM_NCDESTROY, WM_WINDOWPOSCHANGING,
        },
    },
};

const DESKTOP_SUBCLASS: usize = 0x4e444553;

pub(super) fn install(window: HWND) -> bool {
    // SAFETY: The caller owns this desktop HWND on the current UI thread.
    // The fixed subclass ID makes repeated display configuration idempotent.
    unsafe { SetWindowSubclass(window, Some(desktop_proc), DESKTOP_SUBCLASS, 0) }.as_bool()
}

unsafe extern "system" fn desktop_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    id: usize,
    _: usize,
) -> LRESULT {
    if message == WM_MOUSEACTIVATE {
        // Still deliver the click for selection and context menus; never let
        // native mouse activation raise the full-screen wallpaper.
        return LRESULT(MA_NOACTIVATE as isize);
    }
    if message == WM_NCDESTROY {
        // SAFETY: This callback belongs to the HWND being destroyed.
        let _ = unsafe { RemoveWindowSubclass(window, Some(desktop_proc), id) };
    }
    // SAFETY: Preserve winit's normal message handling and subclass chain.
    let result = unsafe { DefSubclassProc(window, message, wparam, lparam) };
    if message == WM_WINDOWPOSCHANGING && lparam.0 != 0 {
        // SAFETY: Windows supplies a writable WINDOWPOS for this synchronous
        // message. Apply the desktop rule after downstream geometry handling.
        let position = unsafe { &mut *(lparam.0 as *mut WINDOWPOS) };
        position.hwndInsertAfter = HWND_BOTTOM;
        position.flags = (position.flags | SWP_NOACTIVATE) & !SWP_NOZORDER;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::{
        Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, GWL_EXSTYLE, GetWindowLongPtrW, HWND_TOPMOST,
            SWP_NOMOVE, SWP_NOSIZE, SendMessageW, SetWindowPos, WINDOW_EX_STYLE, WS_EX_TOPMOST,
            WS_POPUP,
        },
        core::w,
    };

    #[test]
    fn desktop_rejects_mouse_activation_and_topmost_promotion() {
        // SAFETY: This test owns a hidden system-class window on this thread.
        let window = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!("Nickel desktop fixture"),
                WS_POPUP,
                0,
                0,
                32,
                32,
                None,
                None,
                None,
                None,
            )
        }
        .unwrap();
        assert!(install(window));
        assert!(install(window));
        let activation =
            unsafe { SendMessageW(window, WM_MOUSEACTIVATE, Some(WPARAM(0)), Some(LPARAM(0))) };
        let raised = unsafe {
            SetWindowPos(
                window,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            )
        };
        let style = unsafe { GetWindowLongPtrW(window, GWL_EXSTYLE) } as u32;
        // Clean up before assertions, including subclass removal on NCDESTROY.
        unsafe { DestroyWindow(window) }.unwrap();
        assert_eq!(activation.0, MA_NOACTIVATE as isize);
        assert!(raised.is_ok());
        assert_eq!(style & WS_EX_TOPMOST.0, 0);
    }
}
