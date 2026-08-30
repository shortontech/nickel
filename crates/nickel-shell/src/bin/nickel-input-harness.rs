#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("nickel-input-harness is only available on Windows");
    std::process::exit(1);
}

#[cfg(target_os = "windows")]
mod windows_harness {
    use std::sync::{Mutex, OnceLock};

    use nickel_core::hotkeys::{KeyCode, HotkeyAction, HotkeyController, KeyEdge};
    use windows::Win32::{
        Foundation::{LPARAM, LRESULT, WPARAM},
        UI::WindowsAndMessaging::{
            CallNextHookEx, DispatchMessageW, GetMessageW, KBDLLHOOKSTRUCT, MSG, PostQuitMessage,
            SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx, WH_KEYBOARD_LL, WM_KEYDOWN,
            WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
        },
    };

    const VK_LWIN: u32 = 0x5b;
    const VK_RWIN: u32 = 0x5c;
    const VK_R: u32 = 0x52;
    const MARKER: usize = 0x4e49_434b_454c_5445;
    const EXPECTED_EVENTS: usize = 6;

    #[derive(Default)]
    struct State {
        controller: HotkeyController,
        seen: usize,
        failures: Vec<String>,
    }

    static STATE: OnceLock<Mutex<State>> = OnceLock::new();

    unsafe extern "system" fn hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code < 0 {
            return unsafe { CallNextHookEx(None, code, wparam, lparam) };
        }
        let event = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        if event.dwExtraInfo != MARKER {
            return unsafe { CallNextHookEx(None, code, wparam, lparam) };
        }
        let message = wparam.0 as u32;
        let edge = if message == WM_KEYDOWN || message == WM_SYSKEYDOWN {
            KeyEdge::Pressed
        } else if message == WM_KEYUP || message == WM_SYSKEYUP {
            KeyEdge::Released
        } else {
            return LRESULT(1);
        };
        let key = match event.vkCode {
            VK_LWIN | VK_RWIN => KeyCode::SuperLeft,
            VK_R => KeyCode::KeyR,
            _ => KeyCode::KeyA,
        };

        if let Ok(mut state) = STATE.get_or_init(Default::default).lock() {
            let outcome = state.controller.handle(key, edge);
            let index = state.seen;
            let expected = [
                (KeyCode::SuperLeft, KeyEdge::Pressed, None, true),
                (
                    KeyCode::KeyR,
                    KeyEdge::Pressed,
                    Some(HotkeyAction::ShowRun),
                    true,
                ),
                (KeyCode::KeyR, KeyEdge::Released, None, true),
                (KeyCode::SuperLeft, KeyEdge::Released, None, true),
                (KeyCode::KeyR, KeyEdge::Pressed, None, false),
                (KeyCode::KeyR, KeyEdge::Released, None, false),
            ][index.min(EXPECTED_EVENTS - 1)];
            if (key, edge, outcome.action, outcome.suppress) != expected {
                state.failures.push(format!(
                    "event {}: got ({key:?}, {edge:?}, {:?}, suppress={}), expected {expected:?}",
                    index + 1,
                    outcome.action,
                    outcome.suppress
                ));
            }
            state.seen += 1;
            if state.seen >= EXPECTED_EVENTS {
                unsafe { PostQuitMessage(0) };
            }
        }

        // The harness owns marked test input. Never leak it into the user's focused application.
        LRESULT(1)
    }

    pub fn run() -> i32 {
        let hook_handle = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook), None, 0) }
            .unwrap_or_else(|error| {
                eprintln!("FAIL: could not install keyboard test hook: {error}");
                std::process::exit(1);
            });
        println!("READY");

        let mut message = MSG::default();
        while unsafe { GetMessageW(&mut message, None, 0, 0) }.as_bool() {
            unsafe {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        let _ = unsafe { UnhookWindowsHookEx(hook_handle) };

        let state = STATE.get_or_init(Default::default).lock().unwrap();
        for failure in &state.failures {
            eprintln!("FAIL: {failure}");
        }
        if state.seen == EXPECTED_EVENTS && state.failures.is_empty() {
            println!("PASS: shortcut releases cleanly; subsequent R is not captured");
            0
        } else {
            eprintln!(
                "FAIL: observed {} of {EXPECTED_EVENTS} expected events",
                state.seen
            );
            1
        }
    }
}

#[cfg(target_os = "windows")]
fn main() {
    std::process::exit(windows_harness::run());
}
