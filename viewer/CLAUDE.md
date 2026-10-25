> **Legacy (frozen 2026-10-24).** New work happens in `game/`; see the root
> `PLAN.md`. Docs referenced below now live in `docs/archive/`.

# viewer/ — the game (Rust + wgpu)

Renders Neisor and its moon on a gnomonic cube-sphere: an LOD quadtree mesh
far out, real voxel chunks near the ground, one hierarchy driving both LOD and
procedural generation.

## Build traps (each of these has cost someone hours)

- **`cargo build --release` does NOT build examples.** The play harness is an
  example. After editing `src/`, rebuild it explicitly:
  `cargo build --release --example play`
  Skipping this means you verify a **stale binary** and misread working fixes
  as broken. This has happened more than once.
- A **fresh worktree has no `viewer/assets/`** — the world data is gitignored.
  Copy it from a populated tree (see root `CLAUDE.md`).
- Kill stale `triangulum-viewer` / `play` processes first; a locked binary
  fails the link step.

## Invariants — do not "fix" these, they are deliberate

- **Reversed-Z with an infinite far plane.** `Greater` depth compare, clear to
  0, sky triangle at z=0, `fs_sky` unprojects the NEAR plane. Never
  reintroduce a finite far plane or `Less` compares.
- **Sea is `ocean-mask AND elevation <= 0`, never elevation alone.** Neisor has
  genuine dry below-sea-level basins; testing elevation alone floods them.
- **River/lake levels in `rivers.bin` are the HYDRAULIC FILL SURFACE**
  (`max(elev, receiver-level)` from terminals, oceans pinned 0), never
  bed-anchored. See `WATER-CONTRACT.md` before touching any of it.
- **Voxels are gnomonic-anisotropic** (~1.7 × 1.0 m at face centers). That is
  the 10M-columns spec, not a bug. Do not "correct" it.
- **`ColCtx.ground0` is the natural pre-edit terrain.** Strata, steepness,
  caves and trees all anchor to `ground0`; edited columns never host trees.
- **Everything visible is a pure function of (seed, position, absolute time)**
  and edits are journaled. That is what makes recordings replay exactly — a
  change that breaks this property breaks the entire verification stack, so
  treat it as a contract, not an implementation detail.
- Clouds must never fully hide the ground from orbit.

## The trap that has fooled two different models

**Flat terrain rendered as voxel chunks looks smooth and mesh-like.** Each
column top is a single 1 m quad, so coplanar neighbours read as a faceted
sheet, and gentle biomes (savanna especially) look exactly like the distant
LOD mesh. Two separate investigations concluded "this is mesh, not voxels" and
**both were wrong.** Before ever making that claim, prove it with the magenta
diagnostic (paint `rel_flag.w > 0.5` fragments magenta and look at what is
actually mesh). The harness *does* render the voxel near-field — `capture()`
blocks until `chunk_pending` drains.

## The play harness

`examples/play.rs` — scripted, headless, deterministic (fixed 60 Hz timestep),
running the game's own player physics and renderer. **Read its header doc
comment for the authoritative command list.** Runs land in
`interchange/runs/<script-stem>/` as frames + JSON state sidecars + transcript.

- `assert FIELD OP VALUE` makes a script self-checking — a failure exits the
  run non-zero, so a survey fails loudly with nobody reading frames. The null
  literal is the word `none`.
- `shot` captures a **settled** frame (waits for streaming to drain).
  `TRI_POSE_LIVE=1` with `pose`/`posehz` instead draws **live** frames with the
  streaming scheduler running as in gameplay — settled captures hide streaming
  bugs, so use live replay for anything time-domain.
- **`TRI_POSE_LIVE=1` and `TRI_RAW_CAPTURE=1` compose**, and together they are
  the complete live-frame sampler: every `shot` becomes a photograph of the
  actual in-flight frame (one draw, no settle, no drain). That pairing is what
  the convergence reel is built from. Nothing else hints that they combine —
  the gap cost two separate sessions the discovery.
- **`sun LAT LON` sets a GLOBAL sun direction**, so a far-longitude teleport
  afterwards renders at *night*. Omit `sun` in surveys (the default lights
  every location at local noon), or derive it per-pose the way
  `scripts/world_reel.py` does.
- F10 in-game records a real flight to `interchange/recordings/rec_NNN.play`.
  Hand that file to the harness and it replays exactly what the human saw.
- `scripts/repro_shot.py <shot.png>` turns any screenshot back into an exact
  `.play` script. Human bug reports are one command from reproducible.

## Gates

Run from `viewer/` unless noted. Which ones apply depends on blast radius —
see `docs/PROCESS.md`.

**`verify.sh` lives at `viewer/scripts/verify.sh`, not the repo root**, and on
this machine bare `bash` may resolve to a WSL shim with no distribution. If it
does, invoke Git's bash as a *login* shell (it needs `dirname` on PATH):
`"C:\Program Files\Git\bin\bash.exe" -lc 'cd /c/code/triangulum/viewer && ./scripts/verify.sh'`

**The blessed baselines these gates compare against are tracked in git**
(`viewer/interchange/*/accepted*`). A worktree therefore has them; you only
need to copy `viewer/assets/`. `--accept` produces a reviewable diff — which
is why blessing stays a human decision.

| Gate | Command | Guards |
|---|---|---|
| suites | `bash scripts/verify.sh` | physics/geometry invariants, 8 asserting suites |
| world reel | `python scripts/world_reel.py` | pixel changes vs blessed poses, + VOID/GRID lint |
| perf reel | `python scripts/perf_reel.py` | frame cost vs blessed baseline |
| convergence reel | `python scripts/convergence_reel.py` | **time-domain**: does the live view actually reach the settled one, and how fast. Self-blessing — the settled frame is its own reference |
| sky reel | `python scripts/sky_reel.py` | sky/atmosphere appearance |
| sync diff | `python scripts/sync_diff.py` | mesh↔voxel agreement (`sea_calib` must stay 0.0%) |
| census | `cargo run --release --example census` (planet-wide; `-- --at LAT LON --radius KM` for one site) | water/terrain discontinuities at planet scale |
| colcensus | `cargo run --release --example colcensus -- LAT LON [RADIUS_KM]` (site-local; **requires coordinates**) | block-truth water; sees cave/film water samplers miss |

Reels use the same convention: exit **0** clean, **3** findings to review,
`--accept` to bless. Blessing is a human decision — an agent may *propose* a
re-bless with an explanation per changed pose, never perform one silently.

**Intended visual changes mean intended reel diffs.** A changed pose is not a
failure; an *unexplained* changed pose is.

## Diagnostics

`TRI_LOD_RING_DEBUG` (false-color LOD levels), `TRI_STREAM_DEBUG`,
`TRI_GPU_TIMERS`, `TRI_RAW_CAPTURE`, `TRI_NO_IMPOSTORS`, `TRI_NO_MORPH`,
`TRI_FORCE_MORPH`, `TRI_NO_ROCKS`, `TRI_POSE_LIVE`, `TRI_POSE_RENDER`,
`TRI_BUILD`, `TRI_IMPOSTOR_STATS`.

New diagnostics follow the `TRI_LOD_RING_DEBUG` pattern: a standing read-only
probe, provably inert when off, production bit-identical.

## Rust notes

- `par_iter().step_by(n)` **serializes** the rayon pool. Use
  `.filter(|i| i % n == 0)`.
- The big files (`planet.rs`, `renderer.rs`, `terrain.rs`, `shader.wgsl`,
  `main.rs`) are 120-210 KB each and are the main merge-collision risk. Two
  missions must not share one.

## After changing the bakes

`scripts/bake_faces.py`, `bake_rivers.py`, and `bake_weather.py` all feed the
viewer. Changing one means re-running it and keeping `src/planet.rs`'s layout
in sync (`face_*.bin` = n*21 bytes: elev f32, koppen u8, rough/precip/temp/flow
f32).
