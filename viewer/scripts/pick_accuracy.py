#!/usr/bin/env python3
"""Measure `pick`'s accuracy instead of asserting it.

`pick PX PY` (play.rs) answers "what ground point is under this screen pixel".
A pick that is silently 200 m off is worse than no pick at all, so this is the
instrument that says how wrong it is. It runs three independent checks, each
with its own ground truth, because each isolates a different way `pick` could
be wrong:

  A. AIM ROUND-TRIP — absolute ground truth.
     Place the camera at a known point, solve the exact 3D yaw/pitch that looks
     at a known target, `pick` the centre pixel, and measure the great-circle
     distance from the returned coordinates to the target. Everything is in
     play: the projection, the ray, the marcher, the surface model. This is the
     headline number and it is measured in metres.

  B. RAY FIDELITY — isolates the projection inversion.
     play.rs builds the pixel ray by inverting the renderer's own
     view-projection matrix. This script rebuilds it the other way, by hand,
     from camera.rs's algebra (the derivation V-2 already committed in
     altitude_ladder.py). Step the Python ray out by the distance `pick`
     reported and compare where it lands with the coordinates `pick` returned.
     Pure projection error: no terrain model on either side.
     (An earlier version of this check reconstructed the hit as
     `radius = R + column top` and saw up to 0.7 m of "error". That was the
     check being wrong, not `pick`: when a ray strikes the SIDE of a column
     the intersection sits below that column's top by up to one block, and
     every outlier was exactly `1 block * cos(elevation)`. `pick` now prints
     `z=`, the intersection's own height, so the two are distinguishable.)

  C. ON-SURFACE — isolates the marcher.
     Teleport to each returned coordinate and ask the harness what
     `surface_height_km` is there. A point that is on the camera ray (B) AND on
     the surface (C) is the intersection; the only thing left to doubt is
     whether it is the FIRST one, which is what the rendered frames in check
     A's run directory are for. Tolerance is one column STEP, read from the
     harness's own `column_step_max`: a hit on a column boundary can re-probe
     into either neighbour, and at a cliff those differ by metres.

  D. VISUAL AGREEMENT — the done-criterion in the mission's own words.
     Read the rendered frame at each picked pixel and classify it water/land
     by colour (the calibrated mask from altitude_ladder.py), then compare
     with `pick`'s own water/terrain verdict. Colour is a heuristic, so this
     reports an agreement RATE and writes an annotated frame rather than
     gating: a shore-dithered or sun-glinted pixel is genuinely ambiguous.

Usage (from viewer/):
  python scripts/pick_accuracy.py                 full run
  python scripts/pick_accuracy.py --only lake_shore
  python scripts/pick_accuracy.py --no-shots      skip the settled captures
Exit codes: 0 clean, 3 = findings to review (same convention as the reels).
"""

import argparse
import json
import math
import re
import subprocess
import sys
from pathlib import Path

import numpy as np

R_KM = 8660.254037844386  # Neisor, from assets/meta.json
FOV_Y_DEG = 65.0  # camera.rs: perspective_infinite_reverse_rh(65 deg, ...)
PITCH_LIMIT_DEG = 85.94  # camera pitch is clamped to +-1.50 rad
DEPRESSION_DEG = 55.0
RUNGS_KM = [0.05, 0.2, 1.0, 5.0, 20.0, 100.0]
SIZE = (1280, 720)

# name, target lat, target lon, view bearing (deg, 0 = N), note.
# Reused from the world reel / altitude ladder site tables so the coordinates
# are ones this project already trusts.
SITES = [
    ("lake_shore", -0.79691, -67.94009, 40.0, "forest pond field; world reel pond_country aim point"),
    ("river_bank", 15.02469, 17.65934, -75.0, "world reel river_wide aim point"),
    ("snowline", -12.30741, 107.18103, 20.0, "tropical 3.2 km massif - steep relief, the marcher's stress case"),
    ("desert", 0.870, -89.800, -35.0, "world reel desert pose"),
]

# fractional frame positions to pick. The centre one is check A; all of them
# feed checks B and C.
GRID = [(fx, fy) for fy in (0.25, 0.5, 0.75) for fx in (0.1, 0.3, 0.5, 0.7, 0.9)]

# Gates. Thresholds for "report a finding", not claims of accuracy — the
# numbers the run prints are the result.
# `pick` prints lat/lon to 7 decimal degrees = 1.5 cm on Neisor, so check B
# can never resolve better than ~2 cm; the gate is well clear of that and is
# expressed in metres rather than arcseconds so it does not tighten with
# distance for no reason.
RAY_LATERAL_M_GATE = 0.10  # projection inversion, lateral at the hit
SURFACE_M_GATE = 1.5       # on-surface: one voxel of slack
AIM_VS_BOUND_SLACK = 1.5   # aim miss may exceed pick's own error bound by this


# ---------------------------------------------------------------- geometry
# Deliberately re-derived by hand from camera.rs rather than shared with
# play.rs: a shared helper would agree with itself and prove nothing.


def unit(lat_deg, lon_deg):
    la, lo = math.radians(lat_deg), math.radians(lon_deg)
    return np.array([math.cos(la) * math.cos(lo), math.cos(la) * math.sin(lo), math.sin(la)])


def latlon_of(v):
    v = v / np.linalg.norm(v)
    return math.degrees(math.asin(np.clip(v[2], -1, 1))), math.degrees(math.atan2(v[1], v[0]))


def norm(v):
    return v / np.linalg.norm(v)


def local_frame(pos):
    """(radial up, north, east) exactly as camera.rs::frame builds it."""
    up = norm(pos)
    east = np.cross([0.0, 0.0, 1.0], up)
    n = np.linalg.norm(east)
    east = east / n if n > 1e-9 else np.array([1.0, 0.0, 0.0])
    north = norm(np.cross(up, east))
    return up, north, east


def offset_latlon(lat, lon, bearing_deg, dist_km):
    ang = dist_km / R_KM
    up, north, east = local_frame(unit(lat, lon))
    b = math.radians(bearing_deg)
    horiz = north * math.cos(b) + east * math.sin(b)
    return latlon_of(up * math.cos(ang) + horiz * math.sin(ang))


def aim(cam_pos, target_pos):
    """Exact (yaw, pitch) degrees to look from cam_pos at target_pos."""
    look = norm(target_pos - cam_pos)
    up, north, east = local_frame(cam_pos)
    pitch = math.degrees(math.asin(np.clip(np.dot(look, up), -1, 1)))
    horiz = look - up * np.dot(look, up)
    return math.degrees(math.atan2(np.dot(horiz, east), np.dot(horiz, north))), pitch


def view_basis(cam_pos, yaw_deg, pitch_deg, roll_deg):
    """camera.rs::view_basis, by hand."""
    up, north, east = local_frame(cam_pos)
    y, p = math.radians(yaw_deg), math.radians(pitch_deg)
    horiz = north * math.cos(y) + east * math.sin(y)
    look = norm(horiz * math.cos(p) + up * math.sin(p))
    right = np.cross(look, up)
    right = norm(right) if np.linalg.norm(right) > 1e-9 else east
    base_up = norm(np.cross(right, look))
    r = math.radians(roll_deg)
    view_up = norm(base_up * math.cos(r) + right * math.sin(r))
    view_right = norm(np.cross(look, view_up))
    return look, view_up, view_right


def pixel_ray(cam_pos, yaw, pitch, roll, px, py, w, h):
    """World ray through pixel (px,py). The reversed-Z / infinite far plane
    never appears: a pixel's DIRECTION is a function of fov, aspect and the
    view basis only. That is the whole reason this is invertible at all."""
    look, view_up, view_right = view_basis(cam_pos, yaw, pitch, roll)
    th = math.tan(math.radians(FOV_Y_DEG / 2.0))
    ndc_x = (px + 0.5) / w * 2.0 - 1.0
    ndc_y = 1.0 - (py + 0.5) / h * 2.0
    return norm(look + view_right * (ndc_x * th * (w / h)) + view_up * (ndc_y * th))


def great_circle_m(a_latlon, b_latlon):
    """Chord form, NOT `R*acos(dot)`. For the sub-metre separations this
    script measures, `acos` near 1 has a resolution floor of
    `sqrt(eps) * R` = 0.13 m — which is exactly the "error" an earlier version
    of this script reported for every single pose. The chord/asin form is
    accurate all the way down."""
    ua, ub = unit(*a_latlon), unit(*b_latlon)
    half_chord = float(np.linalg.norm(ua - ub)) / 2.0
    return 2.0 * R_KM * math.asin(min(1.0, half_chord)) * 1000.0


# ---------------------------------------------------------------- harness

PICK_RE = re.compile(
    r"pick (?P<px>[\d.]+) (?P<py>[\d.]+): hit=(?P<body>\w+)"
    r"(?: lat=(?P<lat>-?[\d.]+) lon=(?P<lon>-?[\d.]+) dist=(?P<dist>[\d.]+)km"
    r" elev=(?P<elev>-?[\d.]+)deg err=\+-(?P<err>\S+)"
    r" surface=(?P<surface>\w+) z=(?P<z>-?[\d.]+)m vis=(?P<vis>-?[\d.]+)m"
    r" solid=(?P<solid>-?[\d.]+)m koppen=(?P<koppen>\S+))?"
)


def parse_err_m(text):
    """`err=+-2.2m`, or `err=+-GRAZING-unbounded` when the ray is so shallow
    that a one-block surface ambiguity has no bounded lateral size."""
    try:
        return float(text.rstrip("m"))
    except ValueError:
        return float("inf")
ASSERT_RE = re.compile(r"assert (?:OK|FAIL): (?P<field>\w+) \S+ \S+ \(actual (?P<actual>-?[\d.eE+]+)\)")


def run_play(viewer, lines, name, size=None, allow_fail=False):
    script = viewer / "scripts" / f"_{name}.play"
    script.write_text("\n".join(lines) + "\n", encoding="utf-8")
    exe = viewer / "target" / "release" / "examples" / "play.exe"
    if not exe.exists():
        raise SystemExit(f"missing {exe} — cargo build --release --example play")
    cmd = [str(exe), str(script)] + (["--size", size] if size else [])
    r = subprocess.run(cmd, cwd=viewer, capture_output=True, text=True)
    if r.returncode != 0 and not allow_fail:
        sys.stderr.write(r.stdout[-4000:] + r.stderr[-4000:])
        raise SystemExit(f"play failed ({r.returncode}) on {script.name}")
    run_dir = viewer / "interchange" / "runs" / script.stem
    return run_dir, (run_dir / "run.log").read_text(encoding="utf-8"), r.returncode


def rung_tag(alt):
    return f"{alt:g}".replace(".", "p") + "km"


# ---------------------------------------------------------------- passes


def pass_truth(viewer, sites, rungs):
    """Render-free: the exact surface height at each target in the SAME model
    `pick` marches (voxel column top, or the open-water plane), and the exact
    ground_km the harness puts under each camera."""
    lines = ["# generated by pick_accuracy.py (pass 1: geometry truth)",
             "mode fly", "weather pin 0.10 0.0", "weather time 0"]
    for name, tlat, tlon, bearing, _ in sites:
        lines += [f"log TARGET {name}",
                  f"teleport {tlat:.6f} {tlon:.6f} 0.05",
                  f"state target_{name}",
                  "assert surface_height_km > -1000"]
        for alt in rungs:
            clat, clon, _d = rung_geometry(tlat, tlon, bearing, alt)
            lines += [f"teleport {clat:.6f} {clon:.6f} {alt}",
                      f"state cam_{name}_{rung_tag(alt)}"]
    run_dir, log, _ = run_play(viewer, lines, "d2-pick-truth")

    solid = {}
    cur = None
    for line in log.splitlines():
        if "# TARGET " in line:
            cur = line.split("# TARGET ")[1].strip()
        m = ASSERT_RE.search(line)
        if m and cur and m.group("field") == "surface_height_km":
            solid[cur] = float(m.group("actual"))
    truth = {}
    for name, tlat, tlon, _, _ in sites:
        sc = json.loads((run_dir / f"target_{name}.json").read_text())
        water = sc.get("water_surface_km")
        truth[name] = {
            "solid_km": solid[name],
            "water_km": water,
            # what a pixel at the target SHOWS: the water plane if the column
            # holds standing water, else the solid top
            "aim_km": water if water is not None and water > solid[name] else solid[name],
            "mesh_ground_km": sc["ground_km"],
            "cam_ground_km": {},
        }
        for alt in rungs:
            c = json.loads((run_dir / f"cam_{name}_{rung_tag(alt)}.json").read_text())
            truth[name]["cam_ground_km"][rung_tag(alt)] = c["radius_km"] - R_KM - c["alt_km"]
    return truth


def rung_geometry(tlat, tlon, bearing, alt_km):
    d = alt_km / math.tan(math.radians(DEPRESSION_DEG))
    clat, clon = offset_latlon(tlat, tlon, bearing + 180.0, d)
    return clat, clon, d


def pass_pick(viewer, sites, rungs, truth, shots):
    """Aim at each target and pick a grid of pixels."""
    w, h = SIZE
    lines = ["# generated by pick_accuracy.py (pass 2: aim + pick)",
             "mode fly", "weather pin 0.10 0.0", "weather time 0"]
    poses = {}
    for name, tlat, tlon, bearing, _ in sites:
        t = truth[name]
        target_pos = unit(tlat, tlon) * (R_KM + t["aim_km"])
        sun_lat = max(-23.0, min(23.0, tlat))
        for alt in rungs:
            tag = rung_tag(alt)
            clat, clon, d = rung_geometry(tlat, tlon, bearing, alt)
            cam_pos = unit(clat, clon) * (R_KM + t["cam_ground_km"][tag] + alt)
            yaw, pitch = aim(cam_pos, target_pos)
            pitch = max(-PITCH_LIMIT_DEG, min(PITCH_LIMIT_DEG, pitch))
            key = f"{name}_{tag}"
            poses[key] = {"site": name, "alt_km": alt, "cam_lat": clat, "cam_lon": clon,
                          "offset_km": d, "yaw": yaw, "pitch": pitch,
                          "target": [tlat, tlon], "aim_km": t["aim_km"]}
            lines += [f"log POSE {key}",
                      f"teleport {clat:.6f} {clon:.6f} {alt}",
                      f"look {yaw:.6f} {pitch:.6f}",
                      f"sun {sun_lat:.2f} {tlon - 15.0:.2f}",
                      f"state {key}"]
            if shots:
                lines.append(f"shot {key}")
            # centre first: that is the aim round-trip
            lines.append(f"pick {w / 2 - 0.5:g} {h / 2 - 0.5:g}")
            for fx, fy in GRID:
                lines.append(f"pick {fx * w:g} {fy * h:g}")
    run_dir, log, _ = run_play(viewer, lines, "d2-pick-grid", f"{w}x{h}")

    picks = {}
    cur = None
    for line in log.splitlines():
        if "# POSE " in line:
            cur = line.split("# POSE ")[1].strip()
            picks[cur] = []
            continue
        m = PICK_RE.search(line)
        if m and cur:
            g = m.groupdict()
            rec = {"px": float(g["px"]), "py": float(g["py"]), "body": g["body"]}
            if g["lat"] is not None:
                rec.update(lat=float(g["lat"]), lon=float(g["lon"]),
                           dist_km=float(g["dist"]), elev_deg=float(g["elev"]),
                           err_m=parse_err_m(g["err"]), surface=g["surface"],
                           z_m=float(g["z"]), vis_m=float(g["vis"]),
                           solid_m=float(g["solid"]), koppen=g["koppen"])
            picks[cur].append(rec)
    return run_dir, poses, picks


def pass_onsurface(viewer, poses, picks):
    """Teleport to every returned coordinate and ask the harness what the
    surface height is there. A mismatch means the marcher stopped somewhere
    that is not the surface."""
    lines = ["# generated by pick_accuracy.py (pass 3: on-surface re-check)",
             "mode fly", "weather pin 0.10 0.0", "weather time 0"]
    index = []
    for key, recs in picks.items():
        for i, r in enumerate(recs):
            if "lat" not in r or r["body"] != "neisor":
                continue
            lines += [f"teleport {r['lat']:.7f} {r['lon']:.7f} 0.05",
                      "assert surface_height_km > -1000",
                      "assert column_step_max > -1"]
            index.append((key, i))
    _, log, _ = run_play(viewer, lines, "d2-pick-onsurface")
    heights = [float(m.group("actual")) for m in ASSERT_RE.finditer(log)
               if m.group("field") == "surface_height_km"]
    steps = [float(m.group("actual")) for m in ASSERT_RE.finditer(log)
             if m.group("field") == "column_step_max"]
    if len(heights) != len(index) or len(steps) != len(index):
        raise SystemExit(
            f"pass 3: expected {len(index)} of each assert, parsed "
            f"{len(heights)} heights / {len(steps)} steps")
    for (key, i), h, s in zip(index, heights, steps):
        picks[key][i]["reprobe_solid_m"] = h * 1000.0
        picks[key][i]["reprobe_step_blocks"] = s


# ---------------------------------------------------------------- visual


def pixel_is_water(patch):
    """altitude_ladder.py's calibrated water mask: blue-over-RED, not
    blue-over-green — shoal/turquoise water is (90,180,180) and a
    `b > g + 8` test misses it entirely."""
    r, g, b = [float(patch[..., i].mean()) for i in range(3)]
    return b > r + 20 and b > 60


def visual_check(run_dir, poses, picks, out_png):
    """Check D. Returns (agree, total, disagreements) and writes one annotated
    frame so a human can see the picked pixels rather than trust a number."""
    from PIL import Image, ImageDraw
    agree = total = 0
    bad = []
    annotated = None
    for key in poses:
        png = run_dir / f"{key}.png"
        if not png.exists():
            continue
        im = Image.open(png).convert("RGB")
        arr = np.asarray(im, dtype=np.float64)
        draw_on = key == "lake_shore_0p2km"
        if draw_on:
            annotated = im.copy()
            dr = ImageDraw.Draw(annotated)
        for r in picks.get(key, []):
            if "surface" not in r:
                continue
            x, y = int(round(r["px"])), int(round(r["py"]))
            patch = arr[max(0, y - 1):y + 2, max(0, x - 1):x + 2]
            wet = pixel_is_water(patch)
            said = r["surface"] == "water"
            total += 1
            if wet == said:
                agree += 1
            else:
                bad.append((key, r["px"], r["py"], r["surface"],
                            "blue" if wet else "not-blue"))
            if draw_on:
                col = (80, 200, 255) if said else (255, 190, 90)
                dr.line((x - 7, y, x - 2, y), fill=col, width=2)
                dr.line((x + 2, y, x + 7, y), fill=col, width=2)
                dr.line((x, y - 7, x, y - 2), fill=col, width=2)
                dr.line((x, y + 2, x, y + 7), fill=col, width=2)
                dr.text((x + 9, y - 6), r["surface"][0].upper(), fill=col)
    if annotated is not None:
        out_png.parent.mkdir(parents=True, exist_ok=True)
        annotated.save(out_png)
    return agree, total, bad


# ---------------------------------------------------------------- analysis


def analyse(run_dir, poses, picks):
    w, h = SIZE
    report, findings = {}, []
    for key, pose in poses.items():
        recs = picks.get(key, [])
        if not recs:
            findings.append(f"{key}: no pick lines parsed")
            continue
        sc = json.loads((run_dir / f"{key}.json").read_text())
        cam = np.array([sc["camera_x_km"], sc["camera_y_km"], sc["camera_z_km"]])
        rows = []
        for r in recs:
            row = dict(r)
            if "lat" in r and r["body"] == "neisor":
                # B: ray fidelity. Walk the INDEPENDENTLY computed ray out by
                # the distance pick reported and see where it lands. No terrain
                # model on either side of the comparison.
                ray = pixel_ray(cam, sc["yaw_deg"], sc["pitch_deg"], sc["roll_deg"],
                                r["px"], r["py"], w, h)
                pred = cam + ray * r["dist_km"]
                row["ray_lateral_m"] = round(
                    great_circle_m(latlon_of(pred), (r["lat"], r["lon"])), 4)
                row["ray_arcsec"] = round(
                    math.degrees(row["ray_lateral_m"] / 1000.0 / max(r["dist_km"], 1e-9))
                    * 3600.0, 3)
                # also confirm the reported height is on that ray: |pred| and
                # R + z must agree, which pins the hit in 3D, not just in
                # bearing
                row["ray_radial_m"] = round(
                    (float(np.linalg.norm(pred)) - (R_KM + r["z_m"] / 1000.0)) * 1000.0, 4)
                # C: on-surface, tolerant of a column step (a boundary hit can
                # re-probe into either neighbour)
                if "reprobe_solid_m" in r:
                    d = r["reprobe_solid_m"] - r["solid_m"]
                    row["surface_delta_m"] = round(d, 4)
                    row["surface_tol_m"] = round(
                        max(SURFACE_M_GATE, r.get("reprobe_step_blocks", 0.0) * 1.0 + 0.5), 3)
            rows.append(row)

        centre = rows[0]
        entry = {"pose": pose, "picks": rows}
        if "lat" in centre:
            entry["aim_miss_m"] = round(
                great_circle_m((centre["lat"], centre["lon"]), tuple(pose["target"])), 3)
            entry["pick_err_bound_m"] = centre["err_m"]
            if entry["aim_miss_m"] > centre["err_m"] * AIM_VS_BOUND_SLACK + 0.5:
                findings.append(
                    f"{key}: AIM MISS {entry['aim_miss_m']:.1f} m exceeds pick's own "
                    f"stated bound +-{centre['err_m']:.1f} m")
        else:
            findings.append(f"{key}: centre pixel did not hit ({centre['body']})")

        rayv = [r["ray_lateral_m"] for r in rows if "ray_lateral_m" in r]
        radv = [abs(r["ray_radial_m"]) for r in rows if "ray_radial_m" in r]
        arcs = [r["ray_arcsec"] for r in rows if "ray_arcsec" in r]
        surf = [abs(r["surface_delta_m"]) for r in rows if "surface_delta_m" in r]
        over = [r for r in rows if "surface_delta_m" in r
                and abs(r["surface_delta_m"]) > r["surface_tol_m"]]
        entry["ray_lateral_m_max"] = round(max(rayv), 4) if rayv else None
        entry["ray_radial_m_max"] = round(max(radv), 4) if radv else None
        entry["ray_arcsec_max"] = round(max(arcs), 3) if arcs else None
        entry["surface_delta_m_max"] = round(max(surf), 3) if surf else None
        entry["picks_hit"] = len(rayv)
        if rayv and max(rayv) > RAY_LATERAL_M_GATE:
            findings.append(
                f"{key}: RAY off by {max(rayv):.3f} m (gate {RAY_LATERAL_M_GATE} m)")
        if radv and max(radv) > RAY_LATERAL_M_GATE:
            findings.append(
                f"{key}: RAY RADIUS off by {max(radv):.3f} m (gate {RAY_LATERAL_M_GATE} m)")
        for r in over:
            findings.append(
                f"{key}: OFF-SURFACE at ({r['px']:g},{r['py']:g}) by "
                f"{r['surface_delta_m']:.2f} m, past its {r['surface_tol_m']:.2f} m "
                f"column-step tolerance")
        report[key] = entry
    return report, findings


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--only", default="", help="comma-separated site names")
    ap.add_argument("--no-shots", action="store_true")
    ap.add_argument("--out", default="", help="write report.json here")
    args = ap.parse_args()

    viewer = Path(__file__).resolve().parent.parent
    sites = [s for s in SITES if not args.only or s[0] in args.only.split(",")]
    if not sites:
        raise SystemExit(f"no sites match --only {args.only}")

    truth = pass_truth(viewer, sites, RUNGS_KM)
    run_dir, poses, picks = pass_pick(viewer, sites, RUNGS_KM, truth, not args.no_shots)
    pass_onsurface(viewer, poses, picks)
    report, findings = analyse(run_dir, poses, picks)

    visual = None
    if not args.no_shots:
        agree, total, bad = visual_check(
            run_dir, poses, picks,
            viewer / "interchange" / "pick-accuracy-annotated.png")
        visual = {"agree": agree, "total": total,
                  "rate": round(agree / total, 4) if total else None,
                  "disagreements": bad[:40]}

    out = Path(args.out) if args.out else viewer / "interchange" / "pick-accuracy.json"
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps({"truth": truth, "poses": report, "visual": visual},
                              indent=1), encoding="utf-8")

    aim = [e["aim_miss_m"] for e in report.values() if "aim_miss_m" in e]
    ray = [e["ray_lateral_m_max"] for e in report.values()
           if e.get("ray_lateral_m_max") is not None]
    surf = [e["surface_delta_m_max"] for e in report.values()
            if e.get("surface_delta_m_max") is not None]
    hits = sum(e["picks_hit"] for e in report.values())
    print(f"PICK ACCURACY  {len(report)} poses, {hits} picks analysed  -> {out}")
    if aim:
        print(f"  A aim round-trip miss       median {np.median(aim):8.3f} m  max {max(aim):8.3f} m")
    rad = [e["ray_radial_m_max"] for e in report.values()
           if e.get("ray_radial_m_max") is not None]
    if ray:
        print(f"  B ray vs independent calc   median {np.median(ray):8.4f} m  max {max(ray):8.4f} m")
    if rad:
        print(f"    (same, radial component)  median {np.median(rad):8.4f} m  max {max(rad):8.4f} m")
    if surf:
        print(f"  C picked point off surface  median {np.median(surf):8.3f} m  max {max(surf):8.3f} m")
    if visual and visual["total"]:
        print(f"  D water/terrain vs the pixel {visual['agree']}/{visual['total']} agree "
              f"({visual['rate'] * 100:.1f}%)")
        for d in visual["disagreements"][:8]:
            print(f"      pick said {d[3]:8s} at {d[0]} ({d[1]:g},{d[2]:g}); pixel is {d[4]}")
    nan = float("nan")
    for key in sorted(report):
        e = report[key]
        print(f"    {key:24s} aim {e.get('aim_miss_m', nan):8.3f} m "
              f"(bound +-{e.get('pick_err_bound_m', nan):7.2f} m)  "
              f"ray {e.get('ray_lateral_m_max', nan):7.4f} m  "
              f"surf {e.get('surface_delta_m_max', nan):7.3f} m")
    if findings:
        print(f"PICK: {len(findings)} finding(s) to review:")
        for f in findings:
            print(" ", f)
        sys.exit(3)
    print("PICK: clean")


if __name__ == "__main__":
    main()
