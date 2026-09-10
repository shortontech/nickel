//! Atomic Windows emergency-chord recognition. Native hook attribution is the
//! caller's responsibility; injected edges never participate in this state.
use nickel_input::{KeyCode, KeyEdge, windows::NativeKeyboardEvent};
use std::sync::atomic::{AtomicU64, Ordering};

const ENABLED: u64 = 1;
const LEFT: u64 = 2;
const RIGHT: u64 = 4;
const FIRED: u64 = 8;
const GENERATION: u64 = !15;

pub(crate) struct WindowsEmergencyChord(AtomicU64);
impl WindowsEmergencyChord {
    pub(crate) const fn new() -> Self {
        Self(AtomicU64::new(0))
    }

    /// Owner transitions reset held keys and advance the generation atomically.
    /// An unchanged setting preserves a partly pressed chord. Exhaustion cannot
    /// enable a recognizer whose generation could be confused with an old one.
    pub(crate) fn set_enabled(&self, enabled: bool) -> bool {
        let result = self
            .0
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |state| {
                if (state & ENABLED != 0) == enabled {
                    return Some(state);
                }
                match (state & GENERATION).checked_add(16) {
                    Some(generation) => Some(generation | u64::from(enabled)),
                    None => Some(state & GENERATION),
                }
            })
            .expect("chord state update always supplies a value");
        !enabled || result & GENERATION != GENERATION || result & ENABLED != 0
    }

    pub(crate) fn observe(&self, event: NativeKeyboardEvent) -> bool {
        let observed = self.0.load(Ordering::Acquire);
        self.observe_generation(event, observed & GENERATION)
    }

    fn observe_generation(&self, event: NativeKeyboardEvent, generation: u64) -> bool {
        if event.injected {
            return false;
        }
        // Preserve physical_key's scan-code overrides without constructing its
        // allocating NativeKey fallback for unrelated or unknown keys.
        if event.scan_code == 0x29 || (event.scan_code == 0x1c && event.extended) {
            return false;
        }
        let bit =
            match nickel_input::windows::virtual_key_to_key_code(event.virtual_key, event.extended)
            {
                Some(KeyCode::ControlLeft) => LEFT,
                Some(KeyCode::ControlRight) => RIGHT,
                _ => return false,
            };
        let pressed = event.edge == KeyEdge::Pressed;
        let result = self
            .0
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |state| {
                if state & GENERATION != generation || state & ENABLED == 0 {
                    return None;
                }
                if !pressed {
                    return Some(state & !bit & !FIRED);
                }
                let next = state | bit;
                Some(if next & (LEFT | RIGHT) == LEFT | RIGHT {
                    next | FIRED
                } else {
                    next
                })
            });
        result.is_ok_and(|before| {
            pressed && before & FIRED == 0 && (before | bit) & (LEFT | RIGHT) == LEFT | RIGHT
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn edge(right: bool, edge: KeyEdge, injected: bool) -> NativeKeyboardEvent {
        NativeKeyboardEvent {
            virtual_key: 0x11,
            scan_code: 0x1d,
            extended: right,
            edge,
            injected,
        }
    }
    #[test]
    fn windows_emergency_chord_accepts_either_order_once_until_release() {
        for first in [false, true] {
            let chord = WindowsEmergencyChord::new();
            assert!(chord.set_enabled(true));
            assert!(!chord.observe(edge(first, KeyEdge::Pressed, false)));
            assert!(!chord.observe(edge(first, KeyEdge::Pressed, false)));
            assert!(chord.set_enabled(true)); // A refresh must not discard the first key.
            assert!(chord.observe(edge(!first, KeyEdge::Pressed, false)));
            for side in [first, !first] {
                assert!(!chord.observe(edge(side, KeyEdge::Pressed, false)));
            }
            assert!(!chord.observe(edge(first, KeyEdge::Released, false)));
            assert!(chord.observe(edge(first, KeyEdge::Pressed, false)));
            assert!(!chord.observe(edge(!first, KeyEdge::Released, false)));
            assert!(chord.observe(edge(!first, KeyEdge::Pressed, false)));
        }
    }
    #[test]
    fn windows_emergency_chord_ignores_injected_starts_completions_and_releases() {
        let chord = WindowsEmergencyChord::new();
        assert!(chord.set_enabled(true));
        assert!(!chord.observe(edge(false, KeyEdge::Pressed, true)));
        assert!(!chord.observe(edge(true, KeyEdge::Pressed, false)));
        assert!(!chord.observe(edge(false, KeyEdge::Pressed, true)));
        assert!(chord.observe(edge(false, KeyEdge::Pressed, false)));
        assert!(!chord.observe(edge(true, KeyEdge::Released, true)));
        assert!(!chord.observe(edge(false, KeyEdge::Pressed, false)));
        assert!(!chord.observe(edge(true, KeyEdge::Released, false)));
        assert!(chord.observe(edge(true, KeyEdge::Pressed, false)));
    }
    #[test]
    fn windows_emergency_chord_discards_events_from_a_previous_enable_generation() {
        let chord = WindowsEmergencyChord::new();
        assert!(chord.set_enabled(true));
        assert!(!chord.observe(edge(false, KeyEdge::Pressed, false)));
        let old = chord.0.load(Ordering::Acquire) & GENERATION;
        assert!(chord.set_enabled(false));
        assert!(!chord.observe(edge(true, KeyEdge::Pressed, false)));
        assert!(chord.set_enabled(true));
        assert!(!chord.observe_generation(edge(false, KeyEdge::Pressed, false), old));
        assert!(!chord.observe(edge(true, KeyEdge::Pressed, false)));
        assert!(chord.observe(edge(false, KeyEdge::Pressed, false)));
        assert!(!chord.observe_generation(edge(true, KeyEdge::Released, false), old));
        assert!(!chord.observe(edge(false, KeyEdge::Pressed, false)));
    }
    #[test]
    fn windows_emergency_chord_preserves_side_specific_identity_and_scan_overrides() {
        let chord = WindowsEmergencyChord::new();
        assert!(chord.set_enabled(true));
        let mut left = edge(true, KeyEdge::Pressed, false);
        left.virtual_key = 0xa2;
        assert!(!chord.observe(left)); // Explicit left VK overrides extended flag.
        let mut right = edge(false, KeyEdge::Pressed, false);
        right.virtual_key = 0xa3;
        for scan in [0x29, 0x1c] {
            let mut unrelated = right;
            unrelated.scan_code = scan;
            unrelated.extended = true;
            assert!(!chord.observe(unrelated));
        }
        assert!(chord.observe(right));
        assert!(!chord.observe(NativeKeyboardEvent {
            virtual_key: u32::MAX,
            scan_code: u32::MAX,
            ..right
        }));
        assert!(!chord.observe(right));
    }
    #[test]
    fn windows_emergency_chord_generation_exhaustion_cannot_enable() {
        let chord = WindowsEmergencyChord(AtomicU64::new(GENERATION));
        assert!(!chord.set_enabled(true));
        assert!(!chord.observe(edge(false, KeyEdge::Pressed, false)));
        assert!(!chord.observe(edge(true, KeyEdge::Pressed, false)));
        assert!(chord.set_enabled(false));
    }
}
