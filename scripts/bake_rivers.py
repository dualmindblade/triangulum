"""Export river courses and lakes from the planet dataset for the viewer.

The viewer generates river channels by measuring exact distance to these
polylines (cell -> receiver segments of the drainage graph), so rivers
follow the map's real valleys and reach the sea — courses are no longer
invented from noise. Water levels ride the graph too, forced monotonic
downstream so rivers can never flow uphill.

Output viewer/assets/rivers.bin (little-endian):
  magic  b"RIV1"
  u32    n_segments
  u32    n_lake_cells
  n_segments * 9 f32:  ax ay az bx by bz  flow_log10  level_a_km  level_b_km
                       (a = upstream cell unit vector, b = receiver)
  n_lake_cells * 6 f32: x y z  radius_km  level_km  flags
                        flags: 0 fresh lake, 1 salt lake, 2 rim (a non-lake
                        cell bordering a lake — shipped so the viewer can
                        clip lake water to the Voronoi footprint of the
                        actual lake cells instead of blobby per-cell discs)

Usage: python scripts/bake_rivers.py [run_dir]
"""

import os
import struct
import sys

import numpy as np
from scipy.spatial import cKDTree

run_dir = sys.argv[1] if len(sys.argv) > 1 else "output/seed42_r8"
out_path = "viewer/assets/rivers.bin"

d = np.load(os.path.join(run_dir, "planet_data.npz"))
xyz = d["xyz"].astype(np.float64)
xyz /= np.linalg.norm(xyz, axis=1, keepdims=True)
elev = d["elevation_km"].astype(np.float64)
flow = d["flow_accum_m3s"].astype(np.float64)
rcv = d["receiver"].astype(np.int64)
ocean = d["is_ocean"]
lake = d["lake_id"].astype(np.int64)
salt = d["lake_salt"]
area = d["area_km2"].astype(np.float64)
nb = d["neighbors"].astype(np.int64)

n = len(elev)
ids = np.arange(n)

# ---- water levels: the HYDRAULIC FILL SURFACE, not the bed. Each level is
# max(own elevation, receiver's level), propagated up from the terminals —
# monotonic downstream BY CONSTRUCTION, and a depression-filled route holds
# at each pit's spill height instead of bed-diving. The old min-relaxation
# anchored levels to the deepest bed along the path, so a river routed
# through a dry below-sea basin (or exported into ocean bathymetry) carried
# kilometre-negative levels and the viewer dug bottomless slot gorges next
# to sea-level terrain (BUGS.md W-1 family; census families B/C).
# Where fill > bed the viewer's perch guard renders a dry wash / lets the
# lake flood own the water — both correct for a pit.
level = elev.copy()
# an OCEAN cell's water level is the sea surface, not its bathymetry; ocean
# sources are excluded so nothing ever inherits bathymetry.
level[ocean] = 0.0
valid = (rcv >= 0) & (rcv != ids) & ~ocean
src = ids[valid]
dst = rcv[valid]
for it in range(100000):
    nl = np.maximum(level[src], level[dst])  # source never below receiver
    if np.array_equal(nl, level[src]):
        break
    level[src] = nl
print(f"fill-surface construction converged after {it + 1} passes")

# ---- river segments: planetgen flags rivers at ~350 m3/s; extend down to
# 120 so headwaters taper in rather than popping out of nothing
FLOW_MIN = 120.0
seg = (flow > FLOW_MIN) & ~ocean & valid
si = ids[seg]
print(f"{seg.sum()} river segments (flow {flow[seg].min():.0f}..{flow[seg].max():.0f} m3/s)")

# ---- course smoothing: raw cell->receiver polylines kink at every 30 km
# node. Pull each node toward the midpoint of its main upstream (largest
# flow) and its receiver — two passes round the corners while endpoints
# stay shared, so confluences remain watertight. Levels smooth alongside.
main_up = np.full(n, -1, dtype=np.int64)
best_flow = np.zeros(n)
for i in ids[seg]:
    r = rcv[i]
    if flow[i] > best_flow[r]:
        best_flow[r] = flow[i]
        main_up[r] = i
sxyz = xyz.copy()
slevel = level.copy()
touched = np.unique(np.concatenate([si, rcv[si]]))
for _ in range(2):
    nxyz = sxyz.copy()
    nlevel = slevel.copy()
    for i in touched:
        up, dn = main_up[i], rcv[i]
        if up >= 0 and dn >= 0 and dn != i:
            if ocean[dn]:
                # a MOUTH node stays anchored at its own cell: smoothing
                # dragged it toward an ocean receiver up to ~40 km offshore,
                # so the great river's course missed its own mouth cell by
                # 10+ km and the documented mouth sat bone dry (round-6
                # hunt). The unsmoothed final kink is what deltas look like
                # anyway; the segment to the ocean cell still carries the
                # river into the sea.
                pass
            else:
                nxyz[i] = 0.5 * sxyz[i] + 0.25 * sxyz[up] + 0.25 * sxyz[dn]
                nlevel[i] = min(slevel[i], 0.5 * slevel[i] + 0.25 * slevel[up] + 0.25 * slevel[dn])
    sxyz, slevel = nxyz, nlevel
sxyz /= np.linalg.norm(sxyz, axis=1, keepdims=True)
# smoothing moved nodes off their source cells (up to ~7 km): re-anchor
# each level to the FILL SURFACE under the NEW position — otherwise the
# level can sit above the local water table and the viewer's perch guard
# dries the whole reach. Anchoring to raw elev would re-dive every pit to
# its bed, undoing the fill construction.
kd = cKDTree(xyz)
_, near3 = kd.query(sxyz[touched], k=3, workers=-1)
slevel[touched] = np.minimum(slevel[touched], level[near3].min(1))
slevel[ocean] = 0.0  # re-anchoring must not resurrect bathymetry levels
for _ in range(10000):
    before = slevel[dst].copy()
    np.minimum.at(slevel, dst, slevel[src])
    if np.array_equal(before, slevel[dst]):
        break
slevel[ocean] = 0.0

# ---- lakes: peel CONDUIT cells first (BUGS.md W-5). The sim merges chains
# of filled depressions into one lake id: lake 873 spans beds 588..3279 m
# under a 3282 m spill — a "lake" smeared 2.7 km down a mountainside, whose
# flank cells flood the slopes and whose exempted outlet cuts kilometre
# walls through the fill. A conduit cell is chain-like (<= 2 same-lake
# neighbours) AND far below the spill; peeling repeatedly strips the
# strings from their ends while true basins (deep cells SURROUNDED by lake)
# survive. Level = spill of the peeled basin.
lake_ids = np.unique(lake[lake >= 0])
lake_cells = {}
lake_level = {}
lake_set = set()
for lid in lake_ids:
    lake_set.update(ids[lake == lid].tolist())
for lid in lake_ids:
    cells = set(ids[lake == lid].tolist())
    # provisional spill from the full footprint, for the depth test
    all_nb = nb[list(cells)].ravel()
    all_nb = all_nb[all_nb >= 0]
    outs = [c for c in all_nb if lake[c] != lid]
    spill0 = min(elev[outs].min() if outs else elev[list(cells)].max(),
                 elev[list(cells)].max() + 0.005)
    # depth cut: this planet's healthy lakes are shallow dishes (median
    # depth 24 m, p99 376 m — measured); anything hundreds of metres below
    # its own spill is a merged-depression conduit, not basin. One outlier
    # (lid 873) ran 2.7 km deep down a mountain flank.
    cells = {c for c in cells if spill0 - elev[c] <= 0.30}
    while True:
        peel = []
        for c in cells:
            nn = [x for x in nb[c] if x >= 0 and x in cells]
            if len(nn) <= 2 and spill0 - elev[c] > 0.25:
                peel.append(c)
        if not peel:
            break
        cells.difference_update(peel)
    cells = np.array(sorted(cells), dtype=np.int64)
    lake_cells[lid] = cells
    if len(cells) == 0:
        lake_level[lid] = -9999.0
        continue
    beds = elev[cells]
    nbs = nb[cells].ravel()
    nbs = nbs[nbs >= 0]
    outside = nbs[lake[nbs] != lid]
    # neighbours that were peeled off count as outside too (the basin now
    # spills into its own former conduit)
    peeled_nb = np.array(
        [x for x in np.unique(nbs) if lake[x] == lid and x not in set(cells.tolist())],
        dtype=np.int64,
    )
    rim_pool = np.concatenate([outside, peeled_nb]) if len(peeled_nb) else outside
    rim = elev[rim_pool].min() if len(rim_pool) else beds.max()
    lake_level[lid] = min(rim, beds.max() + 0.005)
n_peeled = sum(1 for lid in lake_ids if len(lake_cells[lid]) < (lake == lid).sum())
print(f"conduit peel: {n_peeled} lakes shrank; "
      f"{sum(1 for lid in lake_ids if len(lake_cells[lid]) == 0)} vanished entirely")

# ---- renderability cap (BUGS.md W-5): the viewer's terrain is a smooth
# BLEND of these cells, and a knife-edge dam narrower than the blend scale
# simply does not exist in the rendered world — the sim says "3282 m crater
# lake", the rendered mountain says "open flank 1600 m below". Any flood of
# such a lake ends in a territory-edge water cliff (census-measured: every
# local flood rule just relocates it). So: reconstruct the flood territory
# the viewer will use (lake-cell Voronoi + rim territory within 1.15 r,
# capped at 3 r — mirrors rivers::lake_at + terrain.rs), sample the BLENDED
# elevation along its boundary, and cap the level so no boundary cliff can
# exceed ~15 m + noise. Healthy lakes (dams wider than the blend) keep
# their exact spill; unrenderable ones drop below their beds and export as
# dry basins.
WALL_TOL_KM = 0.015
# planet radius from the data itself: sum of cell areas = 4 pi R^2
R_planet = float(np.sqrt(area.sum() / (4.0 * np.pi)))
for lid in lake_ids:
    cells = lake_cells[lid]  # peeled basin, not the raw sim footprint
    if len(cells) == 0:
        continue
    cell_set = set(cells.tolist())
    nbs_all = nb[cells].ravel()
    nbs_all = nbs_all[nbs_all >= 0]
    rims = np.unique([x for x in nbs_all if x not in cell_set])
    members = np.concatenate([cells, rims])
    mxyz = xyz[members]
    is_lake_m = np.zeros(len(members), dtype=bool)
    is_lake_m[: len(cells)] = True
    r_cell = np.sqrt(area[members] / np.pi)
    # local tangent grid over the lake footprint + reach
    center = mxyz.mean(0)
    center /= np.linalg.norm(center)
    ax = np.array([0.0, 0.0, 1.0]) if abs(center[2]) < 0.9 else np.array([1.0, 0.0, 0.0])
    t1 = ax - center * ax.dot(center)
    t1 /= np.linalg.norm(t1)
    t2 = np.cross(center, t1)
    ext = (np.linalg.norm((mxyz - center) @ np.column_stack([t1, t2]), axis=1).max()
           + 3.2 * r_cell.max() / R_planet)
    step = max(2.0 / R_planet, ext / 60.0)
    g = np.arange(-ext, ext + step, step)
    gu, gv = np.meshgrid(g, g)
    pts = (center[None, :]
           + gu.ravel()[:, None] * t1[None, :]
           + gv.ravel()[:, None] * t2[None, :])
    pts /= np.linalg.norm(pts, axis=1, keepdims=True)
    # nearest member cell -> territory decision (chord distances ~ km)
    mt = cKDTree(mxyz)
    dm, im = mt.query(pts, k=1, workers=-1)
    dm_km = dm * R_planet
    lk = cKDTree(mxyz[is_lake_m])
    dl, il = lk.query(pts, k=1, workers=-1)
    dl_km = dl * R_planet
    r_of_lake = r_cell[is_lake_m][il]
    in_terr = np.where(
        is_lake_m[im],
        dl_km < 3.0 * r_of_lake,
        (dm_km < 1.15 * r_of_lake) & (dl_km < 3.0 * r_of_lake),
    )
    grid_n = len(g)
    terr = in_terr.reshape(grid_n, grid_n)
    # the ground a wall would stand against is the OUTSIDE of the territory
    # edge: non-territory points adjacent to territory. (Measuring the
    # boundary's own points instead caps against covered interior dips —
    # e.g. lake 414's flooded pit ring — which the flood exists to cover;
    # the regression gate caught exactly that.)
    b = np.zeros_like(terr)
    b[1:-1, 1:-1] = ~terr[1:-1, 1:-1] & (
        terr[:-2, 1:-1] | terr[2:, 1:-1] | terr[1:-1, :-2] | terr[1:-1, 2:]
    )
    bpts = pts[b.ravel()]
    if len(bpts) == 0:
        continue
    # blended elevation at the outside edge (IDW k=4 — the raster's blend)
    db, ib = kd.query(bpts, k=4, workers=-1)
    w = 1.0 / np.maximum(db, 1e-9)
    be = (elev[ib] * w).sum(1) / w.sum(1)
    cap = be.min() + WALL_TOL_KM
    if cap < lake_level[lid]:
        lake_level[lid] = cap
# exported lake cells = the PEELED basins only
lc = np.concatenate(
    [lake_cells[lid] for lid in lake_ids if len(lake_cells[lid])]
) if any(len(lake_cells[lid]) for lid in lake_ids) else np.array([], dtype=np.int64)
remaining_lake = np.zeros(n, dtype=bool)
remaining_lake[lc] = True
# lakes capped below their own beds hold no renderable water: export them
# as dry (level below bed floods nothing in the viewer, and the slevel pin
# below only ever raises to a level that is now harmless)
n_dropped = sum(
    1
    for lid in lake_ids
    if len(lake_cells[lid]) and lake_level[lid] < elev[lake_cells[lid]].min()
)
print(f"renderability cap: {n_dropped} of {len(lake_ids)} lakes below their beds (render dry)")

# ---- river nodes inside a lake carry the lake SURFACE level, not their
# bed: the outlet river otherwise leaves the lake tens of metres below its
# own surface (the bed re-anchor above pulled in-lake nodes down to bed
# elevation), and the lake-flood edge stood as a water cliff over the
# sunken corridor (BUGS.md W-1, the lon-68 "pie slice"). Raising only —
# a node already above the fill keeps its level. Downstream monotonicity
# is then restored WITHOUT links into lake cells (a bed-anchored tributary
# entering a lake must not drag the fill back down; its last reach is
# drowned by the flood anyway).
for lid in lake_ids:
    cells = lake_cells[lid]  # only the peeled basin pins river nodes —
    # conduit cells pinned to the summit level were themselves a wall source
    if len(cells) == 0 or lake_level[lid] < -999:
        continue
    slevel[cells] = np.maximum(slevel[cells], lake_level[lid])
open_dst = valid & (lake[rcv] < 0)
src2 = ids[open_dst]
dst2 = rcv[open_dst]
for _ in range(10000):
    before = slevel[dst2].copy()
    np.minimum.at(slevel, dst2, slevel[src2])
    if np.array_equal(before, slevel[dst2]):
        break
slevel[ocean] = 0.0

# rim cells: neighbors of the PEELED lake cells that are not themselves
# remaining lake cells. Peeled conduit cells that border a basin become RIM
# competitors — without them the basin's Voronoi territory would reach down
# its own former conduit and re-flood the flank through the notch.
rim_set = np.zeros(n, dtype=bool)
nbs_of_lakes = nb[lc].ravel()
nbs_of_lakes = nbs_of_lakes[nbs_of_lakes >= 0]
rim_set[nbs_of_lakes] = True
rim_set[remaining_lake] = False
rim = ids[rim_set]
print(f"{len(lc)} lake cells in {len(lake_ids)} lakes, {len(rim)} rim cells")

with open(out_path, "wb") as f:
    f.write(b"RIV1")
    f.write(struct.pack("<II", int(seg.sum()), len(lc) + len(rim)))
    a = sxyz[si]
    b = sxyz[rcv[si]]
    rows = np.column_stack([
        a,
        b,
        np.log10(1.0 + flow[si]),
        slevel[si],
        slevel[rcv[si]],
    ]).astype("<f4")
    f.write(rows.tobytes())
    lrows = np.column_stack([
        xyz[lc],
        np.sqrt(area[lc] / np.pi),
        np.array([lake_level[l] for l in lake[lc]]),
        salt[lc].astype(np.float64),
    ]).astype("<f4")
    f.write(lrows.tobytes())
    # rim rows carry their own ELEVATION in the level field (flags mark them
    # as rims): the viewer's shore-band flood-through must know whether a
    # rim is a true dam (elev >= spill) or a peeled conduit far below it
    rrows = np.column_stack([
        xyz[rim],
        np.sqrt(area[rim] / np.pi),
        elev[rim],
        np.full(len(rim), 2.0),
    ]).astype("<f4")
    f.write(rrows.tobytes())
print(f"wrote {out_path} ({os.path.getsize(out_path)} bytes)")

# scenic helpers: the grandest rivers and lakes, as lat/lon
big = si[np.argsort(-flow[si])[:8]]
for i in big:
    la = np.degrees(np.arcsin(xyz[i, 2]))
    lo = np.degrees(np.arctan2(xyz[i, 1], xyz[i, 0]))
    print(f"river: flow {flow[i]:7.0f} m3/s  level {level[i]*1000:7.1f} m  lat {la:8.3f} lon {lo:8.3f}")
for lid, cnt in sorted(((l, (lake == l).sum()) for l in lake_ids), key=lambda t: -t[1])[:5]:
    cells = ids[lake == lid]
    c = xyz[cells].mean(0)
    c /= np.linalg.norm(c)
    la = np.degrees(np.arcsin(c[2]))
    lo = np.degrees(np.arctan2(c[1], c[0]))
    print(f"lake {lid}: {cnt} cells, level {lake_level[lid]*1000:6.1f} m, salt {bool(salt[cells][0])}  lat {la:8.3f} lon {lo:8.3f}")
