#!/usr/bin/env python3
"""The convergence reel: "does the live view converge to the settled view,
and how fast?" as a standing measured gate — the time-domain sibling of
world_reel (pixels) and perf_reel (frame cost).

Every other visual gate captures SETTLED frames (shot blocks until
streaming drains), which is blind to the whole time-domain bug class:
B-16 (live blur never sharpens), B-4a v1 (shipped, reverted same day),
B-12/B-11/B-18. This reel measures the journey, not the destination.

SELF-BLESSING: the reference for each pose is that pose's own settled
capture — derived, never hand-blessed, so the gate works on day one and
cannot go stale. Each pose runs the SAME generated script twice:

  ref phase   plain harness: pose lines advance the sim clock without
              drawing; every `shot` settles (the existing byte-stable
              capture path) = ground truth at that exact sim time.
  live phase  TRI_POSE_LIVE=1 TRI_RAW_CAPTURE=1: pose lines draw their
              full 60 Hz tick cadence of LIVE frames from a COLD
              streaming state (the scheduler runs exactly as in
              gameplay); every `shot` captures the raw in-flight frame
              (no settle, no drain).

Both phases advance sim_ticks identically (pose lines = pose_ticks each,
shot = 0), so delta(t) = mean |live(t) - settled(t)| compares the SAME
sim instant: water animation, sun motion and weather cancel exactly, and
the residual is pure streaming-convergence state.

Metrics per pose:
  t_converge_s    first sample time from which every delta <= CONVERGE_EPS
                  (and stays there through the end of the window)
  residual_final  mean |delta| at the last sample — the late checkpoint;
                  a pose that never converges keeps this large forever
  progress        residual_final / residual_first — a STUCK pose (the
                  pre-fix B-16 selection freeze) stays near 1.0

Findings (exit 3):
  STUCK           large final residual AND no meaningful progress across
                  the window. Needs NO baseline: catches the B-16 class
                  on day one.
  vs accepted     once blessed (--accept), t_converge / residual_final
                  regressions beyond noise-derived tolerances flag.

Honesty note (measured, see docs/missions/V1-convergence-findings.md):
sim time is the x-axis but tile builds run on rayon workers in REAL
time, so wall-clock leaks into the curves and the numbers are machine-
dependent. On the dev GPU the live phase runs at ~0.5-0.75x real time
(heavy draws, no vsync), so builds get 1.3-2x more wall time per sim
second than a 60 Hz player — measured convergence is mildly OPTIMISTIC
vs gameplay (per-pose sim/wall is in the report; the fast/stuck
separation is orders of magnitude and unaffected). Thresholds carry
margins derived from repeated runs.

Usage (from viewer/):
  python scripts/convergence_reel.py               render + measure + compare
  python scripts/convergence_reel.py --accept      bless current numbers
  python scripts/convergence_reel.py --skip-render re-measure existing frames
Exit codes: 0 clean, 3 findings to review (never blocks a human; missions
treat 3 as "explain every flagged pose").
"""

import argparse
import json
import os
import re
import subprocess
import sys
import time
from pathlib import Path

import numpy as np
from PIL import Image

# name, lat, lon, alt_km, yaw, pitch.
# The table needs at least one known-slow and one known-fast pose: if the
# instrument reports the same number for both, the instrument is broken.
POSES = [
    # the validated B-16 slow specimen: the sustained-blur hold pose (ice
    # region, ~1540 wanted tiles incl. ~400 heavyweight frozen-water L11
    # builds). Post-fix this DRAINS but has a long build-cost tail.
    ("b16_ice_hold",  41.519, 122.511, 3.05,   93, -68),
    # fast cases from the standing perf/world reel tables
    ("forest_ground", -0.906, -67.804, 0.094, -210, -17),
    ("mountain_mid",  47.861,  14.399, 0.592,  -49, -62),
    ("plains_walk",   19.674,  35.265, 0.010,  175,  -5),
]

# Sample schedule in sim seconds: dense early (fast poses converge in
# ~1-3 s), sparse late; the last entry is the late checkpoint.
SAMPLES_S = [0.5, 1.0, 1.5, 2.0, 3.0, 4.0, 5.0, 6.0, 8.0, 10.0,
             12.0, 15.0, 20.0, 25.0, 30.0]
POSE_HZ = 10          # recorded-session cadence; 1 pose line = 6 ticks = 0.1 s

# Converged means "within the settled reel's own change threshold":
# world_reel counts a pose as changed at mean delta 1.0, so a live frame
# within 1.0 of settled is visually the settled frame.
CONVERGE_EPS = 1.0

# STUCK = the day-one B-16 detector: still far from settled at the end of
# the window (mean residual OR changed-pixel fraction — low-contrast ice
# blur keeps the mean small while 8% of pixels stay wrong) AND barely
# better than the first sample. Post-fix b16_ice_hold measures progress
# 0.57-0.64 (draining); the pre-fix selection freeze pinned the drawn set
# (flat curve, progress ~1.0, residual ~3.4, frac ~0.079).
STUCK_RESIDUAL = 3.0
STUCK_FRAC = 0.04
STUCK_PROGRESS = 0.80

# Baseline drift tolerances. Derived from the two-run noise measurement in
# the V-1 findings doc (live arrivals are wall-clock, so per-pose numbers
# wobble; t_converge lands on the sample grid, residuals on slow poses
# swing with build timing). Generous by design: this gate must only fire
# on real regressions, never train people to ignore red.
TCONV_MULT, TCONV_ABS = 2.0, 1.0      # flag if cur > acc*2.0 + 1.0 s
RES_MULT, RES_ABS = 1.8, 2.0          # flag if cur > acc*1.8 + 2.0

# Environment pins: weather time 555.111 is the B-16 hunt clock (the
# season that makes the ice region carry its heavyweight frozen-water
# geometry — the slow specimen's load depends on it). Sun is pinned
# per-pose to local noon like world_reel so far-longitude poses do not
# render at night; light cloud pin keeps the ground visible. All of it is
# identical across both phases, so none of it touches the delta.
WEATHER_TIME_S = 555.111


def sample_name(t: float) -> str:
    return f"s{int(round(t * 10)):04d}"


def build_play(path: Path, lat, lon, alt, yaw, pitch):
    sun_lat = max(-23.0, min(23.0, lat))
    lines = [
        "# generated by convergence_reel.py — identical timeline for both",
        "# phases: pose lines advance 6 ticks each, shot advances none.",
        "mode fly",
        "weather pin 0.10 0.0",
        f"weather time {WEATHER_TIME_S}",
        f"sun {sun_lat:.2f} {lon - 15.0:.2f}",
        f"posehz {POSE_HZ}",
    ]
    pose = f"pose {lat} {lon} {alt} {yaw} {pitch}"
    t = 0.0
    for s in SAMPLES_S:
        n = int(round((s - t) * POSE_HZ))
        lines += [pose] * n
        lines.append(f"shot {sample_name(s)}")
        t = s
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def run_phase(viewer: Path, script: Path, out_dir: Path, live: bool):
    """Run one phase; returns (ok, wall_s_total, {sample_name: wall_s}).

    Wall timestamps come from timestamping the harness's own `shot` trace
    lines as they stream (Rust stdout is line-buffered), measured from the
    first pose/teleport line so planet load + adapter init are excluded.
    """
    exe = viewer / "target" / "release" / "examples" / "play.exe"
    env = os.environ.copy()
    for k in ("TRI_POSE_LIVE", "TRI_RAW_CAPTURE"):
        env.pop(k, None)
    if live:
        env["TRI_POSE_LIVE"] = "1"
        env["TRI_RAW_CAPTURE"] = "1"
    t0 = time.monotonic()
    t_script = None
    shot_wall = {}
    proc = subprocess.Popen(
        [str(exe), str(script), "--out", str(out_dir)],
        cwd=viewer, env=env, text=True, encoding="utf-8", errors="replace",
        stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
    )
    tail = []
    for line in proc.stdout:
        tail.append(line)
        if len(tail) > 40:
            tail.pop(0)
        now = time.monotonic()
        if t_script is None and re.match(r"\[\d+\]", line):
            t_script = now  # first traced script line = planet+GPU init done
        m = re.search(r"\] shot (s\d{4}) ", line)
        if m:
            shot_wall[m.group(1)] = now - (t_script or t0)
    proc.wait()
    if proc.returncode != 0:
        sys.stderr.write("".join(tail))
    return proc.returncode == 0, time.monotonic() - t0, shot_wall


def shot_draws(log: Path) -> dict:
    """Per-sample draw counts from a phase transcript — the geometry-level
    view of the same convergence (the quantity the B-16 hunts tracked)."""
    if not log.exists():
        return {}
    return {m.group(1): int(m.group(2))
            for m in re.finditer(r"shot (s\d{4}) \((\d+) draws\)",
                                 log.read_text(encoding="utf-8",
                                               errors="replace"))}


def measure(run_root: Path, name: str) -> dict | None:
    """Delta curve for one pose from frames already on disk."""
    ref_dir = run_root / name / "ref"
    live_dir = run_root / name / "live"
    curve = []
    for s in SAMPLES_S:
        sn = sample_name(s)
        rp, lp = ref_dir / f"{sn}.png", live_dir / f"{sn}.png"
        if not (rp.exists() and lp.exists()):
            return None
        ref = np.asarray(Image.open(rp).convert("RGB"), dtype=np.float64)
        liv = np.asarray(Image.open(lp).convert("RGB"), dtype=np.float64)
        if ref.shape != liv.shape:
            return None
        d = np.abs(ref - liv).max(axis=2)
        curve.append((s, float(d.mean()), float((d > 6).mean())))
    deltas = [c[1] for c in curve]
    t_conv = None
    for i in range(len(deltas)):
        if all(d <= CONVERGE_EPS for d in deltas[i:]):
            t_conv = SAMPLES_S[i]
            break
    first, final = deltas[0], deltas[-1]
    ref_draws = shot_draws(ref_dir / "run.log")
    live_draws = shot_draws(live_dir / "run.log")
    return {
        "curve": [[s, round(d, 3), round(f, 4)] for s, d, f in curve],
        "t_converge_s": t_conv,
        "residual_first": round(first, 3),
        "residual_final": round(final, 3),
        "changed_frac_final": round(curve[-1][2], 4),
        "progress": round(final / first, 3) if first > 1e-9 else 0.0,
        "settled_draws": ref_draws.get(sample_name(SAMPLES_S[-1])),
        "live_draws": [live_draws.get(sample_name(s)) for s in SAMPLES_S],
    }


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--accept", action="store_true")
    ap.add_argument("--skip-render", action="store_true")
    args = ap.parse_args()

    viewer = Path(__file__).resolve().parent.parent
    out = viewer / "interchange" / "convreel"
    run_root = viewer / "interchange" / "runs" / "conv"
    out.mkdir(parents=True, exist_ok=True)
    current_p = out / "current.json"
    accepted_p = out / "accepted.json"

    if args.accept:
        if current_p.exists():
            accepted_p.write_text(current_p.read_text())
            print("accepted: current convergence table blessed as baseline")
        else:
            print("nothing to accept - run without --accept first")
        return

    report = {"meta": {
        "samples_s": SAMPLES_S, "converge_eps": CONVERGE_EPS,
        "pose_hz": POSE_HZ, "weather_time_s": WEATHER_TIME_S,
    }, "poses": {}}
    findings = []

    for name, lat, lon, alt, yaw, pitch in POSES:
        row = {}
        if not args.skip_render:
            script = viewer / "scripts" / f"_conv_{name}.play"
            build_play(script, lat, lon, alt, yaw, pitch)
            ok_r, wall_r, _ = run_phase(
                viewer, script, run_root / name / "ref", live=False)
            ok_l, wall_l, shot_wall = run_phase(
                viewer, script, run_root / name / "live", live=True)
            row["wall"] = {
                "ref_s": round(wall_r, 1), "live_s": round(wall_l, 1),
                "live_shot_wall_s": {k: round(v, 2)
                                     for k, v in sorted(shot_wall.items())},
            }
            if not ok_r or not ok_l:
                findings.append(
                    f"{name}: {'ref' if not ok_r else 'live'} phase FAILED")
                report["poses"][name] = row
                continue
        m = measure(run_root, name)
        if m is None:
            findings.append(f"{name}: MISSING/mismatched frames")
            report["poses"][name] = row
            continue
        row.update(m)
        # sim-vs-wall skew: how much faster than real time the live phase
        # ran between the first and last sample (build-arrival honesty).
        sw = row.get("wall", {}).get("live_shot_wall_s", {})
        if len(sw) >= 2:
            ws = [sw[sample_name(s)] for s in SAMPLES_S if sample_name(s) in sw]
            span = ws[-1] - ws[0]
            if span > 1e-6:
                row["sim_per_wall"] = round(
                    (SAMPLES_S[-1] - SAMPLES_S[0]) / span, 2)
        report["poses"][name] = row

        if ((row["residual_final"] > STUCK_RESIDUAL
                or row["changed_frac_final"] > STUCK_FRAC)
                and row["progress"] > STUCK_PROGRESS):
            findings.append(
                f"{name}: STUCK — residual {row['residual_final']} "
                f"(frac {row['changed_frac_final']}) after "
                f"{SAMPLES_S[-1]:.0f}s sim with progress {row['progress']} "
                f"(the B-16 signature: live view is not approaching settled)")

    # baseline drift (numbers, perf_reel-style: the frames bless themselves,
    # the SPEED is what gets accepted)
    if accepted_p.exists():
        base = json.loads(accepted_p.read_text()).get("poses", {})
        for name, row in report["poses"].items():
            b = base.get(name)
            if not b or "residual_final" not in row:
                continue
            bt, ct = b.get("t_converge_s"), row.get("t_converge_s")
            if bt is not None and ct is None:
                findings.append(
                    f"{name}: converged in {bt}s at baseline, now NOT within "
                    f"the {SAMPLES_S[-1]:.0f}s window")
            elif bt is not None and ct is not None and ct > bt * TCONV_MULT + TCONV_ABS:
                findings.append(
                    f"{name}: t_converge {bt}s -> {ct}s")
            br, cr = b.get("residual_final"), row.get("residual_final")
            if br is not None and cr is not None and cr > br * RES_MULT + RES_ABS:
                findings.append(
                    f"{name}: residual_final {br} -> {cr}")

    current_p.write_text(json.dumps(report, indent=1), encoding="utf-8")

    print(f"{'pose':16s} {'t_conv':>7s} {'res_1st':>8s} {'res_end':>8s} "
          f"{'progress':>8s} {'sim/wall':>8s}")
    for name, *_ in POSES:
        row = report["poses"].get(name, {})
        if "residual_final" not in row:
            print(f"{name:16s}   (no data)")
            continue
        tc = row["t_converge_s"]
        print(f"{name:16s} {tc if tc is not None else '>' + format(SAMPLES_S[-1], '.0f'):>7} "
              f"{row['residual_first']:8.2f} {row['residual_final']:8.2f} "
              f"{row['progress']:8.2f} {row.get('sim_per_wall', float('nan')):8.2f}")

    if findings:
        print(f"CONVERGENCE REEL: {len(findings)} finding(s) to review:")
        for f in findings:
            print(" ", f)
        sys.exit(3)
    print(f"CONVERGENCE REEL: all {len(POSES)} poses clean"
          + ("" if accepted_p.exists() else " (no baseline yet - --accept to bless)"))


if __name__ == "__main__":
    main()
