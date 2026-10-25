# Tile-build performance restoration

Date: 2026-07-11  
Branch: `sol/tile-perf`  
World/assets: `output/seed42_r8`, `viewer/assets`  
Workload: release `examples/tile_cost.exe`, fixed 611-tile `lake_shore` set

## Result

The final three `tile_cost` medians are **0.288 / 0.278 / 0.253 s**
(median of medians **0.278 s**). The HEAD baseline was **0.771 / 0.814 /
0.783 s** (median of medians **0.783 s**). This is a 64.5% reduction and
passes the mission's `<= 0.32 s` bound.

All four checksums are identical before and after:

| channel | baseline | final |
|---|---:|---:|
| vertices | 746031 | 746031 |
| geometry | 9233981608705220 | 9233981608705220 |
| other | 5130967932755825 | 5130967932755825 |
| shore | 1514547866741203 | 1514547866741203 |

## Profile evidence

Temporary compile-time A/Bs were removed after measurement.

| variant | median evidence | interpretation |
|---|---:|---|
| HEAD | 0.771 / 0.814 / 0.783 s | Reproduces the reported regression. |
| Impostor block disabled | 0.149 / 0.139 s | All non-impostor tile work is about 0.144 s. |
| Candidate lattice + hash only | 0.151 s | Lattice traversal and the hash lottery are negligible; field evaluation owns the regression. |
| Shared position, candidate-only fields | 0.556 / 0.559 s | Removing the duplicate warped lookup and unused tint construction helps, but is not sufficient alone. |
| Same-class interior fast path | 0.344 / 0.335 / 0.333 s | Hoisting work proven constant by `climate_edge` removes another large share. |
| Exact barren/shrub rejection | 0.231 / 0.242 / 0.251 s | Rejecting candidates that provably cannot become billboards restores the old cost class. |
| Final clean rebuild | 0.288 / 0.278 / 0.253 s | Required release build, correct five-octave field, all checksum gates intact. |

A temporary candidate census explains the scale:

- 82,157,511 lattice columns were enumerated.
- 16,822,619 survived the global hash-density bound.
- All 16,822,619 were grass-surface Koppen 28 interiors with the established
  shrub profile.
- Shrubs are deliberately excluded from impostors, so the expensive scan
  produced zero candidates and zero billboard vertices.

This is why the hitch appeared uniformly in non-forest biomes: the old global
density bound could not reject a biome until after its full procedural field
was evaluated.

### Blend3 / five-octave warp

The no-impostor A/B bounds all mesh construction, including blend3's retained
five-octave boundary warp, at about 0.144 s; it is not the 0.63 s regression.
A direct temporary 5-to-3-octave build did not improve this workload
(three-octave medians 0.303 / 0.307 / 0.353 s versus the five-octave final
0.288 / 0.278 / 0.253 s) and changed the `other` checksum to
5130772870125346. The approved five-octave output was therefore retained.
The optimization instead avoids entering the warp where the existing
`climate_edge` proof says it cannot affect the result and shares one warp
where an edge really requires it.

## Implementation

- The cheap hash lottery remains first.
- A same-class interior proof now rejects zero-forest barren/shrub candidates
  before temperature, sea, warp, dither, and tint work. This is monotone:
  snow may suppress vegetation but cannot turn one of these profiles into a
  billboard tree. Candidates that could enter the old candidate vector still
  enter in the same order, so decimation and boost are unchanged.
- Candidate-only `VegetationSurface` computes the exact Koppen, forest,
  main-block, and warped climate values without constructing three unused
  display tints. It uses the existing conservative `climate_edge` mask for an
  exact same-class fast path; boundary columns retain the full field.
- Voxel `col_ctx` now obtains `ClimateSurface` and `BiomeClimate` from one
  climate-position lookup instead of repeating the domain warp.
- A unit test compares the optimized vegetation and voxel views against the
  original full-field APIs bit-for-bit across interiors, categorical edges,
  barren/tree/shrub/snow classes, and multiple cube faces.

## Gates

### Release build

`cargo build --release --example play --example tile_cost` — PASS.

### Play performance (`scripts/perfbench.play`)

| pose | baseline avg / p95 | after avg / p95 | draws |
|---|---:|---:|---:|
| ice | 6.78 / 8.11 ms | 6.57 / 7.21 ms | 851 |
| forest | 10.19 / 11.95 ms | 10.20 / 11.84 ms | 1050 |

The 0.01 ms forest-average difference is timer noise; p95 improved and the
warm-cache draw path and draw count are unchanged.

### Rust tests

`cargo test` — PASS: 37 unit tests, 1 golden integration test, 0 failures.

### Seven play suites

`physics-regressions`, `water-regressions`, `lake-regressions`,
`flooded-caves`, `invariant-survey`, `auto-survey`, and `camera-controls` —
all PASS. `auto-survey.play` was regenerated first (69 probes across 7
classes). The Bash wrapper could not run because this Windows host has no WSL
distribution, so its exact seven-command sequence was executed in PowerShell
with the rebuilt release binary.

### World reel

- `python scripts/world_reel.py` — all 24 poses clean; lint clean.
- Independent SHA-256 comparison of every PNG in `current` versus `accepted`:
  **24/24 byte-identical**.

No commit was created.
