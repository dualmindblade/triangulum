# Solar P2 findings: the moon becomes a real place

Status: complete on `sol/solar-p2`, based at `732360e`. No commit was made and
no reel baseline was blessed.

`SOLAR.md` and `DECISIONS.md` were read first. The referenced interchange spec
was absent from this branch snapshot, so I read the matching
`viewer/interchange/solar-system-and-moon.txt` from the adjacent main checkout;
its requirements and Andrew's bracket verdicts agree with the repo-root design
record.

## Result

The P1 placeholder Moon has been replaced by one deterministic surface law in
[`viewer/src/moon.rs`](viewer/src/moon.rs). The same `MoonGenerator::sample`
result supplies mesh relief, vertex albedo/material channels, fixed probes, and
the teleport map. It is a pure function of `(unit direction, seed)`: the
generator only pre-expands immutable seed records and carries no clock, RNG, or
simulation state.

The visible face is intentionally albedo-led. Broad highlands are smoother than
Neisor; dark, flattened maria interrupt them; fresh rims and broken ray systems
carry the bright structure. Rays have only a tiny relief echo so they do not
turn into radial mountain walls in profile.

### Documented seed-42 tuning

| Feature | Count / probability | Angular radius | Configured physical radius |
|---|---:|---:|---:|
| Large craters | 18 | 1.8-7.5 deg | about 73-306 km |
| Medium craters | 72 | 0.35-1.8 deg | about 14-73 km |
| Small craters | 288 | 0.06-0.35 deg | about 2.4-14 km |
| Maria | 7 | 8-19 deg | about 326-775 km |
| Elongated strikes | 2.5% lottery | same tier ranges | deliberately rare |
| Ray eligibility | youngest 28%, then size/lottery gates | out to 7.5 crater radii | albedo first |

There are 378 craters in one chronological stream. Tier counts are exact but
interleaved. The reduction runs oldest to newest: a younger interior blends to
its pre-impact macro datum and overrides an older floor; its ridged, azimuthally
disturbed rim and ejecta compose over surviving terrain. Large craters add a
broad central uplift with a small summit dimple. Rare shallow-angle impacts use
an elliptical metric; ordinary rims still receive low-amplitude deterministic
lobing so perfect circles do not dominate.

The configured Moon radius is `2338.268590 km` (`0.27 * Neisor`). Returned
relief is a radius ratio, clamped to `[-0.012, 0.009]`; albedo is clamped to
`[0.16, 0.96]`.

## Rendering and LOD choice

I reused the generic cube-sphere `TileKey` / `select_tiles` machinery because
its f64 geometry, 32x32 tile lattice, level morph targets, skirts, and horizon
culling generalize directly. The Moon has its own generator, render pipeline,
and 384 MiB GPU cache, so it cannot perturb Neisor's terrain cache or material
path.

- Screen-space error target: `0.18`.
- Existing level cap: 14.
- Finest possible tile width: about 224 m.
- Finest possible vertex spacing: about 7.01 m.
- Tile positions/normals are generated in f64 Moon-local kilometres. The body
  center and tile origin stay f64 and camera-relative until the f32 uniform and
  vertex upload boundary.
- Near-plane and freecam speed now use the nearest physical body's surface, so
  an 8 km Moon flyby does not inherit Neisor-orbit clipping or 1500 km/s travel.

This is still mesh-only. There are no Moon voxels, landing physics, water, or
weather in P2.

## Deferred material seams

The one-truth module names the future post-sample overlays without inventing
data for them: `polar_ice_deposition`, `permanent_crater_shadow_ice`,
`underground_ice`, and `resource_reservoirs`. They remain P3/P4 work.

## Teleport/photo map

The photo map now has a `Neisor | moon` body selector. The Moon branch makes an
equirectangular raster from exact generator albedo and a finite-difference
relief shade, capped at 640x320. Map clicks create body-local Moon destinations;
in focused-Moon/freecam use they retain local orientation, and a bare click uses
a safe 25 km mesh-phase altitude rather than pretending landing exists. Photo
sidecars carry body-local position/view fields, and markers/lists are filtered
to the selected body.

The default remains Neisor. Its `synth_map` implementation and generation,
biome, water, river, and weather inputs were not edited; Moon selection branches
around that existing path. The zero-delta world reel and sea calibration below
are the render-level checks on that claim.

## Verification

### Build and automated tests

- `cargo test --release --lib`: **53 passed**, including
  `crater_fold_is_deterministic`, `albedo_stays_physical_and_has_lunar_range`,
  and `moon_map_is_deterministic_and_uses_generated_range`.
- `cargo build --release --example play`: passed.
- `cargo build --release`: passed for the app/UI target.
- Python compile lint passed for `world_reel.py`, `sky_reel.py`,
  `moon_contact_sheet.py`, and `sky_moon_crops.py`.
- `git diff --check`: clean.

Windows denied exclusive access to the pre-existing `viewer/target` lock even
with no Cargo/rustc process alive. The final release validation therefore used
the ignored repo-root `target/` through `CARGO_TARGET_DIR`; the source and build
profile were otherwise the requested ones.

All asserting play suites passed from that final binary:

- `physics-regressions.play`
- `water-regressions.play`
- `lake-regressions.play`
- `flooded-caves.play`
- `invariant-survey.play`
- `auto-survey.play`
- `camera-controls.play`
- `solar-system.play`

`solar-system.play` now pins the Moon law at two body-local directions:

| Latitude / longitude | Surface height |
|---|---:|
| `0, 0` | `0.134866 km` |
| `37, -122` | `0.032202 km` |

### Neisor isolation

`python scripts/sync_diff.py` retained `sea_calib` at **0.0% divergent,
mean 0.0, p95 0, luminance +0.0**. No content diff exists in `planet.rs`,
`terrain.rs`, `voxel.rs`, `rivers.rs`, `noise.rs`, or `weather.rs`.

The final 26-pose world reel rendered and linted clean:

- All 24 existing Neisor poses: maximum mean delta `0.000`, maximum changed
  fraction `0.0000`.
- `moon_orbit_face`: no baseline, surface fraction `0.5280`, surface standard
  deviation `11.722`.
- `moon_low_flyby`: no baseline, surface fraction `0.7006`, surface standard
  deviation `8.835`.

### Sky reel and expected face change

The final frames are byte-identical to the stored current sky reel. Everything
except the intentional Moon face stays below threshold:

| Pose | Whole-frame mean delta | Changed fraction |
|---|---:|---:|
| `sun_zenith` | 0.031 | 0.0007 |
| `sunset_west` | 0.477 | 0.0006 |
| `dusk_gradient` | 0.711 | 0.0000 |
| `moon_zenith` | 0.111 | 0.0037 |
| `night_stars` | 0.000 | 0.0000 |

The Moon-specific ROI correctly reports **CHANGED**: mean `11.140`, fraction
`0.3720`. The before frame was P1's smooth procedural placeholder; the after
frame exposes P2 maria/crater albedo. This fixed pinned pose also has
`lunar_shadow = 1.0`, so the new face is copper-tinted by the already-approved
D-16 lunar-eclipse treatment. That is geometry, not a new color shortcut.

Evidence: [before/after Moon crop](viewer/interchange/skyreel/moon_zenith_before_after.png),
[before crop](viewer/interchange/skyreel/moon_zenith_before_crop.png), and
[after crop](viewer/interchange/skyreel/moon_zenith_after_crop.png).

### Fresh-process determinism

Two independent release processes rendered a complete face and the detailed
crater field. Each pair was byte-identical:

| Frame | SHA-256 in both processes |
|---|---|
| `moon_face.png` | `C4578DB577851400BF85832B8323F7C071B0427553D9554E437D57F0F92B3F34` |
| `crater_field.png` | `796B1CB08C97CCBEA2D6DA0B03767F10CF5BB0E7F746A50316FDB422466ABCB1` |

### Frame-budget story

The benchmark intentionally records five cold/cache-filling frames and then 30
steady frames:

| Pose | Cold 5 avg / max | Steady 30 avg / p95 / max |
|---|---:|---:|
| Neisor river ground control | 542.99 / 2402.05 ms | 17.53 / 34.71 / 35.51 ms |
| Near-Moon flyby, 70 km | 226.28 / 1085.15 ms | 11.66 / 13.13 / 16.77 ms |

The detailed Moon is inside a 60 Hz frame for average and p95 after streaming;
one measured maximum is 0.10 ms over 16.67 ms. The visible cold spike is the
cost of synchronously populating a new body cache, matching the P2 choice to
reuse the simple tile path. It is amortized after arrival and is the clear seam
for asynchronous P3 body streaming. The Neisor control remains unchanged and
shows its existing heavier cold and ground-stream costs.

## Visual contact sheet

The final LOD frames cover the four requested views: full face at Neisor
distance, quarter phase, a large ray crater flyby, and a low maria horizon.

[Open the final contact sheet](evidence/solar-p2/contact_sheet.png).

Source/reproduction files are
[`moon-contact.play`](viewer/scripts/moon-contact.play),
[`moon_contact_sheet.py`](viewer/scripts/moon_contact_sheet.py),
[`moon-determinism.play`](viewer/scripts/moon-determinism.play), and
[`moon-bench.play`](viewer/scripts/moon-bench.play). No `--accept` or blessing
operation was run.
