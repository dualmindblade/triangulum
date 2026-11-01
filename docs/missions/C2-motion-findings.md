# C2: things that move look like they move (Fable, 2026-10-31)

Andrew's #5 and #9 from the 2026-10-31 round. Branch
`worktree-agent-aa8453bb1bd724059`. Evidence in `docs/missions/C2-motion-evidence/`.
`game/README.md` has the player's side ("What you see of it", "Water that does
not blink"). Sim rules are **4** (the water changed; the powder rule did not).

## For Andrew: what to try

1. **Place a block of sand or snow on a slope.** It should move sideways out of
   its cell, drop as it goes, and fall, as one block. There should never be
   two of it, and a landed block should never blink out and back.
   `slide-plate-edge.png` is this, frame by frame, before and after.
2. **Knock a wall out from behind a sand cliff** or drop a cube of sand on a
   plate: the whole slope pours, each block sliding over its edge
   (`c2-slide-C.mp4`, a heap over a cube corner; `c2-slide-E.mp4`, 5,760 blocks).
3. **Pour a bucket on a terrace near a step.** The puddle spreads, the step
   takes it, and the wet sheet left behind dries in patches over ten to thirty
   seconds. Nothing appears and disappears in quick succession. The stats line
   now ends with `N flickers`: it should stay 0 (`puddle-ledge.jpg`).
4. **Pour a bucket and look at its edge from close by and from far away.** The
   thin edge is a sheet 4 mm thick, drawn as a faint tint, never a shimmer or
   a pair of wedges (`film-stone.jpg`, `film-sand-border-edge.jpg`,
   `film-andrews-pose.jpg`).
5. **Pour at the lip of a cliff.** The lip cell stays dry; the stream starts
   one cell below it. (Before, the lip blinked with every unit.)

Numbers you can change: `SLIDE_TICKS` in `sim/mod.rs` (4: the slide takes
0.2 s); `FLICKER_FLIPS` and `FLICKER_WINDOW` there (3 within 20 ticks: what
counts as a blink); `LINE_CELLS` in `sim/water.rs` (48: how far thicker water
looks through a sheet for a drop); `FILM_DEPTH` and `FILM_OPACITY` in
`render/shader.wgsl` (water thinner than 0.12 of a voxel fades to 30% of the
look's opacity).

## #5, the slide: the design and why

**What the rule does** (B4, unchanged): a block with a free cell beside it and
a free cell below that one becomes a `Falling` in the next column at its own
height, in one tick, and falls from there.

**What the renderer now draws.** Every block in flight remembers the cell it
left (`Falling::from`, a `Trail`), and `Sim::view` hands the renderer a
`Drawn` per block that is continuous in time:

- *Between ticks.* The session tells the simulation how far its 60 Hz steps
  are through the current tick (`Sim::set_phase`), and a block is drawn part
  way from where the last tick left it to where this one did. The fall itself
  stepped at 20 Hz before; it is smooth now too.
- *The slide.* For `SLIDE_TICKS` (4 ticks, 0.2 s) after a slide the block is
  drawn on the straight way from the centre of the cell it left to the centre
  of its own column, eased out (`1 - (1 - t)^2`): it clears the edge quickly
  and the fall takes over. At one tick it is 44% of the way over and has
  dropped nothing yet (the drawn fall lags the logical one by a tick); at two
  ticks 75% over and 6 cm down, so the cube never sinks visibly into the ledge
  it is leaving; by four ticks it is in its own column and 0.375 m down. The
  earliest landing is the sixth tick (a drop of one layer), so a slide always
  finishes before its block lands.
- *Edges and corners.* The way is a chord in world space between the two cell
  centres (`Planet::pos`), so it is the same across a chunk border, a cube
  edge and a cube corner, where the two cells are on different faces; the
  cube keeps its own column's frame throughout, which differs from the old
  cell's by about a hundred-thousandth of a radian. Tested in
  `a_slide_over_a_cube_edge_names_the_cell_on_the_other_face_and_its_way_is_a_block_long`
  (the way is level and 0.8 to 1.4 m long: cell centres across an edge are
  not exactly a metre apart) and scene B of `c2-slide.play`
  (`slide-cube-edge.png`); scene C is a heap over a cube corner on video.

**Why purely visual, and why it is honest.** Nothing the rule does changed:
same cells, same ticks, same landings. The trail is neither hashed
(`state_hash`) nor in a replay's digest (`state_digest`), and no rule reads it:
`the_trail_is_look_only_and_not_in_the_hash_or_the_digest` clears it and gets
the same hash and digest. Andrew's pile recording (`192516`) reproduced
exactly on this branch before the water bump ("5 of 5 checks matched; the end
state hash matches", `replay-192516-motion.log`), with the new draw on
(`replay-192516-c2-draw.mp4`). A save carries the cell and the age (snapshot
version 5; versions 1 to 4 read as before, without trails), so a reload
mid-slide draws on from where it was: `a_reload_mid_slide_draws_on`, and
scene D of the script (save, reload, the next frames, the landing). What a
save does not carry is the chunk revision (next section): after a reload
every mesh is rebuilt, so it counts as current.

Making the block really occupy the path would have needed a fractional
position across two columns, a sweep against both columns' cells and a rule
change for every slide across a face edge, and would have changed timing,
saves and replays, for a picture the interpolation gives exactly.

## #5, the second copy: found, and what it was

Andrew's note in the photo's sidecar says it: "snow stays in place at its prior
location for a brief moment before disappearing while a falling block has
already appeared at its new location". That is mesh latency. `World::commit`
queues the chunk for a remesh; `stream.rs` builds it on a worker and installs
it a frame or more later. Meanwhile the chunk's mesh still shows the block in
the cell it left and the entity is drawn in the next column: two copies, for
as long as the mesh is late. The straight fall hides it (the entity starts in
the same cell and has dropped 6 cm after a tick); the slide, which moves a
whole cell in one tick, does not. `slide-plate-edge.png` shows it in the
frozen build: frame 3 is the meshed block and the entity side by side as one
stretched shape, frame 4 the jump. `ghosts` was the other half of the same
problem, solved by a timer: a landed block was drawn as a cube for 5 ticks
while its chunk was remeshed, so a later mesh meant a gap and an earlier one
meant the same cube twice, differently shaded.

**Measured frame by frame.** Every job the streamer dispatches records the
chunk's edit revision it was built from (`JobResult::erev`), and installing it
writes that into `Renderer::meshed_at`. Every `Drawn` names the write it is
waiting on: the cell it left (or took) and that chunk's revision once the
write was in. `sim/draw.rs` can therefore say, per block per frame, whether
the mesh has caught up. In the harness, playing Andrew's recording with a
frame every step, the mesh was behind a block only for the one frame after
the tick in which it moved: `motion over 1124 frames: doubles max 1 (blocks
held instead), held max 1, ghosts max 1, gaps max 0, mesh at most 0.00 ticks
behind a block`. The same in every scene of `c2-slide.play`: `stale_max`
0.33 ticks, one frame. In the game, where frames come at vsync and the worker
competes with the player, it is one to a few frames; the same code handles
any number, but I could not watch it (no window may be opened this round).

**The fix, both halves.** `sim/draw.rs` holds a block in flight exactly where
the mesh still shows it, at the cell it left, until the mesh drops it, and
starts its slide clock then, so nothing jumps; a landed block is drawn as a
cube until the mesh shows it and not a frame longer (`GHOST_TICKS_MAX`, 40
ticks, bounds how long the simulation keeps offering it). Where the renderer
cannot tell (no LOD-0 mesh of that chunk, or a reload) it does what it did
before: on its way at once, a cube for `GHOST_TICKS`. `place()` is the whole
decision and is unit-tested without a GPU. The harness counts what this held
back as `motion_*`; `motion_gaps_max == 0` holds in every scene.

**Andrew's "1 falling at the end"** of `192516` is not a bug: the state sidecar
now lists the blocks (`falling_blocks`): a snow block two ticks into its fall
after its seventh slide down the pile, with a chain of landings every six
ticks behind it. He stopped recording while it was still going.

**Cost of the look** (`c2-slide.play` E: a 24 x 24 x 10 block of sand, 5,760
blocks, dropped on a plate; provisional, four other agents were running).
Ticks: 0.2 ms average, 25 to 44 ms for the one tick that starts all 5,760
falling (B4 measured 34.8 ms for 5,041). The draw (`emit`): **4.5 ms a frame
on average over the 8 s, 58 ms at worst**, the worst being the first frame,
when all 5,760 blocks are in flight and held where the mesh still shows them
(`motion_held_max` 5,760); the most blocks drawn in one written frame was
8,893 (in flight plus landed and waiting for their mesh). Nearly all of that
time is per-block work that predates C2 (a curvature-grid record of 25
`Planet::dir` calls and a grid slot per block per frame, about 10 µs a
block); C2 adds a hash lookup and, for a sliding block, two `Planet::pos`
calls. The trail is 24 bytes per block in flight, the hold map one entry per
block drawn, `meshed_at` one entry per resident LOD-0 chunk. A heap over a
cube corner (scene C, 128 slides, 99 blocks at once) cost at most 8.6 ms a
frame in the draw and 0.23 ms on average; one block (scene A) 0.03 ms. So a
whole heap avalanching costs nothing noticeable; a 5,760-block wall dropped
at once costs a frame or two, as it did before.

## #9, the flicker: measured, then fixed (rules 4)

**Measured first, on rules 3.** `--water-trace` logs every cell that goes from
dry to wet or back, tick by tick (the edit store logs them whoever wrote them,
`EditStore::take_flips`), and `SimStats::water_flickers` counts cells on a
floor that flip three times or more within a second. Replay `192820` (nine
buckets poured on a terrace): **4,686 flips over 1,174 cells; 351 cells flipped
three or more times within 20 ticks; 2,345 of the gaps between flips were one
tick**; the cells were wetted to level 1 (1,449 times) or 2 (300), and a
typical one reads

```
tick 3890: 0->1  3891: 1->0  3896: 0->1  3897: 1->0  3898: 0->1  3899: 1->0
3900: 0->1  3901: 1->0  3902: 0->1 ...   (20 flips in 20 ticks)
```

(`replay-192820-flips-rules3.txt`, `replay-192820-one-cell-rules3.txt`;
`game/scripts/flips.py` makes the analysis from any run.log with the trace
on). A 10 Hz blink over hundreds of cells for about a second each: Andrew's
"appearing and disappearing in rapid succession for a moment before finally
settling". Replay `192845` has no flicker at all (0 cells), so the lone cell
is a separate matter. Three causes, all in `sim/water.rs`, in order of weight:

1. **The seek pump.** A level-1 cell with a drop within `--film-reach` gave its
   unit to the drop (1->0); the thicker cell behind it spread into the dry
   cell the next tick (0->1); and again. Every unit of the puddle crossed the
   sheet this way, one cell blinking per unit per tick.
2. **The frontier.** The spread gave the remainder to the neighbours: a cell of
   level 2 with two dry neighbours wetted them with one each and went dry
   itself (2->0), then was wetted again from behind (`0>2 2>0 0>1 1>0` in
   the trace).
3. **The lip.** A cell of level 2 or more beside the lip of a drop spread onto
   the lip, and what landed there fell the next tick: the lip went 0->1->0
   with every unit that passed.

**The rule now.** (a) A sheet of film is one thing: a level-1 cell with water
beside it neither runs nor goes over a ledge; only a lone film cell does.
Thicker water drains *through* the sheet: it looks along the same four lines,
which pass through film cells without spending reach (`LINE_CELLS` = 48 bounds
the walk), and moves half its level into the drop; the sheet stays wet while
it does and dries where it lies afterwards. (b) The remainder of a spread goes
to the spreading cell first. (c) Nothing spreads onto a cell over a drop; the
slide sends water straight past it into the cell below, so the lip stays dry.
The potential argument is untouched: (a) and (c) are falls from layer k to
k - 1, (b) is as even a distribution as before.
`the_potential_falls_on_every_tick_under_every_rule_on_random_stairs` and the
`settle` checks in every film test stay green. I tried two weaker versions
first (a film cell holds only beside *thicker* water; the line spends reach
on film cells) and measured them with the same trace: the first left the
last cell before a ledge blinking (it went over, was refilled from behind),
the second left the puddle's middle out of reach of the drop and evaporating.

**Proof it was the cause and is gone.**
`a_puddle_drains_to_a_ledge_through_its_film_without_blinking` pours a block
four cells from a cliff with a walled basin below. Against the rules-3
`water.rs` (main's, swapped in for one run) it settles with **64 flickers**
(and the lip test with 20); on this branch **0**, with 702 flips, 169 of the
255 units in the basin at once and the rest dried on the table
(`flicker-test-before-after.txt`). `c2-film.play` scene A is the
same scene with the trace on, under the real drying time: **0 flickers** after
15 s and after 75 s, 163 units in the basin, water balance 0; the frozen build
on the same script shows the holes in its sheet at 1 s (`puddle-ledge.jpg`).
`nothing_spreads_onto_a_cell_over_a_drop` checks the lip.

**What it changes elsewhere.** Thin water near a ledge now evaporates rather
than running over one unit at a time: on the B4 shelf test
(`water_on_a_shelf_runs_over_the_ledge...`) a start of 2 or 3 units leaves a
film of 1 on two cells that dries, where before everything went over; starts
of 7, 40 and 255 go over as before but for the sheet they leave. The frozen
build drains the whole bucket of scene A over the ledge in 3 s (blinking);
this branch sends 163 units over and dries the rest where it lies over about
30 s, so **a wet patch lingers where a puddle was**. A waterfall's lip cell is
dry and the stream starts a cell lower. The B4 rule table
(`film_rules_on_the_three_scenes`, drying time 2 s; B4's numbers in
brackets; `film-rules-table-rules4.txt`):

| rule | plateau: left | lake: cells / units left | lake: dried | poured: left | poured: dried |
|---|---|---|---|---|---|
| keep | all 25 | 216 / 580 (204 / 984) | 0 | 255 units in 101 cells (114) | 0 |
| seek | all 25 | 216 / 580 (144 / 364) | 0 | 255 units | 0 |
| merge | 7 beads | 204 / 1,421 (204 / 1,333) | 0 | 55 cells | 0 |
| dry | 0 | 0 / 0 | 719 (992) | 0 | 255 |
| seek+dry | 0 | 0 / 0 | 600 (373) | 0 | 255 |

The drained terrace keeps less behind under the new spread (580 units where
984 stayed), so there is less to dry or run; `seek` alone no longer pumps the
sheet to the drop, so it keeps what `keep` keeps; and under the default more
of what is left dries (600) and less runs off than B4's 62%. Falling streams of one-unit packets still light up one cell
per tick as they pass; the metric leaves cells with an open floor out, and the
look of a dribble is as it was (not this mission's).

**Saves and replays.** `RULES_VERSION` is 4. Saves from rules 1 to 3 continue
with the usual note (same stored meanings). A replay recorded under rules 3 is
refused: "simulation rules version 3 vs 4: it was recorded before water
learned to drain through a film without blinking (C2) (... to watch this
recording, use a build of the game with simulation rules 3)"
(`real-recording-refused.log`). The three recordings of this round are
therefore unplayable on this branch; their rules-3 measurements above were
taken before the bump.

## #9, the lone cell

**Level 1, not 0.** Replay `192845` ends with the crosshair on a water cell at
face 2, i 107216, j 57575, k 558, level 1 (the end shot's sidecar). A level-0
water cell could not come from the simulation (`Sim::write` stores air for 0)
or from the bucket (full), but the one write path everything uses accepted a
liquid at level 0 from the harness's `fill ... water 0` and from any future
caller; `World::commit_levels` now turns a liquid at level 0 into air
(`a_liquid_written_at_level_0_is_air`). Saves are not scanned for old ones;
none can be made now.

**Why it fought the ground.** `Surface::corner` in `mesh.rs` sets the height of
the water surface at a lattice corner to the mean over the cells round it:
water cells by their level, air as 0. A lone level-1 cell has three airs at
every corner, so (1 + 2) / 4 rounds to **0**: its top face lay exactly in the
floor's plane, and the depth test (reversed Z, water tested against the
terrain's depth) cannot tell two coincident planes apart, so each pixel went to
whichever triangle's interpolated depth came out larger: the dithered wedges
in Andrew's photo. A corner with one solid cell beside it, (1 + 1) / 3, is 0
too; with two, (1 + 1) / 2 is 1, which is why some corners stood up and the
slivers were triangles. The frozen build draws it as a sliver from his pose
and as nothing at all on a flat plate from 2 m (`film-andrews-pose.jpg`,
`film-stone.jpg`, left columns).

**The fix, and the look.** A corner of water now stands at least one level,
4 mm, over the floor (`a_film_cells_surface_stands_above_the_floor_at_every_corner`;
it failed before the one-line change). 4 mm is far above what the depth
buffer can confuse at any distance a 1 m cell is still visible from (a float
reversed-Z depth resolves about 1e-7 of the distance: 10 µm at 100 m). That
alone made every film as opaque as a lake, which is what B4 found Andrew
disliked about the wedge, and would have been worse now that sheets hold:
so the water shader fades thin water (`FILM_DEPTH`, `FILM_OPACITY`: below
0.12 of a voxel it falls to 30% of the look's opacity; a puddle's middle at
level 30 is full). Frames at 2, 8, 50 and 200 m on stone and sand, across a
chunk border and over a cube face edge, and at Andrew's own pose
(`film-*.jpg`): a faint blue square at 2 and 8 m, a dot at 50 m, nothing at
200 m where a cell is under a pixel; no shimmer and no wedges in any of them.

## Verification

- `cargo test --release`: **373 passed, 1 failed, 25 ignored** (B4's
  baseline 194 / 0 / 16; the suite has grown since). The one failure,
  `annotate::tests::a_pass_stays_quick_with_a_lot_drawn`, is a wall-clock
  limit ("timing here shares the machine") and passed when run alone
  (19.8 ms worst against 100). New tests: `motion_tests.rs` (6), the two
  film tests, `place()` in `draw.rs`, the flip log in `edits.rs`, the corner
  in `mesh.rs`, level 0 in `world/mod.rs`.
- The rules-3 proof above; the 4 B4/B7 tests whose expectations changed are
  listed under "What changed".

Scripts, all on this branch's build (`--bake` the main checkout's seed42;
`scripts-first-run.txt` is the sweep, the rest reruns after the fixes below):

| script | asserts | failed |
|---|---|---|
| m1-done | 94 | 0 (103 fps, other agents running) |
| m2a-sand | 95 | 0 |
| m2b-water | 97 | 0, after 6 exact levels moved to rules 4 (below) |
| m2b-boundary | 44 | 0, after 5 exact levels moved to rules 4 |
| m2b-river | 42 | 0 |
| m2c-replay | 29 | 0 |
| b4-slide | 49 | 0 |
| b4-film | 10 | 0 |
| b4-oldsave-make and b4-oldsave-load | not run as meant: the make needs a build from before B4, which this machine no longer has (on 8134c61 its own tower falls); loading the 8134c61 save on this branch printed the continuation note, "save simulation rules 3 continue under rules 4: pending work uses the current rules; resting cells stay at rest until disturbed" |
| b7-compare | 34 | 0 |
| b7-close (`--powder-angle 35`, as its header says) | 11 | 0 |
| b7-walk | 24 | 0 |
| edge-corner-flight | 17 | 0 |
| tunnel-wall-reload | 18 | 0 |
| c2-slide (new) | 31 | 0 |
| c2-film (new) | 10 | 0 |

`b7-close` without its `--powder-angle` fails `falling == 0` (nine partial
cells left falling when the script teleports away) on the frozen build too:
not this branch's. Its shore scene, run under the default rules with the trace
on (a 7 x 7 x 12 column of sand dropped into a four-deep pool), counts 398
flickers: water shoved up through the column of landing blocks, each cell
filled by displacement and taken by the next block within a tick
(`13:0>255 13:255>0`). That is M2b's displacement doing what it should in a
violent scene, not the spreading Andrew described, but the metric cannot tell
a landing's write from the rule's, so it counts it: read `water_flickers`
with that in mind when blocks are landing in water. The `m2b` levels that moved: a full cell poured at the
near end of a four-cell trench with a pit at its end used to split 127 / 128
and then 127 / 64 / 64 and the pit got water only later; now the spreading
cell keeps the remainder (128 / 127) and in the second tick the near cell
cannot spread (its neighbour is one lower) so it looks along the trench and
sends half, 64, straight into the pit (64 / 64 / 63, pit 64); the lip cell
over the pit stays dry (one assert looked at the lip from a raised foothold
and passed by accident; it now looks at the pit); and 20 cells from a dug
shore pit the lake's surface rests at 253 rather than exactly 255.

`c2-film.play` and `c2-slide.play` were also run on the frozen 8134c61 build
(`--out ...-before`) for the left columns of every sheet; the only failures
there are the commands and fields that build does not have (`sim trace`,
`flips`, `motion`, `water_flickers`, `motion_gaps_max`); every assert about
the world passes on both.

**Evidence** (`docs/missions/C2-motion-evidence/`): `slide-plate-edge.png`
and `slide-cube-edge.png` (live frames each 60 Hz step, before and after),
`c2-slide-A/B/C/E.mp4` (the plate edge, the cube edge, the heap over a cube
corner, the 5,760-block collapse) and `replay-192516-c2-draw.mp4` (Andrew's
pile on this draw); `puddle-ledge.jpg`, `film-stone.jpg`,
`film-sand-border-edge.jpg`, `film-andrews-pose.jpg`; the rules-3 traces and
analyses, `flicker-test-before-after.txt`, `replay-192516-motion.log`,
`real-recording-refused.log`, `scripts.txt`.

## What changed in existing tests and scripts

- `snapshot_bytes_round_trip_and_reject_damage`: the older layouts are cut
  before version 5's trailer.
- `a_falling_block_waits_at_the_edge_of_the_active_region`: compares the
  state of the waiting block, not `z_prev` (look only).
- `sand_collapses_in_a_chain...`: landed blocks are offered to the renderer
  for `GHOST_TICKS_MAX`, then not.
- `water_on_a_shelf_runs_over_the_ledge...`: what the spread leaves as film
  dries instead of pumping itself over; the film's drying is waited for.
- `Falling` literals in tests use `Falling::new`.
- Outside `sim/`: `stream.rs` (the edit revision a mesh was built from),
  `render/mod.rs` (`meshed_at`), `render/shader.wgsl` (the `depth` varying
  and the thin-water fade), `world/edits.rs` (the flip log),
  `world/mod.rs` (level 0 is air), `mesh.rs` (the corner floor),
  `session.rs` (the phase, the state fields), `bench.rs`, `play.rs`,
  `play/video.rs`, `main.rs` (`--record-video`, `--water-trace`),
  `replay.rs` (the refusal's wording). Nothing in `worldgen/`, `app.rs`
  or `player.rs`.

## Not verified

- **Nobody has played this.** No window was opened. The hold and the ghost
  drop are exercised by the harness's live frames (one frame per step) and the
  replay video; in the game the mesh may be late by more frames, which the
  same code handles, but I have not seen it at vsync, nor the slide at 60 fps
  with the eye.
- The cost numbers are provisional: four other agents were building and
  running while I measured. The frame cost during the big collapse
  (`travel`, one earlier run: cpu average 5 ms, p99 50 ms, worst 60 ms on
  this branch; 8, 79 and 104 ms on the frozen build) is dominated by the
  streamer remeshing the chunks, not the draw, and varied with load.
- Films fed by a boundary reservoir, and `merge` and `keep` under the new
  hold, ran through the existing tests only; I built no scene for them.
  Under `seek` without `dry`, a film sheet whose thicker water cannot reach a
  drop now rests for ever (it used to pump itself over).
- The film's look at 50 to 200 m is "a dot, then nothing", which is right for
  a 1 m cell, but a whole terrace of film from far away is a tint I have not
  looked at.
- Saves holding a level-0 water cell from an old `fill` are not repaired.

## For Andrew to decide

1. **The slide's feel**: 0.2 s, fast off the edge then falling. Linear, slower
   or quicker is one number (`SLIDE_TICKS`) and the easing one line.
2. **Thin water now dries instead of running**: a sheet of level-1 cells near
   a ledge holds and evaporates (10 to 30 s), while anything thicker drains
   through it. Before, the sheet pumped itself over, blinking. If a wet trail
   lingering for half a minute is wrong, `--film-dry-s` shortens it.
3. **A waterfall's lip is dry**: the stream starts one cell below the edge.
4. **How faint a film should be.** 30% of the lake's opacity below 0.12 of a
   voxel is my guess at "a wet patch"; both numbers are in the shader and
   could become look-panel dials.
