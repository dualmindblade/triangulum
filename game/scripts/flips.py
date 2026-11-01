"""Analyse a run.log flip trace: per cell, the ticks it went wet or dry."""
import re, sys
from collections import defaultdict

path = sys.argv[1]
window = int(sys.argv[2]) if len(sys.argv) > 2 else 20
pat = re.compile(r"trace tick (\d+): (\d+)/(\d+)/(\d+)/(\d+) (\d+) -> (\d+)")
cells = defaultdict(list)
per_tick = defaultdict(int)
for line in open(path, encoding="utf-8", errors="replace"):
    m = pat.search(line)
    if not m:
        continue
    tick, f, i, j, k, a, b = map(int, m.groups())
    cells[(f, i, j, k)].append((tick, a, b))
    per_tick[tick] += 1
print(f"{sum(len(v) for v in cells.values())} flips over {len(cells)} cells, ticks {min(per_tick) if per_tick else '-'}..{max(per_tick) if per_tick else '-'}")
# Cells with >= 3 flips within `window` ticks.
flicker = []
for c, fl in cells.items():
    ticks = [t for t, _, _ in fl]
    best = 0
    for n, t in enumerate(ticks):
        m = sum(1 for u in ticks[n:] if u - t < window)
        best = max(best, m)
    if best >= 3:
        flicker.append((best, c, fl))
flicker.sort(reverse=True)
print(f"{len(flicker)} cells flipped 3+ times within {window} ticks")
hist = defaultdict(int)
for best, c, fl in flicker:
    hist[best] += 1
print("most flips in a window:", dict(sorted(hist.items())))
# Gap between consecutive flips of flickering cells.
gaps = defaultdict(int)
for best, c, fl in flicker:
    for (t0, _, _), (t1, _, _) in zip(fl, fl[1:]):
        gaps[t1 - t0] += 1
print("gaps between flips (ticks: count):", dict(sorted(gaps.items())[:15]))
# Levels: what the cell went to when wetted.
wet_levels = defaultdict(int)
for best, c, fl in flicker:
    for t, a, b in fl:
        if b > 0:
            wet_levels[b] += 1
print("level a flickering cell is wetted to:", dict(sorted(wet_levels.items())[:12]))
for best, c, fl in flicker[:12]:
    print(c, best, " ".join(f"{t}:{a}>{b}" for t, a, b in fl[:16]))
# How many cells flipped >= N times at all.
counts = defaultdict(int)
for c, fl in cells.items():
    counts[min(len(fl), 9)] += 1
print("flips per cell (count: cells):", dict(sorted(counts.items())))
# Layers of flickering cells.
layers = defaultdict(int)
for best, c, fl in flicker:
    layers[c[3]] += 1
print("layers of flickering cells:", dict(sorted(layers.items())))
