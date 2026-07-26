# planetgen/ — the world generator (Python)

One seed → a whole earthlike planet. Stage order:
`tectonics → climate1 → hydrology → climate2 → biomes → chronicle → render`.

Python is a deliberate choice here, not a legacy accident: Austin reads and
reviews this code himself. **Favour clarity over cleverness** — a vectorized
one-liner nobody can review is worse than three obvious lines.

## Invariants

- **`config.py` is the control panel.** Every tunable lives there with a
  comment. New behaviour gets a named, commented config field with a sane
  default — never a magic number buried in a stage.
- **CPU numpy is the canonical reference.** Same seed + same res must give the
  same planet. `numba` (hydrology) is **bit-identical** to the pure path;
  `--gpu` (climate, float32) matches within f32 tolerance but is *not*
  guaranteed bit-identical across machines. When something disagrees, CPU
  numpy is right. `tests/accel_test.py` guards this parity — keep it passing.
- **All noise is a pure function of 3D position and seed** (`noise.py`), so the
  viewer can re-evaluate it at any resolution for consistent fine detail.
  Breaking that purity breaks local generation in the game.
- **`planet_data.npz` and the face bakes stay engine-agnostic.** No
  viewer-specific assumptions leak into the dataset.
- **Angles in `planet_data.npz` are RADIANS.** The viewer takes degrees —
  multiply by 180/π when generating poses. This has bitten survey generators.
- Cells are vertices of a subdivided icosahedron; sample arbitrary lat/lon via
  a KD-tree on `xyz` with inverse-distance weighting
  (`grid.py:sample_latlon` is the reference).

## Neisor quirks (the shipped planet, seed 42)

- It has **genuine dry below-sea-level basins**. Sea is `ocean-mask AND
  elevation <= 0`, never elevation alone.
- Its lakes are **cold** (median mean-temperature ~1.7 °C). Any lake feature
  gated above roughly 5 °C will find an empty world and look broken.

## After changing generation

The viewer eats baked derivatives, not the raw dataset. Changing a stage means
re-running the bakes it feeds:

```
python -m planetgen --seed 42 --res 8          # or --from <stage> to reuse cache
python scripts/bake_faces.py output/seed42_r8 1024
python scripts/bake_rivers.py output/seed42_r8
python scripts/bake_weather.py output/seed42_r8
```

A full res-8 run is ~1.5 h; `--from <stage>` reuses the per-stage cache and is
almost always what you want while iterating. `--until <stage>` stops early.

Generation is GPU- and CPU-hungry and the family plays the game on the same
machine — **do not launch long `--gpu` runs without checking**.

## Gates

`python tests/smoke_test.py`, `python tests/accel_test.py` (accelerator
parity). Visual review is the map atlas: `output/<world>/maps/atlas.html`.
