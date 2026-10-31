# Design decisions

Decided by Austin and Andrew on 2026-10-24 unless noted. Change these only
with their OK; record the change here with the date.

## The planet grid

- Cube-sphere. Each face is an N×N grid of columns; columns are radial
  prisms; layer k is the radial block index. A block is identified by
  integers (face, i, j, k).
- Projection: smooth and exactly equal-area, computed numerically once and
  shipped as a table: `Projection::EqualAreaCobe` in `crates/cubesphere`
  (2026-10-25). Every block footprint has the same area to within 1e-6, and
  only 14% of the surface is more than 10° off square (COBE 16%, gnomonic
  31%). Andrew can still pick another grid by eye before the first saved
  world: https://claude.ai/artifact/7qz75zidRkhjMbVcKU3apY
- The 8 cube corners stay. Andrew: "part of the charm".
- Face frames match `viewer/src/planet.rs` `FACES`, so old and new data line up.

## Sizes

- Block size s: 1 m, with ½ m as a switch. Andrew (2026-10-30): the scale
  stays where it is for now.
- World height H, in blocks, is a power of 2, set per world: 2,048, with 512
  below sea level, for now (Andrew, 2026-10-30). The split above and below
  sea level is its own setting.
- Planet size N = k·H columns per face edge. Default N = 131,072 (k = 256).
  With equal-area blocks, R = s·N·√(6/4π) ≈ 0.691·s·N, so R ≈ 90.57 km.
- LOD voxel sizes are 1, 2, 4 … H blocks. The coarsest LOD voxel equals the
  world height. Chunks are 32³ voxels. Because N is a multiple of H, LOD
  voxels tile every face exactly.
- Other bodies (the moon) follow the same rules with their own N.

## Terrain

- planetgen keeps Earth physics at Earth's radius; the shrink happens in the
  bake.
- Vertical scale: 1/70 (`vertical_scale: null`, Earth's slopes) for now
  (Andrew, 2026-10-30: keep things where they are). It stays a dial. Earlier
  (2026-10-24) he said large features (mountain ranges, ocean trenches)
  should sit somewhere between Earth-sized at a local scale (vertical scale
  1) and at a planetary scale (1/70), with ≈ 1/8 the middle (peaks ~1 km,
  deepest trench ~1.26 km, range flanks ~8× steeper than Earth's), so this
  may come back.
- Relief finer than planetgen's resolution comes from the engine's
  generator. Its roughness is a dial for Andrew.
- Sampling is smooth everywhere: no visible raster texels or noise seams.
- Keep the old world's character: geological strata (now as real blocks),
  biome surfaces, trees, rocks, caves, rivers, lakes, shores, ice.
  See `docs/feature-inventory.md`.
- Clouds are a sky effect above the world height; they do not shrink 1/70.
- Time: 24-minute days (one game minute per real second), from Andrew's
  October vision. His July calendar (7-day lunar month, 3 months per season,
  84-day year, all adjustable) carries over unless he changes it: to confirm.

## Blocks

- Block textures are 16×16 pixels (Andrew, 2026-10-30): `texture_size` in
  `game/assets/blocks.json`.

## Water (Andrew)

- One byte of water level per block: 256 levels.
- Water moves only downhill, and only when neighbours differ by at least 2
  levels, so it always comes to rest. Resting water may lean up to 1/256.
- Generated lakes, rivers and oceans start at rest. Simulation runs only
  where something changed.
- At the edge of the loaded area, cells take their values from the
  generator: rivers keep flowing in and out, lakes and oceans stay full.
- River currents come from planetgen's flow data, not the water simulation.

## Things that move

1. **Falling blocks** fall straight down their own column (sand, gravel,
   collapses). Columns keep their shape, so nothing needs converting.
2. **Local contraptions** (doors, drawbridges, elevators, felled trees) keep
   the warped shape of where they were built and snap back when they come to
   rest nearby.
3. **Vehicles** are built on their own perfect-cube grid from a frame block
   and never merge into the planet.

## Saves and replays

- Saves store every chunk that changed, whatever changed it. Block ids are
  stable and the generator is versioned.
- Replays are a snapshot plus the ordered player inputs.
