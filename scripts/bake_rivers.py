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

# ---- water levels: bed elevation, relaxed so no receiver is ever higher
# than its source (depression-filled routes cross ridges: raw elevation
# ascends up to ~650 m along some receiver links)
level = elev.copy()
valid = (rcv >= 0) & (rcv != ids)
src = ids[valid]
dst = rcv[valid]
for it in range(10000):
    before = level[dst].copy()
    np.minimum.at(level, dst, level[src])
    if np.array_equal(before, level[dst]):
        break
print(f"level relaxation converged after {it + 1} passes")

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
            nxyz[i] = 0.5 * sxyz[i] + 0.25 * sxyz[up] + 0.25 * sxyz[dn]
            nlevel[i] = min(slevel[i], 0.5 * slevel[i] + 0.25 * slevel[up] + 0.25 * slevel[dn])
    sxyz, slevel = nxyz, nlevel
sxyz /= np.linalg.norm(sxyz, axis=1, keepdims=True)
# smoothing moved nodes off their source cells (up to ~7 km): re-anchor
# each level to the terrain under the NEW position — otherwise the level
# can sit above the local ground and the viewer's perch guard dries the
# whole reach — then restore downstream monotonicity
kd = cKDTree(xyz)
_, near3 = kd.query(sxyz[touched], k=3, workers=-1)
slevel[touched] = np.minimum(slevel[touched], elev[near3].min(1))
for _ in range(10000):
    before = slevel[dst].copy()
    np.minimum.at(slevel, dst, slevel[src])
    if np.array_equal(before, slevel[dst]):
        break

# ---- lakes: level = spill point (lowest non-lake neighbor of the lake),
# capped just above the highest lake-cell bed
lake_ids = np.unique(lake[lake >= 0])
lake_level = {}
for lid in lake_ids:
    cells = ids[lake == lid]
    beds = elev[cells]
    nbs = nb[cells].ravel()
    nbs = nbs[nbs >= 0]
    outside = nbs[lake[nbs] != lid]
    rim = elev[outside].min() if len(outside) else beds.max()
    lake_level[lid] = min(rim, beds.max() + 0.005)
lc = ids[lake >= 0]
# rim cells: non-lake neighbors of any lake cell (Voronoi competitors)
rim_set = np.zeros(n, dtype=bool)
nbs_of_lakes = nb[lc].ravel()
nbs_of_lakes = nbs_of_lakes[nbs_of_lakes >= 0]
rim_set[nbs_of_lakes] = True
rim_set[lake >= 0] = False
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
    rrows = np.column_stack([
        xyz[rim],
        np.sqrt(area[rim] / np.pi),
        np.zeros(len(rim)),
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
