# How work moves on Triangulum

The bottleneck on this project is **verification**, not implementation. Agents
can write more code than anyone can confirm is correct. Every rule here exists
to convert a judgment call ("does this look right?") into a comparison ("does
this match the reference?") so that work can close without a human loading the
game.

Roles: **Andrew** is art director — final say on how anything looks.
**Austin** sets priorities and is the human verification gate of last resort.
A **coordinator** session plans, dispatches, reviews and merges. Everything
else is agents.

---

## 1. The shift

The unit of work is a **shift**, not a calendar interval. A shift has four
phases and produces one handoff.

### Open
A fresh coordinator session:
1. Reads the latest `docs/handoffs/` note, `BUGS.md` OPEN, and `CLAUDE.md`.
2. **Establishes a green baseline it did not inherit on faith** — runs
   `verify.sh` and the reels *before* dispatching anything. Never build on an
   assumed-clean tree; if something is already red, that is the shift's first
   finding.
3. Picks the lane assignments and gets Austin's sign-off (gate 1).

### Dispatch
2-4 missions, one worktree each, each with a bounded spec (§5).

### Land
Each mission returns → cross-review (§7) → the coordinator merges **serially**
to main. Agents never merge to main themselves.

### Close
`/handoff` writes the shift note. `BUGS.md` and any campaign doc get updated.
Feedback and CLAUDE.md proposals land in `docs/inbox/` for Austin.

---

## 2. Austin appears at exactly two gates

**Gate 1 — Dispatch (~5 minutes).** He approves the lane list. He is the
priority authority; the coordinator is not.

**Gate 2 — Verdict (batched, 20-40 minutes).** He reviews the evidence packets
together and performs the irreducible human verification — the things no tool
can yet judge. One session at the end of a shift, not ten interruptions
through it.

**The discipline that makes this hold: anything needing human eyes must be
declared in the mission spec up front.** If a mission did not predict it needs
a human verdict, it does not get one — it gets an instrument instead. This is
what prevents the failure mode where work lands green, Austin flies around,
and the whole thing re-opens.

Between gates he is not needed. If something is going wrong he can step in at
any time; the process assumes he *won't have to*, not that he can't.

---

## 3. Lanes are about file collision, not models

The big viewer files (`planet.rs`, `renderer.rs`, `terrain.rs`, `shader.wgsl`,
`main.rs` — 120-210 KB each) are the merge-collision risk. Lanes keep
concurrent missions off each other's files.

| Lane | Territory | Concurrency |
|---|---|---|
| **A — coupled core** | terrain / renderer / voxel / scheduler | **Serial.** One mission at a time. |
| **B — generation** | `planetgen/`, the bake scripts | Parallel to all others |
| **C — shader** | `shader.wgsl` fragment work | Parallel; short missions |
| **D — tooling** | harness, reels, probes, scripts | Parallel; always safe |

Heightfield work shares `planet.rs` with Lane A — run it in an A slot, not
beside one.

Any model can take any lane. Pick per mission by difficulty, and **rotate
deliberately** — model diversity is a feature, including on review. Rough
guide, not a rule: hardest coupled-core work suits Fable; Sol is
evidence-first and excellent at deep review and formal reasoning; Opus is
strong across the board. Generation work (Lane B) is the one Austin can
personally review, which is its own kind of verification.

---

## 4. Priorities

1. **Keep it playable.** Performance is a *floor*, not a stream: work on it
   when the gate trips, and then it is the only work.
2. **Keep it sound.** Systemic render issues (seams, popping, LOD
   inconsistency) get fixed at the root. Cosmetic ones go on a list — **three
   similar reports promote a cosmetic issue to systemic.**
3. **Keep it fun to return to.** Every shift ships one visible thing the two
   of them can walk around in.

Art-direction calls belong to Andrew and live in `DECISIONS.md`. Engineering
priority calls live in `BUGS.md`.

---

## 5. What a mission spec contains

Bounded, with explicit done-criteria, written *before* dispatch:

- **Why** — the actual problem, with evidence it is real.
- **What already exists** — the tools and prior art the agent must not
  reinvent. This repo has a lot; an agent that does not know about the census
  or the reels will rebuild them.
- **Done criteria** — checkable, not aspirational.
- **Which gates to run** (§6) and what to do if one flags.
- **Declared human-verdict needs** — anything Austin or Andrew must eyeball.
- **The evidence packet** it must deliver.
- **Feedback requirement** (§9).
- **Explicit permission to reject the mission's own premise.** Missions here
  have been wrong before, and the best outcomes came from agents that measured
  the premise and refuted it. Say so in the spec.

---

## 6. Gates, proportional to blast radius

Full gate list lives in `viewer/CLAUDE.md`. Which ones you run scales with
what you touched:

| You touched | Run |
|---|---|
| water / terrain sampling / generation | **Everything**, plus `WATER-CONTRACT.md` consultation at design time |
| scheduler / streaming / perf | suites, perf reel, world reel, convergence |
| shader only | suites, world reel, sky reel |
| tooling only | suites; whatever your tool is supposed to detect |

Determinism is a standing requirement: a double run must be byte-identical
wherever the system claims to be deterministic.

---

## 7. Review at merge

Every merge gets a review from a **fresh session in a different model family
than the author**. Claude-authored work is reviewed by Sol; Sol-authored work
by Claude. The reviewer sees the task spec, the diff, and the evidence — and
may run the harness to try to falsify the claim.

**Rubric, in this order:**
1. **Audit the evidence.** Does the repro actually prove the claim? This is
   first because it is where reviews find the most.
2. Check against the invariants in the relevant contract doc.
3. What could this break elsewhere?
4. Code quality.

**Proportionality.** Review effort scales with blast radius. A change to a
contract file (`sample()`, the scheduler, water rules) gets the full rubric. A
three-line constant tweak or a tooling addition gets an evidence audit only.
Reviewing everything at full depth just moves the bottleneck into review,
which is the thing this process is trying to avoid.

Reviews are archived in `docs/reviews/` — **git-tracked**, because
`viewer/interchange/` is gitignored and evidence stored there has been lost
before.

**Architecture pass every 1-2 weeks:** a fresh session reads the actual code
against the intended design and reports drift. Findings become refactor tasks
or new invariants. Scope the first one to *describing* real structure and
naming the two or three seams worth extracting — not a large refactor.

---

## 8. Escaped-gate analysis

**When a defect reaches human eyes, fixing it is only half the job.** The other
half is one question:

> Which gate should have caught this, and why didn't it?

This field is **mandatory** in every handoff and every review that closes a
field-reported bug. The answer is worth more than the fix, because it is the
only mechanism that grows the verification net. Without it the project
accumulates one-off repros forever and the net never closes.

The model to imitate is the B-4a post-mortem in `PERF.md` §1: it did not just
record that a cache regressed, it recorded *why every gate was blind to it* —
the probes ran barren poses and the perf reel warmed before measuring. That
paragraph is more valuable than the revert.

Acceptable answers include "no gate could have caught this yet" — followed by
what instrument would, filed as a tooling mission.

---

## 9. The feedback queue

**Every mission ends with a `## Feedback` section** in its findings doc.
Optional by default; **mandatory whenever the mission used or built new
tooling** — that is when a tool's rough edges are freshest and most honest.

It asks for, candidly and specifically:
- Anything noticed about the codebase, tooling or docs — **even if unrelated
  to the task**. Bugs, smells, traps, confusion.
- Complaints about the mission spec: ambiguous, wrong, or effort-wasting.
- What tooling would have made the job faster.
- Anything surprising.
- Every assumption made because the spec did not say.

Findings docs go to `docs/missions/`; the coordinator lifts anything
actionable into `docs/inbox/` for Austin. Vague praise is not feedback and
should be discouraged in the spec.

### The CLAUDE.md inbox

Agents **propose** CLAUDE.md changes into `docs/inbox/`; **only Austin
commits them.** Three rules keep those files healthy:

- **Invariants and pointers only, never status.** Test: *will this still be
  true in three months if nobody touches it?* If not, it belongs in `BUGS.md`
  or a campaign doc.
- **Prune on failure.** When an agent does something dumb because of a line in
  CLAUDE.md, fix or delete that line. The file is a liability as well as an
  asset.
- **If you are overriding it in prompts, the override IS the new content.**
  Repeating a correction twice means it belongs in the file.

---

## 10. Rhythm

Andrew is present on weekends and gates anything about how the game looks.
So:

- **Weekdays** — systemic, performance, and infrastructure work: everything
  that tools can verify without waiting on a person.
- **Weekends** — art-directed passes, where his verdict is the gate.

Contact sheets, trailers and feature clips should land **Friday evening**, so
they are waiting for him rather than blocking on him. Every major feature drop
ships a short clip rendered through the harness.

---

## 11. Scaling up

Currently 2-4 concurrent missions. The rule for adding more:

- **Do not add agents until typical tasks close without Austin loading the
  game.** Agent count is not the constraint; verification is.
- Add one at a time. **Stop when Austin becomes the queue** — idle agents
  waiting on him, or him rubber-stamping evidence he has not really read.

The honest signal that this process is working is not throughput. It is that
bugs stop being discovered by flying around.
