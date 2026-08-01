# Branching Racer

## Status

Idea.

## Player Fantasy

Ride a generated vehicle through a spectacular branching rail system. The vehicle supplies forward
motion; the player reads the world and chooses which rail to take. It should feel like choosing a
personal route through a rollercoaster rather than steering a car across an open road.

Speed creates excitement and consequence, but success depends on anticipation and decisions rather
than continuous precision control.

## Core Loop

1. Travel automatically along the current rail.
2. Read approaching branches, hazards, rewards, landmarks, and incomplete information.
3. Select the next rail before reaching the junction.
4. Experience the chosen segment and its consequences.
5. Build a route through successive decisions toward a destination or score objective.

Branches may differ in risk, speed, scenery, resources, shortcuts, encounters, vehicle stress, or
future connectivity. A choice should be more meaningful than left being safe and right being hard.

## Semantic Actions

- `Navigate(Left)` and `Navigate(Right)` move among offered rails.
- `Primary` commits a highlighted rail or activates a power.
- `Secondary` may use a second power, brake, or change information view.
- `Pause` suspends play without losing the current choice.

The first implementation may use A/D or arrow keys. Pointer input selects a visible branch. Future
gaze selects a large branch region, with a switch or dwell committing the choice. A game option may
commit immediately when only two stable branch regions exist.

## Sources of Depth

- Route knowledge and partial information
- Short-term rewards versus long-term positioning
- Vehicle traits that favor particular rail conditions
- Powers or resources spent at strategic junctions
- Branches that merge, cross, loop, unlock, collapse, or change later options
- Generated route networks with discoverable landmarks and repeatable seeds
- Asymmetric local play, with one player selecting rails and another managing powers

## Procedural Systems

- Valid rail graphs with controlled pacing and reachable destinations
- Curves, elevation, supports, tunnels, buildings, terrain, and distant scenery
- Junction silhouettes and preview language that remain readable at speed
- Vehicles, materials, horns, engines, wheel noise, wind, and rail impacts
- Encounters, rewards, hazards, and route-specific events

The rail generator must validate clearance, curvature, choice visibility, route reachability, and
decision time. Visual spectacle cannot obscure which choices are available.

## Accessibility Without Simplification

Players may increase junction preview distance, widen selection regions, slow only the approach to a
decision, require explicit confirmation, or pause at junctions. These settings preserve the same
rail graph and consequences. Difficulty can come from planning and incomplete knowledge instead of
short reaction windows.

## Shared Technology Exercised

- Procedural graph and rail generation
- Streaming generated environments
- Camera motion and reduced-motion alternatives
- Semantic selection of spatial regions
- Continuous vehicle and environmental audio
- Deterministic routes and replays

## Smallest Playable Experiment

Move a placeholder vehicle automatically along rails containing three consecutive two-way
junctions. Allow left/right selection, show the selected rail clearly, validate every route, and
reach different endpoints based on the choices.

## Open Design Questions

- Is rail selection immediate, explicitly committed, or configurable?
- How far ahead can a player inspect the route without removing uncertainty?
- Are vehicles mechanically distinct or primarily expressions of the same rail system?
- Is there an opponent, a clock, a destination, a score, or a journey structure?
- What persists between runs: maps, vehicles, knowledge, resources, or constructed rails?

## Explicit Non-Goals

- Analog steering as a prerequisite for play
- Twitch reactions as the primary difficulty
- Procedural scenery before the rail-choice loop is interesting
- Eye tracking in the first implementation
