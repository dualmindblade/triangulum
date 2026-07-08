#!/usr/bin/env python3
"""Generate a broad, self-checking play-harness survey from the source planet
data — the automated-discovery front line.

Where hand-authored probes (and invariant-survey.play) check a handful of
coordinates a human picked, this samples MANY coordinates straight from
output/<world>/planet_data.npz and asserts the universal invariant for each
feature class. That turns every single-point bug the AI hunt found into a
whole-class gate:

  * frozen lake  -> must be walkable ice (grounded, not underwater, dry)   [R3]
  * inland land  -> must be walkable (grounded)                            [fall-through-world]
  * high peak    -> must still support the player (grounded)
  * liquid lake  -> must read as water and immerse the player
  * ocean        -> must submerge a settling player (has_water, underwater)[R2/immersion]

Interior cells are sampled where a neighbour ring is needed to avoid edge
false alarms; liquid lakes use the lake cell itself because many real lakes are
too small for an all-neighbour interior ring. Sampling is deterministic (sorted
indices, even stride) so the emitted survey is reproducible and diffable.

Usage:
  python viewer/scripts/gen_survey.py [--world output/seed42_r8]
                                      [--out viewer/scripts/auto-survey.play]
                                      [--per N]
Then run the result:
  ./viewer/target/release/examples/play.exe viewer/scripts/auto-survey.play
A non-zero exit means a coordinate violated its class invariant — a candidate
bug to investigate (or a genuine feature-edge false alarm to tighten out).
"""
import argparse
import numpy as np
from pathlib import Path

# The gnomonic cube-sphere faces, mirrored from viewer/src/planet.rs FACES:
# each is (axis, right, up); a face-local (u, v) in [-1,1] maps to the unit
# direction normalize(axis + u*right + v*up). Cube-face SEAMS are |u|==1 or
# |v|==1 — the cross-face stitch the chunk mesher and physics must get right.
FACES = np.array([
    [[1, 0, 0], [0, 1, 0], [0, 0, 1]],
    [[-1, 0, 0], [0, -1, 0], [0, 0, 1]],
    [[0, 1, 0], [-1, 0, 0], [0, 0, 1]],
    [[0, -1, 0], [1, 0, 0], [0, 0, 1]],
    [[0, 0, 1], [0, 1, 0], [-1, 0, 0]],
    [[0, 0, -1], [0, 1, 0], [1, 0, 0]],
], dtype=float)


def face_dir(face, u, v):
    axis, right, up = FACES[face]
    d = axis + u * right + v * up
    return d / np.linalg.norm(d)


def dir_latlon(d):
    """Unit vector -> (lat_deg, lon_deg), matching planet_data (asin z, atan2)."""
    return np.degrees(np.arcsin(d[2])), np.degrees(np.arctan2(d[1], d[0]))


def seam_coords(step=0.12):
    """Sample lat/lon along every cube-face seam (|u|==1 or |v|==1),
    deduped — shared edges yield the same direction from both faces."""
    ts = np.arange(-1.0 + step, 1.0, step)
    seen, out = set(), []
    for f in range(6):
        for val in (-1.0, 1.0):
            for t in ts:
                for u, v in ((val, t), (t, val)):
                    lat, lon = dir_latlon(face_dir(f, u, v))
                    key = (round(lat, 1), round(lon, 1))
                    if key not in seen:
                        seen.add(key)
                        out.append((lat, lon))
    return out


def interior(mask, nb):
    """Cells in `mask` whose every VALID neighbour is also in `mask`."""
    valid = nb >= 0
    nbm = np.where(valid, mask[np.where(valid, nb, 0)], True)
    return mask & nbm.all(axis=1)


def sample(idx, count):
    """Deterministic even-stride pick of `count` indices from sorted `idx`."""
    idx = np.sort(idx)
    if len(idx) <= count:
        return idx
    stride = len(idx) / count
    return idx[np.floor(np.arange(count) * stride).astype(int)]


def feature_masks(d):
    """Per-cell boolean masks for each feature class — THE shared definition,
    used by both the survey generator and the where.py feature query so their
    thresholds never drift. Interior-only where a point feature could miss its
    edge after the icosphere->cube resample; liquid lakes are intentionally
    sampled at their own cell centers because most are too small for a full
    neighbour ring. Below-freezing water (lake OR sea) renders as a solid ice
    sheet, so it is split out from open water."""
    tmean = d["temp_c_monthly"].mean(axis=0)
    elev = d["elevation_km"]
    nb = d["neighbors"]
    ocean = d["is_ocean"]
    lake = d["lake_id"] >= 0
    river = d["river"]
    dry_land = (~ocean) & ~lake & ~river
    hi = elev >= np.quantile(elev[dry_land], 0.98)
    return {
        "inland-land": interior(dry_land, nb),
        "high-peak":   interior(dry_land & hi, nb),
        "frozen-lake": interior(lake & (tmean < -6.0), nb),
        "sea-ice":     interior(ocean & (tmean < -6.0), nb),
        "liquid-lake": lake & (tmean > -4.0),
        "ocean":       interior(ocean & (tmean > 2.0), nb),
        "river":       river & (~ocean),
    }


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--world", default="output/seed42_r8")
    ap.add_argument("--out", default="viewer/scripts/auto-survey.play")
    ap.add_argument("--per", type=int, default=16,
                    help="base samples per class (peaks/ocean scaled down)")
    args = ap.parse_args()

    d = np.load(Path(args.world) / "planet_data.npz")
    latd = np.degrees(d["lat"])
    lond = np.degrees(d["lon"])
    tmean = d["temp_c_monthly"].mean(axis=0)
    elev = d["elevation_km"]
    xyz = d["xyz"].astype(float)
    m = feature_masks(d)
    inland = m["inland-land"]

    # cube-face seams that fall on dry land: teleport onto the cross-face stitch
    # and assert the player stands (a seam bug drops you through or blocks you).
    # The coordinate comes from the cube geometry, not a planet cell, so map it
    # to its nearest cell and keep only interior dry land.
    edge_coords = []
    for lat, lon in seam_coords():
        rlat, rlon = np.radians(lat), np.radians(lon)
        v = np.array([np.cos(rlat) * np.cos(rlon),
                      np.cos(rlat) * np.sin(rlon),
                      np.sin(rlat)])
        j = int((xyz @ v).argmax())
        if inland[j]:  # nearest cell is interior dry land -> should be walkable
            edge_coords.append((lat, lon, j))

    # (class label, index-mask, count, [(field, op, value), ...], settle secs)
    P = args.per
    grounded = [("grounded", "==", "true")]
    ice = [("grounded", "==", "true"), ("underwater", "==", "false"),
           ("has_water", "==", "false")]
    classes = [
        ("inland-land", m["inland-land"], P,             grounded, 8),
        ("high-peak",   m["high-peak"],   max(6, P // 2), grounded, 8),
        ("frozen-lake", m["frozen-lake"], P,             ice, 8),
        ("sea-ice",     m["sea-ice"],     max(8, P // 2), ice, 8),
        ("liquid-lake", m["liquid-lake"], max(8, P // 2),
                                          [("has_water", "==", "true"),
                                           ("underwater", "==", "true")], 10),
        ("ocean",       m["ocean"],       max(6, P // 2),
                                          [("has_water", "==", "true"),
                                           ("underwater", "==", "true")], 12),
    ]

    lines = [
        "# AUTO-GENERATED by viewer/scripts/gen_survey.py — do not hand-edit.",
        f"# Broad invariant sweep sampled from {args.world}/planet_data.npz.",
        "# Each block settles the player at a real feature cell and asserts the",
        "# universal invariant for that class. A failure = a candidate bug.",
        "",
    ]
    total = 0
    for label, mask, count, asserts, settle in classes:
        idx = sample(np.flatnonzero(mask), count)
        lines.append(f"# ===== {label}: {len(idx)} probes "
                     f"(of {int(mask.sum())} class cells) =====")
        for i in idx:
            total += 1
            lines.append(f"# {label} @ cell {int(i)} tmean={tmean[i]:.1f}C "
                         f"elev={elev[i]:.3f}km")
            lines.append(f"teleport {latd[i]:.4f} {lond[i]:.4f} 0.05")
            lines.append("mode walk")
            lines.append(f"wait {settle}")
            for field, op, val in asserts:
                lines.append(f"assert {field} {op} {val}")
            lines.append("")

    # cube-face-edge class: explicit seam coordinates (not planet cells)
    edges = edge_coords[:: max(1, len(edge_coords) // P)][:P] if edge_coords else []
    lines.append(f"# ===== face-edge: {len(edges)} land seams "
                 f"(of {len(edge_coords)} found) =====")
    for lat, lon, j in edges:
        total += 1
        lines.append(f"# face-edge near cell {j} elev={elev[j]:.3f}km")
        lines.append(f"teleport {lat:.4f} {lon:.4f} 0.05")
        lines.append("mode walk")
        lines.append("wait 8")
        lines.append("assert grounded == true")
        lines.append("")

    Path(args.out).write_text("\n".join(lines), encoding="utf-8")
    n_classes = len(classes) + (1 if edges else 0)
    print(f"wrote {args.out}: {total} probes across {n_classes} classes")


if __name__ == "__main__":
    main()
