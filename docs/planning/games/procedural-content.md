# Procedural Content Strategy

## Goal

Build visually and mechanically rich games from compact, freely licensed, reviewable source. Use
seeded deterministic generators for textures, materials, meshes, buildings, creatures, levels,
animation, sound effects, and encounters where generation provides a good result.

Procedural content is a tool and a distribution strategy, not an aesthetic requirement. Nickel
should use authored fonts, music, sounds, or art when procedural generation would noticeably reduce
quality or coherence.

## Pipeline

```text
compact authored definitions
            |
            v
versioned deterministic generators
            |
            v
runtime or cached meshes, textures, audio, levels, and encounters
```

Authored definitions should expose meaningful artistic parameters rather than arbitrary random
numbers. A seed chooses controlled variation within those parameters; it does not replace art
direction.

## Shared Systems

### Geometry

Generate simple fruit, projectiles, architecture, track pieces, props, and creature bodies from
composable primitives. Keep collision geometry stable and testable even when display geometry
varies.

### Materials and Textures

Build surfaces from masks, palettes, gradients, noise, wear, and small reusable filters. A striped
fruit can begin with a black-and-white stripe mask and map its two tones through a palette filter.
The same filter can later express species, ripeness, damage, team identity, contrast, or emissive
states without duplicating texture assets.

### Animation and Simulation

Use parameterized rigs, procedural secondary motion, and lightweight physics when they make
generated shapes believable. Simulation must remain bounded and reproducible enough for testing,
saves, and replays.

### Audio

Use compact synthesis patches for UI feedback, firing, impacts, splats, creature calls, alarms,
horns, wind, tire noise, and stylized engines. Seeds may select small variations while explicit
parameters preserve a recognizable identity. Short sounds may be generated and cached as PCM;
continuous sounds may respond to live simulation state.

### Worlds and Encounters

Generate buildings, rooms, rails, routes, encounters, rewards, and creature populations from
versioned grammars with explicit constraints. Generation should produce meaningful decisions, not
merely rearranged decoration.

## Reproducibility and Versioning

A content identity should include at least a seed and generator version. Changing an algorithm must
not silently reinterpret an existing save. Generators should either preserve old versions, migrate
saved content, or store the generated result when regeneration cannot remain compatible.

Exact cross-platform audio or floating-point simulation may require caching generated output rather
than assuming sample-for-sample runtime equivalence.

## AI-Assisted Development

AI may help propose generator rules, palettes, patches, level grammars, tests, and variations. The
shipped result should remain compact, deterministic, offline-capable, reviewable, and modifiable
without requiring a generative service at runtime.

Structured source and visual or audible comparison fixtures should make generated changes easier to
review than large opaque asset collections.

## Quality Bar

- Generated output follows a deliberate adult-facing art direction.
- Seeds create coherent variation rather than parameter noise.
- Important gameplay silhouettes and audio cues remain recognizable.
- Collision, navigation, and interaction regions remain valid for every accepted seed.
- Generator failures are reproducible from recorded seed and version information.
- Assets and generator dependencies have compatible licenses and recorded attribution.
- A procedural technique is removable when it cannot meet the desired quality.
