# Vegetation + beaches join the biome field

Branch/worktree: `sol/veg-beach` at base `93be45c`  
World: `output/seed42_r8`  
Date: 2026-07-11

## Result

The reported line at `27.903 33.965` was not a biome boundary. A 2 km
generator probe found warped Koppen 13 throughout; the clean line was the old
tree-only `e_raw < 0.010` rejection. It simultaneously removed trees from
valid grass and still allowed trees on dithered beach sand above that raw
cutoff.

The generic coastal beach and both tree renderers now share column truth:

- `ColCtx` caches the same `climate_surface` used by ground material. Tree
  category is gated by its locally-dithered `main_block`; density follows its
  continuous warped `forest` field through a piecewise curve whose knots are
  the exact legacy biome densities. Existing biome interiors therefore keep
  their species and density while boundaries fade continuously. A barren
  nearest class can grow the climate-appropriate species only where the shared
  field actually selects a grass island.
- `tree_at_scaled` is the one lottery for voxel trees and strided impostors.
  The impostor survivor now receives one authoritative `ColCtx` and delegates
  to that helper, including beach, water, edit, center-carve, and cave-mouth
  guards. Voxel scale is 1; impostor scale remains `stride^2`.
- `beach_frac` still depends on raw and continuous terrain height. Its ramps
  now span 4 m and 6 m around the old midpoints, and voxel material passes
  continuous `h_km`, not floored `ground0`. This removes the one-full / one-
  random / one-empty block-level bands.
- The coastal column comparator is a four-octave fractal field using the
  public biome-ecotone octave, patch-size, lacunarity, and persistence knobs.
  It has an independent seed, so it shares the approved islands-to-speckle
  grammar without mechanically copying every biome island.
- Surface sand and tree rejection call the same `coastal_beach_sand`
  predicate and comparator. No tree anchor can be coastal sand.
- The separately approved V-7 lake-shore rule, lake jitter, and lake
  vegetation exclusion were deliberately left unchanged.

`viewer/src/planet.rs` is byte-untouched.

## Pose probes

`examples/vegprobe.rs` walks a great-circle disc on the canonical column
lattice and reports beach material, the voxel tree lottery, full tree guards,
raw-height residuals, and height bands. Counts use deterministic stride
sampling; before and after use the same sample.

| pose / sample | coastal sand | tree lottery | trees on sand | raw<10 m valid grass / trees | center/full mismatch |
|---|---:|---:|---:|---:|---:|
| Andrew `27.903 33.965`, before (0.4 km, stride 4) | 10,773 | 58 | **23** | 8,906 / **0** | 2 |
| Andrew, after | 6,775 | 98 | **0** | 9,660 / **48** | 0 |
| second coast `0.569 68.915`, before (0.4 km, stride 4) | 1,453 | 143 | **13** | 0 / 0 | 0 |
| second coast, after | 5,295 | 83 | **0** | 0 / 0 | 0 |
| Koppen-255 zone `11.219 -44.825`, before (2 km, stride 16) | 7,384 | 339 | **7** | 4,087 / **0** | 1 |
| Koppen-255 zone, after | 4,716 | 393 | **0** | 5,179 / **55** | 0 |

The sand-count direction differs by terrain field: widening the continuous
height feather grows the second low coast while the coherent comparator opens
large grass islands at Andrew's and the -44.8 fields. The invariant is not a
target sand count; it is coherent structure plus exact tree/material agreement.

At Andrew's site the final ground bands are graded rather than block-banded:

| natural top | sampled columns | sand | trees |
|---:|---:|---:|---:|
| 10 m | 273 | 273 | 0 |
| 11 m | 1,607 | 1,394 | 1 |
| 12 m | 4,023 | 1,835 | 16 |
| 13 m | 9,843 | 3,109 | 22 |
| 14 m | 6,385 | 164 | 33 |
| 15 m | 2,338 | 0 | 15 |

The old result was effectively full sand at 12 m, random at 13 m, and no sand
at 14 m.

### E-2 impostor agreement

At the ledger's jungle reference (`15.942 -78.364`, 1 km, stride 4), 682 of
688 center-eligible anchors also pass `tree_here`; 6 (0.9%) retain the known
neighbor-relief/neighbor-carve mismatch. The ledger measurement was 23 of 915
(2.5%) mismatches, first a cave mouth. The samples are not identical tile
enumerations, so the percentages are a spot-check rather than a strict A/B,
but the dominant cave-mouth class is now rejected by the shared helper and the
residual did not worsen.

## Visual evidence

`scripts/veg-beach-evidence.play` pins weather/sun and captures voxel plus
mesh-only twins at all three required fields. Final frames are under
`interchange/runs/veg-beach-evidence/`:

- `andrew_beach_vox.png` / `andrew_beach_mesh.png`
- `second_beach_vox.png` / `second_beach_mesh.png`
- `koppen255_vox.png` / `koppen255_mesh.png`

The Andrew frame now reads as nested sand/grass islands; the tree population
continues through grass islands and does not follow the old raw-elevation line.
The -44.8 tree edge follows the warped ground field instead of reintroducing
the retired Koppen-255 boundary.

## Gates

### Tests and build

- `cargo test --manifest-path viewer/Cargo.toml`: **PASS** — 30 library tests,
  the `noise_matches_python` integration golden, main/doc targets.
- `cargo test --release --lib --manifest-path viewer/Cargo.toml`: **PASS** —
  30/30.
- `cargo build --release --example play --manifest-path viewer/Cargo.toml`:
  **PASS**.
- Existing warnings only: two deprecated glam camera constructors and the
  pre-existing unused `h_river_ref` assignment.

New unit gates cover the multi-level continuous beach ramp, deterministic
fractal/local beach correlation, dithered grass-vs-sand tree eligibility, and
monotonic density through the exact legacy forest knots.

### Seven asserting play suites

`bash viewer/scripts/verify.sh` could not start because this Windows host maps
`bash` to WSL and has no WSL distribution. The script's exact body was run
natively: regenerate `auto-survey.play` with `--per 16`, then run the same
release harness and seven scripts.

| suite | result |
|---|---|
| physics-regressions | PASS |
| water-regressions | PASS |
| lake-regressions | PASS |
| flooded-caves | PASS |
| invariant-survey | PASS |
| auto-survey | PASS |
| camera-controls | PASS |

### Sync-diff standing table

Final command: `python viewer/scripts/sync_diff.py --label veg-beach-final`.

| pose | divergent | mean d | p95 d | lum bias |
|---|---:|---:|---:|---:|
| river_low | 33.6% | 12.50 | 30 | -7.9 |
| river_mid | 31.9% | 11.60 | 29 | -7.1 |
| lake_shore | 26.8% | 69.26 | 120 | +33.1 |
| savanna | 40.9% | 7.35 | 22 | +0.2 |
| beach | 58.1% | 7.86 | 22 | -3.3 |
| jungle | 43.3% | 18.16 | 60 | +3.4 |
| taiga | 12.0% | 13.51 | 46 | +0.1 |
| desert | 45.3% | 6.77 | 15 | -0.5 |
| peak | 27.1% | 12.58 | 38 | -6.7 |
| valley | 35.9% | 12.04 | 34 | +4.2 |
| groves | 46.0% | 11.56 | 30 | +2.9 |
| ice_top | 55.8% | 8.76 | 19 | +3.4 |
| karst | 42.6% | 63.84 | 119 | +30.2 |
| sea_calib | **0.0%** | **0.00** | **0** | **+0.0** |

Versus base `93be45c`, all river/lake/savanna/beach/desert/peak/groves/ice/
karst/sea rows are exact or effectively exact. Jungle improves from 43.7%,
mean 19.18, p95 61 to 43.3%, mean 18.16, p95 60. Taiga's small 13.31 ->
13.51 mean shift is the new center/cave eligibility guard; valley is 12.05 ->
12.04. `sea_calib` remains exactly zero.

### World reel

`python viewer/scripts/world_reel.py` rendered and linted all 24 poses. It
returned the documented review code with exactly two flags; every flag is
scoped and explained here:

1. `jungle`: mean 3.290, changed fraction 0.1212. This is vegetation-only:
   changed pixels are canopy/ground coverage, while the eroded lake core is
   byte-identical. The accepted reel predates the branch's Koppen-255 warp;
   this mission deliberately makes both tree renderers follow that warped
   surface and adds the authoritative center guard. VOID lint is 0.
2. `orbital_coast`: mean 0.683, changed fraction 0.0366. This is the intended
   land-side coastal color change from the 4 m / 6 m continuous beach feather.
   Water geometry/class is unchanged. VOID lint is 0.

Every water-class reel pose is below flag thresholds. A stricter pixel check
eroded the accepted/current blue-water intersection by three pixels to remove
shore AA; the remaining water cores are byte-identical (`changed=0`, `max=0`)
for dossier shore/lake/ponds, karst, all three rivers, pond country, lake
archipelago, inland sea, coast beach, sea ice, jungle, and orbital coast.

A second identical reel render reproduced all 24 current-frame SHA-256 hashes
byte for byte. It returned the same two flags and identical metrics.

The accepted reel was **not** modified and `--accept` was **not** run; human
sign-off owns re-blessing.

### Water-contract censuses

- `colcensus 13.346 -4.807 0.2` (Difficulty Lake): **exit 0** — `LIP 0`,
  `EDGE 0`, `SHOAL 19,397`, `CAVEP 318`.
- `colcensus 12.199 -44.827 0.2` (pond country): **known exit 2** — `LIP 289`,
  `EDGE 169`, `SHOAL 0`, `CAVEP 0`. This is the pre-existing reopened
  A-4/A-5/P-1 pond debt named in `WATER-CONTRACT.md`; the patch does not touch
  water, terrain sampling, ground height, or water rendering, and the reel's
  pond water core is byte-identical.
- Full `census --lips`: **exit 0**, 221,841,122 samples in 141 s — `WALL 0`,
  `JUMP 0`, `SEAJUMP 0`, `LIP 61,360`; liquid-lake cohort 740, one-block
  liquid-lake cohort 576. These LIPs are the existing frozen/merged-mega-lake
  families documented under S-3/W-5b/W-7; no new wall or jump mechanism
  appeared.

No commit was created.
