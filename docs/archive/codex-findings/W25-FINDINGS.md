# W2.5 — heterogeneous deck + wind map

Status: complete on `sol/w25-heterogeneous` at base
`200c8decae53fa03f04310f9db8055adeddf42e3`. No commit was made.

The architectural gap is closed: live cloud shape no longer comes from the
camera-to-planet-mean scalar. The renderer, cloud shells, and teleport map now
share one deterministic spatial cover/precip raster. At `t=3500`, its global
cover mean is `0.394`, standard deviation is `0.387`, and sampled land spans
the full `0.000..1.000` range. The orbital deck consequently has regional
storm belts, clear lanes, and thin/thick sectors while retaining the existing
local formation fabric.

[Full-resolution contact sheet](viewer/interchange/w25/contact_sheet.png)

![W2.5 contact sheet](viewer/interchange/w25/contact_sheet.png)

## Match gate

Both panels below use seed 42, live weather `t=3500`, center `20°N, 0°E`, and
north-up orientation. The map is zoom 3; the sky camera is at 4,000 km. The
two continents and central strait provide the geographic registration. Gray
on the map is thicker cover; base blue/green showing through is thinner. The
broad carrier agrees across the panels, while only the sky adds sub-synoptic
formation fabric.

| Teleport-map cover overlay | Orbital deck from the same geography |
|---|---|
| ![Map cover overlay](viewer/interchange/w25/cover_match_t3500.png) | ![Same-view orbital deck](viewer/interchange/runs/w25-heterogeneous/match_orbit.png) |

[Labeled side-by-side pair](viewer/interchange/w25/map_sky_match_pair.png)

## Implementation

### Shared synoptic raster

- `SynopticRaster` is a 6 × 64 × 64 RGBA8 cube (`98,304` bytes). R is cover,
  G is precipitation, B is reserved, and A is 255. Face nodes include cube
  edges and use the same face-direction convention as `Planet`.
- Live bytes are a pure function of `(seed, quantized absolute weather time)`.
  Time is quantized to deterministic two-second buckets; any bucket change or
  explicit seek rebuilds from scratch. There is no accumulation, RNG draw,
  wall clock, or frame-count input. Six faces are evaluated in parallel but
  written to fixed byte offsets.
- The renderer keeps the current source key (`Off`, `Live`, or `Pinned`) and
  uploads only when that key changes. The CPU copy is also exposed to the map,
  so the map cannot silently re-evaluate a different weather field.
- The map's cloud texture now invalidates on the same two-second live source
  key. Seasonal-only map bases retain their previous 60-second cadence.

### Per-shell spatial deck

- Each valid low/mid/high shell hit projects its planet direction into the
  cube raster and performs one filtered lookup. A tiny world-stable tangent
  dither hides 64² interpolation contours without moving synoptic geography.
- Sampled cover/precip replace `weather8.xy` for live **shape**. Existing
  analytic cyclone and front boosts still compose afterward. Existing cloud
  fabric remains the local-scale layer; no pass-1 warp/shear/morph term was
  restored or added.
- `weather8` remains the camera-local path for temperature and all ground
  responses (dusting, rain, fog, gloom, humidity probes). Those paths were not
  spatially rewired.
- B-8's low-cover failure had a second contributor: the legacy inverse-cover
  cirrus law made cover zero produce the strongest global fine speckle. Live
  cirrus now gates that law by synoptic cover, so a clear raster lane stays
  clear. Pins retain the legacy law exactly.

### Pin compatibility

- A cover/precip pin creates a spatially uniform cube at the pinned values,
  independent of time.
- The shader also takes the exact scalar pin for shape, bypassing UNORM8
  rounding, and keeps the legacy cirrus presence curve. This preserves the
  instrument/reel pixels while still making the shared map raster uniform.
- Weather-off uploads the deterministic empty raster. Live, pinned, and off
  sources therefore all have explicit capture-safe states.

### Wind map

- The Neisor teleport map has a `Wind` checkbox beside `Clouds now`; it is
  disabled for the moon.
- Short streamlines integrate the baked east/north wind plus deterministic
  cyclone swirl/convergence and synoptic differential drift. Fading tails and
  bright heads give the requested comet-stroke read.
- Default knobs are `wind_map_density = 180` and
  `wind_map_length_km = 1250`. Regional path length scales by `1/sqrt(zoom)`,
  so both whole-world and zoomed views remain legible. The same geometry is
  used by egui and the headless evidence exporter.
- The visualizer is view-local only; it does not mutate or integrate weather
  state. The source tangent field is bit-deterministic, finite, and tested to
  remain orthogonal to the planet direction.

### Capture-law repair required by the determinism gate

The first strict double run exposed pre-existing scheduling dependence in
capture settling. The initial capture draw occurred before temporal LOD
settling was enabled, and the independent tile/chunk drains could wait on the
wrong channel or stop with a stand-in still selected. This affected a few
pinned orbital and time-stepped ground pixels even though weather was pure.

Capture settling now covers the initial draw, drains both stream systems as
one bounded convergence loop, and builds the capture's selected missing/stale
heightfield tiles as one deterministic parallel batch. Live/window rendering
keeps its asynchronous throttles and temporal morphs. This is why the final
30-shot weather replay is byte-identical and the accepted world/sky reels are
also unchanged.

## Multi-scale visual result

- **12,000 km:** the full disc shows several broad clear/thick sectors and
  storm belts simultaneously; the old planet-wide fine-speckle read is gone.
- **4,000 km match view:** the carrier still follows the map's two-continent
  geography and clear strait lane.
- **500 km:** the same coast divides a thin/clear land sector from a thick
  ocean sector, while individual puffs, fibrous streaks, and holes remain.
- **Ground:** the raster-maximum land pose is rainy/veiled under a broad deck;
  the raster-minimum land pose has a clean blue vault. Their raster values are
  1.000 and 0.000 respectively.

The existing weather field already supplied enough synoptic variance once it
reached the shader, so no additional weather octave was needed. W4 seasonal
machinery, moon code, and the reverted pass-1 fabric area were not changed.

## Determinism and regression gates

| Gate | Result |
|---|---|
| `cargo build --release --example play --example weathermap` | PASS |
| `cargo test --release --lib` | PASS — 98 passed, 1 ignored |
| Raster bytes, same `(seed,t)` twice | PASS — exact byte equality |
| Raster vs `weather_at` at face nodes | PASS — cover/precip within `0.5/255` |
| Pinned raster | PASS — every pixel identical; time-independent |
| Wind tangent field | PASS — bit-equal, finite, nonzero, tangent |
| Weather-visual full double run | PASS — 30 vs 30 PNGs, 0 SHA-256 mismatches |
| Cover/wind headless replay | PASS — 3 vs 3 PNGs, 0 SHA-256 mismatches |
| World reel | PASS — all 28 poses clean |
| Sky reel | PASS — all 5 poses clean |
| `sea_calib` | PASS — 0.0% divergent, mean 0.0, p95 0 |
| Asserting play suites | PASS — physics, water, lake, flooded-caves, invariant, auto-survey, camera-controls, solar-system |
| `git diff --check` | PASS |

The `sea_calib` raw maximum channel delta was 1, below the instrument's
established noise floor of 2; its reported divergent fraction is exactly
0.0%.

## Performance budget

The final live play capture measured a `3.468 ms` raster bake; the independent
map exports measured `3.4–4.0 ms`. At one bake per two seconds this is about
`0.029 ms/frame` amortized at 60 fps for the final play measurement. The
texture is 96 KiB on the CPU and 96 KiB on the GPU; steady upload bandwidth is
48 KiB/s. Cloud shading adds one bilinear texture read per active shell sample.

The wind overlay's default 180 paths × 14 short integration steps is evaluated
only while synthesizing an open Neisor map.

Final clean performance reel:

| Scenario | Accepted avg / p95 ms | W2.5 avg / p95 ms | Gate |
|---|---:|---:|---|
| forest_ground | 77.89 / 112.06 | 30.29 / 43.12 | PASS |
| plains_walk | 31.99 / 41.16 | 26.75 / 34.94 | PASS |
| mountain_mid | 39.36 / 61.67 | 25.38 / 27.77 | PASS |
| ocean_storm | 42.03 / 44.07 | 20.54 / 22.15 | PASS |
| orbit | 20.46 / 23.94 | 16.84 / 18.09 | PASS |
| moon_surface | 22.28 / 32.39 | 26.24 / 28.70 | PASS (+17.8% avg, below +20% limit) |
| moon_orbit | 3.27 / 4.40 | 2.73 / 3.09 | PASS |
| ascent_sweep | 47.68 / 59.12 | 33.08 / 35.04 | PASS |

One earlier performance pass caught transient `moon_orbit` variance at
4.12 ms average; the immediate full rerun above was clean. No code or baseline
was changed between those runs.

## Files and evidence

Tracked implementation/docs:

- `viewer/src/weather.rs`, `renderer.rs`, `shader.wgsl`, `ui.rs`, `main.rs`
- `viewer/src/planet.rs` (lightweight weather-test fixture only)
- `viewer/examples/weathermap.rs`
- `viewer/scripts/w25-heterogeneous.play`
- `viewer/scripts/w25_contact_sheet.py`
- `WEATHER.md`, `BUGS.md`, this `FINDINGS.md`

Generated evidence is under `viewer/interchange/w25/` and
`viewer/interchange/runs/w25-heterogeneous/`. The pre-existing untracked
`run.err` and `run.log` were left untouched. No commit was created.
