# W-8 weather fidelity cluster — implementation findings

Mission: review #2 findings 2, 3, 4, 7, 10, and 13. Worktree
`sol/weather-fidelity` at starting HEAD `b3b2d8c`. No terrain sampling,
world, water, or physics rule was changed.

## Finding 3 — semiannual climatology / WEA2

Before, `weather.bin` was WEA1 with 10 layers and both precipitation and
clouds were reconstructed from mean + k=1 only. The review measured global
precipitation RMS 35.08 mm/month and cloud RMS 0.03739, with the two bimodal
canaries collapsing especially badly.

After, the bake writes WEA2 with 14 layers. Precipitation and clouds each add
the k=2 cosine/sine pair; both `weather_at` and the photo-map climatology path
sample them. The WEA1 loader remains a deliberate, loud compatibility path:
it maps the 10 old layers into WEA2's layout, zero-fills the four k=2 layers,
and prints a rebake warning. Unknown/corrupt headers still return a graceful
error. Unit tests assert both layouts and the legacy mapping.

The exact shipped-bake transcript was:

```text
temp k=1 fit: RMS 0.63 C against a mean seasonal swing of 9.35 C
precip harmonic fit: k=1 RMS 35.08032 -> k=1+2 RMS 24.84529 mm/month
cloud harmonic fit: k=1 RMS 0.03739 -> k=1+2 RMS 0.02319 fraction
bimodal cloud site 27.583315 117.155914: k=1 RMS 0.16586 -> k=1+2 RMS 0.02098 fraction; amplitudes annual=0.00622 semiannual=0.23267
bimodal precip site -12.472685 29.169937: k=1 RMS 360.02902 -> k=1+2 RMS 107.91597 mm/month; amplitudes annual=299.80551 semiannual=485.74692
```

Temperature deliberately stays k=1. Its 0.63 C residual is only 6.7% of the
9.35 C mean seasonal swing and it is not the promised/failed bimodal field;
keeping it annual avoids two more always-resident rasters and two more
bilinear reads in every weather and temperature-map sample.

Rebake command, from repository root:

```text
python scripts/bake_weather.py output/seed42_r8 256
```

The ignored binary asset `viewer/assets/weather.bin` is now exactly
22,020,108 bytes (`WEA2`, resolution 256, 14 layers), SHA-256
`930fa1ed8432d991acdf0dbb3d0ef2731065e25a8997cba1749926b757cdb575`.
Running the command twice produced that same hash. The old WEA1 asset was
15,728,652 bytes.

## Finding 4 — advection asymptote

Before, both noise scales used `normalize(dir - drift)`, so effective motion
was `atan(|drift|)` and approached 90 degrees. After, both synoptic and meso
directions use Rodrigues axis-angle rotation by the tangent arc length. The
small-angle direction/sign is unchanged, while long clocks wrap around the
sphere indefinitely. The shipped WEA2 wind sampled at the review coordinate
`51.851 4.809` is 5.842715 m/s east, 5.385360 m/s north.

The new effective column is an oriented, unwrapped rotation (ordinary `acos`
would fold rotations beyond 180 degrees):

| t_s | intended | old effective | new effective |
|---:|---:|---:|---:|
| 3,600 | 28.39° | 26.36° | 28.39° |
| 7,200 | 56.78° | 44.74° | 56.78° |
| 14,400 | 113.55° | 63.23° | 113.55° |
| 24,000 (1 year) | 189.25° | 73.16° | 189.25° |
| 28,800 | 227.10° | 75.84° | 227.10° |
| 48,000 (2 years) | 378.51° | 81.39° | 378.51° |

`great_circle_advection_tracks_unwrapped_arc_through_two_years` asserts these
rotations for both 1.0x synoptic and 1.6x meso arc lengths and asserts unit
length to 1e-12.

## Finding 2 — recorded weather replays

Before, the synthetic `t_s=24000`, pin `[0.97,0.9]` sidecar generated only
pose, sun, and shot lines, and neither capture nor photo teleport consumed the
weather object.

After, `repro_shot.py` emits state plus absolute time. Pin time is retained
because the pin fixes cover/precipitation but seasonal temperature, snow mix,
advection, and particle phase still depend on the weather clock:

```text
# live .play
weather live
weather time 24000

# pinned .play
weather pin 0.97 0.9
weather time 24000

# off .play
weather off
```

The corresponding live capture ends in `--weather live --weather-time 24000`;
pinned ends in `--weather 0.97,0.9 --weather-time 24000`; off ends in
`--weather off`.
An actual headless pair with weather times 0 and 24000 recorded those exact
`weather.t_s` values while both retained `day_cycle_time_s=0` and byte-equal
`sun_dir`. Thus the seek offset changes weather only, not day-cycle
determinism. Weather sampling, deck drift, and precipitation particle phase
all read this absolute weather clock.

The photo index now parses `weather.on`, `weather.pinned`, and `weather.t_s`.
`TeleportAction` carries them only when the existing **Restore photo's time of
day** checkbox is selected; apply restores off, pin/live, and absolute time.
The unit test `photo_weather_restore_is_opt_in_with_time_checkbox` asserts the
cut. Legacy/pre-weather sidecars are deliberately unrecoverable: the repro
tool warns and falls back to live weather at time zero, while photo restore
leaves current weather untouched.

## Finding 10 — Clouds-now honesty

Before, map synthesis always called live `weather_at`, ignored off/pin, and
its signature had no time, so an open map froze. After, `MapEnv` carries
`weather_on`, `weather_pin`, and absolute weather time. The cloud overlay is
empty (with an explicit hover explanation) when weather is off, uses the
pinned cover field when pinned, and calls live `weather_at` only in live mode.

`MapSig` now includes on/pin plus a 60-second weather-time bucket. Live clouds
therefore resynthesize at most once per simulated minute, not every frame;
pinned/off biome maps do not churn. Seasonal temperature/precipitation bases
also use the bucket. `clouds_now_bucket_tracks_only_moving_or_seasonal_fields`
asserts the 59.9/60.0 boundary and the pinned/off no-refresh cases.

## Finding 7 — tuning validation

Before, `{ "days_per_year": 0 }` loaded and made `season_frac` NaN. After,
`WeatherTuning::load` accepts an override only if every float is finite and
all denominator/order/range invariants hold: positive year/frequencies/
precip normalization/gamma/dusting, ordered cover and snow bands,
`precip_threshold` in `[0,1)`, valid `[0,1]` presentation strengths, ordered
cloud-shell heights, nonnegative storm speed/amplitude, and at most 100,000
particles. Any invalid or malformed file emits `weather tuning ignored ...;
using defaults` and rejects the entire override. `season_frac` itself also has
a finite default guard.

`invalid_tuning_override_falls_back_as_a_whole` asserts zero year, inverted
cover/snow, threshold 1, particle overflow, numeric overflow, and direct NaN.
`tuning_load_rejects_degenerate_file` exercises the public filesystem loader
with the zero-year JSON. All return finite defaults; no partial invalid
override reaches uniforms.

## Finding 13 — season command semantics

Before, after `wait 1200`, `weather season 0.25` set epoch to 0.25 and the next
shot rendered phase 0.30. After, the play command first aligns the renderer to
the fixed simulation ticks, then sets
`epoch_frac = (target - weather_t_s/(day_len*days_per_year)) mod 1`. At the
review repro it sets epoch 0.20 and the next sample is exactly 0.25.
`season_command_targets_next_shot_after_elapsed_time` asserts both values.
Non-finite season/pin/time command arguments now fail loudly instead of
feeding NaNs.

## Gates

- `cargo build --release --example play --manifest-path viewer/Cargo.toml`:
  PASS (explicit example build).
- `cargo test --manifest-path viewer/Cargo.toml`: PASS — 15 library tests,
  the Python-noise golden, binary/doc tests all green.
- `bash viewer/scripts/verify.sh`: wrapper cannot start because this Windows
  host has no WSL distribution. Its commands were run individually: release
  build PASS; `gen_survey.py --per 16` regenerated 69 probes; all seven suites
  PASS (`physics-regressions`, `water-regressions`, `lake-regressions`,
  `flooded-caves`, `invariant-survey`, `auto-survey`, `camera-controls`).
- WEA2 determinism: `weather-visual.play` ran twice into separate directories;
  all 8 PNG hashes and all 9 JSON hashes matched pairwise.
- Standing meter: all 14 poses completed, with no terrain/render-rule change;
  `sea_calib` remains exactly 0.0% divergent / mean 0 / p95 0. Current report:
  river_low 12.50, river_mid 11.60, lake_shore 14.77, savanna 7.35, beach
  7.86, jungle 19.19, taiga 7.62, desert 6.77, peak 12.58, valley 12.05,
  groves 11.52, ice_top 8.76, karst 12.16 (mean deltas).
- `git diff --check`: PASS. Existing untracked `run.err` and `run.log` were
  left untouched. The binary asset and this findings file are intentionally
  ignored handoff artifacts, so their exact path/size/hash/command are listed
  above rather than represented as textual diff bytes.
