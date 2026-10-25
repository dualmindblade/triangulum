# Noise round 4 findings

Base: `sol/noise-r4` at `3e4dd07`, world `output/seed42_r8`. No commit was made. All captures are headless `play` shots with weather disabled and a pinned sun.

## Outcome

The descent now uses one categorical boundary decision from orbit through voxels. The 8 km zone and the domain warp share the same scalar carriers, and the field continues through ten octaves instead of handing off between separately seeded and separately normalized stacks. Calm coastal beach edges now suppress fine breakup according to the baked local roughness raster; rugged coasts retain the round-3 field. Ocean ownership, sea class, shore geometry, and water rules were not changed.

## Diagnosis first: descent ladder

I captured the BEFORE ladder with the `3e4dd07` release executable before editing source. The fixed repro is centered at `48.266365, 29.779374`, yaw `88.723928`, pitch `-85.943669`, at 288/50/10/3 km. The 1 km frame follows the same boundary 2.4 km south to `48.250365, 29.779374`, because the smaller footprint at the original coordinate landed wholly inside the pale biome.

![Descent ladder before and after](viewer/interchange/noise-r4/descent_before_after.png)

The old handoff is visible between 10 and 3 km. At 288, 50, and 10 km the boundary has large branching islands. At 3 km it collapses to a nearly straight diagonal, and at 1 km only the old local flecks remain. This corresponds directly to the shader's 4–12 km near/range fade:

- Near/voxel used a 300 m ownership band and the local four-octave comparator (about 7 km, 440 m, 27 m, and 1.7 m).
- Range used an 8 km band plus a 4x4 prefilter and a different comparator: two independent coarse octaves were prepended and the combined stack was normalized separately.
- The domain warp used five still-different vector-noise fields and seed phases. The range-visible islands therefore did not derive from the warp that moved the underlying boundary.

So the mid band looked unchanged because the fade replaced the visible range field with the narrow local selection, not because its detail was merely undersampled. Andrew's two visible layers were mathematically out of agreement.

After the change, the same large lobe and its children can be followed through 288, 50, 10, and 3 km; the tracked 1 km frame resolves the same edge into column-scale detail instead of switching to a second silhouette.

## Continuous boundary implementation

`biome_scalar_noise` is now the common carrier. Every warp octave is a vector projection of that scalar value, and the boundary comparator consumes those exact same five values, with the same seed, frequency, lacunarity, and rotated domain. Five further octaves continue the same field to column scale.

At face center, the ten approximate wavelengths are 85, 23.5, 6.53, 1.81, 0.504, 0.140, 0.0389, 0.0108, 0.0030, and 0.00083 km. The amplitude sequence is `1, 0.55, 1, 0.5, 0.25, ...`; the octave-2 reset retains the approved range-visible energy while using one variance normalization for the entire descent.

The 8 km categorical zone is now used by both `climate_surface` and `climate_surface_pair`. A mesh vertex makes one boundary choice, then carries a one-hot near appearance and area-smoothed far color weights for that same main block. The shader may soften contrast with distance, but it cannot fade to another category silhouette. Voxel columns call the same selection path.

Three new gates make the coupling structural:

- `boundary_field_reuses_the_exact_warp_carriers` recovers each of the five warp carriers and compares it to the boundary scalar bit-for-bit within floating-point roundoff.
- `near_and_range_surfaces_keep_one_boundary_silhouette` checks 16,384 positions for identical near/range main blocks.
- `pond_dossier_keeps_its_approved_grass_surface` prevents the arbitrary comparator polarity from repainting the accepted pond dossier.

## Roughness-adaptive coastline noise

The beach comparator now receives `rough_km`, the baked mean absolute elevation difference to neighboring raster cells, bilinearly sampled at the column. This is a local average roughness, not a point slope. The broad approximately 7 km carrier stays at full strength. The 440 m, 27 m, and 1.7 m amplitudes use:

`fine_gain = 0.08 + 0.92 * smoothstep(0.025 km, 0.500 km, rough_km)`

The adapted variance is normalized before the normal-CDF transform. That changes spatial roughness without biasing expected sand coverage or breaking the shared mesh/voxel beach fraction. The same value reaches surface material and coastal tree eligibility.

![Coastline noise before and after](viewer/interchange/noise-r4/coasts_before_after.png)

| Site | Pose | 5x5 land-side mean roughness | Fine gain | Pixel result |
|---|---:|---:|---:|---|
| smooth A | `36.51795, 110.17686`, 1 km | 0.0356 km | 0.0814 | fine speckle becomes a calm coherent contour |
| smooth B | `16.98657, 86.76201`, 1 km | 0.0345 km | 0.0811 | fine islands collapse into a gentler edge |
| rough A | `-16.04723, -43.40193`, 1 km | 2.9820 km | 1.0000 | byte-identical before/after |
| rough B | `-7.40855, -9.82551`, 1 km | 2.7985 km | 1.0000 | byte-identical before/after |

The rugged proofs are exact SHA-256 matches: rough A is `BBFA95CFBD2DFF84E8E470938D3B689B4F08454C5E9BC712C11B93A838E5DA1E`; rough B is `92C04CD844348416A544B6825EEB3CC9BCD3B07A343DDE295DDDEABA4B89E702`. The smooth frames intentionally changed from `94772F...` to `7CA394...` and from `8E0092...` to `B06623...`.

The statistical gate samples 2,048 nearby column pairs and requires calm local variation to stay below 35% of rugged variation. It also checks 32,768 samples at each roughness endpoint and keeps the 25/50/75% occupancy quantiles within 1.5 percentage points.

## Artist-facing knobs

| Constant | Value | Effect |
|---|---:|---|
| `BIOME_BOUNDARY_ZONE_KM` | 8.0 km | one cross-material ownership zone at every range |
| `BIOME_BOUNDARY_ZONE_MAX_TEXEL_FRACTION` | 0.90 | prevents the metric zone from spanning an excessive baked-cell fraction |
| `BIOME_BOUNDARY_PREFILTER_SIDE_WEIGHT` | 0.15 | symmetric far-color/area support; does not choose a second silhouette |
| `BIOME_BOUNDARY_FIELD_OCTAVES` | 10 | shared warp-scale structure through approximately 0.8 m |
| `BIOME_BOUNDARY_FIELD_SECOND_COARSE_AMPLITUDE` | 0.55 | second broad carrier strength |
| `BIOME_BOUNDARY_FIELD_FINE_PERSISTENCE` | 0.50 | decay after the range-visible octave reset |
| `BIOME_BOUNDARY_FIELD_POLARITY` | -1.0 | arbitrary side assignment chosen to retain the accepted pond surface; area statistics are unchanged |
| `COAST_NOISE_CALM_ROUGHNESS_KM` | 0.025 km | start of coast fine-noise ramp |
| `COAST_NOISE_RUGGED_ROUGHNESS_KM` | 0.500 km | full approved fine noise at and above this roughness |
| `COAST_NOISE_CALM_FINE_GAIN` | 0.08 | residual local character on calm coasts |

## Water contract

The diff does not modify `sea_from_fields`, `Planet::water_frac`, the ocean-mask bake/read, `shore_field`, water classes, water height, or water geometry. `rough_km` only modulates the generic coastal beach material comparator. Physical sea ownership remains the unwarped guard for climate lookup.

Water gauntlet observations:

- `sync_diff.py`: `sea_calib` divergent `0.0%`, mean delta `0.0`, signed luminance `+0.0`.
- Reel water/karst cores `dossier_shore`, `dossier_dlake`, `dossier_ponds`, `dossier_band`, `karst_field`, and `river_wide` are byte-identical. `river_mouth` has a sub-threshold `0.024` mean / `0.0004` changed fraction from adjacent land material; its water geometry is unchanged.
- Difficulty Lake colcensus at `13.346, -4.807`, radius 0.25 km: `LIP 0`, `EDGE 0` (`SHOAL 30069`, `CAVEP 462` informational).
- Pond dossier colcensus at `12.199, -44.827`, radius 0.25 km: `LIP 88`, `EDGE 0`; this is the inherited ledger state and the changed code cannot affect its ground/water predicates. The exact repaired rampart at `12.194, -44.858` remains `LIP 0`, `EDGE 0`.
- Planet `census --lips`: 61,344 raw findings / 42,035 sites, `WALL 0`, `JUMP 0`, `SEAJUMP 0`, `LIP 61344`; `LIQUID_LAKE 740`, `LIQUID_LAKE_1_BLOCK 576`. Both 61,344 and the one-block subtotal are unchanged from the accepted baseline.
- Reel VOID lint emitted no finding; maximum `void_blob_px` is one, far below the 400-pixel threshold.

## World reel findings

`python scripts/world_reel.py` rendered and linted all 24 poses. It exits 1 because these three intentional changes are not re-blessed; no baseline was accepted.

| Flag | Delta | Explanation |
|---|---:|---|
| `jungle` | mean 1.602, changed 2.30% | intended ground/vegetation ownership changes where the old local boundary is replaced by the one shared all-range field |
| `orbital_coast` | mean 0.501, changed 1.70% | intended land biome/beach response near the coast; the ocean mask and sea geometry are unchanged |
| `orbital_biomes` | mean 2.304, changed 7.66% | intended orbital boundary phase/shape change from coupling the 8 km zone to the warp carriers |

The other 21 poses produce no review finding. The required biome and coast categories changed; accepted water and karst cores did not.

## Gate results

| Gate | Result |
|---|---|
| `cargo build --release --example play --example tile_cost` | PASS |
| `cargo test --release` | PASS: 40 library tests + golden integration; the original 37 remain green |
| literal `cargo test` | PASS: 40 + golden integration |
| seven asserting suites | PASS: physics, water, lake, flooded-caves, invariant-survey, auto-survey, camera-controls |
| `python scripts/sync_diff.py` | PASS; `sea_calib 0.0%` |
| `python scripts/world_reel.py` | 3 intentional review flags explained above; VOID lint silent |
| `cargo run --release --example colcensus` ledger sites | results recorded under Water contract |
| `cargo run --release --example census -- --lips` | no cohort rise; full output at `viewer/interchange/noise-r4/census-lips.md` |
| `git diff --check` | PASS |

This Windows host has no installed WSL distribution, so `bash scripts/verify.sh` itself cannot launch. I ran its exact body instead: regenerated `auto-survey.play` with `python scripts/gen_survey.py --per 16`, then executed the release `play` binary on all seven scripts. All seven reported PASS.

## Tile cost and checksums

Five clean idle runs were `0.241, 0.260, 0.257, 0.244, 0.252 s`; median `0.252 s`, below the `0.30 s` gate. Current checksums:

| Stream | Checksum | Versus `3e4dd07` |
|---|---:|---|
| vertices | `746031` | unchanged |
| geometry | `9233981608705220` | unchanged |
| other | `5131994183779440` | intentionally changed (`5130967932755825` before) |
| shore | `1514547866741203` | unchanged |

The changed `other` stream is the intended climate/material output. Identical geometry and shore streams are additional evidence that coastline ownership and water geometry did not move.

## Evidence and repro files

- `viewer/scripts/noise-r4-descent.play` — fixed five-step ladder.
- `viewer/scripts/noise-r4-coasts.play` — two calm and two rugged coast captures.
- `viewer/examples/coastprobe.rs` — deterministic shoreline roughness ranker used to select the four sites.
- `viewer/interchange/noise-r4/before` and `after` — source frames and sidecars.
- `viewer/interchange/noise-r4/descent_before_after.png` and `coasts_before_after.png` — contact sheets above.

