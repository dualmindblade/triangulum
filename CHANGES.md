# Climate tint + dual-range transitions

Feature work for `TRANSITIONS.md` proposal D on `feat/climate-tint`.
The palette is intentionally provisional. The deliverable is one continuous
climate ramp and one shared category decision evaluated by both ground
renderers.

## Files

- `.gitignore`
  - Keeps ordinary `viewer/interchange/` run output ignored, but exposes
    root-level `tint_*.png` so the requested visual evidence can be committed.
- `viewer/src/planet.rs`
  - Owns `climate_surface`, the single temperature/precipitation tint context
    used by mesh vertices and voxel columns.
  - Moves the existing splitmix-style voxel column hash and
    `COLUMNS_PER_FACE` lattice constant here so category and snow hashes have
    one implementation.
  - Adds the existing Köppen-to-main-block classification, a metric 300 m
    cross-block boundary query, smoothly blended Köppen hue/canopy signals,
    and synthetic unit tests for the dual-range rule.
- `viewer/src/terrain.rs`
  - `shade_ground` now calls `climate_surface` using the `Sample`'s existing
    temperature and precipitation. It uses the returned category/tints before
    the existing beach, rock, snow, and lake-shore ordering.
  - Snow and cross-block categories hash the canonical world column at mesh
    vertices. The existing far-canopy darkening weight is bilinearly smoothed
    so it cannot restore a hard same-block class line.
- `viewer/src/voxel.rs`
  - Every rendered column calls the same `climate_surface` function.
  - `surface_mat` consumes its shared grass/sand/snow result; hydrology,
    steepness, and beach rules retain their previous precedence.
  - Grass, sand, and snow receive the shared material tint. Fixed dirt, rock,
    water, ice, and vegetation colors remain fixed.
- `viewer/scripts/tint-seams.play`
  - Weather-off reference shots for a BWk/BSk sand/grass edge, a Cfb/Cwb
    grass/grass edge, the README savanna site, and a low mesh/voxel straddle.
  - Pins the sun and uses absolute poses, so the four frames are deterministic.
- `viewer/interchange/tint_*.png`
  - Eight 1280x720 before/after review frames listed and hashed below.

No `Sample` field was added, and neither hot path adds a `terrain::sample`
call. The new work is raster byte reads, arithmetic, and two column hashes.

## Köppen main-block classification

This table is derived from the old `voxel::surface_mat` match. It does not add
the speculative taxonomy names from `TRANSITIONS.md`.

| id | Köppen | main block |
|---:|:---:|:---|
| 0 | Af | grass |
| 1 | Am | grass |
| 2 | Aw | grass |
| 3 | BWh | sand |
| 4 | BWk | sand |
| 5 | BSh | grass |
| 6 | BSk | grass |
| 7 | Csa | grass |
| 8 | Csb | grass |
| 9 | Csc | grass |
| 10 | Cwa | grass |
| 11 | Cwb | grass |
| 12 | Cwc | grass |
| 13 | Cfa | grass |
| 14 | Cfb | grass |
| 15 | Cfc | grass |
| 16 | Dsa | grass |
| 17 | Dsb | grass |
| 18 | Dsc | grass |
| 19 | Dsd | grass |
| 20 | Dwa | grass |
| 21 | Dwb | grass |
| 22 | Dwc | grass |
| 23 | Dwd | grass |
| 24 | Dfa | grass |
| 25 | Dfb | grass |
| 26 | Dfc | grass |
| 27 | Dfd | grass |
| 28 | ET | grass |
| 29 | EF | snow |
| 255 | ocean sentinel on rendered coastal land | sand |

`rock` and `stone` are not Köppen classes here. They remain local slope
overrides (`steep >= 5` and `steep >= 3` respectively). Underwater floors,
liquid-temperature lake shores, the shared snow decision, slopes, and beaches
still override the table in that order. ET remains grass in the class table,
but the existing temperature snowline can select snow per column.

Same-block class neighbors do not enter the category hash path. Different
blocks use the same canonical `(face, column_i, column_j)` splitmix hash in
both renderers. At the exact raster edge either side has 50% probability of
taking the neighbor; the probability smoothsteps to zero 150 m into each
side, for a 300 m total band.

## Tunables and defaults

All new palette values are linear RGB.

| tunable | default | purpose |
|:---|:---|:---|
| `CROSS_BLOCK_ECOTONE_KM` | `0.300 km` total | Cross-block category band; 150 m per side. |
| `KOPPEN_HUE_NUDGE` | `0.14` | Luminance-preserving, bilinearly smoothed Köppen hue contribution to grass. |
| ecotone hash salt | `0x0EC070AE` | Keeps category dithering independent from snow/trees/brightness. |
| snow hash salt | `0x5A0E` | Existing voxel snowline salt, now shared with mesh vertices. |
| snow center / half-range | `-9.0 C / 1.5 C` | Existing `-10.5..-7.5 C` per-column snowline, now shared. |
| climate temperature anchors | `-4, 4, 16, 26 C` | Smooth piecewise temperature axis for grass. |
| moisture dry limit | `100 + 35 * max(temp_c, 0) mm/yr` | Makes equal rainfall read drier in heat. |
| moisture transition span | `900 mm/yr` | Dry-to-wet grass ramp width. |
| dry grass anchors | `(0.220,0.215,0.150)`, `(0.260,0.240,0.110)`, `(0.180,0.230,0.070)`, `(0.205,0.240,0.055)` | Provisional cold-to-hot dry colors. |
| wet grass anchors | `(0.070,0.160,0.070)`, `(0.075,0.205,0.060)`, `(0.090,0.250,0.050)`, `(0.050,0.230,0.040)` | Provisional cold-to-hot wet colors. |
| sand cold/hot anchors | `(0.545,0.470,0.290)` / `(0.585,0.485,0.255)` over `-4..26 C` | Keeps existing sand character while making it continuous. |
| sand damp response | `250..1400 mm/yr`, max `10%` toward `(0.485,0.430,0.270)` | Subtle continuous precipitation response. |
| snow cold/warm anchors | `(0.795,0.835,0.900)` / `(0.835,0.865,0.905)` over `-30..-4 C` | Continuous snow temperature response. |
| snow wet response | `50..900 mm/yr`, max `12%` toward `(0.850,0.875,0.915)` | Subtle continuous precipitation response. |

The gnomonic raster metric is not a tuning knob: it converts texel distance
to kilometres so the 300 m default remains approximately physical across a
cube face. The existing far-canopy darkening maximum (`22%`) is unchanged;
only its categorical signal is now bilinearly blended.

## Verification

Commands were run from this worktree on 2026-07-09/10 with weather disabled
for visual frames.

- `cargo build --release --example play` (from `viewer/`): PASS.
- `cargo test` (from `viewer/`): PASS.
  - library: 3 passed (`koppen_main_blocks_match_surface_materials`,
    `category_dither_only_crosses_main_blocks`, existing
    `ocean_mask_seam_exact`)
  - integration: existing `noise_matches_python` passed
  - only the two pre-existing glam deprecation warnings were emitted
- `python viewer/scripts/gen_survey.py --per 16`: generated 69 probes across
  7 classes.
- Release play harness suites:
  - `physics-regressions`: PASS
  - `water-regressions`: PASS
  - `lake-regressions`: PASS
  - `flooded-caves`: PASS
  - `invariant-survey`: PASS
  - `auto-survey`: PASS
  - `camera-controls`: PASS
- `git diff --check`: PASS.

`cargo fmt -- --check` is not claimed as a gate: the untouched crate already
differs from this toolchain's rustfmt across many examples and modules. No
whole-crate formatter was run, avoiding an unrelated mechanical rewrite.

`bash viewer/scripts/verify.sh` could not start because this Windows host maps
`bash` to WSL and has no WSL distribution installed. The wrapper's substantive
steps were run directly instead: the same generator, release binary, seven
script paths, and exit-code gate listed above.

### Visual captures

| subject | coordinate / pose | before | after |
|:---|:---|:---|:---|
| BWk/BSk sand↔grass | `0.336040 -90.168022`, alt `2.0 km`, yaw `90`, pitch `-62` | `tint_before_sand_grass_2km.png` | `tint_after_sand_grass_2km.png` |
| Cfb/Cwb grass↔grass | `5.013911 -29.381680`, alt `2.0 km`, yaw `0`, pitch `-62` | `tint_before_grass_grass_2km.png` | `tint_after_grass_grass_2km.png` |
| README savanna acacias | `1.510000 40.490000`, alt `2.0 km`, yaw `-50`, pitch `-38` | `tint_before_scenic_savanna_2km.png` | `tint_after_scenic_savanna_2km.png` |
| Mesh/voxel straddle at BWk/BSk | `0.336040 -90.168022`, alt `0.12 km`, yaw `90`, pitch `-18` | `tint_before_sand_grass_straddle.png` | `tint_after_sand_grass_straddle.png` |

SHA-256:

```text
867b2502b3a20e1d5006fa641ec4f72ea6d52c7ce8c0b99cabc861eafb1df10a  tint_after_grass_grass_2km.png
7babb134a6b951fb7d5f72b86006344fb29c018dec275cad84da27470e79454c  tint_after_sand_grass_2km.png
9560c092bf797ad78244711576c20fdc09621e0261fac6827217dbbaa5635f0f  tint_after_sand_grass_straddle.png
20cc7bfbf73957117d1a88c2cd6c54c75c26ca6acd4b4aa3ba966eff9926b4ac  tint_after_scenic_savanna_2km.png
a63adea5b04f38ffea5edf0ede575e42487e9728b75db25bd5d8c1ce0f886a42  tint_before_grass_grass_2km.png
bbf776ccf9fd201e145d3fe8e4b9db7123316a688a09ffeb04174b7126bbf98d  tint_before_sand_grass_2km.png
efec741aeb666aa28a5bedfbc37cf8b231807b5fddc9a017bc71e75bcb974cb0  tint_before_sand_grass_straddle.png
8f46cd4dd2507ea12197f95e7a72c15f8602be07572489f9d8d266a37f4cbc4a  tint_before_scenic_savanna_2km.png
```

### Double-run determinism gate

Two consecutive executions of
`viewer/target/release/examples/play.exe viewer/scripts/tint-seams.play`
produced byte-identical PNGs:

```text
7babb134a6b951fb7d5f72b86006344fb29c018dec275cad84da27470e79454c  sand_grass_2km.png
867b2502b3a20e1d5006fa641ec4f72ea6d52c7ce8c0b99cabc861eafb1df10a  grass_grass_2km.png
20cc7bfbf73957117d1a88c2cd6c54c75c26ca6acd4b4aa3ba966eff9926b4ac  scenic_savanna_2km.png
9560c092bf797ad78244711576c20fdc09621e0261fac6827217dbbaa5635f0f  sand_grass_straddle.png
```

## Deliberate cut lines and limitations

- Far-mesh category hashing is per vertex, which the mission explicitly
  permits. At the ~2 km reference LOD, roughly 100 m vertex spacing turns the
  300 m band into coarse triangular/sawtooth samples; it has the same hash and
  probability statistics as voxel columns but not the same one-metre spatial
  realization. The after straddling capture shows this honestly. Exact
  per-column far pixels would require passing boundary probability/category
  data to the fragment shader; that is a later transition/shader change.
- Tree density/species and placement were not changed. The mission constrains
  tree placement as out of scope, and this patch is tint + surface material
  category only. The far-canopy color weight is smoothed because it directly
  affects ground shading; voxel trees still change species/density at their
  existing nearest-Köppen boundary.
- No `rock` or `jungle-floor` Köppen category was invented. Today's material
  selector has no such biome main block: rock/stone are slope facts and jungle
  still uses grass ground plus jungle trees.
- The UI minimap retains the legacy categorical palette. It is an atlas/photo
  navigation view, not either world renderer, and changing it would obscure
  the before/after terrain scope.
- Weather/shader ground tinting was not changed. Weather still runs after the
  base vertex/material color; every reference shot uses `weather off`.
- Rivers, lakes, water, terrain heights, geomorphing, caves, and tree placement
  were not touched.
- The ramp uses annual temperature and annual precipitation because those are
  the continuous rasters available in both hot paths. Seasonal dryness cannot
  distinguish every savanna/rainforest pair with identical annual means;
  textures and their measured average-color anchors remain the planned palette
  replacement.
- No git commit was attempted. Pre-existing untracked `MISSION.md`, `run.err`,
  and `run.log` were not modified or included in this work.
