# B1 water surfaces: findings

Branch `worktree-agent-ab44a90b6f16ca22b`. Evidence is in
`docs/missions/B1-water-evidence/`.

| Item | Result |
|---|---|
| #1 water under a solid | Fixed. Andrew's recording, same step: the hole is gone |
| #10 the dip at the shore | Fixed, and why it went away on update is measured |
| Hatched patch on shore walls (not on the list) | Found while checking #10, fixed |
| Review 3 #8, gaps at joins | Fixed, both cases, plus a third cause |
| Hairlines | Cause proven. **Not fixed: it is in B2's `block_at`** |
| Stair-step | Reproduced, no shared cause, not fixed |
| Hill crack, dark river triangle | Not reproduced |

Tests: **172 passed, 0 failed**. All 8 scripts pass and all 5 recordings
reproduce exactly. Water triangles fell or held in every view measured.

**The commit messages for #1, #10 and #8 say NOT YET RUN.** That was true when
they were written, because the linker was gone. All of it has run since. The #10
message also says face emission reads the padded ids, which the next commit
changes.

## What was wrong

### #1, water under a solid

Andrew's "slab" in recording 040850 is a one-block-wide stone wall in layer 513,
over a river whose top cells in layer 512 hold levels 105 to 123. So there is
half a metre of air between the water and the stone. This is not the coplanar
case the bug table expected. The top face was culled because the cell above was
solid, so a strip of surface half a metre below the stone was missing and you
looked through it at the bed.

A top face now hides only under water, or under a solid whose underside it lies
in. A full cell under a solid stays hidden: its top would z-fight with the
solid's underside and tint a view from below twice. A surface that slopes up to
the solid still draws.

- `1_recording_040850_before.png`, `1_recording_040850_after.png`: his frame.
- `1_above_*`, `1_side_*`, `1_underwater_*`: the three views the mission asked
  for, from `scripts/b1-water-under.play`.

### #10, the dip at the shore

**The bug table's reading was half right.** Air does count as height 0, but at a
resting shore the air does not exist. The mesher reads padded cells, and the
generator lowers a chunk's border columns by its 2-voxel skirt so the terrain
grows side faces that hide LOD seams. So the next chunk's ground reads as air.

A temporary audit listed every top-face corner below its own cell's level, and
what pulled it down, on the unfixed build:

| Site | Dipped cells | Skirt ghost | Real air |
|---|---|---|---|
| `find lake`, no edits | 22 | 22 | 0 |
| Andrew's river | 71 | 54 | 17 |
| m2b-river site | 72 | 64 | 8 |

- At the lake, all 22 dipped cells are on chunk borders.
- Every real-air cell in all three river samples is over air, none over water
  or ground: the lip of a waterfall. That is M2b's deliberate slope, so real air
  still counts, and `a_fall_keeps_its_lip` holds it.
- The two sea shores sampled had no dips: with no partial water they draw flat.

**Recording 040850 does not show this bug.** Its "dark wedges" have the same
shapes with the water hidden (`10_wedges_040850_with_water.png`,
`10_wedges_040850_water_hidden.png`): they are riverbed steps seen through the
water. The `find lake` site is the real reproduction: `10_dip_before.png` shows a
V pulled down 47 levels (18 cm) at a chunk corner, and `10_dip_after_fix.png` is
the same pose.

**Why it went away on update.** The simulation does not fill that air: those
cells are sand, asserted before and after. `overlay_lod0` copies an edited
neighbour's true cells over the skirted padding. So a dip clears when the chunk
holding the ghost cell is edited and the dip's own chunk remeshes. Four runs at
one corner where four chunks meet (i 115488, j 64096), on the unfixed build:

| Action | Corner | Audit | Frame |
|---|---|---|---|
| Nothing | `a0 a0 a0 w204` | dip 47 | V visible |
| One bucket, stays inside one chunk | unchanged | dip 47 | not looked at |
| Three stones on dry sand, simulation off | ghosts gone | 0 | **V still there**: nothing remeshed the chunk |
| Water poured across the chunk borders | `a0 s0 s0 w214` | dip 19 | nearly flat |

The 19 that remains is the ghost in the fourth chunk, which is dry and which
water never reaches. So Andrew's "goes away" is real but partial, and depends on
where he poured. The stones row shows the audit alone was not enough: it
recomputed a mesh the streamer never rebuilt. Logs: `why_update_*.log`.

The terrain still needs the skirt, so the padded ids stay.
`World::padded_levels` now also lists the border cells that are really solid, and
only water reads them.

### The hatched patch (not on the list)

The first after frame of #10 still had a hatched triangle on the shore wall, and
so did the before frame. I had left water's face emission on the padded ids,
guessing those faces helped at LOD seams. A water side face against hollowed
ground lies in that ground's own wall and z-fights with it. Water's faces now
read the real cells too, and the patch is gone.

I then assumed the sea had the same problem and added a branch for it. A test
failed on that assumption. A scan of 16,000 chunks found 8,206 of full water, none
with hollowed air beside it: near the sea the skirt hollows ground into water,
which has no side face. I removed the branch, so the sea takes no new lookups.

### Review 3 #8, gaps at joins

- **Chunk joins.** A corner with nothing partial at it is now full, air or not,
  which is what the flat shortcut draws. With this fix off the test reports
  Astra's exact 232.
- **Cube corners.** The padded diagonal does not exist there. It is now left out,
  so the three real cells count once each.
- **A third cause Astra did not list.** Each chunk saw the other's border columns
  lowered by the skirt. `chunks_agree_on_every_shared_water_corner` fails on this
  with the #10 fix off.

### Hairlines: cause proven, not fixed

**It is not the sea, and not a crack.** The surface in recording 041049 is a lake
at +12.03 m. `block_at` rounds it to the voxel grid separately at each LOD:

| LOD | 0 | 1 | 2 | 3 | 4 | 5 and up |
|---|---|---|---|---|---|---|
| Water top (blocks above sea) | 12.03 | 12 | 12 | 16 | 16 | none (#2) |

So there is a 4 m step at the L2/L3 boundary with no wall, and from inside the L2
ring you look under the L3 sheet at the bed.

| Test | Hit pixels (4K) |
|---|---|
| As shipped | 4605 |
| Sag threshold moved | 4606: not the cause |
| Water depth test off | 4605: no water is drawn there at all |
| **L2 water raised one voxel, level with L3** | **0** |

Doubling the resolution gave 3.87× the pixels: a gap of fixed width in the world,
not rasterisation. At those pixels the water frame and the no-water frame are
identical. Vertices on the shared edge agree to 2 mm.

**Proposed fix, after B2 lands:** give coarse LODs a fractional water level, so
each draws the true surface inside its top voxel, as LOD 0 does. The water quad
format already carries the height. It needs `Generated::levels` at coarse LODs
and the `lod == 0` filter in `mesh_chunk_levels` lifted. Any lake off the grid has
this, by up to half the coarser voxel. If #2 is fixed by rounding alone, the same
step will appear at more boundaries.

### The three "only if they share a cause"

- **Stair-step: reproduced, no shared cause, not fixed.** Drawn identically with
  water hidden (`stairstep_with_water.png`, `stairstep_water_hidden.png`). It is
  a seabed terrace seen through the water, at L5 and coarser.
- **Hill crack: not reproduced, not attempted.** M3b found it in opaque terrain
  quads, which none of this touches.
- **Dark river triangle: not reproduced.** No recording or pose exists for it. The
  hatched patch appeared at a chunk border on a river too, so it may be the same
  thing. That is a guess.

## Verification

**Tests.** `cargo test --release`: 172 passed, 0 failed, 14 ignored
(`tests_full_fixed.txt`). Seven are new, in `mesh.rs`. Each fails with its own fix
off and passes with it on (`tests_with_each_fix_off.txt`):

| Test | Off | How it failed |
|---|---|---|
| `water_top_shows_under_a_solid_unless_it_touches_it` | #1 | no surface under the stone |
| `a_resting_lake_does_not_dip_at_chunk_borders` | #10 | a cell not flat at its level |
| `chunks_agree_on_every_shared_water_corner` | #10 | chunks disagree on a corner |
| `a_fall_keeps_its_lip` | air ignored | the guard: catches a fix that flattens flow |
| `no_water_side_face_lies_against_real_ground` | side faces | a face against ground |
| `full_water_meets_across_a_chunk_with_no_levels` | #8 joins | 232 |
| `three_faces_agree_at_a_cube_corner` | #8 corners | not the mean of the three cells |

**Scripts**, final code (`scripts_and_replays_fixed.txt`): m1-done 94 asserts,
m2b-water 97, m2b-boundary 44, m2b-river 41, m3a-tunnel 20, b1-water-under 10, all
passing. `water-lod-seam` and `-high` have no asserts, so I compared frames
against the same build with every fix off: 0 pixels differ in all 6.

**Recordings.** All five reproduce exactly on the fixed build, and 040850's end
hash is unchanged (`b1e6dabf3080bb62`). Nothing here touches the world or the
simulation, so no generator or `sim_rules` bump is needed.

**Taking the commits separately.** They are a stack, and each passes its mesh tests
with those below it: #1 4 of 4, #10 7 of 7, #8 9 of 9 (`tests_each_commit.txt`).
The later ones depend on earlier ones (#10 uses #1's test helpers; #8 and the
side-face fix build on #10's `PaddedWater`). Out-of-order cherry-picks were not
tried.

**Quads**, water triangles drawn, one build with every fix off against the fixed
one. Provisional, though triangle counts do not depend on load:

| View | Before | After | Change |
|---|---|---|---|
| Ocean, low | 48,136 | 48,136 | 0 |
| Ocean, 1.5 km | 69,960 | 69,960 | 0 |
| Sea shore | 98,808 | 98,806 | -2 |
| Lake shore, the dip's pose | 95,140 | 95,108 | -32 |
| Lake from 300 m | 49,732 | 49,660 | -72 |
| River with falls | 75,354 | 75,162 | -192 |
| Andrew's river (without his stones) | 60,756 | 60,692 | -64 |

Opaque triangles are unchanged. A top face under a solid adds quads only where
partial water runs under one, which these views do not contain.

**Border lookup cost**, provisional, 22 lake chunks (`border_lookup_cost.txt`):
0.09 ms per chunk with neighbours cached, against 7.3 ms to generate and 3 ms to
mesh. **Cold it is 10.7 ms**, because a miss generates a neighbouring chunk. That
is on a worker thread, so it delays a mesh rather than a frame. The sea does not
pay: it never reaches the lookup.

## Not verified

- **The real window.** Every frame here is harness-rendered, as the round requires.
- **`--bench`, or any frame time.** Other missions were running throughout, and
  wall times swung by a factor of 3 between identical runs, so I report none.
- **The cold lookup during a real flight.** The 10.7 ms is a worst-case block of 25
  chunks. How much streaming slows over a large lake is unmeasured.
- **By frame:** a full cell under a solid, a sloped surface under a solid, and a
  real cube corner. Those three are proven by tests only.
- **A shore at an LOD boundary.** Dropping those side faces removes a translucent
  film there if the coarser chunk's ground is lower. I expect that to be invisible
  and did not look.
- **Coverage.** The dip audit sampled four sites and the shore scan covered parts
  of face 2. A resting shore elsewhere could behave differently.
- **Andrew's machine.** The final binary was linked with the MSVC 14.44 tools on E:.

## Proposed wording for STATUS.md

> B1 water surfaces: water's top face shows under a solid, and resting water no
> longer dips at chunk borders (the mesher read the skirt as air). Corners meet
> across chunks and cube corners. The hairlines at 041049 are a lake rounding to
> different heights at different LODs: open, after B2.

## For Andrew

Nothing needs deciding now. Two things to look at when you next play:

- Full water beside air no longer slopes at its edge. A chunk with no partial water
  already drew it this way, and simulated water normally spreads out of that state
  within a tick. Frozen water can hold it, as the placed column at i 109775 in your
  040850 save does.
- At the lip of a waterfall the surface still leans out over the drop. That is
  deliberate, but it is the one place a resting surface still slopes.
