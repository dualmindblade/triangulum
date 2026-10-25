# Mission D-1 — B-20: re-aim the dead poses, then make every reel assert its own premise

**Lane D (tooling).** You own `viewer/scripts/world_reel.py` and the reel
pose tables. **Do not touch `viewer/src/renderer.rs`** (mission A-1 owns it)
or **`viewer/examples/play.rs`** (mission D-2 owns it).

The pose re-aim is the visible half. **The systemic half is the point** —
part (b) below is what would have caught this on day one, and it is what I
actually want out of this mission.

---

## Why

Two of the world reel's 29 blessed poses contain nothing resembling what
they are named for. Found by V-2, then independently re-verified by the
previous coordinator with `probe` and by looking at the blessed frames:

- **`lake_archi`** (16.601 −12.222 @3.0 km) is a **featureless blue
  rectangle**. Probes at the pose and at ±0.05-0.08° are all `lake=true`,
  water 84.5 m, ground 51-62 m — fully submerged, no shore anywhere in
  frame.
- **`coast_beach`** (8.097 −35.673 @0.06 km) is **dry highland grassland**
  at 923-963 m with no water within several km. No coast. No beach.

Both are **blessed**, so they have been passing on every merge and
reporting green for the lake-shore and coastal classes they are named for
while containing neither. That is 2 of 29 poses — 7 % of the world reel —
guarding nothing.

Worse: `coast_beach`'s blessed frame shows **visible terracing contour
rings**. The reel certified an artefact as correct by blessing it.

The escaped-gate answer is the interesting part: **nothing in this repo
verifies that a gate is pointed at what it claims to guard.** Same class of
hole as the perf reel warming before measuring (`PERF.md` item 1). I ran
the world reel today: 29/29 clean, exit 0 — including both dead poses.

## What already exists — do not reinvent any of this

| Thing | Where | What it gives you |
|---|---|---|
| the reel itself | `viewer/scripts/world_reel.py` | 29 poses, pixel diff vs blessed, VOID/GRID lint, per-pose sun derivation |
| `probe LAT LON` | play harness command, `examples/play.rs:1062` | in-run, milliseconds: `h`, `water`, `lake`, `frozen`, annual/seasonal temp, lake/pond level, river hw/wet, and column truth (ground / water / cave_water) |
| `sync_diff.py --at LAT LON ALT` | `viewer/scripts/` | ad-hoc pose divergence |
| `repro_shot.py <shot.png>` | `viewer/scripts/` | any screenshot → exact `.play` script |
| the other pose tables | `perf_reel.py`, `sky_reel.py`, `convergence_reel.py`, `convergence_flight.py`, `altitude_ladder.py` | the audit targets |
| V-2's findings | `docs/missions/V2-altitude-ladder-findings.md` | how the dead poses were found |

**`probe` does NOT print koppen today** (VERIFIED — I read the trace
format). Mission D-2 is adding that concurrently. **Do not add it yourself**
— you would collide. Assert on what `probe` returns now: `water`, `lake`,
`frozen`, `h`, and column truth are enough for every pose in the world reel.
If you genuinely need koppen for a specific assertion, say so in your
findings and leave that assertion for a follow-up.

## What to do

### (a) Re-aim or replace the two dead poses

Find coordinates that actually contain a lake shore and an actual
coastline. Use `probe` to confirm **before** rendering — that is the whole
lesson here. Show the probe output for each candidate.

`coast_beach`'s terracing rings: **do not quietly bless them away.** Decide
whether they are a real artefact and, if so, file a `BUGS.md` entry with a
repro. Blessing a frame with a visible defect is how this happened the
first time.

### (b) Make a reel assert its own premise — the systemic fix

A reel pose should have to **contain its class** or the reel fails at
generation time, loudly, before any pixel is compared. `probe` returns in
milliseconds so this is nearly free.

I am deliberately **not** specifying the mechanism. State the invariant you
are enforcing and pick the implementation — the last two missions were
harmed by specs that prescribed an algorithm (V-2 spent ~90 minutes
implementing a metric that turned out to anti-correlate). The observation I
want is: *a pose named for a class contains that class, and this is checked
automatically, not by a human reading a name.*

Consider whether the assertion belongs to the pose table, the reel runner,
or a shared helper — and note that V-1 recommended **deferring** a
`reel_common.py` refactor until a fifth reel exists. Do not build one as a
side effect.

### (c) Audit every other pose table

`perf_reel`, `sky_reel`, `convergence_reel`, `convergence_flight`,
`altitude_ladder`. The previous coordinator's stated suspicion:
*"the other reels' pose tables are probably as rotten as the world reel's.
Medium-high confidence; cheap to check with `probe`."*

**That is a suspicion, not a finding — treat it as a hypothesis to test,
not a conclusion to confirm.** If they are all fine, that is a real and
welcome result; report it as such.

Note `convergence_reel`'s table is only 4 poses and its `b16_ice_hold` is
load-bearing for an open bug — if you find a problem there, flag it, do not
unilaterally re-aim it.

### (d) One bug in an existing gate, while you are in the file

`world_reel.lint_grid`'s water mask is `b > g + 8`, which **misses
turquoise shoal water** (~90,180,180). So the lint is silently not looking
at shallow water — precisely where the shading seams it was written to
catch would appear. Reported by V-2. It is a correctness hole in a gate.

## Done criteria

- [ ] Both poses re-aimed (or replaced), each with `probe` output proving
      the new pose contains its class.
- [ ] A verdict on `coast_beach`'s terracing rings — filed as a bug with a
      repro, or explained away with evidence.
- [ ] Reel poses assert their own premise automatically; a deliberately
      wrong pose fails loudly. **Demonstrate this** — point the check at a
      known-dead pose and show it firing. No repro, no fix.
- [ ] Every other pose table audited, with a per-table verdict.
- [ ] `lint_grid`'s water mask fixed, with evidence it now sees shoal water.
- [ ] `BUGS.md` B-20 closed or amended.

## Gates

Tooling blast radius, so per PROCESS §6: **suites, plus whatever your tool
is supposed to detect.** For you that means the world reel itself, before
and after.

Today's baseline on this tree: suites **8/8 PASS**, world reel **exit 0**
29/29 clean, convergence reel **exit 0** (blessed today), perf reel **exit
0**, convergence flight `--tier deep` **exit 3** (known B-16, left red
deliberately).

**Never `--accept` anything.** Blessing is a human decision — an agent may
*propose* a re-bless with an explanation per changed pose, never perform
one. Your re-aimed poses will need fresh blessed frames; propose them, do
not bless them.

## Declared human-verdict needs — YES, and this is the one mission that has them

Per PROCESS §2 these must be declared up front, so:

1. **New blessed frames for the two re-aimed poses** need a human glance.
   Austin at gate 2.
2. **The terracing rings** may be an art-facing call. If you conclude they
   are a real artefact, Andrew has final say on how it should look —
   produce a frame built for his eyes and park it, do not block on it.

Anything else you discover that needs eyes, declare it in your findings as
early as you can rather than at the end.

## Evidence packet

`docs/missions/D1-reel-premise-findings.md` plus `D1-evidence/`:
probe transcripts for old and new poses, before/after frames for both,
the demonstration of the premise check firing on a known-dead pose, and the
per-table audit results.

## You may reject this mission's premise

Explicitly. In particular:

- If the premise check turns out to be the wrong instrument — for instance
  if pose classes cannot be expressed as probe assertions without becoming
  a second, drifting source of truth — **say so and propose better.** A
  brittle assertion layer that people learn to disable is worse than none.
- If the other pose tables are clean, say so plainly. The suspicion in the
  handoff is medium-high confidence, not a fact.

**Marked assumptions you should check rather than trust:** that both dead
poses are best *re-aimed* rather than *replaced* — I have not looked for
better coordinates and have no opinion; and that 29 poses is the right size
for the world reel at all, which nobody has revisited.

## Feedback — mandatory

You are working on tooling, so `## Feedback` is **required** per PROCESS §9.
What the spec got wrong, what cost you time, what tooling would have made
this faster, every assumption you made because this spec did not say, and
anything you noticed in passing even if unrelated. Be specific; vague praise
is not feedback.

Note for your feedback section, since it will come up: the gate table in
`viewer/CLAUDE.md` says `bash scripts/verify.sh (repo root)`. It is at
`viewer/scripts/verify.sh`. Already filed by me — do not re-file it.

## Dispatch checklist — the four things that bite every worktree mission

1. **Your worktree is NOT at current main.** Merge main into it first, or
   you will lack the CLAUDE.md files and the tools this spec tells you to
   read. (Symptom: tracked files appear "untracked".)
2. **A fresh worktree has no game assets.** `viewer/assets/*.bin` is
   gitignored (~170 MB). **Copy** it from the main tree — never symlink or
   junction. A recursive delete through a junction destroys the originals,
   which cost ~1.5 h of generation.
3. **`cargo build --release` does NOT build examples.** Say
   `cargo build --release --example play` explicitly.
4. **Do not end a turn waiting on a background process.** Poll directly.

**GPU note:** one GPU, several missions this shift. Your work is
settled-frame pixel comparison, which is timing-insensitive — you will not
corrupt anyone and nobody will corrupt you. You will just be slower when
others are rendering. Do not run the perf reel; it needs an idle machine
and A-1 owns that gate this shift.
