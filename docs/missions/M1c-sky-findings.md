# M1c-sky findings

2026-10-25. Worktree `C:/code/triangulum/.claude/worktrees/m1c-sky`, branch
`m1c-sky`, base `b506c60`. Changes are **uncommitted**. No delegation.
`stream.rs`, `mesh.rs`, `engine.rs`, `worldgen/`, and the frozen viewer's
source are unchanged.

## Built

- Ported July's horizon/zenith gradient, low-sun warmth, sun halo, star
  distribution/colors/brightness, altitude fade, blue limb, and exponential
  distance fog. The sun disc is the analytic silhouette of July's 109-radius
  sun at its Kepler distance (~0.51-degree angular radius), drawn in the sky
  pass instead of adding a sphere mesh. Stars rotate into the inertial frame.
- Ported inventory §10.2's sun + hemispheric sky ambient, §10.3's cool
  phase-dependent moonlight and sky-shaped night floor, §10.4's 60% underwater
  tint, and §10.5's 0.90 risers / 0.55 undersides / 0.93 water sides. The sun
  contribution shuts off below the **pixel's** horizon, including from orbit.
  Lighting remains white; the warm tint belongs to the sun/sky, as in July.
- Preserved the existing packed 8-byte quads and AO corners. AO uses July's
  1/.85/.70/.55 brightness levels, reducing strength for large voxels. Face
  normals come from each column's curvature-grid tangents; cross products
  account for the grid's nonorthogonal axes. There is no chunk-origin normal
  trick or integer-height-derived top normal.
- `game/src/sky.rs` implements the viewer's f64 Kepler elements, eight Newton
  steps, axial tilt, rotation convention, and mean-anomaly season phase.
  One absolute clock drives all render effects. The app advances it on its
  existing fixed simulation steps, never on render/settle/readback counts.
  The moon orbit is evaluated for lighting only; no visible moon or eclipses.
- `sky` in `game/assets/world.json` contains a 1,440-second game day, 7-day
  lunar month, 3 months/season, and 84-day year. CLI overrides are
  `--day-length`, `--lunar-month-days`, `--months-per-season`, and `--year-days`.
  Validation requires four coherent seasons. `--time HH:MM` or decimal hours
  starts at the requested local solar hour at the spawn longitude.
  `--time-scale 0` pauses; `[` / `]` traverse 0, .1, 1, 10, 60, 600, 3600×
  without jumping the clock. README and `--help` document them.
- Kept infinite reversed-Z, Depth32Float, clear depth 0, opaque/water
  `Greater`, and camera-relative vertices. The sky unprojects the **near**
  plane and draws at depth 0 with `GreaterEqual` after opaque terrain and
  before water. Hidden sky pixels can fail depth early.

Source references: inventory §§10–11 and 13; `viewer/src/orbits.rs:398–485`,
`viewer/src/shader.wgsl:1647–1724`, `1815–1820`, `2376–2568`.
The old CameraUniform interface stays intact for the other missions;
the renderer adds a 144-byte SkyUniform. An immutable startup configuration
bridges the existing app/bench/play renderer constructors without changing
their engine or mesh interfaces.

The optical lengths use the inventory's Earth-to-new-planet scale:
45/6371 planet radii for altitude fade, 20/6371 for the limb, and 22.2985/R
per metre for fog. At the default radius these are ~640 m, ~284 m and a
~4.06 km fog e-folding distance. They are JSON parameters for later look
tuning. Thus 10 km is already above almost all the new atmosphere; 45 km
of literal atmosphere would engulf this planet. Colors/gains are July's.

## Visual evidence

**36-image contact sheet (durable, reviewable artifact):**

`C:/code/triangulum/.claude/worktrees/m1c-sky/docs/missions/M1c-sky-evidence/contact-sheet.png`

Four columns are noon, golden hour (sun +5°), dusk (sun −3°), and local
solar midnight. The default snowy range is at high latitude, so its sunset
is later: ~18:54 golden hour and ~20:49 dusk, rather than reusing the hills'
~17:36 / ~18:09. Each engine pose drained streaming before capture.

Engine positions (degrees; altitudes above sea level):

| Pose | Latitude | Longitude | Altitude | View |
|---|---:|---:|---:|---|
| Default hills, ground | -4.565354 | 117.474432 | 38.480 m | heading -60°, pitch 0°, eye 1.7 m AGL |
| Snowy range, ground | 70.822742 | 127.611380 | 61.677 m | facing the highest range, eye 1.7 m AGL |
| High view | -4.565354 | 117.474432 | 10,000 m | heading -60°, pitch -35° |
| Orbit | -4.565354 | 117.474432 | 500,000 m | heading -60°, pitch -85° |

The sheet also includes the same snow approach at **1/8 vertical scale,
H4096 / sea1536**, eye 532.638 m ASL. This is a labeled comparison, not a
default change. Tops, sunward sides, and lee sides remain distinct at noon
and low sun. At the default 1/70.3 scale, broad mountain relief is very low
and the far peak barely clears curvature/foreground; lighting does not add
the landforms Opus is building. The 1/8 row makes the face contrast easier
to judge. Distant terrace/ripple patterns remain visible.

July was rebuilt with `cargo build --release --example play` and rendered
with `sun LAT LON` commands derived from the engine's ephemeris. Weather was
off. Its original hills capture landed in foliage, so the final comparison
uses **50 m AGL** on both ground rows. High views match altitude/radius:
956.2 km and 47,810 km on July's 8,660-km-radius planet. The engine's existing
60° FOV and July's 65° FOV were retained. These are look comparisons, not
pixel-identical camera/terrain baselines. The visible July moon remains
present in its reference images.

Absolute raw evidence paths:

- Engine: `C:/code/triangulum/.claude/worktrees/m1c-sky/game/out/m1c-sky/engine-final/`
- Engine 1/8 comparison: `C:/code/triangulum/.claude/worktrees/m1c-sky/game/out/m1c-sky/engine-vscale8-final/`
- July: `C:/code/triangulum/.claude/worktrees/m1c-sky/game/out/m1c-sky/viewer-clear/`
- Windowed `--at` / `--time 17:30` smoke:
  `C:/code/triangulum/.claude/worktrees/m1c-sky/docs/missions/M1c-sky-evidence/app-sunset.png`
- Underwater plus untinted diagnostic control:
  `C:/code/triangulum/.claude/worktrees/m1c-sky/docs/missions/M1c-sky-evidence/underwater.png`
  and `C:/code/triangulum/.claude/worktrees/m1c-sky/docs/missions/M1c-sky-evidence/underwater-control.png`

The durable evidence directory also contains both engine manifests, the
July script, contact-sheet assembly script, test transcript, play transcript,
and the final bench reports. Original PNGs and full run logs stay under
the ignored `game/out/m1c-sky/` directory.

## GPU cost — provisional

RTX 2060 / Vulkan, 1920×1080, release build, 4 worker threads, default world,
240 measured frames per settled pose. These are **whole main GPU pass**
times (opaque terrain + sky + water), excluding HUD/readback. They do not
isolate a sky-only incremental cost. Other agents are active; GPU clocks
also varied (one idle sample was P8 / 315 MHz). Do not treat these as an
exclusive-machine performance gate.

| Pose | Noon avg / p95 ms | Night avg / p95 ms |
|---|---:|---:|
| Ground hills | 0.46 / 0.53 | 0.56 / 0.96 |
| Ground mountains preset | 0.75 / 1.22 | 1.06 / 1.67 |
| 10 km | 0.48 / 0.51 | 0.58 / 1.15 |
| 500 km | 0.43 / 0.49 | 0.38 / 0.41 |

The roughly 1 ms average budget is close: the night mountain pose is
slightly above it, and some tails exceed it. **A strict under-1-ms claim is
not verified.** Earlier runs varied appreciably; the reports above are the
final shader, not a selection of the fastest samples. Daytime work skips
zero-contribution moonlight/stars, and camera air density is evaluated once
in f64 on the CPU.

The fixed 60 s flight (night) produced 29,650 frames: **496.5 average fps,
231.6 fps for the worst 1%, worst frame 7.36 ms**; GPU **0.55 ms average /
1.28 ms p99**. These numbers are provisional too.

Reports:

`C:/code/triangulum/.claude/worktrees/m1c-sky/docs/missions/M1c-sky-evidence/bench-noon.md`

`C:/code/triangulum/.claude/worktrees/m1c-sky/docs/missions/M1c-sky-evidence/bench-night.md`

## Verification and limits

- `cargo test --release`: **40 passed**, including clock speed/pause,
  invalid parameters/time strings, local-hour seek across longitudes,
  stateless ephemeris, and rotating inertial stars.
- `--play scripts/m1-done.play`: **83 assertions, 0 failed, 0 errors**;
  digging/building/saving/reloading and distant bridge captures passed.
- Both reels completed all settled captures. Underwater evidence checked
  that the eye at −3 m ASL occupies a liquid voxel; paused readbacks were
  byte-identical. Across all 2,073,600 underwater pixels, comparison with
  `0.4 * control + 0.6 * (0.02, 0.07, 0.16)` in linear RGB had mean residual
  0.00089 and maximum 0.00591, consistent with 8-bit output quantization.
  The windowed app captured and exited without errors.
- Invalid `--time 24:00` and an incoherent 8-day/84-day calendar both
  returned exit 2; `--lunar-month-days 8 --year-days 96` loaded successfully.
- `git diff --check` passed. No commit was created.

Limits: no visible moon, eclipses, clouds, weather, valley mist or dawn fog;
no cave skylight propagation/carry-light replacement. No continuous
natural-surface normals are present in the packed mesh, nor an edit mask
to distinguish natural terrain from player walls. This pass retains cube
normals at every LOD instead of inventing the integer-terrace normals that
caused July's shading defects. The generator/mesh follow-up is still needed
for smooth large-scale slope lighting. The existing gray far oceans are
worldgen work, not a sky fix.

Time is not persisted in chunk saves. The existing headless play harness
holds the starting sky time rather than advancing it with its player steps;
its session loop is outside this mission's ownership. The new sky reel
seeks the same clock explicitly, and the windowed app advances normally.
The day-length value is the viewer ephemeris's rotation/calendar period;
apparent solar noon intervals also include its orbital motion. Physical
keyboard operation of `[`/`]` and a full multi-season interactive run were
not manually verified; their logic is unit-tested. Only Vulkan was rendered.

## Reproduce

From `game/` (all commands use the populated main-checkout bake):

```powershell
cargo test --release
cargo run --release -- --bake C:/code/triangulum/game/assets/world/seed42 --sky-reel --threads 4 --out out/m1c-sky/engine-final
cargo run --release -- --bake C:/code/triangulum/game/assets/world/seed42 --sky-reel --vscale 0.125 --height 4096 --sea-level 1536 --threads 4 --out out/m1c-sky/engine-vscale8-final
cargo run --release -- --bake C:/code/triangulum/game/assets/world/seed42 --bench --skip-flight --only stand_hills,stand_mountains,alt_10km,alt_500km --frames 240 --threads 4 --time 12:00 --out out/m1c-sky/bench-final-noon
cargo run --release -- --bake C:/code/triangulum/game/assets/world/seed42 --bench --only stand_hills,stand_mountains,alt_10km,alt_500km --frames 240 --threads 4 --time 00:00 --out out/m1c-sky/bench-final-night
```

From `viewer/`, after **copying** the baked face/meta/rivers/weather files
from the populated main checkout (no links, no copied edits):

```powershell
cargo build --release --example play
./target/release/examples/play.exe ../game/out/m1c-sky/engine-final/july-comparison.play --out ../game/out/m1c-sky/viewer-clear --size 1920x1080
```

From the repository root:

```powershell
python docs/missions/M1c-sky-evidence/contact_sheet.py
```
