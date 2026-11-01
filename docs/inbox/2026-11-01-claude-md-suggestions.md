# Suggested CLAUDE.md additions (2026-11-01)

Proposals only. Austin commits CLAUDE.md changes himself.

1. **Bench a regression by stage before calling it a cost.** Keep a build from each
   merge and bench them all under the same conditions. In round 2 this turned "mountains
   are expensive" into one cause and a one-function fix.
2. **Scenes in `.play` scripts stand on `fill` trays**, not natural ground, unless the
   ground is what's being tested. T2's mountains broke 14 scripts this way, and T3's
   coast broke one site again.
3. **Water and sand scenes can turn on a tie-break.** `sim::choice` hashes position and
   tick, so one tick or a layer shift can flip a scene between two valid outcomes. An
   exact assert in such a scene passes by luck.
4. **A frozen build reads today's assets.** The build's asset paths are compiled in, so
   an old exe run against a newer `world.json` fails. Pass `--world`, `--blocks` and
   `--look-file` from a checkout of that commit.
5. **The full test suite needs memory.** It was stopped once with qBittorrent holding
   4.9 GB of 32 GB. Check free memory first.
