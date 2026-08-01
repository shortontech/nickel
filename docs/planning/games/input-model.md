# Game Input Model

## Goal

Let every game operate on player intent without knowing which physical device expressed it. The
first implementations may use WASD, arrow keys, Enter, Space, and a pointer. This conventional path
is sufficient for early game development and must not be delayed for eye-tracking work.

Eye tracking, fixation, dwell detection, switches, foot pedals, muscle-twitch detectors, touch, and
gamepads are later input adapters. They should not require alternate game rules.

## Semantic Boundary

Game logic consumes a small vocabulary similar to:

```rust
enum GameAction {
    Navigate(Direction),
    Aim(Vec2),
    Primary,
    Secondary,
    Cancel,
    Pause,
}
```

This is a planning sketch, not a committed Rust API. Individual games should use only the subset
they need and may define game-specific semantic actions above this shared layer.

Physical events are translated before reaching game logic:

```text
Keyboard and pointer ---------+
Gamepad ----------------------+--> semantic actions --> game
Gaze and dwell ---------------+
Gaze and external switch -----+
Switch scanning or foot pedal-+
```

The adapter owns device details such as key codes, dead zones, switch debounce, fixation smoothing,
dwell timing, tracking confidence, and calibration. The game owns the meaning and consequences of
an action.

## Low-Bandwidth Design Target

Core play should be possible with directional intent and no more than four discrete actions. This
is a design constraint comparable to targeting a particular controller, not a limit on strategic
depth. Depth should come from planning, timing, positioning, combinations, resources, consequences,
and generated situations rather than from a large binding vocabulary.

Continuous aiming is permitted when it has a directional or pointing equivalent. Games must not
require pixel-perfect gaze or sustained rapid activation.

## Future Gaze and Switch Behavior

The preferred low-latency combination is:

```text
gaze chooses intent
switch commits intent
dwell provides a switch-free commit path
```

Interactive regions may be larger than their visible artwork. Future gaze support should allow
hysteresis, target attraction, adjustable dwell time, progress feedback, cancellation, and optional
confirmation. Brief tracking noise should not continually reset deliberate progress.

These requirements preserve the future architecture; they do not put eye tracking or dwell
detection on the initial implementation path.

## Invariants

- Game state must not branch on a diagnosis or physical controller type.
- Input choice must not change saves, progression, content, scoring rules, or multiplayer identity.
- Timing and presentation accommodations must be independently configurable.
- Local multiplayer may assign different devices and action subsets to different players.
- New trigger-like hardware should be bindable without modifying individual games.
