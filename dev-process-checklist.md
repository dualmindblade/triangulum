# Planet Game — Process Upgrade Checklist

## 1. Memory & handoffs (do first)
- Stop relying on long-lived compacted sessions; short disposable sessions + durable artifacts.
- Ask CC to create a `/handoff` slash command → writes a shift-change note to `docs/handoffs/` (what was done, what was *verified* vs attempted, suspicions, next steps, stale-comment warnings).
- Fresh planning instance reads its way back in; no more marathon masters.

## 2. CLAUDE.md rules
- Invariants and pointers only, never status. Test: "still true in 3 months untouched?"
- Split by directory (root tiny; renderer/sim get their own).
- Prune on failure: agent does something dumb → fix/delete the line that caused it.
- If you're overriding it in prompts, the override IS the new content.
- Agents propose additions (inbox file); only you commit them.
- `/handoff` ends with "suggest CLAUDE.md additions."

## 3. Verification (the real bottleneck)
- **Golden images**: record camera paths (esp. mesh/voxel transitions, cube-face edges) → capture fixed frames → you bless them once → every build auto-diffs against them. Judgment problem → comparison problem. Small tolerance threshold + diff heatmap output.
- **False-color debug renders**: LOD levels, normals, chunk boundaries as flat colors. May revive roaming bug-hunter agents — subtle in beauty render, glaring in schematic.
- **No repro, no fix**: every fix ships with a harness sequence that failed before / passes after; repros accumulate into the regression suite.
- **Perf harness before more perf work**: recorded path + frame stats (mean, p95, worst) + Tracy. Makes perf tasks fully self-verifying.
- Agents deliver "evidence packets" (before/after, diffs, harness output) you review in batches.

## 4. Code review
- Review at merge time per task, not on a calendar.
- Fresh-context reviewer: sees only task spec + diff + evidence. Keep cross-model (decorrelated blind spots).
- Rubric order: (1) audit the evidence — does the repro actually prove the claim? (2) check against invariants, (3) what could this break elsewhere, (4) code quality. Reviewer may run the harness to try to falsify.
- Make a `/cross-review` command that packages all this.
- Your job: architecture pass every 1–2 weeks — fresh instance re-reads actual code vs intended design; findings → refactor tasks or invariants.

## 5. Scaling up agents
- Don't add agents until typical tasks close without you loading the game.
- Add one at a time; stop when *you* become the queue (idle agents, rubber-stamping).
- One coupled-core lane (Fable xhigh on mesh/voxel + scheduler); parallel agents get orthogonal work: sim/Python, tooling, golden-path library, tests, cosmetic fixes.
- One branch/worktree per agent, small frequent merges.
- Bounded task specs with explicit done-criteria before farming out.

## 6. Prioritization
- **Perf = a floor, not a stream**: e.g. never <25 fps on golden paths; perf work only when the gate trips, then it's the *only* work.
- **Render issues = triage**: systemic (seams, LOD popping — fix at root, high leverage) vs cosmetic (park on a list; 3 similar reports = promote to systemic).
- **Every cycle ships one visible thing** the two of you can walk around in.
- Order: keep it playable (perf floor) → keep it sound (top systemic fix) → keep it fun to return to (visible thing).
- Priorities map onto agent lanes: systemic = Fable, generation = Python agent (reviewable by you!), cosmetic = filler.
