#!/usr/bin/env python3
"""The perf reel: a standing frame-cost table across representative
scenarios, tracked per commit like the world reel tracks pixels.

The gauntlet gates correctness but never speed, so perf wins evaporate
silently under weekly terrain rewrites. This makes cost a CONTRACT:
each scenario renders a warm-up then a steady bench; the table is
compared against the last accepted baseline and regressions flag.

Usage (from viewer/):
  python scripts/perf_reel.py             bench + compare vs accepted
  python scripts/perf_reel.py --accept    bless current as baseline
Exit codes: 0 clean, 3 = regressions to review.
"""

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

# name, play-script body (warm-up bench 15 then measured bench 30).
# Scenarios chosen for distinct cost profiles: build-heavy, draw-heavy,
# fragment-heavy, streaming-heavy, cross-body.
SCENARIOS = [
    ("forest_ground", """mode fly
weather time 349
teleport -0.906 -67.804 0.094
look -210 -17
bench 15
bench 30"""),
    ("plains_walk", """mode fly
weather time 349
teleport 19.674 35.265 0.010
look 175 -5
bench 15
bench 30"""),
    ("mountain_mid", """mode fly
weather time 349
teleport 47.861 14.399 0.592
look -49 -62
bench 15
bench 30"""),
    ("ocean_storm", """mode fly
weather pin 0.9 0.6
weather time 3500
teleport 20.633 127.615 8.0
look 0 -75
bench 15
bench 30"""),
    ("orbit", """mode fly
weather time 349
teleport 30.0 40.0 250.0
look 0 -80
bench 15
bench 30"""),
    ("moon_surface", """weather time 349
moonpose -12.0 40.0 0.002 30 -10
bench 15
bench 30"""),
    ("moon_orbit", """weather time 349
moonpose 0.0 0.0 1200.0 0 -80
bench 15
bench 30"""),
    ("ascent_sweep", """mode fly
weather time 349
teleport 64.120 -46.429 0.05
bench 10
teleport 64.120 -46.429 0.8
bench 10
teleport 64.120 -46.429 10.0
bench 15
bench 30"""),
]

# regression thresholds vs accepted baseline
AVG_TOL = 1.20   # +20% steady average
P95_TOL = 1.35   # +35% p95


def run_scenario(viewer: Path, name: str, body: str):
    script = viewer / "scripts" / f"_perf_{name}.play"
    script.write_text(body + "\n", encoding="utf-8")
    exe = viewer / "target" / "release" / "examples" / "play.exe"
    r = subprocess.run([str(exe), str(script)], cwd=viewer,
                       capture_output=True, text=True)
    if r.returncode != 0:
        return None
    benches = re.findall(
        r"bench \d+: avg ([0-9.]+) ms\s+p95 ([0-9.]+) ms\s+min ([0-9.]+)\s+max ([0-9.]+)",
        r.stdout,
    )
    if not benches:
        return None
    avg, p95, mn, mx = benches[-1]  # the last bench is the measured one
    return {
        "avg_ms": float(avg),
        "p95_ms": float(p95),
        "min_ms": float(mn),
        "max_ms": float(mx),
    }


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--accept", action="store_true")
    args = ap.parse_args()

    viewer = Path(__file__).resolve().parent.parent
    out_dir = viewer / "interchange" / "perfreel"
    out_dir.mkdir(parents=True, exist_ok=True)
    current_p = out_dir / "current.json"
    accepted_p = out_dir / "accepted.json"

    if args.accept:
        if current_p.exists():
            accepted_p.write_text(current_p.read_text())
            print("accepted: current perf table blessed as baseline")
        else:
            print("nothing to accept - run without --accept first")
        return

    table = {}
    for name, body in SCENARIOS:
        row = run_scenario(viewer, name, body)
        if row is None:
            print(f"  {name:14s}  FAILED")
            table[name] = None
            continue
        print(
            f"  {name:14s}  avg {row['avg_ms']:7.2f}  p95 {row['p95_ms']:7.2f}"
            f"  min {row['min_ms']:6.2f}  max {row['max_ms']:8.2f}"
        )
        table[name] = row
    current_p.write_text(json.dumps(table, indent=1))

    findings = []
    if accepted_p.exists():
        base = json.loads(accepted_p.read_text())
        for name, row in table.items():
            b = base.get(name)
            if not row or not b:
                continue
            if row["avg_ms"] > b["avg_ms"] * AVG_TOL:
                findings.append(
                    f"{name}: avg {b['avg_ms']:.2f} -> {row['avg_ms']:.2f} ms"
                )
            if row["p95_ms"] > b["p95_ms"] * P95_TOL:
                findings.append(
                    f"{name}: p95 {b['p95_ms']:.2f} -> {row['p95_ms']:.2f} ms"
                )
    if findings:
        print(f"PERF REEL: {len(findings)} regression(s):")
        for f in findings:
            print(" ", f)
        sys.exit(3)
    print("PERF REEL: no regressions vs accepted baseline"
          if accepted_p.exists() else "PERF REEL: no baseline yet (--accept to bless)")


if __name__ == "__main__":
    main()
