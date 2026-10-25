# M1b: walk, dig, build, save (Fable, 2026-10-25)

Branch `worktree-agent-a942bd6aeffdd4bb4`, crate `game/`, on top of main
f96a3ba. Evidence in `docs/missions/M1b-evidence/` (downscaled) and, full
size, under `C:\code\triangulum\.claude\worktrees\agent-a942bd6aeffdd4bb4\game\out\`
(gitignored, this machine only). The scenario that produced the frames is
`game/scripts/m1-done.play`; `game/README.md` has the controls and the
harness.

## What was built

| module | what |
|---|---|
| `player` | `Player`: walk mode with radial gravity, a 0.6 × 1.8 m box (eyes 1.62 m) swept axis by axis (k, then i, then j) against unit cells in the local lattice frame; `Frame` turns world metres into lattice units with the grid's own metric (columns are 0.93–1.08 m here, layers exactly s), so half-metre blocks need no special case. Jump 8 m/s under 25 m/s² (1.28 m), walk 4.3, sprint 6.5, swim 2.2 m/s; buoyancy net-upward when the head is under, a gentle sink at the surface, Space/Ctrl swim up/down. Fly mode (G) is the old free-fly. A body found inside rock climbs out. `pick`: Amanatides–Woo DDA through the lattice, 6 m reach, returns the cell, the entered face's normal and the resolved `BlockPos`. |
| `world` | `World` = generator + edit overlay; the one thing streaming, physics and picking read blocks from. `block_lattice(face, i, j, k)` accepts indices past the face (resolved onto the neighbour) and outside the height (bedrock below, air above). LOD-0 natural chunks are cached (1024) for physics. `set_block` records the edit and queues every chunk whose padded grid holds the block, at every LOD, across face edges. `generate(key)` runs the pure generator and then the overlay. |
| `world::edits` | `EditStore`: edited LOD-0 chunks as dense ids (authoritative, never regenerated), dirty set, invalidation queue, debounce clock. |
| `world::save` | `game/saves/<world>/header.json` + `chunks/b0_f{F}_{X}_{Y}_{Z}.chunk`. Manifest: format version 2, generator version, body id, world identity (seed, s, H, sea level, N, vertical scale, roughness), projection name and the EqualAreaCobe table's SHA-256 (hashed from the same bytes the crate embeds, checked against the digest its README documents), the bake's SHA-256 (meta.json plus each face raster's name and size), the block-id table. Everything but the display name must match or the load is refused with every reason listed. Atomic writes (tmp + rename). |
| `sha256` | FIPS 180-4, dependency-free, with the standard test vectors. |
| `session` | What the app and the harness share: step, pick, break, place (refused into the player or into a solid), teleport, mode switch, debounced save, save on exit, reload (flush, new store from disk, new engine), the state JSON. |
| `stream` | Jobs read through `World` and carry the chunk's revision; an edit bumps the revision and remeshes resident chunks; a result from an older revision is rejected and the chunk re-dispatched, so a late generation never overwrites an edit. A mesh the quad arena cannot take waits in an `awaiting` queue and is retried after frees instead of being installed as an empty chunk. `settled()` also waits for pending edits and uploads. |
| `grid` | `Planet::dir` past a face side now goes through `wrap`: explicit `EdgeMap`s derived from `FACES` (neighbour, its shared side, whether the edge coordinate flips), pure index arithmetic, so a centre half a voxel past the edge is the neighbour's own centre half a voxel inside, at any LOD. Corner policy: cross the side with the larger overshoot, then cross again from the neighbour (the diagonal continues onto the third face at the corner), then clamp. `Planet::try_new` rejects a sea level that is not a multiple of H/8 (sea level 513 would leave LOD 0 only: 6.4e9 roots) and an N the coarsest chunks do not tile; the coarsest chunk is also capped at the face so small test planets stay valid. |
| `app`, `ui` | Fixed 60 Hz physics with an accumulator, LMB break / RMB place with 250 ms repeat while held, 1–9 hotbar, G walk/fly, R to the ground, scroll = fly speed; HUD: targeted-block outline (12 projected segments), hotbar with the selected block's name, walk/fly/swim state, edited and unsaved chunk counts, save notes; the water label follows `--hide-water` (the M1a bug). |
| `play` | `--play SCRIPT`: fixed-step headless runs with `at`, `atblock`, `look`, `lookaxis`, `aim`, `turn`, `mode`, `hold`, `tap`, `wait`, `select`, `water`, `save`, `reload`, `shot`, `state`, `bench`, `column`, `log`, `assert`. Shots are settled frames with the game HUD plus a JSON sidecar; asserts cover position, altitude, on_ground, in_water, the crosshair block, `block[f,i,j,k]` and `block_rel[di,dj,dk]`; any failure exits 1. Each run gets its own save directory, wiped first. |
| `main` | `--play`, `--save`, `--no-save`, `--keep-save`, `--walk`/`--fly`, `--hotbar`; a relative `bake` in world.json resolves against `game/`; world parameters and saves are checked before the engine is built, with a message and exit code 2. |

### The coarse-LOD edit rule

A coarse voxel covering edited blocks shows the topmost solid block in it
when that block was placed by an edit — and on a tie in layer, the edit
wins — becomes air (or water) when every fine block in it is non-solid and
the voxel lies entirely in edited chunks, and otherwise keeps the
generator's value. The tie clause is what keeps a deck that is flush with
the field visible: without it the one-block bridge vanished at LOD 1 and
up (found by the 1 km shot, not by any assert; see Evidence). Side effect:
one block placed at surface level colours its whole coarse voxel from afar.
A hole smaller than a voxel does not show from afar, by design.

### Review findings folded in (Astra's M1a review, via the coordinator)

1. `Planet::dir` past a face edge: rewritten as above; tests at all four
   sides of every face, a corner, LOD 0 and a 64-block LOD.
2. Failed `add_mesh` installed as an empty chunk: now the `awaiting` queue.
3. Edits vs render residency, stale results, body ids: edits live in the
   `EditStore`, jobs carry revisions, saves and the manifest carry the body
   id. **`ChunkKey.body` itself is deferred:** adding the field means
   touching four struct literals in `worldgen/mod.rs` tests, which this
   mission was told not to edit; it is a four-line change once worldgen is
   free.
4. The save manifest: done as above.
5. Config traps: `Planet::try_new`, with the reasons in the message.
6. Buried chunks: nothing here relies on their contents. Physics treats a
   uniform-stone chunk as solid, which is all collision needs; digging into
   one shows no strata until Opus's `classify`/`generate` lands. The
   streamer already never visits chunks below their footprint's terrain
   band, so skipping `Enclosed` is a one-line change when `classify` exists.

## Evidence

### Tests (`cargo test --release`, 36 passed)

New this mission: edge maps are mutual and cover the cube; border columns
continue onto the neighbour at every side of every face, at LOD 0 and at a
64-block LOD, and the step across an edge is the same size as a step inside
it (the old code turned back into the face at negative sides); corners
continue onto the third face; unsupported configurations are rejected with
the reason; chunk bytes round-trip and damage is rejected; the edit store
tracks change and dirt; saves round-trip only the changed chunks, a
mismatched manifest is refused with every reason (block size, N, H, seed,
vertical scale, bake, block table, generator version, projection table),
the projection table hash equals the documented one, the bake identity
follows meta.json and raster sizes; edits are authoritative, reach the
neighbour's padding and the right ancestors, and propagate to coarse LODs
(a 12 m bridge at LOD 1–4, a flush surface edit at LOD 1–3, a 2×2×2 dig
reads air at LOD 1 while one dug block does not); player: rests exactly on
a floor with no drift, walls stop the box at the metric-correct distance, a
running jump lands on a step, half-metre blocks (a 0.5 m gap blocks a 0.6 m
body, 1 m admits it; a 1.5 m tunnel keeps a 1.8 m body out, 2 m admits it;
the jump clears a 1 m step), walking across a face edge and round a corner
keeps the floor, the ground flag and the speed with no position jumps,
swimming floats near the surface and Space swims up, picking finds the
facing block, its normal and the place cell.

### The scenario (`game/scripts/m1-done.play`: 83 asserts, 0 failed)

At the hills preset (face 2, i 107156, j 57519, ground top layer 548, flat
grass over dirt over sandstone for ten columns along +i), in 4.4 s:

| frame | shows |
|---|---|
| `01_start` | standing on the grass, walk mode, HUD and hotbar |
| `02_trench` | after 12 breaks from the rim: a 4-column, 3-deep trench (grass, dirt, sandstone walls) |
| `03_tunnel` | from inside the tunnel dug out of the trench's far wall: 3 columns long, 2 high, grass roof |
| `04_bridge` | the stone deck from the field, flush with the grass, targeted block outlined |
| `04_under_bridge` | from the trench floor: the deck overhead with 2 m of air beneath it |
| `05_reloaded` | after `save` and `reload` (a fresh edit store from disk): the same scene; 14 block asserts on the deck, the voids, the tunnel and the untouched neighbours |
| `06_bridge_300m` | the deck from 335 m, LOD 0: a 4 m × 1 m grey dash |
| `06_bridge_550m` | from 600 m, LOD 1: a 2 m grey block |
| `07_bridge_1km` | from 1.07 km, LOD 2: two 4 m stone cubes |

Walking: 6 m in 1.5 s at 4.3 m/s on the ground, a jump leaves the ground
and lands within a second, a 0.4 s walk off the rim lands on the trench
floor, walking into the far wall stops pressed against it (`i == 107160`),
and walking across the deck stays on layer 549. The deck is placed by
aiming at the +j face of the grass beside each trench cell, so the block
lands in the trench cell; every placement is asserted. The last `bench 120`
at the 1 km pose: 319–377 fps.

### Bench (PROVISIONAL: the machine's state drifted across runs)

Machine: RTX 2060, Ryzen 7 2700X, 16 rayon threads, 1920×1080 offscreen,
Vulkan. No other build or render showed in the process list before or
after any run. Full tables in `M1b-evidence/bench-results.md`.

Static poses (120 frames each after settling):

| pose | GPU avg ms | CPU avg ms | draws | loaded | M1a idle run GPU / CPU |
|---|---|---|---|---|---|
| stand_hills | 0.12 | 0.83 | 875 | 3575 | 0.12 / 0.77 |
| alt_100m | 0.50 | 1.18 | 866 | 3470 | 0.19 / 0.89 |
| alt_1km | 0.46 | 0.73 | 628 | 2229 | 0.18 / 0.62 |
| alt_10km | 0.40 | 0.38 | 376 | 1103 | — |
| alt_100km | 0.24 | 0.35 | 150 | 263 | — |
| alt_500km | 0.21 | 0.21 | 294 | 294 | 0.08 / 0.20 |

The chunk counts differ from M1a's because that run was on the
equiangular stub; main now uses EqualAreaCobe. The CPU side matches M1a's
idle run. The GPU side is 2.5× slower at altitude for the same triangle
counts (alt_500km: 0.05M tris in both, 0.21 ms against 0.08), which is a
GPU clock-state difference, not more work; the standing pose is identical.

The flight, six runs in one hour, the pre-mission engine (main f96a3ba,
built into a separate directory and flown with the same bake) between two
M1b runs:

| run | frames | avg fps | 1 % low fps | worst ms | frame p50 / p99 ms | CPU avg / p99 ms | GPU avg / p99 ms |
|---|---|---|---|---|---|---|---|
| M1b, full bench | 44630 | 747.1 | 188.3 | 24.06 | 1.29 / 3.80 | 0.79 / 2.35 | 0.34 / 1.12 |
| M1b, flight only | 46073 | 771.2 | 189.6 | 42.42 | 1.27 / 3.43 | 0.77 / 2.31 | 0.34 / 1.16 |
| baseline f96a3ba, after its build | 32117 | 537.7 | 91.1 | 38.96 | 1.60 / 4.69 | 1.09 / 2.45 | 0.46 / 1.32 |
| M1b A (back to back) | 32028 | 536.1 | 102.4 | 24.95 | 1.55 / 6.98 | 1.11 / 3.73 | 0.41 / 1.30 |
| baseline (back to back) | 27007 | 452.0 | 99.4 | 16.64 | 1.68 / 8.89 | 1.24 / 5.92 | 0.39 / 1.18 |
| M1b B (back to back) | 26380 | 441.6 | 95.2 | 17.90 | 1.72 / 8.99 | 1.24 / 5.62 | 0.40 / 1.17 |

Reading: adjacent runs of M1b and the pre-mission build are within noise
of each other, and the machine slowed over the hour (CPU p99 2.3 → 5.9 ms
with identical code), so none of today's tails belong to the engine. The
process list afterwards showed who else had the CPU: Docker's backend
(1200 s of CPU time), TortoiseGit's cache scanning the worktrees (515 s)
and the other Claude sessions (1300 s); none of them is a build or a
render, which is all the process check before each run looked for. M1a's
verified number (633 fps, 307 fps 1 % low, 4.8 ms worst) was measured on a
quieter machine than this one was today. No regression is attributable to
M1b; the project number wants re-measuring on an idle machine before it is
believed. In the harness, `bench 120` at the 1 km pose gave 319–377 fps.

Worker costs are unchanged in kind: LOD-0 generate median 2.3 ms (p95 3.8),
mesh 0.23 ms, on 16 threads.

### App

`--hide-water --screenshot --exit-after 4` from the hills preset: the HUD
reads "walk: on ground … water hidden … edits 0 chunks (0 unsaved)" with the
hotbar and crosshair (`M1b-evidence/app_hidewater.png`). The M1a HUD said
"shown" here.

## Not verified

- **Mouse-driven play.** Nobody has clicked. Austin should try: click to
  capture, look around, LMB on a block (and hold it: it should repeat every
  quarter second), RMB to place the hotbar block on the targeted face (it
  should refuse to place inside you), 1–9, G in the air (you should fall,
  then land), R, scroll in fly mode, quitting with Q or the window close
  (both save; the console says how many chunks), restarting to find the
  edits, walking into the sea (you should float, Space swims up), and a
  half-metre world (`--block-size 0.5 --n 262144 --save saves/half`) to dig
  in. Then `--block-size 0.5` against the 1 m save should refuse to start
  with the reasons.
- Walking across a real cube edge and corner. The collision tests use a
  synthetic flat floor; no terrain at a cube edge has been walked, and no
  wall at a corner has been pushed against (the corner policy makes the
  diagonal cell a copy of a neighbour's cell; that is approximate).
- The `awaiting` upload path (arena full) never triggered in any run.
- Coarse chunks ignore edits across a face edge in their one-voxel padding
  (a culling detail at cube edges next to edits).
- Many edited chunks: each coarse generation scans every edited chunk's
  bounding box (cheap) and every block of the ones it overlaps; with
  thousands of edited chunks this wants a spatial index.
- The bench ran once, after the mission's builds, with nothing else running
  that I could see; it is one run on one machine.

## What I would change in the plan

1. Bridging by walking backwards (Minecraft style) is not possible yet:
   placing needs a target face, and from the block you stand on only its top
   face is reachable. A sneak key that lets you target the side of your own
   block (M1c or M3) is the usual answer; the harness used `atblock`.
2. `ChunkKey.body`: four lines in worldgen tests once that file is free.
3. The coarse-LOD rule's "one block colours a voxel" side effect is a
   look question for Andrew; a count threshold is a one-line change.
4. Physics generates LOD-0 chunks on the main thread when a teleport lands
   in uncached terrain (about eight chunks, a few ms each). Fine now; a
   prefetch from the streamer's results is the obvious next step.
5. The M1a harness idea of replaying recorded real sessions (`pose` lines)
   is worth bringing over in M1c, now that the fixed step exists.

## Feedback

- Tooling built here: the `--play` harness (`src/play.rs` header is the
  command list), `aim` (target a lattice point), `lookaxis`, `column`
  (print a column's blocks), and `scripts/probe-hills.play`. `aim` is what
  made the scenario deterministic; pitch arithmetic for a 6 m ray against
  1 m cells has margins of a few degrees and the grid's metric varies by
  8 %, so compute the direction from the lattice instead.
- Asserts test world state; frames test rendering. The coarse-LOD tie bug
  passed every assert (the blocks were right) and showed only in the 1 km
  frame, zoomed 9×. Keep both kinds of evidence and look at the pictures.
- The feet layer after walking is `n - 1e-10`; the physics was fine (it has
  an epsilon skin) but state reporting floored it to `n - 1` and every
  `block_rel` assert went one layer wrong. Report with tolerance.
- The worktree needs its own copy of `game/assets/world/seed42` now that
  world.json's bake path is relative: the tests use the absolute main
  checkout path and pass, while the binary silently points at the
  worktree's empty directory. Worth a line in CLAUDE.md rule 4.
- The Bash tool refuses long heredocs and `python -I` cannot import PIL;
  scripts in the scratchpad run fine.
