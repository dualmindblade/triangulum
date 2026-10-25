# Solar system P1 findings

Date: 2026-07-12  
Branch: `sol/solar-p1` from `79ffba3`  
World: `output/seed42_r8` / Neisor radius 8660.254037844386 km  
Commit: none

## Result

P1 is implemented and the complete headless gauntlet is green. Sun, Neisor,
and moon now have a validated, hierarchical f64 Kepler frame. The Sun and moon
are physical sphere meshes; moon phase, solar overlap, and lunar shadow are
geometric. Ground lighting preserves the accepted presentation exactly, while
a whole-globe orbital pose shows the terminator crossing Neisor.

The existing weather replay coordinate remains `weather time T_S` and the
photo sidecar still restores weather off/live/pin and absolute `t_s`. That
absolute coordinate is now also the orbital/season coordinate. Exact photo
time-of-day restore is retained as a daily rotation offset applied after the
absolute seek; it does not create another year/season clock. Sidecars retain
their old fields and add roll plus a `solar` evidence object.

No water rule, `WATER-CONTRACT.md`, `planet.rs`, or `terrain.rs` was changed.

## Orbital frame and clock

`viewer/src/orbits.rs` owns the hierarchy and all eclipse geometry. Each
elliptic position is evaluated directly from absolute time with eight bounded
Newton steps for Kepler's equation: O(1), no integration, no accumulated
state. Hierarchy is validated as `Sun <- Neisor <- moon`. Inertial and
Neisor-fixed positions stay f64; `renderer.rs` subtracts the f64 camera before
the final f32 upload. The shader only sees camera-relative body centers.

`viewer/assets/solar_tuning.json` is a weather-style validated tuning file.
Invalid files fall back as one loud unit, and a valid file may override only
one knob. Shipped clock:

- day: 1800 s / 30 min
- moon orbit: 7 game days
- Neisor orbit/year: 84 game days (12 lunar periods)
- axial tilt: 23 degrees
- starting season: Neisor mean anomaly 0.45, preserving the established
  weather epoch; the t=0 geometric solar declination is about +10 degrees

There is no weather-owned `days_per_year` or `epoch_frac` now. Weather Fourier
sampling, map seasons, Sun declination, and body positions all consume
Neisor's orbital mean-anomaly phase at the same `T_S`. Declination itself is
the z component of the tilted physical Sun-Neisor vector, so its +/-23 degree
range cannot drift from orbital geometry.

`solar-system.play` numerically locks all three body coordinates at t=0 and
t=1,234,567 s. It also produces a dedicated whole-globe terminator frame at
`interchange/runs/final2-solar-system/orbital_terminator.png`.

## Physical scales and appearance

The Sun keeps a near-real radius ratio but uses a presentation-compressed
distance: radius 109 R-Neisor (943,967.69 km), semi-major axis 12,190 R-Neisor
(105,568,496.72 km). From Neisor's surface its physical 0.512 degree angular
radius replaces the old saturated shader core; the existing halo function is
centered on the mesh and remains tunable.

The moon radius is 0.27 R-Neisor (2338.27 km). Its semi-major axis is 13.86
R-Neisor (120,031.12 km), leaving about 111,370.87 km from the sub-lunar
surface at mean distance. This is deliberately much more compressed than an
Earth-like orbit: it yields a 1.203 degree angular radius, matching the old
sky shader's 0.021 rad disc. Preserving the approved sky size was chosen over
astronomical distance scaling, and the distance remains a tuning knob.

The physical moon shader has a clean P2 hook, `moon_surface(normal)`, currently
limited to three very-large-scale deterministic noise octaves. The t=0 phase
is 99.8% illuminated, not forced full. Its center is about 1.9 degrees from
the legacy procedural pose so it avoids starting in Neisor's shadow; it stays
an immediately recognizable disc. This is the accepted moon delta:

| sky pose | mean max-channel delta | fraction > 6 | result |
|---|---:|---:|---|
| sun_zenith | 0.0306 | 0.000694 | clean |
| sunset_west | 0.4772 | 0.000563 | clean |
| dusk_gradient | 0.7108 | 0.000000 | clean |
| moon_zenith | 0.1098 | 0.003720 | clean; physical limb/mottle/1.9 deg pose shift |
| night_stars | 0.0000 | 0.000000 | byte-identical |

All five are below both sky-reel thresholds (mean <= 1.0 and changed fraction
<= 0.01). No pose is over threshold, so there are no flagged before/after
crops to explain and no baseline was blessed. A 1.15 ground moonlight gain
compensates for the physical default direction while geometric phase now
controls its waxing/waning contribution.

## Planet-shaded lighting

The broad terrain day/ambient factor transitions from the exact legacy camera
factor below 2.5 km to the pixel's radial Neisor position by 40 km. Thus every
ground reel frame converges exactly to the accepted look and orbital pixels
span their own day/night state. Direct Lambert light remains per normal. Sky,
stars, and atmospheric day remain camera-based.

`world_reel.py`: all 24 poses clean, including both existing 291 km orbital
poses. Those close orbital poses do not frame the terminator and therefore do
not flag. The new 20,000 km evidence pose does frame the whole disc and clearly
shows the terminator; this is an intentional new pose, not a reel baseline
change.

The expensive per-pixel eclipse overlap is guarded by a conservative CPU
whole-planet contact bound. Ordinary ground frames take the old byte-cheap
lighting path; orbital frames and real contact windows pay for body/pixel
geometry. The bound can admit extra work but cannot reject a surface eclipse.

## Eclipses

Solar coverage uses the exact two-circle intersection area divided by solar
disc area. The CPU and WGSL use the same equation. Terrain evaluates it at the
pixel, so the penumbra moves across Neisor; sky darkness uses coverage at the
camera. Coverage 1 removes direct/day ambient and reveals stars, producing a
night-like totality. The physical foreground moon depth-occludes the emissive
Sun mesh.

Lunar shadow is the fraction of the solar disc hidden by Neisor as seen from
the moon. That continuous factor darkens/copper-interpolates the physical moon
through partial contact and reaches the shipped copper color in the umbra.

Default moon eclipse knobs are `eccentricity=0.08`, `inclination_deg=4.8`,
`longitude_ascending_node_deg=225.0126009`, and
`argument_periapsis_deg=-122.6728680`. Inclination is the main rate control;
node/periapsis set the clustered seasons, eccentricity varies disc/shadow size,
and mean anomaly sets the epoch pose. Semi-major axis changes both angular
disc size and Neisor-shadow width.

A 10-second scan of one 84-day tuning year (locked by a unit test) finds:

- 2 solar center contacts: about game day 4.23 (total) and day 49.79
  (roughly 29% at Neisor center). The wrap/forward gaps are about 38.4 and
  45.6 days, not a periodic every-N-moons schedule.
- 9 lunar-shadow contacts. They cluster at roughly days 0.03, 7.66, 15.41,
  38.56, 46.18, 53.79, 61.40, 68.96, and 76.49; the 23-day gap skips several
  opportunities, and late contacts vary from about 12% to full umbra.

That is 11 contacts per short game year, much more frequent per game day than
an Earth-like calendar, while the nodal gaps and eccentric contact strengths
retain the requested irregular charm.

Reproducible evidence is in `viewer/scripts/eclipse-evidence.play` and the
generated contact sheet is
`viewer/interchange/solar-p1/eclipse-contact-sheet.png`:

| evidence | weather | t_s | camera lat/lon | numeric factor |
|---|---|---:|---|---:|
| partial solar | off | 7621.82 | 28.419789, -28.563336 | occlusion 0.500011 |
| total / night-like ground | off | 7621.82 | 15.489789, -28.563336 | occlusion 1.000000 |
| lunar copper | off | 13785.65 | -17.597195, 9.929233 | lunar shadow 1.000000 |

## Camera and controls

The shared `CameraRig` state machine is covered in both the app and play
harness:

- focused Neisor: default; existing walk/fly/player physics is unchanged
- focused moon / Sun: 3-radius standoff, look-at alignment, and exact f64
  translation with the body's O(1) position on every time jump
- freecam: W/A/S/D local forward/strafe, Space/Ctrl local up/down, mouse
  yaw/pitch, and Q/E roll; translation preserves world orientation
- app key `C`: Neisor -> moon -> Sun -> freecam -> Neisor
- play command: `focus neisor|moon|sun|free`; numeric `focus_id` is
  0/1/2/-1
- G/F return to Neisor walk/fly. Walk mode code was not altered.

The extended `camera-controls.play` asserts all three freecam translation
axes, 68.7549 degrees of one-second Q/E roll, focus switching, alignment 1.0,
moon standoff 7014.8058 km, Sun standoff 2,831,903.07 km, and both 120,000 s
and 240,000 s timeskip realignments.

Block controls are now LMB break / RMB place. The first LMB captures the
mouse; subsequent captured LMB presses break. Q/E do nothing outside freecam.
The three existing edit-driven play suites were migrated from `tap q/e` to
numeric-equivalent `tap lmb/rmb` and remain green. R still toggles torches.

Photo sidecars now record/restore roll in addition to the existing pose.

## Gates

| gate | final result |
|---|---|
| `cargo test --release --lib` | 50/50 pass (42 original + 8 net new) |
| release play build | pass |
| release app build | pass |
| play suites | 8/8 pass: physics, water, lake, flooded-caves, invariant, auto, camera-controls, solar-system |
| `sync_diff.py` | pass; `sea_calib` 0.0%, mean 0.0, p95 0 |
| `world_reel.py` | all 24 clean; no accept |
| `sky_reel.py` | all 5 clean; no accept |
| timeskip determinism | byte-identical SHA-256 `55CE2D0665E1030BB2807BDC05C15706F97529CE8AFC693884F217FF0F6076D4` |
| eclipse evidence | all numeric assertions pass; contact sheet generated |

Determinism used two fresh play processes at totality t=7621.82 and compared
`near_total_ground.png` byte-for-byte.

### Frame cost (`bench 30`, five warm-up frames excluded)

| pose | before avg / p95 | after avg / p95 | result |
|---|---:|---:|---|
| ground, 0.154 km | 13.54 / 18.75 ms | 11.33 / 18.59 ms | no regression; avg -16.3% |
| orbit, 291.4 km | 5.48 / 8.20 ms | 5.19 / 5.95 ms | no regression; avg -5.3% |

The initial cold streaming frames are deliberately excluded from both sides;
the script and exact two poses are in `viewer/scripts/solar-bench.play`.

No windowed viewer was available; all validation above is release/headless.
