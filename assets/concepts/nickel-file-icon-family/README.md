# Nickel File icon-family concepts

These seven source images are the retained masters for Nickel File's repository-owned fallback icon
family. Nickel File derives runtime artwork from them by trimming transparent generation-canvas
space, applying one shared optical inset, and scaling with aspect ratio preserved. Keeping the
masters here makes the derivation repeatable without replacing or independently hand-editing them.

## Provenance

- Generated with OpenAI ImageGen on 2026-08-31.
- Selected from two exploratory six-image batches generated in the Codex desktop application.
- The exact original generation prompts were not retained with the output files.
- Intended prompt set: coherent transparent-background 3D icons for an image file, text file,
  generic folder, home folder, pictures folder, and music folder.
- `unknown-file.png` was generated with OpenAI ImageGen in Codex on 2026-09-03, using
  `text-file.png` as a style/composition reference. Its request was: "Create a generic unknown file
  icon matching the reference icon family's polished soft 3D design. Show one white rounded document
  sheet with the same blue folded corner and blue backing edge, but replace the text lines with one
  centered, unmistakable blue question-mark symbol. Use a genuinely transparent background; no
  words, letters, watermark, or extra objects; preserve the family's perspective, canvas use,
  rounded edges, lighting, and folded-corner construction."
- License: same license as Nickel.

## Source files

| File | SHA-256 |
| --- | --- |
| `image-file.png` | `7ad8b1935c1bb774f41a9e47e0e75e29639310d613fef08f651185ec1c478056` |
| `text-file.png` | `e6bd3410eb2b294b2f3fc600271f35d8ef6cbd9c2182add560bdc584ac539e92` |
| `folder.png` | `befa4351e2f22c200f07103d4b1c2f51de4303e0da9c3a7352fdbeec05066ec2` |
| `home-folder.png` | `2c492d5438c43d6c0b7e376ce07399560d64fa66b2ebf0ae4054195bcf9cb2e4` |
| `pictures-folder.png` | `b4477db1170bae282f22c38099e3327fbdc0e680c8c652b62177e1b57684cc77` |
| `music-folder.png` | `667c2b11bcb2351e6e6476c1b761d4d04b268a02f1af604aff7c2229c10c24ee` |
| `unknown-file.png` | `e2132ad8b5ca505ea2f453f533835f4d81f8a6f33853e50c73081e05d4e141de` |

All retained PNGs are 1254 x 1254 RGBA source masters. Derived runtime sizes should be generated
from these files rather than replacing them.
