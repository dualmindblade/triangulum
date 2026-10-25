# Biome warp round 2 findings

## Result

Both review gaps are addressed without changing terrain or water geometry.

- The continental warp now has a coherent intermediate octave at about
  **3.4 km wavelength / 0.61 km face-centre amplitude**. The complete stack is
  approximately `68 / 15 / 3.4 km` with amplitudes `6.8 / 2.0 / 0.61 km`,
  still bounded below one bake texel and still handing off to the unchanged
  300 m categorical ecotone.
- The lon -45 failures were not a direction-noise seam and not a separate
  vegetation raster. They were dry land carrying the baked Köppen `255`
  sentinel and therefore taking the old "ocean never warps" early return.
  Physical sea ownership is now separate from that class byte, so every dry
  biome-look consumer uses one warped position while true water remains fixed.

No commit was made.

## Gap 2 diagnosis

All three cameras are just inside face 0's `u = -1` edge:

| pose centre | face/u/v |
|---|---|
| 11.219, -44.825 | `0 / -0.993910 / 0.279656` |
| 12.132, -44.816 | `0 / -0.993598 / 0.303036` |
| 12.106, -44.778 | `0 / -0.992281 / 0.302168` |

The direction-space field itself is continuous there. The existing bit-exact
cross-face warp test passes, as does the derived ocean-mask seam test. Raster
probes instead found dry `255` columns in every photographed footprint:

| footprint sample | `e_raw` | warped/public Köppen before fix | terrain sea |
|---|---:|---:|---|
| 11.219 pose, local dE -15 km / dN +10 km | -62.1 m | 255 | false |
| 12.132 pose centre | +57.7 m | 255 | false |
| 12.106 pose, local dE -5 km / dN +15 km | +80.8 m | 255 | false |

This is a known property of the bake: categorical Köppen is nearest-cell,
while elevation and the water authority are interpolated masks. Thus `255`
means "the nearest climate cell was ocean," not necessarily "this rendered
column is water." The old transform did both of the following:

1. returned immediately for a source `255`, even when the terrain sample was
   dry land; and
2. rejected any land warp whose target class was `255`, even when that target
   was also dry land.

That froze the dry-sand/forest boundary on the raw raster. Ground material,
forest darkening, voxel trees, and mesh impostors all agreed on the same wrong
unwarped class, which is why the photographed tree wall and sand line matched
so precisely.

The fix extracts terrain's sea formula unchanged into `sea_from_fields` and
passes the already-known `Sample::sea` bit through the biome hot paths. True
sea remains identity-mapped. A sentinel-crossing land warp is rejected only
when its target is physically sea; dry `255` land is warped normally. The
photo-map now also uses `BiomeClimate::sea`, rather than treating every `255`
as blue water. Voxel materials/trees, far ground/forest tint, and mesh tree
impostors all reach the same `climate_position_with_sea` transform.

## Visual review

The deterministic six-pose script is
`viewer/scripts/biome-warp-round2.play`. Final 2048x1152 frames are under
`viewer/interchange/runs/biome-warp-round2-final/`.

- `seam_forest_727m`: the sand/grass silhouette has nested lobes; tree mass
  follows the warped class instead of ending on the old face-adjacent diagonal.
- `seam_forest_209m`: the photographed triangular, razor-edged forest wall is
  gone. The remaining forest edge varies with the ground class; true shoreline
  geometry is unchanged.
- `seam_biomes_12km`: the dry sentinel plates now carry the broad warp plus the
  3.4 km woven structure instead of exact kilometre-scale diagonals.
- `intermediate_1km`: the former smooth line now has visible intermediate
  teeth/lobes before resolving into the approved local ecotone.
- `approved_orbit` and `approved_19km`: both retain the approved continental
  breakup, palette, and overall silhouette character.

The finest warp octave uses two direction-space projections. This matters at
lon -45: a boundary nearly parallel to one projection still varies along its
length. The broad approved layers retain their original cheap field.

## Verification

### Build and tests

- Explicit required build:
  `cargo build --release --example play --example tile_cost --manifest-path viewer/Cargo.toml`
  — PASS.
- Full `cargo test --release --manifest-path viewer/Cargo.toml` — PASS:
  24 library tests, golden noise test, binary/doc targets.
- New/updated gates cover:
  - the 2–4 km wavelength and 0.2–0.7 km amplitude window;
  - dry `255` land participating in the warp;
  - true sea remaining uncrossable;
  - bit-exact cube-face warp continuity;
  - categorical corner continuity and ecotone area preservation.
- `viewer/scripts/verify.sh` could not launch because this Windows host's
  `bash.exe` is WSL and no distribution is installed. Its exact body was run
  manually: `gen_survey.py --per 16` regenerated 69 probes, then all seven
  release `.play` suites passed (`physics-regressions`, `water-regressions`,
  `lake-regressions`, `flooded-caves`, `invariant-survey`, `auto-survey`, and
  `camera-controls`).

### Tile cost

A controlled A/B used the same final code and fixed 611-tile set, changing
only `BIOME_WARP_OCTAVES` between 2 and 3:

| stack | median of five five-run medians |
|---|---:|
| round-1-shaped two-octave path | 0.295 s |
| final three-octave path | 0.281 s |
| observed delta | **-4.7% (no measurable regression)** |

Wall-clock noise is larger than the added arithmetic, but the result is safely
below the allowed +5% on top of round 1's +9.5%. Vertex count
(`746031`), geometry checksum (`9233981608705220`), and shore checksum
(`1514547866741203`) were bit-identical; only the appearance checksum moved.

### Mesh/voxel sync-diff

`python scripts/sync_diff.py --label warp-round2-after` — PASS/stable.

- `sea_calib`: **0.0%**, mean/p95 delta `0.0 / 0`.
- 11 of 14 standing rows were numerically identical to the before table.
- Intended biome-sensitive movement only: `beach` divergent fraction
  `59.1% -> 58.1%`, `jungle` `43.5% -> 43.7%` (mean `19.20 -> 19.18`), and
  `groves` mean delta `11.54 -> 11.56`.

### World reel: every flag explained

`python scripts/world_reel.py` reported two changes and no VOID lint. A second
complete render had **0 hash changes across all 24 frames**.

1. `dossier_ponds` — mean delta `34.373`, changed fraction `0.2752`.
   This water-labelled camera is inside the Gap-2 footprint at
   12.199, -44.827. The change is the intended dry-land correction: false sand
   becomes warped grass with matching acacias. Water itself is byte-identical:
   accepted/current each contain exactly `667,958` water pixels, water-mask
   XOR is `0`, and max/mean RGB delta over every water pixel is `0 / 0.0`.
2. `orbital_biomes` — mean delta `1.026`, changed fraction `0.0227`.
   Expected biome-only movement from the added octave. Water/terrain geometry
   is untouched and the continental character remains the approved one.

All other reel poses stayed below the change threshold. In particular the
other water dossier, river, lake, inland-sea, coast, frozen-lake, and sea-ice
poses had no flagged change.

### Remaining water-contract gates

- Difficulty Lake colcensus (13.346, -4.807, 0.25 km):
  `LIP 0 / EDGE 0 / SHOAL 30069 / CAVEP 462`.
- Pond dossier colcensus (12.199, -44.827, 0.25 km):
  inherited reopened debt `LIP 336 / EDGE 219 / SHOAL 0 / CAVEP 0`.
  The climate change cannot enter these counts; water/geometry inputs and the
  tile geometry/shore checksums are unchanged.
- Planet `census --lips`: `WALL 0 / JUMP 0 / SEAJUMP 0 / LIP 61360`;
  liquid cohorts `LIQUID_LAKE 740`, `LIQUID_LAKE_1_BLOCK 576`.
  These are the standing inherited lip cohorts; no hydraulic code or input
  changed in this mission.
