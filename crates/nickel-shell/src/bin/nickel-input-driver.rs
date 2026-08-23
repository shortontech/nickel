#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("nickel-input-driver is only available on Windows");
    std::process::exit(1);
}

#[cfg(target_os = "windows")]
fn main() {
    use std::mem::size_of;
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput, VIRTUAL_KEY,
    };

    const VK_LWIN: VIRTUAL_KEY = VIRTUAL_KEY(0x5b);
    const VK_R: VIRTUAL_KEY = VIRTUAL_KEY(0x52);
    const MARKER: usize = 0x4e49_434b_454c_5445;

    fn key(key: VIRTUAL_KEY, released: bool) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: key,
                    dwFlags: if released {
                        KEYEVENTF_KEYUP
                    } else {
                        Default::default()
                    },
                    dwExtraInfo: MARKER,
                    ..Default::default()
                },
            },
        }
    }

    // Include every release in the same atomic SendInput batch. The final plain R verifies that
    // Nickel did not leave Super latched after the shortcut.
    let input = [
        key(VK_LWIN, false),
        key(VK_R, false),
        key(VK_R, true),
        key(VK_LWIN, true),
        key(VK_R, false),
        key(VK_R, true),
    ];
    let sent = unsafe { SendInput(&input, size_of::<INPUT>() as i32) };
    if sent != input.len() as u32 {
        eprintln!(
            "FAIL: Windows accepted {sent} of {} input events",
            input.len()
        );
        std::process::exit(1);
    }
    println!("sent complete Super+R followed by plain R");
}
