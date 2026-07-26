# V-3 — The recorded-flight convergence lane

**Deliverables**
- `viewer/scripts/convergence_flight.py` — the lane.
- `viewer/scripts/rec001-descent.play` — Austin's F10 descent, promoted out of
  the gitignored `interchange/recordings/` so a gate can depend on it.
- `.gitignore`: `viewer/scripts/_*.play` (all four reels generate scripts there).
- `BUGS.md`: a HUNT 5 entry on B-16 — see §4, this lane found a live one.

**Rust touched: none.** Like V-1, the instrument is pure composition of
mechanisms that already existed. `play.exe` is the stock build of `120995b`.

---

## 1. What it is and how to run it

```
cd viewer
python scripts/convergence_flight.py                # merge tier  (~1 min)
python scripts/convergence_flight.py --tier deep    # + the B-16 session (~4 min)
python scripts/convergence_flight.py --tier all     # + untracked local recordings
python scripts/convergence_flight.py --list --tier all   # holds only, no GPU
python scripts/convergence_flight.py --flight ice_session
python scripts/convergence_flight.py --accept       # bless current numbers
python scripts/convergence_flight.py --skip-render  # re-measure frames on disk
```

Exit 0 clean, 3 findings — the house contract. Report
`viewer/interchange/convflight/current.json`, baseline `accepted.json`,
frames `viewer/interchange/runs/convflight/<flight>/{ref,live}/sNNNN.png`
(NNNN = sim-deciseconds, so a frame name decodes to a point in the flight).

**It is a new file, not a mode of `convergence_reel.py`.** The reel is
untouched; the flight lane `import`s `run_phase`, `shot_draws`,
`CONVERGE_EPS` and every STUCK/drift threshold *from* it, so the two lanes
share one comparison discipline by construction and cannot drift apart. The
cost is a hard dependency: `convergence_flight.py` exits with a clear message
if `convergence_reel.py` is absent, so V-1 must land for V-3 to run. If V-1 is
dropped, ~50 lines have to be vendored back.

### Mechanism (identical discipline to V-1)

One generated script, run twice. `pose` advances `pose_ticks` in both phases,
`shot` advances none, so both phases share one timeline:

| phase | env | what `pose` does | what `shot` does |
|---|---|---|---|
| ref | — | advances the sim clock, **no draw** | settles (byte-stable capture) = ground truth at that sim instant |
| live | `TRI_POSE_LIVE=1 TRI_RAW_CAPTURE=1` | draws its full 60 Hz tick cadence, scheduler running as in gameplay | photographs the raw in-flight frame |

Every comparison is `live(t)` vs `settled(t)` at the **same sim instant** —
water animation, sun motion and weather cancel exactly. Self-blessing, no
golden images. The proof it still holds with a moving camera: every healthy
sample in this mission's runs reaches mean delta **exactly 0.000** (the two
orbital holds, all three cruise samples, and `descent_land` from 8 s on).

### What is different from the reel

The pose lines *vary*. Samples go where a player would expect the picture to
resolve:

- **Hold points are detected from the recording**, not annotated: a maximal
  run of pose lines that never leaves a 2 m / 0.05° ball around the run's
  first pose, lasting ≥ 1.5 s. That threshold is far below anything a hand on
  a mouse produces and far above recording quantisation; it found every hold
  in all four available recordings on the first try (`--list` prints them).
  Nothing has to be annotated, so **any** future F10 recording is a candidate
  gate input with zero authoring work.
- **Samples inside a hold are offsets from the moment motion stopped**
  (0.2, 0.5, 1, 2, 4, 8, 16, 30 s, clipped, plus the hold's last instant) —
  that is the axis a player perceives.
- **Cruise samples** on a sparse grid between holds measure how far behind
  the live view runs *while moving*. Reported as `cruise_mean` / `cruise_max`;
  it never reaches zero at a heavy pose, so it is not gated on day one, only
  against a blessed baseline.
- **Tail extension.** If a recording ends inside a hold, the player stopped
  and the tape simply ran out; repeating the final pose keeps the clock
  running so the terminal hold gets a chosen measurement window. This matters
  a lot — see §3.
- **Short holds are measured but not gated** (< 5 samples): a 1.6 s hold
  cannot support a tail trend.

Gates are the reel's, reused literally: `STUCK` = (residual > 3.0 **or**
changed-pixel fraction > 0.04) **and** tail-third/middle-third ratio > 0.97,
baseline-aware so a known-pinned hold does not cry wolf every merge; plus
`t_converge` / `residual_final` / `cruise_mean` drift against `--accept`.

---

## 2. The flight table, and why it is this short

| flight | tier | source | sim | holds |
|---|---|---|---|---|
| `descent_land` | merge | `scripts/rec001-descent.play` | 21.1 s (11.2 recorded + 15 s tail) | 1 ground hold after a 124.8 → 0.26 km drop |
| `ice_session` | deep | `scripts/b16-live.play` | 127.8 s (101.5 recorded + 45 s tail) | 2 orbital, 2 at the B-16 ice pose |
| `rec002_survey` | local | `interchange/recordings/rec_002.play` | 79.5 s | 2 |

**The mission spec is wrong on one point and it constrained the design:**
`viewer/interchange/recordings/rec_001.play`, `rec_002.play` and
`sustained-blur.play` are **not committed**. `viewer/interchange/*` is
gitignored (with two `!` exceptions for tint/v7 PNGs), so those three exist
only in Austin's checkout. The only tracked recording is
`viewer/scripts/b16-live.play`. A standing gate cannot depend on files that a
fresh clone does not have, so I copied `rec_001.play` verbatim to
`viewer/scripts/rec001-descent.play` and committed it — exactly the precedent
`b16-live.play` set (it is `sustained-blur.play` plus a shot tail).
`rec_002.play` stays a `local` tier that is silently skipped when absent.

Two flights is a deliberate floor, not an accident: one cheap healthy control
and one known-hard specimen, which is the same shape V-1 argued for ("if the
instrument reports the same number for both, the instrument is broken").
Adding a recording is a two-line table edit and needs no annotation.

---

## 3. Warm state: the evidence (requirement 2)

Same pose. Same binary (`120995b`, stock). Same machine, same hour, no other
`play.exe` running. The only difference is **how the camera got there**.

| | **cold** — `convergence_reel` `b16_ice_hold` | **warm** — `convergence_flight` `ice_session` hold 3 |
|---|---|---|
| pose | 41.519 122.511 @ 3.05 km, yaw 93 pitch −68 | 41.5189 122.5113 @ 3.0498 km, yaw 92.98 pitch −68.30 |
| how reached | teleport into an empty renderer | 82.8 s of Austin's recorded flight: 6733 km orbit → descent → ground |
| window held | 30 s sim (~42 s wall) | **45 s sim (~60 s wall)** |
| residual first → final | 3.420 → **0.000** | 4.267 → **4.247** |
| changed pixels at end | **0.00 %** | **15.50 %** |
| tail ratio | 0.798 (draining) | **0.998 (pinned)** |
| live draws → settled | 1141 → **1543 / 1543** | 1153 → **1165 / 1540** |
| verdict | clean, **exit 0** | **STUCK, exit 3** |

The warm window is deliberately *longer* than the cold one so the comparison
cannot be dismissed as "you just did not wait as long". `tail_hold_s=45` on
the ice flight exists for exactly that reason.

Inside the warm hold the drawn-tile count moves by **twelve tiles in 45
seconds** (1153 → 1165) and is flatly **frozen at 1165 from offset 16 s to
offset 45 s** — 36 seconds of wall clock, camera stationary, not one new tile
entering the picture. **375 of 1540 wanted tiles never arrive.** The frame is
visibly soft: `runs/convflight/ice_session/live/s1278.png` against
`ref/s1278.png` is the B-16 photograph — a smeared coastline against a crisp
one.

The second ground hold corroborates: hold 2 (same position, 5 s earlier,
different look angle, 4.7 s) is also STUCK — residual 3.10, 9.68 % of pixels
wrong, 1150 / 1540 drawn.

**So the streaming state at a hold point does not merely "differ measurably"
from the same pose reached cold — it inverts the verdict.** The cold
instrument rates this pose fully converged and byte-identical; the warm one
rates it pinned with a tenth of the screen wrong.

### What the warm state actually is (the premise, refined)

The mission's phrasing — "the streaming state that an accumulated session
builds up" — implies a large accumulated cache. **Measurement says that is not
it.** `TRI_STREAM_DEBUG` over the whole flight shows the 50 s orbital cruise
holding only 88–99 wanted tiles with `uncached 0, pending 0`: the cruise
accumulates almost nothing. The distinguishing variable is narrower and more
interesting: **arriving at the pose by descending into it** (a continuous walk
down the LOD ladder, ~15 s from 6733 km to 3 km, the wanted set growing 43 →
297 → 431 uncached as it falls) versus **arriving by teleport into an empty
renderer**. Whatever the cold path gets — an empty cache, a clean burst, no
in-flight ladder — it is enough to converge, and the descent path is not.

I have not isolated which of those it is, and that is a hunt's job, not a
tooling mission's. What the lane hands the next hunt is a **complete repro
packet**, measured over the 45 s hold with `TRI_STREAM_DEBUG=1`:

- `wanted 1540` constant; `uncached 429 → 402` over 2700 live frames — **27
  tiles in 45 seconds, ~0.6/s**, and it *oscillates* (403, 404, 401, 402, 402,
  401, 399, 402) rather than trending down.
- `pending 4–5 (productive 4–5)` for the entire hold — pinned at the admission
  cap (`caps.productive` 3 for `stream_level` 1, plus HUNT 3's 2-slot
  visible-gap reserve). Only five builds are ever in flight; the rest of the
  machine is idle. **This is why a faster machine does not fix it** — a
  hypothesis for whoever is auditing that question, not a claim I proved.
- The head of the uncached list is the same eight keys for the whole hold:
  `f4L11(2002..2005, 1652..1653)`. Those are **the exact eight keys HUNT 3
  reported as "STILL never admit — one more refusing condition remains"**.
  HUNT 4 closed B-16 without that condition being found.
- Zero `STREAMDBG DROP` (no season-bucket mismatches), zero eviction storms.
- The 27 tiles that *did* land never entered the drawn set (draws frozen at
  1174), which is HUNT 1's original conclusion — "arrivals land in cache but
  SELECTION never swaps the viewed area to finer tiles" — still standing in
  warm state.

Reproduce with:
`TRI_POSE_LIVE=1 TRI_RAW_CAPTURE=1 TRI_STREAM_DEBUG=1 play scripts/_convflight_ice_session.play`
(generate the script first with `python scripts/convergence_flight.py --flight ice_session`).

---

## 4. B-16 is not resolved

This is a mission finding, not the deliverable, but it is the most important
thing here. `BUGS.md` and commit `120995b` say "B-16 resolved: scheduling
starvation fixed; residual tail is build cost". On the shipped binary, at the
pose from Austin's own recording, reached the way he reached it:

- the residual does not move at all across 45 s of held sim (2.657 → 2.647,
  tail ratio 0.999);
- the drawn set is frozen for 51 s of wall clock;
- five build slots are in use and ~400 gaps are outstanding.

"Residual tail is build cost" predicts a curve that declines; this one does
not decline, and the build pipeline is not saturated in any hardware sense —
it is saturated at a cap of five. That matches Austin's report that the
symptom still occurs on his more powerful machine.

I have added a HUNT 5 entry to the B-16 block in `BUGS.md` with the repro
above. **B-16 should go back to OPEN-and-active.** I did not attempt a fix:
`renderer.rs` is Lane A and this is a Lane D mission.

---

## 5. Cost, and the merge-time recommendation

Measured on the dev machine (RTX 2060 class), nothing else on the GPU:

| flight | sim | samples (hold + cruise) | ref phase | live phase | **total** | sim/wall |
|---|---|---|---|---|---|---|
| `descent_land` | 21.1 s | 7 + 3 | 15.2 s | 39.5 s | **54.7 s** | 0.56 |
| `ice_session` | 127.8 s | 23 + 3 | 17.9 s | 157.3 s | **175.2 s** | 0.80 |
| `--tier deep` (both) | | | | | **230 s ≈ 3.8 min** | |

The split is not what I expected and it is worth writing down: **the ref phase
is nearly free** (15–19 s including planet load), because `pose` lines do not
draw in it — only the ~25 `shot` settles cost anything, and settles at the
same place are cache hits after the first. **All the cost is the live phase**,
which must draw every tick of the flight at `sim_per_wall` 0.55–0.80. So the
cost of this lane is essentially *the length of the recording divided by 0.7*,
plus ~15 s of load per phase. A 150 s recording is ~4 min; a 10 minute one is
~15 min and does not belong in a gate.

**Recommendation:**
- **Every merge: `--tier merge` — 55 s.** That is `descent_land` only: a
  genuine healthy control (it converges to byte-identical, and it is a
  *descent*, so it guards the B-11 / B-4a class that the cold reel's held
  poses cannot reach) costing less than a minute on top of the reel's 4.3.
- **On demand, and mandatory for any change to the scheduler, streaming,
  eviction or LOD selection: `--tier deep` — ~4 min.** That is where
  `ice_session` lives. It is currently red (§4), so it must be run with
  `--accept` blessing the pinned holds as known, or it will cry wolf; the
  baseline-aware STUCK path exists for exactly this.
- **Do not put a long recording in the merge tier.** The lane's cost is linear
  in recorded seconds with a ~1.4x multiplier and there is no way around it:
  the live phase *is* the flight.

Total gate cost if adopted as recommended: convergence reel 4.3 min + flight
merge tier 0.9 min ≈ 5.2 min per merge, ~9 min when the deep tier is pulled in.

---

## 6. Reproducibility and noise

Three independent live runs of the same generated `ice_session` script
(the ref phase was rendered once and reused):

| run | hold 3 window | residual first → final | changed frac | draws at end | verdict |
|---|---|---|---|---|---|
| A | 18.7 s | 4.267 → 4.259 | 15.55 % | 1162 / 1540 | STUCK |
| B (`TRI_STREAM_DEBUG=1`) | 18.7 s | 2.656 → 2.652 | 9.22 % | 1168 / 1540 | STUCK |
| C | 45 s | 2.657 → 2.647 | 9.19 % | 1174 / 1540 | STUCK |
| D — canonical / blessed | 45 s | 4.267 → 4.247 | 15.50 % | 1165 / 1540 | STUCK |
| (a fifth, `--tier deep` mid-mission) | 45 s | 3.113 → 3.096 | 11.14 % | — | STUCK |

**The magnitude swings ~60 % run to run; the verdict does not** — five for
five, and `tail_ratio` is 0.998–0.999 in every one. The variance enters during
the descent (the runs already differ at the arrival sample: 3.105 / 2.271 /
2.271 / 3.105 / 2.271) because which builds have landed by a given sim instant
is wall-clock dependent, exactly as V-1 documented. Two consequences:

- `residual_final` is a weak baseline quantity on a hard hold. `changed_frac`
  and `tail_ratio` are the robust ones; note that run A would have failed the
  3.0 residual trigger in runs B and C and only `changed_frac > 0.04` kept the
  verdict stable. **Keep the frac trigger — on this specimen it is load
  bearing, not a backstop.**
- Cross-machine comparison of `current.json` is meaningless, same as the reel.

Healthy samples show no noise at all: every converged sample is mean delta
exactly 0.000 (byte-identical to settled), so there is no epsilon floor to
budget for. Also worth noting the anti-thrash and protection windows in
`renderer.rs` are counted in **frames**, not seconds (`let window = if
self.settle_visuals { 0 } else { 180 };`, `burst_until_frame + 150`), so
streaming behaviour is frame-rate dependent and therefore machine dependent by
construction.

---

## 7. Escaped-gate analysis: would this lane have caught the original B-16?

**No. Honestly and structurally no.**

The lane replays recordings, and the only recording that flies into an ice
region is `sustained-blur.play` / `b16-live.play` — which exists *because*
Austin and Andrew had already reported the blur. The ledger says so in as many
words: "DATA NEEDED: an F10 recording starting ~1 min BEFORE flying into a
blurry area". At the moment of the original report the gate set would have
held `rec_001` (a savanna descent) and `rec_002` (an orbital survey). Neither
goes near ice. Both converge clean. B-16 would have sailed past.

That limitation is not a threshold I can tune away, so it is worth stating as
a property rather than a defect: **a recorded-flight lane is a regression net,
not a discovery net.** Its coverage is exactly the coverage of the flights it
is fed, and flights are produced by humans playing — the thing the gate is
meant to precede. The cold reel has the opposite shape: its pose table can be
extended to anywhere on the planet without anyone flying there, but it cannot
reproduce how a place is arrived at. They are complements, and neither
subsumes the other.

**What it would have caught, and did:** the *re-closing* of B-16. This lane is
the gate that belonged between "HUNT 4 / RESOLUTION" and Austin discovering on
his other machine that the blur was still there. Post-fix, on the same
binary, the cold reel says the specimen pose converges to byte-identical in
30 s and exits 0; this lane says the same pose reached by flying is pinned
with 9.2 % of the screen wrong after 45 s. Had it existed in that shift, the
word "resolved" would not have been written, and §4 would have been a
finding then instead of now.

**In class it also covers:** any hold in any recording where the picture stops
improving (B-16, B-12, B-18 residuals) and — via `cruise_mean` — the live view
falling behind *while moving*, which no existing gate measures at all
(`descent_land` cruises at 0.15 / 0.83 / 0.89 mean delta during a 124 km drop;
a regression that doubled it would flag).

**The cheapest way to widen it:** F10 is one keypress. If Austin and Andrew
drop a recording into `interchange/recordings/` whenever they fly somewhere
that felt wrong, promoting it into the gate is a file copy plus a two-line
table edit, and hold detection is automatic. That converts "I saw something
odd near the ice" into a standing measured number with no authoring step. It
is the highest-leverage habit change I can point at from this mission.

---

## 8. The premise: accepted, with a correction

Accepted. Warm state is real, it is measurable, it inverts the verdict at the
specimen pose, and the lane that measures it found a live defect on the
shipped binary within its first full run.

The correction is in §3: warm state is **not** "an accumulated cache". The
orbital cruise accumulates nothing (88–99 wanted, 0 uncached). What matters is
that the pose is **arrived at by descending** rather than teleported into. A
mission that had assumed "accumulate a big cache, then hold" — for example by
flying a long lateral tour and stopping — might have measured nothing. The
detection mechanism is right; the mental model behind it needed adjusting, and
the adjusted model is cheaper to satisfy (an 11 s descent already shows the
class in miniature at `descent_land`'s draw counts: 1831 → 2344).

---

## 9. Honest cut-lines

- **Only two flights, one of them trivial.** `descent_land` converges in
  0.2 s; as a gate it is a smoke test, not a probe. The lane is one specimen
  wide. It needs more recordings and I could not manufacture them — see §7.
- **The B-16 root cause is not chased.** I stopped at the repro packet. The
  eight keys HUNT 3 named are still refused and I did not find the refusing
  condition. `renderer.rs` is Lane A.
- **No pre-fix A/B.** I did not rebuild at `3488665` to show the lane's
  numbers moving when the defect moves. It is a single-file checkout
  (`viewer/src/renderer.rs`) and would have been ~15 min, but the post-fix
  result is stronger evidence than a pre-fix one would have been, and I
  preferred to spend the GPU on removing the window confound in §3. A reviewer
  who wants it should expect the *draw slope* to separate, not the pixel
  residual, which is already saturated post-fix.
- **`cruise` is nearly untested.** `ice_session`'s three cruise samples all
  land in orbit where live == settled trivially (0.000). Only `descent_land`
  exercises it meaningfully, with three samples. The metric is sound but it
  has no real baseline behind it yet.
- **`shot` in the live phase is a small optimistic bias**: one extra draw plus
  a readback per sample hands the background builders wall time a real player
  would not spend. ~26 samples over a 128 s flight is under 2 %. Inherited
  from V-1, not fixable without harness changes.
- **Ref-phase path independence is argued, not proved.** The ref process
  accumulates cache across its own shots. The argument is that every healthy
  sample reaches mean delta exactly 0.000 against it, and a stale-cache
  reference could not be byte-identical to a live frame. I did not run the
  hash-level double-render V-1 did for its poses.
- **No moon flights.** Same cut-line V-1 took, same reason: `moonpose`
  recordings do not exist.
- **Hold detection is position+angle only.** A player who stops moving but
  keeps the mouse drifting 0.1°/frame reads as motion and produces no hold.
  Raising `ANG_EPS_DEG` would fix it and would also start merging genuinely
  distinct look directions; 0.05° found every hold in all four recordings, so
  I left it.
- **Windows/pose granularity.** Samples land on pose-line boundaries (0.1 s at
  `posehz 10`), so sub-100 ms convergence is not resolved. Fine for this class.

---

## Feedback

**On the spec.**
- The spec asserts `viewer/interchange/recordings/rec_001.play`, `rec_002.play`
  and `sustained-blur.play` "are committed and tracked", and builds its
  "that blocker does not exist" argument on it. They are not: `.gitignore`
  has `viewer/interchange/*`. Only `viewer/scripts/b16-live.play` is tracked.
  The conclusion survived (b16-live.play alone is enough, and I promoted
  rec_001), but I spent time re-deriving the constraint the spec had already
  waved away. `git ls-files | grep recordings` is a five-second check and it
  would be a good habit for spec authors making a "this exists" claim.
- The GPU-contention instruction was clear and useful. I verified before every
  measured run; no other `play.exe` was running for any number in this
  document. That paragraph should stay in the template.
- "Explicit permission to reject the premise" did real work here — it is why I
  went looking for the confound in §3 (my 18.7 s window vs the cold lane's
  30 s) instead of shipping the first flattering result. Keep it.
- Ambiguity I had to resolve by assumption: nothing said whether the lane
  should *gate* on the b16 specimen given its known state, so I built the
  baseline-aware STUCK path (a known-pinned hold reports as known, a newly
  pinned one is a finding) and left the tree red-until-blessed. Also unstated:
  sample density, hold tolerances, whether to sample during motion at all
  (I added `cruise` because "keeps up with" is in the spec's own sentence),
  and whether extending a recording's tail is legitimate (I decided yes, and
  it turned out to be the difference between a suggestive result and a
  decisive one).

**On the repo and tooling.**
- **`CLAUDE.md`, `viewer/CLAUDE.md` and the whole of `docs/` are untracked.**
  A fresh worktree — which every mission gets, and which the spec tells us to
  read those files first from — does not contain them. I read them out of the
  main checkout. `docs/PROCESS.md` says reviews live in `docs/reviews/` and are
  "**git-tracked**, because `viewer/interchange/` is gitignored and evidence
  stored there has been lost before"; they are not tracked either. This is the
  single most surprising thing I found and it silently undermines the process
  doc's own guarantee.
- **`convergence_reel.py` is likewise untracked** in the main checkout, so my
  worktree did not have the tool the spec told me to read first. I copied it
  in. If V-1 has not been committed by the time this is reviewed, V-3 does not
  run.
- **`run_phase` and `shot_draws` in `convergence_reel.py` hard-code
  `shot (s\d{4})`.** My first sample names were five digits and both the
  wall-clock column and the draw-count column silently came back empty — no
  error, just `None` everywhere. That cost a re-render. V-1's asked-for
  `reel_common.py` should take a sample-name pattern rather than baking one.
- **`bench N` renders N frames without advancing `sim_ticks`.** That is a
  genuinely useful primitive (give the streamer wall time without moving the
  clock) and it is not in the `play.rs` header comment — neither are `pose`,
  `posehz`, `bench` or `glide`. The header claims to be the authoritative
  command list and is missing the four commands this entire verification stack
  is built on. `b16-live.play` uses `bench` to extend its hold; I used pose
  repetition instead so both phases stay on one timeline, but I had to read
  the match arm to know the difference.
- **`TRI_STREAM_DEBUG` fires on `frame_counter % 300`,** which is 5 s at 60 Hz
  and is coarse for a 45 s observation. It also cannot be aimed at a time
  window. An interval env var (`TRI_STREAM_DEBUG=60`) would have doubled my
  resolution for free.
- V-1's complaint that generated `_*.play` files pollute `git status` was
  correct and I fixed it in `.gitignore` — flagging it because V-1 may have
  made the same edit and the coordinator will see a one-line conflict.
- **`weather_tuning.json` differs between the main checkout and the tracked
  version by line endings only** (CRLF vs LF, `.gitattributes` presumably).
  Harmless, but it means a naive `diff` of an asset directory looks alarming.
- **`viewer/assets/*.edj2` and `*.edt1` are not gitignored.** `.gitignore`
  covers `viewer/assets/*.bin` and `meta.json` but not the multiplayer
  journal/save files beside them, so they sit as permanent `??` noise in every
  `git status` (they are in the main checkout's too). Same class of wart as the
  generated `_*.play`; I did not add them myself in case one of them is
  deliberately meant to be tracked.
- **A footgun I built and then had to fix, worth knowing exists in the other
  reels too:** `--accept` copying `current.json` wholesale is wrong as soon as
  a tool has tiers or a `--flight` filter, because `current.json` only holds
  what the last run selected — blessing after a cheap partial run silently
  drops the expensive flight's baseline, and the next full run reports it as a
  brand-new finding. `convergence_flight.py` merges instead. `perf_reel.py` and
  `convergence_reel.py` always run their whole table so they are safe today,
  but the first time either grows a filter it inherits the bug.

**Surprises worth recording.**
- The ref phase of a 128 s flight costs **19 seconds**. I had budgeted for it
  being the expensive half and designed the sample schedule to be frugal for
  no reason. `pose` not drawing in the plain harness is a much bigger deal
  than it sounds: it makes settled ground truth along an arbitrarily long
  flight essentially free, and it is the reason this lane is affordable at all.
- The cold reel converged the b16 pose to **exactly 0.000 in 30 s** on my run,
  where V-1 measured 1.96–2.19 and ">30 s" a day earlier. Same code, same
  machine. That is a cliff-edge specimen: its verdict depends on whether the
  builds finish inside the window. V-1's tail-trend STUCK statistic is what
  keeps that from flipping the gate, and it earned its keep — but the *headline
  number* for that pose is not stable, and anyone reading `res_end` for it
  should know that.
- The eight keys HUNT 3 gave up on are still, byte for byte, at the head of
  the uncached list. Finding a previous hunt's exact unfinished business
  waiting in a fresh trace was not something I expected from a tooling mission.

**What would have made this faster.**
1. A `--phase ref|live` flag on the reels. I re-rendered the ref phase three
   times to re-run the live one under different env, and it is the one part
   that is provably reusable.
2. Timestamps in `run.log` (even a coarse `+wall_s` prefix behind an env var).
   V-1 asked for this too; I ended up correlating Python-side stdout
   timestamps against `frame_counter` in a separate stderr stream to line up
   the STREAMDBG trace with the samples.
3. A one-line "what does this env var compose with" table. `TRI_POSE_LIVE` ×
   `TRI_RAW_CAPTURE` × `TRI_STREAM_DEBUG` all compose and nothing says so; V-1
   made the same point about the first two and it is now three.
