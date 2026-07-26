# Mission X-1 — B-19: diagnose the 0.5-1 km staircase band

**Diagnosis only. Do not fix anything.**

This mission produces a **root cause with evidence**, not a patch. You may
read anything and measure anything. You may not edit `viewer/src/renderer.rs`
(mission A-1 owns it), `viewer/examples/play.rs` (D-2), or
`viewer/scripts/world_reel.py` and the reel pose tables (D-1).

If a diagnostic toggle would help, prefer the existing ones
(`TRI_LOD_RING_DEBUG`, `TRI_NO_MORPH`, `TRI_FORCE_MORPH`, `TRI_NO_IMPOSTORS`,
`TRI_NO_ROCKS`, `TRI_POSE_RENDER`). If you genuinely need a new one, say so
in your findings and describe it — do not land it in a Lane A file this
shift.

Why diagnosis-only: the fix is very likely art-facing, and **Andrew has
final say on how natural is natural enough.** A fix landed before his call
is a fix that may have to be undone. Get the cause nailed; the cure is a
separate, better-informed mission.

---

## Why

**Found by an instrument on its first run, not by eye** — which is the
outcome this whole verification push exists to produce.

EVERY irregular boundary class — river bank, lake shore, biome ecotone,
snowline — collapses from healthy voxel-scale dither onto a **15-35 m
lattice in the 0.5-1 km altitude window**, then blurs out above ~1.5 km.

Measured pitch:
- `river_bank` **3.4 m → 10.3 m** between 300 m and 500 m
- `ecotone` **10.6 m → 35.4 m** between 700 m and 1 km

This is a **direct violation of Andrew's B-17 art bar** — *"all irregular
boundaries must read as natural curves at every altitude"* — which was
ledgered fixed at `c9cb081`. That fix addressed range/LOD tint and
far-field curves. **This band is nearer, and was never measured.** It also
includes the polygonal pond shores visible in a routine smoke render.

`BUGS.md` B-19 is explicit: **NOT YET DIAGNOSED — no root cause proposed.
Nobody has looked at why.** The measurement is solid; the diagnosis does
not exist. That gap is your entire job.

## What already exists — do not reinvent any of this

| Thing | Where | What it gives you |
|---|---|---|
| the instrument that found it | `python scripts/altitude_ladder.py` from `viewer/` (~66 s) | the whole measurement, plus a lint that flags the band automatically below ~1 km |
| brackets | `--rungs` / `--tag` on the same script | narrow the altitude window |
| the specimen sheet | `docs/missions/V2-evidence/specimen_1km_staircase.png` | 200 m healthy vs 1 km sawtooth, side by side — **built for Andrew's eyes** |
| contact sheets + numbers | `docs/missions/V2-evidence/ladder_*.png`, `report.json` | per-class pitch measurements |
| V-2's findings | `docs/missions/V2-altitude-ladder-findings.md` | how the metric was calibrated, and four formulations that **anti-correlate** — read this before inventing a metric |
| the contract | `TRANSITIONS.md` | Andrew's verdicts are design constraints, not suggestions |
| `probe LAT LON` | play harness | ground truth at a coordinate in milliseconds |
| `sync_diff.py --at LAT LON ALT [YAW PITCH]` | `viewer/scripts/` | mesh↔voxel divergence at an arbitrary pose |

## A lead — explicitly my hypothesis, not a finding

`TRANSITIONS.md` records a **known cut** in the D v1 climate-tint/ecotone
work (`a65591f`):

> *"Known cut: far-mesh dither is per-VERTEX (sawtooth tongues at ~100 m
> spacing); per-pixel needs the A-style shader pass."*

Per-vertex dither on the far mesh is the same *mechanism class* as the
symptom: a boundary quantised to mesh topology instead of to the world.
The band sits between the voxel near-field and the far mesh, so the
LOD/geomorph handoff and the per-pixel shore/biome fields are the obvious
suspects.

**Two warnings about that lead, and I mean both:**

1. The documented cut says **~100 m**; the measurement says **15-35 m**.
   Those do not match. Either the mechanism scales with LOD level in a way
   that explains both, or this is a different cause wearing a similar coat.
   **Do not force the fit.**
2. **This is an inference I drew from reading, not something I measured.**
   The previous coordinator's two recorded spec errors both came from
   exactly this — asserting things confidently about a codebase that had
   been read but not measured, and one of those assertions turned out to be
   an entirely separate bug (B-20). Treat the paragraph above as a
   hypothesis to test or kill, and **say which you did.**

Refuting it cleanly is a good outcome and I will record it as such.

## What I want, and what I am deliberately not specifying

**State the observation, not the method.** V-2 was handed a prescribed
metric ("histogram axis-aligned run lengths") and spent ~90 minutes
implementing it faithfully before the numbers showed it anti-correlated —
the lattice is world-aligned, its screen projection depends on yaw, and run
length is confounded by honest voxel stepping. You know the material better
than I do by the time you are writing code.

The observation I want explained:

> Between roughly 0.5 and 1 km, irregular boundaries stop following the
> terrain and start following a fixed spatial lattice of 15-35 m. Above
> ~1.5 km they blur out. Below ~300 m they are healthy. **What changes in
> that window, and why does it quantise?**

A good answer names the mechanism, points at the code, and predicts
something checkable that was not used to derive it — then checks it.

## Done criteria

- [ ] A named root cause, with the code path identified.
- [ ] **Evidence that distinguishes it from the alternatives you
      considered.** "Consistent with X" is not a diagnosis; the repo rule is
      no repro, no fix, and it applies to diagnoses too.
- [ ] A prediction the hypothesis makes, tested. If the mechanism is what
      you say, something else should follow — show it does.
- [ ] Whether the 15-35 m pitch and the documented ~100 m per-vertex
      sawtooth are the same mechanism. Yes or no, with reasoning.
- [ ] Does it affect **all four** boundary classes for one shared reason,
      or are river bank / lake shore / ecotone / snowline separate causes
      that happen to co-occur in the same altitude window? This materially
      changes what a fix costs.
- [ ] A fix *direction* with a rough cost estimate — not a fix.
- [ ] `BUGS.md` B-19 updated with the diagnosis.
- [ ] An honest "I could not find it, here is what I ruled out and how"
      **is an acceptable deliverable.** This project respects that answer
      and it costs far less than a confident wrong one.

## Gates

You are not changing rendering, so the gate burden is light: **suites**,
plus `altitude_ladder.py` as your own instrument. If you end up touching
anything that renders, escalate to the world reel and say so.

Today's baseline: suites **8/8 PASS**, world reel **exit 0** 29/29,
convergence reel **exit 0** (blessed today), perf reel **exit 0**,
convergence flight `--tier deep` **exit 3** (known B-16, deliberately red).

Do not run the perf reel — it needs an idle machine and A-1 owns it this
shift. Never `--accept` anything.

## Declared human-verdict needs

**Andrew's, eventually — but not blocking, and not yours to collect.**

B-19 is an art-bar violation and Andrew decides how natural is natural
enough. His verdict gates the *fix*, not the *diagnosis*. Austin is routing
the specimen sheet to him separately.

If your diagnosis produces a **new** image that would help Andrew decide —
for instance a demonstration of what the boundary would look like with the
suspected mechanism disabled — put it in your evidence packet and say so.
That is high value. Do not block on it.

## Evidence packet

`docs/missions/X1-b19-findings.md` plus `X1-evidence/`:
the measurements you took, the frames that show the mechanism, the
prediction-test, and the alternatives you ruled out with the evidence that
ruled them out.

## You may reject this mission's premise

Explicitly, and there are two specific ways this premise could be wrong:

- **The measurement could be an artefact of the instrument.** V-2 built
  `altitude_ladder.py` and found this on its first run; a first run is
  exactly when an instrument's own bugs look like findings. If the lattice
  is in the measurement rather than in the game, **that is a major result**
  — report it loudly, it invalidates a `BUGS.md` entry.
- **The band may not be a defect at all.** Some quantisation between a
  voxel near-field and an LOD mesh is inherent. If the honest answer is
  "this is the expected consequence of the handoff and the fix is a
  different architecture", say that and let Andrew decide whether it reads
  badly. `TRANSITIONS.md`'s stance is that the switch does not have to be
  invisible — it has to look good.

**Marked assumptions you should check rather than trust:** that all four
boundary classes share one cause (nobody has checked); that the 0.5-1 km
window is sharp rather than an artefact of the ladder's rung spacing; and
that `c9cb081`'s B-17 fix is unrelated to this band rather than having
caused it — I have not looked, and it is the most recent change in the
neighbourhood.

## Feedback — optional but wanted

`## Feedback` is mandatory only when a mission uses or builds new tooling —
you will be using `altitude_ladder.py` heavily, so it applies. What the
spec got wrong, what cost you time, what tooling would have made this
faster, every assumption you made because this spec did not say, and
anything you noticed in passing even if unrelated.

Note, already filed by me — do not re-file: `viewer/CLAUDE.md`'s gate table
says `bash scripts/verify.sh (repo root)`; it is at `viewer/scripts/verify.sh`.

## Dispatch checklist — the four things that bite every worktree mission

1. **Your worktree is NOT at current main.** Merge main into it first, or
   you will lack the CLAUDE.md files, `docs/`, and the tools this spec
   tells you to read. (Symptom: tracked files appear "untracked".)
2. **A fresh worktree has no game assets.** `viewer/assets/*.bin` is
   gitignored (~170 MB). **Copy** it from the main tree — never symlink or
   junction. A recursive delete through a junction destroys the originals,
   which cost ~1.5 h of generation.
3. **`cargo build --release` does NOT build examples.** Say
   `cargo build --release --example play` explicitly.
4. **Do not end a turn waiting on a background process.** Poll directly.

**GPU note:** `altitude_ladder.py` uses settled `shot` captures, which are
timing-insensitive — you will not corrupt anyone's measurements and nobody
will corrupt your pixels. You will just run slower while others render.
A-1 is taking timing traces this shift; if it asks for a clear GPU, yield.

**Read before you start:** `TRANSITIONS.md` (the contract),
`LANDSCAPE.md` (the campaign this sits in), and `BUGS.md` B-19 and B-17.
Do **not** read `ITERATION_LOG.md` — it is large, historical, and this is
not archaeology.
