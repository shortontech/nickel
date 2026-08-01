# Nickel Shapes Test

`nickel-shapes-test` is an early procedural articulated-organism generator. Fruit is the first
constrained phenotype, not the boundary of the system.

Recipes describe semantic topology. They do not contain vertices, curve control points, bone
coordinates, proportions, or random-number ranges. The generator derives those details
deterministically from the recipe, node relationships, and text seed.

An apple recipe is intentionally small:

```yaml
name: apple
seed: orchard-demo

root:
  kind: sphere
  skin: apple
  children:
    - kind: branch
      children:
        - kind: leaf
        - kind: leaf
```

The branch attachment causes the generator to deform the sphere around the connecting joint. A
dimple is therefore an effect of organism topology, not an authored modifier. The `apple` skin is a
family whose coherent palette and pattern vary with the seed.

Branches can carry other organisms or structures. For example:

```yaml
name: grapes
seed: vineyard-demo

root:
  kind: branch
  children:
    - kind: leaf
    - kind: grape_cluster
```

Future articulation uses semantic motion intent such as `motion: walking`. The generator, rather
than the recipe, will construct the required skeleton, joints, constraints, and gait.

## Constraint-driven creatures

A creature recipe describes anatomy, ecology, locomotion, and reusable terminal components:

```yaml
name: bulb-backed marsh grazer
seed: mossy-pond

root:
  kind: creature
  torso: fruit
  head: frog
  terminal_component:
    recipe: ../components/paw.yaml
    overrides:
      digits:
        count: 3
      constraints:
        grasping: moderate
  back: bulb
  locomotion: quadruped
  diet: herbivore
```

Diet produces weighted ecological pressures for binocular and peripheral vision, bite force,
digestive volume, pursuit speed, and camouflage. Locomotion is then resolved through support-limb
roles, pelvis and shoulder orientation, spine posture, center of mass, neck compensation, head
angle, and gait structure. Changing `quadruped` to `biped` recomputes that dependency chain.

Reusable component files own bounded local construction details. The bundled
`components/paw.yaml` describes palm tissue, digit and bone-count ranges, pads, weight support,
traction, and grasping. Organisms apply only sparse overrides.

Interactive mode uses a `wgpu` depth-tested material shader with key and fill lighting, specular and
rim response, and a small subsurface approximation. PNG mode remains a deterministic CPU reference
render for generator tests.

Open the bundled apple interactively:

```bash
cargo run -p nickel-shapes-test
```

Render any recipe to PNG:

```bash
cargo run -p nickel-shapes-test -- \
  --shape crates/nickel-shapes-test/shapes/grapes.yaml \
  --png grapes.png \
  --seed another-vineyard
```

Serialize the resolved bones, parts, ecological pressures, dependency graph, gait, and component
state for a specific creature:

```bash
cargo run -p nickel-shapes-test -- \
  --shape crates/nickel-shapes-test/shapes/bulb-frog-quadruped.yaml \
  --save-state creature-state.yaml \
  --png creature.png
```

In window mode, press `1` through `4` to change LOD, `S` to save the current frame, and Escape to
exit.
