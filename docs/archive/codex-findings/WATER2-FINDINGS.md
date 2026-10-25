# Andrew field reports 4 + 5 — pond rampart and lake rim band

Branch: `sol/rampart-rimband`, based on `0d5089b`. No commit was made.

## Bug A — pond apron rampart

### Before

The exact pose is
`viewer/interchange/runs/rampart-rimband-baseline/bugA_before.png`.
The fine baseline probes at the photographed cut found natural ground at
about 4.5 m beside pond-benched ground at 26.7 m and water at 28.2 m. The
350 m block census reported:

- `LIP: 323` columns, worst drop 24 blocks;
- `EDGE: 295` columns.

The same artifact continued along the contour in:

- `viewer/interchange/runs/rampart-contour-baseline/contour_south_before.png`;
- `viewer/interchange/runs/rampart-contour-baseline/contour_north_220m_before.png`;
- `viewer/interchange/runs/rampart-contour-baseline/contour_north_550m_before.png`.

### Decision and fix

I chose candidate (a), local suppression of water that needs a dam, rather
than widening the apron. At this site the `pn` profile traverses almost its
entire nominal apron inside one or two metre-scale columns; widening the same
noise-relative profile would retain the artificial retaining landform over a
larger footprint.

The new rule caps structural pond fill at 3 m in both the interior bench and
the exterior apron. The cap is continuous: when required fill crosses 3 m,
ground follows `natural + 3 m` instead of Phase 8h's rejected all-or-nothing
target/natural snap. A pond dig is committed only after the existing
never-perched check proves that this bounded fill can support the water, so a
suppressed pond also leaves no dry crater.

### After

The exact after pose is
`viewer/interchange/runs/rampart-rimband-final/bugA_after.png`. The former cut
now probes at roughly 4.5–7.6 m, with no pond water or pond level. The same
350 m colcensus is now `LIP: 0`, `EDGE: 0`. The long contour checks are:

- `viewer/interchange/runs/rampart-rimband-final/contour_south_after.png`;
- `viewer/interchange/runs/rampart-rimband-final/contour_north_220m_after.png`;
- `viewer/interchange/runs/rampart-rimband-final/contour_north_550m_after.png`.

The remaining contour-scale ground ripple is bounded to a few blocks; the
22–24 m retaining wall and its perched water are gone.

## Bug B — pale band over lake water

### Before and diagnosis verification

The A/B baseline is:

- voxels on: `viewer/interchange/runs/rampart-rimband-baseline/bugB_before_vox.png`;
- voxels off: `viewer/interchange/runs/rampart-rimband-baseline/bugB_before_mesh.png`.

The reported quantization mismatch was real: at the camera the analog lake
level is 122.408949 m, while the old liquid query/top used the floored water
block plus patch lift (`water_surface_km = 123.600 m`). However, instrumenting
the photographed face showed that the dominant pale band was 3.6 m tall, not
sub-metre. Centerline probes located the actual discontinuity:

- about 50 m ahead: lake water 122.4 m, `river_wet = 0.49`;
- about 75 m ahead: graph river water 126.0 m, `river_wet = 0.53`;
- 100–150 m: the same 126.0 m river surface over ~122.4 m ground.

The old `perch < 0.5` occupancy gate therefore switched from lake level to
the full graph river level in one column. The block-only side/foam face was
the photographed band; the mesh paints the fading river on ground near the
lake level and never creates that 3.6 m riser.

### Fix

Both layers were corrected:

1. `ColCtx` retains the authoritative continuous open-water level. Liquid top
   quads, their foam edges and side tops now use that analog height; frozen
   ice remains on its exact walkable block shell and cave pools retain their
   existing lattice path. `water_surface_km` calls the same dry-bank-clamped
   height helper, so swimming agrees with the visible liquid surface.
2. The river never-perched handoff now has a supported visible level. It is
   the graph level through 2 m clearance, tapers continuously to the stable
   natural reference from 2–4 m, and becomes truly dry at 4 m (rather than
   occupying the carved bed at the reference height). The graph level itself
   is retained for groundwater and shore ownership. This removes the binary
   122.4→126.0 m jump without weakening the dry-wash rule.
3. Equal-block liquid-lake shoal caps use the same analog plane and remain
   solid/walkable. No shader change was needed, so the `wflag` water-plane
   exemption that prevents underwater void cracks is unchanged.

### After

The final A/B is:

- voxels on: `viewer/interchange/runs/rampart-rimband-final/bugB_after_vox.png`;
- voxels off: `viewer/interchange/runs/rampart-rimband-final/bugB_after_mesh.png`.

The pale riser is gone and no black/void crack appears. The same centerline
now stays at lake level and then becomes dry/flush through the former
perched-river interval. Site colcensus improved from eight 3-block `LIP`
columns to `LIP: 0`; only 145 informational one-block `EDGE` columns remain.
At the camera, visible/physics liquid height is 124.008949 m including the
unchanged 1.6 m local patch lift.

## Gates

- Explicit release build: `cargo build --release --example play --example colcensus` — pass.
- `cargo test` — pass: 22 library tests, golden noise parity, doctests.
- Survey regeneration: 69 probes across seven classes.
- All seven suites — pass: `physics-regressions`, `water-regressions`,
  `lake-regressions`, `flooded-caves`, `invariant-survey`, `auto-survey`,
  `camera-controls`.
- Exact HEAD-vs-final full sample census, using the same assets: raw findings
  `40,754 → 25,266`, `WALL 39,528 → 24,690`, `JUMP 1,192 → 546`,
  `SEAJUMP 34 → 30`, and `LIP 0 → 0` across about 103 million samples. The
  deduplicated cluster count is not a class total and can split when bridging
  findings disappear; it moved `13,878 → 17,264` while every raw class fell.
- Exact HEAD-vs-final full lip census: 221,841,122 samples in each run,
  `LIP 61,360 → 60,777` and grouped sites `42,050 → 40,385`; `WALL`, `JUMP`,
  and `SEAJUMP` remain zero in this mode. The photographed site deltas are
  recorded above.
- Standing sync-diff: `lake_shore` mean delta 15.39 → 11.77; `karst` 13.3 →
  11.83; savanna/desert unchanged at 7.35/6.77; `ice_top` 8.76 with +3.4
  luminance; `sea_calib` 0.0%.
- Byte determinism: two complete runs produced identical hashes for all 12
  PNG/JSON artifacts. Primary frame SHA-256 values:
  - Bug A: `1affffdc2c35ea0a3cfbcd8858095e4ea811d7ac328bdf7ca3e015fed608817b`;
  - Bug B voxels on: `0edfeca529d3a2adf50f821861083f19d982cbc8969f86c4a76c72d730f23da0`;
  - Bug B mesh-only: `cfb4fe3ef83b45cb7f11d5a3d8bacc3bd5f4171dcbab18b6b953f134939850fb`.

Regression scripts are `viewer/scripts/rampart-rimband.play` and
`viewer/scripts/rampart-rimband-probe.play`.
