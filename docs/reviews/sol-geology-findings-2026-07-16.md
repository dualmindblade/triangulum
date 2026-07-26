# LANDSCAPE Track A, Pass 1 — findings

Date: 2026-07-16  
Requested base: `cb7301e`  
Working-tree HEAD found: `9ecd50a` (`cb7301e` plus the docs-only landscape-campaign launch)

## Result

Both bodies now have deterministic surface rocks. Neisor gets biome/geology-scaled loose stones, 3–6 block boulders, 7–11 block outcrops, and rare 12–17 block erratics. The moon gets sparse background rubble plus stronger crater-rim/ejecta fields derived from the existing crater fold. No block type was added: Neisor uses the existing Rock/Stone/Gravel family, and lunar surface rubble uses dust-coated RegolithHighland rather than the deliberately coal-dark excavated LunarRock palette.

The placement truth is one seeded column lottery:

- `rock_hash01(seed, face, ci, cj)` owns anchor identity.
- The same `neisor_rock_profile` / `lunar_rock_profile`, `rock_kind`, material selection, and late surface gate feed voxel clusters and mesh impostors.
- Voxel chunks render block clusters near the player. Levels 11–14 render faceted crossed stand-ins on absolute stride 8/4/2/1 lattices.
- Far caps use the tree pipeline's lot-ranked thinning and area-conserving size boost (320 at the two distance levels, 900 at the two handoff levels). Absolute stride membership marks parent survivors; only the child density delta uses the established temporal landing dissolve.
- Wet, carved, edited, cave-mouth, and non-natural anchor columns reject exactly. Individual outcrop cells conform to their target column and trim on an invalid target rather than float.

Rocks are the second B-4a v2 species stream in `planet.rs`. The request LOD's max-density lottery runs before profile work. Sparse profile misses and density-winner placement misses are evaluated outside the region lock, sorted/deduplicated into the same bounded sharded LRU, and restored to deterministic request order. Both cached rock records are 16 bytes; the placement record uses a negative albedo sentinel for an exact late rejection. Tree profiles/proofs remain separate, and every pre-existing tree exactness test passed unmodified.

Moon profile misses use the crater batch sampler. The new `debris` value is accumulated directly from crater rim core, disturbed ejecta apron, and ray-halo context while the crater is applied; it is not a second vegetation-like noise field. Scalar and prefetched moon sampling retain bit identity including this new channel.

## Andrew-facing knobs and defaults

Densities are anchor probability per canonical surface column, approximately anchors/km² at a cube-face center:

| Family | Probability | Approx. anchors/km² |
|---|---:|---:|
| forest | 0.00018 | 180 |
| plains | 0.00025 | 250 |
| arid | 0.00042 | 420 |
| coast | 0.00032 | 320 |
| upland | 0.00050 | 500 |
| alpine | 0.00072 | 720 |
| lunar maria background | 0.000015 | 15 |
| lunar highland background | 0.00004 | 40 |
| lunar crater-rubble gain | +0.00050 | +500 at full debris |

Continuous exposed roughness can add 55% on Neisor, hard-capped at 0.00112 (1,120/km²). Lunar density is capped at 0.00054 (540/km²). These rates were chosen after rejecting a denser internal pass that read as a carpet and overflowed the useful 64 MiB candidate working set. The final values read as isolated points on ordinary ground, a recognizable field on exposed alpine/desert ground, and sparse moon rubble with crater context.

Shape probabilities:

| Ground | single | boulder | outcrop | erratic |
|---|---:|---:|---:|---:|
| ordinary | 60.0% | 34.0% | 5.5% | 0.5% |
| exposed/alpine/ejecta | 43.0% | 47.0% | 9.2% | 0.8% |

All knobs are grouped in `voxel::rock_tuning`. Cluster size, far caps, and stand-in dimensions are also grouped next to their consumers for straightforward promotion after Andrew's contact-sheet review.

## Gates

### Release library tests

`cargo test --release --lib`: **green — 120 passed, 0 failed, 1 pre-existing ignored**. Baseline was 109 passed plus one ignored. New coverage includes:

- cached/direct equality over strides 8/4/2/1, cold/hit/concurrent requests, on both bodies;
- all four corners of all six cube faces, on both body lattices;
- malformed request bounds;
- exact voxel late-gate/kind agreement;
- rock requests cannot change tree candidate bits;
- 16-byte sparse-record bounds, seeded lattice alignment, bounded cluster shapes, tuning order;
- crater-owned debris context and prefetched/scalar debris identity.

### `tile_cost` (not blessed)

| | median | vertices | geometry | other | shore |
|---|---:|---:|---:|---:|---:|
| before | 0.181 s | 746,031 | 9,233,981,608,705,220 | 18,863,829,261,183,721 | 1,514,547,866,741,203 |
| after | 0.223 s | 837,175 | 10,251,107,485,886,464 | 19,374,421,847,623,781 | 1,807,378,669,873,619 |

All content checksums changed as expected. The final median adds 42 ms across the fixed 611-tile parallel rebuild (+23%); it is not a hitch-class first-touch regression.

### Reels and sync

- `world_reel.py`: **green, all 28 lint/threshold poses**. This checkout has no `interchange/reel/accepted` directory, so the built-in result is lint-only. A controlled no-rock/default pair was therefore retained. Twenty poses contain localized rock pixels, all below the standing 1.0 mean / 1% changed threshold: `coast_beach`, `desert`, `dossier_band`, `dossier_ponds`, `dossier_shore`, `ecotone_corner`, `frozen_lake`, `frozen_lake_summer`, `groves`, `jungle`, `karst_field`, `mid_altitude`, `moon_low_flyby`, `peak`, `pond_country`, `river_mouth`, `river_valley`, `river_wide`, `savanna`, `taiga`. Eight are byte-identical: `dossier_dlake`, `inland_sea`, `lake_archi`, `moon_orbit_face`, `orbital_biomes`, `orbital_coast`, `sea_ice`, `sea_ice_summer`.
- `sky_reel.py`: **green, all 5 poses; controlled before/after frames are byte-identical**. The mission predicted an expected sky diff, but the minimal moon impostor-only shader branch does not touch any sky-reel pixel.
- `sync_diff.py`: **green; `sea_calib` = 0.0% divergent, mean/p95 delta 0, luminance bias +0.0**. Land poses show expected block/impostor content differences.

### Physics and collision choice

`physics-regressions.play` passed every grounding, water, ice, ceiling, and edit assertion. Direct `physim` transects at savanna and productive forest sites each passed 120/120 support/ceiling columns.

Rocks are deliberately **decorative/non-collidable in Pass 1**. Physics retains terrain and established solid tree trunks as its support truth. This avoids turning sparse visual enrichment into one-block snag hazards or invalidating walkability routes; collision can be an explicit later art/gameplay decision after Andrew approves scale and density.

### B-4/B-4a transition probes

Every checked-in `b4*.play` probe ran with `TRI_RAW_CAPTURE=1`; full blocks are in `viewer/interchange/geology/perf_report.json`. Excluding each script's deliberate first-pose cold/settle block:

| Probe | worst transition frame |
|---|---:|
| b4-ascent-fast-repro | 41.58 ms |
| b4-ascent-fast | 38.01 ms |
| b4-ascent-perf | 38.34 ms |
| b4-ascent-repro | 38.30 ms |
| b4-descent-fast | 40.48 ms |
| b4-descent-fly | 43.03 ms |
| b4a-lateral | 22.33 ms |
| b4a2-converge | 31.64 ms |

The multi-region forest probe retains its known cold relocation spike: 364.31 ms with rocks versus 367.53 ms with `TRI_NO_ROCKS`; rocks do not create that hitch class. At the targeted productive-forest 2 km step, p95 is 61.57 ms with rocks versus 59.18 ms without. Lateral first-move p95 is 22.33 ms with rocks versus 23.43 ms without. The feature-isolation control remains available as `TRI_NO_ROCKS` and disables near clusters, far stand-ins, and candidate warming.

### Byte determinism

Two independent settled contact runs produced identical SHA-256 captures:

- Neisor `ground_close.png`: `F6E3060991284567705B57CFE4EE78423428FC668E3B1FC88C68CFDD4A40785C`
- Moon `moon_mare_surface.png`: `6CE3DDBE6F4329DEC75D4BA790423DE9F8BC15A6E088CCDA834FC9F1FB53F0C6`

## Contact-sheet index

All PNG grids and source frames are under `viewer/interchange/geology/`:

- `geology_before.png` / `geology_after.png` — ten-pose overview grids.
- `geology_before_after.png` — paired review grid.
- `geology_crops.png` — ground/detail crops.
- `world_reel_before_after.png` / `world_reel_crops.png` — 28-pose controlled pair and localized changed-pixel crops.
- `sky_reel_before_after.png` / `sky_reel_crops.png` — five byte-identical controlled sky pairs (there are no delta boxes to crop).
- `report.json`, `world_reel_report.json`, `sky_reel_report.json`, `perf_report.json` — machine-readable evidence.
- `before/`, `after/`, `world_reel_before/`, `world_reel_after/`, `sky_reel_before/`, `sky_reel_after/` — full-resolution source frames.

Ten-pose art order: forest floor; plains; desert; alpine; coast; mid-altitude Neisor; block-scale forest outcrop; moon mare surface; moon crater surface; moon orbit. The first nine original no-rock poses reproduced the pre-feature captures byte-for-byte. The close-up camera was intentionally moved to a deterministic outcrop; its paired baseline uses the same binary with the complete rock stream disabled.

## Honest cut-lines

- No rounded bespoke rock model or new asset technology: this pass is block clusters near and compact faceted stand-ins far.
- No rock collision, mining/drop rules, or gameplay rewards.
- No river/water work, terrain formations, grass/flora, or fragment-color redesign.
- Shader ownership is limited to nine lines in `fs_moon` that reuse the established landing dissolve; no moon or Neisor color path changed.
- Great-distance thinning is intentionally a deterministic representative subset with size compensation, as in the tree pipeline; it does not preserve every one-metre anchor into orbit. The moon-orbit contact pose is therefore correctly byte-identical.
- Sparse forest rates can make a random forest-floor frame nearly unchanged; the dedicated close-up is the shape review pose. Alpine and mid-altitude changes are intentionally subtle at their pixel scale.
- Generated review PNGs are ignored by the repository's normal asset rules but are present in the requested interchange directory. Nothing was committed or blessed.
