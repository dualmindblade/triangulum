---
description: Write a shift-change note to docs/handoffs/ so the next session can pick up cold
argument-hint: "[optional: shift name or focus]"
allowed-tools: Read, Write, Edit, Glob, Grep, Bash, PowerShell
---

Write a **handoff note** — a shift-change document for whoever picks this up
next. Assume they are a fresh session with no memory of this one, reading it
cold.

The note is not a diary. Its single job is: **could a fresh session continue
from what is on disk?** Everything you write should serve that.

## Before writing

1. `git log --oneline` since the last handoff, and `git status` — know what
   actually landed versus what is still loose in the tree.
2. Read the most recent file in `docs/handoffs/` so you continue the thread
   rather than restarting it.
3. Check `BUGS.md` OPEN — did anything you did change the state of an entry?

## Write to `docs/handoffs/YYYY-MM-DD-<short-slug>.md`

Use today's date. Include these sections:

### Landed
What merged, with commit hashes. One line each.

### Verified vs attempted
**This is the most important section — separate these ruthlessly.**

- **Verified**: the claim, and the *specific evidence* that proves it — which
  gate, which run, which numbers. "Tests pass" is not evidence; "physics-
  regressions.play green, world reel 0 unexplained changes, forest pose
  2978→55 ms" is.
- **Attempted / unconfirmed**: work that is in the tree but whose claim is not
  actually proven. Say exactly what would prove it.

Never let an attempted thing read as a verified thing. The next session will
build on whatever you claim, and a false "verified" costs far more than an
honest "I could not confirm this."

### Suspicions
Things that felt wrong but you could not pin down. Hunches are valuable to the
next session precisely because they are cheap to record and expensive to
rediscover. Say how confident you are.

### Escaped-gate analysis
For anything that reached human eyes this shift: **which gate should have
caught it, and why didn't it?** If the answer is "no gate could have", name
the instrument that would and file it as a tooling mission. See
`docs/PROCESS.md` §8.

### Stale-comment / stale-doc warnings
Comments, docs or CLAUDE.md lines that your work made untrue. These rot
silently and mislead later sessions — flag every one you noticed, even if you
did not fix it.

### Next steps
Concrete, ordered, with enough context to start without archaeology. If
something is blocked, say what unblocks it.

### Suggested CLAUDE.md additions
Anything you learned that will still be true in three months and would have
saved you time. **Propose only** — write them into `docs/inbox/` as well;
Austin commits CLAUDE.md changes himself. If a correction came up twice this
shift, it belongs here.

### Open threads for Austin
Anything awaiting his verdict or Andrew's, stated as a specific question, not
a status dump.

## Finally

Keep it tight and readable — a long handoff nobody reads is worse than a short
one they do. Then tell the user the path you wrote and give a 3-5 line
spoken summary of the shift.

$ARGUMENTS
