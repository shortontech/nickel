//! Bounded Windows input synthesis and lock-free emergency release tracking.
//!
//! The low-level hooks can call `release_all` without taking an application
//! mutex. Only virtual-key bits are retained; input payloads, coordinates and
//! timing are never stored here.

use std::{
    mem::size_of,
    sync::atomic::{AtomicU64, Ordering},
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
    KEYEVENTF_UNICODE, SendInput, VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

const INPUT_MARKER: usize = 0x4e49_434b_454c_4d43;
static HELD_KEYS: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];

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
}
