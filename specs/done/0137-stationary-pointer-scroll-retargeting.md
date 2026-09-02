# Spec 0137: Stationary-Pointer Scroll Retargeting

## Problem

Nickel's compositor can change window stacking without receiving pointer motion, notably when the
user switches applications with the keyboard. Smithay retains the pointer focus established by the
last motion event. A subsequent wheel event is therefore delivered to the formerly topmost window:
scroll Chrome, switch to Konsole without moving the pointer, then scroll, and Chrome continues to
scroll.

## Required behavior

- Before dispatching an ungrabbed pointer-axis frame, resolve the surface currently under the
  pointer from current compositor stacking and update pointer focus at the unchanged location.
- Deliver the axis frame to that newly resolved surface.
- Preserve active pointer-constraint ownership; stacking changes must not steal focus from a
  constrained surface.
- Preserve active pointer-grab ownership.
- Apply the behavior to both continuous and discrete horizontal and vertical axis input.

## Acceptance

1. Place Chrome and Konsole so the stationary pointer location is inside both windows.
2. Scroll Chrome and observe Chrome content move.
3. Switch to Konsole by keyboard without moving the pointer.
4. Scroll and observe Konsole receive the wheel input while Chrome remains unchanged.
5. Repeat in the reverse direction.
6. Run the focused `nickel-session` tests and strict Clippy for the crate.

The owner accepted the focused automated verification and code-level routing evidence for
completion; any installed-session regression will be reopened from direct observation.
