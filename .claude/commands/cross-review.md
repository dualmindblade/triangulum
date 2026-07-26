---
description: Package a merge for fresh-context cross-model review and dispatch the reviewer
argument-hint: "<branch|commit-range> [reviewer: sol|claude]"
allowed-tools: Read, Write, Edit, Glob, Grep, Bash, PowerShell, Agent
---

Package the work in `$ARGUMENTS` for **cross-model review at merge time** and
dispatch a reviewer.

Two rules are non-negotiable:
- **Fresh context.** The reviewer must not have participated in the work. No
  shared session, no inherited assumptions.
- **Different model family than the author.** Claude-authored work is reviewed
  by Sol; Sol-authored work by Claude. Decorrelated blind spots are the whole
  point — check the commit trailers to see who wrote it.

## 1. Assemble the packet

Write `docs/reviews/<date>-<slug>-REQUEST.md` containing **only** what a
reviewer legitimately needs — not this session's reasoning, which would just
propagate its blind spots:

- **The claim.** What the change asserts it accomplishes, in the author's own
  words (mission spec / commit message).
- **The diff.** `git diff main...<branch> --stat` plus the full diff, or a
  pointer to the branch if it is large.
- **The evidence.** Gate output, before/after numbers, frames, repro scripts —
  the packet must let the reviewer *re-run* the proof, so include exact
  commands.
- **Blast radius**: which files, which lane, which contracts are implicated
  (see `docs/PROCESS.md` §6 and the routing table in `CLAUDE.md`).
- **Declared human-verdict needs**: anything the author says needs Austin's or
  Andrew's eyes.

## 2. Set the rubric, in this order

1. **Audit the evidence first.** Does the repro actually prove the claim?
   Could it pass while the bug remains? This is where reviews find the most —
   do it before reading the implementation, so the implementation's logic does
   not talk you into accepting weak proof.
2. **Check against invariants** in the relevant contract doc
   (`WATER-CONTRACT.md`, `PERF.md`, `TRANSITIONS.md`, the CLAUDE.md files).
3. **What could this break elsewhere?**
4. **Code quality.**

Tell the reviewer explicitly: **you may run the harness to try to falsify the
claim, and finding the claim overstated is a successful review.** Also tell it
that "found clean" is a real and valuable verdict — a review that manufactures
findings to look thorough is worse than one that reports honest negative
space.

**Proportionality:** contract-file changes get the full rubric; a small
constant tweak or a tooling addition gets an evidence audit only. State which
level you are asking for and why.

## 3. Dispatch

**Claude reviewer** — spawn a fresh agent (Fable for deep/coupled-core work,
Opus otherwise) pointed at the packet, with the rubric and instructions to
write `docs/reviews/<date>-<slug>.md`.

**Sol reviewer** — write the packet as a MISSION doc, then from the repo root:

```powershell
Start-Process -FilePath cmd.exe -ArgumentList '/c codex exec -m gpt-5.6-sol -c model_reasoning_effort=max -s workspace-write -' -WorkingDirectory <repo> -RedirectStandardInput MISSION.md -RedirectStandardOutput run.log -RedirectStandardError run.err -NoNewWindow -PassThru
```

Use reasoning effort `max` (deliberately not the tier above, which lets the
model delegate — Austin does not want that). Sol streams reasoning to
`run.err` and its final answer to `run.log`, and writes deliverable files
directly.

**Codex runs are not harness-tracked**, so completions have been missed.
Immediately after launching, start a background watcher so you get notified:

```bash
until ! tasklist //FI "IMAGENAME eq codex.exe" | grep -q codex.exe; do sleep 60; done; echo MISSION EXITED
```

Freeze detection: codex buffers stdout, so a quiet log is a weak signal. The
real signal is CPU — flat `(Get-Process codex).CPU` plus no new `run.err` means
frozen; kill and relaunch.

## 4. On return

- Archive the review to `docs/reviews/` (git-tracked — `viewer/interchange/`
  is gitignored and evidence stored there has been lost before). Do this
  **before** removing any worktree.
- Re-verify findings against the code yourself before acting. Reviewers have
  been highly precise here, but re-verification is a standing project rule.
- Findings that reveal a *gate* weakness rather than a code defect go to
  `docs/PROCESS.md` §8 territory — file the instrument as a tooling mission.
- Only then merge, serially, to main.

$ARGUMENTS
