# Clouds v2 + crevice rain findings

## Result

Andrew's D-6 and D-8 verdicts are implemented without a mutable weather
simulation or a structural terrain change.

- Clouds now use three seeded 2-D spherical shells: a 1.8 km dark storm base,
  3.8 km broken-cumulus middle, and 8.2 km fibrous-cirrus high layer. Distinct
  shell intersections give real parallax. Cover, precipitation, and air
  temperature continuously change formation weights, thresholds, and color.
- From orbit the same directional patterns are intersected and composited
  low-to-high over the rendered planet. Combined opacity is hard-capped after
  stacking at 0.55, so at least 45% of the lit ground contribution remains.
- Rain darkening reads a signed local trough/peak proxy from the spare alpha
  byte of the existing 72-byte terrain vertex. The proxy is
  `(coarse raster elevation - detailed/carved elevation) / 120 m`, clamped to
  `[-1,1]`. Default `rain_crevice_bias=0.18` gives at most +18% local rain in a
  deep trough and -18% on a local peak. It redistributes the regional
  precipitation value; it does not create a biome or structural feature.
- Nearby rain-particle count uses the same proxy through one resident
  elevation lookup. Snow and open ocean keep their regional particle rate.
- `WEATHER.md` records every layer count, altitude, scale, density, orbital
  cap, and crevice-bias knob plus whole-file validation rules.

## Formations contact sheet

All six frames use seed42_r8 at 6.974 N, 33.203 E. Ground is 0.15 km with
yaw 0 / pitch 12; orbit is 291.4 km with pitch -86. Pins are clear 0.08/0.00,
partial 0.45/0.00, and storm 0.97/0.90 (cover/precipitation).

![Clear, partial, and storm formations from ground and orbit](cloud-formations-contact-sheet.png)

Source frames: `viewer/interchange/runs/weather-visual-resume-a/`. The
extended `weather-visual.play` contains all six poses; clear and storm exercise
both the below-shell and orbital shader paths.

## Gates

| gate | result |
|---|---|
| release harness | PASS - `cargo build --release --example play --manifest-path viewer/Cargo.toml` |
| Rust tests | PASS - 42/42 unit tests (40 prior + 2 new), plus 1/1 noise golden integration test |
| seven suites | PASS - regenerated 69-probe auto-survey; physics-regressions, water-regressions, lake-regressions, flooded-caves, invariant-survey, auto-survey, and camera-controls all exit 0 |
| weather determinism | PASS - 23/23 PNG/JSON/state outputs byte-identical across fresh directories; manifest SHA-256 `92D4D4B707BCAE0D4EBEDC7CA09817C93F95A7BA992A543F599200750522F8AE` |
| sync-diff | PASS - `sea_calib` 0.0% divergent, mean/p95/max delta all 0 |
| world reel | REVIEWED - expected exit 3 for eight cloud-only changes; 24/24 void lint passes, all flags explained below, accepted reel unchanged |
| W3 visibility | PASS - orbital composite clamps post-stack alpha to the validated default 0.55; storm-orbit retains land, coast, and water detail |
| frame cost | PASS - two reversed-order 30-frame probes per build show no measurable regression; full orbit frame remains about 4 ms |
| diff hygiene | PASS - `git diff --check`; no dusting-dither edit and no commit |

`verify.sh` itself could not launch because this Windows host maps `bash` to
WSL and has no WSL distribution. Its exact operations were run directly:
the survey was regenerated and the same seven release-harness scripts were
executed in the same order with every exit code enforced.

The two deterministic run logs differ only in their last line because that
line prints the deliberately different output directory. Every rendered
frame, JSON sidecar, and state-only file is byte-identical.

## Frame cost

Baseline is an isolated build of branch `HEAD` (`5f6e661`); after is the
modified worktree. Each process uses `clouds-bench.play`: a warm-up capture
followed by 30 frames at the same storm pin, coordinates, headings,
resolution, assets, and release profile. A second pair reverses process order
to expose run variance rather than selecting the friendlier sample.

| pose | before run 1 avg/p95 | after run 1 avg/p95 | before run 2 avg/p95 | after run 2 avg/p95 | combined mean |
|---|---:|---:|---:|---:|---:|
| ground, 0.15 km | 16.64 / 26.27 ms | 18.10 / 34.87 ms | 20.69 / 46.59 ms | 17.73 / 33.34 ms | 18.67 -> 17.92 ms |
| orbit, 291.4 km | 3.50 / 5.47 ms | 4.23 / 6.65 ms | 4.50 / 7.41 ms | 3.71 / 5.17 ms | 4.00 -> 3.97 ms |

The order reversal changes the result more than the implementation does,
especially at the terrain-heavy ground pose. Across 60 measured frames per
build, ground changes by -0.75 ms and orbit by -0.03 ms. Each cloud layer uses
only two or three value-noise taps; there is no ray march or volume texture.

## Reel review

The world reel reported eight intended cloud-only changes. A weather pin makes
weather state and time deterministic; it does not make a replacement cloud
shader pixel-identical to the accepted W1 shader. Six ground flags are confined
to the upper image (no changed pixel below y=240), and their complete lower
halves are byte-identical. The two orbital flags are the direct W3 deliverable:
accepted W1 intentionally rendered no clouds from space.

| flagged pose | reel mean / changed fraction | review |
|---|---:|---|
| dossier_shore | 1.306 / 4.88% | new cirrus; lower half 0 pixels changed |
| dossier_band | 2.421 / 10.63% | new cirrus; lower half 0 pixels changed |
| river_wide | 1.266 / 5.58% | new cirrus; lower half 0 pixels changed |
| river_mouth | 0.675 / 3.66% | new cirrus; lower half 0 pixels changed |
| jungle | 1.048 / 5.13% | new cirrus; lower half 0 pixels changed |
| groves | 0.259 / 1.30% | new cirrus; lower half 0 pixels changed |
| orbital_coast | 10.027 / 24.09% | intended capped W3 cloud overlay |
| orbital_biomes | 7.040 / 20.86% | intended capped W3 cloud overlay |

All 24 reel poses passed void lint; the other 16 stayed below the change
threshold. No accepted reel was updated.

## Scope notes

- Cloud layout and motion are pure functions of planet seed, direction, and
  weather time. Runtime RNG, wall clock, accumulation, and mutable simulation
  state are absent.
- The renderer still samples one regional weather state at the camera, as the
  existing presentation contract specifies. Orbital formation shape varies
  directionally over the planet, while its clear/partial/storm type follows
  the requested existing uniform state rather than a new per-pixel climate
  lookup.
- This branch intentionally does not contain main's later deep-tile and
  dusting-hash fixes. The dusting dither line was not changed.
- No commit was created.
