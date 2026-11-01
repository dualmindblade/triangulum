# T3 boundaries findings (round 2 #4)

Andrew asked that every boundary be a line that is rough at every scale, from the
size of the feature down to a block, and does not break up into scattered pieces.
That is what the build now does, on the coast, lake outlines, the snow line, the ice
line, dry grass and deserts. The evidence is before/after frames at his four poses
(`T3-boundaries-evidence/poses.jpg`) and the same lines from 150 km down to the
ground (`ladder_*.jpg`). Start with `poses.jpg` and `ladder_snow.jpg`.

## What was wrong (measured on 18d69c7's code)

The reading in the round page is right about the dither, and it missed two things.

The dither: over Andrew's four windows the bake's gradients are gentle, so a fixed
±1.5 °C or ±40 mm moves a line a long way:

| Window | grad T, °C/km | grad P, mm/km | dither moves the T line | moves the P line |
|---|---|---|---|---|
| lake, 1.5 km | 1.16 | 860 | 1,298 m | 47 m |
| biome, 3 km | 1.72 | 693 | 873 m | 58 m |
| snow, 6 km | 1.31 | 326 | 1,141 m | 123 m |
| polar, 10 km | 0.27 | 387 | 5,645 m | 103 m |

The polar line is 5.6 km of pure scatter. The rain field is much steeper than the
temperature, so the biome blobs in his photo aren't a shredded line: they are the
dither's own 41 m pattern in a band a hundred metres wide.

**Missed 1: the lapse scatters too.** It read the column's whole landform, every
hummock of it, at 0.45 °C per metre at 1/70. Which term does what:

| Line | km of line | small pieces per km | cells in small pieces |
|---|---|---|---|
| snow window, bake alone | 15.8 | 0.06 | 0.00% |
| + lapse | 220 | 3.37 | 1.53% |
| + dither | 473 | 5.74 | 9.24% |
| the game | 424 | 4.94 | 6.09% |
| polar ice, bake alone | 10.2 | 0 | 0 |
| + lapse | 193 | 1.52 | 0.51% |
| + dither | 1,643 | 6.19 | 10.8% |
| the game | 1,634 | 5.79 | 10.2% |

("Small" is under a hectare, 100 m square. Nothing the bake draws is that fine.)

**Missed 2: the sea read its own floor.** The second line of that table's polar half is
sea ice following the landform on the sea bed: the lapse had no idea it was at sea.

Also found: "rivers visibly continue across its shallows" is not water. A channel
under a lake still set `Column::river`, which turned the bed to gravel, and gravel
under 0.62 water looks like the deep. Under a lake or the sea there's now no channel.

## What it does now

`game/src/worldgen/boundary.rs` holds the argument; this is the short form. Nothing is
added to a value before a threshold. A column reads its fields somewhere else, through
a warp of the sphere: `q(k+1) = q(k) + A(k) N(q(k) / L(k))`, octaves of uniformly
decreasing size, 3D on the sphere with no chart.

- **It moves lines in metres.** How far a line moves no longer depends on how gentle
  the field is.
- **It can't make a piece.** Each step is one-to-one (steepest 0.85 over 4 million
  points; the promise needs under 1), and a chain of one-to-one maps is one-to-one
  however many there are. This is `coast.rs`'s argument without its limit on the sum
  of the octaves, and it's what lets the top octaves be strong enough to matter.
- **The ladder.** Octaves of 16, 8, 4, 2 km are positions on the sphere and stretch
  with the radius; 1,024 m to two blocks are metres of ground. That crossover is
  where the coast's old wiggle stopped, and about a dominant mountain wavelength.
  At `--n 262144` there's one more global octave and no gap.
- **Step size.** 0.17 of a wavelength: one step per octave above 128 m, two from 128
  to 32 m, three from 16 m down. With one step throughout, a line looked like smooth
  noise close up, which is his last complaint; two steps of every octave sheared the
  climate into streaks like marble (`too-strong.jpg`, from an early build at 0.18).
- **Height.** The lapse now sees the landform as a 128 m voxel does: hills and
  mountains, not hummocks, bent by the octaves up to 512 m. On the sea it's zero.
- **LOD.** Short octaves come first in the chain, so what a coarse voxel leaves out
  moves the line under two voxels, whatever the long octaves stretch afterwards.
- **Lakes.** `lake_at` reads the field through its own warp, top octave 512 m on
  Neisor (60% of the field's reach). `SHORE_NOISE` is gone. `lake_ends` now uses the
  bent shore, including just beside the course, so a river holds the lake's level
  until it has really left.
- **Coast.** Measured at the same boxes it was half as rough as a lake or a snow line
  (below), so it now reads its distance field through the same kind of warp, top
  octave 2 km, reach under 600 m on Neisor. `wiggle` is gone. It still can't make an
  island, for a better reason than before.
- **Small changes.** Snow now lies on a lake's shore as it does on a beach (a sand ring
  round every frozen lake broke the snow line), the lake bed's sand line lost its
  dither, `find shore` reads the real shore, and the T map reads the warped climate, so
  it draws the lines where they are.

## The numbers

**Scatter** (`boundaries_are_lines_not_scatter`, a gate). Four sites on each of the
snow, ice, dry grass and desert lines, on foot (3 km at 3 m) and from altitude (24 km
at 48 m), on Neisor and at N = 262,144. The old rule is rebuilt from the same fields in
the same windows, so the failing half is in the same run:

| | km of line | pieces under a hectare per km |
|---|---|---|
| new | 2,623 | 0.045, beyond those the bake itself has |
| old | 41,373 | 4.03 |

The old rule has sixteen times the line because a scattered edge has that much edge.

**Roughness at each scale** (`lines_are_rough_at_every_scale`, a gate). In a box of each
size that holds some of a line, how far the line strays from straight, as a share of
the box. A row that stays level is a line as rough at that scale as at any other. Cells
strewn evenly would score 0.29.

| Box, m | 8 | 16 | 32 | 64 | 128 | 256 | 512 | 1024 | 2048 |
|---|---|---|---|---|---|---|---|---|---|
| snow, old | .045 | .056 | .101 | .160 | .195 | .207 | .158 | .160 | .166 |
| snow, new | .080 | .078 | .078 | .061 | .059 | .064 | .075 | .104 | .128 |
| ice, old | .041 | .050 | .112 | .188 | .235 | .260 | .215 | .230 | .239 |
| ice, new | .082 | .078 | .070 | .060 | .057 | .049 | .047 | .037 | .038 |
| dry grass, old | .039 | .049 | .094 | .152 | .168 | .158 | .093 | .097 | .120 |
| dry grass, new | .076 | .074 | .070 | .059 | .054 | .055 | .071 | .097 | .123 |
| lakes, old | .034 | .032 | .046 | .035 | .026 | .031 | .074 | .102 | .095 |
| lakes, new | .076 | .074 | .074 | .062 | .055 | .057 | .076 | .105 | .094 |
| coasts, old | .034 | .020 | .015 | .035 | .024 | .027 | .053 | .068 | .084 |
| coasts, new | .078 | .078 | .075 | .059 | .056 | .055 | .060 | .072 | .084 |

That is Andrew's discontinuity in numbers. Old snow goes from smooth contours of the
dither at 8 m to scatter by 64 m. The old coast and the old lakes have a hole from 16 to
256 m. The gate is the largest step between neighbouring boxes: 1.4 now, 1.8-2.3 before.
The rise in the last boxes of snow and dry grass is two lines in one box, not roughness.
Lakes and coasts are now the same kind of line as each other and as the climate lines.
The "old" lake and coast rows came from 18d69c7's code; the old snow, ice and dry grass
rows from the old rule rebuilt in the test.

**One line at every LOD** (`a_line_is_the_same_line_at_every_lod`, a gate). Three sites
on each of three lines, columns at each voxel size against the finest columns at the
same places, 384 x 384:

| Voxel | columns differing, mean | 99% within | worst |
|---|---|---|---|
| 4 m | 101 | 1.0 voxels | 1.4 |
| 16 m | 253 | 1.0 | 1.4 |
| 64 m | 339 | 1.4 | 32 |
| 256 m | 561 | 1.4 | 14 |

The worst is a patch that barely exists, on a field flat around its threshold, coming
and going whole between LODs. That's why the gate is on the 99th percentile. It's
real and it's rare: 339 columns of 147,456 is the whole of the difference at 64 m.

**Seams** (`boundaries_cross_cube_edges_without_a_seam`). Every side of every face, 256
walks: 99% of steps across an edge are under 0.039 °C and 15.6 mm, and of the steps
beside them, 0.050 °C and 14.4 mm. A single step says little: the warp can stretch a
metre into dozens of metres of the field anywhere on the planet.

**On a slope** (`on_a_slope_the_snow_line_follows_the_contours`). At 1/8, where the landform
stands hundreds of metres high, over 10 mountains the snow line spans 0.29 of the
height in view. This is a stand-in for T2's mountains, which I haven't seen.

**The warp** (`boundary.rs` tests). Steepest step 0.85 over 4 million points. The area
scale through the whole chain stays positive at every voxel size over a small enough
square. Over a 5 cm square at 1 m voxels it reads negative in rare spots, where the
chain stretches that square past the shortest octave. That is the test's square, not a
fold: at 0.5 mm it's positive. At 0.19 I could not be sure of this, which is why the
step is 0.17. Largest move of a column on Neisor: climate 3.9 km, coast 579 m, a lake's
outline under 400 m (a bound, not a measurement).

**The cheap path** (`far_chunks_hold_no_boundary`). About 19,000 chunks beside the lines at
LODs 0-8: no column in any of them takes a different class from the exact path.

**Lakes hold water.** `water_is_contained` (1,515 lake shore windows, 139 km of river):
0 walls, 0 perched. `water_is_contained_wide`: 43.3 million columns, 49.9 million water
cells, **0 walls, 0 perched**. At 1/8: `water_is_contained` 0 walls, 0 perched, and
`water_agrees_across_lods`, `lakes_and_rivers_keep_their_water_at_every_lod` and
`water_shows_the_same_floor_at_every_lod` pass. Getting there took four changes, all of
them from the shore moving: `coast::held`, `lake_ends` looking beside the course,
`SHORE_STRETCH` and `Hydro::lake_yields` (under Merging).

**Everything else.** `cargo test --release`: 374 passed, 0 failed, 32 ignored, on the
final code. Every play script with asserts that I could run, on the final terrain, with
0 failed asserts (asserts in brackets): `m1-done` (94), `m2b-water` (97), `m2b-boundary`
(44), `m2b-river` (42), `m2a-sand` (95), `m2c-replay` (29), `m3a-tunnel` (20), `m3b-hotbar`
(11), `probe-river` (40), `b1-water-under` (10), `b3-corner` (52), `b4-slide` (49), `b4-film`
(10), `b5-menu` (29), `b5-menu-scale` (4), `b5-meshview` (11), `b6-report` (29), `b6-scale`
(5), `b7-walk` (24), `b7-compare` (34), `b7-close` (11), `b7-default-frames` (3),
`edge-corner-flight` (17), `tunnel-wall-reload` (18). The ones with no asserts ran clean:
the water-lod-seam pair, `b2-seam`, `b2-shores`, `m3a-look`, `ocean-orbit`, `probe-shore`,
`probe-hills`. `b4-oldsave-load` can't run: it needs a save from before B4, and any old
save is refused now.

Saves and replays refuse with the generator at 5; the refusal itself is `save.rs`'s test.

## Cost

Time for `chunk_columns` over the same 600 chunks at each LOD, one thread, in turns with
the baseline code. **Provisional:** other agents were running, and the two runs of each
differ by up to 30%.

| LOD | baseline, us | T3, us | chunks taking the exact path |
|---|---|---|---|
| 0 | 2,277-2,547 | 2,876-2,935 | 4% |
| 1 | 2,530-3,104 | 3,158-3,192 | 5% |
| 2 | 2,757-3,143 | 3,454-3,561 | 7% |
| 3 | 2,739-3,060 | 3,214-3,417 | 11% |
| 4 | 2,501-2,894 | 3,387-3,513 | 17% |
| 5 | 2,092-2,481 | 3,205-3,281 | 23% |
| 6 | 1,746-1,943 | 2,731-3,192 | 35% |
| 7 | 1,826-2,099 | 2,627-2,885 | 41% |
| 8 | 2,696-2,814 | 3,359-3,726 | 50% |

About a fifth dearer overall, inside "don't double": +15-20% at LODs 0-3, +30-60% at LODs
4-7. A chunk with no line near it costs 25 climate samples. A chunk with one costs the
warp per column. Coasts and lakes pay per column near the shore. Startup: 13-27 s on both
builds in turns, which says only that any difference is smaller than today's noise.

## Not yours, reported

- **The bare-rock line** (`slope > 0.95 + 0.25 * dither`) still uses the 41 m dither.
  Andrew didn't name it and there is no field to warp, but with T2's mountains there
  will be far more of it, and it's the one long edge left with a dither on it.

## Merging

- **T2.** In `Hydro::shape`, two statements changed: the `channel` line, and
  `SHORE_STRETCH` in step 4's `yields`. The lapse moved from `column_from` into
  `Generator::lapse_c` unchanged: put T2's new formula there. Its input is now
  `lf.sample(...)` at a 128 m voxel, so **a new height that doesn't come through
  `Landform::sample` won't reach the snow line.** Three things exist only because the
  shore moved, and T2's rework may change what they need:
  - `coast::held` runs on `Hydro::shape`'s result: a river valley's soft minimum could
    pull a dry shore below the sea (26 walls).
  - `Hydro::lake_yields`, called in `column_from` straight after `lake_at`: a lake bent
    over a river that stands above it stops short of the river's bank (358 walls, all
    on one river at lat 10.5, lon -72.8, 3 cm above a lake it runs beside).
  - `lake_ends` takes the last point in the band for an outflow, not the first out,
    and looks beside the course as well (5 walls).
- **T1.** Everything is built from `planet.radius` and `planet.block_size` in
  `Generator::new`, and works in metres above sea level. I found nothing that assumes
  layer 512. The seven shore scripts changed their i, not their layers, so T1's layer
  shift applies on top. `t3-poses.play` and `t3-ladder.play` use poses and heights in
  metres, with no layers in them. `coast.rs`'s reach and top octave scale with the radius.
- **Shore scripts.** The coast moved, so `m2b-water`, `m2b-boundary`, `probe-shore`,
  `b4-slide`, `b7-walk`, `b7-compare`, `b7-default-frames` and one powder test moved
  with it: about 79 columns on that row. `m2b-boundary`'s pit has a third layer, since
  the pit no longer drew on the region's edge without it. Any change to the coast
  moves these again.
- **Gates I changed.** `coasts_are_coherent` was written for a smooth line. A line
  rough to the block has teeth, so on foot it now samples at voxels of 2 blocks (it
  sampled every second column of a 1 m line, which draws a thread as a dotted line), a
  sliver is under 3 voxels wide and 100 m² or more, the bar is 2 per km (was 1), and
  pieces are allowed one stray per km of coast. B2's "before" was 18-22 per km.
  `water_agrees_across_lods` now asks for deep water only where the fine columns a voxel
  around are deep too, since a shore's line differs between LODs by a voxel or so.

## Not verified

- **Anything in a real window.** All frames are headless `--play`, as this round asks.
- **The rest of the scripts:** `m1-showcase`, `m2c-video-driver`, `b7-bench` and the 1/8
  look scripts weren't run. They shoot pictures or time things.
- **T2's mountains and T1's slider, merged.** The snow on a slope is tested at 1/8, and
  the big planet at N = 262,144 and, for the coast's reach alone, R = 725 km. The lake
  outline's top octave stretches with the planet, and lakes are not gated at
  N = 262,144 for containment: `water_is_contained` ran on Neisor and at 1/8 only.
- **Half-metre blocks** have the coast gate and nothing else from this mission.
- **A real old save being refused.** The test is `save.rs`'s, on a header.
- **Cost and startup on a quiet machine.** Numbers above are provisional.
- **How the look holds up on foot for long.** I looked at frames at 12-90 m above each
  line and at plan maps at 1 m. I haven't walked one.

## For Andrew

- **Strength.** `STEP_AMP` in `boundary.rs`, 0.17. It can go down freely; at 0.19 a very
  rare step can fold. `CLIMATE_TOP_RADII` sets the largest bends (16 km), and
  `LAPSE_VOXEL_M` how small a hill the snow notices (128 m; 32 m looks like a hundred
  little snowy hilltops). None of them are in `world.json`: a dial there has to go into
  saves, replays and photos, which isn't cheap, so they are constants and each needs a
  generator bump.
- **The pack-ice edge is a clean rough line.** Real pack ice breaks into floes for a
  short way off its edge, and that's about ten lines of code, but it would be small
  pieces on purpose, which is what you asked to be rid of. Say if you want a fringe.
- **Snow on lake shores** instead of sand. One line to put back.
- **Lines have moved from where you saw them**, by up to 3.9 km on Neisor. In your biome
  photo the big dry-grass patch has shifted out of frame: that's the warp's long
  octaves, not a change of climate. Coasts moved under 600 m.
- **The polar cap in that photo is a small cold spot**, about 20 km across, not a real
  cap. It has a good edge now, but its size is the bake's. `CLIMATE_TOP_RADII` bends it
  harder if you want it less round from orbit: it starts to streak not far past where it is.
- **Beaches now vary a lot in width**, because the warp stretches the distance they are
  measured in. On the row the scripts use, sand runs about 35 m; B2's was about 70.

## Proposed wording

For `DESIGN.md`, Terrain, replacing "The coast is a smooth curve...": "Every boundary
(coast, lake outlines, snow, ice, biomes) is a contour of a smooth field, read through a
one-to-one warp of the sphere with octaves of uniformly decreasing size: rough at every
scale, in only the pieces the planet has. Noise is never added to a value to decide a
long edge. (Andrew, round 2)"

For T2's local and global table: global, the octaves of a boundary over 1 km; local, the
octaves from 1 km to a block.

For `STATUS.md`: "Saves and replays from before this are refused (the terrain changed);
the build that made them still plays them."
