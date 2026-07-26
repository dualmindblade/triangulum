# Mission A-1 — B-22 lifecycle instrumentation, then the B-21 fix

**Lane A (coupled core, serial).** You own `viewer/src/renderer.rs` this
shift. No other mission will touch it. Do not touch `examples/play.rs` —
mission D-2 owns it concurrently.

**Two commits, in this order, and the order is the point:**

1. **Instrumentation only.** Observability, provably inert when off,
   production bit-identical. Must not change ordering, caps, or any
   scheduling decision.
2. **The B-21 fix**, measured against commit 1's trace — before and after.

If you land them as one commit you destroy the only clean measurement of
whether B-21 actually matters. That measurement is worth more than the fix.

---

## Why

B-16 (blurry mesh never settles in ice regions) has been diagnosed and
re-diagnosed **five times** and re-closed wrongly twice. The reason is not
that the hunters were careless. It is that **no probe in this codebase can
distinguish waiting from working.**

`self.tile_pending.insert(key, epoch)` happens at `renderer.rs:2545`,
**before** `rayon::spawn` at `2549` (VERIFIED — I read it today). So
"pending for seconds" conflates Rayon queue time with
`build_tile_at_season` execution time, and is not a build-duration
measurement at all. Completions carry no timestamps. The count-1500
eviction path logs nothing. `TRI_GPU_TIMERS` measures the render pass, not
CPU tile construction.

The consequence, in Sol's words: HUNT 4's "the remaining tail is build
cost" was an **inference, not a measurement**, and the whole B-16 thread
was redirected to the B-4c campaign on the strength of it.

**Everything about B-16 is blocked behind this mission.** It is the
highest-value item in the ledger.

## The current state of B-16 (VERIFIED by the coordinator today, on this tree)

Same pose, same binary, same machine, same hour. The only difference is how
the camera arrives:

| | cold (teleport) | warm (replayed descent) |
|---|---|---|
| gate | `convergence_reel.py` `b16_ice_hold` | `convergence_flight.py --tier deep` `ice_session` h3 |
| verdict today | **exit 0**, residual → 0.000, 1543/1543 drawn | **exit 3, STUCK**, residual 2.322, 8.14 % pixels wrong, tail ratio 0.998 |

**That pair is the finding.** The cold lane alone reports success on a
pinned pose — do not use it as your only gate.

Two calibration facts you need, both measured, both surprising:

- **The residual magnitude swings ~60 % run to run; the verdict does not.**
  V-3 measured 4.267 across five replays; I measured 2.322 today. In my run
  the residual was **below** the 3.0 STUCK trigger and only
  `changed_frac 0.0814 > 0.04` fired. Do not tune against `residual_final`
  on this specimen — use `changed_frac` and `tail_ratio`.
- **Hold 2 is not a reliable specimen.** V-3 measured it STUCK (3.10);
  today it read 1.68 / tail 0.86 and did not flag. It is 4.7 s — too short
  to support a tail trend. **h3 (45 s) is the specimen**; it is now
  6-for-6 STUCK across two operators.

## What already exists — do not reinvent any of this

| Thing | Where | What it gives you |
|---|---|---|
| the warm repro | `python scripts/convergence_flight.py --flight ice_session` | ~4 min, exit 3, generates `scripts/_convflight_ice_session.play` |
| the raw trace run | re-run that generated script with `TRI_POSE_LIVE=1 TRI_RAW_CAPTURE=1 TRI_STREAM_DEBUG=1` | the flight replayed with your new events |
| the cold control | `python scripts/convergence_reel.py` | ~4.3 min, exit 0, **blessed baseline as of today** |
| existing stream debug | `TRI_STREAM_DEBUG` in `renderer.rs` | fires on `frame_counter % 300` = 5 s at 60 Hz. Coarse for a 45 s observation, and cannot be aimed at a time window |
| prior art on B-16 | `BUGS.md` B-16, HUNTs 1-5 | five hunts. Read them. HUNT 3's eight refused keys are still refused |
| Sol's full audit | `docs/reviews/R1-b16-tail-audit.md` §Decisive measurement | the event list below is Sol's, verbatim in intent |

`TRI_POSE_LIVE`, `TRI_RAW_CAPTURE` and `TRI_STREAM_DEBUG` all **compose**.
Nothing outside this line says so; three separate sessions have now
rediscovered it.

## Commit 1 — the instrument

Extend `TRI_STREAM_DEBUG` to emit **one JSONL event per key against a
single monotonic clock origin**:

- `missing` — first frame the key is missing, with `covered`, gap rank,
  `on_screen`, approximate projected pixel area
- `admit` — with `source` (`forecast` / `covered_gap` / `parent` / `stale`),
  pending totals, productive totals
- `worker_start` — **inside** the Rayon closure, not before the spawn
- `worker_done` — after `build_tile_at_season`: build ms, vertex/index
  counts, mesh bytes
- `received`
- `first_exact_draw`
- `evict` — from **every** path, with reason (`count1500` / `gentle` /
  `hard`), whether the key was in `live_wanted`, and last-used age

Plus a **1 Hz summary**: `wanted`, `uncached`, `covered_missing`,
on-screen missing, gap-pending, gap-unadmitted, exact-selected draws, and
productive workers started-but-not-done.

Insertion points (Sol's, line numbers VERIFIED current today):
`renderer.rs` 2450-2554, 2013-2040, 2606-2717, 3792-3924.

**Also add an interval control** — `TRI_STREAM_DEBUG=60` should mean "every
60 frames" rather than the hard-coded 300. V-3 lost resolution to this and
asked for it explicitly. Keep bare `TRI_STREAM_DEBUG=1` working as it does
today.

### Then produce the measurement that settles the argument

For each **on-screen** key over the 45 s h3 hold, compute
`T_total = first_exact_draw − first_missing` and partition it:

```
T_unadmitted  +  T_queue  +  T_build  +  T_land/swap  =  T_total
```

That partition — not a pending count, not an aggregate draw curve — is the
measurement. Sol names the fork it decides:

- **Build-cost dominated** — every high-projected-area gap admits within
  one available scheduler frame; `admit → worker_start` and
  `worker_done → first_exact_draw` are negligible against
  `worker_start → worker_done`; productive slots stay occupied; no wanted
  key is evicted and re-admitted. Then HUNT 4 was substantially right.
  **If only ~5 workers run while cores sit idle, report that as "build cost
  under the five-slot scheduler cap", not "not scheduling."**
- **Scheduling still involved** — a high-impact gap spends material time
  unadmitted, admitted-but-not-started, done-but-not-drawn, ranked behind
  off-screen work, or evicted and rebuilt. The dominant interval names the
  owner.

Two specific questions the trace should answer, because two prior hunts
died on them:

1. **HUNT 3's eight keys** — `f4L11(2002..2005, 1652..1653)` sat at the
   head of the uncached list for the entire hold in every run since HUNT 2.
   *Which* refusing condition are they hitting? Name the branch.
2. **HUNT 1's standing claim** — "arrivals land in cache but SELECTION
   never swaps." V-3 measured 27 tiles landing that never entered the drawn
   set. Is that still true, and if so where does `first_exact_draw` never
   fire?

## Commit 2 — B-21

`evict()` at `renderer.rs:3792-3796` (VERIFIED today):

```rust
let cutoff = self.frame_counter.saturating_sub(120);
if self.cache.len() > 1500 {
    self.cache.retain(|_, t| t.last_used >= cutoff);   // no live_wanted check
}
```

HUNT 2's wanted-set protection went only into the **later** gentle VRAM
shed (~3838-3845, `t.last_used < protected && !self.live_wanted.contains(k)`).
The count sweep never got it. B-16's wanted set is ~1540 and its settled
draw count 1543, so the 1500 threshold sits **directly at the symptom
site**, and this path logs nothing — which is exactly why HUNT 4's probes
could not see it.

**A finding of mine, not in the ledger:** the `moon_cache` sweep two lines
below (`if self.moon_cache.len() > 2400 { ... retain ... }`) has the
identical hole. Fix it in the same commit or file it — your call, but say
which.

Add the check. Then re-run the trace and **say plainly whether it changed
anything.** "No measurable effect, the defect is still real, fixed anyway"
is a perfectly good result and I will take it at face value. A claim that
it fixed B-16 needs the h3 verdict to actually flip.

## Done criteria

- [ ] Two commits, in order, first one observability-only.
- [ ] Instrument proven inert when off: **byte-identical settled captures**
      and a clean perf reel with the env var unset.
- [ ] JSONL trace over the h3 hold, archived under `docs/missions/A1-evidence/`.
- [ ] The `T_total` partition table for on-screen keys.
- [ ] An explicit verdict on Sol's fork, in Sol's own terms.
- [ ] A named refusing condition for the eight keys, **or** an honest "I
      could not find it, here is what I ruled out."
- [ ] B-21 fixed, with before/after traces and a plain statement of effect.
- [ ] `BUGS.md` updated: B-22 closed, B-21 closed or amended, B-16's HUNT 6.

## Gates

Scheduler/streaming blast radius, so per `docs/PROCESS.md` §6: **suites,
perf reel, world reel, convergence reel, convergence flight `--tier deep`.**

Run them from `viewer/`. `bash scripts/verify.sh` — note it is at
`viewer/scripts/verify.sh`, **not** repo root, whatever the gate table says.

Today's baseline on this tree, so you know what you inherited:

- suites **8/8 PASS**
- world reel **exit 0**, 29/29 clean
- convergence reel **exit 0**, 4/4 clean — **blessed today**, so drift now flags
- convergence flight `--tier deep` **exit 3**, ice_session h3 STUCK,
  deliberately left unblessed so it keeps crying until B-16 genuinely closes
- perf reel **exit 0**, no regressions — but read the next paragraph before
  you run it.

**The perf reel needs a cold, idle GPU, and this bit me today.** Chained
directly after the world reel it reported a false 37 % regression on
`plains_walk` (avg 24.65 → 33.82, p95 → 46.67). Standalone re-run: clean,
26.02 / 31.25. The elevation was concentrated in the first scenarios and
decayed through the run (+4.6 %, +30 %, +6.3 %, then ~2 %) — residual load
from the previous reel, not code. Zero Rust has changed since `120995b`
(`git diff --stat 120995b..HEAD -- viewer/src/` is empty), so no perf
change was possible anyway. **Run the perf reel alone, first, on an idle
machine.** If it flags, re-run it standalone before believing it.

**If a gate flags:** explain every flagged pose. A changed pose is not a
failure; an unexplained one is. Never `--accept` anything — propose, and
the coordinator or Austin decides.

## The GPU is the real concurrency limit

One GPU, and other missions are running this shift. **Every timing
measurement in this mission must have the GPU to itself.** Before any
measured run, confirm no other `play.exe` is running. If one is, poll until
it clears — but **poll directly, do not end your turn waiting on a
background process** (agents that do this simply stop).

Settled-frame work elsewhere (world reel, altitude ladder) is
timing-insensitive and will not corrupt your pixels — only your timings.

## Declared human-verdict needs

**None, if the trace is decisive** — that is the entire point of this
mission. If you reach a conclusion that needs Austin to fly the game to
confirm, say so explicitly and early; per PROCESS §2 an undeclared need for
human eyes does not get granted, it gets an instrument instead.

## Evidence packet

`docs/missions/A1-lifecycle-findings.md` plus `A1-evidence/`:
raw JSONL traces (before and after B-21), the partition table, the
convergence flight reports for both, and the inert-instrument proof.

## You may reject this mission's premise

Explicitly and with my blessing. Missions here have been wrong before and
the best outcomes came from agents that measured the premise and refuted
it. In particular:

- If the partition shows HUNT 4 was right all along and this is genuinely
  build cost under a five-slot cap, **say so** — that is a real result and
  it redirects the whole campaign honestly.
- If B-21 is unreachable in practice, say so.
- If Sol's event list is the wrong instrument for the question, propose a
  better one before building it.

**Marked assumptions you should check rather than trust:** that the eight
keys are still the head of the uncached list on today's tree; that the
descent path (not the cache size) is the distinguishing variable — V-3
measured the orbital cruise accumulating almost nothing (88-99 wanted, 0
uncached), so "warm state" is *arrival by descent*, not a big cache, and
V-3 did not isolate which part of the descent matters.

## Feedback — mandatory

You are building new tooling, so `## Feedback` in your findings doc is
**required** per PROCESS §9. Candidly: what the spec got wrong, what cost
you time, what tooling would have helped, every assumption you made because
this spec did not say, and anything you noticed about the codebase even if
unrelated. The last three missions' feedback sections have each produced
real fixes. Vague praise is not feedback.

## Dispatch checklist — the four things that bite every worktree mission

1. **Your worktree is NOT at current main.** Merge main into it first, or
   you will lack the CLAUDE.md files and the tools this spec tells you to
   read. (Symptom: tracked files appear "untracked".)
2. **A fresh worktree has no game assets.** `viewer/assets/*.bin` is
   gitignored (~170 MB). **Copy** it from the main tree — never symlink or
   junction. A recursive delete through a junction destroys the originals,
   which cost ~1.5 h of generation.
3. **`cargo build --release` does NOT build examples.** Say
   `cargo build --release --example play` explicitly, every time you change
   `src/`, or you will verify a stale binary.
4. **Do not end a turn waiting on a background process.** Poll directly.

Kill stale `triangulum-viewer` / `play` processes before a build or the
linker fails on a locked binary.
