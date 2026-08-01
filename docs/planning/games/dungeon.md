# Dungeon

## Status

Idea.

## Player Fantasy

Explore a strange generated place, make tactical decisions, assemble capabilities, and gradually
understand a persistent world. The game should support deliberate play without reducing the depth
of its systems or presenting itself as a therapeutic exercise.

The dungeon may share a single loop with the creature garden: expeditions discover organisms, food,
and materials; creatures grown between expeditions provide new traversal and tactical abilities.

## Core Loop

1. Prepare a character, party, creature group, or equipment set.
2. Enter a generated dungeon region.
3. Navigate rooms and choose among encounters, risks, and routes.
4. Resolve combat, traversal, puzzles, negotiation, or resource problems.
5. Return with knowledge, organisms, materials, equipment, or changed creatures.
6. Use those results to reach previously inaccessible regions.

The first prototype should select one small version of this loop rather than implementing every
possible encounter type.

## Semantic Actions

- `Navigate(Direction)` moves focus, chooses a route, or moves on a grid.
- `Primary` selects the focused action.
- `Secondary` opens a contextual alternative or reverses a decision where allowed.
- `Cancel` returns from a panel or target selection.
- `Pause` suspends the game.

Keyboard navigation and pointer selection come first. Panels and world actions should use stable,
large regions so later gaze and switch adapters do not require new game rules.

## Sources of Depth

- Party, creature, equipment, and ability composition
- Positioning and turn order
- Information gathering and persistent world knowledge
- Resource expenditure across an expedition
- Routes gated by capabilities rather than rapid execution
- Encounters with several valid strategies and consequences
- Generated spaces that preserve authored tactical grammar

## Procedural Systems

- Room and corridor graphs, buildings, landmarks, and environmental layers
- Encounter composition, rewards, hazards, and faction state
- Materials, props, equipment, effects, and ambient audio
- Creatures and abilities if the creature garden becomes part of the game
- Seeds and generator versions for reproducible worlds and saved state

Generation must preserve navigation validity, readable choices, encounter constraints, and return
paths. It should create situations worth reasoning about rather than merely produce large maps.

## Accessibility Without Simplification

Turn-based or pausable resolution is preferred where fast action does not add meaningful depth.
Target enlargement, focus persistence, confirmations, reduced animation, and adjustable dwell
behavior should not change the underlying encounter, progression, or rewards.

## Shared Technology Exercised

- Procedural architecture and encounter grammars
- Deterministic simulation and persistence
- Focus navigation and action panels
- Tactical state independent of rendering and physical input
- Generated creatures, equipment, and materials

## Smallest Playable Experiment

Generate a small connected room graph, navigate it with directional actions, present one tactical
choice in each room, and reach different outcomes based on resources retained across the route.

## Relationship to Creature Garden

The strongest combined structure currently appears to be:

```text
explore dungeon
      |
      v
find organisms, food, and materials
      |
      v
grow or alter creatures
      |
      v
assemble abilities and a party
      |
      v
reach new dungeon regions
```

This remains a hypothesis. Creature generation should first prove interesting on its own, and the
dungeon should first prove that deliberate exploration and tactical choices form a satisfying loop.

## Open Design Questions

- Is the player one character, a party, a creature keeper, or a changing combination?
- Is movement grid-based, room-based, or continuous with semantic destinations?
- What information persists between failed expeditions?
- Do creatures replace conventional equipment, complement it, or remain a separate game?
- What is the smallest encounter system that produces real strategic variety?

## Explicit Non-Goals

- Real-time combat that requires rapid repeated input
- A giant generated world before a small route contains meaningful decisions
- Committing the creature garden to this game before either prototype proves its loop
- Eye tracking in the first implementation
