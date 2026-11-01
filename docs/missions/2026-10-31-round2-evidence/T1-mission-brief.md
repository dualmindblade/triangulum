# T1: planet scale (Astra)

You are working on Triangulum, a planetary voxel game (Rust + wgpu) by a father and son. Read these first, in
order: `CLAUDE.md`, `docs/missions/2026-10-31-round2.md` (your rows are **#3 and #2b**; read "Planet size" and
"Local and global" closely, and the rules at the end), `DESIGN.md`, `game/README.md`.

## Goal

1. **Sea level in the middle.** `sea_level = H/2` (1024 of 2048). The coarsest LOD voxel is H/2 (LOD 10,
   1024 m): the bottom voxel of a coarse column can be water and the top one air.
2. **Planet size in half-heights.** `world.json` states the planet's size as N / (H/2), today 128, in place of
   a raw column count. `--n` may stay for tests. Explain any rule that limits the value in the refusal.
3. **A planet-size slider** in the F1 look panel's terrain dials: from 128 to the Earth analogue (832), in
   steps of 32, with an upper limit of 1024. It shows the radius and vertical scale. The panel's terrain dials
   already build a preview world with saving off (`workshop.rs`, M3a), so his own world is never touched: use
   that mechanism, and *Save look* writes it to `world.json` like the other terrain dials. Andrew plays with
   this; it must be obvious what it does.
4. **Every size must really work.** The planet at 128, 256, 512, 832 and 1024: walking, flying, digging,
   picking, water, rivers and lakes, sky, atmosphere, fog, the map, orbit view, saves, a replay. Global
   features stretch with R; local ones stay in metres ("Local and global"). The vertical scale follows R
   (`vertical_scale: null`). Fog, atmosphere and anything else keyed to R or to a fixed distance need to look
   right at each size, not just not crash.

Things worth hunting, unproven: `f32` or precision trouble at R = 725 km; anything sized by a constant chart or
table; hydro or lake quantities held in radians or chord lengths that should be metres, or the reverse;
`atmosphere_height_radii` and `fog_per_radius`; streaming and coverage tree limits; memory and quad buffer
budget; `PosKey`; LOD count tables (`[u32; 16]` is fine for now, but check); coarse-voxel special cases that say
"L9 at sea level 512" (`block_at` has one); ocean depth clamping against the bottom of the world;
`Generator::height_margin_m`.

## Coupling with other missions

Four other agents are at work. T2 (Fable) is adding real mountains and writing the local/global split into the
generator, so `landform.rs`, `hydro.rs::shape` and `Hydro::new` are moving: don't restructure them. If you find a
local feature that scales with R, make the smallest fix that's right, or note it in your findings for the lead.
T3 (Opus) is changing `surface()` and boundary noise. C1 (Opus) is changing `app.rs`, `player.rs`, `camera.rs`.
C2 (Fable) is changing `sim/`.

**Absolute layer numbers.** Moving sea level shifts every layer by +512. Shift them in `game/scripts/*.play`
and any `.play` inside `game/src` tests, and add `game/scripts/shift_layers.py` (idempotence is not needed, but
it must only touch absolute layer numbers: `atblock`, `block[f,i,j,k]`, and whichever other commands take a
layer) so the lead can apply it to scripts other missions add at merge. Unit tests that pass their own
`Planet::new(.., 512, ..)` can stay. Think hard about which scripts' assumptions change beyond the number, such
as a test for the bottom-voxel ocean.

## This environment

- Your worktree is `E:/triangulum-work/T1`, branch `T1-planet-scale`, from main at 18d69c7. Windows 10; the
  game's build config is already written at `game/.cargo/config.toml` and the bake is already copied to
  `game/assets/world/seed42`. Build inside your workspace (`game/target`, the default): ignore the round page's
  `target-dir` line. `game/out/` exists. Use `cargo --offline`.
- The frozen baseline build is `E:/triangulum-build/baseline-8134c61/triangulum.exe`, for "before" evidence.
- You can't commit in a linked worktree here: leave your work in the tree, uncommitted, and the lead commits it.
- Headless only. No real input, and no normal game window: see "Don't disturb the machine".
- Other agents are running, so timings are provisional: label them. Give the bench at each planet size all the
  same; the lead re-takes them on an idle machine.

## Deliver

`docs/missions/T1-planet-findings.md`, with evidence in `docs/missions/T1-planet-evidence/` under 30 MB (frames
at each planet size: same place on the planet, from the ground, from 2 km, from orbit). `game/README.md` updated.
Proposed wording for `DESIGN.md`. In your final message: what you verified and how, what you didn't, and anything
Andrew has to decide. Be plain about what you saw. The lead re-verifies everything before merging.
