# T1 — planet scale

Worktree `T1-planet-scale`, based on `18d69c7`. Changes are uncommitted, as
requested. `main` had the same commit. The requested merge attempt could not
write the linked worktree's `ORIG_HEAD`; no merge or commit was made.

The five requested sizes render and complete the play/replay checks. The
full suite passes (374 tests), all 39 original play scripts pass with their
documented flags, and the five identical benches are recorded. One Vulkan
teardown failure remains unexplained; see below. T2/T3's new terrain and the
other missions are not merged into this worktree.

## Frames and what I saw

Start with the [F1 panel at 832](T1-planet-evidence/panel-832.png). The radius,
vertical scale, Earth analogue button, world-height limits and preview/save
behaviour are all visible. The same latitude/longitude (−4.5654, 117.4744),
view angles and local noon are used throughout the following matrix.
Ground means standing on the actual blocks; the flight pose sets feet to
2000 m (eyes 2001.62 m). Orbit sets feet to 1.75 R above sea level, keeping
the apparent planet size comparable. JSON sidecars contain the exact poses,
clock, look and world parameters.
The curated evidence is 9.95 MB, below the 30 MB limit. Raw videos, saves
and the remaining bench frames stay under `game/out/`; they are not in the
evidence package.

| Half-heights | Radius | Vertical scale | Ground | 2 km | Orbit | Map |
|---|---|---|---|---|---|---|
| 128 | 90.569 km | 1/70.344 | [frame](T1-planet-evidence/128-ground.png) | [frame](T1-planet-evidence/128-two_km.png) | [frame](T1-planet-evidence/128-orbit.png) | [frame](T1-planet-evidence/128-map.png) |
| 256 | 181.138 km | 1/35.172 | [frame](T1-planet-evidence/256-ground.png) | [frame](T1-planet-evidence/256-two_km.png) | [frame](T1-planet-evidence/256-orbit.png) | [frame](T1-planet-evidence/256-map.png) |
| 512 | 362.277 km | 1/17.586 | [frame](T1-planet-evidence/512-ground.png) | [frame](T1-planet-evidence/512-two_km.png) | [frame](T1-planet-evidence/512-orbit.png) | [frame](T1-planet-evidence/512-map.png) |
| 832 | 588.700 km | 1/10.822 | [frame](T1-planet-evidence/832-ground.png) | [frame](T1-planet-evidence/832-two_km.png) | [frame](T1-planet-evidence/832-orbit.png) | [frame](T1-planet-evidence/832-map.png) |
| 1024 | 724.554 km | 1/8.793 | [frame](T1-planet-evidence/1024-ground.png) | [frame](T1-planet-evidence/1024-two_km.png) | [frame](T1-planet-evidence/1024-orbit.png) | [frame](T1-planet-evidence/1024-map.png) |

Before frames use the frozen `baseline-8134c61` executable at the same poses:
[128 ground](T1-planet-evidence/before-128-ground.png),
[128 at 2 km](T1-planet-evidence/before-128-two_km.png),
[128 orbit](T1-planet-evidence/before-128-orbit.png),
[1024 ground](T1-planet-evidence/before-1024-ground.png),
[1024 at 2 km](T1-planet-evidence/before-1024-two_km.png),
[1024 orbit](T1-planet-evidence/before-1024-orbit.png).

The original planet still has a dark, starry sky at 2 km. The larger planets
now retain blue-grey air there; the frozen build kept the thin 640 m
atmosphere at every size. From orbit the continents remain in the same
places, coastlines get finer relative to the disc, and the atmosphere keeps
similar angular thickness. The ground frames still show the existing low,
terraced hills; T2's mountains are not present. The strips around the ocean
in the [intermediate 128 frame](T1-planet-evidence/before-horizon-128.png)
are absent in the final 128 orbit frame. I did not retune the sky colours or
the existing gradient's appearance for Andrew.

## Verification

- `cargo test --offline --release -- --test-threads=4 --nocapture`:
  **374 passed, 0 failed, 26 ignored**. [Full log](T1-planet-evidence/tests.log).
  After that run, a passive-panel test exposed egui clamping a disabled
  automatic vertical scale to 0.01 in a half-metre world. Sliders now clamp
  only on edits. The final workshop test run is **10 passed, 0 failed,
  1 ignored**, including the added regression:
  [before](T1-planet-evidence/passive-panel-before.log),
  [after](T1-planet-evidence/workshop-tests.log). The final binary
  [builds offline](T1-planet-evidence/build-final.log). The final two-line UI guard was checked by these affected tests,
  not by another complete suite run.
- `t1-size.play`, at 128, 256, 512, 832 and 1024: **26 assertions per size**,
  no errors. Walking, flying, aiming, breaking/replacing a block, pouring
  water, swimming/submersion, save/reload, generated rivers/lakes and live
  streaming coverage are exercised. Each records and checks a 141-step
  replay: **3/3 checkpoints and the end hash match**, with the look matching.
  Logs: [128](T1-planet-evidence/128-play.log), [256](T1-planet-evidence/256-play.log),
  [512](T1-planet-evidence/512-play.log), [832](T1-planet-evidence/832-play.log),
  [1024](T1-planet-evidence/1024-play.log).
- **39/39 original `.play` scripts pass** in their final runs, including
  `m1-done.play` (**94 assertions**), the showcase/video scripts, water LOD
  seams, bridge/tunnel reloads, powder, menu/map and replay scenarios.
  [Counts and outcomes](T1-planet-evidence/play-results.md),
  [commands and all attempts](T1-planet-evidence/play-results.json).
  The initial film/showcase failures and the old-flow fixture flags are
  explained below; their corrected runs retain the original checks.
- The real headless preview swaps through all five sizes, checks fly altitude
  and the sky uniform, writes size 832 with `vertical_scale: null`, restores
  the original engine and edit, and compares the home save files byte for
  byte. The current fixture has passed twice consecutively:
  [capture run](T1-planet-evidence/preview.log),
  [repeat](T1-planet-evidence/preview-confirm.log).
- The generator checks **18,859 columns per size**, at ten lake shores and
  33 river sites. Water cells checked: 29,409 / 30,293 / 30,199 / 32,410 /
  32,087 respectively. **No water walls, perched water, or missing generated
  cells** in those samples. This is not an exhaustive sweep of every shore
  at every size.
- Invalid size 129, size 1056, and size 832 with H=4096 are refused before
  opening a renderer/save, naming the tiling or position-key constraint.
  [Refusals](T1-planet-evidence/refusals.json).
- A save and replay made with the frozen generator-4 build are both refused
  by generator 5 (exit 2); the old save's file hashes are unchanged.
  [Messages and commands](T1-planet-evidence/compat-results.json).

## Bench — all numbers provisional

All five runs use 1920×1080, four Rayon workers, LOD factor 5, AO on,
skirt 2, the same seven static poses (120 measured frames each), and the
same 60-second flight. **35/35 static poses settled; no allocation failures
were reported.** Other missions were active throughout. These numbers do
not establish the idle-machine 60 fps target; the stalls are visible in the
1% lows and worst frames.

| Half-heights | Flight avg fps | 1% low fps | Worst frame ms |
|---|---|---|---|
| 128 | 81.3 | 13.8 | 97.26 |
| 256 | 73.9 | 11.3 | 174.61 |
| 512 | 94.4 | 13.6 | 150.23 |
| 832 | 84.3 | 18.5 | 90.39 |
| 1024 | 79.5 | 9.4 | 174.37 |

[Complete numeric bench reports](T1-planet-evidence/benchmarks.md) include
static GPU/CPU times, triangle counts, resident mesh memory and worker costs.
The reported quad allocation peaks during the static sequence range from
20.6 MB at 128 to 42.4 MB at 1024, within the unchanged 512 MiB quad pool.

## What changed

- The shipped world has height 2048, sea level 1024 and LOD 10. Its file says
  `planet_size_half_heights: 128`, rather than a raw column count. The loader
  explains the multiple-of-32 tiling rule and the 20-bit horizontal position
  limit. The size cap is 1024; taller worlds can have a smaller cap. Legacy
  raw-column files and `--n` still work for tests. `--planet-size` uses the new
  units. `--height` keeps the half-height count and centres sea level unless
  a raw override is supplied.
- F1 Terrain has the size slider, Original and Earth analogue buttons, radius
  and vertical scale, and an explicit Follow planet radius setting. The
  existing preview mechanism keeps the home engine, edits, simulation and
  save aside. Resizing a flying preview keeps sea-level altitude, clearing
  higher ground if necessary, and updates the renderer's planet radius.
  Save look writes size and terrain to `world.json`. Automatic vertical scale
  stays `null` through saving and restarting.
- `look.json` records the radius at which its metre dials were authored.
  Atmosphere height follows R; fog density follows 1/R. The panel still shows
  actual metres and fog per km. This applies both at startup and in a preview.
- Long river courses no longer exceed the 128-bit search mask. Search groups
  become larger when needed, preserving every course piece. The hydrology
  lookup grid also follows R; exact distance tests and metre-sized channels
  are unchanged. The global lake finder reaches proportionally farther,
  without probing millions of empty spatial-index cells.
- Ground-relative camera/teleport hints stop at the real bedrock floor and
  clipped ceiling. At size 1024 the sampled trench is −1259.55 m, but its
  actual floor is −1023 m. At 832 it is −1022.97 m and just fits.
- LOD 10 exposed insufficient curvature sampling. The grid now has 9×9
  points, with two quartic spans along each axis. Each coarse vertex still
  reads 25 samples. The outline and powder normals use the same grid spacing.
  The sampled worst L10 position error fell from 23.1353 m to 2.1379 m.
- The coarsest sea remains opaque so the sky cannot show through the planet.
  Its shading uses the same water opacity, viewing angle, sun glint and
  gravel floor as finer water, rather than a fixed premixed colour.
  Sea top faces at LOD 5 and coarser use that backing too: at grazing angles
  the coarsened physical floor's horizon ends before the water's horizon.
  Without the backing, strips of sky still showed through the sea around
  the small planet's rim. LODs 0–4 retain the real transparent water.
- Generator version is 5. Simulation rules and movement rules are unchanged.
  Old saves/replays are refused with instructions to use their original
  build or start a new save directory.

## Coordinates and scripts

`game/scripts/shift_layers.py` applies +512 to actual `atblock` and numeric
`target FACE I J K` commands,
absolute `block[...]`, `level[...]`, `cover[...]` assertions, and absolute
layer fields (`k`, `fk`, `crosshair_k`, `find_k`, `find_target_k`). It leaves
relative offsets, metre altitudes, poses, water levels, tolerances, comments
and typed prose alone. It accepts signed boundary coordinates and exponent
notation. Four self-checks pass. It is intentionally not idempotent.

It changed 25 of the original 39 scripts. There were no `.play` files under
`game/src`. Tests constructing their own planets with sea level 512 retain
those settings. The `atblock ... 2000` ground-finding hints became 2512;
walking ground placement clamps that hint to the world's top. The tunnel
regression's chunk boundary moved from layer 512/cz 16 to layer 1024/cz 32.
`ocean-orbit.play` now includes 300 km and 1000 km views, because the old
150 km view mixes L9 and L10 instead of showing only the coarsest ocean.
The final [migration audit](T1-planet-evidence/migration-audit.log) checks
all **306** absolute references against the starting commit.

At merge, run the helper once on **only the other missions' new scripts**.
Review world-floor/ceiling assumptions separately; a layer substitution
cannot decide which height is meant as a boundary or a ground-search hint.

The showcase's numeric video target also needed shifting; leaving it at 556
sent the pullback underground. `target rel`, `target ll`, `key` and `keyrel`
retain their relative/metre arguments. The helper's self-check covers them.
The tall-world showcase keeps its original radius explicitly with `--n
131072`, while centring sea at 2048 in H=4096.

The film script needed more than a layer substitution. Its original final
checkpoint passed on the frozen build and on this build with sea 512, with
identical counters, but had 12 queued updates after shifting the layers.
Five seconds later the queue was empty, then drying woke it again. Another
five minutes finished both the queue and all drying timers. The script now
waits ten minutes at the end and **also asserts `drying == 0`**, strengthening
its old momentary-quiet check. Simulation code/rules were not changed.
See the [before/control comparison](T1-planet-evidence/film-compare.json)
and [long-tail measurements](T1-planet-evidence/film-tail.log).

The old-save fixture pair now runs the current generator's make script
with `--no-slide`, then loads that same scratch save under normal flow
settings. A truly old generator's save is tested by refusal instead. The
first suite invocation lacked that make flag; the corrected pair passes.

## Measured failures before the fixes

- The three initial regressions all failed: sea level was 512; a file using
  half-heights was rejected for lacking `columns_per_face`; the shipped
  atmosphere stayed 639.7 m high even at eight times the radius. All three
  passed after the changes.
- At size 1024, river 15897 had 1277 pieces in 160 search groups. A 128-bit
  mask could not address all of them. The fixed test checks 984 exact points
  on long courses. The same index probe formerly visited 1,071,612 bins;
  after scaling the index it visits 2366. River/lake index bin counts changed
  from 9,616,541 / 2,178,319 to 67,549 / 10,575. These are counts, not timings.
- The L10 lattice regression failed at 23.1353 m error and passes at 2.1379 m.
- The opaque-ocean material regression failed because no water-shading flag
  reached the renderer. Frames exposed a ring where opaque ocean met water.
- A separate far-water regression then failed on L9 sea tops. Giving those
  distant sea surfaces the same opaque backing removed the remaining rim
  strips in the repeated 128 orbit frame.
- The size-1024 playthrough reproduced `find: no lake within 80 km`, despite
  passing walking, digging, water and the replay. The search reach is now
  scaled to the planet.

## Precision, capacity and scale audit

- Every slider step from 128 through 1024 tiles exactly. Extreme positions
  on all six faces round-trip through both the lattice and `PosKey` (including
  N−1 at the 2^20 limit). Non-power-of-two N at 832 is explicitly exercised.
- CPU positions and camera-relative subtraction stay f64. Fine GPU positions
  use small relative offsets, not a 725 km absolute f32 position. Column
  heights are f32 **metres above sea level**, not planet-centred radii.
  Reversed-Z and Depth32Float were already in place. The warp regression
  evaluates the uploaded f32 grid in f64 against the exact lattice; it is
  not a GPU vertex readback measurement.
- At H=2048 the maximum LOD is 10. The 16-entry LOD counters have room. Even
  the supported H=65536 limit gives LOD 15, fitting the last entry.
- At size 1024 there are 6144 coarsest chunks (32×32×6), versus 96 at 128.
  The quad pool remains 512 MiB. The denser curvature grid reserves 324 MiB
  at 131072 slots (formerly 100 MiB); slots are capped against the adapter's
  storage-buffer limit. This added GPU reservation needs checking alongside
  the bench's actual resident mesh memory.
- The active horizon occluder is already `planet.bottom_radius()`.
  `Generator::height_margin_m` is not the streamer's occluder. I did not
  increase that unused margin to try to compensate for the larger planet.
- Channel widths, channel depths, meander wavelength, coast shaping and
  strata spacing are already in metre/block units. Lake positions, support
  kernels, river networks and baked height scale are global.

## Handoff to T2/T3

T2's landform/local-relief changes and T3's boundary changes are not in this
worktree. I did not restructure `landform.rs`, `Hydro::shape` or `Hydro::new`.
The small changes inside `Hydro::new` are the two index pitches and search-run
grouping. Merge those alongside T2's constructor changes.

The mesh grid now has 81 offsets and 81 directions; quad high-word bit 24
is the far-sea shading flag. `sim/draw.rs` already follows the shared grid
constants. C2's new draw code must keep that layout. Coordinate generator
version increments at publication: another terrain change published after
this generator-5 build needs a fresh version, even if its branch also began
by choosing 5.

Two inherited lake details still deserve attention in that merge:

- `Hydro::lake_at` adds dimensionless `SHORE_NOISE` to a lake field whose
  gradient per metre shrinks with R. Its wavelength is in metres, but the
  resulting displacement can grow with R.
- The apron/shore bands in `Hydro::shape` use field thresholds (for example
  `zone - 0.07`), so their physical width can grow with the global lake field.

Those are local-shore questions for the changing shape/boundary code; the
index fixes do not alter them. Re-run the five-size checks after the terrain
missions are merged. T1 alone does not prove their new peaks, valleys or
shoreline detail.
Also check `column_from`'s `dh / vscale` lapse-rate conversion when T2 splits
global and local relief: fixed-metre relief should not inherit an inverse-R
temperature change. New peaks have 1024 m of headroom above sea in this
world, rather than the former 1536 m.

## Reproduction

From this worktree's `game/`, build with `cargo build --offline --release`.
No window or real input is needed:

```text
python scripts/shift_layers.py --self-test
cargo test --offline --release
cargo test --offline --release t1_preview -- --ignored --nocapture
python scripts/t1_planet_check.py --phase after
python scripts/t1_planet_check.py --phase after --exercise
python scripts/t1_planet_check.py --phase after --bench
python scripts/t1_play_suite.py
```

`out/t1/STOP`, if present, stops a driver before its next size/script. Remove
that scratch control file to run the complete suite. All timings are
**provisional**: other missions were compiling and running on the machine.
The lead should repeat the identical benches with the machine idle.

## Not verified / Andrew's choices

No normal game window, real mouse/keyboard input, minimising, display scaling
or cross-PC replay was tested. The headless preview check drives the same
rebuild and save methods; it does not replace Andrew trying the slider by
hand. New terrain from the other missions is not yet integrated.
The runtime size matrix uses seed 42, H=2048 and 1 m blocks. Existing tests
retain their own other heights/block sizes; this is not an exhaustive run
of every possible CLI combination or seed.

**Observed teardown failure:** one Vulkan preview-test process exited with
`0xc0000005` after printing that all save/restore assertions had succeeded.
[Failure log](T1-planet-evidence/preview-teardown-failure.log). Two subsequent
runs of the current fixture exited cleanly. I have not established the
cause or claimed to fix it. The lead/C1 should repeat preview, return-home
and quit in the real window. The evidence capture now advances egui time
long enough for the panel's opening fade to finish; that makes its text
readable and is not claimed as a fix for the teardown failure.

The default remains 128. Andrew can judge 832 as the proposed Earth analogue
and the atmosphere/fog proportions in the comparison frames. At 1024, deep
trenches reach bedrock; increasing radius cannot create more vertical layers.

## Proposed DESIGN.md wording

> Planet size is `planet_size_half_heights` = N/(H/2), with default 128.
> The terrain panel changes it in steps of 32, up to 1024; 832 is the Earth
> analogue, with Earth's relief reduced to fit the world's half-height.
> N need not be a power of two. It must tile coarsest chunks and fit the
> simulation's 20-bit horizontal coordinates, so taller worlds can have a
> lower size limit. Sea level is H/2 and the coarsest voxel is H/2.
>
> `vertical_scale: null` means R / Earth's radius and follows a size change.
> A numeric override remains available. Resizing terrain uses a preview
> with world saving off; Save look writes its parameters as startup defaults
> without replacing the open world's saved edits.
>
> Atmosphere height and fog distance follow radius. The look stores their
> metre values together with the radius at which they were authored, so
> custom looks retain their proportions when the planet changes size.

T2 supplies the authoritative local/global feature table and mountain policy.
