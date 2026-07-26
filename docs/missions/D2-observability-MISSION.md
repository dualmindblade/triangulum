# Mission D-2 — the harness observability bundle

**Lane D (tooling).** You own `viewer/examples/play.rs`. **Do not touch
`viewer/src/renderer.rs`** — mission A-1 owns it this shift and it is the
single worst merge-collision file in the repo. **Do not touch
`viewer/scripts/world_reel.py` or the reel pose tables** — mission D-1 owns
those.

Four small items, requested by three separate missions. None is hard. The
value is that they compound: every future time-domain or field-report
mission gets cheaper.

---

## Why

Three consecutive missions independently asked for pieces of this. When
three missions ask for the same thing, it stops being a nice-to-have.

## The four items

### 1. `pick X Y` — dump the ground lat/lon under a screen pixel

**Requested by V-2, which called it "the single most useful function in my
script".** V-2 had to reconstruct the projection in Python from `camera.rs`
to get it.

This is the tool that lets any agent or human point at a defect in a
screenshot and get coordinates back. It **composes with `repro_shot.py`**
(screenshot → exact `.play` script), which turns "this looks wrong" into a
reproducible repro automatically. That composition is the reason this is
first on the list.

Add it as a play-harness command alongside `probe`. Trace output in the
same style — this harness's convention is that sidecar/transcript text
beats burning information into pixels.

Think about the degenerate cases and say what you chose: pixel on the sky,
pixel past the horizon, pixel on water vs terrain, pixel on the moon.

### 2. `probe` should print `koppen`

**Requested by V-2**, which had to parse `assets/face_*.bin` directly to
scout a biome ecotone, duplicating `planet.rs`'s loader.

VERIFIED today: `probe` currently prints `h`, `water`, `lake`, `frozen`,
annual/seasonal temp, lake/pond level, river hw/wet, and column truth
(ground / water / cave_water). No koppen. `face_*.bin` is
`n*21 bytes: elev f32, koppen u8, rough/precip/temp/flow f32` — the loader
is in `src/planet.rs`.

**Mission D-1 is depending on this landing** and has been told to assert on
water/lake/elevation in the meantime. Getting koppen in early helps it.

### 3. A first-pose trace line, and optional wall-clock timestamps in `run.log`

**Requested by V-1 and again by V-3.** `pose` and `posehz` emit no trace
lines at all, and `run.log` has no timestamps. Every other harness command
traces.

This cost V-1 a genuine bug: its wall-clock anchor waited for a pose trace
that never comes. V-3 ended up correlating Python-side stdout timestamps
against `frame_counter` in a separate stderr stream to line the
`TRI_STREAM_DEBUG` trace up with its samples.

Silence is *correct* for a 500-line recording — do not make `pose` chatty.
What is wanted is:
- a **single** first-pose trace line, as a "sim started here" anchor; and
- an **opt-in** wall-seconds prefix on `run.log` lines behind an env var.

### 4. Fix `play.rs`'s header doc comment

It claims to be the authoritative command list. It omits **`pose`,
`posehz`, `bench` and `glide`** — the four commands this entire
verification stack is built on. VERIFIED today: all four exist
(`play.rs:795, 799, 863, 1011`); none is documented.

`bench N` in particular **renders N frames without advancing `sim_ticks`**
— a genuinely useful primitive (give the streamer wall time without moving
the clock) that is documented nowhere and that V-3 had to learn by reading
the match arm.

Document whatever else you find missing while you are in there. `viewer/CLAUDE.md`
points at this header as the authoritative list, so it needs to actually be one.

## What already exists — do not reinvent any of this

| Thing | Where |
|---|---|
| the harness and its command dispatch | `viewer/examples/play.rs` |
| `probe`'s implementation, the model to follow | `play.rs:1062` |
| the projection you need for `pick` | `viewer/src/camera.rs` (reversed-Z, infinite far plane — see the invariants in `viewer/CLAUDE.md`) |
| `face_*.bin` layout + loader | `viewer/src/planet.rs` |
| screenshot → repro | `viewer/scripts/repro_shot.py` |
| V-2's Python reconstruction of the projection | `viewer/scripts/altitude_ladder.py` — it already solved this once; read it before deriving anything |

**Do not break `TRI_POSE_LIVE` / `TRI_RAW_CAPTURE` / `TRI_STREAM_DEBUG`
composition.** All three compose and the entire convergence stack depends
on it.

## Done criteria

- [ ] `pick X Y` works, with a demonstration: pick a pixel in a real frame,
      get coordinates, feed them to `probe`, and show they agree with what
      is visibly at that pixel. **No repro, no fix** — a `pick` that is
      silently 200 m off is worse than none.
- [ ] Degenerate cases named and handled (sky, horizon, water, moon).
- [ ] `probe` prints koppen; show it distinguishing two known-different biomes.
- [ ] First-pose trace line lands; a 500-line recording stays quiet.
- [ ] Wall-clock prefix works behind an env var and is off by default.
- [ ] `play.rs` header documents `pose`, `posehz`, `bench`, `glide` and
      anything else missing, including `bench`'s no-sim-advance behaviour.
- [ ] **Determinism is intact**: a double run is byte-identical wherever the
      system claims to be deterministic. This is a standing requirement and
      you are touching the harness everything else is verified with.

## Gates

Tooling blast radius: **suites, plus whatever your tool is supposed to
detect.** Because you are changing the harness itself, also run the **world
reel** — it is the cheapest proof you did not perturb capture behaviour.

Today's baseline: suites **8/8 PASS**, world reel **exit 0** 29/29 clean,
convergence reel **exit 0** (blessed today), perf reel **exit 0**,
convergence flight `--tier deep` **exit 3** (known B-16, deliberately red).

Do not run the perf reel — it needs an idle machine and A-1 owns it.
Never `--accept` anything.

## Declared human-verdict needs

**None.** Every item here is checkable by instrument. If you find one that
is not, say so early rather than at the end.

## Evidence packet

`docs/missions/D2-observability-findings.md` plus `D2-evidence/`:
the `pick` accuracy demonstration, a koppen probe transcript across two
biomes, a before/after `run.log` fragment showing the anchor line and
timestamps, and the double-run determinism check.

## You may reject this mission's premise

Explicitly. In particular:

- If `pick` cannot be made accurate enough to be trustworthy — reversed-Z
  with an infinite far plane is not a friendly projection to invert — then
  **say so and ship it with a stated accuracy bound**, or do not ship it.
  A tool whose error is unstated is a trap.
- If the wall-clock timestamp option turns out to perturb timing, drop it
  and say why.

**Marked assumptions you should check rather than trust:** that these four
items are independent (I believe they are, but only items 1 and 2 have been
looked at closely); and that `pick` belongs in `play.rs` rather than as a
script — I chose the harness because it composes with `probe`, but if the
code says otherwise, follow the code.

## Feedback — mandatory

You are building new tooling, so `## Feedback` is **required** per PROCESS
§9. Specifically valuable from this mission: **`play.rs` is where every
other mission's friction shows up.** If you notice commands that are
undocumented, misnamed, silently no-op, or that trap the unwary, say so —
you are the mission most likely to see them. Plus the standard asks: spec
problems, time sinks, assumptions you had to make, anything surprising.

Note, already filed by me — do not re-file: `viewer/CLAUDE.md`'s gate table
says `bash scripts/verify.sh (repo root)`; it is at `viewer/scripts/verify.sh`.

## Dispatch checklist — the four things that bite every worktree mission

1. **Your worktree is NOT at current main.** Merge main into it first, or
   you will lack the CLAUDE.md files and the tools this spec tells you to
   read. (Symptom: tracked files appear "untracked".)
2. **A fresh worktree has no game assets.** `viewer/assets/*.bin` is
   gitignored (~170 MB). **Copy** it from the main tree — never symlink or
   junction. A recursive delete through a junction destroys the originals,
   which cost ~1.5 h of generation.
3. **`cargo build --release` does NOT build examples.** You are editing an
   example — say `cargo build --release --example play` explicitly, every
   time, or you will test a stale binary and misread working code as broken.
4. **Do not end a turn waiting on a background process.** Poll directly.

Kill stale `triangulum-viewer` / `play` processes before a build or the
linker fails on a locked binary.

**GPU note:** your renders are short and settled; you will not corrupt
anyone's measurements. A-1 is taking timing traces this shift — if it asks
for a clear GPU, yield.
