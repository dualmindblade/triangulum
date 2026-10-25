# B-4a findings — lean impostor candidate cache

Date: 2026-07-14  
Base: `20ab644955186c29e085d395638f11cf5a7e966a`

## Outcome

The enumeration-heavy perf pose is fixed without changing the candidate
truth or renderer pass structure.

- Cold `64.120 -46.429` load fell from 2713.91 ms to 608.07 ms p95/max
  (4.46x faster, -77.6%).
- The bad continuous-descent 2 km step fell from 697.88 ms to 86.37 ms
  p95/max (-87.6%). Two final fresh-process repeats were 88.34 and 86.37 ms;
  every non-cold step was below 89 ms.
- The fixed 611-tile checksums are unchanged.
- World/sky reels are clean and two settled capture runs produced identical
  PNG bytes.
- One required gate is formally non-green: `perf_reel` repeatedly flags only
  the untouched moon-surface p95. Isolated identical runs alternate between
  one-vblank and missed-vblank timing; details are under “Perf reel”.

The archived file named in the mission,
`viewer/interchange/reviews/sol-b4-findings-2026-07-14.md`, is not present in
this checkout or any local Git ref. Baseline attribution below therefore uses
the mission text, commit history, and my own instrumented pre-change runs at
`20ab644`. No instrumentation remains in the diff.

## Design

### Cached truth

Each `Planet` owns a cache keyed by `(cube face, level-14 region i, level-14
region j)`. The key deliberately excludes render tile, render LOD, and season.
The `Planet` instance already scopes seed and loaded climate assets.

A retained candidate is at most 16 bytes:

- `u16` column offsets inside the level-14 region;
- `TreeKind`;
- exact `f64` density.

Absolute `(ci, cj)` and the exact tree lottery are reconstructed when serving
a tile. This avoids retaining two `u64`s and another `f64` per candidate.

The four stride lattices are disjoint tiers. A stride-8 request builds only
stride-8 coordinates; stride 4 adds the coordinates not already on stride 8;
stride 2 and stride 1 do the same. Thus each coordinate’s expensive annual
profile is evaluated at most once while all LODs reuse prior tiers. A finer
request re-filters coarser retained candidates with its original density
threshold, reconstructs the original tuple, and sorts `(ci, cj)` to restore
the old ci-major/cj-minor emission order.

### Exact rejection proofs

The cache may omit a partial region only when the old phase-one enumeration
itself was guaranteed to reject it: proven barren interior or proven
unconditional snow. Boundary regions first test a cheap level-16 grid and
adaptively refine only useful boundary leaves to level 20 (about 12 m). Warm
productive regions stop after the coarse test and enumerate normally.

The stronger annual `-6 °C` non-shrub treeline proof is used only for a whole
render tile. A recursive union proof can show that every subregion is either
an old phase-one rejection or a later authoritative treeline rejection. It
then skips the whole tile. It never removes only some late-rejected candidates:
those candidates historically participate in cap/boost accounting before the
late gate, so partially hoisting them would change surviving warm trees.

That distinction was added during final review and is locked by a mixed
barren/cold-boundary unit test plus a test that preserves the old
pre-decimation candidate stream around `-6 °C`.

### Concurrency

- 16 independently locked shards protect only map lookup, LRU accounting,
  and eviction.
- Enumeration happens outside shard locks.
- Per-region rejection, proof leaves, and each stride tier use `OnceLock`.
  Concurrent same-key misses build once; different regions continue in
  parallel on the existing outer Rayon tile workers.
- Active entries are pinned by `Arc`; eviction never invalidates a worker’s
  view.

No nested Rayon pool was retained: it competed with the renderer’s existing
parallel tile scheduling and worsened transition latency.

### Memory bound and eviction

- Retained candidate slices and retained proof-leaf slices are charged to a
  64 MiB cap, split as 4 MiB per shard.
- Each shard also has a 512-entry cap, so empty/proof-only regions cannot grow
  the map without limit: 8192 entry shells maximum across the cache.
- Eviction selects the least-recently-used unpinned entry in the over-budget
  shard. An entry in active use may cause transient in-flight overshoot; the
  request-end trim evicts it after workers release their `Arc`s.
- `HashMap`, `Arc`, allocator, mutex, and entry-shell overhead is outside the
  64 MiB slice-payload counter, but is bounded by the 8192-entry cap.

### Seasons

The old phase-one enumeration reads annual `temp_precip`, warped vegetation,
species, density, and the deterministic lot. Those are the only cached
quantities. `Sample::temp_c` is explicitly annual, and the exact impostor path
calls annual `col_ctx` for tree ownership. Seasonal water/freeze state, root
height, tint, snow appearance, beach/water/carve/cave/edit gates, and the
authoritative late `ColCtx` work remain after cache lookup. Therefore no
season key or invalidation is needed; seasonal rebuilds safely reuse the same
annual potential-candidate stream.

## Required gates

| Gate | Before/reference | Final | Result |
|---|---:|---:|---|
| `cargo test --release --lib` | 102 passed stated at HEAD | 106 passed, 0 failed, 1 ignored | PASS |
| `tile_cost` median | 0.182 s | 0.175 s | PASS, -3.8% |
| `world_reel.py` | accepted reel | all 28 clean | PASS |
| `sky_reel.py` | accepted reel | all 5 clean | PASS |
| `sync_diff.py` `sea_calib` | 0.0% requirement | 0.0%, mean/p95/lum all zero | PASS |
| verify suites excluding auto-survey | all expected green | 7/7 green | PASS |
| `perf_reel.py`, no `--accept` | own pre-change table | all Neisor rows pass; moon-surface p95 fails | **FORMAL FAIL / host-vblank cut-line** |
| settled capture double run | byte equality required | four of four PNG hashes equal | PASS |

### `tile_cost` exactness

| Value | Before | Final |
|---|---:|---:|
| median | 0.182 s | 0.175 s |
| vertices | 746031 | 746031 |
| geometry | 9233981608705220 | 9233981608705220 |
| other | 18863829261183721 | 18863829261183721 |
| shore | 1514547866741203 | 1514547866741203 |

The final five runs were 0.169, 0.175, 0.168, 0.180, and 0.187 s.

## B-4 raw-capture probes

All values are milliseconds with `TRI_RAW_CAPTURE=1`, each script in a fresh
process. `avg/p95` is shown for multi-frame benches. Single-frame fast-probe
rows have one value because avg, p95, and max are identical.

### Pose A — `64.120 -46.429`

#### `b4-ascent-fast.play`

| altitude km | Before | Final |
|---:|---:|---:|
| 0.05 cold, avg/p95 | 195.25 / 2713.91 | 88.41 / 608.07 |
| 0.2 | 33.56 | 33.53 |
| 0.8 | 36.88 | 32.37 |
| 3 | 37.42 | 33.87 |
| 10 | 37.57 | 35.31 |
| 40 | 44.38 | 38.02 |
| 160 | 42.72 | 35.54 |
| 640 | 27.71 | 24.61 |
| 1229 | 20.88 | 17.51 |

#### `b4-descent-fast.play`

| altitude km | Before | Final |
|---:|---:|---:|
| 640 cold, avg/p95 | 19.40 / 104.88 | 19.06 / 102.81 |
| 160 | 19.14 | 18.75 |
| 40 | 54.98 | 49.89 |
| 10 | 47.25 | 60.77 |
| 5 | 33.40 | 59.47 |
| 4 | 39.10 | 42.34 |
| 3 | 35.58 | 39.65 |
| 2.5 | 40.20 | 37.93 |
| 2 | 2102.47 | 61.04 |
| 1.5 | 315.19 | 225.61 |
| 0.8 | 118.13 | 76.03 |
| 0.2 | 151.81 | 113.81 |
| 0.05 | 106.69 | 95.60 |

This one-frame teleport probe intentionally outruns lookahead. A repeat moved
the remaining urgent-build spike from 1.5 km to 0.8 km (81.63 and 212.45 ms,
respectively), rather than reproducing the old deterministic 2 km enumeration
wall. Never-block ancestor drawing remains out of scope.

#### `b4-descent-fly.play`

| altitude km | Before avg/p95 | Final avg/p95 |
|---:|---:|---:|
| 40 cold | 48.39 / 386.73 | 46.05 / 386.85 |
| 12 | 36.51 / 42.44 | 36.58 / 45.66 |
| 9.6 | 36.95 / 41.25 | 35.04 / 37.82 |
| 7.7 | 38.90 / 52.42 | 31.28 / 34.03 |
| 6.1 | 33.70 / 36.89 | 31.64 / 33.79 |
| 4.9 | 40.92 / 50.41 | 31.81 / 32.89 |
| 3.9 | 41.62 / 48.64 | 32.77 / 34.65 |
| 3.1 | 34.07 / 35.91 | 32.71 / 33.64 |
| 2.5 | 37.81 / 41.57 | 33.19 / 34.02 |
| 2 | 172.28 / 697.88 | 51.17 / 86.37 |
| 1.6 | 44.76 / 68.94 | 40.19 / 62.45 |
| 1.3 | 42.52 / 65.70 | 40.13 / 61.96 |
| 1 | 38.34 / 59.07 | 36.98 / 55.43 |
| 0.8 | 36.00 / 47.16 | 34.38 / 45.41 |
| 0.6 | 38.61 / 58.01 | 36.17 / 56.35 |
| 0.45 | 36.32 / 46.01 | 33.92 / 42.11 |
| 0.35 | 34.80 / 40.83 | 32.54 / 39.47 |
| 0.25 | 36.57 / 45.99 | 33.98 / 44.17 |
| 0.15 | 33.95 / 38.02 | 33.62 / 37.96 |
| 0.05 | 28.39 / 34.85 | 27.47 / 34.62 |

### Pose B — `80.909 -74.619`

#### `b4-ascent-fast-repro.play`

| altitude km | Before | Final |
|---:|---:|---:|
| 0.05 cold, avg/p95 | 56.77 / 369.19 | 57.89 / 440.59 |
| 0.2 | 33.88 | 33.15 |
| 0.8 | 33.99 | 34.96 |
| 3 | 32.79 | 34.60 |
| 10 | 34.44 | 32.51 |
| 40 | 32.99 | 32.29 |
| 160 | 31.17 | 28.37 |
| 640 | 32.48 | 28.25 |
| 1229 | 15.06 | 13.52 |

#### Fast descent, pose-rewritten probe

| altitude km | Before | Final |
|---:|---:|---:|
| 640 cold, avg/p95 | 17.74 / 92.18 | 19.00 / 94.20 |
| 160 | 19.45 | 22.93 |
| 40 | 35.22 | 33.99 |
| 10 | 37.54 | 43.21 |
| 5 | 42.12 | 44.63 |
| 4 | 37.65 | 38.75 |
| 3 | 32.84 | 33.52 |
| 2.5 | 35.73 | 38.07 |
| 2 | 41.03 | 55.48 |
| 1.5 | 289.58 | 237.16 |
| 0.8 | 51.15 | 62.30 |
| 0.2 | 85.30 | 88.61 |
| 0.05 | 68.74 | 70.27 |

#### Continuous descent, pose-rewritten probe

| altitude km | Before avg/p95 | Final avg/p95 |
|---:|---:|---:|
| 40 cold | 32.03 / 159.00 | 32.64 / 155.38 |
| 12 | 32.28 / 36.26 | 30.93 / 36.44 |
| 9.6 | 32.12 / 34.43 | 31.09 / 33.60 |
| 7.7 | 33.12 / 34.37 | 31.11 / 31.42 |
| 6.1 | 33.96 / 34.86 | 33.11 / 34.20 |
| 4.9 | 34.73 / 35.06 | 33.69 / 34.82 |
| 3.9 | 35.19 / 36.62 | 34.73 / 35.54 |
| 3.1 | 36.03 / 36.60 | 34.89 / 35.60 |
| 2.5 | 34.66 / 41.44 | 33.63 / 37.75 |
| 2 | 66.72 / 217.48 | 67.47 / 226.56 |
| 1.6 | 37.31 / 50.32 | 38.15 / 51.41 |
| 1.3 | 38.31 / 45.93 | 35.60 / 44.35 |
| 1 | 36.75 / 48.26 | 36.26 / 48.38 |
| 0.8 | 36.13 / 48.84 | 34.88 / 46.39 |
| 0.6 | 34.42 / 39.64 | 33.62 / 39.60 |
| 0.45 | 34.58 / 41.94 | 34.02 / 41.94 |
| 0.35 | 33.70 / 37.68 | 33.55 / 37.64 |
| 0.25 | 35.76 / 41.59 | 34.66 / 42.83 |
| 0.15 | 30.24 / 32.65 | 30.24 / 32.45 |
| 0.05 | 20.64 / 24.08 | 20.69 / 24.16 |

The pose-B 2 km result is not candidate work: the same probe with
`TRI_NO_IMPOSTORS=1` measured 215.52 ms p95, versus 226.56 ms with the final
candidate path. It is the unchanged non-impostor scheduling floor at that
pose and is outside B-4a’s cache mechanism.

## Reels, synchronization, and suites

| Command/suite | Final |
|---|---:|
| `python scripts/world_reel.py` | 28/28 clean |
| `python scripts/sky_reel.py` | 5/5 clean |
| `python scripts/sync_diff.py` `sea_calib` | 0.0% divergent; mean 0, p95 0, lum +0.0 |
| `physics-regressions` | PASS |
| `water-regressions` | PASS |
| `lake-regressions` | PASS |
| `flooded-caves` | PASS |
| `invariant-survey` | PASS |
| `camera-controls` | PASS |
| `solar-system` | PASS |

`auto-survey` was intentionally omitted because this worktree has no
`output/` survey data, as requested.

## Perf reel

I created an ignored local `interchange/perfreel/accepted.json` from the
pre-change run and never used `--accept` after changing source.

| Scenario | Before avg/p95 | Latest final avg/p95 | Comparator |
|---|---:|---:|---|
| forest_ground | 29.66 / 39.66 | 30.50 / 43.44 | pass |
| plains_walk | 24.49 / 36.39 | 25.16 / 31.94 | pass |
| mountain_mid | 25.89 / 29.99 | 24.59 / 26.20 | pass |
| ocean_storm | 23.38 / 25.38 | 20.54 / 21.93 | pass |
| orbit | 17.27 / 21.93 | 17.23 / 20.14 | pass |
| moon_surface | 16.65 / 17.94 | 17.92 / 28.47 | **p95 fail** |
| moon_orbit | 2.55 / 3.16 | 2.61 / 3.14 | pass |
| ascent_sweep | 35.54 / 39.98 | 33.64 / 34.98 | pass |

Four corrected-source full runs all passed every Neisor scenario. Their
moon-surface results were 18.16/28.03, 20.57/29.30, 20.65/28.17, and
17.92/28.47 ms; other occasional cross-body flags disappeared on repeat.

This is a vblank/compositor bimodality, not an impostor-cache execution path.
Five isolated invocations of the identical generated moon script alternated
between fast samples (15.56/16.64 and 15.93/16.90 ms) and missed-vblank
samples (19.69/28.31, 18.40/28.59, and 19.96/27.99 ms). Toggling
`TRI_NO_IMPOSTORS` did not remove it. The formal reel remains failed because
the required command’s final p95 crossed its threshold; no renderer or moon
timing workaround was added to this terrain-only mission.

## Byte determinism

Exact same command twice:

`target/release/examples/play.exe scripts/tint-seams.play --out interchange/determinism-b4a`

| PNG | Run 1 SHA-256 | Run 2 SHA-256 |
|---|---|---|
| `grass_grass_2km.png` | `D6BA01634DF7ACA30484E21A3FACAECCB78C536F28AED21DDC945AE57263BCBE` | same |
| `sand_grass_2km.png` | `FB86644AF9436EE2E7FC0A01A215987F50EB6520288E817605007DC44C73B999` | same |
| `sand_grass_straddle.png` | `E88C376323DE7880FD84DE04590CB25D668BBC0F04C773F8328BCE1CCEDB70F0` | same |
| `scenic_savanna_2km.png` | `7AD83188C81656F5917BFD267FD780FF3C41796D9E8061A52EC7DE54BB68466E` | same |

## Exactness tests added

1. Cache miss, hit, and concurrent overlapping Rayon misses match the old
   uncached candidate stream bit-for-bit at strides 8, 4, 2, and 1.
2. Cache entries and slice bytes obey the shard entry/payload bounds.
3. Pre-decimation candidates around `-6.1/-6.0/-5.9 °C` remain in the cached
   phase-one stream, preventing cap/boost drift at mixed treelines.
4. A whole-tile mixed barren/cold boundary can be rejected by the union proof,
   while a productive warm version correctly declines the proof.

## Honest cut-lines

- The one-frame fast-teleport probe can still bank a 200+ ms urgent build at
  1.5 or 0.8 km. The realistic continuous target is bounded; drawing an
  ancestor instead of synchronously completing any urgent build is the
  explicitly banked Andrew scheduling decision and was not implemented.
- Pose B’s 2 km continuous spike is unchanged with impostors disabled. It is
  not enumeration work and remains for the scheduling/timestamp effort.
- A first-ever productive warm region still has to derive each previously
  unseen residue tier once. The cache makes that work reusable and bounded in
  residency; it does not manufacture candidate profiles without evaluating
  their authoritative annual field.
- The 64 MiB cap measures retained slice payload, not allocator/map shell
  overhead. The separate 8192-entry cap bounds that overhead; pinned workers
  can briefly exceed a shard limit until request-end trim.
- Upload batching, GPU work, moon rendering, renderer pass structure, and
  never-block ancestor scheduling were untouched.
- The formal perf reel is not wholly green because of the documented
  moon-surface vblank tail. All source-relevant Neisor rows pass.

## Diff ownership

Mission changes are confined to `viewer/src/planet.rs`,
`viewer/src/terrain.rs`, `viewer/src/voxel.rs`, and this file. Existing dirty
files (`viewer/assets/weather_tuning.json`, mission/log files, and populated
multiplayer assets) were not modified by B-4a.
