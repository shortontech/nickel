//! Foreground requests must not join Nickel's input queue to an application's.
//!
//! With separate queues, Windows delivers foreign activation asynchronously.
//! Attaching queues makes activation wait for the other thread to process it.

use std::time::Instant;
use windows::Win32::{
    Foundation::HWND,
    System::Threading::GetCurrentThreadId,
    UI::{
        Input::KeyboardAndMouse::{
            GetAsyncKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
            SendInput, SetFocus, VK_MENU,
        },
        WindowsAndMessaging::{
            GWL_EXSTYLE, GetForegroundWindow, GetWindowLongPtrW, GetWindowThreadProcessId,
            HWND_TOP, IsWindow, SHOW_WINDOW_CMD, SW_HIDE, SW_RESTORE, SW_SHOWNOACTIVATE,
            SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetForegroundWindow,
            SetWindowPos, ShowWindow, ShowWindowAsync, WS_EX_NOACTIVATE,
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
        let requested = request_foreground(window);
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
    let requested = {
        if restore && !request_show_state(window, SW_RESTORE) {
            return false;
        }
        let requested = request_foreground(window);
        let raised = requested && raise_window(window);
        tracing::debug!(window = ?window, requested, raised, "application activation and raise dispatched");
        requested && raised
    };
    // Request admission is not proof of activation. The normal window feed
    // observes the actual foreground window after Windows processes the request.
    tracing::debug!(window = ?window, requested, elapsed_ms = started.elapsed().as_millis(),
        "application foreground request completed");
    requested
}

fn request_foreground(window: HWND) -> bool {
    // SAFETY: All queries validate the HWND; passive shell surfaces must never
    // be activated, including through the recovery path.
    unsafe {
        if !IsWindow(Some(window)).as_bool()
            || GetWindowLongPtrW(window, GWL_EXSTYLE) as u32 & WS_EX_NOACTIVATE.0 != 0
        {
            return false;
        }
        if SetForegroundWindow(window).as_bool() {
            return true;
        }
        // A hook-owned shortcut does not grant foreground permission. Windows
        // unlocks foreground changes on Alt input. Only recover an explicit
        // user activation, and never release or chord a physically held modifier.
        if [0x10, 0x11, 0x12, 0x5b, 0x5c]
            .iter()
            .any(|key| GetAsyncKeyState(*key) < 0)
        {
            return false;
        }
        let mut input = [INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VK_MENU,
                    ..Default::default()
                },
            },
        }; 2];
        input[1].Anonymous.ki.dwFlags = KEYEVENTF_KEYUP;
        // Send both edges as one batch so recovery never deliberately leaves
        // Alt down. The shortcut adapter ignores injected keyboard events.
        let sent = SendInput(&input, std::mem::size_of::<INPUT>() as i32);
        if sent != 2 {
            if sent == 1 {
                let _ = SendInput(&input[1..], std::mem::size_of::<INPUT>() as i32);
            }
            tracing::warn!(sent, "foreground recovery input rejected");
            return false;
        }
        let requested = SetForegroundWindow(window).as_bool();
        tracing::debug!(window = ?window, requested, "foreground recovery completed");
        requested
    }
}

fn raise_window(window: HWND) -> bool {
    // SAFETY: Raise the validated app in its existing Z-order band without
    // resizing, moving, activating synchronously, or making it topmost.
    unsafe {
        SetWindowPos(
            window,
            Some(HWND_TOP),
            0,
            0,
            0,
            0,
            SWP_ASYNCWINDOWPOS | SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE,
        )
        .is_ok()
    }
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
    #[ignore = "live foreground test: set NICKEL_FOCUS_TEST_HWND to a decimal window handle"]
    fn explicitly_selected_live_window_receives_foreground() {
        let address: usize = std::env::var("NICKEL_FOCUS_TEST_HWND")
            .expect("explicit target HWND")
            .parse()
            .expect("decimal HWND");
        let window = HWND(address as *mut _);
        // SAFETY: Read-only diagnostic of the current foreground HWND.
        eprintln!("foreground before activation: {:?}", unsafe {
            GetForegroundWindow()
        });
        let requested = activate_window(window, false);
        let deadline = Instant::now() + Duration::from_secs(2);
        while unsafe { GetForegroundWindow() } != window && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(requested, "native switch request rejected");
        assert_eq!(unsafe { GetForegroundWindow() }, window);
        if let Ok(other) = std::env::var("NICKEL_FOCUS_TEST_OTHER_HWND") {
            use windows::Win32::UI::WindowsAndMessaging::{GW_HWNDNEXT, GetWindow};
            let other = HWND(other.parse::<usize>().expect("decimal comparison HWND") as *mut _);
            let mut below = false;
            while !below && Instant::now() < deadline {
                let mut cursor = window;
                for _ in 0..1000 {
                    // SAFETY: Read-only traversal of the live native Z-order.
                    let Ok(next) = (unsafe { GetWindow(cursor, GW_HWNDNEXT) }) else {
                        break;
                    };
                    if next == other {
                        below = true;
                        break;
                    }
                    cursor = next;
                }
                if !below {
                    thread::sleep(Duration::from_millis(10));
                }
            }
            assert!(below, "selected app must be above the comparison window");
        }
    }

    #[test]
    #[ignore = "requires NICKEL_WINDOWS_NATIVE_FOCUS_TEST=1; briefly changes foreground focus"]
    fn native_focus_fixture_restores_previous_foreground() {
        use windows::Win32::UI::WindowsAndMessaging::{IsWindow, SW_SHOWNOACTIVATE};

        assert_eq!(
            std::env::var("NICKEL_WINDOWS_NATIVE_FOCUS_TEST").as_deref(),
            Ok("1")
        );
        struct Restore {
            previous: HWND,
            fixture: HWND,
        }
        impl Drop for Restore {
            fn drop(&mut self) {
                // SAFETY: Both HWND values were observed or created by this test.
                if unsafe { IsWindow(Some(self.previous)) }.as_bool() {
                    let _ = activate_window(self.previous, false);
                }
                let _ = unsafe { DestroyWindow(self.fixture) };
            }
        }

        // SAFETY: STATIC is a system class. The offscreen fixture is initially
        // hidden and is destroyed by Restore even when an assertion fails.
        let previous = unsafe { GetForegroundWindow() };
        let fixture = unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW,
                w!("STATIC"),
                w!("Nickel native focus fixture"),
                WS_OVERLAPPEDWINDOW,
                -32000,
                -32000,
                160,
                100,
                None,
                None,
                None,
                None,
            )
        }
        .expect("native focus fixture window");
        let restore = Restore { previous, fixture };
        let _ = unsafe { ShowWindow(fixture, SW_SHOWNOACTIVATE) };
        assert_eq!(unsafe { GetForegroundWindow() }, previous);
        assert!(
            activate_window(fixture, false),
            "native focus request rejected"
        );
        let deadline = Instant::now() + Duration::from_secs(2);
        while unsafe { GetForegroundWindow() } != fixture && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(unsafe { GetForegroundWindow() }, fixture);
        drop(restore);
        if unsafe { IsWindow(Some(previous)) }.as_bool() {
            let deadline = Instant::now() + Duration::from_secs(2);
            while unsafe { GetForegroundWindow() } != previous && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(10));
            }
            assert_eq!(unsafe { GetForegroundWindow() }, previous);
        }
    }

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
            let _ = raise_window(window);
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
