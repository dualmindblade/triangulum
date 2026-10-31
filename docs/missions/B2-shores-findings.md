# B2-shores findings (#2 lakes from space, #4 ocean bands, #5 chaotic beaches)

**State: #2 and #4 are done, measured and gated. #5 works but isn't finished
verifying: do not merge it yet.** The final gate run was stopped by the system
because the machine ran critically low on memory, and I was told not to restart
it without being asked. "Not verified" below lists what that left open. One
`sim/` test also fails on #5 until a one-line fix lands in B4's file.

Commits on `worktree-agent-a5287d4d55ddda21c`:

| Commit | What | State |
|---|---|---|
| `47acbe4` | #2 lakes and rivers keep their water | gated; merge candidate |
| `edcb5f5` | #4 same floor through the water at every LOD | gated; merge candidate, see the warning under #4 |
| `44b1a6d` and the commits after it | #5 coasts, generator version 4 | **WIP, do not merge** |

Gates on the snapshot of `edcb5f5` (#2 and #4): `cargo test --release` 167 passed,
0 failed; `water_is_contained_wide` 50 million water cells, 0 walls, 0 perched;
at 1/8 `water_is_contained`, `water_agrees_across_lods` and both new tests pass;
`m2b-water`, `m2b-boundary` and the water-lod-seam pair pass. `m1-done` and
`m2b-river` each failed one timing assert in that run (5 fps, 12 ms tick). That was
machine load, not the change. Taking turns with the frozen build under the same
load, the frozen build failed `bench_fps` once at 55 fps and the snapshot passed
all its runs (108 and 919 fps, same 1,077 draws).

## #2: lakes from space

The lead's reading was right, and it was the only cause. On the chunks the
game really builds (2,298 lake field cells, 19,227 river course points; all
measured with `shore_water_by_lod`):

| LOD | voxel | lake columns losing water, before | river columns, before | after |
|---|---|---|---|---|
| 0 | 1 m | 0 | 0 | 0 |
| 1 | 2 m | 0 | 2.2% | 0 |
| 2 | 4 m | 0.0% (245) | 11.4% | 0 |
| 3 | 8 m | 6.8% | 28.1% | 0 |
| 4 | 16 m | 55.1% | 52.0% | 0 |
| 5 | 32 m | 78.0% | none in the sample | 0 |
| 6 | 64 m | 83.7% | none | 0 |
| 7 | 128 m | 84.6% | none | 0 |
| 8 | 256 m | 84.4% | none | 0 |
| 9 | 512 m | 83.3% | none | 0 |

Every lost column had its ground at or above the rounded surface: the "other
reason" count is 0 at every LOD. The sea is exactly on the voxel grid, so only
lakes and rivers lost out. Rivers lost water too, and worse at LODs 1-4.

Also found: the "sea" Andrew flew over at the end of 041049 is a lake (W 12 m).
That is why he saw grey speckles there; see #4.

The fix drops the extra `h < w_r` test. It holds at every LOD: after it, water
on dry columns 0, perched 0, and walls unchanged at every LOD (lakes 0; rivers
219, 63, 21, 18 at LODs 1-4, the same counts as before, one-voxel steps along a
river's own fall). Gates: new `lakes_and_rivers_keep_their_water_at_every_lod`
(failed before: LOD 1, 405 of 157,357 columns), `water_is_contained`,
`water_agrees_across_lods`.

Rivers past LOD 4-5 aren't drawn at all (`CHANNEL_MAX_LOD`, by design: the
"none" rows). That is not this bug.

## #4: ocean bands

Also as the lead read it. What shows through the water at the same 9,216
columns per site (`shore_floor_by_lod`), share agreeing with LOD 0:

| LOD | 041021 before | after | 041200 before | after |
|---|---|---|---|---|
| 1 | 99.9% | 99.9% | 99.8% | 99.9% |
| 2 | 99.8% | 99.8% | 99.8% | 99.9% |
| 3 | 91.7% | 99.6% | 7.9% | 99.9% |
| 4 | 78.6% | 99.1% | 0.4% | 99.9% |
| 5 | 56.7% | 98.3% | 0% | 99.6% |
| 6 | 12.4% | 97.5% | 0% | 99.4% |
| 7 | 0% | 96.6% | 0% | 99.3% |
| 8 | 0% | 96.1% | 0% | 99.3% |

Before, LODs 6-8 showed umber, mudstone, limestone, shale and sandstone: strata
a voxel or two down. After, only sand and gravel. The remaining few percent are
columns next to the sand/gravel line.

A wet coarse column's ground now ends a voxel under the snapped surface. The
coarse bottom voxel under water is the floor block, not bedrock. `deep_ocean`'s
colour in `blocks.json` is now water over gravel, (0.104, 0.145, 0.243), which
is the one edit outside `worldgen/` and scripts in these two commits.

From orbit over open ocean (review 2 #12, `ocean-orbit.play`, midnight and
noon): 0 pixels show through at 150 km or 75 km; the L9-L8 step is (2.4, 0.6,
-1.7) against (3.0, 1.2, -1.3) before, out of 255. Median night luminance went
from 47.5 to 45.3: a touch darker.

**Warning for the merge.** `edcb5f5` alone makes shelves grey from orbit. LOD 0
has sand down to 6 m, which at 1/70 is whole shelves, and sand under water at
alpha 0.62 reads grey. The rock bands used to hide that from far away. So at
LODs 1-8 shelves and the 041200 inland sea now read as flat grey, then vanish
at L9 where the sea is opaque (`sheet_4_sea_and_orbit.jpg`). That is what
matching LOD 0 means, and the bands are gone, but it's short of "stay blue from
orbit" near coasts. Open ocean stays blue. Staying blue over shelves needs less
sand at LOD 0, which needs the generator bump, so it's in the #5 work, and a #5
frame from 40 km shows the shelves gone (`after-5-partial/sheet_4_sea_and_orbit.jpg`).
Your call whether to merge #4 now or hold it for #5.

The speckles at 041049's end are unchanged by #4: they're LOD-0 sand strewn on
a gravel lake bed by the dither (the line is at 2.5 ± 1 m and the bed is about
3.3 m deep). The #5 work moves the line, and they're gone in its frames.

## Lake surfaces at LOD boundaries (B1's hairlines): not fixed, needs `mesh.rs`

B1 found the hairlines at 041049's end are a step in a lake's surface: W 12.03 m
snaps to 12 m at L2 and 16 m at L3, and from inside the fine ring you look under
the coarser sheet. **That is confirmed and I have not fixed it.** Rounding alone
can't: any lake off the voxel grid has a different surface at different LODs. My
#2 fix doesn't make it worse at the poses measured, and doesn't cure it.

Measured on real lakes at 4K (`game/scripts/b2-seam.play`, `seam_holes.py`).
Holes are pixels with no water drawn, in open water; my count at B1's pose agrees
with its 4,605 to 1%. Doubled is the share of water with two sheets over it:

| Lake (W) | Holes: rounding | snap down | fractional | Doubled: rounding | snap down | fractional |
|---|---|---|---|---|---|---|
| 12 m (B1's pose) | 4,565 | 1 | 15 | 0.0% | 0.0% | 0.0% |
| 6.8 m | 1 | 2 | 2 | 1.9% | 10.5% | 1.7% |
| 18 m | 423 | 427 | 625 | 0.6% | 0.5% | 0.1% |
| 16 m | 1 | 1 | 0 | 1.1% | 1.3% | 0.4% |
| 4 m | 2,592 | 2 | 2 | 0.1% | 41.8% | 0.0% |
| 1.5 m | 0 | 0 | 0 | 0.1% | 0.1% | 0.0% |

The 18 m lake's holes are tiny islets, not seams (looked at by painting them on
the frame; that count was not checked again for the fractional run).

**Snap down (rejected).** Taking the voxel below W is the only fix inside
`block_at`, and the hole count looks perfect: the camera is always on the finer
side, and a coarser surface that is never higher can't be looked under. But it
draws rectangular patches of darker water across lakes. Where the finer surface
sits above a chunk plane and the coarser below it, the streamer can draw both,
because it picks LODs by 3D distance. The colour is two sheets of water: (68, 100,
146) measured against (65, 97, 141) worked out from the blend. 41.8% of one lake
went dark, which is much worse than a hairline. That mechanism is my reading of
the frames and the colour; I did not trace it in the streamer. It also leaves
lakes up to a voxel below their land.

**Fractional surface (recommended, B1's proposal).** Coarse LODs draw the true
surface inside the voxel holding W. It clears the seams and the patches, and
`lake_2500` comes out as one clean sheet. It needs `mesh.rs` (lifting the
`lod == 0` filter), so per the mission I left it alone. The experiment is saved as
`B2-shores-evidence/coarse-levels-experiment.patch`, **not applied on this
branch**, and it touches three files:

- `mesh.rs:238`, one line, B1's.
- `world/mod.rs` `padded_levels`, one line. An edited coarse chunk isn't handled.
- `worldgen/mod.rs`, a few lines.

It is not ready to merge as it stands. Before it lands:

- **It breaks "no walls at a coarse LOD".** Where a lake rounds down, land rounds
  down with it, so the true surface stands above the land beside it, by under
  half a voxel. Lake columns affected, sampled on every 9th lake: 0.2% at L3, 1.6%
  at L4, 6.3% at L5, 17% at L6, 31% at L7, 54% at L8, 79% at L9. Rounding has none.
  Whether a low blue curb beats a hairline is a look call for Andrew.
- `water_agrees_across_lods` fails with it: 16,171 disagreements at L2, because
  the test reads a voxel's top at coarse LODs. The test needs teaching, and until
  it is, it isn't evidence either way.
- It was gated on an earlier state of #5, and it wasn't run against B1's changes
  to `Surface::corner`.

## No generator bump for #2 and #4: claim checked

- `shore_lod0_digest`: 5,988 LOD-0 chunks around lakes, rivers and the recorded
  coasts hash to `1f003c7d...` on the code before and after.
- All five recordings of 2026-10-30 reproduce exactly on the #2 + #4 build.
- The `deep_ocean` colour change touches no rule (the rules digest hashes names
  and flags, not colours).

## #5: chaotic beaches

### Diagnosis (measured)

At 1/70 the bake's height near a coast is around a metre, less than the
landform's noise, so sea level cuts through the noise and the noise's contour
*is* the coast:

- Gullies draw star and saw-tooth shapes along it (`before_041021_LOD0.png`
  shows the teeth).
- The 041200 inland sea is about a metre deep in the bake (read off the height
  map, not measured), so the noise breaks the surface all over it: sign(h)
  differs from sign(bake height) on 13.9% of the columns there, 2.2% at 041021.
- The sand band's inland edge follows the `ocean > 0` edge of a bilinear mask,
  so it shows the raster's texels as stairs.

Baseline, LOD 0 (3 km windows at 2 m), the two recorded coasts and five others
from gentle to steep (quantiles of the bake's coastal slope):

| Coast | blobs and slivers per km | closed pieces | the bake's own |
|---|---|---|---|
| 041021 | 18.5 | 44 | 0 |
| 041200 | 21.0 | 334 | 2 |
| q10 | 20.1 | 485 | 0 |
| q30 | 21.2 | 248 | 0 |
| q50 | 21.8 | 166 | 0 |
| q70 | 6.8 | 6 | 0 |
| q90 | 0.2 | 0 | 0 |

It isn't a 1/70 problem only: at 1/20, 1/8 and half-metre blocks the same five
gentlest coasts measure 3-23 per km, nearly all of them over 8 (full tables to
be added with the "after" run).

### What #5 does

`worldgen/coast.rs`: the shoreline first, the ground after it. A signed distance
from the bake's 0.5 `ocean_frac` contour, worked out on the sphere when the bake
loads (**no bake change, no re-bake**), plus a wiggle of slope under 1 (steepest
measured 0.83), which can move the shoreline but can't make an island. The ground
is then a function of that distance in metres, so it doesn't depend on the
vertical scale. Sand follows the curve and runs a short way under the water; past
that the sea floor is gravel. Lake-bed sand stops short of the bed's depth.

Beyond `worldgen/`, the generator bump took small edits to `world/save.rs` and
`replay.rs` (the refusal text) and `play/find.rs` (`find shore` reads the new
coast). `m2b-water.play`, `m2b-boundary.play` and `probe-shore.play` move to the
new shore at (1986, 59004).

### Results at 1/70

Measured with the wiggle and shore profile as committed, but before the last two
fixes (river banks, underwater sand width), which don't move the sea mask away
from rivers. LOD 0, 3 km windows at 2 m:

| Coast | blobs and slivers per km: before | after | closed pieces: before | after | bake's own |
|---|---|---|---|---|---|
| 041021 | 18.5 | 0.00 | 44 | 0 | 0 |
| 041200 | 21.0 | 0.00 | 334 | 1 | 2 |
| q10 | 20.1 | 0.00 | 485 | 0 | 0 |
| q30 | 21.2 | 0.00 | 248 | 0 | 0 |
| q50 | 21.8 | 0.22 | 166 | 0 | 0 |
| q70 | 6.8 | 0.00 | 6 | 0 | 0 |
| q90 | 0.2 | 0.38 | 0 | 0 | 0 |

From altitude (LOD 5, 16 km at 32 m) the worst coast went from 2.59 to 0.52 per
km, and closed pieces match the bake to within one everywhere. Sand reaches
98-100% of the shoreline at LOD 0, in 1-6 runs per window against up to 265. q90,
the steepest, was already clean; it rose from 0.2 to 0.4, two slivers on 5 km.

Frames (`after-5-partial/`, sheets 2-5): the torn coast is one line with a
continuous beach, the 041200 inland sea is one body of water with its islands,
the sea is even blue from 40 km with no grey shelves, and the shore holds up at
the water line. **Those frames were shot on the snap-down build**, before the last
two fixes. The coast, beach and sea in them should stand. The lakes in
`sheet_1_lakes.jpg` do not: its dark patches are the rejected snap-down, kept to
show why.

### Bugs running it found

- A step in the ground behind a coast, where the land's floor stopped short over
  low land. A unit test caught it; it fades out now.
- **72 water walls at LOD 0**, from `water_is_contained`: sea-level rivers beside
  dry ground below sea level. River banks were skipped at sea level because "the
  sea and the coast hold it", which stopped being true once the shoreline became
  exact. Banks now stay on the land side. Afterwards `water_is_contained_wide`:
  50 million water cells, 0 walls, 0 perched.
- A coarse LOD showed far more underwater sand than LOD 0, because the 1.5-voxel
  minimum width applied below the water too. It now applies to dry sand only.

### Gates on the final game code

The run on the final code (`c5`; the later `c6` build was a no-op for the game, so
these hold for it): `water_is_contained_wide` 0 walls, 0 perched. At 1/8
`water_is_contained`, `water_agrees_across_lods` and both new tests pass. `m1-done`
94 of 94, `m2b-river` 41 of 41, the water-lod-seam pair passes. `cargo test`: 170
passed, 2 failed, both below.

### What still fails

1. **`sim::water_tests::a_pit_dug_at_the_shore...`, in B4's file.** The water total
   is off by exactly -255. It is the test, not the simulation: its 3 x 3 pit assumes
   a straight shore, and on the new coast cell (1986, 59002, 511) is sea, so the pit
   deletes one sea cell. I proved this fix with a temporary edit, then restored the
   file; the test passes with it. In the pit loop:

   ```rust
   let cell = BlockPos { i: land.i + di, j: (land.j as i64 + dj) as u32, k: land.k - dk, ..land };
   if w.block_at(cell) != w.water_id() {
       writes.push((cell, air));
   }
   ```

   `sim/` is B4's, so it isn't on this branch. **#5 can't merge without it.**
2. **`coasts_are_coherent`, my gate, has never passed.** It failed at c5 on 1 sliver
   in 0.87 km (1.14 per km against a bar of 1.0), a small-window effect. I doubled
   the window; that run was stopped before reaching it. It also stops at its first
   failure, so **1/20, 1/8 and half-metre blocks have no "after" coast numbers at
   all.** The "before" numbers there are 3-23 per km.

### Refusal

The old message told players to "start with the saved settings", which can't
work for an old generator. The new one says the save is untouched, that the
build that made it still plays it, and to use `--save saves/another-name`.
`save.rs`'s test asserts the wording and passes. Replays name the changed terrain
and say to use the build that recorded them. **What a player sees on a real old
save was not run.** Refusal comes before any window opens.

## Verified and how

- `lakes_and_rivers_keep_their_water_at_every_lod`, `water_shows_the_same_floor_at_every_lod`:
  each failed before its fix and passes after.
- #2, #4: every gate listed at the top; frames at matching poses against the frozen
  build `945d544` in `B2-shores-evidence/after-2-and-4/`.
- No bump for #2 and #4: digest and replays, above.
- Orbit, open ocean, on `edcb5f5`: nothing shows through.
- #5: the gates and numbers above. In the stopped final run 164 tests passed and
  only the `sim/` test failed, including the coast unit tests, cube-edge continuity
  of the coast, and the refusal test; 7 tests had not finished.

## Not verified

The final run was killed for low memory before it got to these:

- `coasts_are_coherent` at its new window, and coast numbers at 1/20, 1/8 and
  half-metre blocks. **The "every vertical scale and ½ m blocks" requirement is
  unproved.**
- `m2b-water` and `m2b-boundary` after my last edit. They were at 4 and 2 failures
  and I fixed the cause, but haven't seen them pass.
- The 7 unfinished tests on the final build, among them `footprint_bounds_contain_every_sampled_column`,
  which guards the new height bounds. It passed at c5, on the same game code.
- Final frames: nothing has been shot on the final code. The refusal on a real old
  save. The orbit check (review 2 #12) on #5.
- Generation cost and bake load time for #5. The distance field adds work at load,
  and I have no number for it.
- Anything in a real window (none allowed in this round). All timing here is
  provisional: other agents were running. The #2/#4 snapshot's `m2b-river` total
  run time looked slower than the frozen build (8-11 s against 4-11 s), which is
  noisy but unexplained.
- The frozen build exited 139 (a crash) once, after all its asserts passed, on
  `m2b-river`. It's not from this work, but someone should know.

## For Andrew

- **Hairlines or a curb?** See the LOD boundary section. It's his call whether a low
  curb of water beside land, under half a voxel, is better than hairlines.
- Beach look, all dials in `coast.rs`: slope 1:28, top 1.6 m, about 70 m wide, sand
  about 40 m out under the water, sea at least 2 m deep. The beach is narrower than the
  old 300-400 m band, and that's the biggest visible change.
- The sea floor past the shallows is one block, gravel, at every LOD: consistent
  now, but it's his "single grey block". Variety there is a look choice. Sand under
  0.62 water reads grey because alpha doesn't change with depth; a tint by depth
  would be the real answer to pale shallows, and that's renderer work.
- `deep_ocean`'s colour moved a little, darker and slightly bluer: (0.104, 0.145,
  0.243).

## Proposed wording

For `STATUS.md`, once #5 is merged: "Saves and replays from before this are refused
(the terrain changed); the build that made them still plays them."

For `DESIGN.md`, Terrain: "The coast is a smooth curve from the bake's coastline
with bounded detail; noise never decides where the sea ends. (2026-10-31)"
