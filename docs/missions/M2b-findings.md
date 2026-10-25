# M2b: water (Fable, 2026-10-25)

Branch `worktree-agent-ab33dd7bad8d90ac0`, crate `game/`, from main at
090a86f (M1-hardening merged), then main with M2-hydro merged in and the
generated lakes and rivers integrated (the last section). Evidence in
`docs/missions/M2b-evidence/` (downscaled frames and logs) and, full size,
under `C:\code\triangulum\.claude\worktrees\agent-ab33dd7bad8d90ac0\game\out\`
(gitignored, this machine only). Scripts: `game/scripts/m2b-water.play`,
`m2b-boundary.play` and `m2b-river.play` (new), `probe-shore.play` and
`probe-river.play` (the site probes), and the four gates that must still
pass.

## What was built

| module | what |
|---|---|
| `sim::water` (new) | Andrew's rule, exactly as `DESIGN.md` states it. One byte of level per cell (a cell is water iff its level is 1 or more). Each tick every scheduled air or water cell (1) **falls**: as much as fits into the cell below while it is replaceable and not full, bottom-up so a free-standing column drops one cell per tick as a unit; (2) **spreads**: what is left levels with lateral neighbours at least 2 lower by water-filling (the lowest neighbours are included while each is at or below the running mean; every included cell gets the floor of the mean, the cell keeps the floor, the remainder goes one unit each to the lowest), so after a spread no pair differs by more than 1. A cell with room below never spreads; it falls next tick. Updates run in the M2a queue's order through a tick-local overlay (later cells see earlier writes) and are committed as one batch per tick. A cell a neighbour spread into waits a tick (one cell sideways per tick, no chaining); a cell water fell into spreads in the same tick unless the cell below it also received a fall (the middle of a moving column waits, so columns do not tear). The spread pass alternates direction every tick. Every written cell is scheduled again for the next tick, whether or not its level changed (the middle of a column gives and receives the same amount). Only changed cells and their neighbours are scheduled otherwise, so settled water costs nothing. |
| `sim::water` (settling) | The potential `Phi = sum over cells of (1024 k + L) L` strictly falls on every tick in which any unit moves: a fall of d units changes it by at most `-1024 d + 2 d * 254 < 0`; a spread replaces the cell and its included neighbours by the most even integer distribution of the same total, which has the strictly smallest sum of squares when a pair differed by 2 or more. Phi is a non-negative integer, so there is no cycle and no squirming. The argument is in the module doc and the closed-basin tests check it tick by tick. It is for a closed body: the boundary can supply and absorb, and a river flowing in one side and out the other is a steady flow by design. |
| `sim::water` (boundary) | `Sim::boundary_cell`: a neighbour in an inactive chunk is frozen, never written. Where the generator has water it is a **reservoir** at the level it holds now (its edit's if it has one: a lake stays full and refills the region; a lake the player drained near the edge does not refill itself when the region shifts); a cell beside it 2 or more lower pulls half the difference, a cell below it that is not full fills from it. Generated water that is still exactly as generated and below full (a river cell, once M2-hydro gives it a level) also **drains**: inside cells level down to it. Everything else is a **wall**, including a channel dug past the edge, so a lake cannot drain into frozen nothing, and water the player removed at the edge is a frozen hole, not a sink. `SimStats::water_in` / `water_out` count every unit crossing; `water_placed` / `water_removed` / `water_displaced` / `water_lost` the player's and displacement's; `water_balance()` is their sum, and the tests check the water inside changes by exactly that. `--water-period N` / `sim period N`: water steps every N ticks (Andrew's flow-speed dial). |
| `sim` (dispatch) | `Sim::step` sorts the due active cells by rule: blocks with gravity run the fall rule first (ascending, as before), replaceable cells run the two water passes, then one commit. Landings (`land_into`) and the player's placements go through `World::displace_writes`. Per-phase timing in the stats (`water_rule_us`, `water_commit_us`, `absorb_us`). |
| `world` | `block_and_level_at` (one lock per cell read), `natural_cell` (the generator's cell: what the boundary reads, where M2-hydro's levels arrive), `displace_writes` (a solid block taking a water cell pushes the water up its column into the first cells with room; a solid ceiling loses it, counted), `is_submerged` (level-aware), `padded_levels` (the mesher's 34^3 level grid, None when every water cell in reach is full), `Voxels::water_level` / `submerged`. `commit_levels` dedups mesh invalidations through a set and `affected_chunks_into` is allocation-free: a flood commits thousands of cells a tick, and that was 70% of the tick. |
| `world::blocks` | The default hotbar is the first eight solid blocks and water on key 9. |
| `player`, `session`, `app` | `pick(.., liquids)`: holding water the ray also stops at water cells past the eye's own, so left click removes a cell (units counted) and right click places a full one (topping up a partial one); holding anything else it passes through water as before. A solid block placed into water displaces it. Swimming is level-aware: a film is walked on, a full cell swum in; the underwater tint needs the eye below its cell's surface. |
| `mesh`, `render`, shader | Water quads are 16 bytes: the usual two words, four 8-bit corner heights (corner c = cu + 2 cv, as AO) and a spare word; `mesh_set` is generic over the merge key so full water still merges greedily and partial faces merge only with equal corners. A top face sits at level / 255 with each corner averaged over the water cells sharing that lattice corner (Minecraft's rule: weight 10 at 0.8 and above, air counts as 0, solid left out, water above makes it full), so neighbouring surfaces meet exactly; a side face against air takes the same two top corners (watertight falls and partial columns). `Renderer::add_mesh` gives a water quad two slots; `vs_water` reads slot `quad_base + 2 (q - quad_base)` and lowers the top corners. Full water (the ocean, every coarse LOD) carries 255s and is the geometry it was: the LOD-seam tests pass unchanged. |
| `play` | `level[F,I,J,K]`, `level_rel[DI,DJ,DK]`, `water_rel[box]` (units in a box), the water stats as fields, `sim radius M`, `sim period N`. |
| `README` | A "Water (M2b)" section, the scripts, the tests, the known gaps. |
| M2-hydro integration | `Generated::levels` (the one edit in `worldgen/`): the generator already computes the level of every padded cell, border columns included, and now keeps them at LOD 0 whenever one is partial, so `World::padded_levels` averages corners against the real neighbour across chunk borders (lakes and rivers) instead of assuming 255; edited chunks and edited neighbours still overlay. `Sim::frozen` holds one entry per cell (its earliest due tick) instead of one per (tick, cell): a cell outside the region beside a flowing river is notified every tick, and the old set grew by about 18 entries a tick without bound (7,131 after 400 ticks of a diverted river), all of them saved and all woken at once. |

### Decisions written down

- **Boundary reads the cell as it is (edit first), supplies where the
  generator has water, absorbs only into unchanged generated water below
  full, and is a wall everywhere else.** Two of the task's constraints pull
  against each other: no draining a lake through the edge, and no crashing
  in when the region shifts. Reading the generator alone would refill a
  drained lake from its own frozen cells when the player walks off;
  reading the edit alone would let a channel dug past the edge drain a lake
  into nothing. This split satisfies both, with one honest fiction: a
  generated lake beyond the edge is infinite (it is a lake).
- **Displacement pushes water up.** A block landing or placed in water
  sends the water up its own column into the first cells with room; under
  a solid ceiling it is lost and counted (`water_lost`). Conservation holds
  everywhere else, and the tests check it.
- **The bucket is key 9 and changes what the pick ray stops at.** Water
  is not solid, so digging a lake floor must see through it; placing or
  removing water must target it. The selected block decides.
- **Falls go first and columns fall as units; a cell with room below never
  spreads.** Andrew's "falls straight down first, then spreads sideways".
  A standing column over full water spreads from every cell at once (water
  has no cohesion); a free-falling column keeps its shape until it lands.
- **A free-standing cell at level 1 never moves sideways** (it is 2 below
  nothing), so thin films creep one level per tick per cell and stop at
  slope 1 (below).
- **Generated rivers are held until disturbed** (the question M2-hydro
  forces: under Andrew's rule every step drains the pool above it). Nothing
  schedules a generated cell until a change reaches it through the M2a
  change log, so an untouched river keeps its generated shape, steps and
  falls included, at no cost: this is "stop simulating when laminar". A
  disturbance wakes cells by notification and the waking cascades along
  the reach as pools drain through their steps, inside the active region
  only; at the region's edge the generated river upstream supplies (a
  reservoir at its level) and the generated river downstream absorbs (a
  drain at its level, the case that was injection-only until M2-hydro), so
  the loaded area neither dries up nor floods: water flows in and out, and
  is counted. Rejected: feeding every generated cell an equal inflow (a
  source per cell, bookkeeping for every river cell in the region, and the
  rule's half-the-difference transfers would pile the inflow up at the
  inlet above its generated level), and treating every unchanged generated
  cell as an infinite source inside the region (cheap and keeps the bed
  intact, but a river could then never be diverted: its own cells would
  keep flowing in the old bed forever). The honest cost of the choice: a
  reach the player disturbs drains to its lower pools and settles into a
  trickle sloping about two levels per cell from the upstream edge, fed at
  a few units a tick; edits are authoritative, so the generated profile
  does not come back. Austin and Andrew should see one disturbed river
  before this is final (`m2b-river.play`, below, is that scene).

## Evidence

### Tests (`cargo test --release`: 107 passed, 0 failed, 3 ignored)

`M2b-evidence/tests-release.txt`. New this mission, `sim::water_tests`:

- `one_cell_spreads_evenly_then_the_basin_levels_and_rests`: one full cell
  on a 5 x 5 basin floor becomes exactly 51 on five cells in one tick (the
  diagonals wait a tick), settles in 8 ticks with every floor cell holding
  water within a lean of one, the total exactly 255, the potential lower;
  then twenty idle ticks process no updates, change no chunk, queue nothing
  and cost under half a millisecond each.
- `a_pour_falls_as_a_column_then_spreads_and_the_potential_strictly_falls`:
  four cells in the air drop one cell per tick as a unit (no sideways
  spread while falling), the bottom one spreads to 51 the tick it lands
  while the three above wait with room below, and the pour settles in 19
  ticks into one layer at a lean of at most one, conserved; the potential
  strictly fell on every moving tick.
- `a_full_basin_rests_until_touched_and_a_lake_drains_through_a_channel_to_its_sill`:
  a brimful 5 x 5 x 3 lake is notified once and never moves (no updates,
  no chunk changes); opening one wall cell at the surface layer drains that
  layer into a lower basin in 604 ticks, the two layers below the sill stay
  full, the surface keeps a film that leans one level per cell up from the
  sill (7 at the far corner), the sill holds at most 1, and the lower basin
  holds the rest in one layer over all 81 cells, conserved.
- `a_falling_block_displaces_water_upward_and_a_sealed_pool_loses_it`:
  sand dropped into a two-layer pool lands on the floor, 255 units are
  pushed up and spread over a third layer, nothing lost; under a stone lid
  the displacement reports 255 lost; in the open pool the same placement
  pushes past the full second layer into the third and the air above.
- `a_pit_dug_at_the_shore_fills_from_the_sea_and_the_sea_stays_full`: an
  untouched sea processes no updates; a 3 x 3 x 2 pit dug beside the sea
  surface fills (floor full, surface over 200) in 122 ticks, the water
  inside the region changed by exactly `in - out` (both 0: the region is
  112 m and the dish never reaches its edge), and the sea beside the pit is
  a dish exactly one level per metre deep, 237 at the pit and 255 again
  19 m out, full at 40 m and beyond.
- `boundary_cells_read_as_reservoirs_walls_or_drains_and_are_never_written`:
  rock and air are walls, the generated sea is a reservoir at 255 that does
  not drain, a cell the player drained to 100 supplies at 100, a cell the
  player emptied is a wall. With a one-chunk region and injected river
  cells at level 10 along one side, a 3 x 3 x 2 pool drains to 11 beside
  them in 56 ticks, exactly the counted `water_out` leaves, and the frozen
  cells are byte-identical afterwards; injected reservoirs at 200 refill
  the pool to 198..199 in 52 ticks with `water_in` counted and nothing
  absorbed; injected walls move nothing.
- `water_waits_for_its_period_and_the_period_is_in_the_hash`: with period
  4 a placed cell waits for tick 4, moves, and does not move between steps.
- `sloped_water_surfaces_share_corners_and_are_watertight`: every lattice
  corner of the plus-shaped pour carries one height whichever face names
  it (top or side), the formula matches at two corners (38 and 13), and a
  chunk of full water gets no level grid at all.
- `tests::standing_in_a_thin_film_is_not_swimming` (player): a 2/255 film
  is walked on, a full cell swum in.

The M2a determinism set now runs with water in its script (three cells
poured from the air at tick 2, sand dropped into the spread and a stone
built into it at tick 9): the same hash twice at tick 20 and 80, across
rayon pools of 1 and 8 threads, and across a save and reload at tick 20
mid-fall and mid-flow.

### Scripts

`m2b-water.play` (**97 asserts, 0 failed**; `M2b-evidence/m2b-water.log`,
`water_0*.jpg`). Site A: a two-deep hole at +5 and a trench from +1 to +4;
one cell of water placed at the near end splits 127 / 128 in one tick,
then 127 / 64 / 64, reaches the hole's floor by tick 8, and settles within
80 ticks with the hole holding at least 225, the trench a film of at most 6,
the total 255 at every check (`water_rel` over the trench's box); two more
cells poured onto the film from above fill the hole to the brim and leave
the trench and the hole's upper cell at about 100 each (a lean of one),
765 units in a generous box, `water_balance` 765. Site B: a 3 x 3 x 2 pit
dug beside the sea surface fills in under 200 ticks (floor 255, surface
over 200), the sea cell beside it stays over 230, the sea 20 m and 43 m
out stays 255, nothing crossed the boundary; a channel dug inland under
the player's feet floods to its floor and the sea 43 m out is still 255.
Average tick cost under 2 ms, 60-frame bench at the end. Frames:
`01_slope` (a partial cell sloping in the trench), `02_drop` (the flow
reaching the hole), `03_fall_into_the_hole` (the second pour, with the
film a cell poured onto a full cell makes on the grass), `04`/`05` (the
pit filling and full), `06_channel`.

`m2b-boundary.play` (**44 asserts, 0 failed**;
`M2b-evidence/m2b-boundary.log`, `water_07_*.jpg`). With `sim radius 1`:
the same trench, one cell poured, two ticks of flow (127 / 64 / 64), the
player flies 300 m away at the third tick: the queue empties into `frozen`,
forty ticks pass, and with the simulation paused the levels back at the
site are exactly 127 / 64 / 64 / 0 and the hole empty. `sim on`: the frozen
updates wake in one tick and the flow finishes (hole over 225, film at
most 6, 255 conserved, nothing crossed the boundary). Then the shore pit
with the region one chunk wide: the pit fills, `water_in` is positive (the
sea beyond the chunk's -i side is the generator's reservoir), the frozen
sea cells at i 1791 and 1790 are still exactly 255 and the cell just inside
leans on them at 250 or more.

The gates: `m1-done.play` **94 asserts, 0 failed**, `m2a-sand.play` **95, 0 failed**,
`edge-corner-flight.play` **17, 0 failed**, cover violations 0 on all three legs, `tunnel-wall-reload.play` **18, 0 failed**
(`M2b-evidence/*.log`).

### Tick cost

Normal activity (the scripts' transcripts): idle ticks 0.000-0.001 ms;
the trench pour 0.01-0.15 ms; the shore pit's flood averages under 0.1 ms
with the first tick after the dig (the sea's 18 cells of pit filling at
once, hundreds of notifications) the maximum at 7.96 ms. Both scripts
assert `tick_ms_avg < 2`.

The stress test (`cargo test --release -- --ignored water_stress --nocapture`,
run alone; `M2b-evidence/water-stress.txt`):

| case | ticks to settle | worst tick | average moving tick (rule / commit / absorb) |
|---|---|---|---|
| a 2,000-cell pour (20 x 20 x 5 released five layers over a 40 x 40 floor) | 619 (31 s) | 5.97 ms | 3.30 (1.16 / 0.79 / 0.94) ms |
| a 10,000-cell dam break (50 x 20 x 10 into an equal empty compartment) | not settled after 4,000 (200 s) | 18.9 ms | 5.05 (1.95 / 1.12 / 1.43) ms |

(A second run on the merged tree, with the machine quieter: the pour 3.03
ms average and 4.34 worst, the dam break 4.71 average and 6.44 worst, the
same 65,472,480 units moved: the simulation is deterministic across the
merge. The table keeps the worse run.)

The dam break's one-time notification of 10,000 placed cells costs 16.4 ms
(a generated lake is never "placed", so this is the cost of building one by
hand). After it the tick is 4,000-7,000 water updates at about 0.25 us each
in the rule itself; the commit (edit store, mesh invalidations) and the
change-log drain that schedules the next tick are each about as much as
the rule. **That is over the 2 ms budget while such a body moves**: a
10,000-cell dam break holds the 20 Hz tick at about 5 ms (worst 19 ms) on
the main thread, which a 60 fps frame absorbs (one frame in three carries
it) but a 144 Hz one does not; `--water-period 2` halves it.

The honest finding is how long big bodies take to settle: the rule is a
local averaging, which is diffusion, so the settling time grows with the
square of the body's width (the 7 x 7 pour settles in 19 ticks, the 40 x 40
pour in 619 (31 s), the 100-long dam break was still moving after 4,000 ticks, its units moved per tick falling from 50,000 to about 14,000 as the slosh between the two halves damps). A real dam
break would be waves and a few seconds; here the last levels creep across
the box one unit per tick per cell.

### Frames

`water_01_slope.jpg` to `water_07_*.jpg` in `M2b-evidence/`: the sloped
surface along the trench, the drop into the hole, the pit and the channel
at the shore. The flat sea is pixel-identical geometry to before (255
corners), which the unchanged LOD-seam tests also say.

### On the generated world (main with M2-hydro merged)

- `cargo test --release`: **111 passed, 0 failed, 10 ignored** (the hydro gates
  `water_is_contained` and `water_agrees_across_lods` included; the M2b
  tests unchanged).
- `m2b-water.play` **97 / 0** and `m2b-boundary.play` **44 / 0** pass
  unchanged with lakes and rivers in the world (their sites hold no
  generated water but the sea).
- `m2b-river.play` (**41 asserts, 0 failed**; `M2b-evidence/m2b-river.log`,
  `water_08_*.jpg`, `water_09_*.jpg`): the small river above the tallest
  fall (Opus's `waterfall_ground` site, link 1728, 173 m³/s; face 3, two
  cells deep, gravel bed at k 510, partial top cell at k 512 at levels
  84-89). With the region one chunk wide, forty ticks process **no update at
  all**: a generated river is held until disturbed. A trench four cells
  long is dug from the bank down to below the bed; the river's edge cell
  spreads into it the next tick and the trench floor is wet three ticks
  later. After 400 ticks the trench is full to its floor layer and the river
  inside the chunk is still nearly full beside the trench, across the
  channel and along the reach (bottom layer 200 or more), while the frozen
  river beyond the chunk's downstream side is exactly as generated (255).
  **Water crossed the boundary both ways: 47,556 units in from the
  generated river upstream and 44,896 out into the generated river
  downstream** (the drain case, injection-only before the merge, for real),
  a steady flow of about 115 units a tick at 0.56 ms a tick
  that does not stop: the river runs through the loaded area. That is M2's
  "done when" for rivers: diverting one does not dry up the loaded area.

## For Andrew to decide

1. **Flow speed.** One cell per tick (20 cells a second sideways and down)
   is the default; `--water-period N` slows it to one cell per N ticks.
   Water also moves half the difference at a time, so a full cell beside an
   empty trench fills it to half in one tick.
2. **The resting lean.** The rule stops at a difference of 1 per cell, and
   the dynamics leave exactly that: a pit beside the sea leaves a dish one
   level (4 mm) per metre deep, 7 cm at the pit and 19 m across, that never
   flattens; a drained lake keeps a film sloping up from the sill. The
   sloped mesh draws it faithfully. If a flat rest is wanted, the rule needs
   a second, rarer relaxation (for instance a periodic pass that moves one
   unit across a difference of 1 when it lowers the sum of squares over a
   whole row), which must be designed not to oscillate.
3. **Films.** A single placed cell on flat ground spreads into up to 255
   cells at level 1 (4 mm): a bucket emptied on grass makes a wide
   translucent sheet (`water_03`). There is no evaporation and no minimum
   visible level. Minecraft stops a source at 7 blocks; this rule, as
   written, does not stop.
4. **Settling time of large bodies** (above): acceptable for lakes and
   pits, slow for a 100 m dam break. Larger per-tick transfers or a
   pressure term would change the look of flow; this is a design choice.
5. **The surface look**: Minecraft's corner averaging (weights 10 and 1,
   air as 0). A full cell beside air dips to 0.91 at that edge; a lone
   placed cell tapers. Easy to retune in `mesh.rs` `Surface::corner`.
6. **What a disturbed river becomes** (the generated-river decision above,
   and `m2b-river.play`): a reach the player touches drains to its lower
   pools and keeps flowing as a trickle fed from the region's edge; its
   steps and falls, once notified, dissolve. The alternative that keeps the
   generated bed intact makes rivers undivertable.

## Not verified

- A generated waterfall's curtain and the inside of a tight bend are not
  at rest by the rule (Opus's designated drops); the river script's reach
  has neither inside its chunk, so what a disturbance does to a fall (the
  curtain would spread into the gorge's air once notified) has not been
  watched. It will dissolve into a sheet on the cliff; whether that is
  acceptable is part of decision 6 below.
- The windowed app with the bucket: the harness and the tests drive the
  same `Session` (pick, place, break, displacement, the HUD line), and the
  app's one change (the underwater tint through `is_submerged`) compiles,
  but nobody has poured water with the mouse. Austin's session is the gate.
- Timing: the scripts and the stress test ran alone on this machine
  (`tasklist` showed no other engine), but the numbers are one run each.
- A fall or spread across a cube-face edge is handled by `resolve_block`
  (the same path M2a's notifications use) and not separately tested.
- The 16-byte water quads double water's share of the quad buffer; the
  flat sea is few quads and no allocation failure appeared in any run, but
  the arena's water share was not measured.

## What I would change next

- **`absorb` and the queue.** With the commit fixed, the change-log drain
  (a `Vec` of six neighbours and seven `BTreeSet` inserts per changed cell)
  is a third of a flood tick. A "due next tick" vector sorted once at the
  drain, with the `BTreeSet` kept for later-due entries, would halve it and
  keep the order.
- **Reads.** `block_and_level_at` takes the store's read lock per cell;
  a flood reads eight cells per update. Snapshotting the touched chunks
  once per tick when the due list is large would make the rule itself
  nearly free.
- **M2c** gets `water_period` and the direction alternation in the hash
  for free (both are functions of the tick).

## Feedback

- The `--ignored` stress test with `--nocapture` and the per-phase timers
  in `SimStats` found the real cost in one run (the commit's per-block
  `affected_chunks` allocations were 70% of a flood tick); the fix was ten
  lines. Worth keeping the phase timers on the HUD line for the next rule.
- Two bugs the tests caught that a script would not have: `nl + 2 <=
  level` on a `u8` wraps in release (254 + 2 = 0), which made neighbouring
  254/255 cells swap a unit forever (the potential check did not catch it
  because a swap keeps the sum of squares; the "settles" cap did); and a
  cell that gave and received the same amount in one tick was never
  rescheduled, tearing a falling column. **Release-mode overflow is silent;
  `cargo test` runs in release here.** A debug-build run of the water tests
  before merging would have panicked at the first; worth a line in
  `game/README.md` for the next rule.
- `level_rel` / `water_rel` plus `tick N` made the scripts' claims exact
  (127 / 128, 64 / 64) the way `falling_rel` did for sand. The one
  harness quirk worth knowing: holding water, the pick ray stops at the
  film under your own feet, so pour from a layer up or on dry ground.
- The shore site (face 1, i 1799/1800, j 59003, k 511) came from a test
  that prints it; `probe-shore.play` checks the columns. A `find` command
  in the harness ("find shore|flat|slope") would save the next script a
  probe cycle.
