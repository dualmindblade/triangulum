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

## In progress

- M1a engine skeleton (Fable)
- M1-grid equal-area map (Astra)
- M1-bake smooth world bake (Opus)
- M1-inventory of old features (Opus)

## Waiting on you

- Nothing right now. Block size (1 m or ½ m) gets decided after you've
  played M1.

## Decided today

See `DESIGN.md`: smooth exact equal-area grid, world height 512 blocks,
planet 131,072 columns per face (R ≈ 90.6 km), vertical scale ≈ 1/70,
Andrew's 256-level water rule.
