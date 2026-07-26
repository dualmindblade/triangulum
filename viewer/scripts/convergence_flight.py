#!/usr/bin/env python3
"""The recorded-flight convergence lane: "when a real flight stops moving,
does the view sharpen — with all the streaming state that flight built up?"

This is the WARM-STATE sibling of `convergence_reel.py`. That reel holds one
pose from a COLD streaming state, which is the exact blind spot B-16 escaped
through: `BUGS.md` records that the cold-state harness repro at the photo
pose was byte-stable in 3 s and found nothing, and that the defect "needs
accumulated session state". This lane replays an F10 recording — the camera
actually moves — and measures convergence at the flight's own HOLD POINTS,
the moments a player stops and expects the picture to resolve.

MECHANISM — identical discipline to convergence_reel, no Rust changes:

  ref phase   plain harness: `pose` lines advance the sim clock WITHOUT
              drawing; every `shot` settles (the byte-stable capture path)
              = ground truth at that exact sim instant.
  live phase  TRI_POSE_LIVE=1 TRI_RAW_CAPTURE=1: every `pose` line draws its
              full 60 Hz tick cadence of LIVE frames, so the streaming
              scheduler flies the whole recorded route exactly as in
              gameplay; every `shot` photographs the raw in-flight frame.

`shot` advances zero sim ticks in both phases and `pose` advances
`pose_ticks` in both, so the two phases share one timeline and every
comparison is live(t) vs settled(t) at the SAME sim instant — water
animation, sun motion and weather cancel exactly, and the residual is pure
streaming state. Self-blessing: no golden images.

The difference from the reel is WHERE the samples go. Hold points are
detected from the recording itself (a maximal run of pose lines that stays
within POS_EPS_KM / ANG_EPS_DEG of the run's first pose, lasting at least
MIN_HOLD_S). Inside each hold, samples are taken at offsets FROM THE MOMENT
MOTION STOPPED, which is what a player perceives. Between holds a sparse
"cruise" grid measures how far behind the live view runs while moving — that
number never reaches zero and is reported, not gated on day one.

Metrics per hold (same shapes as the reel, so the two lanes read alike):
  t_converge_s    first offset from which every delta stays <= CONVERGE_EPS
  residual_final  mean |delta| at the end of the hold
  tail_ratio      last-third mean / middle-third mean of the curve; ~1.0 =
                  pinned, < 1 = draining. What STUCK gates on.
  stuck           the B-16 signature, baseline-aware exactly as in the reel.
Plus `cruise_mean` / `cruise_max` per flight for the in-motion track.

Findings (exit 3):
  STUCK           big residual at the end of a hold AND a tail that is not
                  draining. Needs no baseline.
  vs accepted     after --accept, regressions in t_converge / residual_final
                  / cruise_mean beyond the reel's own tolerances.

COST — this lane is not free and is not all merge-time. Flights carry a
`tier`: "merge" flights are short enough to run every merge, "deep" flights
are the full accumulated sessions and are opt-in (`--tier deep|all`).
Measured numbers live in docs/missions/V3-recorded-flight-findings.md.

Honesty notes (all inherited from the reel, plus one of our own):
- Sim time is the x-axis but tile builds run on rayon workers in REAL time,
  so wall clock leaks into the curves and absolute numbers are machine-
  dependent. `sim_per_wall` is in the report.
- Each `shot` in the live phase costs one extra draw + readback of wall time
  that a real player would not spend, which hands the background builders a
  small bonus. With ~20 samples over a 100 s flight it is under 2% of the
  flight's wall clock, but it is a bias in the optimistic direction.
- Recordings are replayed with their own weather clock and NO `sun` pin (a
  global sun pin would render far-longitude legs at night); the default
  lights every location at local noon. Identical in both phases, so it does
  not touch the delta.

Usage (from viewer/):
  python scripts/convergence_flight.py                 merge tier
  python scripts/convergence_flight.py --tier all      + the deep flights
  python scripts/convergence_flight.py --flight ice_session
  python scripts/convergence_flight.py --accept        bless current numbers
  python scripts/convergence_flight.py --skip-render   re-measure on disk
  python scripts/convergence_flight.py --list          holds only, no render
Exit codes: 0 clean, 3 findings to review.
"""

import argparse
import json
import math
import sys
from pathlib import Path

import numpy as np
from PIL import Image

sys.path.insert(0, str(Path(__file__).resolve().parent))
try:
    # V-1's reel owns the comparison discipline; this lane reuses the literal
    # code so the two can never drift apart. (If convergence_reel.py is ever
    # dropped, run_phase/shot_draws/the thresholds must be vendored here.)
    from convergence_reel import (  # noqa: E402
        CONVERGE_EPS, RES_ABS, RES_MULT, STUCK_FRAC, STUCK_RESIDUAL,
        STUCK_TAIL_RATIO, TCONV_ABS, TCONV_MULT, run_phase, shot_draws,
    )
except ImportError as exc:  # pragma: no cover - setup error, not a finding
    sys.exit(f"convergence_flight needs scripts/convergence_reel.py: {exc}")


# --- the flight table ------------------------------------------------------
# script paths are relative to viewer/. Only TRACKED scripts belong in the
# default tiers: viewer/interchange/ is gitignored, so a recording that lives
# there is not a gate, it is a local convenience (listed as tier "local" and
# silently skipped when absent).
FLIGHTS = [
    # Austin's F10 descent: 124.8 km -> 0.26 km straight down over 11 s, then
    # a 5 s ground hold. Short, and its warm state is the whole LOD ladder of
    # a descent — the B-11 class. Cheap enough for every merge.
    dict(name="descent_land", script="scripts/rec001-descent.play",
         tier="merge", tail_hold_s=15.0, cruise_every_s=2.0),
    # THE B-16 specimen: Austin's sustained-blur session. 101 s from orbit
    # into the ice region, ending in an 18.7 s hold at the pose that renders
    # soft (41.519 122.511 @3.05, yaw 93 pitch -68). This is the accumulated
    # session state the cold reel cannot reach. Expensive: deep tier.
    # tail_hold_s=45: the recording's own terminal hold is 18.7 s, but the
    # SAME pose reached cold (convergence_reel's b16_ice_hold) needs ~30 s to
    # converge, so an 18.7 s window could not tell "slower than cold" from
    # "shorter window than cold". 45 s puts the warm measurement strictly
    # past the cold one's convergence point.
    dict(name="ice_session", script="scripts/b16-live.play",
         tier="deep", tail_hold_s=45.0, cruise_every_s=20.0),
    # untracked local recordings, if the operator has them
    dict(name="rec002_survey", script="interchange/recordings/rec_002.play",
         tier="local", tail_hold_s=15.0, cruise_every_s=15.0),
]

# Hold detection. A hold is a maximal run of pose lines all within these
# tolerances of the run's FIRST pose — i.e. the camera stopped, from the
# player's point of view. 2 m / 0.05 deg is well below anything a hand on a
# mouse produces and well above recording quantisation.
POS_EPS_KM = 0.002
ANG_EPS_DEG = 0.05
MIN_HOLD_S = 1.5

# Sample offsets inside a hold, measured from the instant motion stopped.
# Dense early (the first second is what a player notices), then doubling.
HOLD_OFFSETS_S = [0.2, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 30.0]
# A hold needs at least this many samples before its tail trend is trusted
# enough to fire STUCK; short holds are measured and reported, not gated.
MIN_SAMPLES_FOR_STUCK = 5

DEG_KM = 111.0  # good enough for a motion threshold


def sample_name(t_s: float) -> str:
    """sim-deciseconds, zero padded. Exactly four digits on purpose: the
    reel's `run_phase` wall-clock parser and `shot_draws` both match
    `shot (s\\d{4}) `, so a wider name silently loses those two columns.
    Caps usable recordings at 999.9 s of sim, which is far past this lane's
    cost budget anyway."""
    return f"s{int(round(t_s * 10)):04d}"


# --- reading a recording ---------------------------------------------------
def read_recording(path: Path):
    """(header_lines, poses, hz). Any `shot`/`bench`/`state` tail in the
    source is dropped: this lane owns the sampling, and `bench` would hand
    the live phase wall time the ref phase does not get."""
    header, poses, hz = [], [], 10.0
    for raw in path.read_text(encoding="utf-8", errors="replace").splitlines():
        line = raw.split("#")[0].strip()
        if not line:
            continue
        tok = line.split()
        head = tok[0].lower()
        if head == "pose":
            poses.append(tuple(float(x) for x in tok[1:6]))
        elif head == "posehz":
            hz = float(tok[1])
        elif head in ("shot", "bench", "state", "assert"):
            continue
        else:
            header.append(line)
    return header, poses, hz


def pose_gap(a, b):
    """(km moved, degrees turned) between two pose samples"""
    dlat = (b[0] - a[0]) * DEG_KM
    dlon = (b[1] - a[1]) * DEG_KM * math.cos(math.radians(a[0]))
    dalt = b[2] - a[2]
    dyaw = abs((b[3] - a[3] + 180.0) % 360.0 - 180.0)
    return math.sqrt(dlat * dlat + dlon * dlon + dalt * dalt), max(dyaw, abs(b[4] - a[4]))


def find_holds(poses, hz):
    """[(i, j, pose)] — maximal runs [i, j) that never leave the tolerance
    ball around poses[i], lasting at least MIN_HOLD_S."""
    out, i, n = [], 0, len(poses)
    while i < n:
        j = i + 1
        while j < n:
            dp, da = pose_gap(poses[i], poses[j])
            if dp > POS_EPS_KM or da > ANG_EPS_DEG:
                break
            j += 1
        if (j - i) / hz >= MIN_HOLD_S:
            out.append((i, j, poses[i]))
        i = j
    return out


def plan(path: Path, tail_hold_s: float, cruise_every_s: float):
    """Everything about one flight that does not need the GPU: the pose list
    (possibly tail-extended), its holds, and where the samples go."""
    header, poses, hz = read_recording(path)
    if not poses:
        return None
    holds = find_holds(poses, hz)
    # Tail extension: if the recording ENDS inside a hold, the player stopped
    # and the recording simply ran out. Repeating the final pose keeps the
    # clock running so the terminal hold gets a full measurement window. This
    # is the same thing b16-live.play does with `bench`, minus the sim skew.
    if holds and tail_hold_s > 0 and holds[-1][1] == len(poses):
        i, j, p = holds[-1]
        need = int(round(tail_hold_s * hz)) - (j - i)
        if need > 0:
            poses = poses + [p] * need
            holds[-1] = (i, len(poses), p)
    # samples: (pose_index_after_which_to_shoot, kind, hold_ordinal, offset)
    samples, taken = [], set()
    for h, (i, j, _p) in enumerate(holds):
        offs = [d for d in HOLD_OFFSETS_S if i + round(d * hz) <= j]
        offs.append((j - i) / hz)          # always the end of the hold
        for d in sorted(set(round(o, 3) for o in offs)):
            k = i + int(round(d * hz))
            if k in taken or k <= 0:
                continue
            taken.add(k)
            samples.append((k, "hold", h, (k - i) / hz))
    if cruise_every_s > 0:
        step = int(round(cruise_every_s * hz))
        in_hold = set()
        for (i, j, _p) in holds:
            in_hold.update(range(i, j + 1))
        for k in range(step, len(poses) + 1, step):
            if k in taken or k in in_hold:
                continue
            taken.add(k)
            samples.append((k, "cruise", -1, k / hz))
    samples.sort()
    return dict(header=header, poses=poses, hz=hz, holds=holds, samples=samples)


def build_play(dest: Path, pl):
    lines = [
        "# generated by convergence_flight.py from a recorded session.",
        "# Both phases run this file: pose lines advance the sim clock",
        "# identically, shot lines advance it not at all.",
    ] + pl["header"] + [f"posehz {pl['hz']:g}"]
    cursor = 0
    for (k, _kind, _h, _off) in pl["samples"]:
        for p in pl["poses"][cursor:k]:
            lines.append("pose %.6f %.6f %.5f %.2f %.2f" % p)
        cursor = k
        lines.append(f"shot {sample_name(k / pl['hz'])}")
    for p in pl["poses"][cursor:]:
        lines.append("pose %.6f %.6f %.5f %.2f %.2f" % p)
    dest.write_text("\n".join(lines) + "\n", encoding="utf-8")


# --- measurement -----------------------------------------------------------
def delta(ref_dir: Path, live_dir: Path, sn: str):
    rp, lp = ref_dir / f"{sn}.png", live_dir / f"{sn}.png"
    if not (rp.exists() and lp.exists()):
        return None
    ref = np.asarray(Image.open(rp).convert("RGB"), dtype=np.float64)
    liv = np.asarray(Image.open(lp).convert("RGB"), dtype=np.float64)
    if ref.shape != liv.shape:
        return None
    d = np.abs(ref - liv).max(axis=2)
    return float(d.mean()), float((d > 6).mean())


def tail_trend(deltas):
    """last-third mean / middle-third mean — the reel's STUCK statistic,
    robust to where the window happens to end."""
    n = len(deltas)
    third = max(1, n // 3)
    mid = deltas[n - 2 * third:n - third]
    last = deltas[n - third:]
    mid_m = sum(mid) / len(mid) if mid else 0.0
    last_m = sum(last) / len(last) if last else 0.0
    return (last_m / mid_m if mid_m > 1e-9 else 0.0), mid_m, last_m


def measure(run_dir: Path, pl):
    ref_dir, live_dir = run_dir / "ref", run_dir / "live"
    ref_draws, live_draws = shot_draws(ref_dir / "run.log"), shot_draws(live_dir / "run.log")
    hz = pl["hz"]
    per_hold = {h: [] for h in range(len(pl["holds"]))}
    cruise = []
    for (k, kind, h, off) in pl["samples"]:
        sn = sample_name(k / hz)
        d = delta(ref_dir, live_dir, sn)
        if d is None:
            return None
        rec = dict(name=sn, t_s=round(k / hz, 2), off_s=round(off, 2),
                   mean=round(d[0], 3), frac=round(d[1], 4),
                   live_draws=live_draws.get(sn), settled_draws=ref_draws.get(sn))
        (per_hold[h] if kind == "hold" else cruise).append(rec)

    holds_out = []
    for h, (i, j, p) in enumerate(pl["holds"]):
        rows = per_hold[h]
        if not rows:
            continue
        deltas = [r["mean"] for r in rows]
        t_conv = None
        for a in range(len(deltas)):
            if all(x <= CONVERGE_EPS for x in deltas[a:]):
                t_conv = rows[a]["off_s"]
                break
        ratio, mid_m, last_m = tail_trend(deltas)
        stuck = bool(
            len(rows) >= MIN_SAMPLES_FOR_STUCK
            and (deltas[-1] > STUCK_RESIDUAL or rows[-1]["frac"] > STUCK_FRAC)
            and ratio > STUCK_TAIL_RATIO)
        holds_out.append(dict(
            hold=h, t_start_s=round(i / hz, 2), duration_s=round((j - i) / hz, 2),
            pose=[round(x, 6) for x in p],
            samples=[[r["off_s"], r["mean"], r["frac"], r["live_draws"],
                      r["settled_draws"]] for r in rows],
            t_converge_s=t_conv,
            residual_first=deltas[0], residual_final=deltas[-1],
            changed_frac_final=rows[-1]["frac"],
            tail_ratio=round(ratio, 3), tail_mid_mean=round(mid_m, 3),
            tail_last_mean=round(last_m, 3),
            gateable=len(rows) >= MIN_SAMPLES_FOR_STUCK, stuck=stuck))

    cm = [r["mean"] for r in cruise]
    return dict(
        holds=holds_out,
        cruise=dict(samples=[[r["t_s"], r["mean"], r["frac"], r["live_draws"],
                              r["settled_draws"]] for r in cruise],
                    mean=round(sum(cm) / len(cm), 3) if cm else None,
                    max=round(max(cm), 3) if cm else None))


# --- driver ----------------------------------------------------------------
def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--accept", action="store_true")
    ap.add_argument("--skip-render", action="store_true")
    ap.add_argument("--list", action="store_true",
                    help="print the detected holds and sample plan, render nothing")
    ap.add_argument("--tier", default="merge", choices=["merge", "deep", "all"])
    ap.add_argument("--flight", default=None, help="run just this flight")
    args = ap.parse_args()

    viewer = Path(__file__).resolve().parent.parent
    out = viewer / "interchange" / "convflight"
    run_root = viewer / "interchange" / "runs" / "convflight"
    out.mkdir(parents=True, exist_ok=True)
    current_p, accepted_p = out / "current.json", out / "accepted.json"

    if args.accept:
        if current_p.exists():
            accepted_p.write_text(current_p.read_text())
            print("accepted: current flight table blessed as baseline")
        else:
            print("nothing to accept - run without --accept first")
        return

    # merge = the cheap tier only; deep = merge + the accumulated sessions;
    # all = everything the operator happens to have, untracked ones included.
    tiers = {"merge": {"merge"}, "deep": {"merge", "deep"},
             "all": {"merge", "deep", "local"}}[args.tier]
    wanted = [f for f in FLIGHTS
              if (f["name"] == args.flight if args.flight else f["tier"] in tiers)]
    if args.flight and not wanted:
        sys.exit(f"no such flight: {args.flight} "
                 f"(have: {', '.join(f['name'] for f in FLIGHTS)})")

    report = {"meta": dict(converge_eps=CONVERGE_EPS, hold_offsets_s=HOLD_OFFSETS_S,
                           pos_eps_km=POS_EPS_KM, ang_eps_deg=ANG_EPS_DEG,
                           min_hold_s=MIN_HOLD_S, tier=args.tier),
              "flights": {}}
    findings, known_stuck, skipped = [], [], []
    # --skip-render re-measures frames already on disk; carry the wall-clock
    # block from the run that produced them, or a re-measure silently erases
    # the only record of what the render cost.
    prev = (json.loads(current_p.read_text()).get("flights", {})
            if (args.skip_render and current_p.exists()) else {})

    for f in wanted:
        src = viewer / f["script"]
        if not src.exists():
            skipped.append(f"{f['name']}: {f['script']} not present")
            continue
        pl = plan(src, f["tail_hold_s"], f["cruise_every_s"])
        if pl is None:
            skipped.append(f"{f['name']}: no pose lines in {f['script']}")
            continue
        n_hold = sum(1 for s in pl["samples"] if s[1] == "hold")
        row = dict(script=f["script"], tier=f["tier"], hz=pl["hz"],
                   # the recording's own header (weather clock, mode) — the
                   # b16 load is season-sensitive, so record what it ran at
                   header=pl["header"],
                   sim_s=round(len(pl["poses"]) / pl["hz"], 1),
                   n_holds=len(pl["holds"]),
                   n_samples=[n_hold, len(pl["samples"]) - n_hold])
        if args.list:
            print(f"{f['name']:14s} {row['sim_s']:6.1f}s sim  {len(pl['holds'])} holds  "
                  f"{n_hold} hold + {len(pl['samples']) - n_hold} cruise samples")
            for h, (i, j, p) in enumerate(pl["holds"]):
                print(f"    hold {h}: t={i/pl['hz']:6.1f}s dur={(j-i)/pl['hz']:5.1f}s "
                      f"lat {p[0]:9.4f} lon {p[1]:9.4f} alt {p[2]:10.4f} "
                      f"yaw {p[3]:7.2f} pitch {p[4]:7.2f}")
            report["flights"][f["name"]] = row
            continue

        run_dir = run_root / f["name"]
        if not args.skip_render:
            script = viewer / "scripts" / f"_convflight_{f['name']}.play"
            build_play(script, pl)
            ok_r, wall_r, _ = run_phase(viewer, script, run_dir / "ref", live=False)
            ok_l, wall_l, shot_wall = run_phase(viewer, script, run_dir / "live", live=True)
            row["wall"] = dict(ref_s=round(wall_r, 1), live_s=round(wall_l, 1),
                               live_shot_wall_s={k: round(v, 2)
                                                 for k, v in sorted(shot_wall.items())})
            if len(shot_wall) >= 2:
                ws = sorted(shot_wall.items())
                span = ws[-1][1] - ws[0][1]
                sim_span = (int(ws[-1][0][1:]) - int(ws[0][0][1:])) / 10.0
                if span > 1e-6:
                    row["sim_per_wall"] = round(sim_span / span, 2)
            if not ok_r or not ok_l:
                findings.append(f"{f['name']}: {'ref' if not ok_r else 'live'} phase FAILED")
                report["flights"][f["name"]] = row
                continue
        else:
            for k in ("wall", "sim_per_wall"):
                if k in prev.get(f["name"], {}):
                    row[k] = prev[f["name"]][k]
        m = measure(run_dir, pl)
        if m is None:
            findings.append(f"{f['name']}: MISSING/mismatched frames")
            report["flights"][f["name"]] = row
            continue
        row.update(m)
        report["flights"][f["name"]] = row

    if args.list:
        return

    base = (json.loads(accepted_p.read_text()).get("flights", {})
            if accepted_p.exists() else {})

    for name, row in report["flights"].items():
        bh = {h["hold"]: h for h in base.get(name, {}).get("holds", [])}
        for h in row.get("holds", []):
            tag = f"{name} hold {h['hold']} (t={h['t_start_s']}s, {h['duration_s']}s"
            detail = (f"residual {h['residual_final']} (frac {h['changed_frac_final']}) "
                      f"at hold end, tail not draining (last third "
                      f"{h['tail_last_mean']} vs middle third {h['tail_mid_mean']}, "
                      f"ratio {h['tail_ratio']})")
            if h["stuck"]:
                if bh.get(h["hold"], {}).get("stuck"):
                    known_stuck.append(f"{tag}): still pinned as blessed — {detail}")
                else:
                    findings.append(f"{tag}): STUCK — {detail} (the B-16 signature: "
                                    f"a held view that is not approaching settled)")
            b = bh.get(h["hold"])
            if not b:
                continue
            bt, ct = b.get("t_converge_s"), h.get("t_converge_s")
            if bt is not None and ct is None:
                findings.append(f"{tag}): converged {bt}s after the stop at baseline, "
                                f"now not inside the hold at all")
            elif bt is not None and ct is not None and ct > bt * TCONV_MULT + TCONV_ABS:
                findings.append(f"{tag}): t_converge {bt}s -> {ct}s")
            br, cr = b.get("residual_final"), h.get("residual_final")
            if br is not None and cr is not None and cr > br * RES_MULT + RES_ABS:
                findings.append(f"{tag}): residual_final {br} -> {cr}")
        bc, cc = base.get(name, {}).get("cruise", {}), row.get("cruise", {})
        if bc.get("mean") is not None and cc.get("mean") is not None \
                and cc["mean"] > bc["mean"] * RES_MULT + RES_ABS:
            findings.append(f"{name}: in-motion cruise_mean {bc['mean']} -> {cc['mean']} "
                            f"(the live view fell further behind while flying)")

    current_p.write_text(json.dumps(report, indent=1), encoding="utf-8")

    print(f"{'flight/hold':26s} {'stop@':>7s} {'dur':>6s} {'t_conv':>7s} "
          f"{'res_1st':>8s} {'res_end':>8s} {'tailrat':>8s}")
    for name, row in report["flights"].items():
        for h in row.get("holds", []):
            tc = h["t_converge_s"]
            print(f"{name + ' h' + str(h['hold']):26s} {h['t_start_s']:7.1f} "
                  f"{h['duration_s']:6.1f} "
                  f"{tc if tc is not None else '>' + format(h['duration_s'], '.0f'):>7} "
                  f"{h['residual_first']:8.2f} {h['residual_final']:8.2f} "
                  f"{h['tail_ratio']:8.2f}"
                  + ("" if h["gateable"] else "  (short hold: not gated)"))
        c = row.get("cruise", {})
        if c.get("mean") is not None:
            print(f"{name + ' [in motion]':26s} cruise mean {c['mean']:.2f} "
                  f"max {c['max']:.2f} over {len(c['samples'])} samples"
                  + (f", sim/wall {row['sim_per_wall']:.2f}" if "sim_per_wall" in row else ""))
    for s in skipped:
        print(f"  skipped: {s}")
    for k in known_stuck:
        print(f"  known-pinned (blessed, not a finding): {k}")

    if findings:
        print(f"CONVERGENCE FLIGHT: {len(findings)} finding(s) to review:")
        for x in findings:
            print(" ", x)
        sys.exit(3)
    n = sum(len(r.get("holds", [])) for r in report["flights"].values())
    print(f"CONVERGENCE FLIGHT: all {n} hold(s) across "
          f"{len(report['flights'])} flight(s) clean"
          + ("" if accepted_p.exists() else " (no baseline yet - --accept to bless)"))


if __name__ == "__main__":
    main()
