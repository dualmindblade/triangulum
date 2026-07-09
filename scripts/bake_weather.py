"""Bake the monthly climate fields into compact harmonic rasters for the
viewer's weather system (WEATHER.md Layer 1).

Per cell, each 12-month series y[m] collapses to its mean and first annual
harmonic, fit at month centers t_m = (m + 0.5)/12:

  value(t_yr) = mean + a*cos(2*pi*t_yr) + b*sin(2*pi*t_yr)

The CARTESIAN coefficients (a, b) are what get rasterized — amp/phase
cannot be IDW-blended across texels (phase wraps at +-pi; a texel between
a January-peak and a December-peak cell would average to July).

Output viewer/assets/weather.bin (little-endian):
  magic  b"WEA1"
  u32    res      (texels per face edge; edge-inclusive like face_*.bin)
  u32    n_layers (10)
  6 faces x 10 layers x res^2 f32, v-major rows, layer order:
    temp_a_c, temp_b_c            (temp mean lives in face_*.bin)
    prc_mean, prc_a, prc_b        (mm/month)
    cld_mean, cld_a, cld_b        (cloud fraction 0..1)
    wind_e, wind_n                (m/s annual mean)

Face convention and k=3 IDW^2 rasterization match scripts/bake_faces.py.
Weather varies at synoptic scales, so the default 256/face (~40 km/texel,
~16 MB total) is deliberately coarse.

Usage: python scripts/bake_weather.py [run_dir] [resolution]
"""

import os
import struct
import sys

import numpy as np
from scipy.spatial import cKDTree

run_dir = sys.argv[1] if len(sys.argv) > 1 else "output/seed42_r8"
res = int(sys.argv[2]) if len(sys.argv) > 2 else 256
out_path = "viewer/assets/weather.bin"

d = np.load(os.path.join(run_dir, "planet_data.npz"))
xyz = d["xyz"].astype(np.float64)
xyz /= np.linalg.norm(xyz, axis=1, keepdims=True)
kd = cKDTree(xyz)

t_m = (np.arange(12) + 0.5) / 12.0
cos_m = np.cos(2 * np.pi * t_m)[:, None]
sin_m = np.sin(2 * np.pi * t_m)[:, None]


def harmonic(series):
    """(mean, a, b) of a (12, n) monthly series, first annual harmonic."""
    mean = series.mean(0)
    a = (series * cos_m).sum(0) * (2.0 / 12.0)
    b = (series * sin_m).sum(0) * (2.0 / 12.0)
    return mean.astype(np.float32), a.astype(np.float32), b.astype(np.float32)


temp = d["temp_c_monthly"].astype(np.float64)
prc = d["precip_mm_monthly"].astype(np.float64)
cld = d["clouds_monthly"].astype(np.float64)
_, temp_a, temp_b = harmonic(temp)
prc_mean, prc_a, prc_b = harmonic(prc)
cld_mean, cld_a, cld_b = harmonic(cld)
wind_e = d["wind_e_monthly"].mean(0).astype(np.float32)
wind_n = d["wind_n_monthly"].mean(0).astype(np.float32)

# reconstruction sanity: the harmonic must track the real seasons. Report
# the residual so a bad fit is loud instead of a silent mush.
recon = temp.mean(0)[None, :] + temp_a[None, :] * cos_m + temp_b[None, :] * sin_m
rms = float(np.sqrt(((temp - recon) ** 2).mean()))
swing = float((temp.max(0) - temp.min(0)).mean())
print(f"temp harmonic fit: residual RMS {rms:.2f} C against a mean seasonal swing of {swing:.2f} C")

LAYERS = [temp_a, temp_b, prc_mean, prc_a, prc_b, cld_mean, cld_a, cld_b, wind_e, wind_n]

FACES = [
    (np.array([1, 0, 0]), np.array([0, 1, 0]), np.array([0, 0, 1])),
    (np.array([-1, 0, 0]), np.array([0, -1, 0]), np.array([0, 0, 1])),
    (np.array([0, 1, 0]), np.array([-1, 0, 0]), np.array([0, 0, 1])),
    (np.array([0, -1, 0]), np.array([1, 0, 0]), np.array([0, 0, 1])),
    (np.array([0, 0, 1]), np.array([0, 1, 0]), np.array([-1, 0, 0])),
    (np.array([0, 0, -1]), np.array([0, 1, 0]), np.array([1, 0, 0])),
]

grid = np.linspace(-1.0, 1.0, res)  # edge-inclusive, like face_*.bin
out = bytearray()
out += b"WEA1"
out += struct.pack("<II", res, len(LAYERS))
for fi, (axis, right, up) in enumerate(FACES):
    U, V = np.meshgrid(grid, grid)  # row-major, v-major
    dirs = (axis[None, None, :]
            + U[..., None] * right[None, None, :]
            + V[..., None] * up[None, None, :]).reshape(-1, 3)
    dirs /= np.linalg.norm(dirs, axis=1, keepdims=True)
    dist, idx = kd.query(dirs, k=3, workers=-1)
    w = 1.0 / np.maximum(dist, 1e-12) ** 2
    w /= w.sum(1, keepdims=True)
    for layer in LAYERS:
        tex = (layer[idx] * w).sum(1).astype("<f4")
        out += tex.tobytes()
    print(f"face {fi} done")

with open(out_path, "wb") as fh:
    fh.write(out)
print(f"wrote {out_path} ({len(out)} bytes)")
