#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("nickel-input-harness is only available on Windows");
    std::process::exit(1);
}

#[cfg(target_os = "windows")]
mod windows_harness {
    use std::sync::{Mutex, OnceLock};

    use nickel_core::hotkeys::{HotkeyAction, default_bindings};
    use nickel_input::{
        KeyEdge,
        windows::{InjectedEventPolicy, NativeKeyboardEvent, WindowsInputAdapter},
    };
    use windows::Win32::{
        Foundation::{LPARAM, LRESULT, WPARAM},
        UI::WindowsAndMessaging::{
            CallNextHookEx, DispatchMessageW, GetMessageW, KBDLLHOOKSTRUCT, MSG, PostQuitMessage,
            SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx, WH_KEYBOARD_LL, WM_KEYDOWN,
            WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
        },
    };

    const MARKER: usize = 0x4e49_434b_454c_5445;
    const EXPECTED_EVENTS: usize = 6;

    struct State {
        adapter: WindowsInputAdapter<HotkeyAction>,
        seen: usize,
        failures: Vec<String>,
    }

    impl Default for State {
        fn default() -> Self {
            Self {
                adapter: WindowsInputAdapter::new(default_bindings())
                    .with_injected_policy(InjectedEventPolicy::Accept),
                seen: 0,
                failures: Vec::new(),
            }
        }
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
        if let Ok(mut state) = STATE.get_or_init(Default::default).lock() {
            let dispatch = state
                .adapter
                .handle_native(NativeKeyboardEvent {
                    virtual_key: event.vkCode,
                    scan_code: event.scanCode,
                    extended: event.flags.0 & 1 != 0,
                    edge,
                    injected: event.flags.0 & 0x10 != 0,
                })
                .expect("the acceptance harness explicitly permits its marked injected events");
            let key = dispatch.normalized.physical;
            let outcome = dispatch.outcomes.first();
            let index = state.seen;
            let expected = [
                (
                    nickel_input::PhysicalKey::Code(nickel_input::KeyCode::SuperLeft),
                    KeyEdge::Pressed,
                    None,
                    false,
                ),
                (
                    nickel_input::PhysicalKey::Code(nickel_input::KeyCode::KeyR),
                    KeyEdge::Pressed,
                    Some(HotkeyAction::ShowRun),
                    true,
                ),
                (
                    nickel_input::PhysicalKey::Code(nickel_input::KeyCode::KeyR),
                    KeyEdge::Released,
                    None,
                    false,
                ),
                (
                    nickel_input::PhysicalKey::Code(nickel_input::KeyCode::SuperLeft),
                    KeyEdge::Released,
                    None,
                    false,
                ),
                (
                    nickel_input::PhysicalKey::Code(nickel_input::KeyCode::KeyR),
                    KeyEdge::Pressed,
                    None,
                    false,
                ),
                (
                    nickel_input::PhysicalKey::Code(nickel_input::KeyCode::KeyR),
                    KeyEdge::Released,
                    None,
                    false,
                ),
            ][index.min(EXPECTED_EVENTS - 1)]
            .clone();
            let action = outcome.map(|outcome| outcome.action);
            let suppress = outcome.is_some_and(|outcome| outcome.suppress);
            if (key.clone(), edge, action, suppress) != expected {
                state.failures.push(format!(
                    "event {}: got ({key:?}, {edge:?}, {:?}, suppress={}), expected {expected:?}",
                    index + 1,
                    action,
                    suppress
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
