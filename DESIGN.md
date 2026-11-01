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
- World height H, in blocks, is a power of 2, set per world: 2,048. Sea level
  is exactly in the middle, H/2, so the bottom half-height can be water and
  the top one air (Andrew, 2026-10-31; it was 512 below sea level).
- Planet size is counted in half-heights: N = u·(H/2) columns per face edge.
  Default u = 128, N = 131,072. The F1 terrain panel changes it in steps of 32,
  up to 1024 (Andrew, 2026-10-31; see `docs/missions/2026-10-31-round2.md`).
  832 is the Earth analogue: Earth's relief reduced until its deepest trench
  fits one half-height (the lead's reading of his words; he can correct it).
  N need not be a power of two. It must tile the coarsest chunks and fit the
  simulation's 20-bit horizontal coordinates, so taller worlds can have a lower
  limit. With equal-area blocks, R = s·N·√(6/4π) ≈ 0.691·s·N, so R ≈ 90.57 km
  today.
- Resizing uses a preview world with saving off. *Save look* writes the size as
  the default for new worlds and doesn't touch the open world's saved edits.
- LOD voxel sizes are 1, 2, 4 … H/2 blocks. The coarsest LOD voxel is half the
  world height (Andrew, 2026-10-31). Chunks are 32³ voxels. Because N is a
  multiple of the coarsest chunk edge, LOD voxels tile every face exactly.
- Other bodies (the moon) follow the same rules with their own N.

## Terrain

- planetgen keeps Earth physics at Earth's radius; the shrink happens in the
  bake.
- Vertical scale: 1/70 (`vertical_scale: null`, Earth's slopes) for now
  (Andrew, 2026-10-30: keep things where they are). `null` means R / Earth's
  radius and follows a change of planet size. It stays a dial. Earlier
  (2026-10-24) he said large features (mountain ranges, ocean trenches)
  should sit somewhere between Earth-sized at a local scale (vertical scale
  1) and at a planetary scale (1/70), with ≈ 1/8 the middle (peaks ~1 km,
  deepest trench ~1.26 km, range flanks ~8× steeper than Earth's), so this
  may come back.
- Relief finer than planetgen's resolution comes from the engine's
  generator. Its roughness is a dial for Andrew.
- **Local and global (Andrew, 2026-10-31).** *Global* features belong to the
  planet and stretch with its radius. *Local* features belong to the ground under
  your feet, are sized in metres, and are the same on any planet. In the code the
  split is `game/src/worldgen/scale.rs`.

  | | Global (stretches with R) | Local (metres) |
  |---|---|---|
  | Shape | continents, ocean basins, shelves, the bake's own slopes | peaks, ridges, spurs, valleys, hills, gullies, metre roughness |
  | Height | the bake's elevation × vertical scale: the *floor* of the land | relief standing on that floor: up to ~800 m on the strongest belts, 10-70 m hills elsewhere |
  | Where | the belts (active uplift, rough highlands), plains, plateaus | which ridge a peak is on |
  | Rivers | where they run, where they join, which lake or sea they reach | channel width and depth, banks, the valley's width and shape, steps and falls |
  | Lakes | where they are | their depth, beaches, the basin's slope to the shore |
  | Water levels | the floor along the course, less the incision; a lake's level follows the lowest river reaching it | |
  | Coast | where the shoreline runs | the beach, its slope, the shallows |
  | Climate | temperature and rain bands | the lapse on local relief: 6.5 °C per real km |
  | Rock | which strata, hardness, soil depth | bed thickness, the fold of the beds, mesa steps |

- Land falls towards water: rivers lie in valleys, and lakes in basins (Andrew,
  2026-10-31). Nothing dips below the bake's surface, so valleys lie on it. A
  river's level is the lowest natural ground along its course upstream, less a
  1.5-3 m incision. A lake takes the lowest level any river reaches it at.
- The world's ceiling is half the world height. Global and local heights are
  summed and eased under it, and where the bake's own heights leave too little
  room the local relief is thinned to fit, so a range keeps its shape instead of
  flattening into a cap. **For Andrew to judge:** on the 90 km planet this changes
  nothing, but at 832 half-heights the highest belts carry about 190 m of relief.
- Sampling is smooth everywhere: no visible raster texels or noise seams.
- The coast is a smooth curve from the bake's coastline with bounded detail;
  noise never decides where the sea ends (2026-10-31).
- Keep the old world's character: geological strata (now as real blocks),
  biome surfaces, trees, rocks, caves, rivers, lakes, shores, ice.
  See `docs/feature-inventory.md`.
- Atmosphere height and fog distance follow R. The look keeps their metre values
  with the radius they were set at, so a custom look keeps its proportions when
  the planet changes size.
- Clouds are a sky effect above the world height; they do not shrink 1/70.
- Time: 24-minute days (one game minute per real second), from Andrew's
  October vision. His July calendar (7-day lunar month, 3 months per season,
  84-day year, all adjustable) carries over unless he changes it: to confirm.

## Blocks

- Block textures are 16×16 pixels (Andrew, 2026-10-30): `texture_size` in
  `game/assets/blocks.json`.

## Water (Andrew)

- One byte of water level per block: 256 levels.
- Water moves only downhill. Sideways it moves when neighbours differ by at
  least 2 levels, so it always comes to rest, and resting water may lean up
  to 1/256. The one exception is a ledge: water beside a lower open space
  goes into it whatever its level, the last level included (Andrew's request,
  2026-10-30).
- Level-1 water with nowhere to go dries, unless it is the edge of a large
  body or of water fed from beyond the loaded area (2026-10-31, `--film`).
  Water evaporates (Andrew confirmed it, 2026-10-31).
- **Waiting for Andrew's OK (`sim_rules` 4, 2026-10-31):** to stop water blinking
  as it spreads (his #9), the game now narrows his ledge rule. A sheet of level-1
  water is one thing: a film cell with water beside it neither runs nor goes over
  a ledge, only a lone film cell does; thicker water drains through the sheet to a
  drop, and the sheet dries where it lies. Nothing spreads onto a cell over a drop,
  so a waterfall's lip is dry. If he says no, this goes and the rule above stands.
- Generated lakes, rivers and oceans start at rest. Simulation runs only
  where something changed.
- At the edge of the loaded area, cells take their values from the
  generator: rivers keep flowing in and out, lakes and oceans stay full.
- River currents come from planetgen's flow data, not the water simulation.

## Things that move

1. **Falling blocks** fall straight down their own column (sand, gravel,
   collapses). A block at rest with a free cell beside it and a free cell
   below that one moves into that column and falls (Andrew's request,
   2026-10-30), so powder rests with neighbouring columns at most one block
   apart. Columns keep their shape, so nothing needs converting. The picture of
   a slide is the renderer's: a block is drawn travelling from the cell it left
   over 0.2 s, and where a chunk's mesh is late the block is drawn where the mesh
   shows it. None of that is state (2026-10-31). Powder with levels
   (`--powder levels`) stays an experiment and stays out of terrain generation
   for now (Andrew, 2026-10-31).
2. **Local contraptions** (doors, drawbridges, elevators, felled trees) keep
   the warped shape of where they were built and snap back when they come to
   rest nearby.
3. **Vehicles** are built on their own perfect-cube grid from a frame block
   and never merge into the planet.

## Moving the player

- Holding a movement key follows a great circle, walking or flying (Andrew,
  2026-10-31). A body faces a direction carried along each move, not a compass
  heading; a heading is how people and scripts say a direction.
- The fly speed has no ceiling (Andrew, 2026-10-31).
- Leaving the window never stops the game: minimised it draws nothing and keeps
  stepping, and losing focus lets go of the mouse.

## Saves and replays

- Saves store every chunk that changed, whatever changed it. Block ids are
  stable and the generator is versioned.
- Replays are a snapshot plus the ordered player inputs.
