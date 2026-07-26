# Mission A-1 — tile lifecycle attribution and B-21

## Outcome

B-22 is closed by the instrumentation-only commit `9d08bc6`
(`Instrument tile streaming lifecycle`). With `TRI_STREAM_DEBUG` absent,
three settled captures before and after that commit are byte-identical. With
the variable present, every terrain key uses one renderer-monotonic JSONL
clock from first missing observation through admission, Rayon execution,
receipt, exact draw, and eviction.

Sol's fork resolves to **Scheduling still involved**. In the pre-B-21 h3
trace, all 100 on-screen missing keys at the first full h3 sample eventually
drew, but their aggregate `T_total` was 96.63% unadmitted. Mean queue time
was 0.336 ms and mean measured build time was 31.679 ms; mean unadmitted time
was 22.731 s. Every one of the 100 keys was dominated by the unadmitted
interval. HUNT 4's claim that the residual tail was tile build duration is
not supported by this measurement.

B-21 is fixed in the second A-1 commit. The count-triggered terrain sweep now
protects `live_wanted`. I fixed the identical moon-cache hole in the same
commit by recording and protecting the current `moon_keys` set.

B-21 mattered, but it did **not** prove B-16 fixed:

- wanted count-1500 evictions fell from 68 to 0 over the raw replay;
- four of HUNT 3's eight keys stopped being evicted and built twice;
- the first full h3 sample improved from 254 uncached / 100 on-screen
  missing to 48 / 23;
- the 23 keys present in both h3 cohorts spent a mean 4.890 s less time
  unadmitted, while queue and build time were unchanged;
- ordinary `ice_session` h3 was clean both before and after B-21, so there
  was no verdict flip to attribute to the fix.

The plain result is: **B-21 was reachable, caused duplicate wanted work, and
its fix measurably shortened the admission backlog. B-16 did not reproduce as
STUCK in the pre-fix h3 run, so this mission cannot claim that B-21 fixed
B-16. The post-fix residual backlog remains admission-dominated.**

## 1. Instrument

`TRI_STREAM_DEBUG` now emits JSON objects to stderr with
`stream_lifecycle: 1`, a monotonically increasing `seq`, and `t_us` measured
from one `Instant` origin. Events are:

- `missing`: covered status, current covered-gap rank, screen intersection,
  and approximate projected pixel area;
- `admit`: `forecast`, `covered_gap`, `parent`, or `stale`, plus pending and
  productive totals;
- `worker_start`: emitted inside the Rayon closure;
- `worker_done`: monotonic build interval, the builder's own build duration,
  vertex/index counts, and mesh bytes;
- `received`: epoch/season match and moving-drop outcome;
- `first_exact_draw`;
- `evict`: `count1500`, `gentle`, or `hard`, with wanted membership and
  last-used age;
- `summary`: at least once per 60 frames, with all counts requested by the
  mission and the exact on-screen missing-key cohort.

I added one event beyond Sol's list: `gap_blocked`. The required event list
can show that a key has not admitted, but cannot name the branch that refused
it. `gap_blocked` records `admission_budget` or `productive_cap`, the active
limits, and gap rank; it deduplicates until the condition changes.

Interval behavior:

- bare `TRI_STREAM_DEBUG=1` retains the old 300-frame text-dump cadence;
- `TRI_STREAM_DEBUG=60` sets that cadence to 60 frames;
- the JSON lifecycle summary is never coarser than 60 frames.

The debug state is `None` when the variable is absent. The debug-off worker
closure and retain paths preserve their original bodies. No cap, ordering,
admission budget, or scheduling predicate changed in commit 1.

### Inert proof

The base and commit-1 runs used `viewer/scripts/b16-repro2.play` in settled
capture mode with `TRI_STREAM_DEBUG` unset. All six PNGs are 711,779 bytes and
have the same SHA-256:

`4441C707B2D5542DEDDD9DAF651CF8394ABFCDA4391C9994A1CE080A8B27A02E`

The full captures and transcripts are in
[`inert-settled-captures.zip`](A1-evidence/inert-settled-captures.zip); the
machine-readable comparison is
[`inert-instrument-proof.json`](A1-evidence/inert-instrument-proof.json).

The perf reel was not clean against its accepted baseline, but immediate
controls prove that is inherited rather than instrumentation overhead:
commit 1 and untouched base `74cb703` reproduce the same mountain/moon costs
as the final build. Section 6 records the formal red gate and all controls.

## 2. Partition method

The raw replay is the generated
[`ice-session-generated.play`](A1-evidence/ice-session-generated.play), run
with:

```text
TRI_STREAM=1
TRI_STREAM_DEBUG=60
TRI_POSE_LIVE=1
TRI_RAW_CAPTURE=1
```

The analyzer is
[`analyze_lifecycle.py`](A1-evidence/analyze_lifecycle.py). It identifies the
fixed final h3 pose (41.518883, 122.511318, 3.0498 km, yaw 92.98, pitch
-68.3), unions the on-screen missing keys seen by its 1 Hz summaries, and
follows each key from its latest applicable `missing` event through
`first_exact_draw`.

Its state machine is:

```text
missing -> unadmitted
admit -> queue
worker_start -> build
worker_done -> land/swap
evict -> unadmitted
first_exact_draw -> complete
```

Every transition charges elapsed monotonic wall time to the old state.
Therefore, including rebuild cycles:

```text
T_unadmitted + T_queue + T_build + T_land/swap = T_total
```

The pre trace produced 100 complete rows, zero censored rows, and zero
partition/state anomalies. The full per-key table is
[`pre-b21-partition.csv`](A1-evidence/pre-b21-partition.csv). The matching
post table is
[`post-b21-partition.csv`](A1-evidence/post-b21-partition.csv).

Lifecycle logging slows the replay substantially; these are trace wall
times, not estimates of a 60 Hz player's elapsed time. Phase attribution and
the before/after traced comparison are valid; the absolute seconds should
not be transferred to gameplay.

## 3. Pre-B-21 measurement

At h3 frame 5040 the trace reported 1,540 wanted, 254 uncached, 249
gap-unadmitted, 5 gap-pending, 5 productive workers active, 100 on-screen
missing, and 1,242 exact-selected draws. Uncached reached zero at frame 5220,
7.883 trace-wall seconds later. The fixed-pose summary continued through
frame 7680.

### All 100 on-screen keys

| interval | sum (ms) | mean (ms) | median (ms) | p95 (ms) | max (ms) |
|---|---:|---:|---:|---:|---:|
| `T_total` | 2,352,287.474 | 23,522.875 | 23,933.424 | 25,147.740 | 27,195.810 |
| `T_unadmitted` | 2,273,108.119 | 22,731.081 | 23,266.053 | 24,736.884 | 25,065.240 |
| `T_queue` | 33.606 | 0.336 | 0.339 | 0.489 | 0.631 |
| `T_build` | 3,167.933 | 31.679 | 32.820 | 46.488 | 95.977 |
| `T_land/swap` | 75,977.816 | 759.778 | 435.316 | 3,505.172 | 4,300.656 |

Aggregate share of `T_total`:

| phase | share |
|---|---:|
| unadmitted | **96.6339%** |
| queue | 0.0014% |
| build | 0.1347% |
| land/swap | 3.2300% |

The builder-reported duration, which excludes event-emission overhead, was
30.036 ms mean / 43.692 ms p95 / 49.236 ms max.

Sixty-nine keys had projected area at least 10,000 px. They were not an
exception: mean `T_unadmitted` was 23.074 s, mean queue 0.339 ms, mean build
32.986 ms, and mean land/swap 642.204 ms. Every high-impact key admitted
orders of magnitude later than one available scheduler frame.

The largest projected keys illustrate the same result:

| key | area px | rank | total ms | unadmitted ms | queue ms | build ms | land/swap ms |
|---|---:|---:|---:|---:|---:|---:|---:|
| f4L13(7995,6581) | 50,672 | 67 | 23,315.256 | 22,631.168 | 0.448 | 33.716 | 649.924 |
| f4L13(7993,6580) | 44,474 | 100 | 23,933.574 | 23,404.276 | 0.362 | 41.632 | 487.304 |
| f4L13(7994,6581) | 43,925 | 78 | 23,315.126 | 23,094.982 | 0.324 | 32.767 | 187.053 |
| f4L13(7994,6580) | 42,615 | 84 | 23,314.701 | 23,216.716 | 0.229 | 46.444 | 51.312 |
| f4L13(8000,6584) | 41,895 | 79 | 24,200.127 | 23,475.449 | 0.330 | 35.404 | 688.944 |

Admission source was `covered_gap` for 93 keys and `forecast` for 7. The
five productive slots remained occupied while the gap list drained. Once a
key admitted, Rayon queue time was negligible and its build was short.

### Verdict on Sol's fork

**Scheduling still involved — the dominant interval is unadmitted time.**
The immediate owner is the admission policy under the five-slot productive
cap, not Rayon queue delay or tile construction duration. A smaller
secondary delay exists between worker completion and first exact draw, but
all 100 visual keys did draw.

This is not “build cost under the five-slot scheduler cap” in the sense Sol
defined for the build-dominated branch. Workers execute short builds when
admitted; the visual keys spend almost all of their lifecycle waiting for
admission while five other productive jobs occupy the cap.

## 4. The two standing hunt questions

### HUNT 3's eight keys

The marked assumption that these eight remain at the uncached head for the
entire hold is false in this trace. All eight draw before the first full h3
sample: four at frame 4918 and four at frame 4944.

The named refusing branch is **`productive_cap`**. All eight hit it in the
pre-B-21 descent trace with `productive_total == productive_limit` (6/6 in
the eager descent state, later 5/5). Three also hit
**`admission_budget`**:

- f4L11(2004,1652)
- f4L11(2004,1653)
- f4L11(2005,1653)

| key | refusal(s) | pre starts | wanted count eviction | first exact frame | post starts | post first exact |
|---|---|---:|---:|---:|---:|---:|
| f4L11(2002,1652) | productive cap | 2 | frame 4867 | 4918 | 1 | 4797 |
| f4L11(2003,1652) | productive cap | 2 | frame 4886 | 4918 | 1 | 4797 |
| f4L11(2004,1652) | budget + productive cap | 2 | frame 4914 | 4944 | 1 | 4818 |
| f4L11(2005,1652) | productive cap | 2 | frame 4474 | 4944 | 1 | 4818 |
| f4L11(2002,1653) | productive cap | 1 | — | 4918 | 1 | 4797 |
| f4L11(2003,1653) | productive cap | 1 | — | 4918 | 1 | 4797 |
| f4L11(2004,1653) | budget + productive cap | 1 | — | 4944 | 1 | 4818 |
| f4L11(2005,1653) | budget + productive cap | 1 | — | 4944 | 1 | 4818 |

Thus HUNT 3's remaining refusal is not mysterious on this run: it is the
same productive cap the surrounding gap backlog hits. B-21 then discards
four completed wanted members of this specific group, forcing four duplicate
builds. Full audits:
[`pre`](A1-evidence/pre-b21-hunt3-eight.csv) and
[`post`](A1-evidence/post-b21-hunt3-eight.csv).

### HUNT 1's “lands but never swaps” claim

It does not hold for the h3 on-screen cohort in this trace:

- 100/100 pre-B-21 cohort keys were accepted by `received`;
- 100/100 emitted `first_exact_draw`;
- zero accepted cohort arrivals lacked an exact draw;
- median worker-done-to-exact-draw was 435 ms, p95 3.505 s, max 4.301 s.

There are 189 distinct non-cohort keys accepted during h3 with no
`first_exact_draw` anywhere in the trace. They include off-screen forecast,
parent, stale, and speculative work and are not evidence that the viewed
area failed to swap. For those keys the event never fires because they never
enter the exact selected draw set before trace end. The visual backlog—the
cohort that can test HUNT 1—does enter it completely.

## 5. B-21 before and after

Terrain fix:

```text
last_used >= cutoff || live_wanted.contains(key)
```

Moon fix:

```text
last_used >= cutoff || live_moon_wanted.contains(key)
```

`live_moon_wanted` is populated from the current `moon_keys` selection. The
moon defect was fixed by inspection; the ice trace exercises Neisor and
does not measure lunar behavior.

| measurement | pre | post | effect |
|---|---:|---:|---:|
| wanted count-1500 evictions, whole trace | 68 | **0** | -68 |
| wanted count-1500 evictions, h3 | 9 | **0** | -9 |
| HUNT 3 eight wanted evictions | 4 | **0** | -4 |
| HUNT 3 eight worker starts | 12 | **8** | four duplicate builds removed |
| first h3 uncached | 254 | **48** | -206 |
| first h3 on-screen missing | 100 | **23** | -77 |
| first h3 exact-selected draws | 1,242 | **1,443** | +201 |
| h3 wall to first zero-uncached summary | 7.883 s | **2.806 s** | -5.076 s |
| raw replay wall | 274.3 s | **222.7 s** | -51.6 s observed |

The post h3 cohort contains 23 keys, all complete and all dominated by
unadmitted time. Its aggregate phase shares are 98.7519% unadmitted,
0.0018% queued, 0.1107% building, and 1.1356% land/swap.

For an apples-to-apples comparison, all 23 post keys are also in the pre
cohort:

| interval, matched 23 | pre mean ms | post mean ms | delta ms |
|---|---:|---:|---:|
| `T_total` | 22,038.249 | 17,171.598 | **-4,866.650** |
| `T_unadmitted` | 21,846.801 | 16,957.279 | **-4,889.522** |
| `T_queue` | 0.325 | 0.308 | -0.017 |
| `T_build` | 19.490 | 19.014 | -0.475 |
| `T_land/swap` | 171.633 | 194.997 | +23.364 |

The improvement is therefore less admission waiting, not faster builds.
The compact machine-readable comparison is
[`pre-post-comparison.csv`](A1-evidence/pre-post-comparison.csv).

### Did B-21 fix B-16?

No claim is possible from this run. The ordinary pre-B-21 h3 replay was
already clean, converging only at its final 45.0 s sample. The ordinary
post-B-21 replay converged by 8.0 s. The final deep gate also converged by
8.0 s. That is compatible with B-21 helping substantially, but the mission's
required criterion was an actual h3 verdict flip and there was none.

Hold 2 demonstrates why it is not the specimen: pre was clean
(`changed_frac=.0499`, `tail_ratio=.925`), the first post run was STUCK
(`.0890`, `.999`), and the final deep gate was clean again
(`.0499`, `.925`). H3 was clean in all three A-1 ordinary runs.

## 6. Gates

No baseline was accepted.

| gate | result | evidence |
|---|---|---|
| release example build | PASS | `cargo build --release --example play` |
| suites | **8/8 PASS** | [`gate-suites.log`](A1-evidence/gate-suites.log) |
| world reel | **exit 0, 29/29 clean** | [`log`](A1-evidence/gate-world-reel.log), [`report`](A1-evidence/gate-world-reel-report.json) |
| convergence reel | **exit 0, 4/4 clean** | [`log`](A1-evidence/gate-convergence-reel.log), [`report`](A1-evidence/gate-convergence-reel.json) |
| convergence flight `--tier deep` | **exit 0, 5/5 clean** | [`log`](A1-evidence/gate-convergence-flight-deep.log), [`report`](A1-evidence/gate-convergence-flight-deep.json) |
| perf reel | **exit 3, inherited flags** | controls below |

### Perf gate audit

The final perf reel was run standalone twice. The confirmation began at 1%
GPU utilization and still flagged mountain and moon. I then built commit 1
and untouched mission base `74cb703` into the same target directory and ran
each immediately under the same conditions.

| scenario | accepted | untouched base | commit 1 | final B-21 |
|---|---:|---:|---:|---:|
| mountain avg ms | 26.14 | 32.55 | 32.42 | 32.14 |
| mountain p95 ms | 27.15 | 38.00 | 37.51 | 37.34 |
| moon surface avg ms | 15.40 | 22.22 | 21.46 | 21.46 |
| moon orbit avg ms | 2.30 | 3.20 | 3.14 | 3.35 |
| moon orbit p95 ms | 2.85 | 3.52 | 3.37 | 3.96 |

The other final scenarios remained within gate tolerances. The exact A-1
change cannot cause a cost already present at `74cb703`; the three builds
agree within normal run noise. This explains attribution of every flag:
they are an inherited accepted-baseline/machine mismatch, not an A-1
regression. The absolute reason the accepted baseline no longer reproduces
was not identified, so the formal perf gate remains red rather than being
called clean.

Evidence:

- [`gate-perf-reel.log`](A1-evidence/gate-perf-reel.log)
- [`gate-perf-reel-rerun.log`](A1-evidence/gate-perf-reel-rerun.log)
- [`gate-perf-reel-commit1-control.log`](A1-evidence/gate-perf-reel-commit1-control.log)
- [`gate-perf-reel-base-control.log`](A1-evidence/gate-perf-reel-base-control.log)

`cargo fmt --all -- --check` is not usable as a repository gate on this
tree: it reports thousands of pre-existing formatting differences across
examples and sources. I did not mass-format files outside Lane A.

## 7. Evidence index

Primary raw evidence:

- [`pre-b21-trace.jsonl`](A1-evidence/pre-b21-trace.jsonl)
- [`post-b21-trace.jsonl`](A1-evidence/post-b21-trace.jsonl)
- [`pre-b21-convergence-flight.json`](A1-evidence/pre-b21-convergence-flight.json)
- [`post-b21-convergence-flight.json`](A1-evidence/post-b21-convergence-flight.json)

Both JSONL files parse with zero malformed lines. Pre has 96,520 lifecycle
events; post has 96,423. The analyzer outputs include the full per-key
partition, 1 Hz h3 timeline, aggregate JSON, and eight-key audit for each
run.

## Feedback

- **The stated h3 baseline did not reproduce before the fix.** The dispatch
  said h3 was 6-for-6 STUCK and that only residual magnitude moved. The
  pre-B-21 run instead fell off a cliff exactly at the final 45 s sample and
  exited clean. The spec correctly warned that endpoint cliffs exist, but
  its stronger “verdict does not move” claim was false on this run. Requiring
  a before trace was the right protection against a false fix claim.
- **Sol's required event list cannot answer the required refusal question by
  itself.** A key with `missing` and no `admit` does not name the rejecting
  `if`. I added the deduplicated `gap_blocked` event. Future lifecycle specs
  should include rejection events whenever they demand a named refusal
  branch.
- **A 1 Hz count summary was insufficient to define the visual cohort.** A
  key can first become missing during the descent, long before h3. I added
  the exact on-screen missing key/area/rank list to summaries so the fixed
  h3 view can be joined back to its earlier `missing` event. This is debug
  only but makes the partition reproducible.
- **JSONL changes the clock it measures.** Serializing roughly 96,000 events
  to one stderr mutex stretched the raw pre replay to 274 s. Builds are not
  performed under the sink lock, and the attribution gap is orders of
  magnitude, but the absolute wall seconds are instrument-perturbed. A
  bounded binary ring buffer drained after capture would preserve the
  monotonic lifecycle without distorting scheduler wall time as much.
- **The trace exposed broad speculative churn unrelated to the visual
  cohort.** Both runs admitted about 18,400 jobs and count-evicted about
  15,600 non-wanted tiles. B-21 correctly removes wanted victims, but it does
  not reduce the off-screen forecast/parent churn. That deserves a separate
  efficiency audit; it is not evidence of B-16 by itself.
- **The moon analogue was not dynamically measured.** I fixed it because the
  predicate was structurally identical and the current selection is readily
  available. The assumption is that all `moon_keys` are wanted even when the
  shared draw buffer truncates the actual drawn subset. Protecting the full
  selected set is consistent with terrain and avoids rebuilding a key that
  remains selected.
- **GPU ownership is a convention, not a tool.** Every timing run checked
  `play.exe` immediately before launch, but another mission can race the
  check. A repo-level named mutex acquired by every timing harness would make
  “GPU clear” enforceable and record the owner automatically.
- **The Windows gate command is underspecified.** Bare `bash` resolved to a
  WSL shim with no distribution; directly invoking Git's bash lacked
  `dirname` until run as a login shell. The working form was
  `C:\Program Files\Git\bin\bash.exe -lc
  'cd /c/code/tri-a1/viewer && ./scripts/verify.sh'`.
- **Perf baselines need provenance and a same-binary control mode.** Two
  final runs from a 1%-utilized GPU were red, yet commit 1 and untouched base
  reproduced the same costs. Recording driver, power state, GPU clocks, and
  baseline commit—or offering `--control EXE`—would turn this from a manual
  three-build audit into one command.
- **The managed sandbox could not lock this worktree's Git index.** Its
  metadata lives at `C:\code\triangulum\.git\worktrees\tri-a1`, outside the
  writable root. I recorded the exact two commits in a workspace-local
  shadow repository and will hand off a bundle/patches. Worktree missions
  need the linked-worktree Git directory writable, not only `.git` readable.
- **Assumptions made:** projected area is the clipped screen-space bounding
  box of tile corners plus center, not exact raster coverage; “on-screen over
  h3” is the union of fixed-pose summary cohorts plus between-summary
  on-screen `missing` events; the high-impact table uses 10,000 px only as a
  reporting cut, not a scheduler rule; monotonic wall time is the requested
  partition clock; and a same-frame eviction/re-admission returns directly
  to queue even when no second `missing` event can be observed.
