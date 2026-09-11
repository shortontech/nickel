//! Bounded Windows input synthesis and lock-free emergency release tracking.
//!
//! The low-level hooks can call `release_all` without taking an application
//! mutex. Only virtual-key and pointer-button bits are retained; input payloads,
//! coordinates and timing are never stored here.

use std::{
    mem::size_of,
    sync::atomic::{AtomicU8, AtomicU64, Ordering},
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_KEYUP,
    KEYEVENTF_UNICODE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN,
    MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE,
    MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_VIRTUALDESK, MOUSEEVENTF_WHEEL,
    MOUSEINPUT, SendInput, VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GA_ROOT, GetAncestor, GetClientRect, GetForegroundWindow, GetSystemMetrics, SM_CXVIRTUALSCREEN,
    SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, WindowFromPoint,
};
use windows::Win32::{
    Foundation::{HWND, POINT, RECT},
    Graphics::Gdi::ClientToScreen,
};

const INPUT_MARKER: usize = 0x4e49_434b_454c_4d43;
static HELD_KEYS: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
const LEFT: u8 = 1;
const RIGHT: u8 = 2;
const MIDDLE: u8 = 4;
static HELD_BUTTONS: AtomicU8 = AtomicU8::new(0);

fn key_input(key: u8, released: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(u16::from(key)),
                dwFlags: if released {
                    KEYEVENTF_KEYUP
                } else {
                    Default::default()
                },
                dwExtraInfo: INPUT_MARKER,
                ..Default::default()
            },
        },
    }
}

fn button_bit(button: nickel_remote_control::pointer::PointerButton) -> u8 {
    match button {
        nickel_remote_control::pointer::PointerButton::Left => LEFT,
        nickel_remote_control::pointer::PointerButton::Right => RIGHT,
        nickel_remote_control::pointer::PointerButton::Middle => MIDDLE,
    }
}

fn button_input(button: nickel_remote_control::pointer::PointerButton, released: bool) -> INPUT {
    let dw_flags = match (button, released) {
        (nickel_remote_control::pointer::PointerButton::Left, false) => MOUSEEVENTF_LEFTDOWN,
        (nickel_remote_control::pointer::PointerButton::Left, true) => MOUSEEVENTF_LEFTUP,
        (nickel_remote_control::pointer::PointerButton::Right, false) => MOUSEEVENTF_RIGHTDOWN,
        (nickel_remote_control::pointer::PointerButton::Right, true) => MOUSEEVENTF_RIGHTUP,
        (nickel_remote_control::pointer::PointerButton::Middle, false) => MOUSEEVENTF_MIDDLEDOWN,
        (nickel_remote_control::pointer::PointerButton::Middle, true) => MOUSEEVENTF_MIDDLEUP,
    };
    mouse_input(0, 0, 0, dw_flags)
}

fn mouse_input(
    dx: i32,
    dy: i32,
    mouse_data: u32,
    dw_flags: windows::Win32::UI::Input::KeyboardAndMouse::MOUSE_EVENT_FLAGS,
) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: mouse_data,
                dwFlags: dw_flags,
                dwExtraInfo: INPUT_MARKER,
                ..Default::default()
            },
        },
    }
}

fn send(inputs: &[INPUT]) -> usize {
    if inputs.is_empty() {
        return 0;
    }
    // SAFETY: INPUT is initialized with the matching tagged union member, the
    // slice remains live for the call, and its bounded length fits u32.
    unsafe { SendInput(inputs, size_of::<INPUT>() as i32) as usize }
}

pub(crate) fn press_keys(keys: &[u8]) -> Result<(), String> {
    if keys.is_empty() || keys.len() > 6 {
        return Err("Windows key chord exceeds its bound".into());
    }
    let inputs: Vec<_> = keys.iter().map(|key| key_input(*key, false)).collect();
    let sent = send(&inputs);
    for key in keys.iter().take(sent) {
        let index = usize::from(*key) / 64;
        let bit = 1_u64 << (u32::from(*key) % 64);
        HELD_KEYS[index].fetch_or(bit, Ordering::AcqRel);
    }
    if sent != inputs.len() {
        release_all();
        return Err("Windows accepted only part of the key chord".into());
    }
    Ok(())
}

pub(crate) fn release_keys(keys: &[u8]) -> Result<(), String> {
    if keys.len() > 6 {
        return Err("Windows key chord exceeds its bound".into());
    }
    let inputs: Vec<_> = keys.iter().rev().map(|key| key_input(*key, true)).collect();
    let sent = send(&inputs);
    for key in keys.iter().rev().take(sent) {
        let index = usize::from(*key) / 64;
        let bit = 1_u64 << (u32::from(*key) % 64);
        HELD_KEYS[index].fetch_and(!bit, Ordering::AcqRel);
    }
    if sent != inputs.len() {
        return Err("Windows accepted only part of the key release".into());
    }
    Ok(())
}

pub(crate) fn send_text(text: &str) -> Result<(), String> {
    for unit in text.encode_utf16() {
        let inputs = [false, true].map(|released| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wScan: unit,
                    dwFlags: KEYEVENTF_UNICODE
                        | if released {
                            KEYEVENTF_KEYUP
                        } else {
                            Default::default()
                        },
                    dwExtraInfo: INPUT_MARKER,
                    ..Default::default()
                },
            },
        });
        let sent = send(&inputs);
        if sent != inputs.len() {
            if sent == 1 {
                let _ = send(&inputs[1..]);
            }
            return Err("Windows accepted only part of the text transaction".into());
        }
    }
    Ok(())
}

pub(crate) fn foreground_is(native: usize) -> bool {
    // SAFETY: GetForegroundWindow is a read-only query.
    unsafe { GetForegroundWindow().0 as usize == native }
}

fn absolute_axis(value: i32, origin: i32, length: i32) -> Result<i32, String> {
    if length <= 1 || value < origin || i64::from(value) >= i64::from(origin) + i64::from(length) {
        return Err("Windows pointer coordinate is outside the virtual desktop".into());
    }
    Ok(((i64::from(value - origin) * 65_535) / i64::from(length - 1)) as i32)
}

pub(crate) fn target_point(native: usize, x: i32, y: i32) -> Result<(i32, i32), String> {
    if x < 0 || y < 0 {
        return Err("Windows pointer coordinate is outside the client area".into());
    }
    let hwnd = HWND(native as *mut std::ffi::c_void);
    let mut rect = RECT::default();
    // SAFETY: hwnd came from the freshly revalidated resource owner and rect is
    // writable for the duration of the call.
    unsafe { GetClientRect(hwnd, &mut rect) }
        .map_err(|_| "Windows client geometry is unavailable")?;
    if x >= rect.right || y >= rect.bottom {
        return Err("Windows pointer coordinate is outside the client area".into());
    }
    let mut point = POINT { x, y };
    // SAFETY: same live hwnd and writable point as above.
    if !unsafe { ClientToScreen(hwnd, &mut point) }.as_bool() {
        return Err("Windows client coordinate conversion failed".into());
    }
    // Require the target to be the root window actually visible at the point;
    // never click through an occluding window into a different authority scope.
    // SAFETY: both calls are read-only native window queries.
    let hit = unsafe { GetAncestor(WindowFromPoint(point), GA_ROOT) };
    if hit != hwnd {
        return Err("Windows pointer target is obscured or changed".into());
    }
    Ok((point.x, point.y))
}

pub(crate) fn move_pointer(screen_x: i32, screen_y: i32) -> Result<(), String> {
    // SAFETY: GetSystemMetrics is a read-only query.
    let (left, top, width, height) = unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    };
    let x = absolute_axis(screen_x, left, width)?;
    let y = absolute_axis(screen_y, top, height)?;
    let input = mouse_input(
        x,
        y,
        0,
        MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
    );
    if send(&[input]) != 1 {
        return Err("Windows rejected pointer movement".into());
    }
    Ok(())
}

pub(crate) fn click(
    button: nickel_remote_control::pointer::PointerButton,
    count: usize,
) -> Result<(), String> {
    if !(1..=2).contains(&count) {
        return Err("Windows click count exceeds its bound".into());
    }
    for _ in 0..count {
        press_button(button)?;
        if let Err(error) = release_button(button) {
            release_all();
            return Err(error);
        }
    }
    Ok(())
}

pub(crate) fn press_button(
    button: nickel_remote_control::pointer::PointerButton,
) -> Result<(), String> {
    if send(&[button_input(button, false)]) != 1 {
        return Err("Windows rejected pointer button press".into());
    }
    HELD_BUTTONS.fetch_or(button_bit(button), Ordering::AcqRel);
    Ok(())
}

pub(crate) fn release_button(
    button: nickel_remote_control::pointer::PointerButton,
) -> Result<(), String> {
    if send(&[button_input(button, true)]) != 1 {
        return Err("Windows rejected pointer button release".into());
    }
    HELD_BUTTONS.fetch_and(!button_bit(button), Ordering::AcqRel);
    Ok(())
}

pub(crate) fn scroll(horizontal_v120: i32, vertical_v120: i32) -> Result<(), String> {
    let mut inputs = Vec::with_capacity(2);
    if vertical_v120 != 0 {
        inputs.push(mouse_input(0, 0, vertical_v120 as u32, MOUSEEVENTF_WHEEL));
    }
    if horizontal_v120 != 0 {
        inputs.push(mouse_input(
            0,
            0,
            horizontal_v120 as u32,
            MOUSEEVENTF_HWHEEL,
        ));
    }
    if send(&inputs) != inputs.len() {
        return Err("Windows accepted only part of the scroll transaction".into());
    }
    Ok(())
}

pub(crate) fn physical_input_idle() -> bool {
    (1_u16..=255).all(|key| {
        // SAFETY: this read-only query samples the current input desktop and
        // does not retain which key was down.
        unsafe { GetAsyncKeyState(i32::from(key)) >= 0 }
    })
}

/// Release every synthesized held edge. This is bounded to 256 events and may
/// run directly from a low-level physical-input hook. Failed releases remain
/// registered so the owner can retry them on its next poll.
pub(crate) fn release_all() {
    if HELD_KEYS
        .iter()
        .all(|word| word.load(Ordering::Acquire) == 0)
        && HELD_BUTTONS.load(Ordering::Acquire) == 0
    {
        return;
    }
    let mut releases = Vec::with_capacity(256);
    let mut identities = Vec::with_capacity(256);
    for (word_index, word) in HELD_KEYS.iter().enumerate() {
        let held = word.load(Ordering::Acquire);
        for bit in 0..64_u32 {
            if held & (1_u64 << bit) != 0 {
                let key = (word_index * 64 + bit as usize) as u8;
                releases.push(key_input(key, true));
                identities.push((word_index, 1_u64 << bit));
            }
        }
    }
    let sent = send(&releases);
    for (word, bit) in identities.into_iter().take(sent) {
        HELD_KEYS[word].fetch_and(!bit, Ordering::AcqRel);
    }
    let key_count = releases.len();
    let buttons = HELD_BUTTONS.load(Ordering::Acquire);
    let mut button_ids = Vec::with_capacity(3);
    for (button, bit) in [
        (nickel_remote_control::pointer::PointerButton::Left, LEFT),
        (nickel_remote_control::pointer::PointerButton::Right, RIGHT),
        (
            nickel_remote_control::pointer::PointerButton::Middle,
            MIDDLE,
        ),
    ] {
        if buttons & bit != 0 {
            releases.push(button_input(button, true));
            button_ids.push(bit);
        }
    }
    let sent_buttons = send(&releases[key_count..]);
    for bit in button_ids.into_iter().take(sent_buttons) {
        HELD_BUTTONS.fetch_and(!bit, Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests {
    use super::absolute_axis;

    #[test]
    fn absolute_pointer_axis_handles_negative_virtual_desktops_and_edges() {
        assert_eq!(absolute_axis(-1920, -1920, 3840).unwrap(), 0);
        assert_eq!(absolute_axis(1919, -1920, 3840).unwrap(), 65_535);
        assert!(absolute_axis(-1921, -1920, 3840).is_err());
        assert!(absolute_axis(1920, -1920, 3840).is_err());
        assert!(absolute_axis(0, 0, 1).is_err());
    }
}
