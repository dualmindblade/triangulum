# D-1 — B-20: the reels now assert their own premise

**Deliverable:** `viewer/scripts/world_reel.py` (premise engine, four
re-aimed poses, one added pose, `lint_grid` water-mask fix) plus
`docs/missions/D1-evidence/`.

**Gates on this tree:** suites **8/8 PASS**. World reel **before** my change:
exit 0, 29/29 clean (`D1-evidence/reel-before.txt`). World reel **after**:
exit 3 with exactly four CHANGED poses, all four being the poses I re-aimed
(`D1-evidence/reel-after.txt`). Perf reel deliberately not run — A-1 owns it
this shift. Nothing was `--accept`ed.

---

## The short version

1. **B-20 undercounted.** The world reel had **four** dead poses, not two.
   `frozen_lake` and `frozen_lake_summer` (51.180 85.830) are a dry green
   plain at 373-409 m with no water and no ice anywhere in frame. I did not
   find them by looking — the premise check found them on its first run,
   which is the whole argument for the premise check.
2. **The terracing rings are not a bug.** They are the 1 m voxel column tops
   on a 2.78° slope, 20.6 m apart. Proved with a `voxels off` A/B: the same
   pose with the voxel near-field disabled renders the identical terrain
   with no rings at all. **Not** the F-5 shading bug returning. No `BUGS.md`
   entry filed; the spec's "the reel certified an artefact as correct" is
   the one thing in it I have to contradict.
3. **The other tables are mostly fine — with one real casualty.**
   `mountain_mid` (47.861 14.399) is a **lakeshore at 3 m elevation**, and
   it is in *two* tables. `convergence_reel`'s `b16_ice_hold` — the one the
   spec told me to flag rather than touch — is **sound**. `sky_reel` already
   had a premise check and nobody had noticed.
4. **`lint_grid`'s water mask was worse than reported.** Recall on
   probe-confirmed water went 0.813 → 0.975 with the fix, and three poses
   went from seeing **0%** of their own water to >96%.

**Human verdicts needed (declared, per PROCESS §2):** four fresh blessed
frames; one naming decision; one art-direction park for Andrew. §7 below.

---

## (b) The systemic fix — what invariant, and where it lives

> **A reel pose named for a world class must contain that class, and that
> is checked automatically before a single pixel is compared — never by a
> human reading the pose's name.**

### The measurement

A uniform grid in **screen** space (32 × 18 = 576 cells) is cast through the
pose's own camera onto the local ground datum, and every hit is `probe`d.
Uniform-in-screen-space is the load-bearing choice: it makes every fraction
the check reports **a fraction of the frame**. `water>=0.05` means "at least
5% of the frame is over a water surface", which is directly what "the pose
contains the class" ought to mean, and it needs no arbitrary radius.

15,552 probes for the whole 28-pose Neisor table in **2.7 s**.

Deliberate approximations, stated in the code so nobody mistakes this for a
renderer: the ground is a sphere at the camera's datum, so **relief and
occlusion are ignored**. That makes the check *optimistic* — it can pass a
pose whose class is hidden behind a ridge, but it cannot pass one whose
class is **absent**, and absence is the B-20 failure mode.

### Where it lives, and what I did not build

- **The premise is a seventh column in the pose table**, next to the
  coordinates it constrains. A pose and its premise cannot drift apart if
  they are on the same line.
- **The evaluator is ~20 lines in `world_reel.py`.** Syntax is
  `term OP value` joined by `&`, `OP` ∈ {`>=`, `<=`}. The premise string is
  printed verbatim in the failure message alongside the measured value, so
  the error explains itself.
- **No `reel_common.py`.** V-1 recommended deferring one until a fifth reel
  exists, and I agree; the ~40 lines of sphere trig are duplicated from
  `altitude_ladder.py` with a comment saying so and saying to extract it if
  a third caller appears. A premature shared module that every reel imports
  is a worse outcome than forty duplicated lines.
- **No opt-out flag.** The spec warned that "a brittle assertion layer
  people learn to disable is worse than none", so there is deliberately no
  `--skip-premise`. The check runs first on every invocation except
  `--accept`, and a failure aborts **before rendering** — a reel whose poses
  are not pointed at their classes is not measuring anything, so comparing
  its pixels would only manufacture confidence.
- **`None` is a legal premise, and it must carry a reason.** Three poses
  have one: `ecotone_corner` (the class *is* the köppen edge) and the two
  moon poses (`probe` samples Neisor). An honest, visible hole beats a fake
  assertion. The summary line reports the count every run:
  `27/30 poses proved their own class; 3 carry no probe-expressible premise`.

### `--check-pose` — the workflow half, and the thing I actually used

```
python scripts/world_reel.py --check-pose NAME,LAT,LON,ALT,YAW,PITCH,PREMISE[,SEASON]
```

Answers "does this candidate contain the class I want?" in ~1 s, **before**
the pose is added to the table and rendered. That ordering is the entire
lesson of B-20, and it is how I found every coordinate below. It is also the
demonstration, because the same command run against the old coordinates
fires and against the new ones passes.

### The demonstration — the check firing on known-dead poses

`D1-evidence/premise-fires-on-dead-poses.txt`, one command, exit 3:

```
lake_archi_OLD          sky 0.000  water 1.000  land 0.000  lake 1.000
    FAILED  : land>=0.10  [measured land=0.0]
coast_beach_OLD         sky 0.056  water 0.000  land 0.944   h 896.9..991.9 m
    FAILED  : water>=0.05  [measured water=0.0]
    FAILED  : h_min<=5     [measured h_min=896.9]
frozen_lake_OLD         sky 0.000  water 0.000  land 1.000  frozen 0.000
    FAILED  : water>=0.10  [measured water=0.0]
    FAILED  : frozen>=0.05 [measured frozen=0.0]
frozen_lake_summer_OLD  water 0.000
    FAILED  : water>=0.10  [measured water=0.0]

lake_archi_NEW          water 0.495  land 0.505  lake 0.495          PASS
coast_beach_NEW         water 0.240  land 0.705  h -2.1..2.0 m       PASS
frozen_lake_NEW         water 0.729  land 0.271  frozen 1.000        PASS
frozen_lake_summer_NEW  water 0.729  frozen 0.000 (thawed)           PASS
```

**The before/after that matters most is the reel's own exit code.** With the
four dead poses in it the reel printed `all 29 poses clean` and exited **0**
(`reel-before.txt`). That is the escaped gate, reproduced on demand.

---

## (a) The re-aimed poses

Every coordinate below was found with `--check-pose` and confirmed by probe
*before* anything was rendered. Frames in `D1-evidence/*_BEFORE.png` /
`*_AFTER.png`.

| pose | was | is now | why |
|---|---|---|---|
| `lake_archi` | 16.601 −12.222 @3.0 | **15.4128 −12.570 @3.0** | old: 100% open water, dead centre of a ~130 km lake. New: the lake's southern island field — 49.5% water / 50.5% land, a strait, islands, a sand spit. Genuinely archipelagic. |
| `coast_beach` | 8.097 −35.673 @0.06 | **15.104945 150.918258 @0.060, yaw 58** | old: dry highland grassland, h 896-992 m. New: V-2's probe-verified ocean waterline (h −1.1 m), camera 0.3 km inland looking seaward. 24% water / 70.5% land: sand, turquoise shoal, open sea. |
| `frozen_lake` | 51.180 85.830 @0.3 | **69.9560 50.9800 @0.3** | old: dry plain, 373-409 m, no water, no ice. New: a real frozen lake — 72.9% water, `frozen` 100%, −4.4 °C, ice sheet with a snowy shore. |
| `frozen_lake_summer` | same as above | **same new site** | and it genuinely **thaws** at season 0.55 (`frozen` 0.000, −2.09 °C, open water + sand shore), so the seasonal pair is now a real contrast instead of two identical dead frames. |

How the lake site was found, since it is reusable: a coarse `probe` map
(`~5,000 points`, seconds) over ±1.5° showed the lake's shoreline; a 0.01°
zoom over the southern shore ranked cells by land↔water flip density; the
top three became `--check-pose` candidates; three were rendered and the best
composition won. The whole search cost under ten minutes of wall clock.

**Assumption I checked rather than trusted:** the spec flagged "both dead
poses are best *re-aimed* rather than *replaced*" as unverified. Re-aiming
was right for all four — each class exists elsewhere on Neisor and keeping
the pose name preserves the reel's class coverage. The spec's other marked
assumption, that nobody has revisited whether 29 poses is the right size:
I did not revisit it either. It is now 30, and my honest view is that the
size is not the problem — the coverage claim was. Four of 29 poses guarding
nothing (14%) is a coverage problem no head-count fixes.

### `gentle_terrace` — a new pose, no baseline

Re-aiming `coast_beach` would have silently deleted the reel's only coverage
of gentle-slope terracing, which is the exact visual class of **F-5**
(*"Concentric terrace-ring shading bands on gentle slopes"*, fixed
2026-07-08). So the old coordinates stay in the table under an honest name,
with premise `land>=0.90 & h_min>=500`. It has **no accepted baseline**,
exactly like the two moon poses — the reel prints `no-baseline` and does not
flag it. Bless it or delete it; both are fine, but it should not stay a lie.

---

## (a2) The terracing rings — verdict: **not a bug**

`D1-evidence/terracing-verdict.txt`, `terracing_voxels_ON.png`,
`terracing_voxels_OFF.png`.

A 200-point transect, 2 m apart, across the rings:

```
float sampler h (m): 923.00 923.10 923.30 923.40 923.50 923.60 923.70 ...
integer column top : 923    923    923    923    923    923    923    ...
```

- the sampler height field is **smooth** — 141 distinct values in 201 probes
  at the trace's 0.1 m print resolution, mean step 0.097 m per 2 m
- the **integer column tops** step by exactly 1 m, twenty times over 400 m
- slope 48.5 m/km = **2.78°** → a 1 m contour every **20.6 m** horizontally,
  which is the ring spacing in the frame

And the decisive A/B: the same pose with `voxels off` (the pure heightfield
mesh) renders the identical terrain **with no rings whatsoever**. The rings
are geometry, not shading, and they are the documented voxel lattice —
`viewer/CLAUDE.md`: *"Each column top is a single 1 m quad."*

So: **the reel did not certify an artefact as correct.** It blessed the
honest voxel look on a gentle slope. I am contradicting the mission spec
here on purpose; see §8.

**Parked for Andrew, not blocking:** whether 1 m risers 20.6 m apart, seen
from 60 m up, read as terrain or as a contour map is an art call, not an
engineering one. `gentle_terrace_NEW.png` is the frame for his eyes. His
stated bar in `DECISIONS.md` D-14 is *"small/slanted terraces fine, no big
cliffs on smooth terrain"* — 1 m risers are small by that bar, so my guess
is he passes it. Not filed in `DECISIONS.md`; it is his call whether it is
even a question.

---

## (c) Per-table audit — full detail in `D1-evidence/pose-table-audit.txt`

The handoff's suspicion ("the other reels' pose tables are probably as
rotten as the world reel's, medium-high confidence") is **partly right and
mostly wrong**. Treated as a hypothesis; here is what it measured.

| table | verdict |
|---|---|
| `perf_reel.py` (8) | **one dead pose**, one implicit-orientation pose, one misnamed-but-functional |
| `convergence_reel.py` (4) | same dead pose (shared coordinates); **`b16_ice_hold` is sound** |
| `sky_reel.py` (5) | **sound — and it already had a premise check** |
| `convergence_flight.py` (3) | sound; not a pose table |
| `altitude_ladder.py` (6) | 5 sound; 1 (`snowline`) suspect, see below |

**The one real casualty — `mountain_mid` (47.861 14.399 @0.592):**

```
land 0.616   LAKE 0.384   h -1.0 .. 11.6 m   annual 1.24 C
probe at the camera: h=2.9 m, adjacent lake surface 1.6 m
```

It is a **lakeshore at 3 m elevation**, 38% open water. The world reel calls
the *same coordinates* `taiga`; `perf_reel` and `convergence_reel` call them
`mountain_mid`. At most one of those names could be right and neither is —
the rendered frame is a treeless green plain meeting a large body of water
with a sand beach. Consequence: the perf reel's build-heavy mountain cost
profile is measuring a flat shoreline, so that cost class is covered by
nothing. **I did not re-aim it** — it is Lane A/perf territory, A-1 owns the
perf gate this shift, and re-aiming it invalidates a blessed perf baseline.
Filed as **B-23**.

**`taiga` in the world reel is the same problem wearing a different hat.**
It is climatically boreal (1.24 °C), so the temperature premise I gave it
**passes** — on a frame that shows no taiga at all. That is the honest limit
of a köppen-free check and I have written it into the pose table as a
comment so the next reader does not mistake its green for coverage. Left
blessed and flagged rather than re-aimed, because deciding whether the reel
wants a taiga pose or a second coast pose is a coverage judgement, not a
tooling one.

**`sky_reel` deserves calling out.** Its `lint()` is already a per-pose
presence check — sun disc ≥ 80 px above 250, moon disc ≥ 60 px above 120,
stars ≥ 40 points. That is precisely the invariant this mission was asked to
invent, done in pixel space, and it has been there the whole time. Its sun
geometry is also self-consistent (checked by hand: 0°, 91°, 95°, 177°, 177°
sun-camera separations for zenith / sunset / dusk / two night poses).

**`altitude_ladder`'s `snowline` — suspect, not condemned.** The SITES
comment calls it a *"tropical 3.2 km massif"*. The target point is at
**h = 361.9 m, `frozen` = false**, and a 120-probe transect finds the
nearest frozen ground **12.1 km away at 1453 m** — far outside what the
0.05/0.2/1 km rungs can frame. **What I could not verify:** whether the
renderer paints a visible snow line by elevation independently of the
sampler's `frozen` flag. V-2 reported this site reading at all seven rungs,
which suggests it does, in which case only the comment is wrong. I am not
going to call a site dead on a flag that may not drive the pixels. Filed as
**B-24** at low priority with exactly that caveat.

---

## (d) `lint_grid`'s water mask

`D1-evidence/lint_grid-shoal-calibration.txt` and `lint_grid-shoal-mask.png`.

```
old:  (b > g + 8) & (b > 80)
new:  ((b > g + 8) | (b > r + 20)) & (b > 80)
```

Calibrated against **`probe` truth**, not by eye: each 40×40 px screen cell
of the premise grid is labelled water/not-water by the probe under it, over
all 28 Neisor reel frames — 15,168 judged cells.

| | precision | recall | fn |
|---|---|---|---|
| old | 0.9764 | **0.8133** | 941 |
| new | 0.9787 | **0.9752** | 125 |

**Zero cells that the old mask saw are dropped** — the new mask is a strict
superset by construction. 816 newly-seen water cells, mean rgb
`(124, 158, 164)`. Per-pose, three poses were seeing **none** of their own
water:

```
coast_beach          0.000 -> 0.978    (138 water cells)
taiga                0.000 -> 0.964    (221)
frozen_lake_summer   0.000 -> 0.998    (420)
orbital_coast        0.792 -> 0.951    (288)
lake_archi           0.975 -> 0.982    (285)
groves               0.800 -> 0.822    (45)
```

**A correction to V-2's report, and a near-miss of my own.** V-2 proposed
`b > r + 20` and described the missed class as turquoise shoal (~90,180,180).
That is right but incomplete — the pale water of a cold coast (~140,162,170)
and bright river water (~204,217,225) are missed too. I first "fixed" this
with a cyan-only predicate, having convinced myself from pixel statistics
that `taiga`'s large flat pale-blue region was atmospheric haze. **It was
the lake.** The probe-truth calibration caught my error; eyeballing colour
histograms had not. That is the argument for calibrating a screen-space
heuristic against sampler truth rather than against intuition, and it is
now cheap to do.

`grid_score` remains informational, so this changes no pass/fail — it makes
the informational number stop being computed over 19% of the water.

---

## Escaped-gate analysis (PROCESS §8)

> Which gate should have caught this, and why didn't it?

**Nothing verified that a gate was pointed at what it claimed to guard.**
Every reel checked *change* (pixels vs a blessed baseline) and some checked
*defects* (VOID/GRID lint). Neither notices that a frame contains nothing:
a pose showing 100% open water is perfectly stable, lints clean, and
reports green forever. The blessing step was the last line of defence and it
is a human glance at 29 thumbnails — which is exactly the judgement call
this project's process exists to convert into a comparison.

Same shape as `PERF.md` §1 (the perf reel warmed before measuring): the
instrument was correct about the thing it measured and silent about whether
that thing was the right thing.

The generalisation, which I think is the durable finding: **a change
detector cannot detect the absence of a subject.** Any gate built on
"compare against a blessed reference" needs a separate, cheap assertion that
the subject is present. `sky_reel` had worked this out independently and
nobody carried it across.

---

## §7 Declared human verdicts

1. **Four fresh blessed frames — Austin, gate 2.** `lake_archi`,
   `coast_beach`, `frozen_lake`, `frozen_lake_summer`. All four are
   `CHANGED` because I re-aimed them, before/after PNGs are in
   `D1-evidence/`. **Proposed re-bless, per pose:**
   - `lake_archi` — mean Δ 37.8, 51.5% of pixels. New site is a lake
     archipelago with a strait, islands and a sand spit; the old one was a
     featureless blue rectangle (its blessed PNG is 11 KB, which tells the
     story on its own).
   - `coast_beach` — mean Δ 67.8, 96.6%. New site is an actual beach: sand,
     turquoise shoal, ocean, sky. Old one was dry grassland 900 m up.
   - `frozen_lake` — mean Δ 137.9, 100%. New site is an ice sheet with a
     snowy shore at −4.4 °C. Old one had no water at all.
   - `frozen_lake_summer` — mean Δ 101.3, 100%. Same site thawed: open
     water and a sand shore. The seasonal pair now actually contrasts.
   The command is `python scripts/world_reel.py --accept`, and it is
   **yours to run, not mine**. Note it blesses `gentle_terrace` too.
2. **`gentle_terrace` — keep or drop?** New pose, no baseline, holds the
   `coast_beach` coordinates so the F-5 terracing class keeps a guard.
3. **`dossier_ponds` needs renaming — a naming decision.** Its frame
   contains **zero water**; it is a dry sand/grass patchwork. It is a
   legitimate pose for what it shows (V-2 reused it as her `sand_patch`
   site) and its premise now asserts that honestly, but the name promises
   ponds. Renaming loses its blessed frame, so it needs a human.
4. **`taiga` — a coverage judgement.** Passes its premise, shows a coast.
   Does the reel want a taiga pose re-aimed to real boreal forest, or is a
   second coast pose fine and the name should change?
5. **The terracing frame — Andrew, parked, not blocking.**
   `gentle_terrace_NEW.png`. My engineering verdict is "correct voxel
   behaviour"; whether it *looks* right is his.

---

## §8 What the spec got wrong

- **"`coast_beach`'s blessed frame shows visible terracing contour rings.
  The reel certified an artefact as correct by blessing it."** The rings
  are real and visible, but they are not an artefact — see §(a2). This one
  mattered: it was stated as fact in both the spec and `BUGS.md` B-20, and
  it primed me to go looking for a bug. What saved it was that the spec
  *also* said "decide whether they are a real artefact", which is the right
  framing, and the `voxels off` A/B settles it in one run.
- **"2 of 29 poses — 7% of the world reel — guarding nothing."** It was
  4 of 29 (14%). Both extra poses are the same site in two seasons.
- **"Assert on what `probe` returns now… are enough for every pose in the
  world reel."** Not quite: `ecotone_corner`'s class *is* a köppen edge and
  there is nothing else to assert about it, and `desert` / `savanna` /
  `jungle` / `groves` / `taiga` can only be partially pinned. Five poses
  carry partial premises and one carries none. This is exactly the
  follow-up the spec invited me to name, so it is a small miss, but "enough
  for every pose" was too strong.
- **Everything else in the spec held up**, including the two claims marked
  VERIFIED (`probe` has no köppen; the world reel ran 29/29 clean with both
  dead poses in it — I reproduced the latter myself, `reel-before.txt`).

---

## Feedback

### Bugs, smells and traps found on the way

1. **`mountain_mid` is a lakeshore, in two tables.** B-23. The perf gate has
   no mountain scenario and has not had one for as long as that pose has
   existed.
2. **`perf_reel`'s `ascent_sweep` has no `look` line.** Its orientation is
   the harness default, which I measured from a `state` sidecar as **yaw 0,
   pitch 0** — a dead-horizon shot, ~50% sky. Probably fine for a streaming
   benchmark, but it means a future change to the camera default silently
   re-poses a blessed perf scenario. Every other scenario in that file sets
   `look` explicitly.
3. **`perf_reel`'s `ocean_storm` is a lake**, not ocean (`lake=true`,
   surface 191-216 m — it is the world reel's `inland_sea`). Cosmetic; the
   workload is right. Mentioned only because it is a third naming slip in
   one small file, and three similar reports promote a cosmetic issue
   (PROCESS §4).
4. **`where.py`'s `frozen-lake` class does not agree with the viewer's
   sampler.** All 5 cells it lists are high-altitude ground: I mapped ±7.5 km
   around `-10.177 107.316` and found **0% water** and 100% `frozen`, at
   2284-2930 m. `where.py` exists precisely so "a weak operator reaches a
   frozen lake on the first try", and it does not. Its sibling class
   `liquid-lake` reports **0 cells** planet-wide, which is F-6's bug
   re-appearing in a different class. Meanwhile a 41,316-point probe sweep
   found 10 genuinely frozen lake cells in ~30 s. Worth a small mission:
   regenerate `feature-coords.md` from probe truth instead of
   `planet_data.npz`, or reconcile the two definitions.
5. **`ground_height_km` returns the water surface, not the terrain** —
   V-2 reported this and it is still true and still undocumented. It cost me
   a wrong turn: I built a two-pass probe (camera ground first, then the
   footprint) before working out that the datum cancels and the pass was
   unnecessary. Worth one line in `viewer/CLAUDE.md`.
6. **Copying `viewer/assets/` across worktrees dirties a tracked file.**
   `weather_tuning.json` is tracked (only `*.bin` and `meta.json` are
   gitignored) and the copy rewrote its line endings, so it shows as
   modified with a zero-line diff forever. Every worktree mission will hit
   this. Either gitignore it too, or add a `.gitattributes` line, or tell
   dispatched agents to expect it.
7. **`interchange/reel/accepted/lake_archi.png` is 11 KB and every other
   blessed frame is 300 KB-1 MB.** A featureless frame compresses to nothing.
   A one-line file-size outlier check over the blessed directory would have
   flagged this pose for free, years before anyone thought about premises.
   Cheap enough that it is probably worth adding to a bless step.

### What cost me time

- **Trusting pixel statistics over sampler truth, for about 25 minutes.** I
  looked at `taiga`'s water — a large flat region at rgb (140,162,170) —
  decided it was atmospheric haze because it was uniform, and built a
  cyan-only mask specifically to exclude it. It was the lake. I only caught
  it because the probe-truth calibration disagreed with me. The general
  lesson is the same one V-2 hit from the other direction: **screen-space
  heuristics on this renderer must be calibrated against `probe`, not
  against intuition about colour**, and now that the footprint machinery
  exists that costs seconds.
- **Two-pass probe geometry I did not need** (see 5 above).
- Nothing else, honestly. The mission was well scoped and the tools worked.

### What tooling would have made this faster

- **A `probe` that returns köppen** — which D-2 is adding. Six world-reel
  poses have partial or absent premises purely because of it. When it lands,
  `ecotone_corner` gets a real premise and `savanna`/`jungle`/`desert`/
  `groves`/`taiga` get complete ones. `taiga` in particular would then fail,
  which is the point.
- **A moon-side `probe`.** The two moon poses have no expressible premise at
  all, and their MOON lint (surface fraction, surface stddev) would pass on
  the wrong moon entirely.
- **The region-scout I wrote is genuinely reusable and I did not ship it.**
  A coarse `probe` map over a lat/lon box, printed as ASCII with a
  land/water/lake/ice legend, plus a ranking by shoreline complexity. It is
  how I found all four new sites and it took maybe 40 lines. I left it in
  the scratchpad because `altitude_ladder.py` already has a `--scout` and
  the spec was explicit about not building shared infrastructure as a side
  effect. If someone wants it, it is worth ~30 minutes to fold into
  `where.py`, which is where an operator would look for it.

### Assumptions I made because the spec did not say

- **Thresholds are calibrated with margin, not set at the observed value.**
  `dossier_shore` really does contain only 2.8% water, so its premise is
  `water>=0.01` — a 2.8× margin. Every threshold is roughly 2-5× below what
  the healthy pose measures. A premise is meant to catch a pose that has
  *left* its feature, not to police small terrain changes; the pixel diff
  already does that. If someone wants premises to be tight tolerances
  instead, that is a different (and I think worse) instrument.
- **A failed premise aborts before rendering.** The alternative — render
  anyway and report both — is defensible, and I can be argued out of this.
  I chose the abort because it is louder and because pixel-diffing a dead
  pose produces a number that looks like information.
- **Exit code 3, not a new one.** A premise failure is more serious than a
  changed pose, but adding a fourth exit code to a convention shared by five
  scripts seemed worse than the loud banner it already prints.
- **`--check-pose` defaults to season 0.45**, the reel's own, with an
  optional 8th field.
- **I re-aimed `frozen_lake` even though the spec only named two poses.**
  With the premise check landed, leaving it dead would have left the reel
  permanently red, and the fix was the same ten minutes as the other two.
- **I added `gentle_terrace` rather than deleting the coordinates.** Adding
  a pose without blessing it is an established pattern here (the moon poses
  say so explicitly), and the reel treats a missing baseline as
  `no-baseline`, not a finding — so it costs one render and flags nothing.

### Surprises

- **`probe` is absurdly cheap.** 41,316 probes in ~30 s including process
  start, and 15,552 in 2.7 s. V-2 said so and I still under-budgeted it.
  This changes what is worth building: an exhaustive planet sweep is a
  30-second operation, and I used one to find frozen lakes rather than
  reasoning about where they ought to be. Whatever documents `probe` should
  lead with the throughput number.
- **`sky_reel` already solved this problem.** The mission was framed as
  "nothing in this repo verifies that a gate is pointed at what it claims to
  guard", and one reel does, and has for a while. Worth a look before the
  next "nothing here does X" premise — the answer was 60 lines away in a
  sibling file.
- **The re-aimed `coast_beach` renders exactly the turquoise shoal water
  that part (d) is about.** The reel gained real coverage of the class its
  own lint had been blind to, by accident, in the same commit.

### On the spec's instruction not to prescribe an algorithm

It worked. "The observation I want is: *a pose named for a class contains
that class*" is what I built against, and the screen-uniform footprint grid
is not something I would have arrived at if the spec had proposed sampling
a radius around the aim point — which is the obvious mechanism and is
strictly worse (it needs an arbitrary radius and its fractions mean
nothing). V-2 said the same thing from the other side. Keep doing it.
