"""Bake the planet dataset onto gnomonic cube-face rasters for the viewer.

The viewer's LOD tree tops out at the planet map; these rasters ARE those top
levels, in the exact face coordinates the game will use.

Face convention (must match viewer/src/planet.rs):
  face 0: +X axis, right=+Y, up=+Z        face 1: -X axis, right=-Y, up=+Z
  face 2: +Y axis, right=-X, up=+Z        face 3: -Y axis, right=+X, up=+Z
  face 4: +Z axis, right=+Y, up=-X        face 5: -Z axis, right=+Y, up=+X
  direction(u, v) = normalize(axis + u*right + v*up),  u, v in [-1, 1]

Output per face: little-endian binary raster, row-major, v-major (row 0 = v
= -1 edge). Layers in order:
  float32 elevation_km
  uint8   koppen id (255 = ocean)
  float32 rough_km      (mean |elevation delta| to graph neighbors)
  float32 precip_mm_yr  (annual precipitation)
  float32 temp_c        (annual mean 2m temperature)
  float32 flow_log10    (log10(1 + river flow accumulation m3/s), max of 3NN)

Usage: python scripts/bake_faces.py [run_dir] [resolution]
"""

import json
import os
import sys

import numpy as np
from scipy.spatial import cKDTree

run_dir = sys.argv[1] if len(sys.argv) > 1 else "output/seed42_r8"
res = int(sys.argv[2]) if len(sys.argv) > 2 else 1024
out_dir = "viewer/assets"
os.makedirs(out_dir, exist_ok=True)

d = np.load(os.path.join(run_dir, "planet_data.npz"))
xyz = d["xyz"].astype(np.float64)
elev = d["elevation_km"].astype(np.float32)
koppen = d["koppen"].astype(np.int16)
is_ocean = d["is_ocean"]
kd = cKDTree(xyz)

# derived per-cell fields for the game's procedural generation
nb = d["neighbors"]
valid = nb >= 0
nb_safe = np.where(valid, nb, 0)
rough = (np.abs(elev[nb_safe] - elev[:, None]) * valid).sum(1)
rough = (rough / np.maximum(valid.sum(1), 1)).astype(np.float32)
precip_yr = (d["precip_mm_monthly"].mean(0) * 12.0).astype(np.float32)
temp_ann = d["temp_c_monthly"].mean(0).astype(np.float32)
flow_log = np.log10(1.0 + d["flow_accum_m3s"]).astype(np.float32)

FACES = [
    (np.array([1, 0, 0]), np.array([0, 1, 0]), np.array([0, 0, 1])),
    (np.array([-1, 0, 0]), np.array([0, -1, 0]), np.array([0, 0, 1])),
    (np.array([0, 1, 0]), np.array([-1, 0, 0]), np.array([0, 0, 1])),
    (np.array([0, -1, 0]), np.array([1, 0, 0]), np.array([0, 0, 1])),
    (np.array([0, 0, 1]), np.array([0, 1, 0]), np.array([-1, 0, 0])),
    (np.array([0, 0, -1]), np.array([0, 1, 0]), np.array([1, 0, 0])),
]

# edge-INCLUSIVE grid: texels at index 0 and res-1 sit exactly on the cube
# edge, so adjacent faces sample identical sphere points along shared edges —
# without this, bilinear clamping leaves a half-texel seam on all 12 edges
grid = np.linspace(-1.0, 1.0, res)
for fi, (axis, right, up) in enumerate(FACES):
    U, V = np.meshgrid(grid, grid)               # row-major, v-major
    dirs = (axis[None, None, :]
            + U[..., None] * right[None, None, :]
            + V[..., None] * up[None, None, :]).reshape(-1, 3)
    dirs /= np.linalg.norm(dirs, axis=1, keepdims=True)
    dist, idx = kd.query(dirs, k=3, workers=-1)
    w = 1.0 / np.maximum(dist, 1e-12) ** 2
    w /= w.sum(1, keepdims=True)
    e = (elev[idx] * w).sum(1).astype(np.float32)
    k_near = koppen[idx[:, 0]]                    # categorical: nearest
    k_out = np.where(is_ocean[idx[:, 0]], 255, k_near).astype(np.uint8)
    r_out = (rough[idx] * w).sum(1).astype(np.float32)
    p_out = (precip_yr[idx] * w).sum(1).astype(np.float32)
    t_out = (temp_ann[idx] * w).sum(1).astype(np.float32)
    f_out = flow_log[idx].max(1).astype(np.float32)  # max: keep river lines fat
    with open(os.path.join(out_dir, f"face_{fi}.bin"), "wb") as f:
        f.write(e.tobytes())
        f.write(k_out.tobytes())
        f.write(r_out.tobytes())
        f.write(p_out.tobytes())
        f.write(t_out.tobytes())
        f.write(f_out.tobytes())
    print(f"face {fi}: elev {e.min():.2f}..{e.max():.2f} km, "
          f"ocean {(k_out == 255).mean() * 100:.0f}%, "
          f"rough {r_out.max():.2f}, precip {p_out.max():.0f}, "
          f"flow {f_out.max():.2f}")

with open(os.path.join(run_dir, "planet.json"), encoding="utf-8") as f:
    seed = json.load(f)["seed"]
# cube edge 10,000 km inscribed in the sphere -> R = edge * sqrt(3)/2
meta = dict(resolution=res, radius_km=float(np.sqrt(3) / 2 * 10000.0),
            face_edge_km=10000.0,
            fields=["elevation_km:f32", "koppen:u8", "rough_km:f32",
                    "precip_mm_yr:f32", "temp_c:f32", "flow_log10:f32"],
            source=run_dir, planet="Neisor", seed=int(seed))
with open(os.path.join(out_dir, "meta.json"), "w") as f:
    json.dump(meta, f, indent=1)
print(f"baked {res}x{res} x6 -> {out_dir} (radius {meta['radius_km']:.0f} km)")
