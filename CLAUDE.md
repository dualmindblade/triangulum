# Triangulum

A planetary voxel game built by a father and son. **Austin** sets priorities
and plays the builds. **Andrew** is the art director and has final say on how
things look. **Claude (Opus)** leads the agents.

## Layout

- `game/` — the new engine (Rust + wgpu). New work goes here.
- `crates/cubesphere/` — cube-to-sphere grid maps, shared by the engine and tools.
- `planetgen/` — the world generator (Python). Its output,
  `output/<world>/planet_data.npz`, is gitignored and takes ~1.5 h to regenerate.
- `scripts/` — bake scripts that turn planetgen output into engine assets.
- `viewer/` — the old viewer. Frozen, kept for reference and comparison.
- `docs/archive/` — the July docs. Don't read them unless a task says to.

## Read first

`PLAN.md` (milestones), `STATUS.md` (where things stand), `DESIGN.md`
(decisions Austin and Andrew made; don't contradict them without asking).

## Rules

1. **Done means someone can see or do it in a build.** Every task ends with
   evidence (a screenshot, a bench number, a test) and says plainly what was
   not verified. "I could not verify this" is a fine answer; an unverified
   claim is not.
2. **Performance target:** 60 fps at 1920×1080 on the RTX 2060 (Ryzen 7
   2700X). Timing runs need the machine to themselves; if another agent is
   running, label your numbers provisional.
3. **Determinism:** world generation is a pure function of (seed, position);
   simulation is fixed-step and deterministic. Saves store changed chunks.
4. **Generated data is not in git** (bakes, saves, bench output). Copy assets
   into a fresh worktree; never symlink or junction them.
5. Windows 10. The Bash tool is usually easier than PowerShell. Kill only
   processes you started.
6. **Licences:** the game is MIT. Cubyz (GPLv3) is ideas only. Seeds of
   Andromeda (MIT) code may be borrowed with attribution.
