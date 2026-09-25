//! Foreground requests must not join Nickel's input queue to an application's.
//!
//! With separate queues, Windows delivers foreign activation asynchronously.
//! Attaching queues makes activation wait for the other thread to process it.

use std::time::Instant;
use windows::Win32::{
    Foundation::HWND,
    System::Threading::GetCurrentThreadId,
    UI::{
        Input::KeyboardAndMouse::SetFocus,
        WindowsAndMessaging::{
            GetForegroundWindow, GetWindowThreadProcessId, SHOW_WINDOW_CMD, SW_HIDE, SW_RESTORE,
            SW_SHOWNOACTIVATE, SetForegroundWindow, ShowWindow, ShowWindowAsync,
        },
    },
};

pub(super) fn show_launcher(window: HWND) -> bool {
    // SAFETY: Windows validates the handle. Synchronous show/focus operations
    // below are permitted only for a window on the calling UI thread.
    if unsafe { GetWindowThreadProcessId(window, None) != GetCurrentThreadId() } {
        tracing::warn!("launcher focus requested from a thread that does not own its window");
        return false;
    }
    let started = Instant::now();
    tracing::debug!(window = ?window, "requesting launcher foreground focus");
    // SAFETY: This thread owns the launcher. Showing without activation avoids
    // an implicit activation before the explicit foreground request. Do not
    // attach input queues or wait/sleep for another application to deactivate.
    let (requested, focused) = unsafe {
        let _ = ShowWindow(window, SW_SHOWNOACTIVATE);
        let requested = SetForegroundWindow(window).as_bool();
        let focused = GetForegroundWindow() == window;
        if focused {
            let _ = SetFocus(Some(window));
        } else {
            // Preserve the contract: a rejected show must not project launcher
            // typing focus. Our own thread's foreground transition is immediate;
            // later focus changes are reconciled through normal window events.
            let _ = ShowWindow(window, SW_HIDE);
        }
        (requested, focused)
    };
    tracing::debug!(
        requested,
        focused,
        elapsed_ms = started.elapsed().as_millis(),
        "launcher foreground request completed"
    );
    if !focused {
        tracing::warn!(requested, "Windows did not grant launcher foreground focus");
    }
    focused
}

pub(super) fn activate_window(window: HWND, restore: bool) -> bool {
    let started = Instant::now();
    tracing::debug!(window = ?window, restore, "requesting application foreground focus");
    // SAFETY: The caller revalidates the enumerated HWND. ShowWindowAsync posts
    // restoration to the owner, so a hung app cannot block Nickel here. Keep
    // input queues separate for the subsequent foreground request as well.
    let requested = unsafe {
        if restore && !request_show_state(window, SW_RESTORE) {
            return false;
        }
        SetForegroundWindow(window).as_bool()
    };
    // Request admission is not proof of activation. The normal window feed
    // observes the actual foreground window after Windows processes the request.
    tracing::debug!(window = ?window, requested, elapsed_ms = started.elapsed().as_millis(),
        "application foreground request completed");
    requested
}

pub(super) fn request_show_state(window: HWND, command: SHOW_WINDOW_CMD) -> bool {
    // SAFETY: Windows validates the target HWND. Foreign show-state changes
    // must be posted, never synchronously sent to a potentially hung owner.
    let requested = unsafe { ShowWindowAsync(window, command) }.as_bool();
    if !requested {
        tracing::warn!(window = ?window, command = command.0,
            "Windows rejected asynchronous window show-state request");
    }
    requested
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::mpsc, thread, time::Duration};
    use windows::{
        Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, IsIconic, SW_MAXIMIZE, SW_MINIMIZE, WS_EX_NOACTIVATE,
            WS_EX_TOOLWINDOW, WS_MINIMIZE, WS_OVERLAPPEDWINDOW,
        },
        core::w,
    };

    #[test]
    fn unresponsive_window_does_not_block_activation_or_launcher_ownership_check() {
        let (ready_tx, ready_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let owner = thread::spawn(move || {
            // SAFETY: STATIC is a system class; this thread owns and destroys
            // its hidden fixture. NOACTIVATE keeps the test from taking focus.
            let window = unsafe {
                CreateWindowExW(
                    WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                    w!("STATIC"),
                    w!("Nickel blocked activation fixture"),
                    WS_OVERLAPPEDWINDOW | WS_MINIMIZE,
                    -32000,
                    -32000,
                    32,
                    32,
                    None,
                    None,
                    None,
                    None,
                )
            }
            .expect("fixture window");
            assert!(unsafe { IsIconic(window) }.as_bool());
            ready_tx.send(window.0 as usize).unwrap();
            // Deliberately do not pump the window's input queue. A synchronous
            // restore would wait here until the fixture is released.
            let _ = release_rx.recv_timeout(Duration::from_secs(5));
            let _ = unsafe { DestroyWindow(window) };
        });
        let address = ready_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("fixture ready");
        let (done_tx, done_rx) = mpsc::channel();
        let caller = thread::spawn(move || {
            let window = HWND(address as *mut _);
            let launcher_rejected = !show_launcher(window);
            let _ = activate_window(window, true);
            let _ = request_show_state(window, SW_MAXIMIZE);
            let _ = request_show_state(window, SW_MINIMIZE);
            let _ = done_tx.send(launcher_rejected);
        });
        let completed = done_rx.recv_timeout(Duration::from_millis(500));
        // Release the owner even on regression, so the test does not strand a
        // window or leave the blocked call running after its assertion fails.
        let _ = release_tx.send(());
        owner.join().unwrap();
        caller.join().unwrap();
        assert!(completed.expect("activation must not wait for the target input queue"));
    }
}
