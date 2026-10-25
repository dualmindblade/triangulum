# B-4 findings: mesh-detail loading and ascent lagspikes

## Outcome

B-4 was reproduced and fixed without changing generated mesh contents or the
settled render path. The worst repeatable fast-ascent frames at the perf-reel
pose fell from 117.60 ms to 33.69-34.79 ms at 3 km and from 251.78 ms to
37.90-38.68 ms at 10 km. The fixed 611-tile cost gate retained all four exact
checksums and improved from a 0.295 s median to 0.183 s (-38%).

Two causes combined:

1. Fine-LOD impostor emission exhaustively sampled enormous candidate lattices
   even when every candidate was later rejected as barren, shrub, or snow.
2. The existing immediate-parent prefetch did not cover horizon and LOD nodes
   exposed by a rapid ascent. Those missed nodes became synchronous `urgent`
   builds on the draw thread.

Moon builds were measured and ruled out. Upload/chunk work contributes to the
initial cold teleport, but it did not explain the 3 km and 10 km ascent spikes.

## Method

- Baseline and fixed runs used the same Windows machine, release harness, asset
  set, weather time 349, and cold process per script.
- Targeted live-frame runs used `TRI_RAW_CAPTURE=1`.
- `b4-ascent-fast.play` warms the 0.05 km pose for 20 frames, then measures the
  first draw after each altitude jump with `bench 1`.
- The stepped scripts use `bench 20`. In this harness, the N=20 p95 index is the
  last sample, so p95 equals max and deliberately retains the hitch.
- `viewer/interchange/perfreel/accepted.json` was absent. I generated an
  untouched pre-change baseline and never ran `perf_reel.py --accept`.

The retained repro scripts are:

- `viewer/scripts/b4-ascent-perf.play`
- `viewer/scripts/b4-ascent-fast.play`
- `viewer/scripts/b4-ascent-repro.play`
- `viewer/scripts/b4-ascent-fast-repro.play`

## Root-cause attribution

Temporary phase timers were placed around tile construction, impostor
enumeration, urgent build wall time, upload, and moon construction, then
removed from the final diff.

### Candidate enumeration dominated the fat LODs

The untouched fast perf-pose run produced this summed worker CPU profile:

| LOD | tiles | total tile CPU | candidate enumeration | enumeration share | lattice visits |
|---:|---:|---:|---:|---:|---:|
| 11 | 75 | 31,113.47 ms | 30,669.71 ms | 98.6% | 27,960,595 |
| 12 | 88 | 9,972.18 ms | 9,404.48 ms | 94.3% | 32,810,713 |
| 13 | 75 | 2,634.59 ms | 2,143.19 ms | 81.4% | 27,995,409 |
| 14 | 97 | 1,437.92 ms | 827.39 ms | 57.5% | 36,248,806 |

One representative level-11 tile spent 878.91 of 884.71 ms in enumeration.
It visited 373,321 lattice positions; 262,807 survived the cheap hash-density
gate and were then rejected as snow; zero impostors were emitted. The slowest
level-11 tile in the aggregate took 664.86 ms.

The diagnostic `TRI_NO_IMPOSTORS=1` isolated the same cost without source
changes:

| fast perf-pose stage | normal | no impostors |
|---|---:|---:|
| 0.05 km cold max | 3,745.50 ms | 577.31 ms |
| 3 km first frame | 117.60 ms | 46.65 ms |
| 10 km first frame | 251.78 ms | 50.13 ms |

After the exact snow/interior hoists, the same instrumented perf-pose sweep
reduced level-11 enumeration CPU from 30,669.71 to 23,513.42 ms (-23.3%) and
level-12 enumeration CPU from 9,404.48 to 8,011 ms (-14.8%). At the documented
polar pose, the region proof skipped enumeration for all profiled level-11
tiles (51/51), level-12 tiles (64/64), and level-13 tiles (52/52); their total
worker CPU sums were 168.95, 209.96, and 183.88 ms respectively.

### Fast ascent turned misses into foreground work

Before the fix, the cold perf-pose draw selected 760 tiles and synchronously
built 760 urgent tiles: 2,790.42 ms urgent-build wall time and 3,084.28 ms total
draw time. Later in the same fast sweep:

| altitude | urgent tiles | urgent-build wall | instrumented draw | harness frame |
|---:|---:|---:|---:|---:|
| 3 km | 10 | 80.29 ms | 88.14 ms | 117.61 ms |
| 10 km | 4 | 215.32 ms | 224.96 ms | 255.48 ms |

Those four 10 km blockers were level-12 impostor tiles. They were neither in
cache nor pending: the immediate-parent-only predictor had no path to the new
horizon/LOD nodes. With deterministic altitude lookahead, the corresponding
instrumented first frames were 34.44 ms and 39.25 ms.

### Suspects ruled secondary or out

- Moon construction was six tiles and 9.87 ms on the cold frame, then remained
  about 10-11 ms across the sweep. A separate moon Rayon pool would not address
  the measured spikes.
- The upload routine recorded 1,453 calls, 596,112,544 bytes, 643.09 ms total,
  and one 236.01 ms maximum on the initial run. That counter includes voxel
  chunks as well as terrain, so it is a mixed cold-load signal. It was not
  present in the isolated 3/10 km urgent-build timing and was not treated as
  the primary cause.

## Fix

### Exact rejection hoists

`planet.rs` now exposes two conservative region proofs used only by impostor
candidate generation:

- every nearest climate raster cell is a non-edge categorical interior and
  its established vegetation profile is barren/shrub-only; or
- every possible unwarped or domain-warped bilinear climate lookup is below
  the unconditional snow temperature.

The snow proof expands the source rectangle by the existing strict 1.20-texel
warp bound, partitions it at raster grid lines, checks every bilinear extremum,
adds a 0.05 C rounding margin, and declines near cube seams. A failed proof
falls back to the original per-candidate path.

The snow comparator was also hoisted ahead of categorical boundary material
selection inside `vegetation_surface`. This returns the same `Snow` result and
the same vegetation metadata while avoiding work whose result cannot survive.
Per-candidate interior snow rejection uses that exact unconditional threshold.

### Deterministic ascent lookahead

The existing four-tile live prefetch budget now checks a deterministic future
Neisor cover first, at `max(altitude + 10 km, altitude * 4)`. Remaining slots
retain immediate-parent prefetch. This sees the horizon/LOD nodes that are not
ancestors of the current cover and lets their expensive builds complete before
selection.

Lookahead is disabled in the settled capture path, just like other live
prefetch behavior. Tile-key semantics, generation, mesh bytes, selection, and
the drawn settled cover are unchanged; only live scheduling order changes.

## Before/after measurements

### Fast ascent at the perf-reel pose (64.120, -46.429)

Values after 0.05 km are one-frame bench results, so p95/max/frame are the same.
Two diagnostics-free fixed runs are shown to expose variance.

| altitude | before | after run 1 | after run 2 |
|---:|---:|---:|---:|
| 0.05 km cold max | 3,745.50 ms | 2,672.49 ms | 2,578.98 ms |
| 0.2 km | 33.95 ms | 34.77 ms | 34.30 ms |
| 0.8 km | 45.20 ms | 34.13 ms | 32.03 ms |
| 3 km | 117.60 ms | 34.79 ms | 33.69 ms |
| 10 km | 251.78 ms | 38.68 ms | 37.90 ms |
| 40 km | 43.69 ms | 43.17 ms | 39.26 ms |
| 160 km | 37.00 ms | 37.94 ms | 38.54 ms |
| 640 km | 28.48 ms | 30.51 ms | 27.84 ms |
| 1,229 km | 18.42 ms | 23.91 ms | 17.70 ms |

The important regressions improved by 70-71% at 3 km and 85% at 10 km.

### Stepped ascent at the documented pose (80.909, -74.619)

Each row is the p95/max of its 20-frame block.

| altitude | before p95/max | after p95/max |
|---:|---:|---:|
| 0.05 km cold | 646.97 ms | 485.66 ms |
| 0.2 km | 67.26 ms | 34.34 ms |
| 0.8 km | 65.02 ms | 33.87 ms |
| 3 km | 60.27 ms | 33.20 ms |
| 10 km | 36.47 ms | 32.23 ms |
| 40 km | 26.11 ms | 24.99 ms |
| 160 km | 23.02 ms | 21.08 ms |
| 640 km | 23.01 ms | 12.87 ms |
| 1,229 km | 10.02 ms | 5.23 ms |

### Fixed tile-cost gate

| version | five runs | median |
|---|---|---:|
| before | 0.291, 0.311, 0.314, 0.295, 0.288 s | 0.295 s |
| after | 0.182, 0.174, 0.200, 0.188, 0.183 s | 0.183 s |

Both versions produced exactly:

- vertices: `746031`
- geometry: `9233981608705220`
- other: `18863829261183721`
- shore: `1514547866741203`

### Full perf reel

The post columns are ranges from two diagnostics-free runs of the final
scheduler. The pre column is the freshly generated untouched baseline.

| scenario | pre avg / p95 / max | post avg / p95 / max |
|---|---:|---:|
| forest_ground | 33.66 / 50.60 / 54.48 | 28.75-28.83 / 34.38-35.04 / 40.03-41.83 |
| plains_walk | 28.36 / 34.56 / 71.06 | 22.81-25.93 / 28.94-36.81 / 32.25-41.86 |
| mountain_mid | 25.88 / 28.30 / 29.54 | 26.27-27.44 / 27.49-30.55 / 28.10-31.08 |
| ocean_storm | 21.83 / 23.57 / 38.82 | 20.70-21.85 / 22.47-24.30 / 24.39-40.28 |
| orbit | 17.49 / 20.49 / 31.81 | 17.49-17.78 / 20.29-24.43 / 30.84-35.07 |
| moon_surface | 19.24 / 23.43 / 25.90 | 16.03-16.63 / 17.46-17.58 / 17.71-18.21 |
| moon_orbit | 2.94 / 3.59 / 3.60 | 2.46-2.76 / 2.98-3.54 / 3.02-3.56 |
| ascent_sweep | 34.66 / 36.46 / 36.89 | 36.15-36.40 / 38.22-39.97 / 41.33-47.84 |

The standard ascent reel is a settled-throughput probe rather than the
one-frame jump that reproduced B-4. Its average rose 4-5% and p95 5-10% because
lookahead deliberately performs future work in the background; all remain
inside the script's 20% average and 35% p95 guardrails. This tradeoff is kept
explicit rather than hidden: it removes 118-252 ms foreground stalls at the
cost of a few milliseconds of background load.

## Verification gauntlet

- `cargo test --release --lib`: **PASS**, 101 passed, 0 failed, 1 ignored.
- Play asserting suites: **PASS** for `physics-regressions`,
  `water-regressions`, `lake-regressions`, `flooded-caves`,
  `invariant-survey`, `camera-controls`, and `solar-system`.
- `auto-survey` was intentionally omitted because this checkout has no
  `output/` world data, as allowed by the mission instructions.
- `python scripts/sync_diff.py`: **PASS**; `sea_calib` remained exactly 0.0%
  divergent.
- `python scripts/world_reel.py`: **PASS**, all 28 poses clean, no unexplained
  pixel change.
- `python scripts/sky_reel.py`: **PASS**, all 5 poses clean.
- `cargo run --release --example tile_cost`: **PASS**, all exact checksums
  unchanged.
- Final source contains no B-4 timing/logging instrumentation.

## Honest cut-lines

- Cold first-load max at the perf pose is still roughly 2.58-2.67 s. It is
  materially better than 3.75 s, but dense or climate-boundary level-11 tiles
  that cannot satisfy either exact region proof still enumerate large candidate
  sets. A reusable lean candidate stream/cache keyed independently of tile LOD
  is the next structural fix; it is larger and riskier than a proven rejection
  hoist, so it was not forced into this mission.
- The lookahead moves work earlier; it does not make remaining dense candidate
  builds free. The small steady `ascent_sweep` regression above is the measured
  scheduling cost.
- The mixed cold upload/chunk maximum was not optimized. It needs separate
  terrain-versus-voxel accounting before changing buffer batching.
- Moon work was not moved to another pool because its measured cost was too
  small to explain B-4.
