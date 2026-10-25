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

## Fly it yourself

Free camera only for now (walking and digging come with M1b):

```
cd C:\code\triangulum\game
cargo run --release                          # vertical scale 1/70 (Earth slopes)
cargo run --release -- --vscale 0.05         # 1/20
cargo run --release -- --vscale 0.125 --height 4096 --sea-level 1536   # 1/8
cargo run --release -- --block-size 0.5 --n 262144 --height 4096 --sea-level 1024 --vscale 0.05   # half-metre blocks at 1/20
```

Click to capture the mouse (Esc releases). WASD to move, Space/Ctrl up and
down, scroll for speed, Shift to boost, P cycles scenic places, F hides
water, H hides the HUD, Q quits. Known look issues that aren't decisions:
flat placeholder lighting (the real sky comes in M1c), missing mid-size
landforms (being fixed now), and ripple patterns on distant steep terrain.

## In progress

- M1b: walking, digging, building, saves, scripted harness (Fable)
- M1-terrain: ridges, valleys and escarpments (Opus)
- M1-review: independent review of the engine (Astra)

## Waiting on you

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
