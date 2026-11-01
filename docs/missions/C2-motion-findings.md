# C2: things that move look like they move (Fable, 2026-10-31)

Andrew's #5 and #9 from the 2026-10-31 round. Branch
`worktree-agent-aa8453bb1bd724059`. Evidence in `docs/missions/C2-motion-evidence/`.
`game/README.md` has the player's side ("What you see of it", "Water that does
not blink").

## For Andrew: what to try

1. **Place a block of sand or snow on a slope.** It should move sideways out of
   its cell, drop as it goes, and fall, as one block. There should never be
   two of it, and a landed block should never blink out and back.
2. **Knock a wall out from behind a sand cliff** (or drop a cube of sand on a
   plate): the whole slope should pour, each block sliding over its edge.
3. **Pour a bucket on a terrace near a step.** The puddle should spread, the
   step should take it, and the wet sheet left behind should dry in patches
   over ten to thirty seconds. Nothing should appear and disappear in quick
   succession. The stats line now ends with `N flickers`: it should stay 0.
4. **Pour a bucket and look at its edge from close by and from far away.** The
   thin edge is a sheet 4 mm thick; from any distance it should either be a
   faint blue tint or nothing, never a shimmer or a pair of wedges.
5. **Pour at the lip of a cliff.** The lip cell stays dry; the stream starts
   one cell below it. (Before, the lip blinked with every unit.)

Numbers you can change, all in `sim/mod.rs` and `sim/water.rs`: `SLIDE_TICKS`
(4: the slide takes 0.2 s), `FLICKER_FLIPS` and `FLICKER_WINDOW` (3 within 20
ticks: what counts as a blink), `LINE_CELLS` (48: how far thicker water looks
through a sheet for a drop).

## #5, the slide: the design and why

**What the rule does** (B4, unchanged, `sim_rules` untouched by this part): a
block with a free cell beside it and a free cell below that one becomes a
`Falling` in the next column at its own height, in one tick, and falls from
there.

**What the renderer now draws.** Every block in flight remembers the cell it
left (`Falling::from`, a `Trail`), and `Sim::view` hands the renderer a
`Drawn` per block that is continuous in time:

- *Between ticks.* The session tells the simulation how far its 60 Hz steps
  are through the current tick (`Sim::set_phase`), and a block is drawn part
  way from where the last tick left it to where this one did. The fall itself
  was stepping at 20 Hz before; it is smooth now too.
- *The slide.* For `SLIDE_TICKS` (4 ticks, 0.2 s) after a slide the block is
  drawn on the straight way from the centre of the cell it left to the centre
  of its own column, eased out (`1 - (1 - t)^2`): it clears the edge quickly
  and the fall takes over. At one tick it is 44% of the way over and has
  dropped 0 (the drawn fall lags the logical one by a tick); at two ticks 75%
  over and 6 cm down, so the cube never sinks visibly into the ledge it is
  leaving; by four ticks it is in its own column and 0.375 m down. The
  earliest landing is the sixth tick (a drop of one layer), so a slide always
  finishes before its block lands.
- *Edges and corners.* The way is a chord in world space between the two cell
  centres, computed from `Planet::pos`, so it is the same across a chunk
  border, a cube edge and a cube corner (where the two cells are on different
  faces); the cube keeps its own column's frame throughout, which differs
  from the old cell's by about a hundred-thousandth of a radian. Tested:
  `a_slide_over_a_cube_edge_names_the_cell_on_the_other_face_and_its_way_is_a_block_long`
  (the way is level and 0.8-1.4 m: cell centres across an edge are not
  exactly a metre apart), and scene B of `c2-slide.play`.

**Why purely visual, and why it is honest.** Nothing the rule does changed:
same cells, same ticks, same landings, same `sim_rules` (3 as far as powder is
concerned; it is 4 because of the water, below). The trail is neither hashed
(`state_hash`) nor in a replay's digest (`state_digest`), and no rule reads it:
`the_trail_is_look_only_and_not_in_the_hash_or_the_digest` clears it and gets
the same hash and digest. A replay recorded under rules 3 reproduces exactly
on this branch before the water bump, including Andrew's pile (`192516`: "5 of
5 checks matched; the end state hash matches"). A save carries the cell and
the age (snapshot version 5; versions 1 to 4 read as before, without trails),
so a reload mid-slide draws on from where it was:
`a_reload_mid_slide_draws_on`, and scene D of the script (save, reload, and
the next frames). What a save does not carry is the chunk revision (next
section): after a reload every mesh is rebuilt, so it counts as current.

Making the block really occupy the path would have needed a fractional
position across two columns, a sweep against both columns' cells, and a rule
change for every slide across a face edge; it would have changed timing,
saves and replays, for a picture that the interpolation gives exactly.

## #5, the second copy: found, and what it was

Andrew's note in the photo's sidecar says it: "snow stays in place at its prior
location for a brief moment before disappearing while a falling block has
already appeared at its new location". That is mesh latency. `World::commit`
queues the chunk for a remesh; `stream.rs` builds it on a worker and installs
it a frame or more later. Meanwhile the chunk's mesh still shows the block in
the cell it left, and the entity is drawn in the next column: two copies, for
as long as the mesh is late. The straight fall hides it (the entity starts in
the same cell and has dropped 6 cm after a tick); the slide, which moves a
whole cell in one tick, does not. `ghosts` was the other half of the same
problem, solved by a timer: a landed block is drawn as a cube for 5 ticks
while the chunk is remeshed, so if the mesh took longer the block blinked out
(a gap), and if it took less the cube and the mesh overlapped for the rest of
the 5 ticks (the same cube twice, differently shaded by AO).

**Measured frame by frame.** Every job the streamer dispatches now records the
chunk's edit revision it was built from (`JobResult::erev`), and installing it
writes that into `Renderer::meshed_at`. Every `Drawn` names the write it is
waiting on: the cell it left (or took) and that chunk's revision once the
write was in. `sim/draw.rs` can therefore say, per block per frame, whether
the mesh has caught up. In the harness, playing Andrew's recording with a
frame every step (`--replay ... --record-video`), the mesh was behind a block
only at `since` 0.00 ticks, i.e. for the one frame after the tick in which it
moved: `motion over 1124 frames: doubles max 1 (blocks held instead), held
max 1, ghosts max 1, gaps max 0, mesh at most 0.00 ticks behind a block`
(`replay-192516-motion.log`). So in the harness the double lasted one frame
per move and there were no gaps. In the game, where frames come at vsync and
the worker competes with the player, it is one to a few frames; I could not
measure the window (no window may be opened in this round).

**The fix, both halves.** `sim/draw.rs` holds a block in flight exactly where
the mesh still shows it, at the cell it left, until the mesh drops it, and
starts its slide clock then, so nothing jumps; a landed block is drawn as a
cube until the mesh shows it, and not a frame longer (`GHOST_TICKS_MAX`, 40,
bounds how long the simulation keeps offering it). Where the renderer cannot
tell (no LOD-0 mesh of that chunk, or a reload), it does what it did before:
on its way at once, a cube for `GHOST_TICKS`. `place()` is the whole decision
and is unit-tested without a GPU. The harness counts what this held back as
`motion_*`; `motion_gaps_max == 0` is asserted in every scene of
`c2-slide.play`.

**Andrew's "1 falling at the end"** of `192516` is not a bug: the state sidecar
lists it (`falling_blocks`, new): a snow block two ticks into the fall after
its seventh slide down the pile, with a chain of landings every six ticks
behind it. He stopped recording while it was still going.

**Cost.** See scene E of the script: TODO numbers. The trail is 24 bytes per
block in flight; the hold map is one entry per block drawn; `meshed_at` is one
entry per resident LOD-0 chunk.

## #9, the flicker: measured, then fixed (rules 4)

**Measured first, on rules 3.** `--water-trace` logs every cell that goes from
dry to wet or back, tick by tick (the edit store logs them whoever wrote them,
`EditStore::take_flips`), and `SimStats::water_flickers` counts cells on a
floor that flip three times or more within a second. Replay `192820` (nine
buckets poured on a terrace): **4,686 flips over 1,174 cells; 351 cells flipped
three or more times within 20 ticks, 2,345 of the gaps between flips were one
tick**, the cells were wetted to level 1 (1,449 times) or 2 (300), and a
typical one reads

```
tick 3890: 0->1  3891: 1->0  3896: 0->1  3897: 1->0  3898: 0->1  3899: 1->0
3900: 0->1  3901: 1->0  3902: 0->1 ...   (20 flips in 20 ticks)
```

(`replay-192820-flips-rules3.txt`, `replay-192820-one-cell-rules3.txt`). A
10 Hz blink over hundreds of cells for about a second each: Andrew's
"appearing and disappearing in rapid succession for a moment before finally
settling". Replay `192845` has no flicker at all (0 cells), so the lone cell
is a separate matter. Three causes, all in `sim/water.rs`, in order of weight:

1. **The seek pump.** A level-1 cell with a drop within `--film-reach` gave its
   unit to the drop (1->0); the thicker cell behind it spread into the dry
   cell the next tick (0->1); and again. Every unit of the puddle crossed the
   sheet this way, one cell blinking per unit per tick.
2. **The lip.** A cell of level 2 or more beside the lip of a drop spread onto
   the lip, and what landed there fell the next tick: the lip went 0->1->0
   with every unit that passed (`0>3 3>0 ... 0>4 4>0` in the trace).
3. **The frontier.** The spread gave the remainder to the neighbours: a cell of
   level 2 with two dry neighbours wetted them with one each and went dry
   itself (2->0), then was wetted again from behind (0->1).

**The rule now.** (a) A sheet of film is one thing: a level-1 cell with water
beside it neither runs nor goes over a ledge; only a lone film cell does.
Thicker water drains *through* the sheet: it looks along the same four lines,
which pass through film cells without spending reach (`LINE_CELLS` = 48 bounds
the walk), and moves half its level into the drop; the sheet stays wet while
it does, and dries where it lies afterwards. (b) Nothing spreads onto a cell
over a drop; the slide sends water straight past it into the cell below, so
the lip stays dry. (c) The remainder of a spread goes to the spreading cell
first. The potential argument is untouched: (a) and (b) are falls from layer
k to k - 1, (c) is as even a distribution as before.
`the_potential_falls_on_every_tick_under_every_rule_on_random_stairs` and the
`settle` checks in every film test ran green.

**Proof it was the cause and is gone.**
`a_puddle_drains_to_a_ledge_through_its_film_without_blinking` pours a block
four cells from a cliff with a basin below: under rules 3 (the test run before
the rule change) it settled with **392 flickers**; now **0**, with 702 flips,
169 of the 255 units in the basin at once and the rest dried on the table
(`flicker-test-before-after.txt`). `c2-film.play` scene A is the same scene
with the trace on and `assert water_flickers == 0`: TODO result.
`nothing_spreads_onto_a_cell_over_a_drop` checks the lip.

**What it changes elsewhere.** Thin water near a ledge now evaporates rather
than running over one unit at a time: on the B4 shelf test
(`water_on_a_shelf_runs_over_the_ledge...`) a start of 2 or 3 units leaves a
film of 1 on two cells that dries, where before everything went over; starts
of 7, 40 and 255 go over as before but for the sheet they leave. A waterfall's
lip cell is dry, and the stream starts a cell lower. The B4 rule table
(keep/seek/merge/dry/seek+dry) shifts for `seek` and `seek+dry`: TODO numbers
from `film_rules_on_the_three_scenes`. Falling streams of one-unit packets
still light up one cell per tick as they pass; the metric leaves cells with an
open floor out, and the look of a dribble is as it was (not this mission's).

**Saves and replays.** `RULES_VERSION` is 4. Saves from rules 1 to 3 continue
with the usual note (same stored meanings). A replay recorded under rules 3 is
refused: "it was recorded before water learned to drain through a film without
blinking (C2)" (`real-recording-refused.log`). The three recordings of this
round are therefore unplayable on this branch; their rules-3 measurements
above were taken before the bump.

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
in Andrew's photo. A corner with a solid cell beside it (1 + 1) / 3 is 0 too;
with two solids (1 + 1) / 2 is 1, which is why some corners stood up and the
slivers were triangles.

**The fix.** A corner of water now stands at least one level, 4 mm, over the
floor (`a_film_cells_surface_stands_above_the_floor_at_every_corner`; it
failed before the one-line change). 4 mm is far above what the depth buffer
can confuse at any distance a 1 m cell is still visible from (a float
reversed-Z depth resolves about 1e-7 of the distance: 10 µm at 100 m). Frames
at 2, 8, 50 and 200 m on stone and sand, across a chunk border and over a cube
face edge, and at Andrew's own pose: `film-*.jpg`, before (the frozen
8134c61 build) and after. TODO what I saw.

## Verification

TODO: cargo test numbers, scripts table, before/after runs on the baseline exe.

## Not verified

- **Nobody has played this.** No window was opened. The hold and the ghost
  drop are exercised by the harness's live frames (one frame per step) and by
  the replay video; in the game the mesh may be late by more frames, which the
  same code handles, but I have not seen it at vsync.
- The cost numbers are provisional: four other agents were building and
  running while I measured.
- Films fed by a boundary reservoir, `merge` and `keep` under the new hold
  were run through the existing tests only; I built no scene for them.
- Saves holding a level-0 water cell from an old `fill` are not repaired.

## For Andrew to decide

1. **The slide's feel**: 0.2 s, fast off the edge then falling. Linear, slower
   or quicker is one number (`SLIDE_TICKS`) and the easing one line.
2. **Thin water now dries instead of running**: a sheet of level-1 cells near
   a ledge holds and evaporates (10 to 30 s), while anything thicker drains
   through it. Before, the sheet pumped itself over, blinking. If a wet trail
   that lingers for half a minute is wrong, `--film-dry-s` shortens it.
3. **A waterfall's lip is dry**: the stream starts one cell below the edge.
4. **The 4 mm film is visible** as a faint tint close up. If the edge of a
   puddle should vanish instead, it is a draw-time choice, not a rule.
