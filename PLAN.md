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
| M1-terrain | Landforms between the bake's scale and the metre scale (ridges, valleys, escarpments, mesas), plus four generator review fixes | Opus | done |
| M1-review | Independent review of the merged engine by a different model family: 15 findings, routed to M1b, M1-terrain and M1c (`docs/reviews/2026-10-25-M1a-astra.md`) | Astra | done |
| M1c-sky | Sky, sun, stars and fog from the old viewer, 24-minute days, day/night lighting | Astra | done |
| M1c-stream | Non-overlapping coverage tree with incremental traversal (finer distant detail), review findings #8 #9 #11 #12 #15, chunk keys per body, sneak key, skipping buried chunks via `classify` | Fable | done |

## M2 — Things fall and flow ✅ (2026-10-25)

The simulation tick and block updates. Falling sand and gravel, collapses.
Water with 256 levels and sloped surfaces, following Andrew's rules in
`DESIGN.md`. Rivers and lakes from planetgen that sit still until disturbed.
Replays that reproduce a bug exactly.

**Done when** digging into a lake drains it into your tunnel, diverting a
river doesn't dry up the loaded area, and the frame rate holds.

| Task | What | Who | State |
|---|---|---|---|
| M1-video | A ~1 minute M1 showcase clip (harness frame recording + ffmpeg); re-render with python game/scripts/showcase.py | Opus | done |
| M1-review2 | Independent review of everything merged since the first review (player, saves, harness, landforms, sky, coverage tree) | Astra | done |
| M2a | Simulation core: fixed 20 Hz tick separate from the sky clock, a deterministic block-update queue with neighbour notifications (across chunk and face edges), an active region around the player, falling sand and gravel, saves of sim-changed chunks, a determinism hash test | Fable | done |
| M1-worldgen-fix | Review 2 #12 (coarse oceans need opaque backing) and #13 (enclosed classification at the world ceiling) | Opus | done |
| M1-hardening | Review 2 #6 (picking across face edges), #7 (tunnel walls missing after reload), #8 (stranded stale meshes), #9 (water seams at real triangle edges), #10 (collision at face transitions); #17 (edit-history memory) deferred | Fable | done |
| M2-hydro | Generated lakes and rivers as water at rest (levels from planetgen, near-continuous river surfaces, waterfalls), gated by a no-water-walls containment test | Opus | done |
| M2b | Water: Andrew's 256-level rule, sloped water surfaces, sleeping when settled, generator values at the loaded-area edge, flow accounting; wired to the generated rivers | Fable | done |
| M2c | Replays: a snapshot plus ordered inputs reproduce a session exactly (F10 records, `--replay` plays back and reports the first divergence), the rest of review 2 #16, a harness `find` command, review 3 #4 #6 #13 | Fable | done |
| M1-review3 | Independent review of M2 (sim, water, rivers, saves): 13 findings (`docs/reviews/2026-10-25-engine-review3-astra.md`) | Astra | done |
| M2-hardening | Review 3 #1 #2 #3 #5 #7 #9 #10 #12 and review 2 #17: the loaded area's moving edge, saves, memory | Fable | done |

## M3 — Andrew's workshop

Blocks defined in files Andrew edits, block textures, a live look panel
(sky, fog, light, colours, terrain roughness) that saves to a file,
non-full blocks (slabs, poles, plants), and parts inside blocks (a pipe in a
wall).

**Done when** Andrew can add a block or change the look of the world without
asking anyone.

| Task | What | Who | State |
|---|---|---|---|
| M3a | Live look panel (F1): sky, light, fog, water, per-face block colours, distance detail, terrain dials as a preview world; `look.json` and `blocks.json` hot-reloaded; a screenshot button | Opus | done |
| M3b | Block and texture editor: add blocks and set their physics in the panel, hotbar assignment, per-face 16×16 textures (texture array, hot-reloaded PNGs), an in-game pixel editor, saves stay compatible when blocks are added | Opus | done |

## M4 — Things that move

Falling trees and Create-style contraptions that snap back into the grid.
Vehicles on their own grid with real physics, starting with a cart and a boat.

## M5 — Orbit and back

The far view of the planet, the moon with the r3 changes (maria and rays),
orbits and eclipses, weather and clouds.

## Later

Inventory, crafting, technological progression, power and fluid networks,
proper vehicles, multiplayer. Possibly a smooth mesh that blends with the
voxels at long distance: Andrew liked how the old planet looked from space
(Austin, 2026-10-30: not for now).
