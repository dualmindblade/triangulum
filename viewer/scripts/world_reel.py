#!/usr/bin/env python3
"""The world reel: a standing pose table covering every water/biome class,
rendered deterministically and DIFFED AGAINST THE LAST ACCEPTED REEL.

This is a change detector, not a quality metric: after any terrain/water
merge, changed poses are flagged with per-pose stats and it is a human/AI
job to explain each change or fix it. It systematizes what Andrew's
field-flying caught this week (regression zoos invisible to census,
colcensus, and sync-diff, because both renderers agreed on the defect).

Every frame also runs FRAME LINT - screen-space heuristics for defect
classes that bit us:
  VOID   clusters of near-black pixels below the horizon in a daytime
         shot (see-through cracks, missing geometry)
  GRID   strong axis-aligned periodicity in water regions (per-quad
         shading seams reading as a checkerboard)

And before ANY of that, every pose must PROVE ITS OWN PREMISE - see the
PREMISE block below. A pose that does not contain the class it is named
for is a gate pointed at nothing, and it reports green forever (B-20).

Usage (from viewer/):
  python scripts/world_reel.py            premise + render + lint + diff
  python scripts/world_reel.py --accept   bless the current reel as accepted
  python scripts/world_reel.py --skip-render   premise + re-diff existing frames
  python scripts/world_reel.py --premise-only  just the premise check + table
  python scripts/world_reel.py --check-pose NAME,LAT,LON,ALT,YAW,PITCH,PREMISE
                                          scout a candidate pose without
                                          rendering it (repeatable)
Exit codes: 0 clean, 3 = changes/lint/premise findings to review (never
blocks a human; missions treat 3 as "explain every flagged pose in
FINDINGS.md").
"""

import argparse
import json
import math
import re
import shutil
import subprocess
import sys
from pathlib import Path

import numpy as np
from PIL import Image

# ---------------------------------------------------------------- PREMISE
#
# THE INVARIANT: a reel pose named for a world class must CONTAIN that
# class, and that is checked automatically before a single pixel is
# compared - never by a human reading the pose's name.
#
# Why this exists: `lake_archi` was a featureless blue rectangle and
# `coast_beach` was dry highland grassland, both BLESSED, both reporting
# green on every merge for classes they did not contain (B-20). Nothing in
# the repo verified that a gate was pointed at what it claimed to guard.
#
# How it is measured: a uniform grid in SCREEN space is cast through the
# pose's own camera onto the local ground datum, and each hit is `probe`d.
# Because the grid is uniform in screen space, every fraction below is
# literally a fraction of the frame - `water>=0.05` means "at least 5% of
# the frame is over a water surface". `probe` costs microseconds, so the
# whole 27-pose check is ~4 s.
#
# Deliberate approximations, so nobody mistakes this for a renderer:
#   - the ground is a sphere at the camera's own datum. Relief and
#     occlusion are ignored, so a foreground ridge does not hide what is
#     behind it. This makes the check OPTIMISTIC: it can pass a pose whose
#     class is occluded, but it cannot pass one whose class is absent -
#     and absence is the failure mode B-20 is about.
#   - rays that miss the planet are counted as `sky` and excluded from the
#     ground measures.
#
# Vocabulary (all fractions are OF THE WHOLE FRAME):
#   sky     rays that never reach the ground
#   water   a finite water surface at the sample (lake, pond, sea, river)
#   land    ground with no water surface
#   lake    probe `lake=true`
#   river   probe river wetness > 0
#   frozen  probe `frozen=true` (the reel pins the season, so this is
#           the season the pose is actually shot in)
#   h_min/h_max     terrain height at the samples, metres
#   temp_min/max    seasonal temperature at the samples, C
#   dist_med_km     median camera->sample distance
# Syntax: `term OP value` joined by `&`, OP is >= or <=.
# `None` means "no premise expressible with today's `probe`" and MUST
# carry a reason on the same line - an honest hole is worth more than a
# fake assertion.
PREMISE_GRID = (32, 18)   # screen samples across, down
FOV_Y_DEG = 65.0          # camera.rs: perspective_infinite_reverse_rh(65 deg)
FRAME_W, FRAME_H = 1280, 720
R_KM = 8660.254037844386  # Neisor, assets/meta.json

# name, lat, lon, alt_km, yaw, pitch, premise  (sun pinned near-noon per
# pose; weather pinned clear: the reel isolates WORLD changes, not sky ones)
POSES = [
    # the regression-dossier zones (A-4/A-5 reverts, 2026-07-11)
    ("dossier_shore",   13.348,  -4.807, 0.011, 178, -19, "water>=0.01 & land>=0.10"),
    ("dossier_dlake",   13.346,  -4.807, 0.064, 113, -64, "lake>=0.05 & land>=0.05"),
    # NAME LIES (D-1, 2026-07-26): there is not a drop of water in this
    # frame. It is a dry sand/grass patchwork and that is what it guards.
    # Rename proposed to Austin; the premise below asserts what is really
    # there so the pose stops claiming a class it has never contained.
    ("dossier_ponds",   12.199, -44.827, 0.100, -90, -45, "land>=0.90"),
    ("dossier_band",    13.013,  -4.546, 0.012, 163, -12, "water>=0.20"),
    # water classes
    ("karst_field",     13.346,  -4.807, 0.289, -78, -55, "lake>=0.05 & land>=0.10"),
    ("river_wide",      15.024,  17.662, 0.154, -75, -21, "river>=0.30 & water>=0.02"),
    ("river_mouth",      7.042,  33.477, 0.300, -55, -16, "water>=0.20 & river>=0.20 & land>=0.05"),
    ("river_valley",     4.990, -29.403, 0.250, -10, -40, "river>=0.30 & water>=0.02 & land>=0.20"),
    # the channel is thinner than one screen sample here, so the water
    # SURFACE fraction is not assertable; river wetness is.
    ("waterfall_site",   9.79546, 107.60747, 3.000, 84.75, -68, "river>=0.30"),
    ("pond_country",    -0.798, -67.941, 0.150,  40, -35, "water>=0.10 & land>=0.10"),
    # RE-AIMED 2026-07-26 (D-1, B-20). Was 16.601 -12.222: dead centre of a
    # 130 km lake, 100% open water, no shore in frame - a blue rectangle.
    # Now the lake's southern island field: 49.5% water / 50.5% land, a
    # strait, islands and a sand spit. NEEDS A FRESH BLESSED FRAME.
    ("lake_archi",      15.4128, -12.570, 3.000,   0, -70, "lake>=0.10 & land>=0.10"),
    ("inland_sea",      20.633, 127.615, 8.000,   0, -75, "water>=0.90"),
    # RE-AIMED 2026-07-26 (D-1, B-20). Was 8.097 -35.673: dry highland
    # grassland at 923 m with no water for kilometres. Now V-2's verified
    # ocean waterline, 0.3 km inland looking seaward: sand, turquoise
    # shoal, open sea. NEEDS A FRESH BLESSED FRAME.
    ("coast_beach",     15.104945, 150.918258, 0.060, 58, -30, "water>=0.05 & land>=0.10 & h_min<=5"),
    # RE-AIMED 2026-07-26 (D-1). Was 51.180 85.830: a dry green plain at
    # 373-409 m with no water and no ice anywhere in frame - a third dead
    # pose, found by the premise check on its first run. Now a genuinely
    # frozen lake (ice sheet + snowy shore, -4.4 C) that THAWS at season
    # 0.55, so the summer pose below is a real seasonal contrast rather
    # than a duplicate. NEEDS A FRESH BLESSED FRAME.
    ("frozen_lake",     69.9560,  50.9800, 0.300,   0, -50, "water>=0.10 & frozen>=0.05"),
    ("sea_ice",         83.997,  40.220, 0.200,   0, -35, "water>=0.50 & frozen>=0.50"),
    # ADDED 2026-07-26 (D-1). These are `coast_beach`'s OLD coordinates.
    # The frame is a 2.8 deg slope terraced into 1 m contour rings 20.6 m
    # apart - the honest voxel lattice, proved by a `voxels off` A/B (the
    # rings vanish), NOT the F-5 shading bug returning. It was the reel's
    # only coverage of that look, so re-aiming coast_beach would have
    # silently dropped it. No accepted baseline yet, like the moon poses.
    ("gentle_terrace",   8.097, -35.673, 0.060,  77, -30, "land>=0.90 & h_min>=500"),
    # biome/terrain classes. `probe` does not return koppen today (D-2 is
    # adding it), so these premises are PARTIAL: they pin the part of the
    # class that is checkable and say so. A partial premise still catches
    # the B-20 failure mode - a pose that has drifted off its feature
    # entirely - which is what it is for.
    ("ecotone_corner",  16.684, -23.367, 0.414, -91, -80, None),   # needs koppen: the class IS the koppen edge
    ("desert",           0.870, -89.800, 0.120, -35, -30, "water<=0.02"),        # partial: aridity needs precip/koppen
    ("savanna",          1.510,  40.490, 0.150, 150, -40, "land>=0.90 & temp_min>=15"),  # partial: needs koppen
    ("jungle",          -0.906, -67.804, 0.094,-210, -17, "temp_min>=20"),       # partial: needs koppen
    # SUSPECT (D-1, 2026-07-26): climatically boreal (1.2 C) but the frame
    # is a treeless plain meeting open water - 38% of it is lake. The
    # temperature premise below PASSES on a frame that shows no taiga at
    # all, which is the honest limit of a koppen-free check. Flagged for a
    # human verdict; do not read its green as coverage of the taiga class.
    ("taiga",           47.861,  14.399, 0.592, -49, -62, "temp_max<=8 & land>=0.20"),
    ("peak",            69.000, 122.000, 0.800,  20, -35, "h_min>=3000 & frozen>=0.50"),
    ("groves",           3.960, -32.560, 0.150,  60, -25, "land>=0.50"),          # partial: needs koppen/tree cover
    # range checks - the class here is the RANGE, and distance is exactly
    # what the footprint model measures, so these are fully assertable.
    ("mid_altitude",    15.024,  17.650, 6.000, -90, -60, "dist_med_km>=2.0"),
    ("orbital_coast",   15.238, 150.848, 291.4, 146, -86, "water>=0.20 & land>=0.20 & dist_med_km>=100"),
    ("orbital_biomes",  42.765,  38.439, 288.2, -64, -85, "land>=0.50 & dist_med_km>=100"),
]

SUMMER_POSES = [
    # the summer counterparts shoot the SAME ground in season 0.55, so the
    # ice premise is deliberately dropped - melting is the thing being
    # looked at. The water must still be there.
    ("frozen_lake_summer", 69.9560, 50.9800, 0.300, 0, -50, "water>=0.10"),
    ("sea_ice_summer", 83.997, 40.220, 0.200, 0, -35, "water>=0.50"),
]

# name, moon-local lat/lon, altitude km, local yaw/pitch. Moon poses use the
# physical epoch Sun; pinned art directions are observer-relative and are not
# valid while the camera itself moves between bodies.
# These are P2 additions and deliberately have no accepted baseline yet.
# `probe` samples NEISOR's terrain, so there is no premise it can express
# about a moon pose. That is a real hole, not an oversight: it needs a
# moon-side probe. The MOON lint below is the only content check they get.
MOON_POSES = [
    ("moon_orbit_face", 10.0, 40.0, 1700.0, 0, -86, None),
    ("moon_low_flyby", 35.4023, 20.0000, 8.0, -90, -20, None),
]
ALL_POSES = [*POSES, *SUMMER_POSES, *MOON_POSES]
# every pose that a Neisor-side probe can say anything about
PREMISE_POSES = [(*p, 0.45) for p in POSES] + [(*p, 0.55) for p in SUMMER_POSES]

CHANGE_MEAN = 1.0   # mean |delta| over the frame that counts as "changed"
CHANGE_FRAC = 0.01  # or >1% of pixels differing by >6


def build_play(path: Path):
    lines = [
        "# generated by world_reel.py",
        "mode fly",
        "weather pin 0.10 0.0",
        "weather time 0",
        "weather season 0.45",
    ]
    for name, lat, lon, alt, yaw, pitch, _ in POSES:
        sun_lat = max(-23.0, min(23.0, lat))
        lines += [
            f"teleport {lat} {lon} {alt}",
            f"look {yaw} {pitch}",
            f"sun {sun_lat:.2f} {lon - 15.0:.2f}",
            f"shot {name}",
        ]
    lines.append("weather season 0.55")
    for name, lat, lon, alt, yaw, pitch, _ in SUMMER_POSES:
        sun_lat = max(-23.0, min(23.0, lat))
        lines += [
            f"teleport {lat} {lon} {alt}",
            f"look {yaw} {pitch}",
            f"sun {sun_lat:.2f} {lon - 15.0:.2f}",
            f"shot {name}",
        ]
    lines.append("weather season 0.45")
    lines.append("sun physical")
    for index, (name, lat, lon, alt, yaw, pitch, _) in enumerate(MOON_POSES):
        lines += [
            f"moonpose {lat} {lon} {alt} {yaw} {pitch}",
            *( ["focus free"] if index == 1 else [] ),
            f"shot {name}",
        ]
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")


# ------------------------------------------------- premise: the geometry
# Duplicated deliberately from altitude_ladder.py rather than shared: V-1
# recommended deferring a reel_common.py until a fifth reel exists, and
# forty lines of sphere trig is a cheaper duplicate than a premature
# module every reel then depends on. If a third caller appears, extract it.


def _unit(lat_deg: float, lon_deg: float) -> np.ndarray:
    la, lo = math.radians(lat_deg), math.radians(lon_deg)
    return np.array([math.cos(la) * math.cos(lo), math.cos(la) * math.sin(lo),
                     math.sin(la)])


def _latlon_of(v: np.ndarray) -> tuple[float, float]:
    v = v / np.linalg.norm(v)
    return (math.degrees(math.asin(float(np.clip(v[2], -1, 1)))),
            math.degrees(math.atan2(v[1], v[0])))


def _local_frame(pos: np.ndarray):
    """(radial up, north, east) exactly as camera.rs builds it."""
    up = pos / np.linalg.norm(pos)
    east = np.cross([0.0, 0.0, 1.0], up)
    n = np.linalg.norm(east)
    east = east / n if n > 1e-9 else np.array([1.0, 0.0, 0.0])
    north = np.cross(up, east)
    return up, north / np.linalg.norm(north), east


def footprint(lat, lon, alt_km, yaw, pitch):
    """Uniform SCREEN-space grid through this pose's camera, hit against the
    local ground datum. Returns one entry per grid cell: (lat, lon,
    distance_km) or None for a ray that leaves the planet.

    Uniform in screen space is the whole point - it makes every fraction
    derived from these samples a fraction of the FRAME, which is the thing
    a pose is supposed to contain."""
    cam = _unit(lat, lon) * (R_KM + alt_km)
    up, north, east = _local_frame(cam)
    y, p = math.radians(yaw), math.radians(pitch)
    horiz = north * math.cos(y) + east * math.sin(y)
    look = horiz * math.cos(p) + up * math.sin(p)
    look = look / np.linalg.norm(look)
    right = np.cross(look, up)
    n = np.linalg.norm(right)
    right = right / n if n > 1e-9 else east
    view_up = np.cross(right, look)
    th_y = math.tan(math.radians(FOV_Y_DEG / 2.0))
    th_x = th_y * (FRAME_W / FRAME_H)
    gx, gy = PREMISE_GRID
    out = []
    for iy in range(gy):
        for ix in range(gx):
            ndc_x = (ix + 0.5) / gx * 2.0 - 1.0
            ndc_y = 1.0 - (iy + 0.5) / gy * 2.0
            d = look + right * (ndc_x * th_x) + view_up * (ndc_y * th_y)
            d /= np.linalg.norm(d)
            b = float(np.dot(cam, d))
            c = float(np.dot(cam, cam)) - R_KM * R_KM
            disc = b * b - c
            hit = None
            if disc >= 0.0:
                s = math.sqrt(disc)
                for t in (-b - s, -b + s):
                    if t > 0:
                        hit = (*_latlon_of(cam + d * t), t)
                        break
            out.append(hit)
    return out


PROBE_RE = re.compile(
    r"probe (-?[\d.]+) (-?[\d.]+): h=(-?[\d.]+)m water=(\S+) lake=(\w+) "
    r"frozen=(\w+) annual=(-?[\d.]+)C seasonal=(-?[\d.]+)C lake_lvl=(\S+) "
    r"pond_lvl=(\S+) riv hw=(-?[\d.]+)m wet=([\d.]+) \|"
)


def run_probe_pass(viewer: Path, lines: list[str], tag: str) -> list[dict]:
    """One `play` run of nothing but `probe` lines. ~15 k probes in ~4 s."""
    script = viewer / "scripts" / f"_world-reel-{tag}.play"
    script.write_text("\n".join(lines) + "\n", encoding="utf-8")
    exe = viewer / "target" / "release" / "examples" / "play.exe"
    r = subprocess.run([str(exe), str(script)], cwd=viewer,
                       capture_output=True, text=True)
    if r.returncode != 0:
        sys.stderr.write(r.stdout[-1500:] + r.stderr[-1500:])
        raise SystemExit(f"premise probe pass failed ({r.returncode})")
    rows = []
    for m in PROBE_RE.finditer(r.stdout):
        g = m.groups()
        rows.append({
            "h_m": float(g[2]),
            "water": g[3] != "-",
            "lake": g[4] == "true",
            "frozen": g[5] == "true",
            "temp_c": float(g[7]),
            "river": float(g[11]) > 0.0,
        })
    return rows


def measure_poses(viewer: Path, poses) -> dict:
    """poses: (name, lat, lon, alt, yaw, pitch, premise, season). Returns
    {name: measures}. One probe run for the whole table."""
    lines = ["weather pin 0.10 0.0", "weather time 0"]
    index, k, season_now = {}, 0, None
    for name, lat, lon, alt, yaw, pitch, premise, season in poses:
        if season != season_now:
            lines.append(f"weather season {season}")
            season_now = season
        cells = []
        for hit in footprint(lat, lon, alt, yaw, pitch):
            if hit is None:
                cells.append(None)
            else:
                lines.append(f"probe {hit[0]:.6f} {hit[1]:.6f}")
                cells.append((k, hit[2]))
                k += 1
        index[name] = cells
    rows = run_probe_pass(viewer, lines, "premise")
    if len(rows) != k:
        raise SystemExit(f"premise: parsed {len(rows)} probes, expected {k}")

    out = {}
    for name, *_ in poses:
        cells = index[name]
        total = len(cells)
        on = [(rows[i], d) for c in cells if c is not None for (i, d) in [c]]
        m = {"samples": total, "ground": len(on),
             "sky": round(1.0 - len(on) / total, 4)}
        if not on:
            out[name] = m
            continue
        frac = lambda key: round(sum(1 for r, _ in on if r[key]) / total, 4)  # noqa: E731
        m["water"] = frac("water")
        m["lake"] = frac("lake")
        m["river"] = frac("river")
        m["frozen"] = frac("frozen")
        m["land"] = round(1.0 - m["sky"] - m["water"], 4)
        m["h_min"] = round(min(r["h_m"] for r, _ in on), 1)
        m["h_max"] = round(max(r["h_m"] for r, _ in on), 1)
        m["temp_min"] = round(min(r["temp_c"] for r, _ in on), 2)
        m["temp_max"] = round(max(r["temp_c"] for r, _ in on), 2)
        m["dist_med_km"] = round(float(np.median([d for _, d in on])), 3)
        m["dist_max_km"] = round(max(d for _, d in on), 3)
        out[name] = m
    return out


def eval_premise(expr: str, m: dict) -> list[str]:
    """`term OP value` joined by `&`. Returns the failed terms, verbatim,
    each annotated with what was actually measured."""
    fails = []
    for term in expr.split("&"):
        term = term.strip()
        for op in (">=", "<="):
            if op in term:
                key, val = term.split(op, 1)
                key, val = key.strip(), float(val)
                got = m.get(key)
                if got is None:
                    fails.append(f"{term}  [no measure named '{key}']")
                elif (got < val) if op == ">=" else (got > val):
                    fails.append(f"{term}  [measured {key}={got}]")
                break
        else:
            fails.append(f"{term}  [unparsable: need >= or <=]")
    return fails


def check_premises(viewer: Path, poses, verbose: bool):
    """Returns (findings, measures). A finding here means a pose is not
    pointed at the class it is named for."""
    measures = measure_poses(viewer, poses)
    findings = []
    if verbose:
        print(f"{'pose':22s} {'sky':>6s} {'water':>6s} {'land':>6s} {'lake':>6s} "
              f"{'river':>6s} {'frozen':>6s} {'h_min':>8s} {'h_max':>8s} "
              f"{'t_min':>7s} {'d_med':>8s}  premise")
    for name, lat, lon, alt, yaw, pitch, premise, season in poses:
        m = measures[name]
        if verbose:
            print(f"{name:22s} {m['sky']:6.3f} {m.get('water', float('nan')):6.3f} "
                  f"{m.get('land', float('nan')):6.3f} {m.get('lake', float('nan')):6.3f} "
                  f"{m.get('river', float('nan')):6.3f} {m.get('frozen', float('nan')):6.3f} "
                  f"{m.get('h_min', float('nan')):8.1f} {m.get('h_max', float('nan')):8.1f} "
                  f"{m.get('temp_min', float('nan')):7.2f} "
                  f"{m.get('dist_med_km', float('nan')):8.2f}  "
                  f"{premise if premise else '- (no probe-expressible premise)'}")
        if not premise:
            m["premise"] = None
            continue
        m["premise"] = premise
        bad = eval_premise(premise, m)
        m["premise_ok"] = not bad
        if bad:
            findings.append((name, premise, bad, m))
    return findings, measures


def report_premise_failures(findings) -> None:
    print()
    print("=" * 72)
    print(f"PREMISE FAILURE: {len(findings)} pose(s) do NOT contain the class "
          "they are named for.")
    print("A reel pose that does not contain its class reports green forever")
    print("and guards nothing (BUGS.md B-20). Nothing was rendered.")
    print("=" * 72)
    for name, premise, bad, m in findings:
        print(f"\n  {name}")
        print(f"    premise : {premise}")
        print(f"    measured: sky={m['sky']} water={m.get('water')} "
              f"land={m.get('land')} lake={m.get('lake')} river={m.get('river')} "
              f"frozen={m.get('frozen')}")
        print(f"              h={m.get('h_min')}..{m.get('h_max')} m  "
              f"temp={m.get('temp_min')}..{m.get('temp_max')} C  "
              f"dist_med={m.get('dist_med_km')} km")
        for b in bad:
            print(f"    FAILED  : {b}")
    print("\nRe-aim the pose (scout candidates with --check-pose, which probes")
    print("without rendering) or correct its premise - and say which, in the")
    print("findings doc. Do not delete the premise to make this go away.")


def lint_void(im: np.ndarray) -> tuple[int, float]:
    """Largest connected near-black blob below the horizon band, plus the
    near-black fraction. Daytime pinned sun: real shadow floors are dim but
    not black; voids (sky-through-cracks at z=0 clear color, unlit gaps)
    are."""
    h = im.shape[0]
    below = im[int(h * 0.45):, :, :]
    dark = (below.max(axis=2) < 14).astype(np.uint8)
    frac = float(dark.mean())
    # cheap 4-connected blob size via repeated dilation-free row scan:
    # label runs per row, merge with the row above (two-pass union small)
    biggest = 0
    prev = {}
    labels = {}
    parent = {}

    def find(a):
        while parent.get(a, a) != a:
            parent[a] = parent.get(parent[a], parent[a])
            a = parent[a]
        return a

    nxt = 0
    for y in range(dark.shape[0]):
        row = dark[y]
        cur = {}
        x = 0
        w = dark.shape[1]
        while x < w:
            if row[x]:
                x2 = x
                while x2 < w and row[x2]:
                    x2 += 1
                lbl = None
                for px in range(x, x2):
                    if px in prev:
                        r = find(prev[px])
                        if lbl is None:
                            lbl = r
                        elif r != lbl:
                            parent[find(r)] = find(lbl)
                for px in range(x, x2):
                    if lbl is None:
                        lbl = nxt
                        nxt += 1
                    cur[px] = lbl
                labels[lbl] = labels.get(find(lbl), 0)
                x = x2
            else:
                x += 1
        counts = {}
        for px, lbl in cur.items():
            counts[find(lbl)] = counts.get(find(lbl), 0) + 1
        for lbl, c in counts.items():
            labels[lbl] = labels.get(lbl, 0) + c
            biggest = max(biggest, labels[lbl])
        prev = cur
    return biggest, frac


def lint_grid(im: np.ndarray) -> float:
    """Axis-aligned periodicity score restricted to WATER pixels: open water
    should be featureless, so any comb structure in its edge profile is a
    seam artifact (the water2 checkerboard class). The block lattice on
    LAND is honest periodicity and is excluded by the water mask. Returns
    0 when the central band holds too little water to judge."""
    h, w = im.shape[:2]
    crop = im[int(h * 0.35):int(h * 0.80), int(w * 0.15):int(w * 0.85)]
    r, g, b = crop[:, :, 0], crop[:, :, 1], crop[:, :, 2]
    # `b > g + 8` alone MISSES turquoise shoal water (~90,180,180, where
    # b ~= g) and the pale water of a cold coast (~140,162,170) - i.e. it
    # was blind to shallow water, which is exactly where the per-quad
    # shading seams this lint hunts would show (found by V-2, fixed by
    # D-1). The `b > r + 20` arm catches both. Calibrated against `probe`
    # truth on 15,168 screen cells of the reel: recall on water-surface
    # cells 0.813 -> 0.975, precision 0.9764 -> 0.9787, and NOT ONE cell
    # the old mask saw is dropped. Three poses (coast_beach, taiga,
    # frozen_lake_summer) went from seeing 0% of their own water to >96%.
    # See docs/missions/D1-evidence/lint_grid-shoal-calibration.txt.
    water = ((b > g + 8) | (b > r + 20)) & (b > 80)
    if water.mean() < 0.15:
        return 0.0
    lum = crop @ np.array([0.2126, 0.7152, 0.0722])
    score = 0.0
    for axis in (0, 1):
        d = np.abs(np.diff(lum, axis=axis))
        both = water[1:, :] & water[:-1, :] if axis == 0 else water[:, 1:] & water[:, :-1]
        d = np.where(both, d, 0.0)
        prof = d.sum(axis=1 - axis) / (both.sum(axis=1 - axis) + 1)
        prof = prof - prof.mean()
        if prof.std() < 1e-6:
            continue
        ac = np.correlate(prof, prof, mode="full")[len(prof) - 1:]
        ac /= ac[0] + 1e-9
        score = max(score, float(ac[4:96].max()))
    return score


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--accept", action="store_true")
    ap.add_argument("--skip-render", action="store_true")
    ap.add_argument("--premise-only", action="store_true",
                    help="run only the pose-premise check and print the table")
    ap.add_argument("--check-pose", action="append", default=[],
                    metavar="NAME,LAT,LON,ALT,YAW,PITCH,PREMISE[,SEASON]",
                    help="scout an ad-hoc pose against a premise without "
                         "rendering it; repeatable. Use '-' for no premise. "
                         "SEASON defaults to the reel's 0.45.")
    args = ap.parse_args()

    viewer = Path(__file__).resolve().parent.parent
    reel = viewer / "interchange" / "reel"
    current = reel / "current"
    accepted = reel / "accepted"
    run_dir = viewer / "interchange" / "runs" / "world-reel"
    play = viewer / "scripts" / "world-reel.play"

    # --check-pose: the scouting path. Answers "does this candidate pose
    # contain the class I want?" in ~1 s, BEFORE it is added to the table
    # and rendered. That ordering is the whole lesson of B-20.
    if args.check_pose:
        poses = []
        for spec in args.check_pose:
            f = [s.strip() for s in spec.split(",", 7)]
            if len(f) not in (7, 8):
                raise SystemExit("--check-pose needs "
                                 "NAME,LAT,LON,ALT,YAW,PITCH,PREMISE[,SEASON]")
            poses.append((f[0], float(f[1]), float(f[2]), float(f[3]),
                          float(f[4]), float(f[5]),
                          None if f[6] == "-" else f[6],
                          float(f[7]) if len(f) == 8 else 0.45))
        poses.sort(key=lambda p: p[7])   # one `weather season` line per group
        findings, _ = check_premises(viewer, poses, verbose=True)
        if findings:
            report_premise_failures(findings)
            sys.exit(3)
        print(f"\nPREMISE: all {len(poses)} candidate pose(s) contain their class")
        return

    # The premise check runs FIRST and it has no opt-out. A reel whose
    # poses are not pointed at their classes is not measuring anything, so
    # comparing its pixels would only manufacture confidence.
    if not args.accept:
        findings, measures = check_premises(
            viewer, PREMISE_POSES, verbose=args.premise_only)
        reel.mkdir(parents=True, exist_ok=True)
        (reel / "premise.json").write_text(
            json.dumps(measures, indent=1), encoding="utf-8")
        if findings:
            report_premise_failures(findings)
            sys.exit(3)
        checked = sum(1 for p in PREMISE_POSES if p[6])
        print(f"PREMISE: {checked}/{len(ALL_POSES)} poses proved their own "
              f"class; {len(ALL_POSES) - checked} carry no probe-expressible "
              f"premise (see the table in world_reel.py)")
        if args.premise_only:
            return

    if not args.skip_render:
        build_play(play)
        exe = viewer / "target" / "release" / "examples" / "play.exe"
        r = subprocess.run([str(exe), str(play)], cwd=viewer, capture_output=True, text=True)
        if r.returncode != 0:
            sys.stderr.write(r.stdout[-1500:] + r.stderr[-1500:])
            raise SystemExit(f"reel render failed ({r.returncode})")
        current.mkdir(parents=True, exist_ok=True)
        for name, *_ in ALL_POSES:
            shutil.copyfile(run_dir / f"{name}.png", current / f"{name}.png")

    findings = []
    report = {}
    for name, *_ in ALL_POSES:
        cur_p = current / f"{name}.png"
        if not cur_p.exists():
            findings.append(f"{name}: MISSING current frame")
            continue
        im = np.asarray(Image.open(cur_p).convert("RGB"), dtype=np.float64)
        if name.startswith("moon_"):
            # Space is legitimately black around a moon pose, so Neisor's
            # below-horizon void detector is the wrong invariant. Require a
            # substantial, non-flat generated surface instead.
            lum = im.max(axis=2)
            surface = lum > 45
            surface_frac = float(surface.mean())
            surface_std = float(im[surface].std()) if surface.any() else 0.0
            row = {
                "moon_surface_frac": round(surface_frac, 4),
                "moon_surface_std": round(surface_std, 3),
                "mean_delta": "no-baseline",
            }
            if surface_frac < 0.08:
                findings.append(f"{name}: MOON lint - generated surface missing")
            if surface_std < 5.0:
                findings.append(f"{name}: MOON lint - surface is visually flat")
        else:
            blob, dark_frac = lint_void(np.asarray(Image.open(cur_p).convert("RGB")))
            grid = lint_grid(im)
            row = {"void_blob_px": blob, "dark_frac": round(dark_frac, 5),
                   "grid_score": round(grid, 3)}
            if blob > 400:
                findings.append(f"{name}: VOID lint - {blob}px near-black blob")
        # grid score is INFORMATIONAL for now: healthy scenes carry designed
        # periodicity (block lattice edges, the frozen-ice checker) and the
        # cheap autocorr floor sits too close to the artifact band. The
        # accepted-baseline diff below is the real net for that class;
        # revisit calibration when a live specimen exists again.
        acc_p = accepted / f"{name}.png"
        if acc_p.exists():
            old = np.asarray(Image.open(acc_p).convert("RGB"), dtype=np.float64)
            if old.shape == im.shape:
                d = np.abs(old - im).max(axis=2)
                row["mean_delta"] = round(float(d.mean()), 3)
                row["changed_frac"] = round(float((d > 6).mean()), 4)
                if row["mean_delta"] > CHANGE_MEAN or row["changed_frac"] > CHANGE_FRAC:
                    findings.append(
                        f"{name}: CHANGED vs accepted (mean {row['mean_delta']}, "
                        f"frac {row['changed_frac']})"
                    )
            else:
                row["mean_delta"] = "size-mismatch"
        else:
            row["mean_delta"] = "no-baseline"
        report[name] = row

    (reel / "report.json").parent.mkdir(parents=True, exist_ok=True)
    (reel / "report.json").write_text(json.dumps(report, indent=1), encoding="utf-8")

    if args.accept:
        accepted.mkdir(parents=True, exist_ok=True)
        for name, *_ in ALL_POSES:
            src = current / f"{name}.png"
            if src.exists():
                shutil.copyfile(src, accepted / f"{name}.png")
        print(f"accepted: {len(ALL_POSES)} poses blessed as the reference reel")
        return

    if findings:
        print(f"REEL: {len(findings)} finding(s) to review:")
        for f in findings:
            print(" ", f)
        sys.exit(3)
    print(f"REEL: all {len(ALL_POSES)} poses clean (lint ok, no unexplained change)")


if __name__ == "__main__":
    main()
