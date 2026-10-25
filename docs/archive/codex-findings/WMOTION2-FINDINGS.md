# W-MOTION pass 2 — cyclones, shear, and fronts

- Date: 2026-07-13
- Branch/base: `sol/cyclones` at `8449a0d`
- World: `output/seed42_r8`
- Status: **PASS — all requested correctness, determinism, visual, sea-sync, and performance gates are green.**
- Commit: none created.

## Result

W-MOTION pass 2 is live as a bounded, stateless structured field shared by the CPU weather model and the GPU cloud fabric:

- Two seeded analytic cyclones ship by default, with a fixed shader/runtime ceiling of four.
- Each center follows a closed-form zonal track and is selected/modulated by the baked cloud/precipitation climatology.
- The cloud domain is inverse-mapped into each nearby vortex's co-rotating frame, producing plainly visible orbital rotation over 60–120 seconds at 1x.
- Latitude-dependent differential rotation slides zonal cloud bands past one another.
- Each cyclone carries a finite asymmetric front: a narrow leading ridge and broader trailing smear.
- Cyclone/eyewall/front loads alter local cover, precipitation, humidity, storm intensity, deck alpha, and cloud color. The eye clears the deck. These are storms expressed by the weather field, not decals.
- W2 ground shadows sample the exact same three physical cloud shells and structured fabric as the visible deck. W2 storm-edge gloom uses the same resolved CPU cyclone bank through the existing compass probes.

No W4, biome, terrain, voxel, or moon source remains modified. The pre-existing untracked `run.err` and `run.log` were left untouched.

## Analytic model

For system `i`, eight seed-addressed track candidates are scored against annual baked cloud and precipitation fields. The best storm corridor supplies a fixed latitude and initial longitude. At any absolute weather time:

```text
lambda_i(t) = lambda_i0 + track_speed * t / (planet_radius * cos(latitude_i))
center_i(t) = (cos(lat) cos(lambda), cos(lat) sin(lambda), sin(lat))
```

The current seasonal bake is sampled again at the moving center. Its cloud/wetness score continuously modulates system intensity, so a track crossing a climatologically quiet region fades instead of carrying an unconditional decal around the planet.

For physical cloud sample direction `p`, normalized chord distance is `d² = 2(1 - dot(p, center_i))`. Within three radii, the inverse co-rotation is a Rodrigues rotation about the moving center:

```text
falloff = intensity_i * exp(-d² / radius²)
angle   = -hemisphere_sign * cyclone_spin_rate * t * falloff
domain  = Rodrigues(domain, center_i(t), angle)
```

The resulting domain is then rotated about the planet spin axis by the requested differential law:

```text
theta(latitude, t) = (w0 + w1 * cos²(latitude)) * t
```

The storm shape uses a negative eye, positive eyewall, and Gaussian outer envelope. Its attached front is evaluated in the moving center's tangent frame. Positive cross-front distance uses the narrow leading width; negative distance uses the wider trailing width; a separate along-front taper makes the ridge finite. The same load is injected into CPU and GPU cover/precipitation paths.

## Determinism and anchoring

- Every track, bearing, center, domain rotation, and front is a pure function of seed, direction, baked weather, tuning, season, and absolute weather time. There is no storm lifetime state or RNG draw at runtime.
- CPU center/phase work is performed from absolute `f64` weather time. The hourly particle presentation clock never feeds structured motion.
- The unit gate checks centers bit-for-bit at `t=0` and `t=1234.5`, verifies movement between those instants, and checks center/domain/front continuity across `3599.999 -> 3600.001` seconds.
- All visible shape inputs are planet directions, seeded baked fields, and weather time. Camera position/direction only determines the physical ray/shell intersection; it never enters a shape law.
- Existing stabilized `weather8` deck inputs remain the base cover/precipitation/temperature source. Structured loads are added locally after that base; no camera-fed scalar was introduced.
- The cyclone bank is resolved once per frame and reused by the camera sample, 14 deck/edge probes, ground storm-edge probes, GPU uniforms, and map raster. This keeps CPU/GPU ownership coherent and avoids per-probe candidate rescoring.

## Tuning and validation

The shipped overrides are in [`viewer/assets/weather_tuning.json`](viewer/assets/weather_tuning.json). Invalid files still fall back as one unit.

| Knob | Default | Validation |
|---|---:|---:|
| `cyclone_count` | 2 | `0..=4` |
| `cyclone_radius_km` | 950 | `(0, 3000]` |
| `cyclone_spin_deg_s` | 0.75 | magnitude `<= 5` |
| `cyclone_track_km_s` | 0.35 | `[0, 5]` |
| `cyclone_cover_boost` | 0.48 | `[0, 1.5]` |
| `cyclone_precip_boost` | 0.68 | `[0, 1.5]` |
| `differential_rotation_deg_h` | 18 | magnitude `<= 360` |
| `differential_rotation_shear_deg_h` | 24 | magnitude `<= 360` |
| `front_strength` | 0.38 | `[0, 1.5]` |
| `front_leading_km` | 90 | ordered scale rule below |
| `front_trailing_km` | 360 | ordered scale rule below |
| `front_length_km` | 1800 | `0 < leading < trailing < length <= 6000 km` |

Validation tests cover every new bound, non-finite values through the existing finite gate, count capping, and the front-scale ordering invariant.

## Visual evidence

The aggregate contact sheet is [`time-strips.png`](viewer/interchange/wmotion2-evidence/time-strips.png). Individual full-resolution strips:

| Evidence | Times | Finding |
|---|---|---|
| [Cyclone from orbit](viewer/interchange/wmotion2-evidence/cyclone-orbit.png) | `0, 60, 120, 180 s` | The eye, eyewall, and spiral fabric rotate plainly between every adjacent frame; direction is unambiguous by two minutes. |
| [Rotating ground shadow](viewer/interchange/wmotion2-evidence/cyclone-shadow.png) | `86400, +60, +120, +180 s` | Fixed ground/camera/sun pose; cloud-shadow bands move and rotate with the overhead cyclone. Four-frame crop temporal range: mean `11.646/255`, p95 `36`, max `206`. |
| [Ground front passage](viewer/interchange/wmotion2-evidence/front-passage.png) | `600, 1200, 1800, 2400 s` | A narrow sharp leading ridge arrives, followed by the broader gray trailing deck, then moves on with its parent. |
| [Cyclone approach / W2 gloom](viewer/interchange/wmotion2-evidence/cyclone-gloom.png) | `0, 1200, 2400, 3600 s` | Directional storm-edge darkening and precipitation build as the cyclone approaches; the eye then clears overhead. |

Raw frames and JSON sidecars are under `viewer/interchange/runs/weather-visual/`. The extended script is [`viewer/scripts/weather-visual.play`](viewer/scripts/weather-visual.play); [`viewer/scripts/wmotion2_contact_sheet.py`](viewer/scripts/wmotion2_contact_sheet.py) builds the evidence sheets.

The complete 30-frame weather-visual suite was run twice from the same release binary. SHA-256 comparison found **30/30 byte-identical PNGs, 0 mismatches**, including the new cyclone-overhead orbital pose and front-passage ground pose.

## Reel audit — deliberately not blessed

`world_reel.py` was run without `--accept` and intentionally exited with 12 findings. The accepted reel was not changed. Each linked crop is `accepted | W-MOTION 2 | |delta| x24`; the aggregate is [`all-flagged.png`](viewer/interchange/wmotion2-evidence/reel-crops/all-flagged.png), and machine-readable measurements are in [`report.json`](viewer/interchange/wmotion2-evidence/reel-crops/report.json).

At pinned `t=0`, cyclone and front presence are already nonzero even though their rotation angles are zero. In addition, the shader-cost budget replaces pass 1's third independent warp-noise tap with a deterministic combination of the first two. That changes the pinned cloud fabric. Ground deltas are the corresponding same-fabric W2 light/shadow projection; orbital deltas show the fabric directly.

| Pose | Mean delta / changed fraction | Crop and explanation |
|---|---:|---|
| `dossier_shore` | `0.958 / 0.0488` | [Crop](viewer/interchange/wmotion2-evidence/reel-crops/dossier_shore.png): overhead cloud lobes relocate; shoreline and blocks are identical. |
| `dossier_band` | `1.542 / 0.0799` | [Crop](viewer/interchange/wmotion2-evidence/reel-crops/dossier_band.png): the horizon deck is reshaped by the funded warp/front loads; horizon geometry is unchanged. |
| `river_wide` | `2.595 / 0.0486` | [Crop](viewer/interchange/wmotion2-evidence/reel-crops/river_wide.png): sky fabric moves and its W2 illumination reaches the river/ground; channel edges do not move. |
| `river_mouth` | `3.425 / 0.1007` | [Crop](viewer/interchange/wmotion2-evidence/reel-crops/river_mouth.png): broad sky-deck replacement plus same-fabric ground light; river-mouth geometry is fixed. |
| `lake_archi` | `1.731 / 0.0000` | [Crop](viewer/interchange/wmotion2-evidence/reel-crops/lake_archi.png): low-amplitude, sub-threshold cloud/W2 tone across the view; the water boundary is identical. |
| `coast_beach` | `1.489 / 0.0050` | [Crop](viewer/interchange/wmotion2-evidence/reel-crops/coast_beach.png): a thin horizon-cloud change and faint W2 shadow tint; beach contours remain fixed. |
| `desert` | `1.817 / 0.0073` | [Crop](viewer/interchange/wmotion2-evidence/reel-crops/desert.png): thin horizon deck plus faint projected lighting; terrain texture and silhouette are unchanged. |
| `jungle` | `2.236 / 0.0955` | [Crop](viewer/interchange/wmotion2-evidence/reel-crops/jungle.png): obvious sky-cloud relocation and matching illumination; trees, lake, and coast retain their shapes. |
| `taiga` | `1.341 / 0.0000` | [Crop](viewer/interchange/wmotion2-evidence/reel-crops/taiga.png): sub-threshold full-field cloud/W2 tone; the land/water boundary is identical. |
| `groves` | `0.793 / 0.0219` | [Crop](viewer/interchange/wmotion2-evidence/reel-crops/groves.png): small sky-cloud and ground-shadow shift; grove and shoreline silhouettes are fixed. |
| `orbital_coast` | `11.721 / 0.3100` | [Crop](viewer/interchange/wmotion2-evidence/reel-crops/orbital_coast.png): direct changed cloud fabric and structured storm cover; coast/biome pixels underneath keep their boundaries. |
| `orbital_biomes` | `6.160 / 0.2225` | [Crop](viewer/interchange/wmotion2-evidence/reel-crops/orbital_biomes.png): direct cyclone/front cover plus the two-tap warp; surface and ice boundaries remain fixed. |

Reel lint found no new void defect (`0` or the existing isolated `1 px` reports only). Both moon poses are exactly `0.0` mean delta and `0.0` changed fraction. Unaffected terrain/water poses remain byte-identical or below the reel finding threshold. No baseline was blessed.

## Performance

The exact base `8449a0d` was compiled from a clean archive in a temporary directory and run with the same [`viewer/scripts/cyclones-bench.play`](viewer/scripts/cyclones-bench.play), release settings, world assets, resolution, warm-up, and 30-frame measurement windows. Five-run medians:

| Pose | Base runs (ms) | Base median | Pass 2 runs (ms) | Pass 2 median | Delta |
|---|---|---:|---|---:|---:|
| Ground, established jungle pose | `32.38, 30.57, 32.35, 31.67, 30.70` | `31.67 ms` | `30.22, 29.78, 31.45, 29.98, 32.15` | `30.22 ms` | **`-1.45 ms`** |
| Orbit, cyclone overhead | `11.16, 10.76, 11.50, 11.24, 10.57` | `11.16 ms` | `10.72, 10.78, 10.69, 10.64, 11.51` | `10.72 ms` | **`-0.44 ms`** |

Both requested `<= +1.0 ms` budgets pass.

Math budget per structured shell sample:

- one fixed two-dimensional spin-axis rotation for differential rotation;
- per active nearby cyclone, one Gaussian envelope and one Rodrigues rotation;
- compact radial eye/eyewall work plus two oriented dot distances for a nearby front;
- intensity and distance rejection before the expensive path; default loop count 2, hard maximum 4;
- one pass-1 3-D value-noise tap retired per shell sample. That saves eight lattice-corner hashes; its radial warp lane is now a deterministic linear combination of the remaining two independent taps.

The retired noise tap funds the exact planet-direction structured evaluation used by both visible clouds and W2 shadows; measured medians are net faster rather than merely within tolerance.

## Gate ledger

| Gate | Result |
|---|---|
| `cargo test --release --lib` from `viewer/` | **PASS:** 85 passed, 0 failed, 1 pre-existing ignored; includes tuning rejection and two-time/hour-boundary cyclone determinism tests. |
| `cargo build --release --example play` from `viewer/` | **PASS.** Only the three pre-existing deprecation/unused warnings. |
| Direct asserting suites | **PASS:** `physics-regressions`, `water-regressions`, `lake-regressions`, `flooded-caves`, `invariant-survey`, `auto-survey`, `camera-controls`, `solar-system`; 69 probes total. Survey regenerated with `gen_survey.py --per 16`. |
| Extended weather visual | **PASS:** 30 frames, double run, 0 SHA-256 mismatches. |
| Sea synchronization | **PASS:** `sync_diff.py --label wmotion2-final` reports `sea_calib 0.0%`, mean `0`, p95 `0`, max `0`, signed `+0.0`. |
| Reel | **AUDITED / NOT BLESSED:** 12 explained findings with crops; accepted output untouched. |
| Ground performance | **PASS:** `-1.45 ms` median versus exact base. |
| Orbit performance | **PASS:** `-0.44 ms` median versus exact base. |
| Worktree scope | **PASS:** no W4/biome/moon/terrain/voxel source diff; no commit. |

## Files

Core implementation:

- `viewer/src/weather.rs`
- `viewer/src/renderer.rs`
- `viewer/src/shader.wgsl`
- `viewer/src/ui.rs`
- `viewer/assets/weather_tuning.json`

Validation/evidence helpers:

- `viewer/examples/weatherprobe.rs`
- `viewer/scripts/weather-visual.play`
- `viewer/scripts/cyclones-bench.play`
- `viewer/scripts/wmotion2_contact_sheet.py`
- `viewer/scripts/wmotion2_reel_crops.py`
