# Creature Garden

## Status

Technology experiment. It is not currently committed to being a standalone game or appearing in
Nickel's menu.

## Player Fantasy

Raise unusual generated creatures whose bodies, movement, behavior, and capabilities change in
response to care, environment, diet, inheritance, and chance. The pleasure should come from
discovery, attachment, experimentation, and surprising systemic consequences rather than from a
child-coded virtual-pet presentation.

This concept takes inspiration from the broad idea of games about growing physically strange pets,
but Nickel should develop its own creatures, terminology, systems, art direction, and progression.

## Core Loop

1. Observe creatures and their current needs, traits, and behavior.
2. Choose food, materials, habitat changes, activities, or pairings.
3. Let creatures grow, adapt, or reproduce.
4. Discover how physical and behavioral traits changed.
5. Use the resulting creatures in new experiments or, potentially, dungeon expeditions.

Observation should reveal useful patterns without making every outcome immediately predictable.

## Semantic Actions

- `Navigate(Direction)` moves among creatures, objects, or action regions.
- `Aim(Vec2)` points at a creature or habitat location when spatial placement matters.
- `Primary` selects, places, feeds, or interacts according to the current context.
- `Secondary` opens an alternative action or information panel.
- `Cancel` and `Pause` leave the current interaction safely.

Keyboard and pointer input come first. The interaction model should also work when gaze chooses a
creature or object and a switch commits the current action.

## Sources of Depth

- Interactions among inherited traits, diet, habitat, and behavior
- Physical forms that produce genuine advantages and disadvantages
- Creatures learning or adapting to the environment
- Selective breeding or another open-ended inheritance system
- Habitat construction and population dynamics
- Attachment to imperfect individuals rather than optimization toward one ideal body
- Potential expedition roles if the system joins the dungeon game

## Procedural Systems

- Body topology, proportions, limbs, joints, markings, palettes, and surface traits
- Parameterized rigs and locomotion that tolerate unusual bodies
- Temperament, preferences, learned behaviors, and social interactions
- Trait inheritance, mutation, growth, and environmental influence
- Creature calls, movement sounds, and habitat ambience
- Habitat objects, food, materials, and environmental effects

Generated forms must remain renderable, selectable, saveable, and capable of some valid movement.
Failures should be reproducible from creature identity, lineage, seed, and generator version.

## Accessibility Without Simplification

The game should tolerate unhurried observation and deliberate interaction. Time may pause while an
action menu is open. Large stable creature and object regions, focus persistence, dwell progress,
and switch confirmation can be added later without changing growth, inheritance, or progression.

## Shared Technology Exercised

- Procedural bodies and materials
- Generalized rigs and animation
- Seeded behavior and inheritance
- Persistent generated state
- Synthesized creature and habitat audio
- Semantic interaction with moving irregular objects

## Smallest Playable Experiment

Generate one creature from a compact body description, render it with a two-tone procedural
material, animate valid movement, feed it one of two foods, and produce a visible deterministic
growth change that survives save and reload.

## Relationship to Dungeon

This experiment may become the dungeon's home and party system. Dungeon materials could influence
growth, while generated creature bodies could determine combat, traversal, sensing, carrying, or
puzzle abilities. The resulting expedition-and-growth loop may be stronger than two separate games.

Keep the experiment independent until procedural bodies, movement, and growth are interesting and
robust. Merge the plans only after both sides contribute meaningful decisions to one shared loop.

## Open Design Questions

- Are changes driven by diet, inheritance, environment, use, or a combination?
- How readable should the relationship between a choice and a mutation be?
- What prevents optimization from collapsing the population toward one best creature?
- How much direct control over creatures is desirable?
- Is this a standalone sandbox, the dungeon's companion system, or both views of one game?

## Explicit Non-Goals

- Copying another pet game's creatures, presentation, terminology, or progression
- Treating unusual bodies as jokes about disability
- Promising a standalone release before the experiment is fun
- Eye tracking or dwell detection in the first implementation
