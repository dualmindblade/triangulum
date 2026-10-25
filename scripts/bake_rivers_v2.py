"""Bake planetgen's rivers and lakes for the engine: `hydro.bin` + `hydro.json`.

Writes next to the world bake (default game/assets/world/seed42), which the
engine opens together with the face rasters. The engine turns this into
water at rest (game/src/worldgen/hydro.rs); everything here is in planetgen's
own units (km on the 6,371 km source sphere, unit vectors for positions), so
the engine applies its vertical scale and block size itself.

What it exports
---------------
* **River network.** Every drainage-graph link cell -> receiver whose cell
  carries more than Q_MIN m^3/s (and is not ocean). Nodes are planetgen
  cells, stored so that a node's receiver always comes first (terminals
  first), so one forward pass makes levels monotone downstream.
* **Node levels** (km): the hydraulic fill surface of the *world bake's*
  elevation (the surface the engine's terrain is built on, Catmull-Rom like
  the engine samples it), never the bed: level = max(own bake elevation,
  receiver's level), pinned to the lake surface inside lakes and to 0 at the
  sea, and also 0 at the last land node before the sea (an estuary) so the
  river meets the coast at sea level. Each node also carries its terminal
  level (the lake or sea it drains into) and its source level (the lake it
  last left, upstream along the main stem) so the engine can taper its
  valley incision to nothing where a river enters or leaves standing water.
* **Course curves.** Each link is a centripetal Catmull-Rom curve through
  (main upstream, a, b, receiver of b), sampled at SUB points: tributaries
  arrive tangent to the river they join, and the raw 30 km zigzag of the
  cell graph becomes smooth bends.
* **Lakes.** planetgen's lakes after the July conduit peel (W-5), each capped
  so the world bake's own surface holds it: the level never exceeds the
  lowest bake elevation on the lake's shore band by more than RIM_TOL_KM
  (the engine raises the ground that little to make the shore). Cells whose
  bake surface stands above the capped level leave the lake; this repeats
  until stable.
* **Lake field cells.** Every planetgen cell within 2 supports of a lake
  cell, flagged with its lake (or -1). The engine's lake field is the
  Shepard (degree-0 MLS) average of those flags with a Wendland C4 kernel of
  support LAKE_SUPPORT spacings: smooth (C4), never overshooting, and the
  same formula this script uses to check the shore band.

Usage: python scripts/bake_rivers_v2.py [--run output/seed42_r8]
       [--bake game/assets/world/seed42] [--qmin 120]
"""

import argparse
import json
import os
import pickle
import struct
import subprocess
import sys
import time

import numpy as np
from scipy.spatial import cKDTree

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import world_interp as wi  # noqa: E402

SUB = 8                 # curve points per link (SUB - 1 interior points)
LAKE_SUPPORT = 2.0      # lake field support, in mean cell spacings
LAKE_ZONE = 0.30        # engine: lake water where the field exceeds this
BAND = (0.06, 0.24)     # shore band checked for the rim cap (field values)
RIM_TOL_KM = 0.025      # lake level may exceed the bake's shore by this
PEEL_DEEP_KM = 0.30     # July conduit peel (W-5)
PEEL_CHAIN_KM = 0.25


def git_head():
    try:
        return subprocess.check_output(["git", "rev-parse", "--short", "HEAD"],
                                       cwd=os.path.dirname(os.path.abspath(__file__)),
                                       stderr=subprocess.DEVNULL).decode().strip()
    except Exception:
        return None


# ----------------------------------------------------------------------------
# The world bake's elevation, sampled exactly as the engine does (bake.rs):
# equal-angle texels, edge clamped per face, Catmull-Rom bicubic.
# ----------------------------------------------------------------------------

class BakeElevation:
    def __init__(self, bake_dir):
        meta = json.load(open(os.path.join(bake_dir, "meta.json")))
        lay = meta["layout"]
        self.res = int(lay["resolution"])
        assert lay["projection"] == "equal_angle" and lay["edge_inclusive"]
        f = next(f for f in meta["fields"] if f["name"] == "elevation_km")
        n = self.res * self.res
        self.faces = []
        for face in range(6):
            path = os.path.join(bake_dir, lay["file_pattern"].replace("{face}", str(face)))
            with open(path, "rb") as fh:
                fh.seek(int(f["offset"]))
                self.faces.append(np.frombuffer(fh.read(4 * n), "<f4").astype(np.float64).reshape(self.res, self.res))

    @staticmethod
    def _cr(t):
        t2, t3 = t * t, t * t * t
        return np.stack([0.5 * (-t3 + 2 * t2 - t), 0.5 * (3 * t3 - 5 * t2 + 2),
                         0.5 * (-3 * t3 + 4 * t2 + t), 0.5 * (t3 - t2)], -1)

    def __call__(self, dirs):
        dirs = np.atleast_2d(np.asarray(dirs, np.float64))
        face, a, b = wi.direction_to_face(dirs)
        m = self.res - 1
        x = np.clip((np.arctan(a) / (np.pi / 4) * 0.5 + 0.5) * m, 0, m)
        y = np.clip((np.arctan(b) / (np.pi / 4) * 0.5 + 0.5) * m, 0, m)
        x0, y0 = np.floor(x), np.floor(y)
        wx, wy = self._cr(x - x0), self._cr(y - y0)
        ix, iy = x0.astype(np.int64), y0.astype(np.int64)
        out = np.zeros(len(dirs))
        for f in range(6):
            sel = np.flatnonzero(face == f)
            if len(sel) == 0:
                continue
            g = self.faces[f]
            acc = np.zeros(len(sel))
            for dy in range(-1, 3):
                yy = np.clip(iy[sel] + dy, 0, m)
                row = np.zeros(len(sel))
                for dx in range(-1, 3):
                    xx = np.clip(ix[sel] + dx, 0, m)
                    row += wx[sel, dx + 1] * g[yy, xx]
                acc += wy[sel, dy + 1] * row
            out[sel] = acc
        return out


# ----------------------------------------------------------------------------
# Lake field: Shepard average of lake flags (Wendland C4), as hydro.rs does.
# ----------------------------------------------------------------------------

def wendland(q):
    q = np.minimum(q, 1.0)
    u = 1.0 - q
    return u ** 6 * (35.0 * q * q + 18.0 * q + 3.0)


def lake_field(tree, cells, lake_of_cell, query, r_chord, lid, kmax=64):
    """Lake `lid`'s own field (its cells' share of the kernel weight) at unit
    directions `query`, as hydro.rs computes it."""
    dist, idx = tree.query(query, k=kmax, distance_upper_bound=r_chord, workers=-1)
    if np.isfinite(dist[:, -1]).any():
        raise RuntimeError("lake field kmax truncates the support")
    ok = np.isfinite(dist)
    w = np.where(ok, wendland(np.where(ok, dist, 0.0) / r_chord), 0.0)
    w[~ok] = 0.0
    idx_c = np.where(ok, idx, 0)
    lk = np.where(ok, lake_of_cell[idx_c], -1)
    num = (w * (lk == lid)).sum(1)
    den = w.sum(1)
    return np.where(den > 0, num / np.maximum(den, 1e-300), 0.0)


def tangent_grid(center, ext_chord, step_chord):
    c = center / np.linalg.norm(center)
    ax = np.array([0.0, 0.0, 1.0]) if abs(c[2]) < 0.9 else np.array([1.0, 0.0, 0.0])
    t1 = ax - c * ax.dot(c)
    t1 /= np.linalg.norm(t1)
    t2 = np.cross(c, t1)
    g = np.arange(-ext_chord, ext_chord + step_chord, step_chord)
    gu, gv = np.meshgrid(g, g)
    p = c[None, :] + gu.ravel()[:, None] * t1[None, :] + gv.ravel()[:, None] * t2[None, :]
    return p / np.linalg.norm(p, axis=1, keepdims=True)


# ----------------------------------------------------------------------------

def catmull_centripetal(p0, p1, p2, p3, ts):
    """Centripetal Catmull-Rom between p1 and p2 (rows of 3-vectors), at ts in [0, 1]."""
    def knot(a, b, t):
        return t + max(np.linalg.norm(b - a), 1e-12) ** 0.5
    t0 = 0.0
    t1 = knot(p0, p1, t0)
    t2 = knot(p1, p2, t1)
    t3 = knot(p2, p3, t2)
    out = []
    for s in ts:
        t = t1 + s * (t2 - t1)
        a1 = (t1 - t) / (t1 - t0) * p0 + (t - t0) / (t1 - t0) * p1
        a2 = (t2 - t) / (t2 - t1) * p1 + (t - t1) / (t2 - t1) * p2
        a3 = (t3 - t) / (t3 - t2) * p2 + (t - t2) / (t3 - t2) * p3
        b1 = (t2 - t) / (t2 - t0) * a1 + (t - t0) / (t2 - t0) * a2
        b2 = (t3 - t) / (t3 - t1) * a2 + (t - t1) / (t3 - t1) * a3
        c = (t2 - t) / (t2 - t1) * b1 + (t - t1) / (t2 - t1) * b2
        out.append(c / np.linalg.norm(c))
    return np.array(out)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    ap.add_argument("--run", default=os.path.join(root, "output", "seed42_r8"))
    ap.add_argument("--bake", default=os.path.join(root, "game", "assets", "world", "seed42"))
    ap.add_argument("--out", default=None, help="output directory (default: the bake directory)")
    ap.add_argument("--qmin", type=float, default=120.0, help="smallest exported discharge, m^3/s")
    args = ap.parse_args()
    out_dir = args.out or args.bake
    t_start = time.time()

    d = np.load(os.path.join(args.run, "planet_data.npz"))
    xyz = d["xyz"].astype(np.float64)
    xyz /= np.linalg.norm(xyz, axis=1, keepdims=True)
    elev = d["elevation_km"].astype(np.float64)
    flow = d["flow_accum_m3s"].astype(np.float64)
    rcv = d["receiver"].astype(np.int64)
    ocean = d["is_ocean"].astype(bool)
    lake_id = d["lake_id"].astype(np.int64)
    nbr = d["neighbors"].astype(np.int64)
    n = len(elev)
    ids = np.arange(n)
    hyd = pickle.load(open(os.path.join(args.run, "cache", "hydrology.pkl"), "rb"))["hydro"]
    lake_level_cell = np.asarray(hyd["lake_level"], np.float64)
    lake_salt_cell = np.asarray(hyd["lake_salt"], bool)
    assert np.array_equal(np.asarray(hyd["lake_id"]) >= 0, lake_id >= 0), "hydrology cache does not match planet_data.npz"
    spacing_km = float(json.load(open(os.path.join(args.bake, "meta.json")))["source"]["mean_neighbour_spacing_km"])
    bake_elev = BakeElevation(args.bake)
    eb = bake_elev(xyz)  # bake surface at every cell centre (km)
    print(f"{n} cells, spacing {spacing_km:.2f} km; bake surface at cell centres vs planetgen: "
          f"median |d| {np.median(np.abs(eb - elev)) * 1000:.1f} m", flush=True)

    # ------------------------------------------------------------------ lakes
    lids = np.unique(lake_id[lake_id >= 0])
    cells_of = {}
    level_of = {}
    for lid in lids:
        cells = set(ids[lake_id == lid].tolist())
        spill0 = lake_level_cell[next(iter(cells))]
        cells = {c for c in cells if spill0 - elev[c] <= PEEL_DEEP_KM}
        while True:
            peel = [c for c in cells if sum(1 for x in nbr[c] if x >= 0 and x in cells) <= 2 and spill0 - elev[c] > PEEL_CHAIN_KM]
            if not peel:
                break
            cells.difference_update(peel)
        cells_of[lid] = cells
        level_of[lid] = spill0
    n_peeled = sum(1 for lid in lids if len(cells_of[lid]) < (lake_id == lid).sum())

    r_chord = wi.km_to_chord(LAKE_SUPPORT * spacing_km)
    tree_all = cKDTree(xyz)
    # (Neighbouring lakes keep their own levels: the engine gives every lake
    # its own field, so where two come close each one's shore apron dams the
    # other. planetgen never makes two lakes' cells neighbours.)
    valid = (rcv >= 0) & (rcv != ids) & ~ocean
    seg_src = ids[valid & (flow > args.qmin)]
    node_cells = np.unique(np.concatenate([seg_src, rcv[seg_src]]))
    is_node = np.zeros(n, bool)
    is_node[node_cells] = True
    mouth = valid & ocean[np.where(rcv >= 0, rcv, 0)]

    def cap_lakes(dirty):
        """Cap each dirty lake's level to its shore band's bake surface and
        drop its cells that stand above the level, until stable."""
        nonlocal capped, dropped_cells
        for it in range(8):
            lake_of = np.full(n, -1, np.int64)
            for lid in lids:
                for c in cells_of[lid]:
                    lake_of[c] = lid
            live = np.flatnonzero(lake_of >= 0)
            if len(live) == 0:
                return
            near = tree_all.query_ball_point(xyz[live], 2.0 * r_chord, workers=-1)
            fc = np.unique(np.concatenate([np.asarray(v, np.int64) for v in near]))
            ftree = cKDTree(xyz[fc])
            flake = lake_of[fc]
            changed = False
            for lid in sorted(dirty):
                cells = np.array(sorted(cells_of[lid]), np.int64)
                if len(cells) == 0:
                    continue
                c = xyz[cells].mean(0)
                ext = np.linalg.norm(xyz[cells] - c / np.linalg.norm(c), axis=1).max() + 1.6 * r_chord
                pts = tangent_grid(c, ext, r_chord / 10.0)
                f = lake_field(ftree, xyz[fc], flake, pts, r_chord, lid)
                band = (f >= BAND[0]) & (f <= BAND[1])
                if band.any():
                    rim = bake_elev(pts[band]).min()
                    if level_of[lid] > rim + RIM_TOL_KM:
                        level_of[lid] = rim + RIM_TOL_KM
                        capped += 1
                        changed = True
                dry = [cc for cc in cells if eb[cc] >= level_of[lid]]
                if dry:
                    cells_of[lid].difference_update(dry)
                    dropped_cells += len(dry)
                    changed = True
            if not changed:
                return

    def river_levels():
        """Fill surface of the bake elevation, pinned at lakes, the sea and
        estuaries, then relaxed downstream so no link climbs."""
        lake_of = np.full(n, -1, np.int64)
        for lid in lids:
            for c in cells_of[lid]:
                lake_of[c] = lid
        in_lake = lake_of >= 0
        level = eb.copy()
        level[ocean] = 0.0
        for lid in lids:
            for c in cells_of[lid]:
                level[c] = level_of[lid]
        # estuaries: the last land node before the sea is at sea level
        level[mouth & ~in_lake] = 0.0
        fixed = ocean | in_lake | mouth
        src = ids[valid & ~fixed]
        dst = rcv[src]
        for _ in range(100000):
            nl = np.maximum(level[src], level[dst])
            if np.array_equal(nl, level[src]):
                break
            level[src] = nl
        # A lake capped below its rim (or a lake outlet whose bake surface
        # stands above the surface) would send its outlet uphill: relax
        # downstream so no link climbs (the engine cuts the channel through).
        lsrc = ids[valid]
        ldst = rcv[lsrc]
        open_dst = ~fixed[ldst]
        lsrc, ldst = lsrc[open_dst], ldst[open_dst]
        lowered = np.zeros(n, bool)
        for _ in range(100000):
            before = level[ldst].copy()
            np.minimum.at(level, ldst, level[lsrc])
            moved = level[ldst] < before
            if not moved.any():
                break
            lowered[ldst[moved]] = True
        return level, lake_of, lowered

    capped = 0
    dropped_cells = 0
    dirty = set(lids.tolist())
    lowered_lakes = 0
    for outer in range(20):
        cap_lakes(dirty)
        level, lake_of_lid, lowered = river_levels()
        # A lake downstream of a lowered one must not stand above the river
        # that feeds it (it would climb): lower it to that river and repeat.
        dirty = set()
        links = ids[valid]
        into = lake_of_lid[rcv[links]]
        for c, lid in zip(links[into >= 0], into[into >= 0]):
            if lake_of_lid[c] != lid and level[c] < level_of[lid] - 1e-12:
                level_of[lid] = level[c]
                dirty.add(int(lid))
        if not dirty:
            break
        lowered_lakes += len(dirty)
    lakes_live = [lid for lid in lids if len(cells_of[lid]) > 0]
    lake_index = {lid: k for k, lid in enumerate(lakes_live)}
    lake_of = np.full(n, -1, np.int64)
    for lid in lakes_live:
        for c in cells_of[lid]:
            lake_of[c] = lake_index[lid]
    live = np.flatnonzero(lake_of >= 0)
    near = tree_all.query_ball_point(xyz[live], 2.0 * r_chord, workers=-1)
    field_cells = np.unique(np.concatenate([np.asarray(v, np.int64) for v in near])) if len(live) else np.zeros(0, np.int64)
    lake_levels = np.array([level_of[lid] for lid in lakes_live])
    lake_salts = np.array([bool(lake_salt_cell[sorted(cells_of[lid])[0]]) for lid in lakes_live])
    in_lake = lake_of >= 0
    fixed = ocean | in_lake | mouth
    lsrc = ids[valid]
    climbing = int((level[lsrc] < level[rcv[lsrc]] - 1e-12).sum())
    print(f"lakes: {len(lids)} from planetgen, {n_peeled} peeled (W-5), {capped} caps to the bake's shore "
          f"(+{RIM_TOL_KM * 1000:.0f} m), {lowered_lakes} lowered below a capped lake upstream, {dropped_cells} cells left dry, "
          f"{len(lakes_live)} kept ({len(live)} cells, {len(field_cells)} field cells)", flush=True)
    print(f"levels: fill surface, {int(lowered[is_node].sum())} nodes lowered below their fill to keep outlets "
          f"monotone, {climbing} climbing links left (of {len(lsrc)})", flush=True)
    # walk to the terminal (memoised): link depth and terminal level
    term = np.full(n, np.nan)
    depth = np.full(n, -1, np.int64)
    term[ocean] = 0.0
    depth[ocean] = 0
    for c in node_cells:
        path = []
        x = c
        while depth[x] < 0:
            path.append(x)
            r = rcv[x]
            if not valid[x] or r < 0 or r == x:
                depth[x] = 0
                term[x] = level[x]
                path.pop()
                break
            x = r
        dx, tx = depth[x], term[x]
        for p in reversed(path):
            dx += 1
            depth[p] = dx
            tx = level[p] if in_lake[p] else tx
            term[p] = tx
    # terminal of a lake node is its own lake; of a node flowing into a lake, that lake
    # (the walk above carries the nearest lake downstream because lake nodes reset tx)
    # source level: the lake a node's main stem last left
    main_up = np.full(n, -1, np.int64)
    best = np.full(n, -1.0)
    for c in seg_src:
        r = rcv[c]
        if flow[c] > best[r]:
            best[r] = flow[c]
            main_up[r] = c
    BIG = 1.0e6
    srcl = np.full(n, BIG)
    for c in node_cells[np.argsort(-depth[node_cells], kind="stable")]:  # upstream first
        if in_lake[c]:
            srcl[c] = level[c]
        elif main_up[c] >= 0:
            srcl[c] = srcl[main_up[c]]
    # node order: every receiver before its sources (terminals first)
    node_order = node_cells[np.lexsort((node_cells, depth[node_cells]))]
    node_idx = np.full(n, -1, np.int64)
    node_idx[node_order] = np.arange(len(node_order))
    seg_of_node = np.full(len(node_order), -1, np.int64)
    segs = []
    for c in node_order:  # segments in node order too
        if valid[c] and flow[c] > args.qmin:
            seg_of_node[node_idx[c]] = len(segs)
            segs.append(c)
    segs = np.array(segs, np.int64)
    # course curves
    ts = np.arange(1, SUB) / SUB
    interior = np.zeros((len(segs), SUB - 1, 3))
    for k, c in enumerate(segs):
        b = rcv[c]
        p1, p2 = xyz[c], xyz[b]
        up = main_up[c]
        p0 = xyz[up] if up >= 0 else 2 * p1 - p2
        bb = rcv[b]
        # The main stem runs on smoothly through b; a tributary arrives
        # straight (not tangent to the river it joins), so the two channels
        # overlap only right at the junction.
        main = main_up[b] == c
        p3 = xyz[bb] if (main and valid[b] and bb >= 0 and bb != b) else 2 * p2 - p1
        interior[k] = catmull_centripetal(p0, p1, p2, p3, ts)
    seg_len_km = np.array([np.linalg.norm(np.diff(np.vstack([xyz[c], interior[k], xyz[rcv[c]]]), axis=0), axis=1).sum() * 6371.0 for k, c in enumerate(segs)])
    kind = np.zeros(n, np.uint8)
    kind[ocean] = 1
    kind[in_lake] = 2
    kind[mouth & ~in_lake] = 3
    print(f"rivers: {len(segs)} links over {seg_len_km.sum():.0f} km (planetgen units), {len(node_order)} nodes, "
          f"{int((kind[node_order] == 1).sum())} sea terminals, {int((kind[node_order] == 3).sum())} estuary nodes", flush=True)

    # ------------------------------------------------------------------ write
    os.makedirs(out_dir, exist_ok=True)
    path = os.path.join(out_dir, "hydro.bin")
    with open(path, "wb") as fh:
        fh.write(b"TRH1")
        fh.write(struct.pack("<IIIIII", 1, len(node_order), len(segs), len(lakes_live), len(field_cells), SUB))
        fh.write(struct.pack("<dd", r_chord, LAKE_ZONE))
        for c in node_order:
            r = rcv[c]
            rn = node_idx[r] if (valid[c] and r >= 0 and is_node[r] and r != c) else -1
            fh.write(struct.pack("<dddffffiiiB3x", *xyz[c], level[c], flow[c],
                                 term[c] if np.isfinite(term[c]) else level[c], srcl[c],
                                 rn, seg_of_node[node_idx[c]], lake_of[c], kind[c]))
        for k, c in enumerate(segs):
            fh.write(struct.pack("<ii", node_idx[c], node_idx[rcv[c]]))
            fh.write(interior[k].astype("<f8").tobytes())
        for k in range(len(lakes_live)):
            fh.write(struct.pack("<fB3x", lake_levels[k], int(lake_salts[k])))
        fcx = xyz[field_cells].astype("<f8")
        for k, c in enumerate(field_cells):
            fh.write(struct.pack("<dddi4x", *fcx[k], lake_of[c]))
    size = os.path.getsize(path)

    # notable places for evidence shots (lat, lon in degrees)
    def latlon(p):
        p = p / np.linalg.norm(p)
        return [round(float(np.degrees(np.arcsin(p[2]))), 5), round(float(np.degrees(np.arctan2(p[1], p[0]))), 5)]
    big = segs[np.argsort(-flow[segs])][:10]
    mouths = [c for c in segs if ocean[rcv[c]]]
    mouths = sorted(mouths, key=lambda c: -flow[c])[:10]
    drops = []
    for k, c in enumerate(segs):
        drops.append((level[c] - level[rcv[c]], k, c))
    drops.sort(reverse=True)
    face_of, _, _ = wi.direction_to_face(xyz)
    edge_cross = [c for c in segs if face_of[c] != face_of[rcv[c]]]
    edge_cross = sorted(edge_cross, key=lambda c: -flow[c])[:10]
    lake_sizes = sorted(((len(cells_of[lid]), lid) for lid in lakes_live), reverse=True)[:10]
    meta = {
        "format": "triangulum-hydro",
        "format_version": 1,
        "script": "scripts/bake_rivers_v2.py",
        "git_head": git_head(),
        "source": os.path.abspath(os.path.join(args.run, "planet_data.npz")),
        "bake": os.path.abspath(args.bake),
        "units": "positions are unit vectors; levels and elevations km on the 6,371 km source sphere",
        "qmin_m3s": args.qmin,
        "curve_points_per_link": SUB,
        "lake_field": {"method": "Shepard (degree-0 MLS) of lake flags, Wendland C4", "support_spacings": LAKE_SUPPORT,
                        "support_chord": r_chord, "zone": LAKE_ZONE},
        "lake_rim_tolerance_km": RIM_TOL_KM,
        "counts": {"nodes": int(len(node_order)), "links": int(len(segs)), "lakes": int(len(lakes_live)),
                   "lake_cells": int(len(live)), "field_cells": int(len(field_cells)),
                   "planetgen_lakes": int(len(lids)), "lakes_peeled": int(n_peeled), "lakes_capped": int(capped),
                   "lake_cells_left_dry": int(dropped_cells)},
        "bytes": size,
        "seconds": round(time.time() - t_start, 1),
        "places": {
            "largest_rivers": [{"latlon": latlon(xyz[c]), "flow_m3s": round(float(flow[c]))} for c in big],
            "largest_mouths": [{"latlon": latlon(0.5 * (xyz[c] + xyz[rcv[c]])), "flow_m3s": round(float(flow[c]))} for c in mouths],
            "steepest_links": [{"latlon": latlon(0.5 * (xyz[c] + xyz[rcv[c]])), "drop_km": round(float(dk), 4), "flow_m3s": round(float(flow[c]))} for dk, k, c in drops[:20]],
            "face_edge_crossings": [{"latlon": latlon(0.5 * (xyz[c] + xyz[rcv[c]])), "faces": [int(face_of[c]), int(face_of[rcv[c]])], "flow_m3s": round(float(flow[c]))} for c in edge_cross],
            "largest_lakes": [{"latlon": latlon(xyz[sorted(cells_of[lid])].mean(0)), "cells": int(nc), "level_km": round(float(level_of[lid]), 4)} for nc, lid in lake_sizes],
        },
    }
    with open(os.path.join(out_dir, "hydro.json"), "w") as fh:
        json.dump(meta, fh, indent=1)
    print(f"wrote {path} ({size / 1e6:.1f} MB) and hydro.json in {time.time() - t_start:.1f} s")


if __name__ == "__main__":
    main()
