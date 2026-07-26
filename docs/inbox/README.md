# The inbox

Proposals waiting on Austin. Nothing here is committed policy — that is the
whole point of the folder.

Two kinds of thing land here:

**1. CLAUDE.md proposals.** Agents may propose additions; only Austin commits
them. A proposal should state the line it wants added, which CLAUDE.md, and
what went wrong without it.

The bar for a CLAUDE.md line is: *will this still be true in three months if
nobody touches it?* Status, current bugs and in-flight campaigns fail that
test and belong in `BUGS.md` or a campaign doc instead.

**2. Agent feedback worth acting on.** Every mission ends with a `## Feedback`
section in its findings doc (`docs/missions/`). The coordinator reads those and
lifts anything actionable to a file here — tooling gaps, spec problems,
codebase smells noticed in passing, traps that cost an agent time.

Feedback is optional by default and **mandatory whenever a mission used or
built new tooling**, because that is when a tool's rough edges are freshest.

## Triage

Austin's options on any item: commit it, reject it, or turn it into a mission.
Rejected items are worth a one-line reason — a proposal that keeps coming back
usually means something real is unaddressed.

Delete items once resolved. This folder should stay short; if it is long, the
process is generating more suggestions than anyone is absorbing, and that is
itself a finding.
