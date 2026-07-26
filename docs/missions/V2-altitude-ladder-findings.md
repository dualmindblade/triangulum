# V-2 — Altitude ladder (B-17 instrument)

**Deliverable:** `viewer/scripts/altitude_ladder.py` + contact sheets in
`viewer/interchange/altitude-ladder/`.
**Constraint honoured:** no Rust touched, nothing rebuilt. Everything ran on the
prebuilt `viewer/target/release/examples/play.exe` (mtime 2026-07-23 10:33).

---

## 1. What it is

B-17 ("all irregular boundaries must read as natural curves **at every
altitude**") is an altitude-sweep invariant. Nothing tested it that way — the
world reel has 29 poses at scattered altitudes, so answering it meant flying
around and remembering. The ladder renders **one ground feature from seven
altitudes with the camera geometry held fixed**, and lays the rungs out
side by side.

Six sites, one boundary class each:

| site | class | target | note |
|---|---|---|---|
| `lake_shore` | irregular lake shore | -0.79691, -67.94009 | forest pond field (reel `pond_country` aim point) |
| `river_bank` | river bank | 15.02469, 17.65934 | reel `river_wide` aim point |
| `ocean_coast` | ocean coastline | 15.10556, 150.92017 | `orbital_coast` archipelago, snapped to the waterline |
| `sand_patch` | sand/grass patch edge | 12.19894, -44.82774 | reel `dossier_ponds`; a near-field class |
| `snowline` | snow/rock line | -12.30741, 107.18103 | tropical 3.2 km massif; elevation-driven boundary |
| `ecotone` | biome ecotone | 10.76433, 81.17593 | desert→rainforest köppen edge (98 → 9386 mm/yr) |

Rungs: **0.05, 0.2, 1, 5, 20, 100, 300 km** — the spec's ladder, unchanged.
Kept as specified — they are roughly half-decade steps and they worked: the
0.05 km rung renders cleanly above the canopy at a 55° depression (the camera is
50 m up and 35 m back), and 300 km matches the reel's orbital poses, above which
cloud cover starts dominating the frame instead of terrain. The one change I
would make is adding a rung near 0.5 km, because that is where the artefact
starts and the standard ladder jumps 0.2 → 1.0 km straight over it; the
`--rungs` bracket exists for exactly that reason.

### How the framing is held

This is the part worth the most and it is not a heuristic:

- The camera sits on a **great circle at `d = alt / tan(55°)`** from the target,
  along the reverse of the site's view bearing. Depression is **constant at 55°
  on every rung**, so the projected ground plane is an exact similarity
  transform about frame centre — only scale changes.
- Yaw/pitch are solved **in 3D**, from the camera's actual position to the
  actual target position. Planet curvature (R = 8660.254 km) and the terrain
  height at *both* endpoints are handled exactly, not approximated by a flat
  tangent plane.
- Getting the terrain heights needs a **render-free pre-pass**: `probe` for the
  target's sampler truth (height + water surface), `state` for the exact
  `ground_km` the harness will put under each camera. Without it the bottom
  rungs aim tens of degrees off: at 50 m altitude the camera is only 35 m from
  the target horizontally, so a 5 m error in either endpoint's height is a 6-8°
  pitch error, and the aim point walks off the feature entirely.
- Weather is pinned (`weather pin 0.10 0.0`, `time 0`, `season 0.45`) and the
  sun is derived **from the target, not the camera**, so every rung of a site
  shares one lighting solution. (`sun LAT LON` is a GLOBAL direction — the
  camera moves up to 210 km between rungs, so deriving it per-rung would have
  changed the light along the ladder.)

### Running it

From `viewer/`:

```
python scripts/altitude_ladder.py                       # 6 sites, 7 rungs
python scripts/altitude_ladder.py --only lake_shore     # one site
python scripts/altitude_ladder.py --skip-render         # re-sheet + re-lint
python scripts/altitude_ladder.py --list                # the site table
python scripts/altitude_ladder.py --scout 20            # site maintenance
python scripts/altitude_ladder.py --rungs 0.3,0.5,0.7,1,1.5,2,3 --tag=-bracket
```

Exit **0** clean / **3** findings to review, matching the reels.

**Wall clock** (this machine, RTX-class GPU, 1280x720):

| step | cost |
|---|---|
| probe/state pre-pass (6 sites x 7 rungs + 6 probes) | ~2 s |
| render pass, 42 settled frames | ~55 s |
| sheets + lint + NCC | ~8 s |
| **full run** | **~66 s** |
| `--skip-render` re-sheet | ~8 s |
| `--scout ALT` (one frame per site) | ~10 s |

Probes are effectively free — 10,806 of them ran in 3.2 s including process
start. That is what makes the site-scouting workflow below practical.

### Output — and where the durable copy lives

`viewer/interchange/*` is gitignored, and `.gitignore` itself warns that
evidence has been lost to that rule before. So the sheets that matter are
copied to **`docs/missions/V2-evidence/`**:

| file | what it is |
|---|---|
| `specimen_1km_staircase.png` | **start here** — the primary finding in one image: three boundary classes at 200 m vs 1 km, native pixels |
| `ladder_<site>.png` x6 | the six standard contact sheets (the Andrew/Austin deliverable) |
| `ladder-bracket_river_bank.png`, `ladder-bracket_ecotone.png` | the fine 0.3–3 km bracket that locates the onset |
| `report.json`, `report-bracket.json` | every number quoted in this document |

The live run writes to `viewer/interchange/altitude-ladder/`:
- `ladder_<site>.png` — the contact sheet. **Row A** full frames, altitude left
  to right, red crosshair on the aim point (the identical ground point on every
  rung), captioned with the ground width the frame spans and the m/px.
  **Row B** native-pixel 512x288 centre crops, so the boundary can be judged
  without resampling. Flagged rungs get an orange border.
- `ladder-bracket_<site>.png` — the fine bracket (0.3 … 3 km).
- `report.json` — every rung's geometry, framing check, NCC and lint numbers.
- `truth.json` / `poses.json` — the pre-pass truth and the exact poses used.

The generated harness scripts are `viewer/scripts/_altitude-ladder*.play`. The
leading underscore is the repo's "regenerated on every run, never hand-edit"
convention and keeps them out of the diff.

---

## 2. Evidence the framing tracks

Three independent checks, all in `report.json`.

**(a) Geometric, per rung.** Take the pose the harness *actually stored* in the
`.json` sidecar (lat/lon/alt/radius/yaw/pitch), cast the frame-centre ray, and
intersect it with the sphere at the target's height. Distance from that hit to
the target:

> **Maximum aim miss across all 42 rungs: 0.22 m.**
> As a fraction of the frame's ground half-width: worst case **0.47 %**
> (at the 50 m rung, where the half-frame is only 47 m); **< 0.003 %** at every
> rung ≥ 5 km.

The residual is entirely the 5-decimal rounding on the `look` line in the
generated `.play`; it is not modelling error.

**(b) Image-based, adjacent rungs.** Each rung covers `ratio` times more ground
than the one below. Shrink the lower frame by `ratio` and normalised-cross-
correlate it against the higher frame's centre crop. If the framing tracked, the
peak sits at zero offset with a high correlation.

> Median NCC peak **0.77** over 36 adjacent pairs; median centre offset
> **3.2 %** of the crop half-width.
> `ocean_coast`: 0.92, 0.98, **1.00**, 0.96, 0.54, 0.74.
> `snowline`: 0.86, 0.91, 0.95, 0.93, 0.97, 0.89.
> `ecotone`: 0.82, 0.89, 0.95, 0.54, 0.88, 0.90.

**(c) Visual, on the sheet.** The crosshair is at frame centre on every column,
and frame centre *is* the aim point by construction. On `ocean_coast` the
crosshair sits on the waterline in all seven frames, from a 138 m-wide frame to
an 830 km-wide one. On `ecotone` the desert/forest edge passes through the
crosshair at all seven. Those two sheets are the ones to show Andrew first.

**Where NCC drops, framing is still exact.** Two sites report a weak
rung-to-rung correlation. In both cases check (a) shows a sub-metre aim miss, so
it is not a pose problem — the *content* changed:
- `lake_shore` 1 → 5 km (NCC 0.09): the pond is 300 m across; at 5 km altitude
  the frame is 13.8 km wide and the pond is ~8 px. The feature fell out of
  resolution, not out of frame.
- `sand_patch` 0.2 → 1 km (NCC 0.21): the sand patches are ~30 m and stop
  existing in the frame entirely, plus the LOD transition described below
  genuinely changes what is drawn.

The script now says this explicitly in the finding text rather than implying a
framing failure.

---

## 3. What the ladder revealed

**It found something, and it is reproducible across four independent boundary
classes.**

### 3.1 The 0.5–1 km staircase band (primary finding)

Between roughly **500 m and 1 km altitude every irregular boundary collapses
from voxel-scale dither onto a ~15–35 m lattice.** Below it the boundary is
finely dithered; above ~1.5 km the LOD blends and the edge softens; in the band
itself the boundary is a hard, regular, axis-aligned staircase.

All six of the clearest specimens are laid out side by side in
`docs/missions/V2-evidence/specimen_1km_staircase.png` (the durable copy); the
originals regenerate into `viewer/interchange/runs/altitude-ladder/`:

| frame | what it shows |
|---|---|
| `river_bank_0p2km.png` | smooth, naturally meandering bank — **good** |
| `river_bank_1km.png` | the same river drawn as a chain of rectangular blocks with a regular ~21 m sawtooth on **both** banks |
| `ecotone_0p2km.png` | a beautiful fractal desert/forest dither at voxel scale — **good** |
| `ecotone_1km.png` | the same edge as hard 35 m rectangular teeth, all fine dither gone |
| `lake_shore_0p2km.png` | smooth pond shore — **good** |
| `lake_shore_1km.png` | polygonal, straight-edged pond shores with square notches |

`lake_shore_1km.png` **is the defect the mission described** ("noticeably
polygonal pond boundaries in the mid-field — geometric, straight-edged lake
shores"). It reproduces deterministically at a named altitude.

The `--tag=-bracket` run (0.3/0.5/0.7/1/1.5/2/3 km) puts numbers on the onset —
`step_m` is the measured lattice pitch of the boundary:

| altitude | lake_shore | river_bank | snowline | ecotone |
|---|---|---|---|---|
| 0.3 km | 3.2 m | 3.4 m | 8.3 m | 9.4 m |
| 0.5 km | 4.5 m | 10.3 m | 11.4 m | 7.6 m |
| 0.7 km | 10.2 m | **15.0 m** | **25.6 m** | 10.6 m |
| 1.0 km | **19.4 m** | **21.5 m** | 32.7 m | **35.4 m** |
| 1.5 km | 44.5 m | 17.1 m | 37.5 m | 37.0 m |
| 2.0 km | 14.1 m | 22.0 m | 47.9 m | 64.4 m |
| 3.0 km | 76.5 m | — | 58.2 m | 72.7 m |

Bold = the lint fired. The transition is sharp: `river_bank` goes 3.4 m → 10.3 m
between 300 m and 500 m and never comes back. `ecotone` triples between 700 m
and 1 km. Note the pitch keeps growing above the band — the artefact does not go
away, it gets *blurrier*, which is why it is easy to miss when flying: you climb
past it in about two seconds.

This is the same defect class as `viewer/interchange/reel/accepted/river_wide.png`
(the blessed reel pose at 154 m altitude already shows a sawtoothed far bank),
so the reel has been carrying a specimen of it all along without anything
naming it.

### 3.2 A 14 km ruled coastline

`ocean_coast` is straight. Not "smooth" — **straight**. At 50 m, 200 m, 1 km and
5 km the water/beach boundary is a single unbroken diagonal line across the
frame; the lint's jumpiness at 1 km and 5 km is 1.4 and 1.8 with **zero**
measured lateral jumps, which is what a ruled edge looks like numerically. Only
at 20 km (55 km-wide frame) does the coast start to curve, and the 100/300 km
rungs are genuinely, beautifully fractal.

So the coastline has irregular structure at ≥ 20 km scale and at ≤ 30 m scale
(the voxel dither in the 50 m crop) and **nothing in between**. That reads as a
ruler, not a coast. Whether it is this location or the class is a question for
whoever owns the shoreline generator — the instrument only reports it.

### 3.3 Two dead world-reel poses (unrelated to B-17, found on the way)

While sourcing coordinates I rendered the blessed reel poses. Two of them do not
show the feature they are named for:

- **`lake_archi` (16.601, -12.222, 3 km)** — `interchange/reel/accepted/lake_archi.png`
  is a **featureless blue rectangle**. A probe grid of 625 points over ±12 km
  around the aim point returns water at *every single point*. There is no
  archipelago and no shore anywhere in the frame or near it.
- **`coast_beach` (8.097, -35.673, 60 m)** — `accepted/coast_beach.png` is a
  grassy plain. A probe grid of ±4 km finds **no water at all** and terrain at
  857–1025 m elevation. It is not a coast and there is no beach.

Both poses are guarding nothing: a regression in lake-shore or beach rendering
would leave those frames pixel-identical. They should be re-aimed or dropped.
(`pond_country`, `river_wide`, `dossier_ponds`, `orbital_coast` are all
excellent and I reused their coordinates.)

### 3.4 Boundary classes live in different altitude bands

Not a defect, but it changes how B-17 should be read. "Every altitude" cannot
mean the same thing for every class:

- `sand_patch` (~30 m patches) is resolvable **only** at 50 m and 200 m. At 1 km
  there is no boundary in frame at all.
- `ecotone` blending is 8 km wide (`BIOME_BOUNDARY_ZONE_KM`), so it only becomes
  a *boundary* rather than a gradient at ≥ 1 km.
- `ocean_coast` and `snowline` are the only two classes that read at all seven
  rungs.

So the honest form of the bar is "natural at every altitude **at which the class
is resolvable**", and the ladder now shows that band explicitly (the
`no-usable-boundary` rungs).

---

## 4. The staircase lint — verdict and numbers

**Verdict: it works, but only in a band, and it is gated to that band.**

It gates on rungs whose ground sampling distance is **≤ 3 m/px** (altitude
≲1 km), where the voxel lattice is still a usable reference scale. Above that
it prints its numbers, labels them `(informational)` on the sheet, and never
fails the run. This is not a hedge — I have the false positives to justify it,
below.

### 4.1 Four formulations that did NOT discriminate

Labelled specimens: BAD = `river_bank_1km`, `lake_shore_1km`;
GOOD = the 0.05/0.2 km rungs plus the 20/100/300 km rungs, all labelled by eye
before measuring.

| attempt | metric | BAD | GOOD | verdict |
|---|---|---|---|---|
| 1 | fraction of boundary in axis-aligned chain-code runs ≥ 8 px | 0.14, 0.08 | 0.28–0.64 | **anti-correlated** |
| 2 | implied world lattice pitch ÷ voxel size | 16x, 14x | 0.5–2x *but also* 144x/1172x/2930x at 20/100/300 km | fires on every high rung |
| 3 | run-length entropy / modal-run fraction | 0.24, 0.38 | 0.14–0.32 | overlapping |
| 4 | autocorrelation peak of the contour turn signal and of the boundary residual | 0.75, 0.73 | 0.18–**0.82** | overlapping, GOOD scores highest |
| 5 | tangent-direction concentration (top-2 histogram bins) | 0.34, 0.26 | 0.36–0.66 | anti-correlated |

Attempts 1, 3, 4 and 5 all fail for the **same reason**, and it is the
interesting one: *at 50 m altitude a 1 m voxel is 8 screen pixels, so a healthy
boundary genuinely is a big regular staircase.* Every scale-free "is this
staircased" statistic scores the honest voxel look higher than the artefact.
Attempt 2 fails from the other side: normalising by voxel size makes every rung
above ~5 km look catastrophic, because at 47 m/px the renderer legitimately has
no voxel-accurate geometry to draw.

That is the same trap `world_reel.lint_grid` fell into (designed periodicity
sitting in the artefact band) arriving from a different direction.

### 4.2 The formulation that works

Scan the dominant class boundary **line by line** and difference it. A natural
curve's lateral step per scanline is unimodal about its own local slope — 0s and
1s for a gentle boundary, 3s and 4s for a steep one. A lattice-quantised
boundary is **flat-then-jump**. Measured on `river_bank_1km`, right bank:

```
+1 +0 +0 +0 +0 +12 +0 +0 +0 +0 +1 +0 +0 +0 +0 +1 +0 +0 +12 +0 +0 +0 +1 ... +12 ... +13 ... +14
```

versus `river_bank_0p2km`, same bank:

```
+3 +1 +0 +3 +1 +1 +0 +1 +3 +1 +0 +3 +1 +1 +3 +0 +1 +1 +0 +3 +1 +1 +3 ...
```

Three conditions, all required:

| condition | what it rules out |
|---|---|
| `jumpiness = p98(|d|) / mean(|d|) ≥ 6` | boundaries that are just steep |
| `step_px ≥ 7` | jumps too small to read on screen |
| `step_voxels = step_m / 1.35 m ≥ 8` | **the honest 50 m voxel look** |

The third condition is the one the four failed attempts were missing.

### 4.3 Measured performance

Over the 33 gate-eligible rungs in the standard + bracket runs:

- **8 flags, 0 false positives.** Every flagged frame is visibly staircased.
  Nothing below 500 m altitude fires, including `snowline_0p05km`
  (jumpiness 6.9, step 16.6 px — a huge honest voxel staircase) which fails the
  `step_voxels` condition at 1.4x and is correctly ignored.
- **Recall is partial.** Two visibly-bad frames sit just under threshold:
  `snowline_1km` (jumpiness 5.8 vs gate 6.0; step 32.7 m = 24x voxel) and
  `river_bank_0.5km` (7.65x voxel vs gate 8x — it misses by 0.35). The lint is
  a *finder*, not a proof of absence.
- **The thresholds are fitted on ~18 labelled frames from one seed.** 6.0 / 7 px
  / 8x are provisional. Treat a change in them as a change to the instrument,
  not a tweak.
- **Above 3 m/px it is informational only**, because attempt 2's failure mode
  returns: `lake_shore_100km` and `ocean_coast_300km` read as gorgeous fractal
  coastlines and would score 2298x and 3516x voxel if the gate applied there.

So: a calibrated gate in the 0.05–1 km band, honest numbers with no gate above
it. It found all three primary staircase specimens on its own, which is what the
instrument was for.

---

## 5. Cut lines

- **No moon site.** `moonpose` is a body-local pose with its own yaw/pitch
  convention and no `teleport`-style altitude ladder; wiring the ladder geometry
  to it is a separate piece of work and the moon has no water/biome boundaries,
  which is where B-17 bites.
- **No accepted-baseline diff.** The ladder is a *finder*, not a change
  detector; the world reel already owns change detection. Adding
  `--accept`/blessing would be straightforward and is the obvious next step if
  this becomes a gate.
- **1280x720 only.** Rendering at 1920x1080 would give the lint ~2.2x more
  boundary pixels and probably tighten the thresholds, at ~2x the wall clock.
  `--size` is plumbed through but I calibrated at 720p and the thresholds are in
  pixels, so changing it invalidates them.
- **One seed.** Everything here is seed 42. The 0.5–1 km band is a property of
  the LOD pipeline, not the seed, but the specific pitches are not.
- **The class masks are colour heuristics** (`water`, `snow`, `sand`,
  `generic` two-means). They are the weakest link — see feedback.
- **Only the largest connected component's boundary is linted.** A frame with a
  staircased small pond and a clean big lake reports the big lake.
- **No `--accept`-style pruning of dead rungs.** Rungs where the class is not
  resolvable still render and still occupy a column. That is deliberate (it
  shows the resolvability band) but it costs frames.

---

## 6. Feedback

### 6.1 Bugs / smells found on the way

1. **Two world-reel poses guard nothing** — `lake_archi` and `coast_beach`; see
   §3.3. A reel pose that shows none of its feature is worse than no pose,
   because it reports "clean" forever. Something should assert that a pose's
   frame actually contains its class — the `probe` command already returns
   everything needed to check it, and it costs milliseconds.

2. **`world_reel.lint_grid`'s water mask misses turquoise water.** It classifies
   water as `b > g + 8`, but shoal/beach water renders around (90,180,180) where
   `b ≈ g`. I hit this directly: the entire `ocean_coast` ladder reported
   `no-usable-boundary` at four rungs until I switched to `b > r + 20`. That
   means `lint_grid` is silently *not looking at* shallow water — which is
   exactly where the per-quad shading seams it was written to catch would show.
   Worth a look independently of this mission.

3. **`sun LAT LON` is a footgun and the docs are right to shout.** The subtler
   half is that it interacts with camera *motion*: if you derive the sun from
   the camera position (the obvious reading of `world_reel`'s code) then a
   ladder whose camera moves 210 km between rungs gets different light per rung,
   and the whole comparison is lighting, not LOD. Deriving from the *target* is
   the fix. Worth one line in the play.rs header.

4. **`ground_height_km` returns the water surface, not the terrain.** At the
   lake_archi probe, `probe` reported `h=51.3m water=84.5m` and the camera's
   `ground_km` came back as 84.5 m. That is presumably deliberate (altitude
   above the thing you would land on) but it is not what "ground" says, and
   anything doing `alt + ground = terrain height` over water is wrong. Not
   documented anywhere I found.

5. **`shot` is much faster than I expected** — 42 settled frames in 55 s,
   including 300 km rungs. The docs' framing of "waits for streaming to drain"
   made me budget 10x that. Saying "~1.3 s/frame" somewhere would have changed
   how I planned the whole mission.

6. **My own bug, but the shape is worth recording:** I shadowed a function
   parameter (`tag`) with a loop variable in the same function, and the result
   was that the render pass wrote frames to `runs/altitude-ladder300km/` while
   `--skip-render` read `runs/altitude-ladder/`. Full runs were self-consistent
   so nothing looked wrong until a `--skip-render` produced a 21 km aim miss out
   of nowhere. **The framing check caught it** — which is a small argument that
   the geometric self-check earns its keep beyond satisfying the mission.

### 6.2 Complaints about the spec

- **"Reuse `world_reel.py`'s coordinates where sensible — they are known-good"**
  was the single most expensive sentence in the brief. Two of the six suggested
  sites (`lake_archi`, `coast_beach`) show nothing at all, and I only found that
  out by rendering them. If the spec had said "verify each pose actually shows
  its feature first", I would have started with the 10-second check instead of
  building around them.
- **The proposed staircase metric ("histogram the *axis-aligned* run lengths")
  is the wrong invariant and the spec states it as if it were right.** The
  lattice is world-aligned; its projection into screen space depends on yaw, so
  screen-axis alignment only holds for near-nadir views with yaw ≈ 0/90. More
  importantly the run *length* is confounded by the honest voxel look at low
  altitude, which no amount of histogramming fixes. I spent maybe 90 minutes
  implementing the spec's version faithfully before concluding it could not
  work. The escape hatch ("shipping it informational is acceptable") was
  genuinely useful and I nearly took it — the thing that saved it was printing
  the raw per-scanline differences instead of a statistic. **Suggestion for
  future specs: propose the *observation*, not the algorithm.**
- **"Same ground feature framed at every rung" is under-specified in one
  important way**: at 300 km a 300 m pond is sub-pixel, so *something* has to
  give. I chose "hold the camera geometry, let the feature shrink, and lint
  whatever boundary dominates the frame at that scale". The alternative ("hold
  the feature's apparent size") is impossible without FOV control, which the
  harness does not expose. Worth stating which one is wanted.
- **Exit-code convention "0 clean, 3 findings" doesn't compose with a finder.**
  This tool's *purpose* is to find things, so a healthy run exits 3 forever.
  The reels have an `--accept` baseline to make 0 reachable; this doesn't, so
  "3" here means "read the sheet", not "something regressed". That distinction
  should probably be in `PROCESS.md`.

### 6.3 Tooling that would have made this faster

- **A `screenshot LAT LON ALT YAW PITCH` one-liner** (or `--pose` on play.exe)
  so a single frame doesn't need a generated `.play` file. Half my scouting
  round-trips were "write a 6-line script, run it, read one PNG".
- **`probe` should print `koppen`.** It prints temp/precip/river/lake but not the
  biome class, so there is no way to scout a biome ecotone through the harness.
  I had to parse `assets/face_*.bin` in Python (21-byte interleaved records,
  koppen at offset `n*4`, 255 = ocean) to find one — which worked, but is a
  parallel implementation of `planet.rs`'s loader that will rot.
- **A documented pixel→lat/lon inverse.** I had to reconstruct the projection in
  Python from `camera.rs` (`view_basis`, 65° `perspective_infinite_reverse_rh`,
  the `right = cross(look, radial_up)` convention) to snap targets onto the
  boundary the renderer actually draws. This is ~25 lines and it is now the most
  useful function in my script (`pixel_to_ground` / `--scout`). A `pick X Y`
  command in `play.rs` that dumps the ground lat/lon under a pixel would be
  worth more than any other single harness addition for art verification, and
  would let `repro_shot.py` and this both stop guessing.
- **A "does this pose show its class" assertion.** `assert has_water == true`
  exists but is about the *camera column*, not the frame. `assert frame_class
  water > 0.05` would catch dead reel poses automatically.

### 6.4 Surprises

- Probes cost nothing. 10,806 in 3.2 s. I built the whole site-selection
  workflow around that after measuring it, and it changed the mission — targets
  are chosen by search, not by hand.
- The defect band is **narrow and low**. I expected B-17 problems to be an
  orbital-altitude issue. They are worst at 700 m–1 km, which is exactly the
  altitude a player flies at, and exactly the altitude the world reel samples
  least (its poses cluster at ≤300 m and ≥3 km).
- The 100 km and 300 km coastlines are the *best-looking* frames in the whole
  set. The far field is not the problem.
- `perspective_infinite_reverse_rh` takes a **vertical** FOV of 65°, which at
  16:9 is a 97° horizontal FOV. That is very wide, and it means the frame's
  corners are looking at ground 2-3x further away than frame centre — every
  per-frame statistic mixes LOD levels. I restricted the lint to the dominant
  boundary rather than fight it, but anyone writing a frame-wide metric should
  know.

### 6.5 Assumptions I had to make

1. **Depression 55°, held constant.** Not specified. Chosen so the projected
   ground plane is a similarity transform between rungs (which is what makes
   row-B crops and the NCC check valid), and so the 300 km rung's offset (210 km)
   stays far inside the horizon (2299 km).
2. **Camera placed *behind* the target along the reverse view bearing**, rather
   than target-under-camera with varying pitch. The latter is simpler but the
   comparison becomes "different perspective at every rung".
3. **Bearings reused from the world reel's poses**, on the theory that a
   human-tuned yaw frames the feature well. Unverified for the three sites I
   relocated.
4. **Aim height = water surface if the target is wet, else terrain height.**
   `probe` gives both; for a shoreline point they differ by under a metre.
5. **Voxel width 1.35 m** as the mean of the documented 1.7 x 1.0 m gnomonic
   anisotropy. The `step_voxels` gate is sensitive to this, and the true value
   varies across a cube face.
6. **`weather pin 0.10 0.0`, `time 0`, `season 0.45`** copied from
   `world_reel.py` without independent justification, so the two instruments
   agree.
7. **The lint judges the largest connected class component**, on the assumption
   that the dominant boundary is the one Andrew is looking at.
8. **`no-usable-boundary` is not a finding.** A rung where the class does not
   appear is reported but does not fail the run, because for several classes it
   is the physically correct answer.
9. **Sites are seed-42 coordinates baked into the script**, refined with
   `--scout`. If the planet is re-baked they must be re-scouted; the script does
   not detect that.
