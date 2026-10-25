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

## In progress

- M1a engine skeleton (Fable), plus vertical-scale renders for Andrew
- M1-bake smooth world bake (Opus)

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
