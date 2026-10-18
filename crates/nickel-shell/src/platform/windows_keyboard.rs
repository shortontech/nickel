//! Local on-screen keyboard delivery to the foreground Windows recipient.
//! The epoch is an opaque lease; neither the JSX package nor its pointer event
//! may choose a target HWND. A focus transition revokes the previous lease.

use std::sync::{LazyLock, Mutex};

use nickel_session_protocol::{OnScreenKeyboardInput, OnScreenKeyboardSnapshot, WindowId};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, IsWindow};

#[derive(Default)]
struct KeyboardLease {
    foreground: Option<isize>,
    epoch: u64,
}

impl KeyboardLease {
    fn observe(&mut self, foreground: Option<isize>) {
        if self.foreground != foreground || self.epoch == 0 {
            self.foreground = foreground;
            self.epoch = self.epoch.wrapping_add(1).max(1);
        }
    }

    fn admits(&self, epoch: u64, foreground: Option<isize>) -> bool {
        self.epoch == epoch && self.foreground.is_some() && self.foreground == foreground
    }
}

static LEASE: LazyLock<Mutex<KeyboardLease>> =
    LazyLock::new(|| Mutex::new(KeyboardLease::default()));

fn foreground_window() -> Option<isize> {
    // SAFETY: These calls only inspect the current foreground handle. IsWindow
    // validates the handle before it is admitted as a keyboard recipient.
    unsafe {
        let hwnd = GetForegroundWindow();
        (!hwnd.0.is_null() && IsWindow(Some(hwnd)).as_bool()).then_some(hwnd.0 as isize)
    }
}

pub fn on_screen_keyboard_snapshot() -> Result<OnScreenKeyboardSnapshot, String> {
    let foreground = foreground_window();
    let mut lease = LEASE.lock().map_err(|_| "keyboard lease poisoned")?;
    lease.observe(foreground);
    Ok(OnScreenKeyboardSnapshot {
        epoch: lease.epoch,
        recipient: foreground.map(|hwnd| WindowId(hwnd as u64)),
        ..Default::default()
    })
}

pub fn deliver_on_screen_keyboard_input(
    epoch: u64,
    input: OnScreenKeyboardInput,
) -> Result<(), String> {
    let foreground = foreground_window();
    let mut lease = LEASE.lock().map_err(|_| "keyboard lease poisoned")?;
    lease.observe(foreground);
    if !lease.admits(epoch, foreground) {
        return Err("keyboard recipient changed before delivery".into());
    }
    match input {
        OnScreenKeyboardInput::Text { text } => {
            if text.is_empty() || text.encode_utf16().count() > 16 {
                return Err("keyboard text exceeds its bound".into());
            }
            crate::windows_remote_input::send_text(&text)
        }
        OnScreenKeyboardInput::Key { keysym, modifiers } => {
            let keys = chord(keysym, &modifiers)?;
            crate::windows_remote_input::press_keys(&keys)?;
            crate::windows_remote_input::release_keys(&keys)
        }
    }
}

fn chord(keysym: u32, modifiers: &[u32]) -> Result<Vec<u8>, String> {
    if modifiers.len() > 5 {
        return Err("keyboard chord exceeds its bound".into());
    }
    let mut keys = Vec::with_capacity(modifiers.len() + 1);
    for modifier in modifiers {
        let key = match modifier {
            0xffe1 | 0xffe2 => 0x10, // Shift
            0xffe3 | 0xffe4 => 0x11, // Control
            0xffe9 | 0xffea => 0x12, // Alt
            0xffeb | 0xffec => 0x5b, // Super
            0xfe03 => 0xa5,          // AltGr / right Alt
            _ => return Err("unsupported keyboard modifier".into()),
        };
        if keys.contains(&key) {
            return Err("duplicate keyboard modifier".into());
        }
        keys.push(key);
    }
    let key = match keysym {
        0x61..=0x7a => (keysym - 0x20) as u8, // a-z
        0x41..=0x5a | 0x30..=0x39 => keysym as u8,
        0x20 => 0x20,
        0xff08 => 0x08,                                    // Backspace
        0xff09 => 0x09,                                    // Tab
        0xff0d => 0x0d,                                    // Enter
        0xff1b => 0x1b,                                    // Escape
        0xff50 => 0x24,                                    // Home
        0xff51 => 0x25,                                    // Left
        0xff52 => 0x26,                                    // Up
        0xff53 => 0x27,                                    // Right
        0xff54 => 0x28,                                    // Down
        0xff55 => 0x21,                                    // Page Up
        0xff56 => 0x22,                                    // Page Down
        0xff57 => 0x23,                                    // End
        0xff63 => 0x2d,                                    // Insert
        0xffff => 0x2e,                                    // Delete
        0xffbe..=0xffc9 => (0x70 + keysym - 0xffbe) as u8, // F1-F12
        _ => return Err("unsupported keyboard key".into()),
    };
    if keys.contains(&key) {
        return Err("keyboard target duplicates modifier".into());
    }
    keys.push(key);
    Ok(keys)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focus_transition_revokes_press_time_epoch() {
        let mut lease = KeyboardLease::default();
        lease.observe(Some(10));
        let first = lease.epoch;
        assert!(lease.admits(first, Some(10)));
        lease.observe(Some(11));
        assert!(!lease.admits(first, Some(10)));
        assert!(!lease.admits(first, Some(11)));
        assert!(lease.admits(lease.epoch, Some(11)));
        lease.observe(None);
        assert!(!lease.admits(lease.epoch, None));
    }

    #[test]
    fn keysyms_map_to_bounded_windows_chords() {
        assert_eq!(chord('a' as u32, &[0xffe3]).unwrap(), [0x11, 0x41]);
        assert_eq!(chord(0xff08, &[]).unwrap(), [0x08]);
        assert_eq!(chord(0xffbe, &[]).unwrap(), [0x70]);
        assert!(chord(0xffe3, &[0xffe3]).is_err());
        assert!(chord('a' as u32, &[0xffe3, 0xffe4]).is_err());
    }
}
