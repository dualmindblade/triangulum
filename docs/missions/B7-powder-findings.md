# B7: powder that holds a level (Opus, 2026-10-31)

Andrew's #8: "Experiment with alternate powder mechanics similar to water levels
but with an angle of inclination." Off by default. Branch
`worktree-agent-a570e097b35777d2a`. Evidence in `docs/missions/B7-powder-evidence/`.

## For Andrew: what to try

Run the game with a switch. Without one it is today's game, to the pixel.

```
cargo run --release -- --powder levels            sand and gravel rest at 35 degrees
cargo run --release -- --powder-angle 25          a gentler slope (any angle from 1 to 70)
cargo run --release -- --powder-angle 45          the angle today's rule has along an axis
cargo run --release                               today's rule
```

The HUD's stats line says `powder: levels at 35 degrees` when it is on. This mode
plays on its own save, `saves/<world name>-powder`, so your real world is not
touched, and either kind of game refuses the other's save and tells you why.

Things to try, in plain words. Sand and gravel are on the default hotbar.

1. **Pour sand on one spot.** Hold the right button on the ground. A smooth pile
   grows, with no steps in it, at the angle you asked for. This is the first
   thing to judge: `compare-tower.jpg` has what to expect, today's rule in the top
   row.
2. **Walk up it and down the far side.** No jumping and no stairs: you go up the
   slope. Then jump on it, and sneak on it.
3. **Dig into the side of a pile.** It slumps back to the angle, and the top comes
   down with it.
4. **Dig a shaft in a beach.** You get a dimple, not a staircase. Generated sand is a
   special case: see "A generated slope holds" below.
5. **Build a sand tower or a cube, and knock a wall out from behind a sand cliff.**
   This is where it looks worst while it moves: see "In motion" below. It looks
   best when it has stopped.
6. **Tip sand into a pool or a lake.** It sinks, pushes the water up, and comes
   out of the water at a slope with a shoreline.
7. **Start at 25, then at 45, then at 60**, and do the same thing each time. The
   angle is the one number this whole experiment is about. A pile from another angle
   stays as it is until something touches it. `--save saves/try-25` starts clean.

Numbers you can change:

| what | how | now | what it does |
|---|---|---|---|
| the angle | `--powder-angle DEG` | 35 | 1 to 70 degrees. Dry sand is 30 to 35 in the real world |
| how much a cell pours over a drop | `POUR` in `sim/powder.rs` | 128 units a tick | lower pours a tower slower |
| how fast a part-full cell falls | `FALL` in `sim/powder.rs` | 2 cells a tick | through air; one through water |
| the thinnest layer that spreads | `LEAST_SPREAD` in `sim/powder.rs` | 8 units (3 cm) | stops tiles of dust at a pile's foot |
| how steep a generated slope holds | `powder_step`, `hold` | one block per block | see "A generated slope holds" |
| how tall an edge the slope lifts you up | `SLOPE_STEP_UP` in `player.rs` | 0.6 of a block | past this a pile's edge is a wall |

255 units is a whole block, as for water.

## What the frames show

Every row of these is the same scene with the same camera, today's rule at the
top, then 25, 35 and 45 degrees. All from `scripts/b7-compare.play`.

| file | scene |
|---|---|
| `compare-tower.jpg` | a tower 32 blocks high, at rest |
| `compare-cube.jpg` | a 7 x 7 x 7 cube dropped on a table, at rest |
| `compare-pit.jpg` | a pit dug in a bed of placed sand |
| `compare-cliff.jpg` | a cliff, its wall taken away |
| `compare-beach.jpg` | a four-deep shaft in a real beach |
| `compare-pool.jpg` | sand tipped into a pool |
| `compare-tower-falling.jpg`, `compare-cube-landing.jpg` | in motion |
| `close-up.jpg` | 35 degrees close to: toe, slope, diagonal, wall, post, chunk corner, shelf, shore, cube edge |
| `walk.jpg` | by hand on a real beach: poured, on top, going down, dug |
| `textured.jpg` | a test world where sand has a picture: the slope wears it |

Measured from the settled heights of those scenes (`surfaces.txt`). The first
column is the steepest step between neighbouring columns, along an axis:

| rule | tower | cube | pit | cliff | along a diagonal, the steepest of those |
|---|---|---|---|---|---|
| today | 45.0 | 45.0 | 45.0 | 45.0 | 54.7 |
| 15 | 14.9 | 16.8 | 17.4 | 17.2 | 16.7 |
| 25 | 25.0 | 25.0 | 27.4 | 27.4 | 26.5 |
| 35 | 36.1 | 37.1 | 35.8 | 35.8 | 35.1 |
| 45 | 45.0 | 45.0 | 45.0 | 45.0 | 45.0 |
| 60 | 60.0 | 60.0 | 60.0 | 60.0 | 60.0 |

The numbers over the angle are the last cells of a toe, which stand a few units past
the slope because of `LEAST_SPREAD`. Down the diagonal a pile is now as steep as along
an axis, where today's rule is ten degrees steeper.

## The rule

The module doc of `game/src/sim/powder.rs` has all of it. In short:

- **A cell holds a level**, in the byte water uses, so it costs no new layer and
  saves, hashes and replays already carry it. A stored 0 is a whole block, because
  that is what every generated sand cell already holds: nothing needs converting,
  and generated sand stays whole until disturbed.
- **A column stands at** 255 k + L. Its top cell compares itself with the surface
  of the eight columns around it. Where the drop exceeds the slope (255 tan(angle),
  and sqrt(2) times that along a diagonal) it sends half the excess that way. A cell
  with more to send than it holds shares it out, steepest first, ties by a hash.
- **It comes to rest, provably.** Phi = sum of (510 k + L) L is the shape of water's
  potential. Moving d units from a cell standing at Ha to one standing at Hb changes
  it by 2 d (Hb - Ha + d), which is negative exactly when d < Ha - Hb, and every
  move meets that. The module doc goes through them. Phi is a non-negative integer,
  so only finitely many moves can happen. `settle` in `powder_tests.rs` checks that it
  strictly falls on every tick that moves powder and that no unit appears or goes, on
  every scene, including six random heaps from 12 to 69 degrees.

**What the hard cases do:**

| case | what happens |
|---|---|
| a full block landing on a partial cell | it lands whole on top, then next tick fills the cell from above, and what is left stays in the cell it landed in |
| a falling powder cell | a whole block over a free cell is the falling block it always was. A partial cell falls by levels, 2 cells a tick through air, 1 through water, drawn as a flat tile |
| powder meeting water | it pushes the whole cell's water up its column, through any loose powder above. Nothing is lost. Partial powder is a wall and a floor to water, because one byte can't hold both. For the eye only, the water beside a slope goes on across its cells, so a pool has a shoreline and not a wall of water |
| two powders | they do not mix: a partial cell of gravel is a wall to sand. The alternatives were turning sand into gravel or storing a ratio |
| a block in flight | powder never moves into its cell, and a block doesn't start falling into one. See below: the proof depends on it |

**Two things I got wrong first, which the tests or the frames caught:**

- Going one way only, the steepest, made four-armed stars, because a rough surface
  within the slope is at rest and nothing evens out a lobe. All the ways at once
  gives a pile that comes out the same on every side: 32 blocks stand 2.40, 1.73, 1.02
  and 0.32 high going one way along an axis, and 2.40, 1.70, 1.00 and 0.30 the other.
- The potential rose, in a test of a 60-block tower. A block in flight whose cell
  has been filled settles above whatever stands over it (`advance_falling`, fine
  under today's rule, which never fills it), and that can be a whole stack. So nothing
  moves into a cell a block is in. A second form of the same thing, a whole cell
  starting to fall into one, only showed on the real beach, after I changed how fast
  tiles fall: two blocks in flight overlapped, and the upper one settled 1.3 layers
  higher than it had been. Both are in `flying` in `powder.rs`.

**A generated slope holds.** This is my call, and it's the one to check. A cell
still exactly as the generator made it keeps today's limit, one block per block, if
that is steeper than the angle. Without it, one dig on a beach at 20 degrees would
set every natural slope in the active region flowing down to 20 degrees. With it, a
four-deep shaft on the beach at 20 degrees makes 12 moves and is still in 11 ticks.
Everything you placed, and everything disturbed, follows the angle. So a pit on a real
beach keeps natural walls of up to 45 degrees with loose sand on them, and there is
a stair-stepped natural beach right beside a smooth pile (`walk.jpg`, first frame).

## Seeing it

`game/src/world/slope.rs`. A pile at 35 degrees falls 0.7 of a block per cell, so
its surface crosses layers inside a column, and water's way of drawing a level, a
sloped top that stays inside its own cell, leaves a stair of risers. So the slope is
a height field over columns.

- A corner's height depends only on the columns round it, so every column, chunk
  and cube face that shares it gets the same integer. Tested across a chunk, a chunk
  corner, a cube edge and a cube corner.
- The middle of a column stands at the column's own height, so a top is four
  triangles. That is why a lone part-full cell is a small tent and a one-wide strip
  doesn't vanish, and why the surface has no grain along either axis.
- Ground lower than the powder at a corner holds it down to the ground, so a pile
  feathers out rather than ending in a kerb.
- Cells a slope cuts through leave the cube mesh. Where the surface dips below a whole
  cell's top, that cell is drawn by the slope too.
- Only columns that are partial or touch a partial column are drawn this way. The
  rest of the sand is cubes. One ring of a flat beach blends into a pile on it.
- Smooth shading, with a gradient at each corner taken from the columns round it, so
  cells agree. In the block's own colours and textures, through the terrain's own
  fragment shader.

No holes, by cases (`every_edge_of_a_slope_is_closed`, and the module doc): two slope
columns share corners; beside anything lower a slope has a skirt down to where its
cubes begin; a neighbour higher than the surface has its own cube side showing,
because the slope's cells no longer hide it. I looked at frames from level with the
toe, from a hand above the slope, along a diagonal, from under the plate, against a
wall, round a post, over a chunk corner, on a shelf, at a shore, across a cube edge,
and at 15, 25, 35, 45 and 60 degrees. I found no hole.

It has its own pipeline and its own draw records, and with no slope in view neither
costs anything. Coarse detail levels show a partial cell as a whole block, so a pile
turns to steps at a distance (`overlay_coarse` is untouched).

## Standing on it

**What I chose:** the collision is the surface. The box stands on the highest point
of the slope under its footprint, on the same triangles that are drawn. The sweeps
treat the cells a slope replaces as open, and the surface carries the body after
them. So:

- No stairs, no stepping. Walking over a pile four ways, the height changes by at most
  0.077 of a block in a 60th of a second, and the body is never off the ground.
- Your feet are on the highest point under a 0.6 m box, so on a 35-degree slope your
  centre stands about 0.2 of a block above the surface directly under it. It should
  feel like a box, not a point. It also means no cube under the slope can catch your
  toes.
- Going down, you stay on the ground instead of skipping off each step.
- It works at any angle. There is no sliding back off a steep pile.
- Past 0.6 of a block of sudden rise, as at the edge of a pile on a shelf, it's a wall.
- Falling through can't happen: at terminal speed, 70 m/s, more than a cell a step, a
  body stopped on the surface from six places, and it is lifted back if a step ever
  leaves it under it. A slope overhead, on a stone shelf, does not lift you.

**Picking** stops where you see the surface: a ray that crosses a cell above its slope
goes on to what is behind, tested with a post behind a low cell. Breaking a cell takes
the whole cell, whatever it holds. A block placed on a partial cell lands on it.

## In motion

This is the part I'd change next. At rest it looks right from every side. While it
moves, what's in the air is flat tiles, and consecutive part-full cells draw as a
solid stack, so a tower sheds a few slabs that look like shelves
(`compare-tower-falling.jpg`). A collapsing cliff goes in strips. It lasts a second
or two. I tried three versions: one cell a tick made a solid sleeve slide down the
tower, tents in mid-air made fringes, and this was the least bad.

## Saves and replays

**I chose the simplest thing that can't hurt a real save.** A levels game keeps its own
save, `saves/<world name>-powder`. A levels game refuses a normal save, a normal game
refuses a levels save, and a build without this refuses one too (`bad flow setting
"powder=levels"`). A refused save is not touched: `diff -r` against a copy says so. To
the normal game a partial cell would be a whole block, and sand would multiply. No
save is ever deleted.

- The save format and `sim_rules` are unchanged for the default path: `RULES_VERSION`
  stays 3 and this mission adds nothing to a snapshot or a chunk file. The level goes
  where water's does, and the angle goes in the flow settings' text, which is hashed,
  saved and replayed. With the switch off that text is what it was, and a test pins it.
- Saved in the middle of a collapse and reloaded, a levels world runs on to the same
  hash (`a_save_in_the_middle_of_a_collapse_reloads_exactly`, and scene D of
  `b7-compare.play`).
- A levels session records and replays: `b7-walk.play` pours, walks, digs, saves and
  reloads, and its replay matches 17 of 17 checks and the end hash, in the script and
  again from the command line.

**The real save was not used.** `saves/Neisor` is from generator version 3, so this
build and main both refuse it for that reason. I tested on a save main made instead,
and read the real one only to copy it.

## Proposed wording for DESIGN.md

(I have not edited it.) Under "Things that move", if Andrew keeps this:

> **Powder levels (experiment, 2026-10-31, `--powder levels`).** A powder cell holds a
> level, and moves downhill to its eight neighbours only where the height difference
> exceeds the slope of its angle of repose (35 degrees, a dial), so a pile rests at that
> angle. Generated powder is whole until disturbed, and a generated slope holds at one
> block per block. Partial powder is a wall and a floor to water, and two powders do
> not mix. Coarse detail levels show it as whole blocks.

## The default game has not changed

Everything here compares this build, main merged in, with main itself, 04d1cfe
(`default-unchanged.txt`).

| check | result |
|---|---|
| every test | 362 passed, 0 failed, 25 ignored: main's 348 and these 39 |
| 11 scripts (the five asked for and six more) | the same asserts pass on both builds |
| their 94 state files, 7,164 fields, every counter and position | 0 differ. (Left out: timings, how far streaming had got, and the two new counters) |
| 7 clean 1920 x 1080 frames: ground, falling sand, settled sand, beach, dug beach, river, lake | the same to the pixel |
| a recording made by main, played by this build | 8 of 8 checks, same end hash |
| a recording made by this build, played by main | 8 of 8 checks, same end hash |
| a save made by main, 200 ticks on each | the same world hash, `619328c4...` |
| frame cost, the builds in turn, 24 benches each of 600 frames | wall median 0.775 ms on main, 0.770 here; fastest 0.75 both; GPU 0.33 both (provisional) |
| draws and triangles | 813 and 473,428 on both |

In the default path the new code is a branch on a flag each tick, an atomic load on
each player step, each mesh job and each commit, and one empty list per frame. The
shader has one more function, and `vs_main` passes it constant `false`s.

## Costs (all provisional: another agent was running)

| 30 x 30 x 12 block with nothing round it | ticks to rest | last visible move | first tick | then average | then worst | moves |
|---|---|---|---|---|---|---|
| today's rule (B4's number) | 123 | | | 0.84 ms | 3.57 ms | |
| 25 degrees | 825 | 790 | 106 ms | 3.85 ms | 7.38 ms | 651,648 |
| 35 degrees | 499 | 465 | 104 ms | 3.33 ms | 4.83 ms | 298,992 |
| 45 degrees | 317 | 295 | 103 ms | 2.93 ms | 4.04 ms | 137,426 |

- About four times the cost of a tick of the block rule, on the biggest thing I tried,
  and steady. A collapse this big also runs about four times as long as today's: the
  block rule's blocks fly, and these units walk.
- The first tick is 103 ms, and I did not find out where it goes. All 10,800 cells were
  notified at once by a test's `fill`, which a player can't do, but a tick that long
  is a visible hitch, so it wants looking at before this is anything but an experiment.
- **Meshing is the real cost.** One chunk full of slope takes 13 to 15 ms in
  `padded_powder`, because it reads the cells of nine columns for every column, each
  read through the edit store's lock. It's on the worker threads, not the frame, but a
  big flow remeshes those chunks at up to 20 Hz. If this stays, that's the first thing
  to speed up.

A settled pile costs nothing: nothing queued, nothing frozen, no update runs, tested.

## Feedback

- The frames found what the tests could not: the stars, the sleeve, the wall of water at
  a pool's edge. Each passed every assert. Every one of them was one look at a contact
  sheet away.
- A test written to one scene (`digging_a_real_beach...`) caught a hole in the proof
  that six random heaps had missed, because real ground has shapes I don't think of.
- Replaying each build's recording on the other is a very cheap proof of "nothing
  changed". Worth keeping for any mission with a switch.

## Verification

- `cargo test --release`, main merged in: **362 passed, 0 failed, 25 ignored**.
- `cargo test --release -- --ignored stress --test-threads=1`: 5 passed.
- The new tests in a debug build, which catches integer overflow: 38 passed, 0 failed.
- Scripts, every assert passing, on this build and on main:

| script | asserts |
|---|---|
| m1-done | 94 |
| m2a-sand | 95 |
| b4-slide | 49 |
| m2b-water | 97 |
| m2b-boundary | 44 |
| m2b-river | 42 |
| m2c-replay | 29 |
| edge-corner-flight | 17 |
| tunnel-wall-reload | 18 |
| b4-film | 10 |
| b7-default-frames (new) | 3 |
| b7-compare (new), under today's rule and at 15, 25, 35, 45 and 60 degrees | 34 each |
| b7-close (new), at 35 degrees | 11 |
| b7-walk (new) | 24 |

New tests: 23 of the rule and a stress test in `sim/powder_tests.rs`, 7 of the surface
in `world/slope_tests.rs`, 8 of walking in `player/slope_tests.rs`. They cover: rest at
each angle, conservation, the potential tick by tick, round piles, partial falls,
landing on partial cells, water, two powders, pouring, digging at the foot, order of
placing, rayon pools of 1 and 8, restore, save and reload, a chunk corner, a cube edge
and a cube corner, the edge of the active region, a real beach, walking, falling at
terminal speed, jumping, sneaking, a shelf, a slope overhead, a cube edge underfoot,
and picking.

## What I did not verify

- **Nobody has played this.** No window was opened: everything is headless, as asked.
  How it feels to walk on is unjudged, and that is the point of it. The walking numbers
  say it's smooth. They don't say it's nice.
- **The game's own start has not run with the switch.** `--powder levels` was exercised
  through `--play` and `--replay`, and the save refusals through `--play`, because the
  game proper opens a window. So `saves/<world name>-powder`, the path a plain start
  chooses, has never been created for real. It's two lines in `main.rs`.
- **Generated terrain beyond one beach.** I did not survey how far a dig spreads on
  steep generated sand or gravel, or on a river bed.
- The mesh view does not tint or outline slopes: they draw as normal in it.
- Slopes have no corner shadows (ambient occlusion). They are lit by their angle.
- A cube-corner pile is tested for rest and matching corners, but walking on one isn't.
- Half-metre blocks (the `block size` switch) are untested with this.
- All timings are provisional.

## What I cut

- **One angle for all powders.** Sand and gravel share the dial.
- **An in-game switch.** Command line only, plus `sim powder` in scripts.
- **Water in a partial cell.** It is drawn that way, not stored that way, so a low
  partial cell is a dam a block high to flowing water, and water stands on top of a
  partial cell as if it were full.
- **Mixed powders in a cell.**
- **Slopes at coarse detail.** Far away it is whole blocks.
- **Sliding off steep piles.** You can walk up 70 degrees.

## For Andrew to decide

1. **Should this replace today's rule?** My recommendation: **it's worth carrying on
   with, not worth switching yet.** At rest it is a big step: compare the top row of
   `compare-tower.jpg` with the rest. But the transition isn't solved, partial cells
   wall water off, and it costs a real mesh job per chunk. The decision needs your
   hands on it, walking and pouring.
2. **What angle?** 35 is real sand. 45 is the same steepness as today along an axis.
3. **Is a generated slope holding right?** A dig on a beach gives a dimple, not a
   wave. Natural stairs stay beside smooth piles. The alternative is that a dig sets
   a whole beach flowing, once, and then it's smooth for good.
4. **Should generated sand be smooth from the start?** Out of scope here, and I didn't
   touch the generator, but it's the obvious next question if you like this: natural
   stairs next to smooth piles look like two different materials.
5. **Should sand and gravel differ?** Gravel really does rest steeper.
6. **Do you want to slide on steep piles?**
7. **How should it look while it falls?** That's the weakest part: see "In motion".

## Branch and commit

Branch `worktree-agent-a570e097b35777d2a`, code at commit 2206d1c, with main (04d1cfe, R4 and S1)
merged in. Not merged, not pushed.
