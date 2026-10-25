# M2a: the simulation core, falling sand and gravel (Fable, 2026-10-25)

Branch `worktree-agent-afd84da6ba31ec2d0`, crate `game/`, from main at
f7c44fd (M1 feature-complete). Evidence in `docs/missions/M2a-evidence/`
(downscaled shots and logs) and, full size, under
`C:\code\triangulum\.claude\worktrees\agent-afd84da6ba31ec2d0\game\out\`
(gitignored, this machine only). Scripts: `game/scripts/m2a-sand.play`
(new), `game/scripts/m1-done.play`, `game/scripts/edge-corner-flight.play`.

## What was built

| module | what |
|---|---|
| `sim` (new) | `Sim`: a fixed 20 Hz tick (`TICK_S`), run on the main thread by `Session::step` every third 60 Hz player step (`STEPS_PER_TICK`). It is a separate clock from the sky's: `[` and `]` scale the sky's seconds in `app.rs` and never reach the simulation, so a 3600x day does not make sand fall faster and a scripted session ticks at the same steps whatever the clock. The block-update queue is a `BTreeSet<(due tick, PosKey)>`; `PosKey` packs (face, i, j, k) so integer order is lexicographic order. Nothing in the module iterates a HashMap. Each tick: drain `World`'s change log (every changed block and its six neighbours are scheduled, `World::neighbours` resolving steps past a face side onto the neighbouring face through `Planet::wrap_cell`; a six-neighbour step is past at most one side, so the corner policy never has to decide), recompute the active region if the player crossed a chunk border, process due updates in order, move the blocks in flight, drain the log again. Rules are driven by block flags (`has_gravity`, `is_replaceable`); water plugs in beside the gravity rule, with `boundary_block` as its hook for cells outside the region (the generator's value). |
| `sim` (active region) | LOD-0 chunks whose bounding sphere comes within `--sim-radius` (112 m) of the player, evaluated when the player's chunk changes (0.86 ms per recompute; at walking speed that is once every seven seconds, at 1 km/s every tick). An update due in an inactive chunk moves to `frozen` under its chunk and re-enters the queue, due now, when that chunk is active again; a block falling there hangs mid-air. Both are saved. Render residency is never consulted. |
| `sim` (falling) | A block with `gravity` whose cell below is `replaceable` leaves the grid (its removal goes through `World::set_block`, which notifies the block above) and becomes a `Falling { id, face, i, j, z, vel }` in its own column, accelerating at the player's 25 m/s² to 70 m/s and landing in the first cell above a non-replaceable one; landings are written as one batch commit per tick, with a column's own landings made visible to the blocks above it in the same tick so a stack lands in order. The chain starts one block per tick after a dig; a column whose every block is notified at once (placed while paused, resumed) starts in a single tick. A block built into the cell a falling block occupies makes it settle in the first free cell above. Falling blocks do not collide with each other or the player; one landing in the player's cell pushes the player up (the existing rule). |
| `sim::draw` | Falling blocks drawn through the terrain pipeline: six quads in `mesh.rs`'s format per block id, allocated once, and a curvature-grid record per block per frame at the block's current radius (`Renderer::add_mesh` with no quads for the slot; last frame's slots are reused after its submit). No renderer or shader changes. A landed block is drawn as a cube for five more ticks while its chunk remeshes. |
| `sim` (state) | `Snapshot` (tick, queue, frozen updates, blocks in flight) with a byte format that is validated on read; `state_hash`, SHA-256 over the tick, the queue, the frozen updates, the blocks in flight and every edited chunk's ids and water levels. |
| `world::edits` | The authoritative store holds `Edited { ids, water levels, revision }` per chunk (review #15). One batch `commit` under one lock feeds the dirty set, and `World::commit` derives mesh invalidations and the simulation's change log from its result (one write path for the player and the simulation). `dirty_snapshot` hands out revisions without clearing; `acknowledge` clears only the revisions a save actually persisted (review #2). |
| `world::save` | Format 3. A save is one generation: every dirty chunk and the simulation state are written to new files (`chunks/*.gN.chunk`, `sim.gN.bin`), fsynced, and `commit.json` (file per chunk, SHA-256 per file) is switched last by rename (review #1; `write_atomic` no longer deletes the destination first). Loading verifies every digest, every key against the world's bounds, duplicate keys and palette ids before installing anything (review #14). Leftovers of an interrupted save are ignored and removed by the next save. The bake identity is a SHA-256 of the rasters' contents, cached beside the bake (`content.sha256.json`) on names, sizes and times (review #4). Version-2 saves are refused. |
| `world` | `affected_chunks` steps a whole voxel of each LOD to find the padding neighbour (review #11: at LOD 1 block 65 is chunk 0's padding and was missed). `water_level_at`, `set_water`, `commit_levels` for M2b. |
| `world::blocks` | `gravity` and `replaceable` flags in `blocks.json` (sand and gravel fall; air and water give way), validated for contradictions at load. |
| `engine`, `app`, `bench` | `hide_water` is a view flag applied at draw time (the water draw list is skipped, the underwater tint off); generation never reads it, F no longer rebuilds the world or the streamer, and the harness keeps its chunks across `water on|off` (review #5). Held clicks repeat every 15 fixed steps instead of by wall clock (review #16). |
| `session`, `main` | `Session` owns the `Sim`; `--sim-radius M`, `--no-sim`; saves write the simulation whenever chunks are written, the state changed or the save already exists. State fields for the harness: `sim_enabled sim_tick falling queued frozen sim_active_chunks falls landings vanished tick_ms_last/avg/max refocus_ms`. |
| `play` | `tick N`, `sim on|off`, `falling_rel[DI,DJ]`, `sim_hash` (computed on demand); shots, bench and travel draw the blocks in flight; the HUD's stats panel shows the simulation line. All in their own hunks (Opus's frame recording lands beside them). |

### Decisions written down

- **The tick is tied to player steps, not time.** 20 Hz = every third 60 Hz
  step. The app's overload cap (eight steps per frame, then the backlog is
  dropped) slows the simulation under load instead of skipping it; a replay
  (M2c) counts steps.
- **Snow does not fall.** It is the surface skin of every cold mountain; with
  gravity, every tunnel through a snowfield would shed its roof. One flag in
  `blocks.json` turns it on if Andrew wants that.
- **Water is replaceable.** A block falling into water takes the cell and the
  water is gone. M2b's levels decide what displacement should do.
- **Stone does not collapse** (not in scope; Andrew's design decision).
- **A version-2 save is refused**, not migrated: it has no commit manifest,
  no digests and no simulation state. Austin's `saves/Neisor` from M1 will be
  refused on the first run after the merge; `--save saves/Neisor-m2` or
  deleting it is the way on.

### Review findings folded in (Astra's second review, forwarded mid-task)

Done: **1** (atomic rename without delete, fsync, generation commit switched
last), **2** (dirty revisions acknowledged after the commit), **4** (bake
content digest, cached), **5** (hide_water at draw time), **11** (voxel-step
halo invalidation), **14** (digests, bounds, duplicates, palette ids),
**15** (ids, water levels and revisions in the store; one batch commit;
activity from player coordinates), **16** in part (held clicks repeat on
fixed steps; the sky clock and integer-positioned inputs are M2c's replay
driver, below). Not touched: **3** (Opus, in `main.rs`'s harness branch) and
the hardening list.

Each has a test: `acknowledging_a_stale_revision_keeps_the_chunk_dirty`,
`the_store_holds_water_levels_and_a_batch_commits_under_one_lock`,
`water_levels_round_trip_exactly`, `an_interrupted_save_leaves_the_previous_generation_whole`,
`out_of_bounds_duplicate_and_unknown_id_chunks_are_refused`,
`missing_save_is_empty_and_damaged_files_are_reported` (a flipped byte fails
its digest), `bake_identity_follows_the_contents_and_caches_the_digest`,
`coarse_halo_invalidation_steps_a_whole_voxel`,
`hiding_water_does_not_change_what_an_edit_makes_authoritative`.

## Evidence

### Tests (`cargo test --release`: 90 passed, 2 ignored)

`M2a-evidence/tests-release.txt`. New this mission, the simulation:
`pos_key_round_trips_and_orders_canonically`,
`snapshot_bytes_round_trip_and_reject_damage`,
`neighbours_cross_chunk_borders_and_face_edges_but_not_the_world_top_or_floor`
(the +i neighbour of face 0's last column is on face 2 and steps back; the
corner block has two neighbours on other faces; layer 0 and the top have
five), `sand_collapses_in_a_chain_and_a_column_drains_in_order` (two stone,
four sand, one gravel; the upper stone dug: blocks in flight go 1, 2, 3, 4, 5
on successive ticks, the first lands in the sixth tick, the column ends one
layer lower in the same order, nothing is queued at rest),
`a_block_built_into_a_falling_blocks_path_makes_it_settle_above_it`,
`updates_outside_the_active_region_wait_and_persist_and_run_when_it_arrives`
(a sand block 400 m away with a 50 m region: seven frozen updates, nothing
moves for ten ticks, the frozen set survives a snapshot round trip, walking
over wakes it and the block falls four layers; a block in flight whose chunk
goes inactive hangs at the same `z`), and the determinism set:

- `the_same_inputs_give_the_same_hash_twice_and_the_scenario_is_not_trivial`:
  a scripted 80-tick session (a seven-block tower on a floating base placed
  so the tower crosses a chunk boundary, a 3 x 3 slab dropped at once, a far
  block left frozen, the base dug at tick 4, a stone built into the fall at
  tick 12) hashes the same twice at tick 20 and at tick 80, the two hashes
  differ, and a 600 m region (the far block falls) hashes differently.
- `the_hash_is_the_same_across_rayon_pools_of_one_and_eight_threads`.
- `the_hash_is_the_same_across_a_save_and_a_reload_in_the_middle`: the same
  session saved at tick 20 (mid-fall) through the real `SaveDir` (chunks,
  water levels and `sim` state), loaded into a fresh store, world and `Sim`,
  hashes equal to the saved state and, after the remaining 60 ticks, equal to
  the straight run.

### The stress test (`cargo test --release -- --ignored stress`)

5,041 sand blocks (71 x 71) placed 20 m above the hills and released at once;
`M2a-evidence/stress-5041.txt`:

| tick | cost | what |
|---|---|---|
| 1 | 67.8 ms | 5,041 changes absorbed, ~35,000 notifications scheduled, 15,000 updates processed, 5,041 blocks removed from the grid |
| 2 | 3.3 ms | the notifications of the removals |
| 3-22 | 0.40-1.30 ms | 5,041 blocks in flight |
| 23-27 | 5.3-10.7 ms | 897 / 1,416 / 1,152 / 784 / 792 landings, one batch commit each |

Average while blocks were in flight 4.5 ms (26 ticks, the spikes included);
every block landed, none vanished. A second run with less going on beside
it: worst tick 38.7 ms, average in flight 2.4 ms (`stress-5041.txt`); the
shape is the same, the absolute numbers moved with the machine. The mass
start is the known spike: each removal is its own `set_block` so the block
above can see the air in the same drain; batching removals per tick would
halve it at the cost of one-tick-per-layer starts (either is deterministic).

### Tick cost at normal activity

From `m2a-sand.play`'s transcript and `final.json` (two towers, digging, a
reload; `M2a-evidence/m2a-sand.log`): per-tick cost **0.001-0.020 ms**,
average over the last 100 ticks **0.023 ms**, maximum **0.87 ms** (the
active-region recompute on a chunk crossing, `refocus_ms 0.859`). The
script asserts `tick_ms_avg < 2`.

### Scripts

`m2a-sand.play` (**95 asserts, 0 failed**; `M2a-evidence/m2a-sand.log`,
`sand_*.jpg`). Scene A: a three-deep shaft in column 107155, a tunnel one
cell along +i into column 107156's two bottom layers, three sand and one
gravel built on 107156's untouched surface block, that block knocked out
from inside the tunnel: blocks in flight 1, 2, 3, 4 on the next four ticks
(`falling_rel[1,0]` from the shaft), four landings, the column drained
three layers into the tunnel as sand, sand, sand, gravel with air above, and
the tunnel full of sand from the shaft's side. Scene B: a deeper hole (seven
layers), the tower built and its support knocked out with `sim off`
(nothing moves for five ticks), `sim on` (all three start in one tick),
`save`, `reload` (three still in flight, the tick counter restored), forty
ticks, and sand, sand, gravel at the bottom with the column open to the sky.
Shots: `01_tower`, `02_collapse_a/b/c` (mid-fall, the gaps opening between
the blocks), `02_collapse_d_landed`, `03_reloaded_midfall`,
`03_reloaded_landed`, `04_both_scenes`.

`m1-done.play`: **94 asserts, 0 failed** (`m1-done.log`).
`edge-corner-flight.play`: **17 asserts, 0 failed**, cover violations 0 on
all three legs (`edge-corner-flight.log`); the simulation ticked throughout
at 1 km/s (a region recompute every tick).

### Bench (PROVISIONAL: Opus's video rendered through both runs)

`cargo run --release -- --bench`, RTX 2060, 1920x1080 offscreen, lod
factor 5, the same world as M1c-stream's table. Two runs, both with
`ffmpeg.exe` (1 GB, every core) and a second engine instance on the machine
(`tasklist` before and after; there was no quiet window during this task).
The bench builds no `Session`, so the simulation never ticks in it; the only
addition on its frame path is `FallingDraws::emit` with an empty list.

Static poses, 120 frames after settling (ms averages, p95 in brackets),
against M1c-stream's idle numbers:

| pose | GPU | CPU (p95) | M1c-stream idle GPU / CPU (p95) |
|---|---|---|---|
| stand_hills | 0.25 | 0.43 (0.59) | 0.25 / 0.38 (0.43) |
| alt_1km | 0.53 | 0.33 (0.36) | 0.54 / 0.34 (0.42) |
| alt_10km | 0.47 | 0.33 (0.43) | 0.25 / 0.26 (0.29) |
| alt_500km | 0.36 | 0.22 (0.24) | 0.25 / 0.24 (0.35) |

The fixed 60 s flight (`bench-m2a.md`, `bench-m2a-loaded.md`):

| run | frames | avg fps | 1 % low fps | worst ms | frame p50 / p99 ms | CPU avg / p99 ms | GPU avg / p99 ms |
|---|---|---|---|---|---|---|---|
| M1c-stream, idle (reference) | 84 621 | 1 422 | 608 | 5.87 | 0.72 / 1.18 | 0.36 / 0.65 | 0.23 / 0.27 |
| M2a, video rendering, run 2 | 49 638 | 837 | 70 | 41.6 | 0.89 / 8.75 | 0.60 / 2.72 | 0.21 / 0.29 |
| M2a, video rendering + a second engine, run 1 | 24 370 | 410 | 48 | 67.3 | 1.69 / 13.78 | 1.02 / 6.61 | 0.42 / 1.34 |

The medians and the GPU time are M1c-stream's; the 1 % low is the
scheduler's under a 16-thread encode and is **not a measurement of this
branch**. Nothing in M2a runs per frame in the flight. Re-run
`cargo run --release -- --bench --bake C:/code/triangulum/game/assets/world/seed42 --out out/bench`
on a quiet machine before reading the low; I could not get one.

`edge-corner-flight.play` re-run under the same load: 2.9 / 6.6 / 3.9 s of
wall time for the three legs (M1c-stream's idle log: 1.9 / 4.0 / 3.4), main
thread avg 0.47 / 0.59 / 0.64 ms (idle log 0.37 / 0.43 / 0.59), cover
violations 0 (`edge-corner-flight-idle.log`).

## Not verified

- Timing on an idle machine: Opus's video (ffmpeg and a second engine) ran
  during both benches and all the flight scripts; `tasklist` showed them
  before and after each run. The static poses and the medians agree with
  the M1c-stream numbers; the flight's 1 % low and the maxima do not mean
  anything here. **The "no regression" flight number is unverified** until
  someone runs the bench on a quiet machine.
- The windowed app with sand: the harness and the tests drive the same
  `Session`, and the app's hooks (draw the blocks, the HUD line, F without
  a rebuild, step-counted repeat) compile and are small, but nobody has
  watched a collapse with the mouse. Austin's play session is the gate.
- A fall across a cube-face edge cannot happen (a column never changes
  face); notifications across one are tested at the lattice level only.
- The bake content hash costs a few seconds the first time (550 MB through
  the crate's SHA-256) and is cached; the cache lives beside the bake, so a
  bake script that cleans its directory recomputes it once.
- `edge-corner-flight`'s third leg took 9.9 s of wall time against 3.4 s in
  M1c-stream's log, with ffmpeg running; the cover checks and the asserts
  are what the script gates. Re-run standalone before reading the maxima.

## What I would change in the M2b / M2c plan

- **M2b (water).** The pieces it needs are in place: `has_gravity`-style
  flags, `World::water_level_at` / `set_water` / `commit_levels` with levels
  stored and saved exactly, the rule slot in `Sim::apply`, and
  `boundary_block` for cells outside the region. Two things to decide
  first: (1) whether a boundary cell reads its edit or the generator
  (`boundary_block` returns the generator's block today); (2) sleeping.
  Andrew's rule only moves water when neighbours differ by two levels, so
  water goes quiet by itself, but a lake of 10,000 cells notified by one
  dig would schedule 10,000 updates; make the rule schedule only cells
  whose level changed, and keep "simulation runs only where something
  changed" as the invariant the determinism hash checks. The mass-start
  spike above says: batch the rule's writes per tick (one commit), and
  accept one-tick-per-cell propagation; water wants that anyway.
- **M2c (replays).** Review #16 is the spec: give every input an integer
  step position and drive the app, the harness and a replay from one
  driver (`Session::step` already is that driver for the simulation; the
  app's click repeat now counts steps, the harness's `hold`/`tap` already
  do). The remaining wall-clock inputs are the first click (its event) and
  the sky clock in the harness (never advanced). The snapshot a replay
  starts from is the save (commit + sim state); the hash is the check.
- **Active region.** 0.86 ms per recompute is fine on foot and a tick's
  budget at 1 km/s; before M4's vehicles, make it incremental (the lattice
  distance is a cheaper first test than `chunk_bounds`).
- **Saves.** Format 3 refuses format 2. If Austin wants his M1 save kept,
  a one-off migrator (chunks are byte-identical; wrap them in a commit)
  is an hour; I did not write it because the task said refuse.

## Feedback

- The `tick N` / `sim on|off` pair made the script's timing claims exact
  ("falling 1, 2, 3, 4 on successive ticks") in a way `wait` could not; the
  ticks-per-layer arithmetic in the script header came straight from the
  unit test. Worth keeping as the pattern for M2b: a script asserts counts
  per tick, a test asserts the physics.
- `falling_rel[DI,DJ]` and the sim fields in `state` were enough; I did not
  need a per-entity dump. If M2b wants levels per cell, `block_rel`'s
  pattern (`level_rel[DI,DJ,DK]`) is the shape.
- The forwarded review landed after the sim was written; folding the store
  and save changes under the sim tests (which exercise save + reload) was
  the right order: the determinism-across-reload test caught the
  `tick`-resume subtlety before the script did.
- Reviewing my own first tick: the `json!` macro has a recursion limit
  (about 25 fields); `Session::state` now builds the sim fields as a second
  object. Worth a line in `viewer/CLAUDE.md`-style notes for `game/`.
- `--out` and `out/` do not exist in a fresh worktree; the harness creates
  its own output directory but a shell redirect into `game/out/` does not.
  Two minutes lost; `mkdir -p game/out` belongs in the worktree checklist
  next to copying the bake.
