"""Bake the monthly climate fields into compact harmonic rasters for the
viewer's weather system (WEATHER.md Layer 1).

Per cell, each 12-month series y[m] collapses to smooth Fourier terms fit at
month centers t_m = (m + 0.5)/12. Temperature keeps the dominant annual
term; precipitation and clouds also keep the semiannual term needed by
bimodal/monsoon regimes:

  value(t_yr) = mean
              + a1*cos(2*pi*t_yr) + b1*sin(2*pi*t_yr)
              + a2*cos(4*pi*t_yr) + b2*sin(4*pi*t_yr)

The CARTESIAN coefficients (a, b) are what get rasterized — amp/phase
cannot be IDW-blended across texels (phase wraps at +-pi; a texel between
a January-peak and a December-peak cell would average to July).

Output viewer/assets/weather.bin (little-endian):
  magic  b"WEA2"
  u32    res      (texels per face edge; edge-inclusive like face_*.bin)
  u32    n_layers (14)
  6 faces x 14 layers x res^2 f32, v-major rows, layer order:
    temp_a_c, temp_b_c            (temp mean lives in face_*.bin)
    prc_mean, prc_a1, prc_b1, prc_a2, prc_b2  (mm/month)
    cld_mean, cld_a1, cld_b1, cld_a2, cld_b2  (cloud fraction 0..1)
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
cos_1 = np.cos(2 * np.pi * t_m)[:, None]
sin_1 = np.sin(2 * np.pi * t_m)[:, None]
cos_2 = np.cos(4 * np.pi * t_m)[:, None]
sin_2 = np.sin(4 * np.pi * t_m)[:, None]


def harmonics(series):
    """(mean, a1, b1, a2, b2) for a (12, n) monthly series."""
    mean = series.mean(0)
    a1 = (series * cos_1).sum(0) * (2.0 / 12.0)
    b1 = (series * sin_1).sum(0) * (2.0 / 12.0)
    a2 = (series * cos_2).sum(0) * (2.0 / 12.0)
    b2 = (series * sin_2).sum(0) * (2.0 / 12.0)
    return tuple(x.astype(np.float32) for x in (mean, a1, b1, a2, b2))


def reconstruction(mean, a1, b1, a2=None, b2=None):
    """Reconstruct all 12 month centers from the selected coefficients."""
    out = mean[None, :] + a1[None, :] * cos_1 + b1[None, :] * sin_1
    if a2 is not None:
        out = out + a2[None, :] * cos_2 + b2[None, :] * sin_2
    return out


def rms(actual, fitted):
    return float(np.sqrt(np.mean((actual - fitted) ** 2)))


temp = d["temp_c_monthly"].astype(np.float64)
prc = d["precip_mm_monthly"].astype(np.float64)
cld = d["clouds_monthly"].astype(np.float64)
_, temp_a, temp_b, _, _ = harmonics(temp)
prc_mean, prc_a1, prc_b1, prc_a2, prc_b2 = harmonics(prc)
cld_mean, cld_a1, cld_b1, cld_a2, cld_b2 = harmonics(cld)
wind_e = d["wind_e_monthly"].mean(0).astype(np.float32)
wind_n = d["wind_n_monthly"].mean(0).astype(np.float32)

# Reconstruction sanity: report the residuals of the coefficients that are
# about to be rasterized. These lines are also the WEA2 fidelity gate: a bad
# source field or accidentally dropped k=2 term is loud during every rebake.
temp_recon = reconstruction(temp.mean(0), temp_a, temp_b)
swing = float((temp.max(0) - temp.min(0)).mean())
print(
    f"temp k=1 fit: RMS {rms(temp, temp_recon):.2f} C "
    f"against a mean seasonal swing of {swing:.2f} C"
)
for label, series, coeffs, unit in (
    ("precip", prc, (prc_mean, prc_a1, prc_b1, prc_a2, prc_b2), "mm/month"),
    ("cloud", cld, (cld_mean, cld_a1, cld_b1, cld_a2, cld_b2), "fraction"),
):
    mean, a1, b1, a2, b2 = coeffs
    before = rms(series, reconstruction(mean, a1, b1))
    after = rms(series, reconstruction(mean, a1, b1, a2, b2))
    print(f"{label} harmonic fit: k=1 RMS {before:.5f} -> k=1+2 RMS {after:.5f} {unit}")

# Keep the two review canaries visible in the bake transcript. Nearest-cell
# lookup is intentional: these coordinates came directly from the source
# cells, and dot-product lookup avoids depending on any face rasterization.
for label, lat, lon, series, coeffs, unit in (
    ("cloud", 27.583315, 117.155914, cld, (cld_mean, cld_a1, cld_b1, cld_a2, cld_b2), "fraction"),
    ("precip", -12.472685, 29.169937, prc, (prc_mean, prc_a1, prc_b1, prc_a2, prc_b2), "mm/month"),
):
    la, lo = np.radians([lat, lon])
    q = np.array([np.cos(la) * np.cos(lo), np.cos(la) * np.sin(lo), np.sin(la)])
    i = int(np.argmax(xyz @ q))
    mean, a1, b1, a2, b2 = (x[i:i + 1] for x in coeffs)
    before = rms(series[:, i:i + 1], reconstruction(mean, a1, b1))
    after = rms(series[:, i:i + 1], reconstruction(mean, a1, b1, a2, b2))
    amp1 = float(np.hypot(a1[0], b1[0]))
    amp2 = float(np.hypot(a2[0], b2[0]))
    print(
        f"bimodal {label} site {lat:.6f} {lon:.6f}: "
        f"k=1 RMS {before:.5f} -> k=1+2 RMS {after:.5f} {unit}; "
        f"amplitudes annual={amp1:.5f} semiannual={amp2:.5f}"
    )

LAYERS = [
    temp_a, temp_b,
    prc_mean, prc_a1, prc_b1, prc_a2, prc_b2,
    cld_mean, cld_a1, cld_b1, cld_a2, cld_b2,
    wind_e, wind_n,
]

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
out += b"WEA2"
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
