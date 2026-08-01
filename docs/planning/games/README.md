# Nickel Games

## Purpose

Nickel's games are ordinary, enjoyable open-source games built around a broad input model. They
are not therapy, assessments, or simplified games for disabled players. A player using gaze, a
switch, a foot pedal, a gamepad, a keyboard, or a mouse should play the same mechanics, earn the
same progression, and share the same scores.

The games also provide focused proving grounds for Nickel's rendering, input, procedural-content,
audio, persistence, and accessibility infrastructure. A documented game is not necessarily a
release commitment. Experiments may remain developer-only and do not need a launcher or menu entry
until they are genuinely fun.

## Product Principles

- Reduce motor demands, not intellectual, strategic, or creative demands.
- Do not use child-coded art, language, rewards, or presentation merely because a game supports
  assistive input.
- Consume semantic player actions rather than keyboard keys, buttons, gaze samples, or platform
  events in game logic.
- Begin development with conventional keyboard and pointer controls. Add gaze, dwell, switches,
  pedals, muscle-trigger devices, and other adapters later without redesigning the games.
- Prefer large, stable action regions and deliberate decisions over high-speed precision.
- Keep accessibility options within the same game. They may change timing, target size, feedback,
  confirmation, or animation, but not replace the game with a lesser version.
- Prefer compact, deterministic procedural content while allowing authored assets when they produce
  a materially better result.
- Keep shipped game code and tooling in Rust.

## Maturity

Each game advances independently through these states:

1. **Idea:** A documented premise and open design questions.
2. **Technology experiment:** A narrow test of shared rendering, generation, input, audio, or
   simulation infrastructure.
3. **Playable prototype:** A complete loop that can be evaluated for fun.
4. **Candidate:** A game worth polishing and integrating with Nickel.
5. **Shipped:** A supported game exposed to users.

Moving forward requires evidence from the current state. A successful technology experiment does
not establish a fun game, and a playable prototype does not automatically belong in Nickel's menu.

## Initial Games

| Game | Initial state | Central idea | Shared technology exercised |
| --- | --- | --- | --- |
| [Fruit Cannon](fruit-cannon.md) | Technology experiment | Aim and fire fruit with generated appearances and effects | Materials, meshes, physics, audio, aiming |
| [Branching Racer](branching-racer.md) | Idea | Choose rails through a fast rollercoaster-like course | Route generation, anticipation, streaming, timing |
| [Dungeon](dungeon.md) | Idea | Explore through deliberate navigation and tactical choices | Buildings, encounters, persistence, procedural worlds |
| [Creature Garden](creature-garden.md) | Technology experiment | Grow strange creatures whose forms affect their abilities | Bodies, inheritance, animation, behavior |

The creature garden may become the dungeon's home, party, or companion system instead of remaining
a separate game. It stays separate during experimentation so neither concept prematurely dictates
the other's design.

## Shared Plans

- [Input model](input-model.md)
- [Procedural content](procedural-content.md)

When implementation begins, create a narrowly scoped numbered specification under `specs/`. A
planning document describes durable intent; an active specification defines a bounded deliverable
with verification and completion criteria. Archive completed specifications under `specs/done/`.
