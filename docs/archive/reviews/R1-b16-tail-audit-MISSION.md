# MISSION R-1: audit the B-16 "residual tail is build cost" conclusion

You are reviewing a **conclusion**, not code. Triangulum is a planetary-scale
voxel game (Rust + wgpu). Your job is to decide whether a claim the team has
already acted on is actually supported by its own evidence — and if it is not,
to say what measurement would settle it.

This is an **analysis-only** mission in a git worktree.

- **Do NOT build.** Do NOT run `cargo build` or `cargo run`.
- **Do NOT run the game or the play harness.** This worktree deliberately has
  no game assets (`viewer/assets/*.bin` is gitignored, ~170 MB), so nothing
  will render anyway. Another mission is using the GPU; a build here would
  also fight it for CPU.
- **Write exactly one file**: `docs/reviews/R1-b16-tail-audit.md`. Change
  nothing else. Do not commit (a linked worktree cannot, here).

Reason from source, the bug ledger, and the recorded evidence.

## The claim under audit

`BUGS.md`, entry **B-16** ("Blurry mesh never settles in some areas (ice)").
Read the entire entry — all four HUNT rounds — before anything else.

HUNT 2 found admission starvation. HUNT 3 landed three scheduling fixes
(wanted-set eviction protection, coarsest-first `covered_missing` ordering, a
2-slot visible-gap reserve piercing both the live budget and the shared
productive cap). **HUNT 4 then concluded:**

> gap-list instrumentation shows ~405 L11 gaps all admitted-or-pending,
> draining at build throughput. The remaining tail is BUILD COST, not
> scheduling ... post-fix convergence is minutes (was: NEVER). The three
> scheduling defects are FIXED. Remaining speed-up belongs to the B-4c
> cold-build campaign.

That conclusion closed the bug and redirected the work to a different campaign.

## The new datum that prompted this audit

Austin (the project lead, 2026-07-26) reports the B-16 symptom **still occurs
on his other, more powerful machine**. It has *not* been retested there since
the latest fixes landed, so treat this as a soft signal, not proof.

But note the tension it creates. If the remaining tail is genuinely **build
throughput**, then faster hardware — more cores, faster GPU — should drain it
*faster*. A symptom that persists or worsens on stronger hardware is prima
facie evidence that something other than raw build cost is involved.

**Consider confounds seriously before concluding anything.** Off the top:
a more powerful machine may drive a larger window, higher resolution, or a
different draw distance, and therefore *want more tiles* — which would
preserve the symptom without contradicting the build-cost explanation at all.
Frame rate differences change how many scheduler ticks happen per wall second.
There may be others. Enumerate what you find.

## Fresh evidence you have that HUNT 4 did not

Mission V-1 built a convergence instrument two days after HUNT 4 closed:

- `viewer/scripts/convergence_reel.py` — read it, especially the two-phase
  ref/live mechanism and its comments.
- `docs/missions/V1-convergence-findings.md` — the full findings.
- `docs/missions/V1-evidence/run1-report.json` and `run2-report.json` —
  **the raw measured curves.** Read the `b16_ice_hold` curve directly.

That curve is the sharpest new fact available. At the B-16 hold pose, the
live-vs-settled residual is **essentially flat for 25 seconds** (3.420 →
3.367), then drops to 1.964 at the single final 30 s sample. Live draw counts
over the same window crawl 1141 → 1432 against 1543 settled.

Ask yourself what a 25-second plateau followed by a cliff actually implies
about the mechanism, and whether "draining at build throughput" predicts that
shape. A pure throughput-limited drain would suggest one shape; a plateau
suggests something else. V-1 noticed the pixel curve lags the draw-count curve
and suggested arriving tiles mostly land where the picture was already close,
with the visually-worst gaps filling last — evaluate that hypothesis too,
including whether it is consistent with HUNT 3's "coarsest-first + visible-gap
reserve" ordering actually doing what it was intended to do.

## Where the mechanism lives

`viewer/src/renderer.rs` (selection, admission, the productive cap, the
visible-gap reserve, eviction), `viewer/src/terrain.rs` (tile build),
`viewer/src/voxel.rs` (chunks). These files are large (130-210 KB); navigate,
do not read them end to end. Relevant env-var probes: `TRI_STREAM_DEBUG`,
`TRI_BUILD`, `TRI_GPU_TIMERS`, `TRI_LOD_RING_DEBUG`. `PERF.md` carries the
performance campaign, including the B-4c cold-build item HUNT 4 redirected to.

## Deliverable: `docs/reviews/R1-b16-tail-audit.md`

1. **Verdict on the claim.** Is "the remaining tail is build cost, not
   scheduling" supported by the evidence that existed at HUNT 4? Supported,
   partially supported, or unsupported — and separate *what was proven* from
   *what was inferred*.
2. **Reconcile the new datum.** Does persistence on more powerful hardware
   contradict the conclusion, or is it explained by a confound? If you cannot
   tell from available evidence, say so plainly — that is a real answer.
3. **The curve shape.** What does the 25-second plateau then cliff imply? Does
   it fit build-throughput draining, or does it point elsewhere?
4. **The decisive measurement.** This is the most valuable part of the
   deliverable. Austin can run something on the other machine tomorrow.
   **Specify exactly what to run and what result would distinguish the
   competing explanations** — ideally a single run with a clear fork: "if X,
   it is build cost; if Y, scheduling is still involved." Be concrete about
   commands, env vars and what to record. Prefer something that works without
   a rebuild if possible; say so if a rebuild is unavoidable.
5. **Anything else you find.** If auditing this surfaces unrelated defects or
   dubious reasoning in the surrounding code or ledger, report it.

## Standards

Evidence-first. Quote the specific code or data supporting each statement.
Distinguish what you verified from what you suspect, and mark confidence.
**"Found clean — the conclusion holds" is a fully valid and useful verdict**;
do not manufacture findings to look thorough. Equally, do not soften a real
refutation. This project's standing rule is that every review finding gets
re-verified by a human afterwards, so precision serves you better than volume.

If the mission premise itself is wrong — for instance if the ledger already
answers this and Austin's recollection is stale — say that directly.
