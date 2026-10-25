# Lunar column lattice and performance findings

## Result

The moon now owns a 2,700,000-column-per-face lattice while Neisor remains at
10,000,000. The lunar value is exactly `10,000,000 * 27 / 100`, matching the
configured `0.27 R` radius ratio. It is also exactly divisible by the
32-column chunk edge: 84,375 chunks span every cube-face axis. This restores
Neisor-equivalent metric block proportions and reduces lunar column density by
`1 / 0.27^2 = 13.717421...`.

`VoxelBody::columns_per_face()` is the one body contract. The body-generic
direction/column helpers, canonical cross-face mapping, chunk selection,
chunk geometry, collision and support/ceiling/water queries, raycasts, edits,
teleport-to-surface, photo-map moon landing, scripted assertions, `moonprobe`,
and `colcensus --body moon` all consume it. `moonprobe` now reports and samples
the canonical lunar column selected by the requested direction.

The existing body-relative lunar patch radius and the renderer's landed
two-regime f64 offset rule remain in place. A landed-only distant LOD-ring
threshold of 0.25 removes the draw-call floor without changing max-level tiles
at the voxel/mesh rim; both sides still sample the same lunar law.

## Block proportions

`moon-landing.play` digs one block and probes the mean world-space distance to
the +i/+j neighboring column centers, normalized by the same Neisor probe.
Radial shells are one metre on both bodies. The measured
`block_width_height_ratio` is exactly 1.000000 and the new assertion requires
`1.0 +/- 0.02`.

The wider hole now genuinely owns the player column. The script deliberately
waits 1.3 s for the player to settle on the one-block-deeper floor before the
replacement-block jump; the subsequent airborne clearance is 1.943750 m.
The existing Neisor jump pin remains exactly 1.334533 m.

## Crater prefetch

For compact lunar builds, the sampler now gathers canonical crater cells for
all ten octaves and ray carriers once for the complete footprint, expands each
crater once, caches its invariant impact-center macro surface, and stores
age-sorted per-sample indexes. `build_chunk` calls the body batch API for its
40x40 context grid, and landed 33x33 moon mesh tiles use the same batch sampler.
Broad coarse-LOD tiles, whose vertices share essentially no crater cells, keep
the faster existing scalar/cache path. Crater, mare, ray, and material equations
were not changed.

The release test `prefetched_batch_is_bit_identical_to_scalar_sampling`
compares every `f64` output field by bits over a tile footprint, a cube-face
rim, and unrelated directions.

Fresh-process PNG determinism is also byte-identical before/after prefetch:

| frame | SHA-256 before and after |
|---|---|
| `crater_field.png` | `55E99DD11BFCE0A78501CA737C32983F661CE7C1BB11449B5253662509ACA25F` |
| `moon_face.png` | `CE4BD4D5C1D10291D5C23895D71526350BA904CC5D5DCDC0A8A9DFC97D12EACA` |

## Performance

All numbers are release/headless on this machine. The navigation rows use the
same `moon-nav-bench.play` and `TRI_RAW_CAPTURE=1` before and after; the sweep
crosses the maria landing and the highland/crater-floor site.

| gate | before | after | result |
|---|---:|---:|---:|
| Landed steady, fully warmed (30 frames) | P3 reference: 28 ms | avg 12.98 ms, p95 13.76 ms | pass, avg 53.6% below P3 |
| Landed identical 5-warm + 30-frame protocol | avg 40.88 ms, p95 170.92 ms | avg 12.27 ms, p95 13.72 ms | avg -70.0%, p95 -92.0% |
| NAV post-hold segment worst p95 | 80.80 ms | 17.77 ms | pass, -78.0% |
| Maria post-hold p95s | 80.80 / 65.99 ms | 17.77 / 13.20 ms | both more than halved |
| Highlands post-hold p95s | 66.58 / 56.01 ms | 9.65 / 8.89 ms | both more than halved |
| One warm lunar chunk, 5 builds | avg 9.08 ms, p95 9.14 ms | avg 6.34 ms, p95 6.45 ms | avg -30.2% |
| Compact 33x33 tile sampler (`uv size 0.0078125`) | scalar 5.076 ms | prefetched 3.656 ms | -28.0%, bit-identical |

## Lunar edit reset (intentional and loud)

Old moon column keys cannot be interpreted on the new lattice. The old
`edits_moon_seed42.bin` is left untouched for recovery but is never loaded.
New saves use `edits_moon_lattice2700000_seed42.bin`. When only the legacy file
exists, startup prints:

`MOON EDIT RESET: ignoring legacy 10000000-column lattice edits; new lunar lattice has 2700000 columns/face (Neisor edits are untouched)`

Neisor continues to read and write the existing `edits_seed42.bin` with the
same `EDT1` format and validation bound. Neisor torches are likewise unchanged.

## `COLUMNS_PER_FACE` audit

Every remaining use is either explicitly Neisor-only or participates in the
body dispatch:

- `planet.rs`: definition plus Planet climate/category comparator generation
  and Planet-only tests. All are Neisor-only.
- `terrain.rs`: Planet terrain/ecotone/beach/tree paths and tests. All are
  Neisor-only; lunar terrain never enters this module's Planet sampler.
- `shader.wgsl`: a comment describing the Neisor ecotone frequency.
- `caveprobe`, `colmap`, `colof`, `physim`, `probe`, and `vegprobe`: explicitly
  Neisor/Planet diagnostic examples. `play`'s remaining constant use is the
  Neisor-only `probe LAT LON` terrain command.
- `renderer.rs`: the sole direct use is the Neisor-only torch-light position
  path. Generic chunk streaming uses `body.columns_per_face()`.
- `main.rs`: direct Neisor constants validate Neisor edits/torches and label
  the rejected legacy moon lattice. Lunar validation uses
  `LUNAR_COLUMNS_PER_FACE`.
- `voxel.rs`: the Neisor re-export, Neisor compatibility wrappers
  (`canonical_column`, `column_of`, `col_ctx`), Neisor trait implementations,
  the exact lunar 27/100 derivation, the `BodyId` dispatch, and lattice tests.
  Every body-generic caller uses the parameterized helpers.

No unexplained direct constant use remains in a body-generic path.

## Gates

- `cargo test --release --lib`: 73 passed, 0 failed, 1 pre-existing ignored.
- Release play build and main-binary `cargo check`: pass (only the three
  pre-existing deprecation/unused warnings).
- Script suites: physics, water, lake, flooded caves, invariant survey,
  auto survey, camera controls, `solar-system`, `moon-landing`, `blackhole`,
  and `seasonal-cycle` all pass.
- `world_reel`: all 28 poses clean. All Neisor poses are byte-clean. The
  current reel's two moon poses are orbital (1,700 km) and low flyby (8 km),
  not landed voxel poses, so neither changes. The landed block-pose changes
  are instead covered by `moon-landing.play`: `maria_grounded` and
  `lunar_jump_apex` show the wider lattice, `dug_regolith` shows the full-width
  one-block hole, and `ray_field_close` uses the landed distant-ring budget.
- `sky_reel`: all 5 poses clean; the moon mesh output remains byte-identical.
- `sync_diff`: `sea_calib` is exactly 0.0% divergent, mean/p95/max delta 0.
- No commit was created.
