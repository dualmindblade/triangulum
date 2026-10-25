# Handoff — 2026-07-26: the process shift, and B-16 reopened

First shift under the new process. Two goals: stand up the management layer,
and close the verification gap that was forcing human eyes. Both done. The
verification work then reopened a bug that had been closed.

**If you read one thing: B-16 is not resolved, and the reason it looked
resolved is a measurement artifact.** See §Verified.

## Landed (18 commits, `120995b..031f776`)

Process: `795a44e` CLAUDE.md ×3 + `docs/PROCESS.md` + `/handoff` + `/cross-review`;
`2dc1280` Austin's corrections; `cd30273` the dispatch checklist.
Tooling: `3346d5a` V-1 convergence reel; `4f727dd` its gate rebuilt;
`b1a6878` V-2 altitude ladder; `85148c7` V-3 recorded-flight lane.
Findings: `893eb10` B-19/B-20; `031f776` Sol's audit → B-21/B-22.

Tree hygiene: 14 merged branches and 3 stale worktrees removed; `docs/` is now
git-tracked and holds handoffs, reviews, missions, inbox and evidence.

## Verified (claim + the evidence, re-checked by the coordinator, not taken on report)

- **The convergence reel separates converging from pinned.** Healthy poses are
  **byte-identical** (md5) to their settled reference at 30 s and differ at
  0.5 s; `b16_ice_hold` differs at both. Confirmed by hashing frames directly,
  not by reading the report.
- **B-16 is NOT resolved.** Same pose, same stock `120995b` binary, same
  machine, same hour: reached by **teleport** it converges to 0.000 in 30 s;
  reached by **replaying Austin's own descent** it is pinned at 4.267 → 4.247
  across 45 s, 15.5 % of pixels wrong, draws frozen at 1165/1540 for 36 s.
  Five replays, verdict 5/5. The cold/warm pair *is* the finding.
- **The pipeline is capped, not saturated.** `stream_caps` default
  `productive: 3` plus HUNT 3's `+2` visible-gap reserve
  (`renderer.rs:2536`) is a **hard ceiling of 5** concurrent productive builds
  regardless of core count. Read in source. Explains why faster hardware does
  not help.
- **B-21 is real.** `evict()` (`renderer.rs:3792`) has **no `live_wanted`
  check**; the gentle shed (`3838`) does. Read both. B-16's wanted set ~1540 is
  above the 1500 threshold, so the unprotected sweep is live at the symptom
  site — and it logs nothing.
- **B-20 is real.** `lake_archi` is a featureless blue rectangle;
  `coast_beach` is dry grassland at ~950 m with no water for km. Probed both
  and looked at the blessed frames.
- **Gauntlet green** on merged main: `verify.sh` 8/8 PASS.

## Attempted / NOT verified

- **B-19 (the staircase band) has no root cause.** The measurement is solid;
  the diagnosis does not exist. Nobody has looked at why.
- **B-21's contribution to B-16 is unproven.** High confidence in the code
  defect, medium that it is active. Needs B-22's trace.
- **The rebuilt STUCK gate has not run end-to-end.** Its logic is validated
  against archived curves (b16 0.915 ok; truncated 0.999 STUCK; healthy 0.000),
  but a full reel re-run after the change has not happened. Do that first.
- **Nothing has been re-measured on Austin's faster machine.** Sol is explicit:
  that report does not currently contradict anything, because that machine has
  not run the latest fixes.

## Suspicions

- The other reels' pose tables are probably as rotten as the world reel's
  (B-20). Nothing verifies a gate points at what it claims to guard. Medium-high
  confidence; cheap to check with `probe`.
- HUNT 4's "heavyweight frozen-water L11 builds" may not be the cost centre at
  all. At 3.05 km every terrain tile builds the same 33×33 grid and the frozen
  branch changes colour/payload, not geometry. Low confidence either way —
  which is the point: it was never measured.

## Escaped-gate analysis

- **B-16 escaped because every gate captured settled frames.** Now covered by
  the cold lane (`convergence_reel.py`) and the warm lane
  (`convergence_flight.py`). Both are required: **the cold lane alone reports
  success on a pinned pose.**
- **B-16 was re-closed wrongly because no probe can distinguish waiting from
  working** — `tile_pending` is set before `rayon::spawn`. Filed as **B-22**;
  it is the prerequisite for closing B-16 honestly. This is the highest-value
  item in the ledger right now.
- **B-19 escaped because no gate swept altitude.** Now covered by
  `altitude_ladder.py`.
- **B-20 escaped because nothing checks that a gate is aimed correctly.** Not
  yet covered. The fix is cheap: `probe` returns water/lake/koppen in
  milliseconds, so a reel can assert its own premise at generation time.

## Stale docs / comments

- `BUGS.md` B-16 and commit `120995b` say "resolved … residual tail is build
  cost". **Both are wrong**; HUNT 5 and the Sol audit are appended in place.
- `PERF.md` §1 redirects remaining B-16 work to the B-4c cold-build campaign.
  That redirect was premature (B-22). **PERF.md has not been updated — do it.**
- `play.rs`'s header claims to be the authoritative command list but omits
  `pose`, `posehz`, `bench` and `glide` — the four commands this whole
  verification stack is built on. `bench N` renders without advancing
  `sim_ticks`, documented nowhere.

## Next steps, ordered

1. **Re-run `convergence_reel.py` end-to-end** to confirm the rebuilt gate
   behaves live. Cheap, and it unblocks trusting everything else.
2. **B-22 lifecycle instrumentation.** Lane A, observability only, must not
   change ordering or caps. Sol specified the exact events and insertion
   points. Everything about B-16 is blocked behind this.
3. **B-21 fix** — add the `live_wanted` check to the count sweep. Small, but
   gate it on B-22's trace so we can see whether it changed anything.
4. **B-20**: re-aim the two dead poses, then make reels assert their own
   premise. Audit the other pose tables.
5. **B-19** wants a diagnosis mission; it is Andrew-facing and independent of
   the B-16 thread, so it parallelises cleanly.
6. Harness observability bundle (`pick X Y`, `probe` prints koppen, a
   first-pose trace line, `run.log` timestamps) — three missions asked for
   pieces of this.

## Suggested CLAUDE.md additions

Two were approved and committed this shift (`24e477e`). Nothing new pending;
`docs/inbox/` holds the rest, including a recommendation to **defer** the
`reel_common.py` refactor.

## Open threads for Austin

1. **Run the convergence reel on the faster machine** — but note Sol's caveat:
   it must be a *current* checkout, and revision + executable hash recorded, or
   the result is uninterpretable. Sol's full protocol is in
   `docs/reviews/R1-b16-tail-audit.md` §Decisive measurement. It is honest that
   current probes *cannot* fully close the question without B-22.
2. **B-19 is Andrew's call** on how natural is natural enough. The specimen
   sheet (`docs/missions/V2-evidence/specimen_1km_staircase.png`) is built for
   his eyes.
3. Two spec errors of mine are recorded in `docs/inbox/` — I asserted things
   about the codebase confidently from reading rather than measuring, and one
   of those assertions turned out to be B-20.
