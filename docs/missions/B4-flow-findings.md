# B4: powder and water slide, and the level-1 film (Opus, 2026-10-31)

Andrew's #7 and #9 from the 2026-10-30 bug round. Branch
`worktree-agent-a65302437b6f6f6c7`. Evidence in `docs/missions/B4-flow-evidence/`.
`game/README.md` has the player's side of all this.

## For Andrew: what to try

Run the game normally for the default rules. To compare, add a switch:

```
triangulum --film keep        the slide only; a drained area keeps its film
triangulum --film seek        film runs to a drop, and stays where there is none
triangulum --film merge       thin water beads up
triangulum --film dry         film dries; it never runs anywhere
triangulum --film seek+dry    the default: runs if it can, dries if it cannot
triangulum --no-slide         the game before this round, exactly
```

Things to try, in plain words:

1. **Stack sand.** Place sand blocks in a tower. It can't stand: every block
   past the first slides off and you get a low heap.
2. **Dig in a beach.** A shaft four deep fills itself in and leaves a shallow
   dip. Holes in sand and gravel do not keep their walls any more.
3. **Pour sand on one spot** and watch the pile. Is one block per block the
   slope you want? That's the steepest this rule can stand; #8 is where it
   changes.
4. **Knock a wall out from behind a sand cliff.** It pours away over a few
   seconds and ends as a staircase. Too fast, too slow?
5. **Dig beside a river.** Its gravel bed slides into your trench.
6. **Drain a small pool** and watch the bed. The thin sheet should run off
   towards the drop, and what can't get there dries in patches.
7. **Pour one bucket on flat ground.** It spreads into a puddle, then dries
   away from its edge in about three minutes. **Water you poured disappears.**
   Is that all right? If not, `--film seek` keeps it for ever.
8. **Pour a bucket on a hillside.** It runs down the steps instead of clinging
   where it landed.
9. **Look at a lake's edge and a slow trickle.** They should not dry up.

Numbers you can change:

| what | how | now | what it does |
|---|---|---|---|
| how far a film runs to a drop | `--film-reach N` | 8 cells | more drains more, costs a little more |
| how long a film waits to dry | `--film-dry-s S` | 10 s | it dries between S and 3 S after it comes to rest |
| what counts as a puddle | `PUDDLE_CELLS` in `sim/water.rs` | 256 cells | a bigger body of level 2 or more keeps its edge |
| how fast water moves | `--water-period N` | 1 | unchanged from M2b |

## What changed in the rules

Proposed wording for `DESIGN.md` (I have not edited it):

> **Water.** Water moves only downhill. Sideways it moves when neighbours
> differ by at least 2 levels, so it always comes to rest, and resting water
> may lean up to 1/256. The one exception is a ledge: water beside a lower
> open space goes into it whatever its level, the last level included.
> Level-1 water that has nowhere to go dries (2026-10-31, `--film`, Andrew's
> to confirm), unless it is the edge of a large body or of water fed from
> beyond the loaded area.

> **Things that move, 1. Falling blocks** fall straight down their own column
> (sand, gravel, collapses). A block at rest with a free cell beside it and a
> free cell below that one moves into that column and falls (2026-10-31). So
> powder rests with neighbouring columns at most one block apart. Columns
> keep their shape, so nothing needs converting.

## #7, powder

A resting block with a free cell beside it and a free cell below that one
becomes a falling block in the next column at its own height.

- **It settles.** A slide needs a free cell of its own below the free cell,
  counting what is in flight in that column. So every slide ends at least one
  layer lower, the sum of the blocks' layers never rises, and a heap cannot
  cycle or jitter. `every_slide_ends_lower_so_a_heap_cannot_cycle` checks it
  tick by tick. My first version broke this: four blocks round a one-cell pit
  all went and stacked above where they started. That test caught it, and the
  own-cell rule is the fix.
- **Direction.** The lowest of `choice`, a hash of the two cells and the tick.
  Every slide of a tick is decided against the same world, and when two want
  one free cell the lower hash wins, so neither the order blocks ran in nor the
  lattice's axes decide anything. 400 blocks with all four ways open went
  99 / 95 / 105 / 101. Heaps placed in opposite orders hash the same on every
  tick.
- **Edges.** Tested across a chunk corner, a cube edge (both faces) and a cube
  corner (all three faces). A block whose way lies through a chunk outside the
  region stays put, with its update filed under that chunk, so it is saved and
  costs nothing. It goes the tick the region arrives. That also holds when only
  the cell under the free cell is outside.
- **Save and reload mid-collapse** reaches the same hash as the straight run,
  in a test and in `b4-slide.play` scene C.

**The slope.** One block per block along the grid, exactly: a 400-block pile
is 8 high and 139 columns wide, with the profile 8 7 6 5 4 3 2 1 along +i. That
is 45 degrees along an axis. The pile is diamond-shaped, so down its diagonal
it drops 8 6 4 3 1, about 55 degrees. Nothing is steeper, and that is the whole
of what #8 would change.

**How far a collapse cascades.** Not far. A settled pile stands exactly at the
limit, the worst shape there is, yet one block cut from the foot of a 300-block
pile moved 2 to 6 blocks in 9 to 17 ticks, and the block under its 6-high middle
moved 5 in 11 ticks. A collapse is only large when the shape was far past the
slope to begin with. Each block can only go down, so the upper bound is every
powder block in the active region, once.

| collapse (provisional) | blocks | ticks to settle | peak in flight | average tick | worst tick |
|---|---|---|---|---|---|
| 13 x 8 x 10 cliff, wall removed (`b4-slide.play`) | 1,040 | under 430 | 170 at tick 6 | not separated | not separated |
| 30 x 30 x 12 block with nothing round it (`stress_sand_collapse`) | 10,800 | 123 (6.2 s) | 1,835 | 0.84 ms | 3.57 ms |
| 5,041 blocks dropped at once (`stress_five_thousand...`) | 5,041 | under 200 | 5,041 | 1.98 ms | 34.8 ms (the mass start) |

A column takes blocks only as it has room, so a cliff pours rather than bursts.

## #7, water

A cell that can neither fall nor spread, beside a free cell whose floor is
open, moves half its level, rounded up, straight into the cell below that free
cell. The level-1 cell at a ledge now goes over.

**Why water still comes to rest.** A slide moves d units from layer k to k - 1,
exactly like a fall, and Phi sees only layers and levels, so the fall's bound
holds unchanged: Phi changes by -A d + 2 d (B - L + d) <= -(1024 - 508) d < 0.
This is why it is one move into the lower cell and not a sideways step and then
a fall: a sideways step of the last unit leaves the sum of squares as it was,
and Phi would stand still on that tick. A tested ledge unit lowers Phi by
exactly 1024. Module doc of `sim/water.rs`.

**The way opens late.** Digging the cell under the free cell does not notify
the block or water beside it, because they are diagonal. An empty cell with
room below now wakes the water and powder beside it. That is one extra read on
each notified empty cell, rather than four more neighbours on every change.

## #9, the film

**Reproduced first, on the old rules** (`b4-film-before.log`, frames below):

- A drained terrace keeps a wedge rising one level per cell from its edge: on
  a terrace four cells wide, 1 2 3 4, and up to 8 in the corners. It never
  moves again: `water_moved` is identical at four and six minutes. It draws as
  a solid blue sheet over the whole terrace, a few millimetres deep and as
  opaque as a lake, which is what Andrew saw.
- One poured block is not "up to 255 cells at level 1", as the M2b findings
  guessed. It stops within 15 ticks as a cone, 114 cells, 37 at level 1, 5
  levels high in the middle, with a level-1 rim beside level-2 cells.

**A prediction of mine that was wrong:** I expected the ledge rule to erode the
wedge to a flat film. It only lowers it by one level, because a level-1 cell
beside a dry one is stuck again. So the slide alone does not fix #9, and `keep`
is close to the old picture.

**The rules on the three scenes.** `film_rules_on_the_three_scenes`, drying
time 2 s in this test only (the default is 10 s, so drying ticks run about five
times longer in play). Scenes: 25 cells at level 1 on a table with no edge in
reach; a 234-cell lake drained off a terrace six cells wide; one poured block.

| rule | plateau: left | lake: cells / units left | lake: dried | poured: left | poured: dried |
|---|---|---|---|---|---|
| keep | all 25 | 204 / 984 | 0 | 255 units | 0 |
| seek | all 25 | 144 / 364 | 0 | 255 units | 0 |
| merge | 7 beads, 25 units | 204 / 1,333 | 0 | 55 cells, 255 units | 0 |
| dry | 0 | 0 / 0 | 992 | 0 | 255 |
| seek+dry | 0 | 0 / 0 | 373 | 0 | 255 |

- **seek** does most of the work where there is a drop, and nothing where there
  is none. It follows four straight lines, so cells in a terrace's corners,
  whose lines miss the ledge, stay. That is cheap, but it is a real limit.
- **merge** gets rid of level-1 cells without losing water, but it leaves thin
  water standing on the bed in tiles (see the frames), so it does not clear a
  drained area.
- **dry** clears everything, but evaporates water that could have run off.
- **seek+dry, the default.** The bed ends clean, and about 62% of what `dry`
  would have evaporated reaches the water below instead.

**Water that leaves the world is counted:** `water_dried`, in `water_balance`.
`settle` in `film_tests.rs` checks every unit is in the world or counted, on
every tick, under every rule.

**Why drying cannot run for ever.** A level-1 cell dries unless the thicker
water it leans on reaches generated water at the region's edge, or runs past
256 cells. Without that, a film fed from beyond the edge would dry and refill
for ever and its chunks would never sleep. A strip fed at level 6 settled at
5 4 3 2 1 with 0 dried and then ran 600 ticks with no change at all. 625 cells of
level 2 with a level-1 edge lost only the 4 corner cells, which touch nothing
thicker. Drying removes A k + 1 from Phi; `merge` moves water uphill, so it has
its own potential, `film_potential`, which adds 510 per level-1 cell.

**Tested, not asserted:** on random stairs, four seeds under all five rules,
the potential strictly fell on every tick that moved or dried anything, every
unit was accounted for, each came to rest, and 100 more ticks ran nothing.

**No real lake was drained.** I tried to add one. The nearest is one cell deep over a flat bed and runs past the active
region, so its edge is a reservoir at level 204 and refills whatever you drain,
by design. So the lake scene is built, and real ground is covered by pouring on
a stepped hillside, which shows the rule well (`film-ground-100s.jpg`).

## Costs (all provisional: other agents and an uninstaller were running)

Wall-clock comparisons between processes were useless here: the same `--no-slide`
scene read 4.2 to 4.7 ms on a pour and 12.1 to 14.7 ms on a dam break, and one
slide run read 32.6 ms when other agents started building. So
`stress_flow_cost_of_the_new_rules` alternates the two rule sets in one process
and keeps each one's fastest of six runs.

| measure | rules before | new rules |
|---|---|---|
| big flow, fastest of six, same ~2.18M updates | 4.81 ms a tick | 5.28 ms a tick (**+9.8%**) |
| river diverted, `m2b-river.play` | 0.94 ms | 0.83 to 1.15 ms over two runs, with 46% more water moving |
| 2,000-cell pour | 619 ticks (M2b) | 617 ticks, 3.96 ms average, 7.39 worst |
| 10,000-cell dam break | not settled in 4,000 (M2b) | not settled in 4,000, 8.89 ms average |
| one cell, 5 x 5 basin | 8 ticks | 8 ticks |
| four-cell pour, 7 x 7 | 19 ticks | 19 ticks |
| shore pit (walls standing) | 122 ticks | 122 ticks |
| lake through a channel | 604 ticks | 720 ticks: the surface now drains on |
| idle, settled water | 0 updates | 0 updates, tested under every rule |

The dam break's worst tick was 81 ms against M2b's 18.9. Its steady tick is about
7 ms and no sampled tick passes 20 ms, so it is one isolated tick, and the log
samples ticks, so I can't place it. I can't say whether it is load or real.

**Must-not-regress list:** settled water costs nothing (`assert_quiet`, every
rule). A film does not keep chunks awake: waiting cells sit in their own list, at
no cost, and `b4-film.play` ends with an empty queue under every rule (under the
default, with 52 cells of scene D still waiting their turn to dry). Generated
rivers stay frozen until disturbed (`m2b-river.play` section 1 unchanged). No
perpetual jitter of water or sand (`assert_stays_still`, `assert_quiet`).

## Saves and replays

- `RULES_VERSION` is 3. A real recording from before is refused:
  `simulation rules version 2 vs 3: it was recorded before sand and water
  learned to slide off ledges (... Your world and saves are fine; to watch this
  recording, use a build of the game with simulation rules 2)`.
  `real-recording-refused.log`. The existing refusal tests pass unchanged.
- Simulation state is version 3 and still reads 1 and 2.
- **A real bug found by loading a copy of the real save.** It was saved at the
  cube corner, and a game starts at spawn, so the first tick moved the whole
  region and woke every changed pair across a chunk side around spawn: 137
  updates. They were no-ops under the old rule. Under the new one they started
  a wave with nobody touching anything: 53 slides, 52,143 units moved in 200
  ticks. A load no longer wakes pairs. `a_save_loaded_away_from_where_it_was_
  saved_wakes_nothing` fails without the fix and passes with it.
- The real save now does exactly what the old build does with it: 200 ticks, 0
  slides, 0 units moved, queue empty, 437 frozen before and after. I worked on a
  copy; the original was only read.
- A save made by the old build holding a sand tower, film at a ledge and a
  wedge loads, and 400 ticks wake nothing (`b4-oldsave-load.play`, 16 asserts).
  One block set on the tower brings it down, and the film 8 cells away stays.

## Verification

- `cargo test --release`: **194 passed, 0 failed, 16 ignored** (baseline 165 / 0 / 14).
- `cargo test --release -- --ignored stress`: 4 passed.
- `--no-slide` matches the build before unit for unit on `m2b-river.play`:
  1,200,009 moved, 47,556 in, 44,896 out.

| script | asserts | failed |
|---|---|---|
| m1-done | 94 | 0 (67 fps under load, 468 alone) |
| m2a-sand | 95 | 0 |
| m2b-water | 97 | 0 |
| m2b-boundary | 44 | 0 |
| m2b-river | 42 | 0 |
| m2c-replay | 29 | 0 |
| edge-corner-flight | 17 | 0 |
| tunnel-wall-reload | 18 | 0 |
| b4-slide (new) | 49 | 0 |
| b4-film (new), under all five rules | 10 | 0 |
| b4-oldsave-load (new) | 16 | 0 |

`m1-done` failed its `bench_fps > 60` once, at 45 fps, while other agents ran.
Run back to back when the machine was quieter, the before build (265 fps) and
this one (468) both passed, so I read it as load, not this change.

**Evidence** in `docs/missions/B4-flow-evidence/`. Before and after at the same
pose: `slide-tower.jpg`, `slide-cube.jpg`, `slide-cliff.jpg`, `slide-beach.jpg`,
`slide-corner.jpg`. Every rule side by side: `film-terraces-6min.jpg`,
`film-plateau.jpg`, `film-poured-10s.jpg`, `film-poured-180s.jpg`,
`film-ground-100s.jpg`. `film-drain-sequence.jpg` is the default draining. Logs
beside them; `film-tests.txt` has the rule table, `stress.txt` the costs.

## What changed in existing tests and scripts

Scripts:

| script | change | why |
|---|---|---|
| `m2a-sand` | `sim slide off` at the top | it tests falling itself, on towers that stand until undercut, and a free-standing tower can't stand now |
| `m2b-water` | `sim slide off` for the beach pit | its sand walls slump; that section is about the water, and `b4-slide` digs the same beach under the new rule |
| `m2b-water` | `crosshair == water` became `!= none` | the film has already run into the hole, so the ray reaches the dirt floor |
| `m2b-water` | the exact 765 is over a wider box | the third pour's film runs off down a natural step; the total stays exact |
| `m2b-boundary` | `sim slide off` for the beach pit | the same pit |
| `m2b-river` | one water assert became `gravel`, `slides > 0` added | the gravel bed slides into the trench's near end |
| `m2c-replay`, `m1-done` | none | they pass as they were |

Tests: `sand_collapses_in_a_chain...`, `a_block_built_into...`,
`updates_outside...` and `a_falling_block_waits...` run under `straight_down()`
(the old rules), because each needs a block to stay where it lands.
`a_pit_dug_at_the_shore...` does too, and
`a_pit_dug_in_a_beach_slumps...` is its new-rule twin. `the_same_inputs...` counts
falls plus slides. The 5,041-block stress test's accounting includes slides.
`the_hash_covers...`, the snapshot round trip and the refusal test were extended.

Outside `sim/`: small additions to `main.rs`, `session.rs`, `play.rs` and
`replay.rs` for the switches, which had to reach the command line, the harness
and the replay format. Nothing in `mesh.rs`, `worldgen/`, `player.rs`, `ui.rs` or
`app.rs`.

## Not verified

- **Nobody has played this.** No window was opened. Everything is headless, so
  the feel is unjudged, and the longer stats line has not been seen on screen.
- **No in-game switch.** `--film` is command-line only: `ui.rs` is B5's.
- **Generated terrain.** I measured one beach and one river bed. I did not
  survey steep generated sand or gravel for how big a natural slide can get
  once something touches it.
- The cost numbers are all provisional, and the dam break's 81 ms tick is
  unexplained.
- Water waiting at the region's edge is untested on its own; the powder case
  is tested. `seek` is tested across a cube edge but not a cube corner.
- `merge` beside a boundary reservoir is untested.
- The tests ran in release only. M2b warned that release hides `u8` overflow.
  The new additions are bounded by checks, but I did not run a debug build.

## For Andrew to decide

1. **Should poured water dry up?** The default makes a bucket on flat ground
   disappear in about three minutes. `--film seek` keeps it. This is the main
   call: it is the first time water leaves the world on its own.
2. **Which film rule.** `seek+dry` is my recommendation, but the frames are the
   real comparison.
3. **Does the sand slope feel right?** One block per block is fixed by this
   rule. If it should be gentler or vary by powder, that is #8.
4. **Beaches and river beds now slump** when dug. Holes in sand no longer keep
   their shape, and a trench beside a river takes the bed with it. Wanted?
5. **Natural slopes stand until touched.** A generated sand slope steeper than
   the rule allows stays until something reaches it, and then it goes. That
   matches "generated rivers stay frozen until disturbed".

## Feedback

- Loading a copy of a real save found the worst bug of this mission, which every
  test passed. Worth doing whenever the rules change.
- The one-process alternating cost test gave a clean number in a minute, when
  separate A/B runs on a busy machine could not. Worth keeping for the next rule.
- `fill` and `levels` made scenes one line each.
