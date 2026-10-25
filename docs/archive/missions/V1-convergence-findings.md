# V-1: The convergence reel — findings

Mission: turn "does the live view converge, and how fast?" from a human
judgment call into a measured, gateable number. Tool:
`viewer/scripts/convergence_reel.py`.

## What was built and how to run it

```
cd viewer
python scripts/convergence_reel.py               # render + measure + gate
python scripts/convergence_reel.py --accept      # bless current numbers
python scripts/convergence_reel.py --skip-render # re-measure existing frames
```

Exit 0 clean, 3 findings to review — same contract as world_reel /
perf_reel. Report: `viewer/interchange/convreel/current.json` (blessed
baseline: `accepted.json`). Frames for human review:
`viewer/interchange/runs/conv/<pose>/{ref,live}/sNNNN.png` (NNNN =
sim-deciseconds).

**Mechanism — and the part that surprised me: it needed zero harness
changes.** The instrument is pure composition of two mechanisms that
already existed:

- For each pose one script is generated (`scripts/_conv_<pose>.play`):
  a `posehz 10` line, 300 identical `pose` lines (30 s of sim), with
  `shot sNNNN` sampling lines interleaved on a schedule that is dense
  early and sparse late.
- **Ref phase** (plain run): `pose` lines advance the sim clock without
  drawing; every `shot` is the existing settled capture — ground truth
  at that exact sim time. Settled captures are byte-deterministic by
  design (world reel relies on this).
- **Live phase** (`TRI_POSE_LIVE=1 TRI_RAW_CAPTURE=1`): pose lines draw
  their full 60 Hz tick cadence of live frames from a cold streaming
  state — the exact mechanism that reproduced B-16 — and
  `TRI_RAW_CAPTURE` turns every `shot` into a raw capture of the
  in-flight frame (one draw, no settle, no drain).

Both phases advance `sim_ticks` identically (pose = 6 ticks, shot = 0),
so every comparison is live(t) vs settled(t) at the **same sim time**:
water animation, sun motion and weather cancel exactly. The proof this
works: converged poses reach mean delta **0.000 — byte-identical** to
the settled reference (see table). There is no golden image anywhere;
the reference is derived per run. That is the self-blessing property —
the gate worked on day one against the existing codebase and cannot go
stale.

Per pose the report carries: the full delta curve, `t_converge_s`
(first sample from which every delta stays <= 1.0 mean — world_reel's
own "changed" threshold), `residual_first` / `residual_final` (late
checkpoint at 30 s), `progress` (final/first — a stuck pose stays near
1.0), per-sample live draw counts vs the settled draw count (the
geometry-level view the B-16 hunts tracked), per-sample wall clock, and
`sim_per_wall` (how fast the live phase ran vs real time).

Gates:
- **STUCK** (day one, no baseline): (`residual_final > 3.0` OR
  `changed_frac_final > 0.04`) AND `progress > 0.80` — still far from
  settled with no meaningful improvement across the whole window. This
  is the pre-fix B-16 signature (flat curve at residual ~3.4 / 7.9%
  changed pixels / progress ~1.0). The frac trigger exists because
  low-contrast ice blur keeps the mean small while many pixels stay
  wrong. Post-fix b16 passes on the progress gate (0.57–0.64: it IS
  draining), which is the correct current truth.
- **Baseline drift** (after `--accept`): `t_converge` worse than
  `acc*2.0 + 1.0 s`, `residual_final` worse than `acc*1.8 + 2.0`, or a
  pose that converged at baseline no longer converging in-window.
  Tolerances are derived from the measured two-run noise below.

## The numbers (criterion 2: the instrument separates the poses)

Run 1, dev machine (RTX 2060 class), whole reel 4m19s wall:

| pose | t_converge | res @0.5s | res @30s | progress | sim/wall |
|---|---|---|---|---|---|
| b16_ice_hold (41.519 122.511 @3.05, 93/-68) | **>30 s** | 3.42 | **2.19** | 0.64 | 0.73 |
| forest_ground (-0.906 -67.804 @0.094) | **4.0 s** | 5.87 | 0.000 | 0.00 | 0.58 |
| mountain_mid (47.861 14.399 @0.592) | 0.5 s | 0.89 | 0.000 | 0.00 | 0.54 |
| plains_walk (19.674 35.265 @0.010) | 0.5 s | 0.55 | 0.000 | 0.00 | 0.57 |

Run 2 (immediately after, cache-warm OS, 4m16s wall):

| pose | t_converge | res @0.5s | res @30s | progress | sim/wall |
|---|---|---|---|---|---|
| b16_ice_hold | **>30 s** | 3.42 | **1.96** | 0.57 | 0.73 |
| forest_ground | **4.0 s** | 5.87 | 0.000 | 0.00 | 0.57 |
| mountain_mid | 0.5 s | 0.89 | 0.000 | 0.00 | 0.54 |
| plains_walk | 0.5 s | 0.55 | 0.000 | 0.00 | 0.61 |

The separation is not subtle. The B-16 hold pose never converges inside
the window and holds a large residual (its curve is nearly flat: 3.42 →
3.36 across 0.5–25 s, first real movement only at 30 s); every healthy
pose decays to **exactly 0.000**. Draw counts corroborate at the
geometry level: live draws at the b16 pose crawl 1141 → 1432 over 30 s
of sim against 1543 settled — the post-fix "draining at build
throughput" tail, precisely as HUNT 4 recorded (wanted ~1540). The
forest pose measures 4–5 s to convergence, which is the b4a2-v2
"convergence trade" documented in PERF.md item 1 ("live frames now
converge to full detail over ~5-10 s at a new pose") — the reel turns
that prose promise into a standing number.

## Determinism / noise (constraint 1)

**Finding: wall-clock leaks into the live curves, exactly as suspected.
Sim time is the honest x-axis, but it is not sufficient for
determinism.** Tile/chunk builds run on rayon workers in real time;
which builds have landed by sim-sample t depends on machine speed and
load. Evidence and quantification:

- **Ref phase is byte-deterministic**: all 45 ref frame hashes for
  forest_ground / mountain_mid / plains_walk are identical between run
  1 and run 2 (45/45). (b16's run-1 ref hashes were accidentally
  captured after run 2 had begun rewriting them, so they are excluded
  from the hash claim; its metric comparison below stands in, and
  settled captures are the world reel's own byte-stable path.)
- **Live phase is NOT frame-deterministic — but only before
  convergence.** 6 of 45 healthy-pose live frames differ between runs,
  and they are exactly the pre-/at-convergence samples
  (forest s0005/s0020/s0030/s0040, mountain s0010, plains s0040);
  every post-convergence live frame is byte-identical. The metrics are
  correspondingly stable across the two full renders:

  | pose | t_conv r1 vs r2 | res_end r1 vs r2 | max per-sample curve delta |
  |---|---|---|---|
  | b16_ice_hold | >30 vs >30 | 2.191 vs 1.964 | 0.227 |
  | forest_ground | 4.0 vs 4.0 | 0.000 vs 0.000 | 0.009 |
  | mountain_mid | 0.5 vs 0.5 | 0.000 vs 0.000 | 0.001 |
  | plains_walk | 0.5 vs 0.5 | 0.000 vs 0.000 | 0.000 |
- **Sim runs slower than wall here, which biases the numbers mildly
  optimistic.** I originally expected the opposite. Measured
  `sim_per_wall` is 0.54–0.73 at these poses (draws cost more than
  16.7 ms on the dev GPU, and there is no vsync): background builders
  get 1.3–2x more wall time per sim second than a real 60 Hz player
  gets, so real gameplay convergence is somewhat *slower* than the reel
  reports. The separation this gate exists for is orders of magnitude,
  so the bias does not threaten it — but absolute numbers should be
  read as lower bounds, and cross-machine comparisons of `current.json`
  are not meaningful. The per-pose `sim_per_wall` is in the report so a
  reviewer can see the skew for any run.

Threshold consequences: `t_converge` did not move at all between runs
(grid resolution 0.5 s early); residuals on converged poses are exact
zeros (no noise floor at all — the same-sim-time comparison eliminated
it); the slow pose's residual swings ~10% with build timing (2.19 vs
1.96) and its progress ~0.07 (0.64 vs 0.57, comfortably under the 0.80
STUCK gate). The baseline tolerances (t_conv 2.0x + 1 s, residual
1.8x + 2.0) sit far outside everything observed while remaining far
inside the failure signal (a pre-fix B-16 shows progress ~1.0 and a
flat residual ~3.4 vs the exact 0.0 of a healthy pose). Two full
renders agreed on every gateable number; the reel exited 0 both times,
and 0 again after `--accept` with the baseline comparison active.

## Escaped-gate analysis: would this have caught B-16?

**Partially — and honestly, only after HUNT 1 had already produced the
key ingredient.**

- **The original in-game B-16 (accumulated-session starvation): no.**
  The bug ledger is explicit that the cold-state harness repro at the
  photo pose was byte-stable in 3 s (b16-repro2.play found nothing);
  the starvation needed the streaming state a long flight builds up.
  My reel starts every pose cold, so the *specific* trigger Austin and
  Andrew hit would have slipped past this pose table too. What WOULD
  have caught it before humans: running this same instrument over a
  recorded flight (the b16-live.play pattern — pose replay is already
  the reel's substrate, so the extension is natural: sample live frames
  along the recording, settle refs at chosen hold points). That needs
  the F10 recording to exist, which is exactly why the ledger asks for
  recordings that start a minute early.
- **The B-16 class (admission starvation at a heavy pose): yes.** The
  cold-start hold pose IS in the current table, and a pre-fix build
  would have shown the signature this reel flags with no baseline:
  draws frozen (the ledger records them stuck at exactly 1135 for 8+
  seconds) → flat delta curve → progress ~1.0 with residual ~3.4 →
  STUCK finding, exit 3. Post-fix, the same pose shows progress 0.64
  and a climbing draw count, which correctly does NOT flag — the
  remaining tail is the B-4c build-cost campaign, and the blessed
  baseline will hold the line on it (`residual_final` regressing past
  ~6 or `t_converge` appearing/vanishing flags).
- **B-4a v1 (shipped and reverted same day): yes, at the forest pose.**
  The v1 regression was cold/first-touch cost over productive forest
  that the warm-before-measure perf reel hid. forest_ground cold
  convergence is now a standing number (4.0 s); v1's multi-second
  first-touch walls would have pushed `t_converge` and the early curve
  far past the blessed tolerance. This closes the exact blindness the
  PERF.md post-mortem names ("probes ran barren poses; the perf reel
  warms before measuring").
- **B-12 (settle blur) / B-11 (altitude catch-up): yes in class** — both
  manifest as the early curve staying high for seconds at a static or
  just-teleported pose, which is precisely what the dense early samples
  measure. B-18-style dissolve artifacts appear as a persistent nonzero
  residual (live never reaches settled pixels).

## No-regression gates

- `bash viewer/scripts/verify.sh`: rebuild + all 8 asserting suites
  PASS (physics/water/lake/flooded-caves/invariant/auto-survey/
  camera-controls/solar-system).
- `python scripts/world_reel.py` against the blessed accepted reel
  (copied from the main checkout, since interchange/ is unversioned):
  **all 29 poses clean — no flagged pose.** Expected: this mission
  changes zero production or harness code (the tool is one new Python
  script plus generated .play files).
- The convergence baseline was blessed locally (`--accept`,
  interchange/convreel/accepted.json — machine-local like the other
  reels' baselines) and the reel exits 0 against it.

## Cost

Whole reel, 4 poses: **~4.3 minutes wall** on the dev machine (run 1:
4m19s, run 2: 4m16s). Split: ~2.4 min for b16_ice_hold (its ref settle builds ~1540
tiles incl. ~400 heavyweight frozen L11s, and its live phase draws
~1500 tiles per frame for 30 s of sim), ~2 min for the three healthy
poses together. Fits "run on every merge" as-is; if the merge queue
ever needs it faster, the healthy poses could drop their window to 10 s
(they are all byte-exact by 5 s), taking the reel under 3 minutes.

## Honest cut-lines (what I did not do, and why)

- **No recorded-flight (warm-state) convergence lane.** The reel is
  cold-start per pose. The accumulated-session B-16 trigger class is
  therefore still only covered by ad-hoc TRI_POSE_LIVE replays of F10
  recordings, not by a standing gate. It is the natural next extension
  (same sampling mechanism along a recording), but it needs a blessed
  recording artifact checked in, sizeable (50 KB+) and human-chosen —
  a scoping decision I did not want to make unilaterally.
- **No moon poses.** The pose table is Neisor-only; moon streaming has
  its own cost profile (PERF.md moon items). Adding a `moonpose` lane
  is mechanical (the script generator would emit `moonpose` instead of
  `pose`) but unproven, and the mission's specimens were both Neisor.
- **Single window (30 s) for all poses.** A per-pose window would let
  the healthy poses finish at 10 s and give b16 60+ s to show its full
  tail. I kept one window for simplicity of the report and because the
  tail beyond 30 s belongs to the B-4c build-cost campaign, not to this
  gate.
- **`progress` compares first sample to last.** A pose that regresses
  *mid-window* after converging (arrival storms, oscillating selection)
  is caught by `t_converge`'s stays-there rule but not by `progress`;
  if oscillation becomes a live class, add max-over-suffix.
- **No sub-0.5 s samples**: the first sample is at 0.5 s sim, so the
  reel does not resolve differences in the first ~30 frames (teleport
  burst territory, B-12's domain). The dense-early schedule can be
  extended if that class recurs.
- The wall-clock timestamps ride on Python timestamping the harness's
  line-buffered stdout — good to ~tens of ms, plenty for seconds-scale
  curves, but not a profiler.

## Feedback

- **The building blocks were already there, and that is worth
  celebrating in the docs**: `TRI_POSE_LIVE` (from the B-16 hunt) plus
  `TRI_RAW_CAPTURE` (documented only as a footnote in capture_rgba and
  the viewer CLAUDE.md diagnostics list) compose into a complete
  live-frame sampler with zero code changes. Nothing anywhere hints
  that they compose; a line in the play.rs header near the
  TRI_POSE_LIVE comment ("combine with TRI_RAW_CAPTURE to photograph
  the live frame mid-stream") would have saved me the renderer.rs
  spelunking that found it, and might have made B-16's hunts faster
  too — HUNT 1 built pose-live replay before discovering it needed
  exactly this pairing.
- **`pose`/`posehz` lines emit no trace lines.** Every other command
  traces. Cost me a real bug: my wall-clock anchor waited for a pose
  trace that never comes. For 500-line recordings silence is right, but
  a single first-pose trace (or a posehz acknowledgment) would give
  scripts a "sim started now" anchor in the transcript. Related: run.log
  lines carry no timestamps at all — even a coarse +wall-seconds prefix
  behind an env var would have made my mtime forensics unnecessary.
- **Mission spec gaps I had to fill by assumption**:
  (a) The spec's fast specimen ("forest pose") is called fast, but the
  codebase calls that exact pose the *slow* cold-build specimen (B-4c:
  "dense-forest cold tile builds still wall ~3.2 s"). It measured 4 s to
  converge — fast relative to b16, slow relative to plains/mountain
  (0.5 s). The reel keeps all four so the table shows the real spread;
  worth knowing the spec's "fast" pose is the second-slowest thing in
  the table. (b) "Whether wall-clock leaks in" — it does, but the spec
  frames it as if sim-time measurement might avoid it; it cannot, and
  saying so up front ("the x-axis is sim time; arrivals are wall-time;
  quantify the damage") would have been clearer. (c) No guidance on
  whether the b16 pose should gate STUCK at its *current post-fix*
  state (I chose: no — it is draining, so it passes STUCK and the
  blessed baseline holds the tail).
- **`weather time 555.111` matters and nothing says so.** The b16 load
  (heavyweight frozen-water tiles) depends on the season implied by the
  weather clock. The hunt scripts pin it; BUGS.md never mentions that
  the repro is season-sensitive. I inherited the hunt's clock for the
  whole reel and documented it in the script; if someone "cleans up"
  that constant to 0, the slow specimen quietly stops being slow.
- **Tooling that would have made this faster**: (1) a way to get
  per-sample wall time from the harness itself (see the trace
  timestamps point); (2) `world_reel.py`-style scripts each reimplement
  the subprocess-run/report/exit-3 scaffold — a tiny shared
  `reel_common.py` would make the fourth reel a 100-line file; (3) the
  generated `_perf_*.play` / `_conv_*.play` files land in `scripts/` as
  permanent untracked clutter (they show in every `git status` on main)
  — either gitignore `viewer/scripts/_*.play` or generate into
  `interchange/`.
- **Surprises worth recording**: (1) converged live frames are
  *byte-identical* to settled ones — I expected an ease/dither noise
  floor and budgeted an epsilon for it; there is none. The epsilon of
  1.0 is therefore pure margin. (2) The live phase runs *slower* than
  real time at heavy poses (sim/wall 0.54–0.73) — I and the mission
  spec both implicitly assumed the harness outruns real time. It makes
  the instrument optimistic, not pessimistic (documented above).
  (3) The b16 pixel curve is a *lagging* indicator of its draw-count
  curve: draws climb steadily 1141 → 1249 through 25 s while pixels
  barely move, then both jump at 30 s — most arriving tiles land where
  the picture was already close; the visually-worst gaps fill late.
  Consistent with HUNT 4's "coarsest-first + visible-gap reserve"
  ordering, and a good reason the gate samples pixels, not tile counts.
- **Assumptions made because the spec did not say**: window length
  (30 s), sample grid (0.5–30 s, 15 samples), eps (1.0 = world_reel's
  CHANGE_MEAN), sun/weather pinning (world_reel's convention + the
  hunt's weather clock), 1280x720 default size, cold start per pose,
  STUCK thresholds (3.0 residual / 0.80 progress), baseline tolerance
  shapes, and that `--accept` blesses numbers (perf_reel-style) rather
  than frames (world_reel-style) — the frames bless themselves here.
