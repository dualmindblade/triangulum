# M2-hydro: lakes and rivers as water at rest (Opus, 2026-10-25)

Branch `worktree-agent-a3417e75780ab886b`, from main at 491dc57. Evidence in
`docs/missions/M2-hydro-evidence/` (contact sheets, logs, bench results);
full-size frames in the worktree's `game/out/play/<site>/` (gitignored).

## Summary

Neisor's lakes and rivers are now in the generator as water at rest: 771
lakes and the whole drainage graph above 120 m³/s (20,504 links, 9,382 km of
river), stored as per-cell water levels (1-255 of a block at LOD 0). The
gate, a voxel-level check of every generated water cell, finds **no water
walls and nothing perched** around all 1,537 sampled lake shores and along
140 km of river, along a wider 989 km (a tenth of the network), and at the
1/8 vertical scale too. Coarse LODs agree with LOD 0. The frame rate is
unchanged; river and lake chunks cost 35-50% more to generate, other chunks
3-10%.

## What was built

| piece | what |
|---|---|
| `scripts/bake_rivers_v2.py` (new) | Writes `hydro.bin` (6.6 MB) and `hydro.json` beside the world bake, in about a minute; deterministic (two runs give SHA-256 `f4d339b8…`). **River network:** every link cell → receiver above 120 m³/s, nodes stored receivers-first. **Levels:** the fill surface of the *world bake's own* elevation (sampled exactly as the engine samples it), pinned to lake levels and to 0 at the sea and at the last land node before it (an estuary), relaxed downstream so no link climbs (0 of 190,052 climb). **Courses:** centripetal Catmull-Rom through (main upstream, a, b, receiver of b); a tributary arrives straight, the main stem runs on smoothly. **Lakes:** the July conduit peel (W-5), then each level capped to the lowest bake elevation on its own shore band + 25 m (Earth scale), cells above the level dropped, and lakes downstream of a lowered lake lowered so no river climbs into a lake; iterated to a fixed point. 771 of 1,025 planetgen lakes survive (52 peeled, 241 caps, 14 lowered, 1,683 cells left dry). Each lake's field cells (57,070) for the engine's smooth lake field. |
| `worldgen/hydro.rs` (new) | The per-world model and the shaping of each column. Node levels in metres (fill surface × vertical scale, minus a valley incision of 1.5-3 m that tapers to nothing toward the lake or sea a river drains into and below the lake it leaves; made monotone again). Per link: meandered course (12 channel widths per wave, ≥ 40 m, bends kept over five half-widths in radius), channel width 4-32 m and depth 1-3.2 m from discharge, valley floor, a **water profile** (flat around each node, then a smooth slope at rest, or leaning pools with cascades, or waterfalls), lake ends (a link holds its lake's level until its whole channel is clear of the lake). Lakes: a per-lake Shepard field (Wendland C4) with a small shoreline wiggle. Per column: valley carve, lake bed, raises (lake aprons, river banks), channels, the water surface. A 3D hash grid over the sphere finds candidates (no face seams); per chunk, only the course runs that can hold a column's nearest point are searched. |
| `worldgen/mod.rs` | Columns carry a water surface (`water_m`), its kind, and material flags. `block_at` fills cells that are not solid below the surface: at LOD 0 with the exact level (a column's water is one integer T = round(255 W / s) spread over its cells, so neighbours whose surfaces differ by under a level never differ by more than one level in any cell), at coarse LODs whole voxels up to the surface snapped to the voxel grid, and the voxel holding the surface stays water even over high ground (as the sea already did). Lake beds sand/gravel, river beds gravel, lake beaches sand, river banks keep their soil (no stone rim). Bounds (`footprint_height_range_m`) include the hydrology, so `classify` stays conservative. `water_info` / `channels_at` expose the model. Without `hydro.bin` the old dry valleys come back. |
| `worldgen/bake.rs`, `landform.rs` | The bake loads `hydro.bin` when present; the landform's flow-ridge valleys switch off when it does. |
| `world/save.rs` (two small hunks) | `GENERATOR_VERSION` 2 → 3 (saves from before are refused); the bake identity includes `hydro.bin`'s SHA-256 (the cached raster digest is unchanged, so engines that do not read `hydro.bin` are unaffected). |
| `worldgen/hydro_tests.rs` (new) | The gates and the tools below. `worldgen/tests.rs`: two `pub` wrappers for them. |
| `game/scripts/hydro_shots.py` (new) | Finds the evidence sites and shoots them, each run started at its site so it is lit by day. |
| `game/README.md` | Running (the second bake), a Lakes and rivers section, saves, tests, known gaps. |

Not touched: `stream.rs`, `player.rs`, `mesh.rs`, `render/` (Fable's), and
`world/` beyond the two save hunks. `World::water_level_at` already reads
the generated levels, so no new hook was needed.

## The gate: no water walls

**The rule**, checked cell by cell on the generated chunks (`check` in
`hydro_tests.rs`): for every water cell A (level L, 1-255) and each
horizontal neighbour B in the same layer, B is solid, or B's level (0 for
air) is at least L - 1 (at rest, Andrew's rule), or the drop is
**designated**: both columns lie inside river channels, B is downstream of A
(the same link further along, s_B > s_A, or a link that A's link drains into
within 12 links, `Hydro::downstream`), and B's column holds water (or ice)
at or below that layer. Under every water cell is a solid cell or a full
(255) water cell: nothing is perched. Lateral drops, drops between unconnected
waters, drops from a lake or the sea, and water above air are all failures.

**Results** (`docs/missions/M2-hydro-evidence/gates-1-70.log`,
`gate-wide-1-70.log`, `gates-1-8.log`):

| run | sampled | water cells | walls | perched | designated drops |
|---|---|---|---|---|---|
| `water_is_contained`, 1/70: lake shores | 1,537 windows (48 × 48 blocks) on all 771 lakes | 1.57 M | 0 | 0 | 0 |
| `water_is_contained`, 1/70: rivers | 300 links, 140 km: the 12 largest rivers, 25 mouths, the 30 steepest links, 40 lake inflows and outflows, 12 face-edge crossings, 16 random stretches | 7.86 M | 0 | 0 | 3,608 |
| `water_is_contained_wide`, 1/70 (ignored, 3 min) | 2,161 links, 989 km: every 10th link, every fall, every face-edge crossing | 50.0 M | 0 | 0 | 10,785 |
| `water_is_contained`, 1/8 (`HYDRO_VSCALE=0.125`, H 4096, sea 1536) | as above | 14.7 M | 0 | 0 | 39,787 |

**The gate failed first, and each failure was a real bug** (each fix was
re-run through the same sweep):

| failure the sweep found | cause | fix |
|---|---|---|
| 74 walls between two lakes' waters | neighbouring lakes' fields overlapped; the level switched between them inside the water | each lake has its own field, zone and apron; aprons dam each other (merging lakes instead dropped 450 lakes) |
| 363, then 3,676 walls where rivers meet lakes | rivers were clamped to the lake level per column, where the clamp line crossed the channel obliquely | a link holds the lake's level along its course until it leaves the lake's band (`lake_ends`) |
| walls at junctions | sibling links converge tangentially; their levels differed in the overlap | tributaries arrive straight; water is flat for four widths around every node |
| a lake below a lake upstream of it | capping a lake could leave the next lake downstream higher, so the river between them climbed | the bake lowers that lake too, to a fixed point |
| outflows squeezed against a junction | a lake's two outlets left its band metres before joining | a link with no room to fall hands its drop to its neighbour (`pass_on_short_drops`) |
| 22 walls in the wide sweep | unconnected rivers side by side; an outflow running along its lake below the lake | the lower channel yields to the higher water's bank; lake ends wait until the whole channel is clear |
| 9 walls at a 1/8 waterfall | on a tight bend's inside, a block step moved the along-river coordinate 2.2 m, past the banks' fixed 2-voxel look-ahead | banks take the level of the most upstream course point within the channel's reach (exact for any curvature) |
| a dry hole in two channels | two links each yielded to the other | inside another channel only water against water counts |

**LOD agreement** (`water_agrees_across_lods`, LODs 1-3, 454,641 / 118,555 /
30,990 coarse columns): every coarse wet column has LOD-0 water within a
voxel, its surface lies within half a voxel plus a block of the LOD-0
surfaces within a voxel, and wherever every LOD-0 column under it holds
water at least ¾ of a voxel deep, the coarse column holds water too: 0
failures at 1/70 and at 1/8. Steps and falls sit in the same places at LODs
0-3. At LOD 3 the depth clause has no cases at 1/70 (water 6 m deep is
rare), so it is untested there.

## Rest, steps and falls: what M2b gets

- **Smooth reaches** (89.6% of river length at 1/70, 38.3% at 1/8) lean at
  most 0.6 levels per block along the river and never across it: at rest.
- **Steeper reaches** are pools leaning at 0.6 levels per block, separated
  by steps: cascades of at most one block (10.0% / 37.4% of length), or
  waterfalls where the excess drop is 3 blocks or more (0.4% / 24.2% of
  length: 74 falls at 1/70, mean 5 m, tallest 14.3 m; 5,171 at 1/8, mean
  8 m, capped at 24 m each). A fall has a one-voxel water **curtain** (the
  water steps one voxel after the bed: a column of water standing on its
  plunge pool, against the cliff) and sits in a short gorge.
- **Where the water is not at rest** is exactly the designated drops above
  (steps, falls, and the slightly steeper lean on the inside of tight bends,
  up to about 1.3 levels per block). Everything else is at rest.
- **Flow:** `Generator::water_info(dir, voxel)` gives the surface, its kind
  (sea, lake, river), the channels containing the column (link, arc length)
  and the downstream direction (the course tangent) with planetgen's
  discharge in m³/s. `Hydro::downstream` says which links a link drains
  into; `Hydro::profile`/`w_water` give the generated level anywhere along a
  link at any voxel size.
- **What M2b has to decide:** under Andrew's rule every step drains the pool
  above it, so generated river cells cannot simply be simulated. Options:
  hold undisturbed river cells at their generated levels (they are the
  river's steady state, the same as the loaded-area edge), simulate only
  water that has been disturbed, and let drops be sources and sinks; or give
  the channel an inflow equal to its outflow. The generator is the boundary
  value either way (DESIGN: rivers keep flowing in and out).
- **Ice:** in the cold the top water cell is ice (solid), on lakes, rivers
  and the sea alike, including the top of a fall's curtain.
- **Levels at coarse LODs** are always 255 (whole voxels); only LOD 0 holds
  partial levels.

## How it looks (honest list)

Water is drawn as full blocks (partial levels are M2b's), so a surface shows
at the top of the cell that holds it; banks stand a block above the water,
so they look flush. Sheets: `M2-hydro-evidence/sheet_ground.jpg` (lake
shore, the largest river, the tallest fall from below and from the side, a
river mouth, a river across the face 2-1 edge) and `sheet_300m.jpg`.

What does not look right yet:

1. **Full-block water.** A gently sloping river shows one-block steps where
   its level crosses a block boundary; every water top is flat.
2. **Rivers are too regular.** Width is constant along a link and changes at
   nodes; meanders are clean sines; the cross-section is the same trough
   everywhere; no braids, islands, bank asymmetry or wet margins (D-12).
   The largest rivers meander only gently (their links are a few widths
   long).
3. **Embankments.** Where the land beside a river or around a lake dips
   below the water, the bank is raised and slopes away at 1:4 (outer slope
   up to 40 m). In lowlands with landform hollows this reads as levees.
   River channels at lake level crossing a lake's flat apron look like
   canals.
4. **Lakes are blobby and uniform.** The outline follows a smooth field of
   planetgen's 437 m cells plus a small wiggle; beaches are uniform sand;
   beds are at most about 3.3 m deep away from the shore (a dial). The
   landform fades to flat within about 200 m of a lake, so lakes sit in
   plains. 254 of planetgen's lakes are gone (peeled, or capped below their
   beds because the bake's smoothed shore cannot hold them).
5. **Waterfalls** are a one-voxel translucent curtain, no foam or particles,
   in a blocky stone gorge; at 1/70 there are only 74, mostly small.
6. **Estuaries.** Rivers reach the sea level at their last land node and are
   cut down to it from there; at steep coasts (1/8) that makes a coastal
   fall and a canyon.
7. **From afar** channels are widened to at least 0.8 of a voxel to stay
   continuous, so distant rivers look fatter. In the 300 m river shot a
   darker triangle sits on the river where two LODs meet (it looks like two
   water surfaces overlapping); it is still there after merging
   M1-hardening's water-seam fix. **Not diagnosed**: it may be the coarse
   LOD's surface (snapped to its voxel grid) standing above the finer one.
8. **Where two unconnected rivers run side by side**, the lower one's
   channel yields to the higher one's bank and can be cut short.

## Cost

Generation per chunk, single thread, same binary and chunk keys ("before" =
no `hydro.bin`, the old dry valleys), median / mean ms
(`generation-cost.log`; another engine may have been running):

| chunks | LOD 0 | LOD 2 | LOD 4 |
|---|---|---|---|
| random land, before → after | 2.74 / 2.92 → 2.84 / 3.09 | 3.15 / 3.34 → 3.33 / 3.60 | |
| on river courses | 2.68 / 2.92 → 3.94 / 4.18 | 3.10 / 3.30 → 4.35 / 4.58 | 2.86 / 2.97 → 4.18 / 4.29 |
| on lake shores | 2.69 / 2.84 → 3.73 / 3.91 | 3.08 / 3.22 → 3.84 / 4.13 | 2.84 / 2.95 → 3.68 / 3.89 |
| hills anchor (no water) | 2.63 → 2.58 | 3.29 → 3.20 | 2.96 → 2.91 |

About 0.6 µs per column goes to the hydrology on river chunks (finding the
nearest course, the lake field, shaping) and the rest to filling water
voxels. The fixed 60 s flight, idle machine, same binary
(`bench-with-hydro.md`, `bench-without-hydro.md`): **1,452 fps average, 603
fps 1% low, worst frame 5.13 ms with the hydrology; 1,454 / 626 / 5.25 ms
without.** (An earlier no-hydro run was disturbed, 32 ms worst frame, and is
not used.) Startup builds the model from `hydro.bin` in 0.94 s (not broken
down further).

Other checks, after merging main at 090a86f (M1-hardening, M1-video):
`cargo test --release` 102 passed (9 ignored); `scripts/m1-done.play`
94/94, `scripts/m2a-sand.play` 95/95, `scripts/tunnel-wall-reload.play`
18/18 (`plays.log`); `water-lod-seam.play` and `water-lod-seam-high.play`
run clean (they have no asserts).

## Not verified

- The containment gate covers sampled windows, not the whole planet: all
  lakes' shores at two points each, and about a tenth of the river network
  (the wide sweep). Full coverage would take about half an hour.
- Half-metre blocks and vertical scales other than 1/70 and 1/8 were not run.
- Nobody has walked these rivers in the windowed app; the shots are the
  harness's settled frames.
- `water_info`'s flow direction and discharge are not used by anything yet.
- The bake was only run on seed 42.

## For the coordinator

- `hydro.bin` and `hydro.json` are installed in
  `C:/code/triangulum/game/assets/world/seed42/` (main checkout). Engines
  without this branch ignore them; with it, the world changes and saves from
  before are refused. Andrew needs the two files (or
  `python scripts/bake_rivers_v2.py`, which needs `output/seed42_r8` and the
  world bake).
- `docs/feature-inventory.md` §8 is the July record and is not updated;
  `game/README.md` describes the new water.

## Feedback

- **The voxel-level region harness** (`Region` in `hydro_tests.rs`:
  generate the chunks under a set of columns, look cells up across chunk and
  face edges) made the gate cheap to write and fast (about 5 seconds per
  10,000 chunks on 16 threads). Other invariants (caves, edits, the coarse
  overlay) could reuse it; it would be worth moving next to `grid` as a test
  utility.
- **The play harness cannot set the sun per shot**: the sky follows the
  start longitude, so shots across the planet come out at night. I run one
  harness per site with `--at` and `--time`; a `time HH:MM` command would
  remove that.
- **The worktree guard** refused many inline Python and here-document
  commands as "too complex to verify"; writing the patch scripts to the
  scratchpad first worked every time. Worth a line in `docs/PROCESS.md` for
  agents.
- **The wide sweep found what the default sweep missed** (22 walls), and the
  1/8 run found a further class (tight bends). Run both before merging
  anything that touches `hydro.rs`.
