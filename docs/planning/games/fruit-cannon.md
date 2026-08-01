# Fruit Cannon

## Status

Technology experiment. This is the preferred first game prototype because a small playable loop can
exercise much of Nickel's shared procedural technology.

## Player Fantasy

Operate an expressive cannon that fires strange generated fruit through, into, or at a reactive
world. The presentation can be colorful and funny without becoming child-coded. Weight, ripeness,
surface, shape, and internal composition should affect how fruit flies, hits, breaks, and sounds.

## Core Loop

1. Aim at a target, route, structure, or opportunity.
2. Choose or receive a fruit with visible properties.
3. Fire.
4. Observe its trajectory, impact, splatter, ricochet, or structural effect.
5. Adapt the next shot to the resulting state.

The eventual game may emphasize score attacks, puzzles, destruction, defense, or turn-based local
competition. The first experiment only needs satisfying aim, fire, flight, and impact.

## Semantic Actions

- `Aim(Vec2)` selects a direction or target region.
- `Primary` fires.
- `Secondary` may change fruit or cannon mode.
- `Pause` suspends play without penalty.

Keyboard and pointer controls come first. Arrow keys or WASD adjust aim, Space fires, and a pointer
aims directly. Future gaze can aim while dwell or a separate switch fires.

## Sources of Depth

- Fruit mass, shape, bounce, fragmentation, and surface behavior
- Cannon power, arc, ammunition order, and limited special actions
- Structures, moving targets, wind, terrain, and chain reactions
- Choosing between immediate score and changing the board for a later shot
- Generated challenges with reproducible seeds

Depth should not depend on rapid clicking or precision smaller than a stable target region.

## Procedural Systems

The first visual proof is a generated fruit with a black-and-white striped mask. A two-tone palette
filter maps the mask to chosen colors. Later passes can add shape families, gradients, spots,
roughness, stems, damage, ripeness, internal layers, and animated deformation.

Other generated content may include:

- Fruit geometry and physical properties
- Material masks and palette filters
- Cannons, targets, structures, and terrain
- Firing, flight, impact, splat, and break sounds
- Challenge layouts and ammunition sequences

## Accessibility Without Simplification

Aim regions can be larger than visible targets, and optional aim attraction can stabilize a chosen
target. Adjustable pacing, trajectory previews, reduced motion, dwell progress, and confirmation
may change interaction without changing the challenge definition or score rules.

## Shared Technology Exercised

- Procedural meshes, masks, palettes, and material filters
- Seeded sound synthesis
- Simple physics and collision
- Directional and pointer-equivalent aiming
- Deterministic challenge generation
- Large semantic interaction regions

## Smallest Playable Experiment

Render one striped fruit, allow keyboard and pointer aiming, fire it from one cannon, collide with
one target, and synthesize distinct firing and impact sounds. Record the seed needed to reproduce
the fruit.

## Open Design Questions

- Is the durable loop destruction, puzzles, score attack, defense, or a mixture?
- Does the player choose fruit deliberately or adapt to a generated queue?
- Which physical differences remain readable and strategically meaningful?
- Should local multiplayer alternate shots or assign asymmetric roles?

## Explicit Non-Goals

- Eye tracking and dwell detection in the first implementation
- A large content catalog before firing one fruit feels good
- A separate simplified control mode
- A launcher or menu entry for the technology experiment
