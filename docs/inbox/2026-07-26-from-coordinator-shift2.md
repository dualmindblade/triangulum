# Inbox: findings from the shift-2 coordinator (2026-07-26)

Four items, found while establishing the baseline and dispatching. The first
is the one I would act on.

---

## 1. Every gate's blessed baseline is gitignored and exists only on this machine

**This is a project risk, not a wart.**

`.gitignore:13` is `viewer/interchange/*`, with `!` exceptions for a handful
of tint/v7 PNGs. Everything the gates compare against lives under it:

| Baseline | Size | Tracked? |
|---|---|---|
| `viewer/interchange/reel/accepted/` — 29 blessed world-reel frames | 15 MB | **no** |
| `viewer/interchange/perfreel/accepted.json` — the perf contract | 1 KB | **no** |
| `viewer/interchange/convreel/accepted.json` | 7 KB | **no** |
| `viewer/interchange/skyreel/accepted/` | — | **no** |
| `viewer/interchange/convflight/` | — | **no** |

`git ls-files viewer/interchange/ | wc -l` returns **16**, and all sixteen are
one-off review PNGs.

Three consequences:

- **A fresh clone has no working gates.** It renders and compares against
  nothing.
- **Every dispatched mission hits this.** A worktree gets no baselines, so a
  mission that runs the world reel sees 29 poses with no reference. I copied
  all baselines into all four worktrees by hand today; that step is not in the
  dispatch checklist and would otherwise have silently degraded four missions
  into "everything looks new."
- **If this machine dies, the blessed reference set is gone** — and that set
  encodes months of accumulated human review decisions. It is 15 MB.

This is the same class of hole `docs/PROCESS.md` §7 already identified for
reviews (*"git-tracked, because `viewer/interchange/` is gitignored and
evidence stored there has been lost before"*). The reviews got moved. **The
baselines never did.**

**Proposal:** track the blessed baselines. Either move them to
`viewer/baselines/` or add `!` exceptions for the `accepted` sets. 15 MB of
rarely-changing PNGs is nothing to git, and it makes blessing an auditable,
reviewable commit — which is what a human decision ought to be. Right now a
re-bless is invisible to version control, so nobody can review one or undo it.

Your call — it is a repo-policy change, and it would make `--accept` produce a
diff, which is a behaviour change for every mission.

---

## 2. `viewer/CLAUDE.md` — the verify.sh path is wrong

The gate table says:

> | suites | `bash scripts/verify.sh` (repo root) | ... |

There is no `scripts/verify.sh` at repo root. It is at
**`viewer/scripts/verify.sh`**. The header above the table already says "Run
from `viewer/` unless noted", so the `(repo root)` annotation is the error.

Cost: one failed command for me; it would cost every fresh mission the same.

**Proposed edit:** drop `(repo root)` from that row.

---

## 3. `CLAUDE.md`'s "kill stale processes" line is dangerous with concurrent missions

Root `CLAUDE.md` says:

> Kill stale `triangulum-viewer` / `play` processes before a build, or the
> linker fails on a locked binary.

That is correct and useful advice for one mission on the machine. With four
missions running concurrently — which is what `docs/PROCESS.md` §11 says is
normal — **a mission that follows it blanket-kills another mission's
measurement run mid-flight.** The victim gets a truncated capture and no
indication why.

I worked around it today by telling all four missions, in their dispatch
prompts, to kill only processes they started. That override should live in the
file rather than in my prompt — PROCESS §9's own rule is *"if you are
overriding it in prompts, the override IS the new content."*

**Proposed edit** to root `CLAUDE.md`:

> Kill stale `triangulum-viewer` / `play` processes before a build, or the
> linker fails on a locked binary. **When other missions are running on the
> machine, kill only processes you started** — a blanket kill destroys another
> mission's measurement run.

---

## 4. The perf reel needs an idle GPU, and nothing says so

Establishing the baseline, I ran the world reel and then the perf reel back to
back. The perf reel reported a **37 % regression on `plains_walk`** (avg
24.65 → 33.82, p95 30.14 → 46.67) — a clean gate failure by its own
thresholds.

It was false. Standalone re-run: 26.02 / 31.25, exit 0, no regressions. The
elevation was concentrated in the scenarios that ran first and decayed through
the run:

| scenario (run order) | run 1 avg | run 2 avg | delta |
|---|---|---|---|
| forest_ground | 29.84 | 28.52 | +4.6 % |
| plains_walk | 33.82 | 26.02 | **+30.0 %** |
| mountain_mid | 29.15 | 27.42 | +6.3 % |
| ocean_storm | 16.19 | 15.84 | +2.2 % |
| orbit | 17.84 | 17.33 | +2.9 % |
| ascent_sweep | 30.11 | 29.56 | +1.9 % |

— residual load from the previous reel, decaying. And it cannot have been real:
`git diff --stat 120995b..HEAD -- viewer/src/ viewer/examples/` is **empty**.
No Rust has changed since the last shift, so no perf change was possible.

This is the same class as the blind spot `PERF.md` item 1 already documents
(*"the perf reel warms before measuring"*) — inverted. That post-mortem is held
up in PROCESS §8 as the model to imitate, so it seems worth closing the loop.

**Proposed addition** to `PERF.md`'s Instruments section, or the gate table:

> The perf reel needs a cold, idle machine. Do not chain it after another reel
> — residual load inflates the first two scenarios by up to 30 % and produces a
> false regression. Run it alone, first. If it flags, re-run it standalone
> before believing it.

I have already told all four missions this shift not to run the perf reel at
all, and left that gate to the one mission that needs it.

---

## Not filed, because it is status rather than invariant

The handoff says *"`PERF.md` §1 redirects remaining B-16 work to the B-4c
cold-build campaign. PERF.md has not been updated — do it."* `PERF.md` contains
no reference to B-16 at all (grepped the whole file). The premature redirect
lives in `BUGS.md`'s B-16 HUNT 4 entry and is already corrected in place by
HUNT 5 directly below it. There is nothing to edit in PERF.md, so I did not
invent an edit.
