# Plan

Each milestone ends with a build Austin and Andrew can play, a short video,
and one performance number: does a fixed flight hold 60 fps at 1080p on the
RTX 2060, and what are the worst 1% of frames. No dates are promised.
The full reasoning is in the analysis page linked from `STATUS.md`.

## M0 — Prove it ✅ (2026-10-24)

A throwaway prototype drew Neisor from the ground to orbit in at most
0.30 ms of GPU and 1.2 ms of main-thread time per frame (target: 8 and 4).
Code: branch `worktree-agent-ace814068b00b2cc5`, `spike/voxel-core/`.

## M1 — A planet you can dig

The new engine in `game/`, next to the old viewer.

**Done when** you can walk and fly anywhere on Neisor, dig a tunnel, build a
bridge, quit, reload, see the bridge from a distance, and the fixed test
flight holds 60 fps at 1080p.

| Task | What | Who | State |
|---|---|---|---|
| M1-grid | `crates/cubesphere`: six projections incl. the smooth exact equal-area `EqualAreaCobe`, exact inverses, tests | Astra | done |
| M1-bake | `scripts/bake_world.py`: smooth, artifact-free rasters from `planet_data.npz`, with geology fields and automated checks | Opus | done |
| M1-inventory | `docs/feature-inventory.md`: every feature of the old viewer, how it works, which milestone ports it | Opus | done |
| M1a | Engine skeleton from the prototype: planet parameters, palette chunks, block definitions, generator v1 (bake sampling, 1/70 vertical scale, strata, biome surfaces, ocean), LOD up to H, renderer, free-fly camera, fps overlay, headless bench | Fable | done |
| M1b | Player physics, block picking, break/place with a hotbar, saves of changed chunks, edits visible at distance, scripted play harness | Fable | done |
| M1-terrain | Landforms between the bake's scale and the metre scale (ridges, valleys, escarpments), so the vertical scale can be judged on real-looking terrain | Opus | started |
| M1-review | Independent review of the merged engine by a different model family: 15 findings, routed to M1b, M1-terrain and M1c (`docs/reviews/2026-10-25-M1a-astra.md`) | Astra | done |
| M1c | Sky and sun from the old viewer, 24-minute days, main-thread fixes (incremental traversal on a non-overlapping coverage tree), review findings #8 #9 #11 #12 #15, chunk keys scoped to a body, skipping Enclosed chunks once `classify` lands, a sneak key for bridging, edits across a face edge in coarse padding | — | next |

## M2 — Things fall and flow

The simulation tick and block updates. Falling sand and gravel, collapses.
Water with 256 levels and sloped surfaces, following Andrew's rules in
`DESIGN.md`. Rivers and lakes from planetgen that sit still until disturbed.
Replays that reproduce a bug exactly.

**Done when** digging into a lake drains it into your tunnel, diverting a
river doesn't dry up the loaded area, and the frame rate holds.

## M3 — Andrew's workshop

Blocks defined in files Andrew edits, block textures, a live look panel
(sky, fog, light, colours, terrain roughness) that saves to a file,
non-full blocks (slabs, poles, plants), and parts inside blocks (a pipe in a
wall).

**Done when** Andrew can add a block or change the look of the world without
asking anyone.

## M4 — Things that move

Falling trees and Create-style contraptions that snap back into the grid.
Vehicles on their own grid with real physics, starting with a cart and a boat.

## M5 — Orbit and back

The far view of the planet, the moon with the r3 changes (maria and rays),
orbits and eclipses, weather and clouds.

## Later

Inventory, crafting, technological progression, power and fluid networks,
proper vehicles, multiplayer.
