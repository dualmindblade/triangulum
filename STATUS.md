# Status

*Updated 2026-10-24 by Claude (Opus).*

## Where things stand

- **Direction approved:** keep the repo, build a new engine core in `game/`
  next to the old viewer. Analysis and plan:
  https://claude.ai/artifact/7q7Lk9YM4DZxCX5WfcvxPo · grid comparison:
  https://claude.ai/artifact/7qz75zidRkhjMbVcKU3apY
- **M0 passed.** The prototype is far inside the frame budget on the 2060.
- **M1 started.** Four tasks running in parallel; see `PLAN.md`.
- The July process and docs are retired to `docs/archive/`. The old viewer
  still builds and runs from `viewer/`.

## Done

- **M1-grid** (Astra, verified by Opus, merged): `crates/cubesphere` with
  six projections. The recommended one, `EqualAreaCobe`, is smooth, exactly
  equal-area (1e-6) and the squarest: 14% of the surface more than 10° off
  square. Compare all four candidates on the grid page linked above.

- **M1-inventory** (Opus): `docs/feature-inventory.md` catalogues every
  feature of the old viewer, how it worked and where it lands. Andrew's
  specs and reference images, which were only on one disk, are now in git
  under `docs/source/`.

- **M1-bake** (Opus, verified by Opus, merged): `scripts/bake_world.py`
  removes the old "texel artifacts" at the source. Cell-structure noise is
  down from 18% to 0.6%, and it carries planetgen's geology (soil depth,
  continental cores, volcanic zones, erodibility) for real strata. Run it
  with `python scripts/bake_world.py` (about 20 s); check it with
  `python scripts/bake_world_check.py`.

- **M1a** (Fable, verified by Opus, merged): the new engine in `game/`, on
  the real equal-area grid. On an idle machine the fixed 60 s test flight
  runs at **633 fps average, 307 fps for the worst 1% of frames, worst
  single frame 4.8 ms** (the 60 fps target is 16.7 ms). Strata are real
  blocks; water is translucent. World height costs nothing: 2,048 and
  4,096 draw identical chunks.

- **M1b** (Fable, verified by Opus, merged): walking, digging, building and
  saving. The scripted M1 scenario (walk, dig a trench and a tunnel, build a
  bridge, save, reload, check the blocks, view the bridge from 1 km) passes
  83 of 83 checks. Six findings from Astra's review are fixed.

- **M1c-sky** (Astra, verified by Opus, merged): the July viewer's sky,
  sun, stars, fog and lighting, matched side by side against the old viewer
  at noon, golden hour, dusk and night. 24-minute days.

- **M1-terrain** (Opus, verified by Opus, merged): real landforms: ridges,
  gullied flanks, river valleys, mesas on hard dry rock. Oceans stay blue
  from orbit, and a cube-edge seam is fixed. Before/after sheets at every
  vertical scale: `docs/missions/M1-terrain-evidence/`. Saves from before
  this change are refused (the terrain under them changed).

- **M1c-stream** (Fable, verified by Opus, merged): streaming rebuilt
  around a coverage tree that never overlaps or leaves gaps. On an idle
  machine the 60 s test flight runs at **1,447 fps average, 602 fps for the
  worst 1% of frames, worst frame 5.7 ms**; at 1/8, which ran at 14 fps,
  it's now 964 / 381 fps. Also a sneak key (Ctrl while walking).
  (Correction: I first wrote that water surfaces now meet between detail
  levels. Astra's second review showed that fix only covers vertices; the
  edges between them can still be up to 23 m apart, far out. It's on the
  hardening list.)

- **M2a** (Fable, verified by Opus, merged): the simulation. A 20 Hz tick,
  separate from the sky clock, with a strictly ordered block-update queue.
  Sand and gravel fall when you dig out what holds them up, and land
  where they should, even across a save and reload mid-fall. The same
  inputs always give the same world (checked by a state hash). Saves are
  now crash-safe: written all-or-nothing, retried on failure, validated on
  load. Saves made before this are refused.

**M1 is feature-complete.** The last step is yours: play it and tell me what
feels wrong.

## Play it yourself

```
cd C:\code\triangulum\game
cargo run --release
```

You start walking on the hills. Click to capture the mouse (Esc releases).
WASD walk, Space jump (swim up in water), Shift sprint, G walk/fly, left
mouse breaks (hold to repeat), right mouse places, 1-9 pick the block, R
drops you to the ground, F hides water, H hides the HUD, V vsync, P scenic
places, Q saves and quits. Edits save automatically to `game/saves/Neisor/`.
Days are 24 minutes: `[` and `]` slow down and speed up time (up to
3,600×), and `--time 17:30` starts at golden hour.

Try other worlds (each keeps its own save):

```
cargo run --release -- --vscale 0.05                                    # 1/20
cargo run --release -- --vscale 0.125 --height 4096 --sea-level 1536    # 1/8
cargo run --release -- --block-size 0.5 --n 262144 --height 4096 --sea-level 1024 --vscale 0.05 --save saves/half   # half-metre blocks
```

Hold Ctrl while walking to sneak: you won't walk off edges, so you can
back out over a gap and place blocks under you. Known issues: ripple
patterns on distant steep terrain, and gully stripes that can look combed
on gentle hills (Andrew's call). `--lod-factor 10` shows finer distant
detail (still about 500 fps).

## In progress

- M1-hardening: the remaining review findings: picking and collision at
  cube edges, tunnel walls after reload, water seams far out (Fable)
- M2-hydro: lakes and rivers as real water, held by their banks (Opus)

## Waiting on you

- **Watch the M1 video:** `docs/media/m1-showcase.mp4` (85 s; stills in
  `docs/media/m1-showcase-stills/`). Things in it that are Andrew's call:
  - **Vertical scale.** At 1/70 the highest range is a white plain with
    contour stripes; at 1/8 it's real mountains (last shot of the video).
  - **The atmosphere is thin.** It's scaled to the 90 km planet, so the sky
    turns toward space above about 1.5 km, and 1/8 mountains reach into
    it. Whatever vertical scale he picks, the atmosphere's height should
    follow; it's one setting.
  - **Oceans from orbit** are now an even grey-blue; the deep blue before
    was a bug (space showing through). Bluer water is a colour choice.
  - **Dusk and night are weak,** and golden hour is subtle.
  - **Underground isn't dark yet,** and grass shows on every face of a
    grass block, so tunnel ceilings are green.
  - **Distant terrace stripes** ripple on steep slopes far away.
- **Andrew:** with 24-minute days, do the July calendar numbers still stand
  (7-day lunar month, 84-day year)? No rush.
- **Andrew:** pick a grid by eye if he wants something other than the
  recommended smooth equal-area one (grid page linked above).
- Block size (1 m or ½ m) gets decided after you've played M1.

## Decided

See `DESIGN.md`: the smooth equal-area grid (`EqualAreaCobe`), planet
131,072 columns per face (R ≈ 90.6 km), Andrew's 256-level water rule.

## Under review

- World height: 2,048 blocks suggested (512 felt small); the engine
  benchmark measures what height costs.
- Vertical scale: between 1/70 (Earth's slopes) and 1 (Earth's heights);
  Andrew decides from renders at 1/70, 1/20 and 1/8, coming with M1a.
