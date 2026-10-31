# B2-shores findings (#2 lakes from space, #4 ocean bands, #5 chaotic beaches)

**State: #2 and #4 are done and measured. #5 is written and type-checked, but
it has never been linked or run**, because the machine lost its linker partway
through (the SDK and C++ workload removal). Nothing below about #5's result is
known yet. The sections say which is which.

Commits on `worktree-agent-a5287d4d55ddda21c`:

| Commit | What | State |
|---|---|---|
| `47acbe4` | #2 lakes and rivers keep their water | measured; merge candidate |
| `edcb5f5` | #4 same floor through the water at every LOD | measured; merge candidate, see the warning under #4 |
| `44b1a6d`, `bcc5be3` | #5 coasts, generator version 4 | **WIP, do not merge**: never run |

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
sand at LOD 0, which needs the generator bump, so it's in the #5 commit. Your
call whether to merge #4 now or hold it for #5.

The speckles at 041049's end are unchanged by #4: they're LOD-0 sand strewn on
a gravel lake bed by the dither (the line is at 2.5 ± 1 m and the bed is about
3.3 m deep). The #5 commit moves the line.

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

### What the WIP commit does (not yet run)

`worldgen/coast.rs`: the shoreline first, the ground after it. A signed distance
from the bake's 0.5 `ocean_frac` contour, worked out on the sphere when the bake
loads (**no bake change, no re-bake**), plus a wiggle of slope under 1, which
can move the shoreline but can't make an island; the ground is then a function
of that distance in metres, so it's scale-independent. Sand follows the curve
and runs a short way under the water; past that the sea floor is gravel, which
should also fix the grey shelves. Lake-bed sand stops short of the bed's depth,
which should remove the speckles.

The generator goes to 4. Beyond `worldgen/`, that took small edits to
`world/save.rs` and `replay.rs` (the refusal text) and `play/find.rs` (`find
shore` reads the new coast).

### Refusal

The old message told players to "start with the saved settings", which can't
work for an old generator. The new one says the save is untouched, that the
build that made it still plays it, and to use `--save saves/another-name`.
Type-checked, and `save.rs`'s test asserts it, but not run. Saves are refused
before any window opens.

### Expected fallout, to fix once it can run

`scripts/m2b-water.play` and `m2b-boundary.play` hard-code a shore at face 1,
i 1803: the shoreline will move, so they'll need new coordinates. River mouths
may need a change if the shoreline lands upstream of a river's sea-level
stretch; `water_is_contained` will show.

## Verified and how

- `lakes_and_rivers_keep_their_water_at_every_lod`, `water_shows_the_same_floor_at_every_lod`:
  each failed before its fix and passes after.
- `water_is_contained`, `water_agrees_across_lods`, `every_ocean_column_is_opaque_underneath`,
  `coarse_lods_keep_the_ocean`, `bedrock_floor_at_layer_zero`, `far_ocean_matches_the_shipped_colour`.
- Before and after frames at matching poses (frozen build `945d544` against a
  build of `edcb5f5`): `B2-shores-evidence/after-2-and-4/`.

## Not verified

- **All of #5**: no test, frame or number has come from that code.
- The full gate run on `edcb5f5` is listed in the section below when it finishes.
- Water-line frames at #2 and #4: unchanged by design (LOD 0), not looked at closely.
- LOD boundaries across a lake: the surface snaps differently per LOD (W 12 m is
  16 m at L4, 0 m at L5), so there's a step at a boundary, as ground has. Not
  measured; B1's seam work may meet it.
- Nothing in a real window (none allowed in this round). No timing was taken.

## For Andrew

- Grey shelves (above): sand under 0.62 water reads grey because alpha is flat
  with depth. A depth-dependent water tint would be the real fix; that's
  renderer work, outside this mission.
- The sea floor past the shallows is one block, gravel, at every LOD: consistent
  now, but it's his "single grey block". Variety there is a look choice.
- `deep_ocean`'s colour moved a little, darker and slightly bluer.
