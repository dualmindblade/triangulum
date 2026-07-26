# D-2 — the harness observability bundle: findings

**Branch** `mission/d2` (worktree `C:\code\tri-d2`, cut from `74cb703`).
**Lane D (tooling).** Files touched: `viewer/examples/play.rs` (owned),
`viewer/scripts/pick_accuracy.py` (new), `docs/missions/D2-*`.
`renderer.rs`, `world_reel.py` and the reel pose tables were not touched.

All four items landed. `probe`'s koppen change (item 2, the one D-1 is waiting
on) is in the first commit on the branch.

---

## Gates

| Gate | Before | After |
|---|---|---|
| suites (`verify.sh`) | 8/8 PASS | 8/8 PASS |
| world reel | exit 0, 29/29 clean | exit 0, 29/29 clean |
| determinism double-run | — | byte-identical (`05-determinism.txt`) |
| perf reel | not run — A-1 owns it this shift | not run |
| convergence reel / flight | not run (no time-domain change) | — |

The baseline was measured in this worktree before any edit, not inherited:
`06-gates-before.txt`. Nothing was `--accept`ed.

---

## 1. `pick X Y` — the ground point under a screen pixel

`pick PX PY` casts the camera ray through a pixel and reports where it lands.
Fractional pixels are allowed (`pick 639.5 359.5` is the exact frame centre of
a 1280-wide frame).

```
[8] pick 639.5 359.5: hit=neisor lat=-0.7969240 lon=-67.9400971 dist=0.25917km
    elev=35.00deg err=+-2.2m surface=water z=115.60m vis=115.6m solid=114.6m
    koppen=0/Af | col=1.49m bracket=0.50m evals=87
```

### How the ray is built — and why reversed-Z was a non-issue

The spec flagged this as the risky part ("reversed-Z with an infinite far
plane is not a friendly projection to invert"). **It does not need inverting.**
A pixel's *direction* is a function of fov, aspect and the view basis only;
the depth mapping never enters. And the full 4×4 inverts cleanly in f64, so
`pick` unprojects NDC `z = 1` (where reversed-Z puts the near plane) through
`camera.view_proj_for_surface_altitude(...).inverse()` — the renderer's own
matrix, at the renderer's own aspect and near plane. There is no duplicated
frustum constant anywhere in `pick`, so it cannot drift from `camera.rs` the
way a copied 65° would.

### What it marches

The **visible** surface: the solid column top, or the open-water plane when a
column carries standing water. Marching `surface_height_km` alone would return
the *lakebed* for a pixel that plainly shows water. Bracketed by a ±30 km
(×`--exagg`) shell around the body, then an adaptive coarse march (step =
0.35 × radial clearance, clamped to 0.5 m … 2 km) to the first sign change,
then 60 bisections.

Neisor, the moon (at its orbital centre) and the Sun are all tested; the
nearest hit wins. This works in every camera mode — focused, freecam, landed
on the moon.

### Accuracy — measured, not asserted

`viewer/scripts/pick_accuracy.py` (new, exit 0/3 like the reels). **24 poses**
= 4 sites × 6 altitudes from 50 m to 100 km, **384 picks**, three independent
ground truths plus a visual check:

```
PICK ACCURACY  24 poses, 384 picks analysed
  A aim round-trip miss       median    0.064 m  max    0.096 m
  B ray vs independent calc   median   0.0103 m  max   0.0124 m
    (same, radial component)  median   0.0039 m  max   0.0059 m
  C picked point off surface  median    0.000 m  max    1.000 m
  D water/terrain vs the pixel 378/384 agree (98.4%)
PICK: clean
```

- **A — aim round-trip.** Place the camera at a known point, solve the exact
  3D yaw/pitch onto a known target, `pick` the centre pixel, measure the
  great-circle miss. Absolute ground truth; everything is in play.
  **≤ 9.6 cm across every pose.**
- **B — ray fidelity.** The script re-derives `camera.rs`'s projection by hand
  (the algebra V-2 already committed in `altitude_ladder.py`), walks that ray
  out by the distance `pick` reported, and compares. Pure projection error.
  **≤ 1.24 cm lateral, ≤ 0.59 cm radial** — which is the floor set by printing
  lat/lon at 7 decimal degrees (1.5 cm on Neisor), i.e. the inversion is exact
  to the resolution the check can see.
- **C — on-surface.** Teleport to every returned coordinate and ask the harness
  what `surface_height_km` is there. Tolerance is one column *step*, read from
  the harness's own `column_step_max`: a hit on a column boundary can re-probe
  into either neighbour. All 384 inside tolerance.
- **D — visual.** Classify each picked pixel water/land by colour and compare
  with `pick`'s verdict. 98.4%; the six disagreements are discussed below and
  are a real, stated limitation.

Both of check B's earlier formulations were wrong before they were right, and
both are written up in the script's docstring, because the failure modes are
instructive:

1. Reconstructing the hit as `radius = R + column top` produced up to 0.70 m
   of apparent error. Every outlier was exactly `1 block × cos(elevation)`:
   when a ray strikes the **side** of a column, the intersection sits below
   that column's top. `pick` now prints `z=` (the intersection's own height)
   next to `vis=` (the column's top) so the two are distinguishable, and the
   check uses the reported distance instead.
2. `R·acos(dot)` has a resolution floor of `sqrt(eps)·R` = **0.13 m** on
   Neisor. Every pose reported exactly 0.129 m or 0.182 m (= 0.129×√2) of
   "error" — pure `acos` noise. The chord/`asin` form resolves it away.
   (`altitude_ladder.py::great_circle_km` uses the `acos` form; see Feedback.)

### The stated error bound

Every hit line carries `err=+-N.Nm`:

```
err = half a column pitch  +  one block / tan(elevation angle)
```

The column pitch is *measured* per hit (voxels are gnomonic-anisotropic,
`col=` in the output shows 1.37–1.53 m across the sites tested), and the block
term carries the surface-model slack — `pick` marches the column lattice while
the renderer may be drawing the LOD mesh, and those can disagree by about a
block. Below ~2° elevation the bound has no finite value and the tool says so:
`err=+-GRAZING-unbounded`.

Measured against it, the bound is **conservative by roughly 15×**: the aim
round-trip missed by ≤ 0.10 m where `pick` printed ±1.4–1.6 m. That is the
right direction to be wrong in, but it is worth knowing the bound is a bound
and not an estimate.

### Degenerate cases — named, chosen, exercised

Full transcript: `D2-evidence/01-pick-degenerate.AFTER.log`.

| Case | Behaviour | Why |
|---|---|---|
| **sky** | `hit=sky ray_lat=… ray_lon=…` | The ray's own celestial direction is still useful (aim a `focus`, find the moon). Never guesses a hit. |
| **past the horizon** | a normal hit with a small `elev` and a large `err` | Not a special case — it is the honest continuous limit. From 300 km the sky/ground transition falls between pitch −14° and −15°, matching the analytic horizon dip `acos(R/(R+h))` = 15.13°. At −15° the hit is 1983 km away at `elev=2.22°`, `err=+-26.5m`. |
| **water** | `surface=water`, with `vis=` the water plane and `solid=` the bed beneath | Marching only the walkable surface would return the lakebed for a pixel that shows water. A **frozen** sheet reads as `terrain` — it is solid ice, the same rule every other world query uses. A **flooded cave** is excluded (rock is above it). |
| **moon** | marched as a body at its orbital centre; nearest hit wins | `focus moon` centre pixel → `hit=moon lat=-9.19 lon=-153.21 dist=4712 km`; off-disc pixels → `hit=sky`. After `moonland`, a downward pixel → `hit=moon` 3.2 m away. |
| **sun** | `hit=sun dist=…km` | No columns; a plain sphere. |
| **eye at/below the surface** | hit at distance 0, flagged `EYE-AT-OR-BELOW-SURFACE` | Underwater or inside terrain. Silently reporting the eye's own column as a "pick" would be a trap. |
| **pixel outside the frame** | hard error, run exits non-zero | A clamped pixel would answer the wrong question. |
| **marcher budget exhausted** | `WARNING=step-budget-exhausted-may-not-be-first-surface` | Kept distinct from `sky`. A truncated ray reported as sky is exactly the quiet wrong answer this tool must not give. |

Not geometry, therefore invisible to `pick`, and stated in the header:
**trees, rocks, impostors and clouds.** A pixel on a canopy returns the ground
under the canopy.

### The one limitation I want on the record

`pick`'s water/terrain verdict is **column truth**. Beyond the voxel
near-field the LOD mesh may render the same column differently. All six of
check D's disagreements are one site's centre pixel at 5 / 20 / 100 km:

| altitude | centre pixel RGB | `pick` |
|---|---|---|
| 0.05 / 0.2 / 1 km | (67,116,151) blue | `water` |
| 5 / 20 / 100 km | (63,125,60) green | `water` |

A small forest pond that is water in column truth is drawn as land by the mesh
from 5 km up. That is either legitimate LOD or a `sync_diff` finding; it is
not a `pick` error, and adjudicating it is not this mission. Flagged in
Feedback.

---

## 2. `probe` prints koppen

```
probe 0.87000 -89.80000: h=829.2m water=- lake=false frozen=false
  koppen=6/BSk precip=148mm annual=2.28C seasonal=2.28C …
```

`koppen=<id>/<code>` plus `precip=<mm>`. The id is `planet.koppen(face,u,v)` —
the **domain-warped** class the surface materials actually use, and literally
the same call `write_state` already puts in the JSON sidecar, so the
transcript and the sidecar cannot disagree. `255` is the bake's no-class
sentinel and prints as `255/--`.

Two independent cross-checks, because a mislabelled class table would be worse
than no label:

- **The code table.** `07-koppen-table-crosscheck.txt` compares
  `play.rs::KOPPEN_CODES` element by element against
  `planetgen/biomes.py::KOPPEN`, the authority that produced the `koppen:u8`
  byte in every `face_*.bin`. All 30 identical.
- **The raster.** `08-koppen-vs-raw-raster.txt` reimplements
  `face_from_dir` + `raw_koppen` in Python and reads `face_*.bin` directly —
  exactly the duplication V-2 had to do by hand and that this item exists to
  remove. **7/8 probes match the raw baked texel exactly**; the eighth is at
  `altitude_ladder.py`'s `ecotone` site, where the raw texel is the `255`
  sentinel and the warped read gives `Cfb`. That is the documented design
  (`planet.rs`: "255 is the baked climate sentinel and can occur on dry land"),
  not a discrepancy.

Discriminating two known-different biomes, with the temperature/precipitation
from the same probe as corroboration:

| site (world reel pose name) | koppen | precip | annual |
|---|---|---|---|
| `desert` 0.870 −89.800 | `6/BSk` cold steppe | 148 mm | 2.28 °C |
| `jungle` −0.906 −67.804 | `0/Af` tropical rainforest | 12219 mm | 28.40 °C |
| `frozen_lake` 51.180 85.830 | `26/Dfc` subarctic taiga | 258 mm | 2.55 °C |
| `sea_ice` 83.997 40.220 (ocean) | `255/--` | 38 mm | −15.31 °C |

The letters track the numbers the way Köppen defines them: `EF` (ice cap) only
appears where the probe also reports sub-zero annual temperature (−1.50 °C at
12.78 20.0), and `BWk` where it is cool and arid (1.95 °C, 173 mm).

**Caveat worth knowing when reading a probe line:** `koppen` is warped,
`temp`/`precip` are not (`planet.rs` keeps them unwarped for terrain, weather,
ecology and water rules). Away from class edges they agree; *inside* a warp
band they legitimately do not — the `ecotone` probe reports `BWk` with
1832 mm. That is not a bug in either field.

---

## 3. The first-pose anchor, and opt-in wall-clock timestamps

A 500-pose recording in the F10 format (`03-pose-anchor.play`):

**Before** — the entire transcript, for 500 poses:
```
[2] mode fly
[3] weather time 554.373000
run complete -> …
```

**After** — exactly two more lines, and no more however long the recording:
```
[4] posehz 10 -> 6 sim tick(s) per pose
[5] pose #1 ANCHOR sim_ticks 0 t 0.0000s -> lat 13.000000 lon -4.900000
    alt 6.00000 km yaw 170.0 pitch -35.0 | 6 ticks/pose live false
    render false (later poses are silent)
```

The anchor carries what a wall-clock driver actually needs: the sim tick and
sim time the recording starts at, the tick cadence, and whether `TRI_POSE_LIVE`
/ `TRI_POSE_RENDER` are in effect. `posehz` traces too — F10 recordings emit
exactly one, and the cadence is the first thing any time-domain reader wants.

`TRI_LOG_WALLCLOCK=1` adds an absolute anchor line and a `[+seconds]` prefix
to **run.log only** (stdout stays the plain live stream):

```
[    0.000] # TRI_LOG_WALLCLOCK t0 = 1785099802.609856 unix_s
[    1.490] [2] mode fly
[    1.490] [4] posehz 10 -> 6 sim tick(s) per pose
[    1.494] run complete -> …
```

`t0` is unix seconds, so a Python driver lines its own `time.time()` up
directly instead of correlating stdout against `frame_counter` in a separate
stderr stream (what V-3 had to do).

**It does not perturb timing and it is off by default.** Verified
(`05-determinism.txt`): with the variable set, every PNG and every JSON sidecar
is bit-identical to the unset run, and run.log is identical once the prefix and
the `t0` line are stripped. When unset the code path is one `Option` check on a
line that was already doing a `format!` and a `println!`.

---

## 4. The header doc comment

Rewritten to actually be the authoritative list `viewer/CLAUDE.md` claims it
is. It was missing:

- **commands:** `pose`, `posehz`, `bench`, `glide`, **`dwell`** (the spec did
  not know about this one either), `stream`, `voxels`, the `moonland` aliases
  (`moonteleport`, `teleport-to-moon-surface`), and `sun physical|live`.
- **`bench`'s no-sim-advance behaviour**, now called out explicitly as the
  primitive it is: real wall time for the streamer without moving the clock.
- **43 of the 59 `assert` fields.** The old list had 16. The full set is now
  grouped by what it is for (pose/camera, physics, water, clock/sky,
  columns/edits, vegetation, moon).
- the **implicit-`==` two-token assert form** (`assert grounded true`), which
  existed only in an inline comment.
- the **environment variables** the harness itself reads — `TRI_POSE_LIVE`,
  `TRI_RAW_CAPTURE`, `TRI_POSE_RENDER`, `TRI_LOG_WALLCLOCK` — including the
  `TRI_POSE_LIVE` + `TRI_RAW_CAPTURE` composition that `viewer/CLAUDE.md` says
  cost two separate sessions to discover.
- the "examples are not built by a bare `cargo build`" trap, at the top.

---

## 5. One fix outside the four items

`look` and `turn` traced yaw/pitch with `{:.0}`. Writing the horizon sweep for
item 1, `look 0 -15` and `look 0 -15.2` produced **identical transcript
lines** — for two poses whose ground points are 140 km apart. Now `{:.4}`.
Nothing in the tree parses those lines (checked), and generators already aim to
5 decimals (`altitude_ladder.py`), so the transcript was strictly less precise
than the scripts feeding it.

---

## Verifying the mission spec

| Spec claim | Verdict |
|---|---|
| `pose`, `posehz`, `bench`, `glide` exist at `play.rs:795, 799, 863, 1011` and none is documented | **Correct** at `74cb703`. It also missed `dwell`, `stream` and `voxels`. |
| `probe` prints h/water/lake/frozen/temps/levels/river/column truth, no koppen | **Correct.** |
| `face_*.bin` is `n*21 bytes: elev f32, koppen u8, rough/precip/temp/flow f32` | **Correct**, and *planar*, not interleaved (`n*4` elev, then `n*1` koppen, …) — worth stating, since "n*21 bytes" reads as a struct. |
| `pose`/`posehz` emit no trace lines | **Correct.** |
| reversed-Z + infinite far plane makes the projection hard to invert | **Refuted.** See item 1. The depth mapping is irrelevant to a pixel's direction, and the matrix inverts fine. This was the spec's stated main risk and it was not one. |
| the four items are independent | **Confirmed.** |
| `pick` belongs in `play.rs`, not a script | **Confirmed by the code.** The marcher needs `col_ctx_body` / `surface_height_km` / `water_surface_km` at every step — a script would have to reimplement the `face_*.bin` + `rivers.bin` + edits stack that V-2 already had to duplicate once. |
| every item is checkable by instrument, no human verdict needed | **Confirmed.** Nothing here needs Austin or Andrew. |

---

## Evidence packet — `docs/missions/D2-evidence/`

| File | What it shows |
|---|---|
| `01-pick-degenerate.play` / `.BEFORE.log` / `.AFTER.log` | **The repro.** BEFORE: exits **1** at the first `pick` line (`unknown command`). AFTER: exit 0, all seven degenerate cases. |
| `02-probe-koppen.play` / `.BEFORE.log` / `.AFTER.log` | `probe` before and after, same coordinates. |
| `03-pose-anchor.play` (500 poses) / `.BEFORE.log` / `.AFTER.log` / `.AFTER-wallclock.log` | 0 trace lines → 2; and the timestamped variant. |
| `04-pick-accuracy-report.json`, `04-pick-accuracy.txt`, `04-pick-accuracy-annotated.png` | The full accuracy run: per-pick numbers, the summary, and a frame with the picked pixels marked W/T. |
| `05-determinism-A.play` / `-B.play` / `05-determinism.txt` | Double run byte-identical; `TRI_LOG_WALLCLOCK` changes no rendered byte; inserting `pick` lines changes no rendered byte; `pick`'s own output is reproducible. |
| `06-gates-before.txt` / `06-gates-after.txt` | suites 8/8 and world reel 29/29 clean, both sides. |
| `07-koppen-table-crosscheck.txt` | `play.rs::KOPPEN_CODES` == `planetgen/biomes.py::KOPPEN`. |
| `08-koppen-vs-raw-raster.txt` | probe's koppen vs an independent Python read of `face_*.bin`. |
| `09-pick-to-probe.play` / `.log` | The composition the mission asked for, end to end: pixel → coordinates → `probe` → asserts that pass. |

The `BEFORE` logs were produced by a binary built from `74cb703`'s
`play.rs` in this worktree, not by reasoning about the diff.

---

## Feedback

Mandatory (new tooling). Not re-filing `viewer/CLAUDE.md`'s
`scripts/verify.sh` path error — the spec already did.

### Things that will bite the next mission

1. **`repro_shot.py` cannot actually reproduce a play-harness shot, and fails
   quietly-ish.** It reads `sun_lat_deg`, `sun_lon_deg`, `weather` and
   `exaggeration` from the sidecar. `main.rs` writes all four
   (`main.rs:681-695`). **`play.rs::write_state` writes none of them.** So
   `repro_shot.py` on a frame from a `.play` run prints a warning to stderr and
   silently substitutes sun `(30, 30)` and "live weather at time zero" — a
   different scene from the one in the PNG. This mission's spec advertises
   `pick` as composing with `repro_shot.py`; that composition is currently
   half-broken on the `play.rs` side. I did **not** fix it: the sidecar schema
   is shared with tools I do not own and A-1/D-1 are live in the tree this
   shift. It is a small, well-defined ticket (add four keys to `write_state`)
   and it should be someone's next 20 minutes.

2. **`R·acos(dot)` great-circle distances are noise below ~0.2 m on Neisor.**
   `sqrt(f64::EPSILON) × 8660 km = 0.129 m`. `altitude_ladder.py::great_circle_km`
   uses that form, and `framing_check` reports `miss_m` to two decimals — so
   every sub-decimetre framing number in `report.json` and on the ladder
   contact sheets is fabricated precision. The chord form
   (`2·R·asin(|a−b|/2)`) is a one-line fix. I did not touch `altitude_ladder.py`
   (V-2's file, and D-1 is working next door). This cost me a full re-run and a
   wrong conclusion before I spotted it.

3. **`pick`'s water verdict is column truth; the LOD mesh can disagree.** A
   forest pond at `−0.79691 −67.94009` is `water` in column truth at every
   altitude, but the rendered pixel is green (land) from 5 km up and blue below
   1 km. Either legitimate LOD or something `sync_diff.py` should be catching.
   Numbers in item 1. Somebody who owns water should look; I am not qualified
   to call it.

4. **`verify.sh` silently skips `gen_survey.py` in any fresh worktree**,
   because it guards on `output/seed42_r8/planet_data.npz` and `output/` is
   gitignored. So "regenerate the data-driven sweep so it reflects the current
   planet" does not happen, and `auto-survey.play` runs whatever is committed.
   It still passes — but a mission that changed the planet and trusted
   `verify.sh` to re-derive the sweep would be trusting something that did not
   run. A one-line `echo "skipping gen_survey (no output/)"` would make it
   visible.

5. **`viewer/assets/weather_tuning.json` shows as modified in a fresh
   worktree.** It is a CRLF/LF difference introduced by copying `assets/` from
   a populated tree (`git diff --ignore-cr-at-eol` is empty). Harmless, but it
   makes `git status` permanently dirty and it is one careless `git commit -a`
   from landing on main. A `.gitattributes` entry would settle it.

### Rough edges in the harness itself (I am the mission most likely to see them)

6. **There is no way to print a value without asserting on it.** I wanted "tell
   me `surface_height_km` here" three separate times and ended up writing
   `assert surface_height_km > -1000` as a print idiom, parsing the
   `(actual …)` out of the transcript. That works but it is a hack in three of
   my scripts and probably in others. A `dump FIELD [FIELD…]` that traces the
   same values `assert` can read would be the single highest-leverage small
   addition to this harness. `state` writes a sidecar but does not trace, and
   the sidecar exposes fewer fields than `assert` does.

7. **`look`/`turn` transcript precision** — fixed here, but the class of bug is
   worth naming: several trace lines round harder than the script that produced
   them. `teleport` traces lat/lon at `{:.6}` (15 cm) which is fine; `look` was
   at `{:.0}`. Worth a sweep.

8. **`glide` and `dwell` share one match arm and one undocumented convention**
   (`dwell SECONDS` reads `f(1)`, `glide …` reads `f(6)`), and neither appeared
   in the header. If you pass `dwell` six arguments it silently ignores five.

9. **`sun physical` / `sun live` are aliases for the same thing** and were
   undocumented; `sun` with a bad first token falls through to `f(1)?` and
   errors with "bad number", not "did you mean `physical`?".

10. **`assert` accepts `>` against a `none` value and silently fails it.**
    `assert water_surface_km > -1000` on a dry column reports
    `ASSERT FAIL: water_surface_km > -1000 (actual none)` — correct, but it
    reads like a numeric comparison failing rather than a null. Cost me one
    debugging cycle.

### Assumptions I had to make because the spec did not say

- **`pick` output is a trace line, not a JSON sidecar**, in `key=value` form so
  it is regex-parseable (the spec said "trace output in the same style", so I
  followed `probe`).
- **`pick` marches the *visible* surface**, not the walkable one, so a water
  pixel returns the water plane. Frozen sheets read as terrain; flooded caves
  are excluded. The spec listed "water vs terrain" as a degenerate case to
  choose, so this is the choice.
- **`TRI_LOG_WALLCLOCK` is the env var name**, it prefixes `run.log` only, and
  it leads with an absolute unix-epoch anchor. The spec said "an env var" and
  "run.log lines".
- **The anchor line's content** (sim tick, sim time, pose, cadence, live/render
  flags). The spec said "a single first-pose trace line".
- **`posehz` traces too.** One extra line per script; the alternative was a
  cadence number that appears nowhere.
- **I added `z=` to `pick`'s output** mid-mission after check B exposed
  column-side hits. It is load-bearing: without it `vis=` is ambiguous.

### Time sinks

- The two wrong formulations of check B (≈40 minutes total). Both were my
  measurement being wrong while `pick` was right, which is the good failure
  mode but not a cheap one. The `acos` floor in particular is a trap I would
  not have found without the values quantising to two exact numbers.
- Building a pre-change binary to produce honest `BEFORE` logs (~2 minutes of
  build, worth every second — "no repro, no fix" should mean a real run, not a
  diff read aloud).

### Surprising

- The `pick` centre pixel at the world reel's `pond_country` pose returns
  `−0.7969240 −67.9400971`. `altitude_ladder.py`'s `lake_shore` site — derived
  independently by V-2, months of sessions ago, by inverting the projection in
  Python — is `−0.79691 −67.94009`. **2.4 m apart**, and V-2's target was
  snapped to a class boundary rather than the frame centre, so they should not
  even agree that closely. Two independent implementations of the same
  inversion landing on the same square metre was the moment I believed the
  numbers.
