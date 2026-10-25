# Field Bug Hunt 3 Findings

## 1. Medium: voxel path draws a light-cyan water slab over open river-mouth water

Repro: `viewer/scripts/hunt3-1.play`

Frames:
- `viewer/interchange/runs/hunt3-1/voxel_default.png` shows a large light-cyan, jagged polygon covering the foreground water.
- `viewer/interchange/runs/hunt3-1/pure_mesh.png` at the same pose is uniform water with no slab.
- `viewer/interchange/runs/hunt3-1-syncdiff/diff.png`, `mesh.png`, and `voxels.png` preserve the sync-diff pair.

Pose: `teleport 7.042 33.477 0.30`, `look -55 -16`, `weather off`.

Instrument evidence:
- Harness probe: `probe 7.04200 33.47700: h=-12.7m water=0.4m lake=false lake_lvl=- pond_lvl=- riv hw=365.5m wet=1.00 | col ground=-13 water=0 cave_water=-`
- `sync_diff.py --at 7.042 33.477 0.30 -55 -16`: `divergent 21.8%`, `mean d 24.99`, `p95 d 26`, `lum bias +21.4`.
- `colcensus.exe 7.042 33.477 1.0`: `LIP: 0`, `EDGE: 558`, `CAVEP: 0`.
- `census.exe --at 7.042 33.477 --radius 2`: `0 raw findings`, `0 sites`, `80802 samples`.

Why it counts: the slab is not a shoreline quantization edge in the frame; it sits across open foreground water, and pure mesh renders the same water continuously. Block truth reports river water at the camera column and no LIP/CAVEP breach.

Suggested fix direction: audit the near-field voxel water material/mask at river-mouth blends; the voxel path appears to apply a shallow/river tint surface over water that the mesh path treats as one continuous river surface.

## Coverage

Ran:
- `viewer/scripts/hunt3-breadth.play`: scenic sweep over temperate river valley, jungle delta, steppe river mouth, inland sea, lake archipelago, desert river, groves/ponds, taiga caves, savanna, highest peak, and Difficulty Lake.
- Altitudes covered: walk/eye height around `0.0025`, low flight `0.05-0.30`, mid `1-12`, high `80`, and orbit `1200`.
- Conditions covered: weather off/default noon, `weather pin 1.0 0.9`, `weather season 0.0`, `weather season 0.5`, pinned dawn sun, and pinned night-side sun.
- `viewer/scripts/hunt3-feature-sweep.play`: frozen-lake class, sea ice, open oceans, generated river targets, high peaks, and sampled cube-face edge land.
- Physics: `viewer/scripts/physics-regressions.play`, `viewer/scripts/auto-survey.play`, and `viewer/scripts/hunt3-walk-karst.play` all completed without assertion failure.
- Renderer-diff checks: scenic/water/ice poses included river mouth, delta, ocean, sea ice, groves, taiga, savanna, peak, Difficulty Lake, and conifer forest. High vegetation/ice/karst divergences were visually consistent with expected voxel terrain or ledgered patch/ice/cave-rim classes.
- Determinism: `viewer/scripts/hunt3-determinism.play` rendered twice to `hunt3-determinism-a` and `hunt3-determinism-b`; all PNG MD5 hashes matched, including live weather.

Not counted because they resemble ledgered items:
- Difficulty Lake eye-level blocky dirt faces: `colcensus` around `13.346 -4.807` reports dense `CAVEP` and the frame resembles the known residual cave-pool rim / karst false-pool class.
- Sea-ice near-field/pure-mesh divergence at `83.997 40.220`: visually matches the known ice/patch-boundary class rather than a new structural mismatch.

Not reached:
- No exhaustive global random sweep.
- No full night/orbit matrix for every biome.
- No windowed viewer session and no rebuild, per mission constraints.
