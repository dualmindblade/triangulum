#!/usr/bin/env python3
"""The altitude ladder: ONE ground feature, SEVEN altitudes, one contact sheet.

Andrew's standing art bar (B-17) is an altitude-sweep invariant --
"irregular boundaries must read as natural curves AT EVERY ALTITUDE" --
and nothing tested it that way. The world reel has 29 poses at scattered
altitudes, so "does this shoreline still look natural when I climb from
50 m to orbit?" was a memory test across a flight. This turns it into a
side-by-side.

THE FRAMING CONTRACT (the part that makes the comparison mean anything):
every rung of a site looks at the SAME ground point from the SAME
depression angle. The camera is placed on a great circle at
`d = alt / tan(DEPRESSION)` from the target, along the reverse of the
site's view bearing, and the exact yaw/pitch to the target is solved in
3D (so planet curvature and the terrain height at BOTH ends are handled,
not approximated). Only scale changes between rungs; the projected ground
plane is a similarity transform about frame centre. That is what makes
row B of the sheet -- and the NCC framing check -- legitimate.

Two harness passes:
  1. PROBE/STATE pass. `probe` gives the sampler truth (height + water
     surface) at each target; `state` gives the exact `ground_km` the
     harness will put under each camera. Both are render-free and cost
     milliseconds. Without them the low rungs aim tens of degrees off,
     because at 50 m altitude a 5 m error in either endpoint's terrain
     height is a 6 deg pitch error.
  2. RENDER pass. 7 shots per site, weather and sun pinned (sun derived
     from the TARGET, not the camera, so all rungs of a site share one
     lighting solution -- `sun LAT LON` is a GLOBAL direction).

Output (viewer/interchange/altitude-ladder/):
  ladder_<site>.png   contact sheet: row A full frames, row B native-pixel
                      centre crops, altitude increasing left to right
  report.json         per-rung geometry, framing check, staircase lint

STAIRCASE LINT: gates ONLY where it was calibrated -- rungs whose ground
sampling distance is <= 3 m/px (altitude <~1 km), where the voxel lattice is
still a usable reference scale. Above that the numbers are printed and
labelled "(informational)" and never fail the run. Four earlier formulations
did not discriminate at all; see `lint_staircase` and
docs/missions/V2-altitude-ladder-findings.md for the calibration table.

Usage (from viewer/):
  python scripts/altitude_ladder.py                render + sheets + lint
  python scripts/altitude_ladder.py --only lake_shore,river_bank
  python scripts/altitude_ladder.py --skip-render  re-sheet existing frames
  python scripts/altitude_ladder.py --list         print the site table
  python scripts/altitude_ladder.py --scout 20     re-snap targets to the
       class boundary the renderer actually draws at 20 km (site maintenance;
       run it coarse-to-fine, 20 -> 5 -> 1, and paste the result into SITES)
  python scripts/altitude_ladder.py --rungs 0.3,0.5,0.7,1,1.5,2,3 --tag=-bracket
       a finer ladder under its own name, for bracketing an artefact's onset
Exit codes: 0 clean, 3 = findings to review (same convention as the reels).
"""

import argparse
import json
import math
import subprocess
import sys
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw, ImageFont

# ---------------------------------------------------------------- constants

R_KM = 8660.254037844386  # Neisor, from assets/meta.json
FOV_Y_DEG = 65.0  # camera.rs: perspective_infinite_reverse_rh(65 deg, ...)
PITCH_LIMIT_DEG = 85.94  # camera pitch is clamped to +-1.50 rad
DEPRESSION_DEG = 55.0
# Half-decade-ish rungs from "standing on the bank" to low orbit. 0.05 km is
# the lowest altitude where a 55 deg depression still clears treetops; 300 km
# matches the world reel's orbital poses, above which cloud cover dominates.
RUNGS_KM = [0.05, 0.2, 1.0, 5.0, 20.0, 100.0, 300.0]

# name, class, target lat, target lon, view bearing (deg, 0=N), mask, note.
# Targets were chosen with the probe scout (see findings doc): every water
# target sits ON a land/water transition, verified by probing a grid around
# it, because a target in open water gives a featureless low rung.
SITES = [
    ("lake_shore", "irregular lake shore", -0.79691, -67.94009, 40.0, "water",
     "forest pond field; world reel `pond_country` aim point"),
    ("river_bank", "river bank", 15.02469, 17.65934, -75.0, "water",
     "world reel `river_wide` aim point; known staircase specimen"),
    ("ocean_coast", "ocean coastline", 15.10556, 150.92017, 146.0, "water",
     "shore of the `orbital_coast` archipelago, --scout snapped to the waterline"),
    ("sand_patch", "sand/grass patch edge", 12.19894, -44.82774, -90.0, "sand",
     "world reel `dossier_ponds` aim point; a NEAR-FIELD class - the patches "
     "are ~30 m, so only the bottom rungs resolve them"),
    ("snowline", "snow/rock line", -12.30741, 107.18103, 20.0, "snow",
     "tropical 3.2 km massif; the boundary is elevation-driven, not water"),
    ("ecotone", "biome ecotone", 10.76433, 81.17593, 0.0, "generic",
     "desert->rainforest koppen edge (98 -> 9386 mm/yr); a FAR-FIELD class - "
     "biome blending is 8 km wide, so only the top rungs resolve it"),
]


# ---------------------------------------------------------------- geometry


def unit(lat_deg: float, lon_deg: float) -> np.ndarray:
    la, lo = math.radians(lat_deg), math.radians(lon_deg)
    return np.array([math.cos(la) * math.cos(lo), math.cos(la) * math.sin(lo), math.sin(la)])


def latlon_of(v: np.ndarray) -> tuple[float, float]:
    v = v / np.linalg.norm(v)
    return math.degrees(math.asin(np.clip(v[2], -1, 1))), math.degrees(math.atan2(v[1], v[0]))


def local_frame(pos: np.ndarray):
    """(radial up, north, east) exactly as camera.rs builds it."""
    up = pos / np.linalg.norm(pos)
    east = np.cross([0.0, 0.0, 1.0], up)
    n = np.linalg.norm(east)
    east = east / n if n > 1e-9 else np.array([1.0, 0.0, 0.0])
    north = np.cross(up, east)
    return up, north / np.linalg.norm(north), east


def offset_latlon(lat: float, lon: float, bearing_deg: float, dist_km: float):
    """Great-circle offset: walk `dist_km` from (lat,lon) on bearing."""
    ang = dist_km / R_KM
    up, north, east = local_frame(unit(lat, lon))
    b = math.radians(bearing_deg)
    horiz = north * math.cos(b) + east * math.sin(b)
    return latlon_of(up * math.cos(ang) + horiz * math.sin(ang))


def aim(cam_pos: np.ndarray, target_pos: np.ndarray) -> tuple[float, float]:
    """Exact (yaw, pitch) in degrees for a camera at cam_pos to look at
    target_pos, in the harness's convention (yaw 0 = north, +east; pitch 0 =
    horizon, negative = down)."""
    look = target_pos - cam_pos
    look = look / np.linalg.norm(look)
    up, north, east = local_frame(cam_pos)
    pitch = math.degrees(math.asin(np.clip(np.dot(look, up), -1, 1)))
    horiz = look - up * np.dot(look, up)
    yaw = math.degrees(math.atan2(np.dot(horiz, east), np.dot(horiz, north)))
    return yaw, pitch


def look_vector(cam_pos: np.ndarray, yaw_deg: float, pitch_deg: float) -> np.ndarray:
    up, north, east = local_frame(cam_pos)
    y, p = math.radians(yaw_deg), math.radians(pitch_deg)
    horiz = north * math.cos(y) + east * math.sin(y)
    v = horiz * math.cos(p) + up * math.sin(p)
    return v / np.linalg.norm(v)


def ray_sphere(cam_pos: np.ndarray, look: np.ndarray, radius: float):
    """Nearest positive hit of the ray with the sphere of `radius`."""
    b = float(np.dot(cam_pos, look))
    c = float(np.dot(cam_pos, cam_pos)) - radius * radius
    disc = b * b - c
    if disc < 0:
        return None
    s = math.sqrt(disc)
    for t in (-b - s, -b + s):
        if t > 0:
            return cam_pos + look * t
    return None


def great_circle_km(a: np.ndarray, b: np.ndarray) -> float:
    ua, ub = a / np.linalg.norm(a), b / np.linalg.norm(b)
    return R_KM * math.acos(float(np.clip(np.dot(ua, ub), -1, 1)))


def pixel_to_ground(sidecar: dict, px: float, py: float, w: int, h: int, radius_km: float):
    """Invert the harness's projection for one pixel: which ground point is
    under it. Rebuilds camera.rs's view basis (roll 0) and the 65 deg
    infinite-reverse-RH frustum, then hits the sphere at `radius_km`."""
    cam = unit(sidecar["lat_deg"], sidecar["lon_deg"]) * sidecar["radius_km"]
    look = look_vector(cam, sidecar["yaw_deg"], sidecar["pitch_deg"])
    radial_up, _, east = local_frame(cam)
    right = np.cross(look, radial_up)
    n = np.linalg.norm(right)
    right = right / n if n > 1e-9 else east
    view_up = np.cross(right, look)
    th_y = math.tan(math.radians(FOV_Y_DEG / 2.0))
    th_x = th_y * (w / h)
    ndc_x = (px + 0.5) / w * 2.0 - 1.0
    ndc_y = 1.0 - (py + 0.5) / h * 2.0
    d = look + right * (ndc_x * th_x) + view_up * (ndc_y * th_y)
    return ray_sphere(cam, d / np.linalg.norm(d), radius_km)


def rung_geometry(site, alt_km: float):
    """Camera lat/lon for one rung. Depends only on the ladder geometry, so
    it is known before any harness pass (the probe pass needs it)."""
    _, _, tlat, tlon, bearing, _, _ = site
    d = alt_km / math.tan(math.radians(DEPRESSION_DEG))
    clat, clon = offset_latlon(tlat, tlon, bearing + 180.0, d)
    return clat, clon, d


def frame_scale(alt_km: float, height_px: int):
    """Ground sampling distance at frame centre, and the ground width the
    frame spans there. Both scale linearly in altitude because DEPRESSION is
    held constant, which is exactly what makes the rungs comparable."""
    slant = alt_km / math.sin(math.radians(DEPRESSION_DEG))
    rad_per_px = 2.0 * math.tan(math.radians(FOV_Y_DEG / 2.0)) / height_px
    cross_km = slant * rad_per_px  # across-range: unforeshortened
    down_km = cross_km / math.sin(math.radians(DEPRESSION_DEG))
    return slant, cross_km, down_km


# ---------------------------------------------------------------- harness


def run_play(viewer: Path, script: Path, size: str | None = None) -> Path:
    exe = viewer / "target" / "release" / "examples" / "play.exe"
    if not exe.exists():
        raise SystemExit(f"missing prebuilt harness {exe} (do not rebuild it here)")
    cmd = [str(exe), str(script)] + (["--size", size] if size else [])
    r = subprocess.run(cmd, cwd=viewer, capture_output=True, text=True)
    if r.returncode != 0:
        sys.stderr.write(r.stdout[-2000:] + r.stderr[-2000:])
        raise SystemExit(f"play failed ({r.returncode}) on {script.name}")
    return viewer / "interchange" / "runs" / script.stem


def probe_pass(viewer: Path, sites, rungs) -> dict:
    """Pass 1: sampler truth at every target, and the exact `ground_km` the
    harness puts under every camera. Render-free."""
    import re

    lines = ["# generated by altitude_ladder.py (pass 1: geometry truth)",
             "mode fly", "weather pin 0.10 0.0", "weather time 0", "weather season 0.45"]
    keys = []
    for site in sites:
        name, _, tlat, tlon, _, _, _ = site
        lines.append(f"probe {tlat:.6f} {tlon:.6f}")
        for alt in rungs:
            clat, clon, _ = rung_geometry(site, alt)
            lines.append(f"teleport {clat:.6f} {clon:.6f} {alt}")
            lines.append(f"state {name}_{rung_tag(alt)}")
            keys.append((name, alt))
    script = viewer / "scripts" / "_altitude-ladder-probe.play"
    script.write_text("\n".join(lines) + "\n", encoding="utf-8")
    run_dir = run_play(viewer, script)

    log = (run_dir / "run.log").read_text(encoding="utf-8")
    pat = re.compile(r"probe (-?[\d.]+) (-?[\d.]+): h=(-?[\d.]+)m water=(\S+)")
    probes = pat.findall(log)
    if len(probes) != len(sites):
        raise SystemExit(f"probe pass: expected {len(sites)} probes, parsed {len(probes)}")

    truth = {}
    for site, (_, _, h_m, water) in zip(sites, probes):
        name = site[0]
        h_km = float(h_m) / 1000.0
        wet = water != "-"
        aim_km = float(water.rstrip("m")) / 1000.0 if wet else h_km
        truth[name] = {"target_h_km": h_km, "target_water": wet, "aim_km": aim_km,
                       "cam_ground_km": {}}
    for name, alt in keys:
        sc = json.loads((run_dir / f"{name}_{rung_tag(alt)}.json").read_text())
        truth[name]["cam_ground_km"][rung_tag(alt)] = sc["radius_km"] - R_KM - sc["alt_km"]
    return truth


def rung_tag(alt_km: float) -> str:
    return f"{alt_km:g}".replace(".", "p") + "km"


def render_pass(viewer: Path, sites, rungs, truth, size, tag: str = "") -> Path:
    lines = ["# generated by altitude_ladder.py (pass 2: the ladder)",
             "mode fly", "weather pin 0.10 0.0", "weather time 0", "weather season 0.45"]
    poses = {}
    for site in sites:
        name, _, tlat, tlon, _, _, _ = site
        t = truth[name]
        target_pos = unit(tlat, tlon) * (R_KM + t["aim_km"])
        # one lighting solution per site, derived from the TARGET so every
        # rung of a site is lit identically (`sun` is a GLOBAL direction).
        sun_lat = max(-23.0, min(23.0, tlat))
        sun_lon = tlon - 15.0
        for alt in rungs:
            rtag = rung_tag(alt)
            clat, clon, d = rung_geometry(site, alt)
            cam_pos = unit(clat, clon) * (R_KM + t["cam_ground_km"][rtag] + alt)
            yaw, pitch = aim(cam_pos, target_pos)
            pitch = max(-PITCH_LIMIT_DEG, min(PITCH_LIMIT_DEG, pitch))
            poses[(name, alt)] = {"cam_lat": clat, "cam_lon": clon, "alt_km": alt,
                                  "offset_km": d, "yaw": yaw, "pitch": pitch,
                                  "cam_ground_km": t["cam_ground_km"][rtag]}
            lines += [
                f"teleport {clat:.6f} {clon:.6f} {alt}",
                f"look {yaw:.5f} {pitch:.5f}",
                f"sun {sun_lat:.2f} {sun_lon:.2f}",
                f"shot {name}_{rtag}",
            ]
    script = viewer / "scripts" / f"_altitude-ladder{tag}.play"
    script.write_text("\n".join(lines) + "\n", encoding="utf-8")
    run_dir = run_play(viewer, script, size)
    return run_dir, poses


# ---------------------------------------------------------------- masks


def class_mask(im: np.ndarray, kind: str) -> np.ndarray:
    """Binary mask of the material class whose boundary we are judging."""
    r, g, b = im[..., 0], im[..., 1], im[..., 2]
    if kind == "water":
        # blue-over-RED, not blue-over-green: shoal/turquoise water is
        # (90,180,180) and the world reel's `b > g + 8` misses it entirely,
        # which silently turned the whole ocean_coast ladder into
        # "no-usable-boundary".
        return (b > r + 20) & (b > 60)
    if kind == "snow":
        lum = im @ np.array([0.2126, 0.7152, 0.0722])
        return (lum > 190) & (im.max(axis=2) - im.min(axis=2) < 40)
    if kind == "sand":
        return (r > b + 30) & (r > 120) & (g > b)
    # generic: two-means on chroma, the largest colour split in the frame
    lum = im @ np.array([0.2126, 0.7152, 0.0722])
    thr = np.percentile(lum, 50)
    lo, hi = lum[lum <= thr].mean(), lum[lum > thr].mean()
    for _ in range(12):
        mid = 0.5 * (lo + hi)
        a, c = lum <= mid, lum > mid
        if not a.any() or not c.any():
            break
        lo, hi = lum[a].mean(), lum[c].mean()
    return lum > 0.5 * (lo + hi)


# ---------------------------------------------------------------- lint


VOXEL_M = 1.35  # gnomonic-anisotropic voxels are ~1.7 x 1.0 m at face centres
JUMPINESS_GATE = 6.0
STEP_PX_GATE = 7.0
STEP_VOXEL_GATE = 8.0
# Above this ground sampling distance the renderer legitimately runs coarse
# LOD and there is no in-frame reference scale left to judge "coarser than it
# should be" against, so the lint reports but never gates. ~2 m/px is the
# scale at which a 1.35 m voxel is still about a pixel.
LINT_GATE_MAX_GSD_M = 3.0


def boundary_profiles(mask: np.ndarray, margin: int = 3):
    """Every single-valued boundary profile of the mask's largest component,
    scanned both by row and by column, both sides. Yields (values, axis);
    axis 0 means the lateral displacement between samples is horizontal."""
    from scipy import ndimage

    lab, n = ndimage.label(mask)
    if n == 0:
        return []
    sizes = ndimage.sum(mask, lab, range(1, n + 1))
    comp = lab == (int(np.argmax(sizes)) + 1)
    out = []
    for axis, m in ((0, comp), (1, np.ascontiguousarray(comp.T))):
        h, w = m.shape
        for side in (0, 1):
            runs, vals = [], []
            for y in range(h):
                idx = np.flatnonzero(m[y])
                edge = int(idx[0] if side == 0 else idx[-1]) if idx.size else -1
                if idx.size >= 8 and margin < edge < w - 1 - margin and margin < y < h - 1 - margin:
                    vals.append(edge)
                else:
                    if len(vals) >= 60:
                        runs.append(np.array(vals))
                    vals = []
            if len(vals) >= 60:
                runs.append(np.array(vals))
            out += [(v, axis) for v in runs]
    return out


def lint_staircase(mask: np.ndarray, cross_km: float, down_km: float) -> dict:
    """Staircase detector. Four earlier formulations failed to discriminate
    (see the findings doc); this one works on the observation that made them
    fail. Scan the dominant class boundary line by line and difference it. A
    natural curve's lateral step is unimodal about its own local slope -- 0s
    and 1s for a gentle boundary, 3s and 4s for a steep one. A boundary
    quantised to a lattice is FLAT-THEN-JUMP: dozens of exact zeros, then a
    single lateral jump of the lattice pitch. So:

      jumpiness = p98(|d|) / mean(|d|)   how far the tail sits from the trend
      step_px   = median jump among |d| >= max(3, 3*mean|d|)

    A staircase needs BOTH: a big isolated jump (jumpiness) that is also
    physically large (step_px). Honest voxel stepping at 50 m altitude has a
    large jumpiness but a step_px near the voxel's own pixel size, so the
    pair separates it from LOD quantisation. Above LINT_GATE_MAX_GSD_M the
    numbers are still reported but never gate -- see the findings doc."""
    profs = boundary_profiles(mask)
    if not profs:
        return {"status": "no-usable-boundary"}
    tot = 0.0
    acc = {"jumpiness": 0.0, "step_px": 0.0, "step_m": 0.0, "slope_px": 0.0,
           "jump_frac": 0.0}
    used = 0
    for v, axis in profs:
        d = np.diff(v.astype(float))
        slope = float(np.abs(d).mean())
        if slope > 3.0:  # boundary nearly parallel to this scan axis: skip
            continue
        p98 = float(np.percentile(np.abs(d), 98))
        thr = max(3.0, 3.0 * slope)
        big = np.abs(d)[np.abs(d) >= thr]
        step_px = float(np.median(big)) if big.size >= 3 else 0.0
        lateral_m = (cross_km if axis == 0 else down_km) * 1000.0
        w = len(v)
        tot += w
        used += 1
        acc["jumpiness"] += p98 / max(slope, 0.5) * w
        acc["step_px"] += step_px * w
        acc["step_m"] += step_px * lateral_m * w
        acc["slope_px"] += slope * w
        acc["jump_frac"] += float(big.sum() / (np.abs(d).sum() + 1e-9)) * w
    if tot == 0:
        return {"status": "boundary-parallel-to-scan"}
    r = {k: round(v / tot, 3) for k, v in acc.items()}
    r.update(status="ok", profiles=used, profile_px=int(tot),
             gsd_m=round(math.sqrt(cross_km * down_km) * 1000.0, 3))
    r["step_voxels"] = round(r["step_m"] / VOXEL_M, 2)
    r["gate_applies"] = r["gsd_m"] <= LINT_GATE_MAX_GSD_M
    # three conditions, all needed. jumpiness says "flat-then-jump"; step_px
    # says the jump is big enough to read on screen; step_voxels says the
    # lattice is COARSER THAN THE VOXELS, which is what separates an LOD
    # artefact from the honest blocky look at 50 m altitude.
    r["staircase"] = bool(r["gate_applies"]
                          and r["jumpiness"] >= JUMPINESS_GATE
                          and r["step_px"] >= STEP_PX_GATE
                          and r["step_voxels"] >= STEP_VOXEL_GATE)
    return r


# ---------------------------------------------------------------- framing


def framing_check(sidecar: dict, target_pos: np.ndarray, alt_km: float, height_px: int):
    """Cast the frame-centre ray from the pose the harness ACTUALLY stored and
    intersect it with the sphere at the target's height. The miss distance is
    reported in metres and as a fraction of the frame's ground half-width, so
    "did the rungs frame the same thing" has a number, not a vibe."""
    cam = unit(sidecar["lat_deg"], sidecar["lon_deg"]) * sidecar["radius_km"]
    look = look_vector(cam, sidecar["yaw_deg"], sidecar["pitch_deg"])
    hit = ray_sphere(cam, look, float(np.linalg.norm(target_pos)))
    if hit is None:
        return {"status": "centre-ray-misses-planet"}
    miss_km = great_circle_km(hit, target_pos)
    _, cross_km, down_km = frame_scale(alt_km, height_px)
    half_km = down_km * height_px / 2.0
    return {"status": "ok", "miss_m": round(miss_km * 1000.0, 2),
            "miss_frac_halfframe": round(miss_km / half_km, 6)}


def ncc_pair(low: np.ndarray, high: np.ndarray, ratio: float):
    """Independent image evidence that two adjacent rungs show the same
    ground. The higher rung covers `ratio` times more ground, so shrinking the
    LOWER frame by `ratio` makes the two directly comparable about frame
    centre. Returns the normalised-correlation peak and its pixel offset."""
    h, w = high.shape[:2]
    cw, ch = int(w / ratio), int(h / ratio)
    if cw < 40 or ch < 24:
        return None
    lo = np.asarray(Image.fromarray(low).resize((cw, ch), Image.Resampling.LANCZOS)
                    .convert("L"), dtype=np.float64)
    hi_crop = high[(h - ch) // 2:(h - ch) // 2 + ch, (w - cw) // 2:(w - cw) // 2 + cw]
    hi = np.asarray(Image.fromarray(hi_crop).convert("L"), dtype=np.float64)
    # search a +-15% window for the correlation peak
    m = max(4, int(0.15 * min(cw, ch)))
    inner_lo = lo[m:-m, m:-m]
    a = inner_lo - inner_lo.mean()
    if a.std() < 1e-6:
        return {"peak": None, "note": "flat-frame"}
    best = (-2.0, 0, 0)
    for dy in range(-m, m + 1, max(1, m // 8)):
        for dx in range(-m, m + 1, max(1, m // 8)):
            b = hi[m + dy:ch - m + dy, m + dx:cw - m + dx]
            if b.shape != a.shape:
                continue
            b = b - b.mean()
            den = math.sqrt(float((a * a).sum()) * float((b * b).sum())) + 1e-9
            r = float((a * b).sum()) / den
            if r > best[0]:
                best = (r, dy, dx)
    return {"peak": round(best[0], 4), "dy_px": best[1], "dx_px": best[2],
            "scale_px": cw, "offset_frac": round(math.hypot(best[1], best[2]) / (cw / 2.0), 4)}


# ---------------------------------------------------------------- sheet


def font(size: int):
    try:
        return ImageFont.load_default(size=size)
    except TypeError:  # very old Pillow
        return ImageFont.load_default()


def contact_sheet(site, rungs, frames, rows, out_path: Path):
    name, klass, tlat, tlon, bearing, _, note = site
    col_w, gap, pad = 400, 10, 20
    col_h = round(col_w * 9 / 16)
    crop_h = round(col_w * CROP_H / CROP_W)
    n = len(rungs)
    w = pad * 2 + n * col_w + (n - 1) * gap
    head, capA, capB, band = 86, 42, 22, 40
    h = pad + head + capA + col_h + band + capB + crop_h + pad
    sheet = Image.new("RGB", (w, h), (17, 19, 24))
    dr = ImageDraw.Draw(sheet)
    f_title, f_lab, f_small = font(26), font(18), font(14)

    dr.text((pad, pad), f"ALTITUDE LADDER   {name}   [{klass}]", font=f_title,
            fill=(238, 240, 245))
    dr.text((pad, pad + 32), f"target {tlat:.5f} {tlon:.5f}    view bearing {bearing:g} deg    "
                             f"depression {DEPRESSION_DEG:g} deg held constant on every rung"
                             f"    {note}", font=f_small, fill=(150, 158, 172))
    dr.text((pad, pad + 52), "row A: full frame, crosshair marks the aim point (identical "
                             "ground point on every rung).   row B: native-pixel centre crop.",
            font=f_small, fill=(120, 200, 250))

    y_capA = pad + head
    y_rowA = y_capA + capA
    y_band = y_rowA + col_h
    y_capB = y_band + band
    y_rowB = y_capB + capB

    for i, alt in enumerate(rungs):
        x = pad + i * (col_w + gap)
        row = rows[i]
        alt_txt = f"{alt * 1000:.0f} m" if alt < 1 else f"{alt:g} km"
        dr.text((x, y_capA), alt_txt, font=f_lab, fill=(255, 214, 120))
        span_m = row["frame_ground_width_km"] * 1000.0
        span = f"{span_m:.0f} m" if span_m < 2000 else f"{span_m / 1000.0:.1f} km"
        dr.text((x, y_capA + 22), f"frame spans {span}    {row['gsd_m']:.2f} m/px",
                font=f_small, fill=(150, 158, 172))
        sheet.paste(frames[i].resize((col_w, col_h), Image.Resampling.LANCZOS), (x, y_rowA))
        cx, cy = x + col_w // 2, y_rowA + col_h // 2
        for a in ((-12, 0, -5, 0), (5, 0, 12, 0), (0, -12, 0, -5), (0, 5, 0, 12)):
            dr.line((cx + a[0], cy + a[1], cx + a[2], cy + a[3]), fill=(255, 70, 70), width=2)

        fr, lt = row.get("framing", {}), row.get("staircase", {})
        ncc = row.get("ncc_to_next") or {}
        bits = [f"aim miss {fr.get('miss_m', '?')} m"]
        if ncc.get("peak") is not None:
            bits.append(f"next-rung NCC {ncc['peak']:.2f} @ {ncc['offset_frac'] * 100:.0f}%")
        dr.text((x, y_band + 8), "   ".join(bits), font=f_small, fill=(150, 158, 172))
        if lt.get("status") == "ok":
            step = (f"{lt['step_m']:.0f} m" if lt["step_m"] < 1000
                    else f"{lt['step_m'] / 1000:.1f} km")
            c2 = (f"jump {lt['jumpiness']:.1f}  step {lt['step_px']:.1f}px={step}"
                  + ("" if lt["gate_applies"] else "  (informational)"))
        else:
            c2 = f"lint: {lt.get('status', '-')}"
        dr.text((x, y_capB), c2, font=f_small,
                fill=(255, 150, 120) if row.get("flag") else (150, 158, 172))
        sheet.paste(row["crop"].resize((col_w, crop_h), Image.Resampling.NEAREST), (x, y_rowB))
        if row.get("flag"):
            dr.rectangle((x - 2, y_rowB - 2, x + col_w + 1, y_rowB + crop_h + 1),
                         outline=(255, 110, 80), width=3)
            dr.text((x + 6, y_rowB + 4), "STAIRCASE", font=f_lab, fill=(255, 110, 80))

    out_path.parent.mkdir(parents=True, exist_ok=True)
    sheet.save(out_path)


CROP_W, CROP_H = 512, 288


# ---------------------------------------------------------------- scout


def scout(viewer: Path, sites, alt_km: float, size: str):
    """Site-table maintenance. A target that is 500 m off the waterline gives
    a featureless bottom rung, and probe grids only resolve to their step, so
    snap each target to the class boundary the RENDERER actually draws:
    render one rung, find the boundary pixel nearest frame centre, and invert
    the projection back to lat/lon."""
    # its own run name: scouting must never overwrite the ladder's frames,
    # or a later --skip-render would re-sheet a single-rung run.
    truth = probe_pass(viewer, sites, [alt_km])
    run_dir, _ = render_pass(viewer, sites, [alt_km], truth, size, "-scout")
    print(f"# scouted at {alt_km} km -- paste into SITES")
    for site in sites:
        name, _, tlat, tlon, bearing, kind, _ = site
        png = run_dir / f"{name}_{rung_tag(alt_km)}.png"
        im = Image.open(png).convert("RGB")
        arr = np.asarray(im, dtype=np.float64)
        m = class_mask(arr, kind)
        # only the largest blob of each side counts: thresholding shaded snow
        # or hazy water throws off speckle everywhere, and snapping a target
        # to a 3-pixel speck moves it somewhere with no boundary at all.
        from scipy import ndimage
        keep = np.zeros_like(m)
        for side in (m, ~m):
            lab, n = ndimage.label(side)
            if n:
                sizes = ndimage.sum(side, lab, range(1, n + 1))
                keep |= lab == (int(np.argmax(sizes)) + 1)
        m = m & keep
        edge = np.zeros_like(m)
        edge[:-1, :] |= (m[:-1, :] != m[1:, :]) & keep[:-1, :] & keep[1:, :]
        edge[:, :-1] |= (m[:, :-1] != m[:, 1:]) & keep[:, :-1] & keep[:, 1:]
        ys, xs = np.nonzero(edge)
        if ys.size == 0:
            print(f"#   {name}: no class boundary in frame at {alt_km} km "
                  f"(class_frac {m.mean():.3f})")
            continue
        cx, cy = im.width / 2, im.height / 2
        d2 = (xs - cx) ** 2 + (ys - cy) ** 2
        k = int(np.argmin(d2))
        sc = json.loads((run_dir / f"{name}_{rung_tag(alt_km)}.json").read_text())
        hit = pixel_to_ground(sc, float(xs[k]), float(ys[k]), im.width, im.height,
                              R_KM + truth[name]["aim_km"])
        if hit is None:
            print(f"#   {name}: boundary pixel ray misses the planet")
            continue
        la, lo = latlon_of(hit)
        off = great_circle_km(hit, unit(tlat, tlon)) * 1000.0
        print(f'    ("{name}", ..., {la:.5f}, {lo:.5f}, {bearing:g}, "{kind}", ...)'
              f"   # moved {off:.0f} m")


# ---------------------------------------------------------------- main


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--skip-render", action="store_true")
    ap.add_argument("--only", default="", help="comma-separated site names")
    ap.add_argument("--list", action="store_true")
    ap.add_argument("--scout", type=float, default=0.0, metavar="ALT_KM",
                    help="render one frame per site at ALT_KM, find the class "
                         "boundary nearest frame centre, and print a refined "
                         "target lat/lon to paste into SITES")
    ap.add_argument("--size", default="1280x720")
    ap.add_argument("--rungs", default="", help="override the altitude ladder, "
                    "comma-separated km (e.g. a fine bracket 0.3,0.5,0.7,1,1.5,2,3)")
    ap.add_argument("--tag", default="", help="suffix for the run/output names so a "
                    "bracket run does not clobber the standard sheets")
    args = ap.parse_args()

    global RUNGS_KM
    if args.rungs:
        RUNGS_KM = [float(v) for v in args.rungs.split(",")]

    if args.list:
        for s in SITES:
            print(f"{s[0]:12s} {s[1]:24s} {s[2]:>10.5f} {s[3]:>11.5f}  bearing {s[4]:>6g}  {s[6]}")
        return

    viewer = Path(__file__).resolve().parent.parent
    out = viewer / "interchange" / "altitude-ladder"
    out.mkdir(parents=True, exist_ok=True)
    sites = [s for s in SITES if not args.only or s[0] in args.only.split(",")]
    if not sites:
        raise SystemExit(f"no sites match --only {args.only}")
    if args.scout:
        scout(viewer, sites, args.scout, args.size)
        return

    run_dir = viewer / "interchange" / "runs" / f"_altitude-ladder{args.tag}"
    if args.skip_render:
        truth = json.loads((out / f"truth{args.tag}.json").read_text())
    else:
        truth = probe_pass(viewer, sites, RUNGS_KM)
        (out / f"truth{args.tag}.json").write_text(json.dumps(truth, indent=1), encoding="utf-8")
        run_dir, poses = render_pass(viewer, sites, RUNGS_KM, truth, args.size, args.tag)
        (out / f"poses{args.tag}.json").write_text(
            json.dumps({f"{k[0]}@{k[1]}": v for k, v in poses.items()}, indent=1),
            encoding="utf-8")

    report, findings = {}, []
    for site in sites:
        name, klass, tlat, tlon, bearing, mask_kind, note = site
        t = truth[name]
        target_pos = unit(tlat, tlon) * (R_KM + t["aim_km"])
        frames, rows, raws = [], [], []
        for alt in RUNGS_KM:
            tag = rung_tag(alt)
            png = run_dir / f"{name}_{tag}.png"
            if not png.exists():
                findings.append(f"{name}@{alt}km: MISSING frame")
                continue
            im = Image.open(png).convert("RGB")
            arr = np.asarray(im, dtype=np.float64)
            raws.append(np.asarray(im, dtype=np.uint8))
            slant, cross_km, down_km = frame_scale(alt, im.height)
            sc = json.loads((run_dir / f"{name}_{tag}.json").read_text())
            row = {
                "alt_km": alt, "slant_km": round(slant, 4),
                "cam_lat": sc["lat_deg"], "cam_lon": sc["lon_deg"],
                "yaw_deg": round(sc["yaw_deg"], 4), "pitch_deg": round(sc["pitch_deg"], 4),
                "gsd_m": round(math.sqrt(cross_km * down_km) * 1000.0, 3),
                "frame_ground_width_km": round(cross_km * im.width, 4),
                "framing": framing_check(sc, target_pos, alt, im.height),
            }
            m = class_mask(arr, mask_kind)
            row["class_frac"] = round(float(m.mean()), 4)
            row["staircase"] = lint_staircase(m, cross_km, down_km)
            row["flag"] = bool(row["staircase"].get("staircase"))
            cx, cy = im.width // 2, im.height // 2
            row["crop"] = im.crop((cx - CROP_W // 2, cy - CROP_H // 2,
                                   cx + CROP_W // 2, cy + CROP_H // 2))
            frames.append(im)
            rows.append(row)
        if len(rows) != len(RUNGS_KM):
            continue

        # adjacent-rung NCC: independent evidence the framing tracked
        for i in range(len(RUNGS_KM) - 1):
            ratio = RUNGS_KM[i + 1] / RUNGS_KM[i]
            rows[i]["ncc_to_next"] = ncc_pair(raws[i], raws[i + 1], ratio)

        contact_sheet(site, RUNGS_KM, frames, rows, out / f"ladder{args.tag}_{name}.png")
        for r in rows:
            r.pop("crop", None)
        report[name] = {"class": klass, "target": [tlat, tlon], "bearing": bearing,
                        "note": note, "target_truth": t, "rungs": rows}

        worst = [r for r in rows if r.get("flag")]
        if worst:
            alts = ", ".join(f"{r['alt_km']:g}km (step {r['staircase']['step_m']:.0f} m = "
                             f"{r['staircase']['step_voxels']:.0f}x voxel)" for r in worst)
            findings.append(f"{name}: STAIRCASE at {alts}")
        bad_aim = [r for r in rows if r["framing"].get("miss_frac_halfframe", 0) > 0.02]
        if bad_aim:
            findings.append(f"{name}: FRAMING drift on "
                            f"{[r['alt_km'] for r in bad_aim]} km rungs")
        weak = [r for r in rows[:-1]
                if (r.get("ncc_to_next") or {}).get("peak") is not None
                and r["ncc_to_next"]["peak"] < 0.30]
        if weak:
            findings.append(
                f"{name}: WEAK rung-to-rung correlation on "
                f"{[r['alt_km'] for r in weak]} km - the frames stopped showing the "
                "same thing. NOT a framing failure (the aim-miss column proves the "
                "pose): either the feature fell below one pixel, or the renderer "
                "changed its mind about the terrain between the two rungs.")

    (out / f"report{args.tag}.json").write_text(json.dumps(report, indent=1), encoding="utf-8")
    print(f"contact sheets -> {out}")
    if findings:
        print(f"LADDER: {len(findings)} finding(s) to review:")
        for f in findings:
            print(" ", f)
        sys.exit(3)
    print(f"LADDER: {len(sites)} site(s) clean")


if __name__ == "__main__":
    main()
