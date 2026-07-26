# Triangulum

A planetary-scale voxel game built by a father and son. `planetgen/` (Python)
grows a whole world from one seed; `viewer/` (Rust + wgpu) renders it from
orbit down to one-meter blocks underfoot.

**Andrew is the art director and has final say on how things look.** Austin
sets priorities and is the human verification gate. Everything else is agents.

## The one rule that matters here

**No repro, no fix.** Every change ships with a harness sequence that failed
before and passes after. A claim without evidence is not a result. If you
cannot produce evidence, say so plainly — "I could not verify this" is a
useful, respected answer. A confident claim that turns out to be unverified
costs this project far more than an honest gap.

Verification is the bottleneck, not implementation. Budget accordingly.

## Read-if-you-touch-it routing

Do NOT read everything. This repo has ~250 KB of design prose; read the root
of what you touch and follow these triggers:

| If your work touches | Read first |
|---|---|
| water, lakes, rivers, shores, terrain sampling | `WATER-CONTRACT.md` (invariants + the gate enforcing each) |
| frame cost, hitches, streaming, scheduling | `PERF.md` |
| the mesh↔voxel boundary, LOD morph, popping | `TRANSITIONS.md` |
| weather, clouds, precipitation, seasons | `WEATHER.md` |
| the moon, orbits, sky bodies, eclipses | `SOLAR.md` |
| landscape/terrain feature work | `LANDSCAPE.md` (the campaign and its tracks) |
| anything with a look Andrew must approve | `DECISIONS.md` (his open queue) |
| the viewer at all | `viewer/CLAUDE.md` |
| planetgen at all | `planetgen/CLAUDE.md` |
| learning Rust as a newcomer | `RUST-PRIMER.md` |

`BUGS.md` is the durable cross-operator bug ledger (humans, Claude, Sol,
Opus). Check it before hunting anything so you do not re-report a known
issue. `ITERATION_LOG.md` is a narrative chronicle — **do not read it** unless
you are doing deliberate archaeology; it is large and mostly historical.

`docs/PROCESS.md` is how work actually moves: shifts, lanes, gates, review.

## Facts that stay true

- **Generated data is not in git.** `output/`, `viewer/assets/*.bin`,
  `meta.json` are gitignored. A fresh clone or a **fresh worktree has no
  world to render** — copy `viewer/assets/` from a populated tree. Copy;
  never symlink or junction (a recursive delete through a junction destroys
  the originals, which cost ~1.5 h of generation).
- Windows, PowerShell 5.1. The Bash tool is usually easier. Embedded double
  quotes in PS here-strings get mangled — keep commit messages free of `"`.
- Kill stale `triangulum-viewer` / `play` processes before a build, or the
  linker fails on a locked binary.
- Licensed MIT, deliberately: the game stays free.

## Working agreements

- One worktree per mission; small, frequent merges; the coordinator merges
  serially to main.
- Every mission ends with an evidence packet and a `## Feedback` section
  (see `docs/PROCESS.md`). Feedback is optional normally, **mandatory when
  you used or built new tooling**.
- Propose CLAUDE.md changes into `docs/inbox/`; only Austin commits them.
  If you find yourself being told the same correction twice, that correction
  belongs in a CLAUDE.md — file it.
