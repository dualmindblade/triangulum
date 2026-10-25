# W2 findings — weather you can see coming

Branch `sol/w2-visuals`, base `95489991690544bfa145280dc2b257d41e33f324`.
No commit was made, no reel was blessed, and no windowed viewer was used.

## Outcome

W2 is implemented as presentation-only weather. It does not touch W4's
structural season machinery, biome/blend fields, terrain ownership, or moon
code.

- **Cloud shadows:** every ground fragment (mesh and voxel through the same
  `fs_main`) traces toward the physical Sun, intersects all enabled cloud
  shells, and calls the existing `cloud_layer_sample` at those intersections.
  The projected alpha therefore uses the exact visible cloud thresholds,
  seed, shell scales, drift, evolving warp, octave shear, and broad morph.
  Layer transmittance is stacked and removes direct light only; it is gated by
  daytime and the established `wcam` fade. Above the high shell, the capped
  orbital composite alpha instead gives the ground a subtle dark footprint.
- **Fog banks / valley mist:** `Weather` now carries a deterministic humidity
  proxy built from seasonal climatological wetness plus live cover/precip.
  A pure local-solar sunrise function supplies a ±2-hour default window (and
  returns zero in polar day/night). The shared ground shader combines that
  with D-8's existing signed concavity byte and a height ceiling. The sky pass
  uses the same camera D-8 sample to draw the matching soft horizontal veil.
  Near-saturated cover plus precipitation independently creates broad fog.
- **Storm-edge lighting:** eight great-circle probes at 650 km around the
  camera are sampled on CPU and reduced to `base + dot(dir, gradient)`. The
  field darkens the corresponding sky horizon and distant terrain, with a
  subtle tunable warm/green cast. It fades with `wcam` and is not sampled in
  orbit.

The weather sampling budget remains bounded below 20 per frame:

- Ground: 1 camera + 8 compass probes = 9.
- Camera-weather/deck handoff: 1 camera + 10 symmetric deck samples + 8
  probes = 19.
- Orbit: 1 camera + the established 14-point camera-free deck mean = 15.

All new weather values remain pure functions of planet seed, position, and
weather/solar time. There is no accumulation, runtime RNG draw, or wall clock.

## Tuning surface

`viewer/assets/weather_tuning.json` now ships these validated W2 knobs:

| knob | default |
|---|---:|
| `cloud_shadow_strength` | 0.35 |
| `orbit_cloud_shadow_strength` | 0.10 |
| `fog_density` | 0.85 |
| `fog_ceiling_km` | 0.24 |
| `fog_dawn_window_h` | 2.0 |
| `storm_edge_probe_km` | 650.0 |
| `storm_edge_strength` | 0.32 |
| `storm_green_cast` | 0.08 |

All participate in whole-file validation. Non-finite values, strengths/cast
outside `[0,1]`, negative fog density, non-positive ceiling, dawn windows
outside `(0,12]`, or probe distances outside `(0,3000]` reject the entire
override and restore defaults.

## Visual evidence

The permanent source script is `viewer/scripts/w2-contact.play`; the composer
is `viewer/scripts/w2_contact_sheet.py`.

- [W2 contact sheet](viewer/interchange/w2-evidence/contact-sheet.png)
  (SHA-256 `2d91c084e2b828961d19c817090048acb8b6fa9f77819f4cb900602242a81420`)
  contains clear/broken/overcast at one pose, the two dawn-window edges plus
  sunrise, and a four-direction storm pan.
- The mist pose at `(13°, 133°)` was selected from the actual terrain law:
  D-8 concavity is `1.0`, canonical-time humidity is `1.0`, and local surface
  elevation is `0.400 km`. Mist is absent at the window edges and pools at
  sunrise without hiding terrain above the ceiling.
- The storm pan at `(15°, 35°, t=600)` deliberately has clear local weather
  (`cover=0`, `precip=0`) while the fitted southern storm lobe has gradient
  magnitude about `0.603` and base `0.375`. South reads darker while north
  remains bright; broad precipitation fog cannot confound this pose.
- A strength-zero shadow control confirmed that the broken-cloud projection
  changes only direct-lit pixels. Across the sampled open-ground footprint,
  the default response was spatially varying and at most 6 RGB levels—subtle
  at cover 0.65, as intended for the pinned reels.

`weather-visual.play` gained exactly three W2 shots:

| shot | SHA-256 |
|---|---|
| `w2_shadowed_midday.png` | `837b9f84b83e435c1cca7db2d91fa79bc778048df46a42f3e1133102c9741e1b` |
| `w2_dawn_valley_mist.png` | `50777a55251e086e8f78fd2b7d3bee3c8c6edb8fcfb63040a114ef29bfb325f1` |
| `w2_storm_edge.png` | `1f27a02f413c8aba81477264b32d2cbf25228aec9d48dd159e83f3b782ce9635` |

The complete 29-file determinism manifest is
[determinism-sha256.txt](viewer/interchange/w2-evidence/determinism-sha256.txt).

## Gates

### Tests and suites

- `cargo test --release --lib`: **82 passed, 0 failed, 1 pre-existing ignored**.
  New coverage validates W2 defaults/rejection, sunrise versus noon/sunset and
  polar no-sunrise behavior, and the first-order storm gradient direction.
- `cargo build --release --example play`: **PASS**.
- Direct PowerShell suite run: **all PASS** — `physics-regressions`,
  `water-regressions`, `lake-regressions`, `flooded-caves`,
  `invariant-survey`, `auto-survey`, `camera-controls`, `solar-system`.
- Extended `weather-visual.play`: two normal-settling independent runs,
  **29 PNG/JSON files each, 0 SHA-256 mismatches**.

The double-run investigation also found an instrument bug: a capture arriving
with a populated cache could draw ancestor tile stand-ins while background
children raced a rapid non-blocking drain. Capture settlement now blocks on
selected tile results just as it already did for chunks. Live rendering is
unchanged. Orbital cloud lookup also reconstructs the camera ray from fragment
coordinates (the established sky-pass method), independent of terrain
triangulation.

### Reels (not blessed)

Both generators explicitly pin weather time 0. World uses cover/precip
`0.10/0.0`; sky uses `0.0/0.0`.

- Sky reel: **all 5 poses clean**.
- World reel: exactly four expected low-amplitude cloud-shadow findings.
  Every one has `changed_frac = 0.0` (no pixel exceeds the reel's >6 threshold),
  unchanged material boundaries/silhouettes, and a negative signed RGB shift:

| pose | mean absolute delta | signed mean RGB | explanation |
|---|---:|---:|---|
| `river_wide` | 1.612 | -1.341 | projected cirrus/broken-shell shade on the sunlit stepped bank |
| `river_mouth` | 1.495 | -1.158 | subtle projected shell shade over dry delta ground; water/category edge unchanged |
| `coast_beach` | 1.625 | -1.320 | projected shade crosses beach relief; coastline and beach category unchanged |
| `desert` | 2.426 | -1.970 | strongest open-ground projected footprint; dune/material pattern unchanged |

  Accepted/current/amplified-delta evidence:
  [all four crop triptychs](viewer/interchange/w2-evidence/reel-crops/all-flagged.png)
  (SHA-256 `9e2e59786768b23604d840bd095c829d4a34c5eaa8491cb0568ac47df9efe1bb`),
  plus individual
  [river_wide](viewer/interchange/w2-evidence/reel-crops/river_wide.png),
  [river_mouth](viewer/interchange/w2-evidence/reel-crops/river_mouth.png),
  [coast_beach](viewer/interchange/w2-evidence/reel-crops/coast_beach.png), and
  [desert](viewer/interchange/w2-evidence/reel-crops/desert.png).
- Final sync-diff: `sea_calib = 0.0%`, mean/p95/max delta all zero. The shared
  fragment path therefore introduced no mesh-versus-voxel weather split.

### Performance

`fpsbench2.play`, 20 warm-up frames followed by the required 30 measured
frames at ground and orbit:

| pose | before avg / p95 | final avg / p95 | avg delta |
|---|---:|---:|---:|
| ground jungle | 41.21 / 81.89 ms | 36.35 / 48.24 ms | **-4.86 ms** |
| orbit | 20.91 / 24.30 ms | 18.89 / 21.02 ms | **-2.02 ms** |

The ground gate is `<= +1.5 ms`; observed delta is comfortably below it.
The negative values are normal run-to-run GPU variance, not a claimed
optimization.

## Worktree hygiene

- No commit and no `--accept`/bless action.
- Pre-existing untracked `run.err` and `run.log` were left untouched.
- No changes to W4 structural season code, biome/blend fields, terrain sample
  ownership, or moon code.
